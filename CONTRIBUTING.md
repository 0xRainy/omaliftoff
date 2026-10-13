# Contributing to OmaLiftoff

Thanks for helping! OmaLiftoff is two parts that ship from this one repo:

* **Bar widget** (`omarchy-plugin/`, QML + a small JS model). Plugin id `liftoff.tracker`; it only reads the local cache.
* **Companion** (`src/`, Rust): the `liftoff` binary (daemon, TUI, notifications, installer helpers).

## Ground rules

* **Be kind.** Assume good faith; keep reviews about the code.
* **Stay inside the API budgets.** Launch Library 2's free tier is 15 requests/hour per IP. Any change that adds
  network calls must go through the planner (`src/planner.rs`) and keep `max_calls_per_hour` honest.
* **No personal data in the repo.** Never commit `config.toml`, cache/state files, ntfy topics, screenshots with
  usernames, hostnames or notification history.
* **No telemetry, no accounts.** OmaLiftoff only talks to the documented public APIs and the user's own ntfy server.
* **The widget never fetches.** QML reads `launches.json`; all network access belongs to the companion.

## Dev setup

```bash
cargo test --locked                       # Rust unit tests
node omarchy-plugin/tests/model.test.js   # widget model tests
node omarchy-plugin/tests/widget.test.js  # BarWidget.qml static checks
shellcheck install.sh
LIFTOFF_API=dev cargo run -- tui          # lldev endpoint: no rate limit, stale data
```

To try the widget on Omarchy from a checkout: `./install.sh` (dev install copies the widget into the plugins
folder). In the TUI, `Ctrl-x` `t` `f` toggles a synthetic test flight (desktop alerts only, never ntfy).

## Pull requests

1. Open an issue first for anything bigger than a bug fix, so we can agree on the approach.
2. Keep PRs small and focused; one topic per PR.
3. CI must be green: `cargo test`, the widget model tests and `shellcheck`. Run `cargo fmt` and `cargo clippy`.
4. Add or update tests for behavior changes (countdown, scheduling, dedupe and parsing all have tests).
5. Update `README.md` and `CHANGELOG.md` (under *Unreleased*) when users would notice the change.
6. UI changes: attach a screenshot (dark theme, no personal data in view).

Commit messages: short imperative subject (`tui: show booster landing zone`), body explaining *why*.

## Releases

Maintainers bump `version` in `Cargo.toml` and `manifest.json` together, update `CHANGELOG.md`, and push a
`vX.Y.Z` tag; CI builds and attaches the prebuilt binary that `install.sh` downloads.

## Data sources

The Space Devs Launch Library 2, Spaceflight News API, and SpaceX's public (unofficial, undocumented) content
API. Respect their terms; OmaLiftoff is not affiliated with any of them.

## License

By contributing you agree your contributions are licensed under the MIT License (see `LICENSE`).
