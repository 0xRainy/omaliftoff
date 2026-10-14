#!/usr/bin/env bash
# Regression check: secrets entered in the installer's ntfy wizard must never reach another
# process's argv or environment (both readable via /proc by other local users), any log, or
# any file other than the 0600 config.
#
# Then runs doctor/test-notify/status/alerts (also with a broken config line) the same way.
# Runs `install.sh --setup` twice (fresh setup, then "keep existing") in a throwaway HOME under
# `strace -f -v -e trace=execve`, with a scripted `gum` stand-in answering the prompts and a
# local mock ntfy server for the test push. Fails if a sentinel topic/token shows up anywhere.
#
# usage: tests/secret-leak.sh <path-to-liftoff-binary>   (needs bash, strace, python3)
set -euo pipefail
BIN_SRC=$(readlink -f "${1:?usage: $0 <liftoff binary>}")
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
command -v strace >/dev/null || { echo "secret-leak: strace is required" >&2; exit 2; }
command -v script >/dev/null || { echo "secret-leak: script (util-linux) is required" >&2; exit 2; }

TOPIC="leaksentinel-topic-$RANDOM$RANDOM"
TOKEN="leaksentinel-token-$RANDOM$RANDOM"
W=$(mktemp -d); trap 'kill "${MOCK:-0}" 2>/dev/null || true; rm -rf "$W"' EXIT
export HOME="$W/home" XDG_CONFIG_HOME="$W/home/.config" XDG_DATA_HOME="$W/home/.local/share" \
  XDG_CACHE_HOME="$W/home/.cache" XDG_STATE_HOME="$W/home/.local/state"
mkdir -p "$HOME/.local/bin" "$W/shim"
cp "$BIN_SRC" "$HOME/.local/bin/liftoff"

# mock ntfy server (test push target); records what it receives. It binds port 0 (the kernel
# picks a free port, so no collision with other listeners) and only then writes the port to
# $W/port: the file appearing means the socket is already listening, so the wizard's test push
# can't race the server's startup.
python3 - "$W/port" "$W/ntfy.log" <<'PY' &
import http.server, os, sys
portfile, log = sys.argv[1], sys.argv[2]
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0)); body = self.rfile.read(n)
        open(log, "ab").write(self.path.encode() + b" " + body + b"\n")
        self.send_response(200); self.end_headers(); self.wfile.write(b"{}")
    do_GET = do_POST
srv = http.server.HTTPServer(("127.0.0.1", 0), H)   # listening from here on
with open(portfile + ".tmp", "w") as f: f.write(str(srv.server_address[1]))
os.rename(portfile + ".tmp", portfile)              # atomic: never a half-written port
srv.serve_forever()
PY
MOCK=$!
# wait for the readiness handshake (or the mock dying), never a fixed sleep
while [[ ! -s $W/port ]]; do
  kill -0 "$MOCK" 2>/dev/null || { echo "secret-leak: mock ntfy server failed to start" >&2; exit 2; }
  sleep 0.05   # poll interval only; the condition is the port file
done
PORT=$(<"$W/port")

# scripted gum: answers by prompt header; never needs a terminal
cat > "$W/shim/gum" <<GUM
#!/usr/bin/env bash
cmd=\$1; shift
hdr=""; pw=0; sel=""; args=("\$@")
while [[ \$# -gt 0 ]]; do case \$1 in
  --header) hdr=\$2; shift ;; --password) pw=1 ;; --selected) sel=\$2; shift ;; --value) shift ;; esac; shift; done
case \$cmd in
  confirm) exit 0 ;;
  style) printf '%s\n' "\${args[@]}" ;;
  choose) case \$hdr in
      *"ntfy server"*) echo "my own server / topic / access token" ;;
      *) echo "\${sel:-SpaceX}" ;; esac ;;
  input) if [[ \$pw == 1 ]]; then echo "$TOKEN"
    else case \$hdr in
      *"Server URL"*) echo "http://127.0.0.1:$PORT" ;;
      *Topic*) echo "$TOPIC" ;;
      *) echo "" ;; esac; fi ;;
esac
GUM
chmod +x "$W/shim/gum"
export PATH="$W/shim:$PATH"

