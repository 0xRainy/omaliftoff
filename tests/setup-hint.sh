#!/usr/bin/env bash
# The setup wizard's ntfy subscribe hint, for the default ntfy.sh server and a custom server.
# Runs `install.sh --setup` on a pty with a scripted gum stand-in in a throwaway HOME. The test
# push is declined and systemctl is stubbed, so nothing leaves the machine and no service restarts.
#
# usage: tests/setup-hint.sh <path-to-liftoff-binary>   (needs bash, script from util-linux)
set -uo pipefail
BIN_SRC=$(readlink -f "${1:?usage: $0 <liftoff binary>}")
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
command -v script >/dev/null || { echo "setup-hint: script (util-linux) is required" >&2; exit 2; }
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
fail=0
run() {  # $1 = label, $2 = server choice, $3 = custom server URL, $4 = custom topic
  local h="$W/$1"; mkdir -p "$h/home/.local/bin" "$h/shim"
  cp "$BIN_SRC" "$h/home/.local/bin/liftoff"
  printf '#!/bin/sh\nexit 0\n' > "$h/shim/systemctl"
  cat > "$h/shim/gum" <<GUM
#!/usr/bin/env bash
cmd=\$1; shift; hdr=""; sel=""; q=""; args=("\$@")
while [[ \$# -gt 0 ]]; do case \$1 in --header) hdr=\$2; shift ;; --selected) sel=\$2; shift ;; --value|--default=*) [[ \$1 == --value ]] && shift ;; --*) ;; *) q=\$1 ;; esac; shift; done
case \$cmd in
  confirm) case \$q in *"test push"*|*"Keep the current"*) exit 1 ;; *) exit 0 ;; esac ;;
  style) printf '%s\n' "\${args[@]}" ;;
  choose) case \$hdr in *"ntfy server"*) echo "$2" ;; *) echo "\${sel:-SpaceX}" ;; esac ;;
  input) case \$hdr in *"Server URL"*) echo "$3" ;; *Topic*) echo "$4" ;; *) echo "" ;; esac ;;
esac
GUM
  chmod +x "$h/shim/"*
  HOME=$h/home XDG_CONFIG_HOME=$h/home/.config XDG_DATA_HOME=$h/home/.local/share XDG_CACHE_HOME=$h/home/.cache \
    XDG_STATE_HOME=$h/home/.local/state PATH="$h/shim:$PATH" \
    script -qec "bash '$REPO/install.sh' --setup" /dev/null > "$h/out" 2>&1 </dev/null
  sed 's/\x1b\[[0-9;]*m//g' "$h/out" > "$h/plain"
}
expect() {  # $1 = label, $2 = file, $3 = regex (grep -E), $4 = "yes"|"no"
  if grep -qE -- "$3" "$2"; then got=yes; else got=no; fi
  if [[ $got == "$4" ]]; then echo "PASS  $1"; else echo "FAIL  $1 (want match=$4 for /$3/)"; sed 's/^/      /' "$2" | grep -iE 'ntfy|topic|server|subscri' | head -8; fail=1; fi
}
run default "ntfy.sh (public, random private topic)" "" ""
expect "default: subscribe hint names the random topic" "$W/default/plain" "Subscribe in the ntfy app using topic: 'liftoff-[0-9a-f]{24}'" yes
expect "default: no 'add the server' step for ntfy.sh" "$W/default/plain" "add the server" no
run custom "my own server / topic / access token" "https://ntfy.example.test" "custom-topic_1"
expect "custom: hint names the server and the topic" "$W/custom/plain" "In the ntfy app, add the server https://ntfy.example.test, then subscribe using topic: 'custom-topic_1'" yes
expect "both: open/scan URL is server/topic" "$W/custom/plain" "or scan / open: https://ntfy.example.test/custom-topic_1" yes
[[ $fail == 0 ]] && echo "setup-hint: OK"
exit $fail
