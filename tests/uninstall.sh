#!/usr/bin/env bash
# install.sh --uninstall keeps config/cache/state with --yes, and --purge removes everything
# (no prompt). Throwaway HOME; systemctl/omarchy/cargo are stubbed (cargo "builds" by copying
# the given binary), so nothing outside that HOME is touched and nothing goes on the network.
#
# usage: tests/uninstall.sh <path-to-liftoff-binary>
# ok/bad always succeed, so `cond && ok .. || bad ..` is intended.
# shellcheck disable=SC2015,SC2001
set -uo pipefail
BIN_SRC=$(readlink -f "${1:?usage: $0 <liftoff binary>}")
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
export HOME="$W/home" XDG_CONFIG_HOME="$W/home/.config" XDG_CACHE_HOME="$W/home/.cache" \
  XDG_DATA_HOME="$W/home/.local/share" XDG_STATE_HOME="$W/home/.local/state" LIFTOFF_API="http://127.0.0.1:9"
mkdir -p "$HOME" "$W/shim"
for c in systemctl omarchy notify-send; do printf '#!/bin/sh\nexit 0\n' > "$W/shim/$c"; done
# shellcheck disable=SC2016  # literal shim body; only the binary path is substituted
printf '#!/bin/sh\nmkdir -p "$CARGO_TARGET_DIR/release" && cp %q "$CARGO_TARGET_DIR/release/liftoff"\n' "$BIN_SRC" > "$W/shim/cargo"
chmod +x "$W/shim/"*; export PATH="$W/shim:$PATH"
fail=0
ok() { echo "PASS  $1"; }
bad() { echo "FAIL  $1"; fail=1; }
install() {
  bash "$REPO/install.sh" --yes --from-source </dev/null >"$W/install.out" 2>&1 || { bad "install failed"; sed 's/^/    /' "$W/install.out"; exit 1; }
  printf 'purge-sentinel-topic\n' | "$HOME/.local/bin/liftoff" config set-secret notify.ntfy_topic >/dev/null
  printf 'purge-sentinel-token\n' | "$HOME/.local/bin/liftoff" config set-secret notify.ntfy_token >/dev/null
  "$HOME/.local/bin/liftoff" alerts >/dev/null 2>&1 || true   # writes state.json
  "$HOME/.local/bin/liftoff" refresh                          # writes into the cache dir
}
leftovers() { find "$HOME" -mindepth 1 \( -iname '*liftoff*' -o -type f \) -print | sort; grep -rlF -e purge-sentinel "$HOME" 2>/dev/null; }

install
[[ -n $(leftovers) ]] && ok "install created files" || bad "install created nothing?"
bash "$REPO/install.sh" --uninstall --yes </dev/null >"$W/u1.out" 2>&1 || bad "uninstall --yes failed"
[[ ! -e $HOME/.local/bin/liftoff ]] && ok "--yes: binary removed" || bad "--yes: binary left"
[[ ! -e $XDG_CONFIG_HOME/systemd/user/liftoff.service ]] && ok "--yes: unit removed" || bad "--yes: unit left"
[[ -f $XDG_CONFIG_HOME/liftoff/config.toml ]] && ok "--yes alone keeps config (unchanged behaviour)" || bad "--yes deleted the config"

install
bash "$REPO/install.sh" --uninstall --purge </dev/null >"$W/u2.out" 2>&1 || bad "uninstall --purge failed"
left=$(leftovers)
if [[ -z $left ]]; then ok "--purge: nothing left (no liftoff paths, no files, no secrets)"; else bad "--purge left:"; sed 's/^/    /' <<<"$left"; fi
bash "$REPO/install.sh" --purge </dev/null >/dev/null 2>&1 && bad "--purge without --uninstall accepted" || ok "--purge without --uninstall is refused"
[[ $fail == 0 ]] && echo "uninstall: OK"
exit $fail