run() {  # $1 = label; runs the wizard on a pty so the installer is interactive
  script -qec "strace -f -v -s 65536 -e trace=execve -o '$W/trace.$1' bash '$REPO/install.sh' --setup" /dev/null \
    > "$W/out.$1" 2>&1 </dev/null || { echo "secret-leak: installer failed ($1)"; cat "$W/out.$1"; exit 1; }
}
run fresh
run keep

# the other commands that read the secrets: doctor (ntfy health check), test-notify (desktop
# notify-send + ntfy push), status, alerts. A recording notify-send/xdg-open stand-in captures
# their argv too. Then the same with a broken config line holding the topic: parse errors
# (shown on the terminal and, from the daemon, in the journal) must not quote it.
for c in notify-send xdg-open; do printf '#!/bin/sh\nprintf "%%s\\n" "$0" "$@" >> %q\n' "$W/desktop.argv" > "$W/shim/$c"; chmod +x "$W/shim/$c"; done
cmds() {  # $1 = label
  local c
  for c in doctor "test-notify" "test-notify --ntfy" "test-notify --desktop" "status --json" "alerts --dry"; do
    # shellcheck disable=SC2086  # $c is a fixed word list
    LIFTOFF_API="http://127.0.0.1:$PORT" strace -f -v -s 65536 -e trace=execve -o "$W/trace.cmd.$1.${c// /_}" \
      "$HOME/.local/bin/liftoff" $c > "$W/out.cmd.$1.${c// /_}" 2>&1 || true
  done
}
cmds ok
cfg="$XDG_CONFIG_HOME/liftoff/config.toml"
cp "$cfg" "$W/cfg.bak"
sed -i -E 's/^(ntfy_topic = .*)$/\1 broken/' "$cfg"
grep -qF "$TOPIC\" broken" "$cfg" || { echo "secret-leak: could not break the config"; exit 1; }
cmds broken
cp "$W/cfg.bak" "$cfg"; rm -f "$W/cfg.bak"

fail=0
check() {  # $1 = what, $2 = file
  for s in "$TOPIC" "$TOKEN"; do
    if grep -qF "$s" "$2"; then
      echo "LEAK: ${s%%-*}-${s#*-} found in $1:"; grep -nF "$s" "$2" | cut -c1-300 | sed 's/^/    /'
      fail=1
    fi
  done
}
for t in fresh keep; do check "argv/env of child processes (strace execve, $t run)" "$W/trace.$t"; done
for f in "$W"/trace.cmd.* "$W"/out.cmd.*; do check "${f##*/} (argv/env or terminal output of liftoff ${f##*cmd.})" "$f"; done
[[ -s $W/desktop.argv ]] && check "notify-send/xdg-open argv" "$W/desktop.argv"
grep -qs "OmaLiftoff" "$W/desktop.argv" || { echo "secret-leak: test-notify --desktop did not call notify-send"; fail=1; }
grep -qs "line [0-9]*, column" "$W/out.cmd.broken.status_--json" "$W/out.cmd.broken.alerts_--dry" || { echo "secret-leak: broken config was not reported"; fail=1; }
# files: only the config may contain them, and it must be 0600
if ! { grep -qF "$TOPIC" "$cfg" && grep -qF "$TOKEN" "$cfg"; }; then echo "secret-leak: wizard did not store the secrets"; fail=1; fi
mode=$(stat -c %a "$cfg"); [[ $mode == 600 ]] || { echo "LEAK: $cfg is mode $mode, want 600"; fail=1; }
while IFS= read -r f; do [[ $f == "$cfg" ]] || check "file $f" "$f"; done < <(grep -rlF -e "$TOPIC" -e "$TOKEN" "$HOME" 2>/dev/null || true)
# the token must never be printed; the topic is shown to the user once (they need it to subscribe)
for t in fresh keep; do grep -qF "$TOKEN" "$W/out.$t" && { echo "LEAK: token printed to the terminal ($t)"; fail=1; }; done
grep -qsF "$TOPIC" "$W/ntfy.log" || { echo "secret-leak: test push did not reach the mock ntfy server (port $PORT); installer output:"; sed 's/^/    /' "$W/out.fresh"; fail=1; }

if [[ $fail == 0 ]]; then echo "secret-leak: OK (no secret in argv, env, logs or other files; config 0600)"; fi
exit $fail
