# Changelog

## 0.1.5 (2026-10-14)

**Security**
- The installer no longer passes your private ntfy topic as a command argument (to
  `liftoff config set`, `qrencode` and a `gum` prompt), where any local user could read it
  from the process list. Secrets now go through `liftoff config set-secret <key>` on stdin;
  `config set` refuses `notify.ntfy_topic`, `notify.ntfy_token` and `api_key`. If you share
  this machine with other users, consider picking a new topic (`liftoff setup`).
- `config.toml` is now created owner-only (0600) and replaced atomically, instead of being
  written and chmodded afterwards.
- A config file with a syntax error no longer has its offending line quoted in error messages
  (`test-notify`, `alerts`, `status`; terminal and journal). That line could be the ntfy topic or
  token. Errors now give the line and column only.
- The bar widget and panel render every launch, pad, weather and error string as plain text.
  With Qt's default text format, a launch name like `<img src=http://…>` from the API was
  rendered as HTML and the image was fetched.
- The widget's files (cache, nudge, test marker) are size-checked (2 MiB) before parsing, and
  `liftoff refresh` is run as an argv array, not through a shell with the unquoted
  `liftoffBinary` setting.
- CI actions are pinned to commit SHAs.

Fixes.

- **No T-10 alert and no reserved syncs for hour-precision launches**: LL2 often keeps NOTAM-window
  launches (e.g. Zhuque-2E) at net_precision "HR" until after liftoff. The planner only reserved
  final syncs for SEC/MIN, and lead alerts at T-30 or sooner required minute precision, so those
  launches got neither a T-10 alert nor T-5m/T-60s/T-5s/T+90s syncs — while the 4-minute routine
  refresh near T-0 burned the whole hourly budget. HR launches now get reserved syncs and
  approximate lead alerts ("~T-10m", "Liftoff in ~10m (~04:40 ±1h)").
- **Coarse NETs showed fake or shifted dates**: LL2 sends Q1–Q4, WK, AM, PM, FY and DEC, but OmaLiftoff only
  knew made-up codes ("Q", "W", ...), so e.g. a Q4 launch read "NET ~Dec 30". And because day-and-coarser
  NETs (calendar labels LL2 stores as a 00:00 UTC timestamp) were converted to local time like instants,
  dates shifted by a day depending on your time zone (LL2's Nov 7 read "Nov 6" in Phoenix). All NET text
  now comes from one formatter (daemon/TUI and widget): local time only for minute/hour precision, LL2's
  calendar date for everything coarser, identical in every time zone. Coarse NETs never show a countdown.
  AM/PM, week, fiscal-year and decade NETs are shown as approximate ("~") since LL2 data doesn't confirm
  their encoding.
- The panel's NET line no longer shows a fake time plus the raw precision code.
- **A failed LL2 fetch was logged as a success**: on a timeout the daemon wrote
  "fetched 40 launches (…timed out…)", counting the launches it still had in the cache. It now writes
  "fetch failed: <reason> (GET <url>); keeping 40 cached launches (fetched 42m ago)", and `liftoff fetch` exits
  non-zero with the same line. The TUI's manual refresh shows it as a toast, and doctor says "last successful
  fetch … ; latest attempt failed: …". Successful fetches are logged as before. The cache's fetch time only
  moves on success (unchanged), and a failed attempt still counts against the local hourly budget, since LL2
  may have counted it.
- **Day-precision (and coarser) launches vanished from the bar a day early**: LL2 stores a launch
  it only knows to the day as 00:00 UTC of that day, and the "launch is over" check treated that as
  a liftoff time, 30 minutes after which the launch was dropped. A launch "NET Oct 14" left the bar,
  `liftoff status` and the TUI at 00:30 UTC Oct 14, which is 17:30 on Oct 13 in Phoenix. One rule
  (daemon/TUI and widget, with test cases shared by both) now keeps day-and-coarser launches current
  through the end of their labelled UTC period: the day, month, quarter or half named by LL2's code
  (Q1 NETs also sit on Jan 31 and Feb 28 in LL2 data), or the year. Week, fiscal-year and decade NETs
  (no LL2 samples) are kept longest. Such launches also no longer speed up the refresh pace, or make the
  bar icon pulse and the TUI countdown blink, around 00:00 UTC.
- `liftoff config set` with a secret key is refused before the config is read, so the refusal no
  longer creates a default `config.toml`. `liftoff config` edits the file as stored and no longer
  writes a `LIFTOFF_API` override into it.
- HTTP errors (LL2, news, ntfy) no longer print the URL twice; the reason comes first, then the request once.
- The setup wizard's ntfy subscribe hint is simpler: "Subscribe in the ntfy app using topic: '<topic>'"
  (plus a QR code); for a custom server it also names the server to add in the app.
- When the setup wizard's test push fails, it now says why (e.g. "Connection refused",
  "status code 401").
- `install.sh --uninstall --purge` removes everything without asking: binary, service, widget,
  and config (including the ntfy topic and token), cache, state and the saved installer.
  `--uninstall --yes` still keeps your data. `install.sh --help` now lists every option.
