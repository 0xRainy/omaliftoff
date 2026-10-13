# OmaLiftoff goals

## What OmaLiftoff is
A local-first rocket launch tracker for Omarchy: a Rust `liftoff` binary (daemon + CLI + ratatui TUI) and a
QML bar widget that only reads the daemon's local cache.

## Goals
1. Track **every launch worldwide, from every provider**, via Launch Library 2 (LL2).
2. Stay inside LL2's free budget of 15 requests/hour, with reserved syncs at T-5m, T-60s, T-5s and T+90s.
3. Add richer data where a provider publishes it (today: SpaceX flight plans and milestones from the
   unofficial SpaceX content API), without making any provider required.
4. Alerts: desktop notifications (mako / `notify-send`) and optional ntfy phone push.
5. Bar widget UI: Nerd Font rocket icon only (U+F14DE), uncolored text; the icon is colored only while live
   (accent pulse) or on bad status (red); icon-only in the left/right bar sections.
6. Conform to the Omarchy plugin standard so it can be listed on plugins.omarchy.org.

## Non-goals
- No hosted backend, accounts or telemetry. Everything runs on the user's machine.
- No `sudo` and no `curl | bash` anywhere in the plugin/install flow.
- Test Flight (`Ctrl-x t f`) never sends phone pushes (ntfy) and never touches real alert state or the LL2 budget.
- No emoji or extra colors in the bar; no second icon set.

## What counts as scope creep
Anything that extends or diverges from the above, for example: new data sources or providers that need keys,
new notification channels, a server/sync component, new widget kinds or bar styling, changes to the
request budget or sync schedule, new config surface with user-visible behavior, new dependencies with
network access, or relicensing/asset changes (e.g. the preview GIF).

## Approval rule
- **Automatic (no approval needed):** in-scope bug fixes, tests, CI, lint/format, docs that describe existing
  behavior, dependency patch bumps.
- **Needs the human owner's approval before merge:** anything listed under scope creep, anything touching a
  non-goal, and marketplace submissions or releases.
- When unsure, treat it as needing approval.
