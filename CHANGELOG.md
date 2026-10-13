# Changelog

## 0.1.0 (unreleased)

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
