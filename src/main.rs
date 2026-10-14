// collapsible_if (let-chains, Rust 1.88) and manual_is_multiple_of (Rust 1.87) suggest newer APIs;
// allowed so the crate keeps building on older toolchains.
#![allow(clippy::collapsible_if, clippy::manual_is_multiple_of)]
mod api;
mod cli;
mod config;
mod daemon;
mod model;
mod newlaunch;
mod notify;
mod nudge;
mod patch;
mod planner;
mod schedule;
mod spacex;
mod store;
mod testflight;
mod tui;

use anyhow::Result;
use chrono::Utc;
use config::{Config, paths};

const HELP: &str = "liftoff — rocket launch tracker

USAGE: liftoff <command>
  daemon          fetch loop + notifications (run via systemd --user)
  tui             interactive dashboard
  fetch           fetch now (respects the hourly budget) and write the cache
  refresh         ask the running daemon to refresh soon (used by the bar)
  status [--json|--waybar]   one-line next-launch text for bars/scripts
  alerts [--dry]  evaluate notifications once against the cache
  test-notify [--desktop|--ntfy]   send a sample notification
  setup           (re)run the notification wizard
  doctor          check service, cache, API budget, plugin, ntfy
  config get|set <key> [value]   read/edit config.toml (dotted keys)
  config set-secret <key>        set notify.ntfy_topic/ntfy_token/api_key from stdin
  paths           print config/cache/state paths
Env: LIFTOFF_API=dev|prod|<url> overrides api_base.";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let flag = |f: &str| args.iter().any(|a| a == f);
    match cmd {
        "daemon" => daemon::run(),
        "tui" => tui::run(),
        "fetch" => {
            let cfg = Config::load()?;
            let c = daemon::fetch_and_save(&cfg, true)?;
            println!(
                "{} launches from {} → {}{}",
                c.launches.len(),
                c.source,
                paths::cache_file().display(),
                c.last_error.map(|e| format!(" (warning: {e})")).unwrap_or_default()
            );
            Ok(())
        }
        "refresh" => {
            store::write_atomic(&daemon::refresh_request_file(), b"1")?;
            Ok(())
        }
        "status" => {
            let cache = store::load_cache(&paths::cache_file());
            let now = Utc::now();
            let Some(l) = schedule::next_launch(&cache.launches, now) else {
                println!("\u{f14de} —");
                return Ok(());
            };
            let text = format!("\u{f14de} {} · {}", l.short_rocket(), schedule::display_countdown(l, now));
            if flag("--waybar") || flag("--json") {
                let mut tip = format!("{}\n{} · {}\n{}", l.name, l.provider, l.status_name, l.pad);
                let ev = cache.flight_plans.get(&l.id).map(|p| p.events.as_slice()).filter(|e| !e.is_empty()).unwrap_or(&l.milestones);
                if let Some(m) = model::next_milestone(ev, (now - l.net).num_seconds()) {
                    tip.push_str(&format!("\nnext: {} {}", m.label, model::fmt_rel(m.offset_secs)));
                }
                println!("{}", serde_json::json!({"text": text, "tooltip": tip, "class": l.status_class, "alt": l.status_class}));
            } else {
                println!("{text}");
            }
            Ok(())
        }
        "alerts" => {
            let cfg = Config::load()?;
            let cache = store::load_cache(&paths::cache_file());
            let sp = paths::state_file();
            let mut st = store::load_state(&sp);
            let dry = flag("--dry");
            let a = daemon::run_alerts(&cfg, &cache, &mut st, dry);
            if !dry {
                store::save_json(&sp, &st)?;
            }
            println!("{} alert(s)", a.len());
            Ok(())
        }
        "test-notify" => {
            let cfg = Config::load()?;
            let a = schedule::Alert {
                key: "test".into(),
                launch_id: "test".into(),
                kind: schedule::AlertKind::Lead(10),
                net: Utc::now(),
                title: "🚀 T-10m · OmaLiftoff test".into(),
                body: "This is a test notification from liftoff.".into(),
                url: Some("https://www.youtube.com/@SpaceX/live".into()),
                critical: false,
                also_covers: vec![],
            };
            let mut n = cfg.notify.clone();
            if flag("--ntfy") {
                n.desktop = false;
                n.ntfy = true;
            }
            if flag("--desktop") {
                n.ntfy = false;
                n.desktop = true;
            }
            let errs = notify::send(&n, &a);
            std::thread::sleep(std::time::Duration::from_millis(300));
            if n.ntfy && n.ntfy_topic.is_empty() {
                return Err(anyhow::anyhow!("ntfy: no topic configured (liftoff setup)"));
            }
            if errs.is_empty() {
                println!("sent");
                Ok(())
            } else {
                Err(anyhow::anyhow!("errors: {errs:?}"))
            }
        }
        "setup" => cli::setup(),
        "doctor" => cli::doctor(),
        "config" => cli::config_cmd(&args[1..]),
        "paths" => {
            let _ = Config::load()?; // creates a default config.toml on first run
            println!("config {}\ncache  {}\nstate  {}", paths::config_file().display(), paths::cache_file().display(), paths::state_file().display());
            Ok(())
        }
        _ => {
            println!("{HELP}");
            Ok(())
        }
    }
}
