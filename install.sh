#!/usr/bin/env bash
# OmaLiftoff companion installer: daemon + TUI + notifications for the OmaLiftoff bar widget.
# Runs from a checkout of this repo, normally the plugin folder itself:
#
#   omarchy plugin add https://github.com/0xRainy/omaliftoff --enable
#   bash ~/.config/omarchy/plugins/liftoff.tracker/install.sh
#
#   ./install.sh               install or upgrade (idempotent)
#   ./install.sh --setup       notification + starred-provider wizard only (`liftoff setup`)
#   ./install.sh --uninstall   remove binary, service and plugin (asks about config/cache)
#   ./install.sh --yes         accept defaults, no prompts
#   ./install.sh --from-source skip the prebuilt binary
# ok/warn always succeed, so `cmd && ok .. || warn ..` is intended.
# shellcheck disable=SC2015
set -euo pipefail

REPO="0xRainy/omaliftoff"
REPO_URL="https://github.com/$REPO"
PLUGIN_ID="liftoff.tracker"
BIN_DIR="$HOME/.local/bin"
BIN="$BIN_DIR/liftoff"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/liftoff"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
UNIT="$UNIT_DIR/liftoff.service"
PLUGIN_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/omarchy/plugins/$PLUGIN_ID"

MODE=install YES=0 FROM_SOURCE=0
for a in "$@"; do
  case "$a" in
    --setup) MODE=setup ;;
    --uninstall) MODE=uninstall ;;
    --yes|-y) YES=1 ;;
    --from-source) FROM_SOURCE=1 ;;
    -h|--help) sed -n '2,10p' "${BASH_SOURCE[0]:-/dev/null}" 2>/dev/null || true; exit 0 ;;
    *) echo "unknown option: $a" >&2; exit 2 ;;
  esac
done

# Must run from a checkout (the plugin folder or a git clone): never download-and-run.
SRC=""
if [[ -n "${BASH_SOURCE[0]:-}" && -f "${BASH_SOURCE[0]}" ]]; then
  d="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  [[ -f "$d/Cargo.toml" && -f "$d/manifest.json" ]] && SRC="$d"
fi
if [[ -z $SRC && $MODE == install ]]; then
  echo "Run install.sh from a checkout of $REPO_URL, e.g. after" >&2
  echo "  omarchy plugin add $REPO_URL --enable" >&2
  echo "run: bash $PLUGIN_DIR/install.sh" >&2
  exit 1
fi
VERSION=""
[[ -n $SRC ]] && VERSION=$(sed -n 's/^ *"version": *"\([^"]*\)".*/\1/p' "$SRC/manifest.json" | head -1)

# ---------- UI helpers (gum when available, plain prompts otherwise) ----------
TTY=/dev/tty
if [[ $YES == 1 ]] || ! { : <"$TTY"; } 2>/dev/null; then INTERACTIVE=0; else INTERACTIVE=1; fi
GUM=0; [[ $INTERACTIVE == 1 ]] && command -v gum >/dev/null && GUM=1

