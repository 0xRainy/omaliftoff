# Changelog

## 0.1.1 (unreleased)

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
