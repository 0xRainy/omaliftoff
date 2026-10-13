//! Hidden "Test Flight": a synthetic, looping Falcon 9 Starlink launch that
//! exercises the TUI end to end. It lives only in TUI memory: it is never
//! written to the cache or dedupe state and never seen by the daemon; the bar
//! widget sees it only through a heartbeat marker file (display only), and its
//! alerts go to the desktop only — never ntfy.
use crate::model::{status, Booster, FlightPlan, Launch, Milestone, ProviderStats, RocketStats};
use crate::schedule::{Alert, AlertKind};
use chrono::{DateTime, Duration, Utc};

pub const ID: &str = "liftoff-test-flight";
/// Loop starts at T-2:00.
pub const START: f64 = -120.0;
/// Real-time until this T+ (booster landing + SECO-1), then fast-forward the coast.
pub const REALTIME_UNTIL: f64 = 540.0;
pub const WARP: f64 = 20.0;
/// Hold on "deployed" this long (real seconds) before restarting.
pub const HOLD_AFTER: f64 = 20.0;

/// Starlink Group 10-22 (Sep 3 2025) flight plan from spacex.com.
const EVENTS: &[(i64, &str, &str)] = &[
    (-2280, "Go prop load", "SpaceX Launch Director verifies go for propellant load"),
    (-2100, "RP-1 load", "RP-1 (rocket grade kerosene) loading begins"),
    (-2100, "S1 LOX load", "1st stage LOX (liquid oxygen) loading begins"),
    (-960, "S2 LOX load", "2nd stage LOX loading begins"),
    (-420, "Engine chill", "Falcon 9 begins engine chill prior to launch"),
    (-60, "Startup", "Command flight computer to begin final prelaunch checks"),
    (-60, "Pressurize", "Propellant tank pressurization to flight pressure begins"),
    (-45, "Go for launch", "SpaceX Launch Director verifies go for launch"),
    (-3, "Ignition", "Engine controller commands engine ignition sequence to start"),
    (0, "Liftoff", "Falcon 9 liftoff"),
    (72, "Max Q", "Max Q (moment of peak mechanical stress on the rocket)"),
    (144, "MECO", "1st stage main engine cutoff (MECO)"),
    (148, "Stage sep", "1st and 2nd stages separate"),
    (156, "SES-1", "2nd stage engine starts (SES-1)"),
    (176, "Fairing sep", "Fairing separation"),
    (368, "Entry burn", "1st stage entry burn begins"),
    (395, "Entry end", "1st stage entry burn ends"),
    (477, "Landing burn", "1st stage landing burn begins"),
    (501, "Landing", "1st stage landing"),
    (519, "SECO-1", "2nd stage engine cutoff (SECO-1)"),
    (3287, "SES-2", "2nd stage engine starts (SES-2)"),
    (3288, "SECO-2", "2nd stage engine cutoff (SECO-2)"),
    (3849, "Deploy", "Starlink satellites deploy"),
];

pub fn events() -> Vec<Milestone> {
    EVENTS.iter().map(|(o, l, d)| Milestone { offset_secs: *o, label: (*l).into(), description: (*d).into() }).collect()
}

fn last_event() -> f64 {
    EVENTS.last().unwrap().0 as f64
}

/// Real seconds for one loop.
pub fn period() -> f64 {
    (REALTIME_UNTIL - START) + (last_event() - REALTIME_UNTIL) / WARP + HOLD_AFTER
}

/// Simulated mission time (seconds relative to T-0) after `elapsed` real seconds; loops.
pub fn sim_t(elapsed: f64) -> f64 {
    let e = elapsed.rem_euclid(period());
    let rt = REALTIME_UNTIL - START;
    let warp_len = (last_event() - REALTIME_UNTIL) / WARP;
    if e <= rt {
        START + e
    } else if e - rt <= warp_len {
        REALTIME_UNTIL + (e - rt) * WARP
    } else {
        last_event()
    }
}

pub fn plan() -> FlightPlan {
    FlightPlan {
        slug: "test-flight".into(),
        url: "https://www.spacex.com/launches/".into(),
        title: "TEST FLIGHT".into(),
        vehicle: "Falcon 9".into(),
        launch_site: "SLC-40, Florida".into(),
        return_site: "Droneship".into(),
        recovery: "TEST: the first stage lands on A Shortfall of Gravitas in the Atlantic Ocean.".into(),
        summary: vec!["Synthetic loop for testing OmaLiftoff. Not a real launch.".into()],
        events: events(),
        fetched_at: None,
    }
}