say()  { if [[ $GUM == 1 ]]; then gum style --foreground 212 --bold "==> $*"; else printf '\033[1;35m==>\033[0m %s\n' "$*"; fi; }
info() { printf '    %s\n' "$*"; }
ok()   { printf '    \033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '    \033[33m!\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# ask_yn "question" default(y|n)
ask_yn() {
  local q=$1 def=${2:-y} r
  [[ $INTERACTIVE == 1 ]] || { [[ $def == y ]]; return; }
  if [[ $GUM == 1 ]]; then
    if [[ $def == y ]]; then gum confirm "$q" <"$TTY"; else gum confirm --default=false "$q" <"$TTY"; fi
    return
  fi
  read -r -p "    $q [$([[ $def == y ]] && echo Y/n || echo y/N)] " r <"$TTY" || r=""
  r=${r:-$def}; [[ $r =~ ^[Yy] ]]
}
# ask "prompt" default [--password]
ask() {
  local q=$1 def=${2:-} pw=${3:-} r
  [[ $INTERACTIVE == 1 ]] || { printf '%s' "$def"; return; }
  if [[ $GUM == 1 ]]; then
    if [[ $pw == --password ]]; then r=$(gum input --password --header "$q" <"$TTY"); else r=$(gum input --header "$q" --value "$def" <"$TTY"); fi
  else
    if [[ $pw == --password ]]; then read -r -s -p "    $q: " r <"$TTY"; echo >&2; else read -r -p "    $q [$def]: " r <"$TTY"; fi
  fi
  printf '%s' "${r:-$def}"
}
# choose "header" default option...
choose() {
  local h=$1 def=$2; shift 2
  [[ $INTERACTIVE == 1 ]] || { printf '%s' "$def"; return; }
  if [[ $GUM == 1 ]]; then gum choose --header "$h" --selected "$def" "$@" <"$TTY"; return; fi
  local i=1 o r; echo "    $h" >&2
  for o in "$@"; do echo "      $i) $o" >&2; i=$((i+1)); done
  read -r -p "    choice [$def]: " r <"$TTY" || r=""
  if [[ $r =~ ^[0-9]+$ ]] && (( r >= 1 && r <= $# )); then printf '%s' "${!r}"; else printf '%s' "$def"; fi
}
have() { command -v "$1" >/dev/null 2>&1; }

# ---------- steps ----------
check_deps() {
  say "Checking dependencies"
  have curl || die "curl is required (package: curl)"
  ok "curl"
  if [[ -d /usr/share/omarchy/shell ]] || have omarchy-shell; then ok "Omarchy shell (Quattro)"
  else warn "Omarchy Quattro shell not found: the bar widget needs it; the TUI and daemon work anywhere."; fi
  if have notify-send; then ok "notify-send"; else warn "notify-send missing (package: libnotify)"; fi
  if have makoctl || pgrep -x mako >/dev/null 2>&1; then ok "mako"; else info "no mako detected (any notification daemon works)"; fi
  have systemctl || die "systemd is required for the background service"
}

arch_triple() {
  case "$(uname -m)" in
    x86_64) echo x86_64-unknown-linux-gnu ;;
    aarch64|arm64) echo aarch64-unknown-linux-gnu ;;
    *) echo "" ;;
  esac
}

install_prebuilt() {
  local t; t=$(arch_triple); [[ -n $t ]] || return 1
  [[ -n $VERSION ]] || return 1
  local name="liftoff-$t.tar.gz" tmp base="$REPO_URL/releases/download/v$VERSION"; tmp=$(mktemp -d)
  # shellcheck disable=SC2064
  trap "rm -rf '$tmp'" RETURN
  # release asset for exactly this checkout's version, checksum required
  curl -fsSL -o "$tmp/$name" "$base/$name" 2>/dev/null || return 1
  curl -fsSL -o "$tmp/$name.sha256" "$base/$name.sha256" 2>/dev/null || return 1
  (cd "$tmp" && sha256sum -c "$name.sha256" >/dev/null) || die "checksum mismatch for $name"
  tar -C "$tmp" -xzf "$tmp/$name"
  install -Dm755 "$tmp/liftoff" "$BIN"
  ok "prebuilt $t binary (release v$VERSION)"
}

build_from_source() {
  have cargo || die "Rust (cargo) is needed to build OmaLiftoff: install the 'rust' package (pacman) or rustup, then re-run"
  local src=$SRC
  # build output goes to the user cache, not into the plugin folder
  local tdir="${XDG_CACHE_HOME:-$HOME/.cache}/liftoff/build"
  info "building $src (a few minutes the first time)"
  if [[ $GUM == 1 ]]; then gum spin --title "cargo build --release" -- env CARGO_TARGET_DIR="$tdir" cargo build --release --locked --manifest-path "$src/Cargo.toml"
  else CARGO_TARGET_DIR="$tdir" cargo build --release --locked --manifest-path "$src/Cargo.toml"; fi
  install -Dm755 "$tdir/release/liftoff" "$BIN"
  ok "built from source"
}

install_binary() {
  say "Installing liftoff → $BIN"
  if [[ $FROM_SOURCE == 0 ]] && have cargo && ! ask_yn "Download the prebuilt v$VERSION binary? (No = build from this checkout with cargo)" y; then FROM_SOURCE=1; fi
  if [[ $FROM_SOURCE == 0 ]] && install_prebuilt; then :; else
    [[ $FROM_SOURCE == 1 ]] || info "no prebuilt v$VERSION release for $(uname -m); building from source"
    build_from_source
  fi
  case ":$PATH:" in *":$BIN_DIR:"*) ;; *) warn "$BIN_DIR is not on your PATH" ;; esac
  # keep a copy of this installer for `liftoff setup` / uninstall
  mkdir -p "$DATA_DIR"
  install -m755 "$SRC/install.sh" "$DATA_DIR/install.sh"
  "$BIN" paths >/dev/null   # writes a default config.toml on first run
}

