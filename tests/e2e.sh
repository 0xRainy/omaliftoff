#!/usr/bin/env bash
# End-to-end checks of the REAL liftoff binary: fake LL2 + mock ntfy (tests/fake_ll2.py), a
# recording notify-send stand-in, and a faked clock (libfaketime: no product hook, so the same
# script also runs against old binaries to prove a check catches its bug). Offline: every URL
# the binary uses points at 127.0.0.1 (news and SpaceX content are switched off).
#
# usage: tests/e2e.sh <path-to-liftoff-binary>   (needs bash, python3, libfaketime)
set -uo pipefail
BIN=$(readlink -f "${1:?usage: $0 <liftoff binary>}")
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
FT=""
for f in /usr/lib/x86_64-linux-gnu/faketime/libfaketime.so.1 /usr/lib/aarch64-linux-gnu/faketime/libfaketime.so.1 /usr/lib/faketime/libfaketime.so.1; do
  [[ -f $f ]] && { FT=$f; break; }
done
[[ -n $FT ]] || { echo "e2e: libfaketime is required (package: faketime / libfaketime)" >&2; exit 2; }
W=$(mktemp -d); trap 'kill "${MOCK:-0}" 2>/dev/null || true; rm -rf "$W"' EXIT
mkdir -p "$W/srv" "$W/shim"
python3 "$HERE/fake_ll2.py" "$W/srv" & MOCK=$!
while [[ ! -s $W/srv/port ]]; do kill -0 "$MOCK" 2>/dev/null || { echo "e2e: fake LL2 failed to start" >&2; exit 2; }; sleep 0.05; done
PORT=$(<"$W/srv/port"); API="http://127.0.0.1:$PORT"
# shellcheck disable=SC2016  # literal shim body; only the log path is substituted
printf '#!/bin/sh\nprintf "%%s\\n" "NOTIFY $*" >> %q\n' "$W/notify.log" > "$W/shim/notify-send"
printf '#!/bin/sh\nexit 0\n' > "$W/shim/xdg-open"; chmod +x "$W/shim/"*
export PATH="$W/shim:$PATH"
unset LIFTOFF_API