/// The synthetic launch as seen at wall time `now` and mission time `t`.
pub fn launch(now: DateTime<Utc>, t: f64) -> Launch {
    let net = now - Duration::milliseconds((t * 1000.0) as i64);
    let (sid, abbrev, name) = if t < 0.0 {
        (status::GO, "Go", "Go for Launch")
    } else if t < last_event() {
        (status::IN_FLIGHT, "In Flight", "Launch in Flight")
    } else {
        (status::SUCCESS, "Success", "Launch Successful")
    };
    Launch {
        id: ID.into(),
        name: "Falcon 9 Block 5 | TEST FLIGHT · Starlink Group 99-1".into(),
        mission: "TEST FLIGHT · Starlink 99-1".into(),
        mission_description: "Synthetic test flight (looping). Timeline from a real Starlink flight plan. Alerts are desktop-only and marked TEST.".into(),
        mission_type: "Communications".into(),
        orbit: "LEO".into(),
        provider: "SpaceX".into(),
        provider_abbrev: "SpX".into(),
        rocket: "Falcon 9".into(),
        rocket_full_name: "Falcon 9 Block 5".into(),
        status_id: sid,
        status_abbrev: abbrev.into(),
        status_name: name.into(),
        status_class: status::class(sid).into(),
        net,
        net_precision: "SEC".into(),
        window_start: Some(net - Duration::minutes(30)),
        window_end: Some(net + Duration::minutes(90)),
        probability: Some(95),
        pad: "Space Launch Complex 40".into(),
        location: "Cape Canaveral SFS, FL, USA (TEST)".into(),
        country: "USA".into(),
        lat: Some(28.5618),
        lon: Some(-80.577),
        webcast_live: t > -600.0 && t < last_event(),
        rocket_stats: RocketStats { length_m: Some(70.0), diameter_m: Some(3.65), launch_mass_t: Some(549.0), leo_kg: Some(22800.0), thrust_kn: Some(7607.0), reusable: true, ..Default::default() },
        provider_stats: ProviderStats::default(),
        boosters: vec![Booster {
            kind: "Core".into(),
            serial: "B1077".into(),
            flight_number: Some(24),
            reused: true,
            landing_attempt: true,
            landing_success: (t > 501.0).then_some(true),
            landing_type: "ASDS".into(),
            landing_location: "A Shortfall of Gravitas".into(),
            landing_abbrev: "ASOG".into(),
            downrange_km: Some(600.0),
            ..Default::default()
        }],
        milestones: events(),
        ..Default::default()
    }
}

/// Events crossed going from mission time `prev` to `cur` (handles the loop restart).
pub fn crossed(prev: f64, cur: f64) -> Vec<Milestone> {
    if cur < prev {
        return vec![];
    }
    events().into_iter().filter(|e| (e.offset_secs as f64) > prev && (e.offset_secs as f64) <= cur).collect()
}

/// Delivery channels for test-flight alerts. Desktop only, whatever the config says.
#[derive(Debug, PartialEq)]
pub enum Channel {
    Desktop,
}
pub fn channels(_cfg: &crate::config::NotifyConfig) -> Vec<Channel> {
    vec![Channel::Desktop]
}

pub fn alert(e: &Milestone) -> Alert {
    Alert {
        key: format!("test:{}", e.offset_secs),
        launch_id: ID.into(),
        kind: AlertKind::Lead(0),
        net: Utc::now(),
        title: format!("TEST · {} {}", e.label, crate::model::fmt_rel(e.offset_secs)),
        body: format!("OmaLiftoff test flight — {}", e.description),
        url: None,
        critical: false,
        also_covers: vec![],
    }
}

/// Marker file the bar widget reads while a TUI test flight is running
/// (`$XDG_CACHE_HOME/liftoff/testflight.json`). Display only: the daemon never
/// reads it, so it can't touch alert state, ntfy or the LL2 budget. The widget
/// ignores it once `heartbeat_ms` is older than [`STALE_MS`], so a crashed TUI
/// can't leave the bar stuck live.
pub fn marker_file() -> std::path::PathBuf {
    crate::config::paths::cache_file().with_file_name("testflight.json")
}
pub const STALE_MS: i64 = 5000;

pub fn marker_json(now: DateTime<Utc>, t: f64) -> serde_json::Value {
    serde_json::json!({
        "test": true,
        "label": "Test Flight",
        "pid": std::process::id(),
        "heartbeat_ms": now.timestamp_millis(),
        "stale_ms": STALE_MS,
        "launch": launch(now, t),
        "flight_plan": plan(),
    })
}