- Tests: `tests/e2e.sh` drives the real binary against a fake LL2 and a mock ntfy with a faked clock
  (libfaketime). It covers NET text in UTC, Phoenix and Tokyo, the T-24h/T-1h/T-10m alerts firing once
  (desktop + ntfy) across daemon restarts, slips, hour-precision alerts and syncs, failed-fetch
  reporting, and both fixes above. `tests/setup-hint.sh` checks the wizard's subscribe hint for ntfy.sh
  and for a custom server, and `tests/uninstall.sh` checks `--yes` (data kept) and `--purge` (nothing left). All run in CI.
- README: `curl` is only used by the installer's prebuilt download, not for ntfy.

## 0.1.4 (2026-10-13)

Fixes.

- **Lead, delayed and hold alerts showed a fake time for coarse NETs**: a launch LL2 only knows to the day
  got e.g. "Liftoff in 23h 14m (17:00)" or "New NET Thu Oct 15 17:00". These now read
  "Liftoff NET Oct 14 (~1 day)", "New NET Oct 15 (~2 days)" and "on hold. NET Oct 15".
- **TUI showed fake times/days for coarse NETs**: the detail and hero NET lines and the launch list now show
  only the known precision ("2026", "Dec 2026", "Q4 2026", "Nov 6") instead of "Wed Dec 30 17:00 (Y)".
- Month/quarter/year NETs use the UTC calendar, so a launch LL2 dates 2027-01-01 00:00 UTC no longer shows
  as 2026 west of UTC. Bar, TUI and alerts share the same text.

## 0.1.3 (2026-10-13)

Fixes.

- **"New launch added" alerts showed a fake time plus "(year)"** for launches LL2 only knows to the year
  (or month/quarter). The alert now shows just the known precision, e.g. "NET 2026", "NET Dec 2026",
  "NET Q4 2026".

## 0.1.2 (2026-10-13)

Fixes.

- **Test flight could end early with two TUIs open**: closing one TUI that had a test flight running left
  a stop record, so the test flight in every other TUI switched off too. Closing a TUI now only clears
  the bar marker; an explicit Ctrl-x t f still stops all of them.
- Settings text: the live-window setting now only describes the rocket icon pulse; no emoji in the vehicle
  setting.
- Maintenance: cargo fmt, clippy -D warnings and cargo audit in CI; new marketplace preview image.

## 0.1.1 (2026-10-13)

Fixes.

- **Bar stuck on "set up OmaLiftoff" after install**: the widget could not watch a cache file that did not
  exist yet. While setup is pending it now re-checks for the cache every 3 s, so the bar switches to the
  countdown as soon as the daemon's first fetch lands (no shell restart needed).
- **TUI scrambled in very large windows** (a maximized 4K Ghostty, e.g. 540x129 = 69,660 cells): ratatui
  0.29 computes cell positions with 16-bit math, so everything past cell 65,535 was drawn at the wrong
  place (bottom panes on the top rows). The TUI now sends frames with its own diff that handles any size.
- **Bar and TUI countdowns could differ by a second**: the bar rounded the remaining seconds while the TUI
  truncated them, and the bar ticked at a random phase. Both now use one rule (whole seconds since T-0,
  floored, after the same clock-skew and T-0 nudge corrections), and both wake right after each whole
  second of the skew-corrected clock, so they flip together. (With a measured skew of ~3 s the bar used to
  tick on local seconds and trail the TUI by almost a full second.)
- **Site**: the screenshot lightbox (click or Enter to enlarge) was missing from the published page.

## 0.1.0 (2026-10-13)

First public release.

- **Bar widget** for the Omarchy Quattro shell (`liftoff.tracker`): countdown, current flight event while
  live, Nerd Font rocket that pulses in the accent color while live and turns red on hold/failure,
  icon-only in the left/right sections (`textMode`), popup with upcoming launches and a Live section.
- **TUI** (`liftoff tui`): Upcoming, Details (mission, orbit, rocket, booster/landing, window, weather,
  patch), Pads world map, Flight Plan / Telemetry pane, 7-day timeline that becomes a live animated mission
  timeline near T-0, pane focus and scrolling, T-0 nudge keys.
- **SpaceX flight plans** from the (unofficial) spacex.com content API, matched to Launch Library launches.
- **Notifications**: notify-send and optional ntfy (server/topic/token), T-10 min for all launches,
  T-24 h / T-1 h for starred providers, hold/slip notices, and "new launch added" alerts (silent first run,
  batched when more than 3 arrive at once).
- **API budget planner**: reserves single-launch syncs at T-5 min, T-60 s, T-5 s and T+90 s inside LL2's
  15 calls/hour; clock-skew correction from HTTP `Date` headers.
- Install as a standard Omarchy plugin (`omarchy plugin add`); the bundled `install.sh` sets up the companion
  from that checkout (version-pinned, checksum-verified prebuilt binary or local cargo build, systemd user
  unit, bar placement, notification wizard, `--uninstall`). The widget offers "set up OmaLiftoff" until then.
- `liftoff setup`, `liftoff doctor`, `liftoff config get|set`.