install_service() {
  say "Background service (systemd --user)"
  mkdir -p "$UNIT_DIR"
  cat >"$UNIT" <<'UNIT'
[Unit]
Description=OmaLiftoff rocket launch tracker (LL2 cache + notifications)
After=network-online.target graphical-session.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=%h/.local/bin/liftoff daemon
Restart=on-failure
RestartSec=30
Nice=10

[Install]
WantedBy=graphical-session.target
UNIT
  systemctl --user daemon-reload
  systemctl --user enable liftoff.service >/dev/null 2>&1 || true
  info "first fetch (uses 1 of the 15 free Launch Library calls this hour)"
  "$BIN" fetch >/dev/null 2>&1 && ok "launch cache written" || warn "first fetch failed; the daemon will retry"
  systemctl --user restart liftoff.service && ok "liftoff.service running" || warn "could not start liftoff.service (journalctl --user -u liftoff)"
}

install_plugin() {
  say "Omarchy bar widget ($PLUGIN_ID)"
  if ! have omarchy; then warn "omarchy CLI not found; skipping the bar widget"; return; fi
  if [[ $SRC == "$PLUGIN_DIR" ]]; then
    # running from the folder `omarchy plugin add` created: just make sure it's enabled
    omarchy plugin enable "$PLUGIN_ID" --yes >/dev/null 2>&1 || true
    ok "plugin folder $PLUGIN_DIR (updates: omarchy plugin update $PLUGIN_ID)"
  elif [[ -d $PLUGIN_DIR/.git ]]; then
    ok "widget already installed via omarchy plugin add (updates: omarchy plugin update $PLUGIN_ID)"
  else
    # dev checkout elsewhere: copy manifest + QML (plugin folders must not contain symlinks)
    rm -rf "$PLUGIN_DIR"; mkdir -p "$PLUGIN_DIR/omarchy-plugin"
    cp "$SRC/manifest.json" "$SRC/LICENSE" "$SRC/README.md" "$SRC/install.sh" "$PLUGIN_DIR/"
    cp "$SRC"/omarchy-plugin/*.qml "$SRC"/omarchy-plugin/*.js "$PLUGIN_DIR/omarchy-plugin/"
    omarchy plugin validate "$PLUGIN_DIR" >/dev/null || die "plugin failed validation: omarchy plugin validate $PLUGIN_DIR"
    omarchy-shell -q shell rescanPlugins >/dev/null 2>&1 || true
    omarchy plugin enable "$PLUGIN_ID" --yes >/dev/null 2>&1 || true
    ok "copied from $SRC and enabled (dev install)"
  fi
  local sec; sec=$(choose "Where should the widget sit in the bar? (left/right show only the rocket icon)" center left center right)
  omarchy bar move "$PLUGIN_ID" --section "$sec" >/dev/null 2>&1 && ok "placed in the $sec section" \
    || info "place it later: omarchy bar move $PLUGIN_ID --section $sec"
}

# Secrets (ntfy topic, token) never go into a command's arguments: other local users can read
# every process's argv via /proc. They travel in shell variables, reach liftoff on stdin
# (`config set-secret`) and are shown to the user only by the printf builtin.
set_secret() { printf '%s\n' "$2" | "$BIN" config set-secret "$1" >/dev/null; }
random_topic() { printf 'liftoff-%s' "$(od -An -N12 -tx1 /dev/urandom | tr -d ' \n')"; }

notify_wizard() {
  say "Notifications"
  local cur_topic cur_on
  cur_topic=$("$BIN" config get notify.ntfy_topic 2>/dev/null || true)
  cur_on=$("$BIN" config get notify.ntfy 2>/dev/null || echo false)
  info "Desktop alerts (notify-send/mako) fire at T-10 min for every launch, and at T-24 h + T-1 h"
  info "for your starred providers. Holds and big slips of launches you were alerted about re-notify."
  if ask_yn "Enable desktop alerts?" y; then "$BIN" config set notify.desktop true >/dev/null; else "$BIN" config set notify.desktop false >/dev/null; fi
  info "'New launch' alerts tell you when a launch is added to the schedule (first run only records what's there;"
  info "more than 3 at once arrive as one summary)."
  local nl; nl=$(choose "New launch alerts for…" "all providers" "all providers" "starred providers only" "off")
  case $nl in
    off) "$BIN" config set notify.new_launch_alerts false >/dev/null ;;
    starred*) "$BIN" config set notify.new_launch_alerts true >/dev/null; "$BIN" config set notify.new_launch_scope starred >/dev/null ;;
    *) "$BIN" config set notify.new_launch_alerts true >/dev/null; "$BIN" config set notify.new_launch_scope all >/dev/null ;;
  esac

  if [[ $cur_on == true && -n $cur_topic ]]; then
    info "Phone push is set up (topic $cur_topic)."
    if ask_yn "Keep the current phone push settings?" y; then return; fi
  fi
  echo
  info "Phone push uses ntfy (https://ntfy.sh): a free, open-source push service. Install the app:"
  info "  Android: https://play.google.com/store/apps/details?id=io.heckel.ntfy"
  info "           https://f-droid.org/en/packages/io.heckel.ntfy/"
  info "  iOS:     https://apps.apple.com/us/app/ntfy/id1625396347"
  if ! ask_yn "Enable phone push via ntfy?" n; then
    "$BIN" config set notify.ntfy false >/dev/null; ok "phone push off (run 'liftoff setup' any time)"; return
  fi
  local server topic token="" how
  how=$(choose "Which ntfy server?" "ntfy.sh (public, random private topic)" "ntfy.sh (public, random private topic)" "my own server / topic / access token")
  if [[ $how == ntfy.sh* ]]; then
    server="https://ntfy.sh"; topic=$(random_topic)
    info "Public ntfy topics have no password: anyone who knows the name can read them. The random"
    info "name is the only protection, so don't share it."
  else
    server=$(ask "Server URL" "https://ntfy.sh"); server=${server%/}
    # no default shown: a pre-filled value would be passed to gum as an argument
    topic=$(ask "Topic (empty = random private name)" "")
    [[ -n $topic ]] || topic=$(random_topic)
    token=$(ask "Access token (empty for none)" "" --password)
  fi
  [[ $topic =~ ^[A-Za-z0-9_-]{1,64}$ ]] || die "invalid topic name (letters, digits, - and _; up to 64)"
  # the server URL is not secret, but must not smuggle credentials (user:pass@host) into argv
  [[ $server =~ ^https?://[^/@[:space:]]+(/[^@[:space:]]*)?$ ]] || die "invalid server URL (http(s)://host[/path], no user:password@)"
  "$BIN" config set notify.ntfy_server "$server" >/dev/null
  set_secret notify.ntfy_topic "$topic"
  set_secret notify.ntfy_token "$token"
  "$BIN" config set notify.ntfy true >/dev/null
  echo
  info "Subscribe in the ntfy app: + → topic '$topic'$([[ $server != https://ntfy.sh ]] && echo " on server $server")"
  info "or scan / open: $server/$topic"
  # URL on stdin, not as an argument
  if have qrencode; then printf '%s' "$server/$topic" | qrencode -t ansiutf8; else info "(install qrencode to get a QR code here)"; fi
  ask_yn "Subscribed? Send a test push now?" y || return 0
  while :; do
    if "$BIN" test-notify --ntfy >/dev/null 2>&1; then ok "test push sent"; else warn "sending failed ($server)"; fi
    ask_yn "Did it arrive on your phone?" y && { ok "phone push configured"; break; }
    ask_yn "Try again?" y || { warn "kept the settings; check with 'liftoff doctor'"; break; }
  done
}

starred_wizard() {
  say "Starred providers"
  info "Starred providers get extra alerts (T-24 h, T-1 h), priority for the precise T-5 s sync, and a ★ in the TUI."
  local cur sel arr=() p
  cur=$("$BIN" config get starred_providers 2>/dev/null || echo '["SpaceX"]')
  if [[ $INTERACTIVE == 1 && $GUM == 1 ]]; then
    mapfile -t arr < <(gum choose --no-limit --header "Star providers (space to toggle)" --selected "SpaceX" \
      SpaceX "Rocket Lab" "Blue Origin" "United Launch Alliance" Arianespace "Indian Space Research Organization" \
      "China Aerospace Science and Technology Corporation" "Russian Federal Space Agency (ROSCOSMOS)" \
      "Japan Aerospace Exploration Agency" "Firefly Aerospace" <"$TTY" || true)
  else
    sel=$(ask "Comma-separated provider names or abbreviations (current: $cur)" "SpaceX")
    IFS=',' read -r -a arr <<<"$sel"
  fi
  [[ ${#arr[@]} -gt 0 ]] || arr=(SpaceX)
  local toml="[" first=1
  for p in "${arr[@]}"; do p=$(echo "$p" | xargs); [[ -z $p ]] && continue
    p=${p//\\/}; p=${p//\"/}
    [[ $first == 1 ]] || toml+=", "; toml+="\"$p\""; first=0; done
  toml+="]"
  "$BIN" config set starred_providers "$toml" >/dev/null && ok "starred: $toml"
  info "Change later: 's' in the TUI, or liftoff config set starred_providers '[\"SpaceX\"]'"
}

uninstall() {
  say "Uninstalling OmaLiftoff"
  systemctl --user disable --now liftoff.service >/dev/null 2>&1 || true
  rm -f "$UNIT"; systemctl --user daemon-reload 2>/dev/null || true; ok "service removed"
  if have omarchy && { [[ -d $PLUGIN_DIR ]] || omarchy plugin list 2>/dev/null | grep -q "$PLUGIN_ID"; }; then
    omarchy plugin remove "$PLUGIN_ID" --yes >/dev/null 2>&1 || true
  fi
  rm -rf "$PLUGIN_DIR"
  ok "bar widget removed"
  rm -f "$BIN"; ok "binary removed"
  if ask_yn "Also delete config, cache and state (~/.config/liftoff, ~/.cache/liftoff, ~/.local/state/liftoff)?" n; then
    rm -rf "${XDG_CONFIG_HOME:-$HOME/.config}/liftoff" "${XDG_CACHE_HOME:-$HOME/.cache}/liftoff" \
      "${XDG_STATE_HOME:-$HOME/.local/state}/liftoff" "$DATA_DIR"
    ok "data removed"
  else info "kept config/cache (and $DATA_DIR)"; fi
}

case $MODE in
  uninstall) uninstall ;;
  setup)
    [[ -x $BIN ]] || die "liftoff is not installed ($BIN)"
    notify_wizard; starred_wizard
    systemctl --user restart liftoff.service 2>/dev/null || true ;;
  install)
    upgrade=0; [[ -x $BIN ]] && upgrade=1
    # never rewrite an existing config without someone answering the questions
    [[ -f ${XDG_CONFIG_HOME:-$HOME/.config}/liftoff/config.toml && $INTERACTIVE == 0 ]] && upgrade=1
    [[ $GUM == 1 ]] && gum style --border rounded --padding "0 2" --border-foreground 212 "󱓞 OmaLiftoff" "rocket launches in your Omarchy bar"
    check_deps
    install_binary
    install_service
    install_plugin
    if [[ $upgrade == 1 ]] && ! ask_yn "Upgrade done. Re-run the notification/starred setup?" n; then :; else notify_wizard; starred_wizard; fi
    systemctl --user restart liftoff.service 2>/dev/null || true
    say "Done"
    info "liftoff tui        dashboard (? for keys)"
    info "liftoff doctor     health check"
    info "liftoff setup      notifications / starred providers"
    info "bash $SRC/install.sh --uninstall   removes the companion (widget: omarchy plugin remove $PLUGIN_ID)"
    ;;
esac