pass=0; fail=0
check() {  # $1 = name, $2 = expected substring/regex (grep -E), $3 = actual
  if grep -qE -- "$2" <<<"$3"; then pass=$((pass + 1)); echo "PASS  $1"
  else fail=$((fail + 1)); echo "FAIL  $1"; echo "      want /$2/"; echo "      got  $(head -c 400 <<<"$3" | tr '\n' ' ')"; fi
}
fresh() {  # new throwaway HOME pointed at the fakes; $1 = NET, $2 = precision
  local h; h=$(mktemp -d "$W/home.XXXX")
  export HOME=$h XDG_CONFIG_HOME=$h/.config XDG_CACHE_HOME=$h/.cache XDG_STATE_HOME=$h/.local/state XDG_DATA_HOME=$h/.local/share
  printf '%s' "$1" > "$W/srv/net.txt"; printf '%s' "${2:-MIN}" > "$W/srv/prec.txt"; rm -f "$W/srv/fail.txt" "$W/srv/status.txt"
  : > "$W/notify.log"; : > "$W/srv/ntfy.log"
  "$BIN" config set api_base "$API" >/dev/null
  "$BIN" config set spacex_flight_plans false >/dev/null
  "$BIN" config set news_for_next 0 >/dev/null
  "$BIN" config set notify.ntfy_server "$API" >/dev/null
  printf 'e2e-topic\n' | "$BIN" config set-secret notify.ntfy_topic >/dev/null
  "$BIN" config set notify.ntfy true >/dev/null
}
# at <TZ> <UTC time "YYYY-MM-DD HH:MM:SS"> <cmd...>: run with a frozen clock (@ prefix: ticking from there)
at() {
  local tz=$1 t=$2 tick=""; shift 2
  [[ $t == @* ]] && { tick=@; t=${t#@}; }
  TZ=$tz LD_PRELOAD=$FT FAKETIME="$tick$(TZ=$tz date -d "$t UTC" '+%Y-%m-%d %H:%M:%S')" FAKETIME_DONT_FAKE_MONOTONIC=1 "$@"
}
notifies() { grep -c '^NOTIFY' "$W/notify.log" || true; }
posts() { grep -c '^POST' "$W/srv/ntfy.log" || true; }

echo "== NET text in UTC / Phoenix / Tokyo (day labels are UTC calendar dates; 0.1.5 Phoenix day-early fix)"
fresh 2026-11-07T00:00:00Z DAY
at UTC "2026-11-01 12:00:00" "$BIN" fetch >/dev/null 2>&1
for tz in UTC America/Phoenix Asia/Tokyo; do check "DAY NET reads Sat Nov 7 in $tz" 'NET Sat Nov 7$' "$(at "$tz" "2026-11-01 12:00:00" "$BIN" status)"; done
fresh 2026-10-20T18:00:00Z MIN
at UTC "2026-10-19 12:00:00" "$BIN" fetch >/dev/null 2>&1
for p in "UTC|18:00" "America/Phoenix|11:00" "Asia/Tokyo|03:00"; do
  check "MIN NET T-1h alert shows local clock in ${p%%|*}" "Liftoff in 59m \(${p#*|}\)" "$(at "${p%%|*}" "2026-10-20 17:01:00" "$BIN" alerts --dry 2>&1)"
done

echo "== LL2 is the ground truth: a listed, unfinished launch is never hidden (B1)"
fresh 2026-10-14T00:00:00Z DAY
at UTC "2026-10-13 12:00:00" "$BIN" fetch >/dev/null 2>&1
check "DAY launch shown 00:30Z+1m on its day (Phoenix evening before)" 'NET Wed Oct 14$' "$(at America/Phoenix "2026-10-14 00:31:00" "$BIN" status)"
check "DAY launch shown at 23:59Z on its day" 'NET Wed Oct 14$' "$(at UTC "2026-10-14 23:59:00" "$BIN" status)"
check "DAY launch after its day, still listed: shown, awaiting update" 'NET Wed Oct 14 · awaiting update$' "$(at UTC "2026-10-15 00:01:00" "$BIN" status)"
fresh 2026-10-14T18:00:00Z MIN
at UTC "2026-10-14 12:00:00" "$BIN" fetch >/dev/null 2>&1
check "MIN launch 10 min past NET: T+ count" 'T\+00:10:00$' "$(at UTC "2026-10-14 18:10:00" "$BIN" status)"
at UTC "2026-10-16 18:00:00" "$BIN" fetch >/dev/null 2>&1   # LL2 still lists it two days later
check "MIN launch 2 days past NET, still listed: shown, awaiting update" 'NET Wed Oct 14 [0-9:]+ · awaiting update$' "$(at UTC "2026-10-16 18:00:00" "$BIN" status)"
check "status --json carries it too" '"text":"[^"]*awaiting update"' "$(at UTC "2026-10-16 18:00:00" "$BIN" status --json)"
check "overdue launch: no alert storm" '^0 alert' "$(at UTC "2026-10-16 18:00:00" "$BIN" alerts 2>/dev/null)"
echo 3 > "$W/srv/status.txt"   # LL2 marks it Launch Successful (status id 3)
at UTC "2026-10-16 18:30:00" "$BIN" fetch >/dev/null 2>&1
check "finished per LL2 (status 3): no longer shown" '—$' "$(at UTC "2026-10-16 18:30:00" "$BIN" status)"

echo "== B2: config set refuses a secret in argv without creating the config"
h=$(mktemp -d "$W/b2.XXXX")
out=$(HOME=$h XDG_CONFIG_HOME=$h/.config "$BIN" config set notify.ntfy_topic argv-sentinel 2>&1); rc=$?
check "secret in argv refused (exit != 0)" '^[1-9]' "$rc"
check "no config.toml created by the refusal" '^absent$' "$([[ -e $h/.config/liftoff/config.toml ]] && echo present || echo absent)"
check "refusal does not echo the value" '^ok$' "$(grep -q argv-sentinel <<<"$out" && echo leaked || echo ok)"

echo "== alert thresholds fire once (desktop + ntfy), restart dedupe"
fresh 2026-10-20T18:00:00Z MIN
at UTC "2026-10-19 17:00:00" "$BIN" fetch >/dev/null 2>&1
check "first fetch seeds new-launch alerts silently" '^0 alert' "$(at UTC "2026-10-19 17:00:00" "$BIN" alerts 2>/dev/null)"
check "T-24h alert" 'T-24h' "$(at UTC "2026-10-19 18:30:00" "$BIN" alerts 2>&1)"
check "T-1h alert" 'T-1h' "$(at UTC "2026-10-20 17:01:00" "$BIN" alerts 2>&1)"
check "T-10m alert" 'T-10m' "$(at UTC "2026-10-20 17:51:00" "$BIN" alerts 2>&1)"
check "re-run at the same time: nothing new" '^0 alert' "$(at UTC "2026-10-20 17:51:30" "$BIN" alerts 2>/dev/null)"
check "3 desktop notifications" '^3$' "$(notifies)"
check "3 ntfy pushes to the mock" '^3$' "$(posts)"
check "ntfy push carries the topic" '"topic":"e2e-topic"' "$(cat "$W/srv/ntfy.log")"
check "T-10m is critical on the desktop" '^NOTIFY .*-u critical .*T-10m' "$(cat "$W/notify.log")"
at UTC "@2026-10-20 17:52:00" timeout 4 "$BIN" daemon > "$W/d1.log" 2>&1
at UTC "@2026-10-20 17:53:00" timeout 4 "$BIN" daemon > "$W/d2.log" 2>&1
check "daemon restarts send nothing new" '^3$' "$(notifies)"
echo 2026-10-20T21:00:00Z > "$W/srv/net.txt"
at UTC "2026-10-20 17:54:00" "$BIN" fetch >/dev/null 2>&1
check "3h slip: one Delayed alert" 'Delayed' "$(at UTC "2026-10-20 17:54:00" "$BIN" alerts 2>&1)"
check "Delayed alert not repeated" '^0 alert' "$(at UTC "2026-10-20 17:54:30" "$BIN" alerts 2>/dev/null)"

echo "== HR precision: approximate T-10 + reserved final syncs"
fresh 2026-10-20T18:00:00Z HR
at UTC "2026-10-20 17:00:00" "$BIN" fetch >/dev/null 2>&1
check "HR T-10 alert, worded approximately" '~T-10m .*~9m \(~18:00 ±1h\)' "$(at UTC "2026-10-20 17:51:00" "$BIN" alerts --dry 2>&1)"
check "HR launch gets the 4 reserved syncs" '^4 T-5m' "$(python3 -c 'import json,sys; s=json.load(open(sys.argv[1]))["sync"]; print(s.get("reserved"), s.get("next_label"))' "$XDG_CACHE_HOME/liftoff/launches.json")"
: > "$W/srv/requests.log"
at UTC "@2026-10-20 17:54:58" timeout 4 "$BIN" daemon > "$W/d3.log" 2>&1
check "daemon started 2 s before T-5m runs the sync on time" 'sync T-5m .*: ok' "$(cat "$W/d3.log")"
check "sync hit /launches/e2e-1/" '^/launches/e2e-1/' "$(cat "$W/srv/requests.log")"

echo "== reserved syncs are never overslept (daemon started 1-3 s before each slot; already running across it; clock jumps)"
# A loop that wakes a hair early must not skip the due slot and sleep 15 s (that put T-5s after liftoff).
# Each run gets 2.5 s of real time after its slot: an on-time sync needs well under 1 s.
sync_run() {  # $1 = slot label, $2 = slot UTC time, $3 = seconds before it to start the daemon
  fresh 2026-10-20T18:00:00Z MIN
  "$BIN" config set notify.desktop false >/dev/null; "$BIN" config set notify.ntfy false >/dev/null
  at UTC "2026-10-20 17:00:00" "$BIN" fetch >/dev/null 2>&1
  local start; start=$(date -u -d "$2 UTC - $3 sec" '+%Y-%m-%d %H:%M:%S')
  at UTC "@$start" timeout "$(( $3 + 2 )).5" "$BIN" daemon > "$W/sync.log" 2>&1
  grep -E "^sync $1 .*: ok" "$W/sync.log" || tail -3 "$W/sync.log"
}
for slot in "T-5m|2026-10-20 17:55:00" "T-60s|2026-10-20 17:59:00" "T-5s|2026-10-20 17:59:55" "T\+90s|2026-10-20 18:01:30"; do
  for k in 1 2 3; do
    check "${slot%%|*} sync on time, daemon started ${k}s before" "^sync ${slot%%|*} .*: ok" "$(sync_run "${slot%%|*}" "${slot#*|}" "$k")"
  done
done
check "daemon already running 20 s across T-5m syncs it" '^sync T-5m .*: ok' "$(sync_run T-5m "2026-10-20 17:55:00" 20)"
# suspend/resume: the fake clock (offset file, re-read on every call) jumps while the daemon sleeps
jump_run() {  # $1 = fake start (UTC), $2 = seconds to jump forward 1 s in, $3 = real seconds to run
  fresh 2026-10-20T18:00:00Z MIN
  "$BIN" config set notify.desktop false >/dev/null; "$BIN" config set notify.ntfy false >/dev/null
  at UTC "2026-10-20 17:00:00" "$BIN" fetch >/dev/null 2>&1
  local off; off=$(( $(date -u -d "$1 UTC" +%s) - $(date +%s) ))
  printf '%+d' "$off" > "$W/ft"
  TZ=UTC LD_PRELOAD=$FT FAKETIME_TIMESTAMP_FILE="$W/ft" FAKETIME_NO_CACHE=1 FAKETIME_DONT_FAKE_MONOTONIC=1 \
    timeout "$3" "$BIN" daemon > "$W/jump.log" 2>&1 &
  local pid=$!; sleep 1; printf '%+d' "$(( off + $2 ))" > "$W/ft"; wait "$pid"
  cat "$W/jump.log"
}
out=$(jump_run "2026-10-20 17:54:40" 60 4)   # suspended across T-5m, resumed inside its 3-min validity
check "resume after T-5m (still valid): sync runs right away" '^sync T-5m .*: ok' "$out"
out=$(jump_run "2026-10-20 17:54:40" 255 6)  # resumed past T-5m validity, 2 s before T-60s
check "resume past T-5m validity: next sync (T-60s) still on time" '^sync T-60s .*: ok' "$out"

echo "== LL2 budget over a launch window (T-65m..T+5m at 120x speed): <=14 calls/rolling hour, all 4 syncs"
fresh 2026-10-20T18:00:00Z MIN
"$BIN" config set notify.desktop false >/dev/null; "$BIN" config set notify.ntfy false >/dev/null
: > "$W/srv/requests.tlog"; t0=$(date +%s.%N)
TZ=UTC LD_PRELOAD=$FT FAKETIME="@$(date -u -d '2026-10-20 16:55:00 UTC' '+%Y-%m-%d %H:%M:%S') x120" timeout 36 "$BIN" daemon > "$W/win.log" 2>&1
win=$(python3 -c '
import sys
t0 = float(sys.argv[2]); base = 16 * 3600 + 55 * 60   # fake seconds of day at start
ts = [base + (float(l.split(" ", 1)[0]) - t0) * 120 for l in open(sys.argv[1]) if l.split(" ", 1)[1].startswith("/launches")]
print("max/h", max(sum(1 for u in ts if t - 3600 < u <= t) for t in ts), "total", len(ts))
' "$W/srv/requests.tlog" "$t0")
check "LL2 calls per rolling hour stay <= 14 ($win)" '^max/h ([0-9]|1[0-4]) ' "$win"
for k in "T-5m" "T-60s" "T-5s" "T\+90s"; do check "$k sync ran in the window" "^sync $k .*: ok" "$(cat "$W/win.log")"; done
check "no refresh held/refused in the window" '^ok$' "$(grep -qE 'held|refused|exhausted' "$W/win.log" && echo held || echo ok)"

echo "== failed fetch is reported as failed"
fresh 2026-10-20T18:00:00Z MIN
at UTC "2026-10-19 10:00:00" "$BIN" fetch >/dev/null 2>&1
touch "$W/srv/fail.txt"
out=$(at UTC "2026-10-19 10:42:00" "$BIN" fetch 2>&1); rc=$?
check "fetch exits non-zero on LL2 503" '^[1-9]' "$rc"
check "fetch says failed, keeps the cache" 'fetch failed: .*503.*keeping 1 cached launches \(fetched 42m ago\)' "$out"
at UTC "@2026-10-19 10:50:00" timeout 3 "$BIN" daemon > "$W/d4.log" 2>&1
check "daemon logs the failure, not 'fetched N'" 'fetch failed: .*keeping 1 cached launches' "$(cat "$W/d4.log")"
check "daemon never claims a fetch on failure" '^ok$' "$(grep -q 'fetched 1 launches' "$W/d4.log" && echo claimed || echo ok)"
check "status still served from cache" 'Falcon 9 · T-' "$(at UTC "2026-10-19 10:50:00" "$BIN" status)"

echo
echo "e2e: $pass passed, $fail failed"
[[ $fail == 0 ]]