fn write_json(v: &serde_json::Value) {
    let p = marker_file();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    // tmp + rename so the widget never reads a half-written file
    let tmp = p.with_extension(format!("json.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, v.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

fn read_marker() -> serde_json::Value {
    std::fs::read_to_string(marker_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(serde_json::Value::Null)
}

/// True if any TUI switched the test flight off after `started_ms` (a stop is global:
/// with several TUIs open, Ctrl-x t f in one stops them all).
pub fn stopped_since(started_ms: i64) -> bool {
    read_marker()["stopped_ms"].as_i64().is_some_and(|s| s >= started_ms)
}

pub fn write_marker(now: DateTime<Utc>, t: f64) {
    write_json(&marker_json(now, t));
}

/// Test flight switched off: leave a stop record (not a test) so the bar drops it at
/// once and any other running TUI stops its own test flight too.
pub fn stop_marker() {
    write_json(&serde_json::json!({ "test": false, "stopped_ms": Utc::now().timestamp_millis() }));
}

/// TUI exiting with its test flight on: remove the marker if this process wrote it.
pub fn clear_marker() {
    let m = read_marker();
    if m["pid"].as_u64() == Some(std::process::id() as u64) {
        stop_marker();
    }
}

/// Fire desktop notifications (labelled TEST) for crossed events. Never ntfy.
pub fn deliver(cfg: &crate::config::NotifyConfig, evs: &[Milestone]) {
    if !cfg.enabled || !cfg.desktop {
        return;
    }
    for e in evs {
        for ch in channels(cfg) {
            match ch {
                Channel::Desktop => {
                    let _ = crate::notify::desktop(&alert(e));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loop_timing() {
        assert_eq!(sim_t(0.0), -120.0);
        assert_eq!(sim_t(120.0), 0.0);
        assert_eq!(sim_t(660.0), REALTIME_UNTIL);
        // warp: 20 real seconds later = 400 sim seconds later
        assert!((sim_t(680.0) - (REALTIME_UNTIL + 400.0)).abs() < 1e-6);
        let reach_end = 660.0 + (last_event() - REALTIME_UNTIL) / WARP;
        assert!((sim_t(reach_end) - last_event()).abs() < 1e-6);
        // holds at the final event, then restarts at T-2:00
        assert!((sim_t(reach_end + HOLD_AFTER / 2.0) - last_event()).abs() < 1e-6);
        assert_eq!(sim_t(period()), -120.0);
        assert_eq!(sim_t(period() + 120.0), 0.0);
        // monotonic within one loop
        let mut prev = f64::MIN;
        let mut x = 0.0;
        while x < period() - 0.01 {
            let t = sim_t(x);
            assert!(t >= prev);
            prev = t;
            x += 0.5;
        }
        assert!(period() < 20.0 * 60.0, "loop should be short enough to watch");
    }
    #[test]
    fn launch_status_follows_time() {
        let now = Utc::now();
        let l = launch(now, -30.0);
        assert_eq!(l.status_class, "go");
        assert_eq!((now - l.net).num_seconds(), -30);
        assert_eq!(launch(now, 200.0).status_id, status::IN_FLIGHT);
        assert_eq!(launch(now, 4000.0).status_class, "success");
    }
    #[test]
    fn crossing_and_restart() {
        let c: Vec<String> = crossed(140.0, 150.0).into_iter().map(|e| e.label).collect();
        assert_eq!(c, vec!["MECO", "Stage sep"]);
        assert!(crossed(3849.0, -120.0).is_empty());
        assert_eq!(crossed(-1.0, 0.0)[0].label, "Liftoff");
    }
    #[test]
    fn never_ntfy() {
        let mut cfg = crate::config::NotifyConfig::default();
        cfg.ntfy = true;
        cfg.ntfy_topic = "real-topic".into();
        assert_eq!(channels(&cfg), vec![Channel::Desktop]);
        let a = alert(&events()[9]);
        assert!(a.title.starts_with("TEST"));
        assert!(a.url.is_none());
        // enforce at source level: this module must not reach the ntfy path
        let src = include_str!("testflight.rs");
        let body = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(!body.contains(concat!("notify::", "send")) && !body.contains(concat!("notify::", "ntfy")));
        assert!(!body.contains(concat!("store::", "save")) && !body.contains(concat!("write_", "atomic")), "must not touch cache/state");
        assert!(!body.contains(concat!("state_", "file")), "must not touch alert state");
    }

    #[test]
    fn marker_is_separate_file_with_heartbeat() {
        let p = marker_file();
        assert_eq!(p.file_name().unwrap(), "testflight.json");
        assert_ne!(p, crate::config::paths::cache_file());
        let now = Utc::now();
        let m = marker_json(now, 150.0);
        assert_eq!(m["test"], true);
        assert_eq!(m["label"], "Test Flight");
        assert_eq!(m["heartbeat_ms"], now.timestamp_millis());
        assert_eq!(m["launch"]["id"], ID);
        assert_eq!(m["launch"]["status_id"], status::IN_FLIGHT);
        assert!(m["flight_plan"]["events"].as_array().unwrap().len() > 10);
    }
}
