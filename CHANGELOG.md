# Changelog

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
