//! Normalised data model shared by daemon, TUI and (via JSON) the shell plugin.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const CACHE_VERSION: u32 = 1;

/// LL2 status ids (see /launch_statuses/).
pub mod status {
    pub const GO: i64 = 1;
    pub const TBD: i64 = 2;
    pub const SUCCESS: i64 = 3;
    pub const FAILURE: i64 = 4;
    pub const HOLD: i64 = 5;
    pub const IN_FLIGHT: i64 = 6;
    pub const PARTIAL_FAILURE: i64 = 7;
    pub const TBC: i64 = 8;
    pub const PAYLOAD_DEPLOYED: i64 = 9;

    pub fn is_finished(id: i64) -> bool {
        matches!(id, SUCCESS | FAILURE | PARTIAL_FAILURE | PAYLOAD_DEPLOYED)
    }
    /// Coarse colour class used by the TUI and the QML widget.
    pub fn class(id: i64) -> &'static str {
        match id {
            GO => "go",
            TBC => "tbc",
            HOLD => "hold",
            IN_FLIGHT => "flight",
            SUCCESS | PAYLOAD_DEPLOYED => "success",
            FAILURE | PARTIAL_FAILURE => "failure",
            _ => "tbd",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Launch {
    pub id: String,
    pub name: String,
    pub mission: String,
    pub mission_description: String,
    pub mission_type: String,
    pub orbit: String,
    pub provider: String,
    pub provider_abbrev: String,
    pub rocket: String,
    pub status_id: i64,
    pub status_abbrev: String,
    pub status_name: String,
    /// "go" | "tbc" | "tbd" | "hold" | "flight" | "success" | "failure"
    pub status_class: String,
    pub net: DateTime<Utc>,
    /// LL2 net_precision abbrev: SEC, MIN, HR, HALF, TOD, DAY, W, M, Q, H1, Y, ...
    pub net_precision: String,
    pub window_start: Option<DateTime<Utc>>,
    pub window_end: Option<DateTime<Utc>>,
    pub probability: Option<i64>,
    pub weather_concerns: Option<String>,
    pub pad: String,
    pub location: String,
    pub country: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub webcast_url: Option<String>,
    pub webcast_live: bool,
    pub patch_url: Option<String>,
    pub image_url: Option<String>,
    pub info_url: Option<String>,
    pub last_updated: Option<DateTime<Utc>>,
    // ---- extras from LL2 mode=detailed (already in the list response; no extra calls)
    pub rocket_full_name: String,
    pub rocket_stats: RocketStats,
    pub provider_stats: ProviderStats,
    pub boosters: Vec<Booster>,
    /// LL2 launch timeline (relative milestones), sorted by offset.
    pub milestones: Vec<Milestone>,
    pub pad_launch_count: Option<i64>,
    pub orbital_attempt_of_year: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct RocketStats {
    pub description: String,
    pub length_m: Option<f64>,
    pub diameter_m: Option<f64>,
    pub launch_mass_t: Option<f64>,
    pub leo_kg: Option<f64>,
    pub gto_kg: Option<f64>,
    pub thrust_kn: Option<f64>,
    pub total: Option<i64>,
    pub successes: Option<i64>,
    pub failures: Option<i64>,
    pub consecutive: Option<i64>,
    pub landings_ok: Option<i64>,
    pub landings_attempted: Option<i64>,
    pub reusable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct ProviderStats {
    pub total: Option<i64>,
    pub successes: Option<i64>,
    pub failures: Option<i64>,
    pub consecutive: Option<i64>,
    pub pending: Option<i64>,
    pub landings_ok: Option<i64>,
    pub landings_attempted: Option<i64>,
    pub founded: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Booster {
    pub kind: String,
    pub serial: String,
    pub flight_number: Option<i64>,
    pub reused: bool,
    pub turnaround: String,
    pub landing_attempt: bool,
    pub landing_success: Option<bool>,
    pub landing_description: String,
    pub landing_type: String,
    pub landing_location: String,
    pub landing_abbrev: String,
    pub landing_lat: Option<f64>,
    pub landing_lon: Option<f64>,
    pub downrange_km: Option<f64>,
}

/// One countdown / flight event relative to T-0 (negative = before liftoff).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Milestone {
    pub offset_secs: i64,
    pub label: String,
    pub description: String,
}

/// SpaceX.com mission page data (content.spacex.com), matched to an LL2 launch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct FlightPlan {
    pub slug: String,
    pub url: String,
    pub title: String,
    pub vehicle: String,
    pub launch_site: String,
    pub return_site: String,
    /// Landing / recovery sentence extracted from the mission copy.
    pub recovery: String,
    pub summary: Vec<String>,
    pub events: Vec<Milestone>,
    pub fetched_at: Option<DateTime<Utc>>,
}

impl Launch {
    /// Short vehicle label for the bar ("Falcon 9").
    pub fn short_rocket(&self) -> String {
        let r = self.rocket.trim();
        if r.is_empty() { self.name.split('|').next().unwrap_or("").trim().to_string() } else { r.to_string() }
    }
    /// True when the NET is precise enough for a ticking clock.
    pub fn precise(&self) -> bool {
        matches!(self.net_precision.as_str(), "SEC" | "MIN" | "HR" | "")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Article {
    pub title: String,
    pub url: String,
    pub news_site: String,
    pub summary: String,
    pub published_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Throttle {
    pub limit: i64,
    pub current_use: i64,
    pub next_use_secs: i64,
    pub checked_at: Option<DateTime<Utc>>,
}

impl Throttle {
    pub fn remaining(&self) -> i64 {
        (self.limit - self.current_use).max(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Cache {
    pub version: u32,
    pub source: String,
    pub fetched_at: Option<DateTime<Utc>>,
    pub next_fetch_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub throttle: Throttle,
    /// Providers starred in config (copied so the QML widget can highlight them).
    pub starred: Vec<String>,
    pub launches: Vec<Launch>,
    /// SNAPI articles keyed by LL2 launch id (only for the next few launches).
    pub news: BTreeMap<String, Vec<Article>>,
    pub news_fetched_at: Option<DateTime<Utc>>,
    /// SpaceX flight plans keyed by LL2 launch id.
    pub flight_plans: BTreeMap<String, FlightPlan>,
    pub flight_plans_fetched_at: Option<DateTime<Utc>>,
    /// Reserved final-sync status (written by the daemon, shown by TUI/bar).
    pub sync: SyncStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct SyncStatus {
    /// Last reserved sync: "T-5s · Falcon 9 | X" and when.
    pub last_label: String,
    pub last_at: Option<DateTime<Utc>>,
    /// NET change (s) seen by the last sync (re-anchor), if any.
    pub last_net_shift_secs: Option<i64>,
    pub next_label: String,
    pub next_at: Option<DateTime<Utc>>,
    pub reserved: usize,
    /// When the next routine (unreserved) LL2 refresh may happen.
    pub next_routine_at: Option<DateTime<Utc>>,
    /// Median server-minus-local clock skew (ms) from HTTP Date headers.
    pub clock_skew_ms: Option<i64>,
    /// Skew actually applied by clients (0 unless |skew| > 1 s; Date has 1 s resolution).
    pub clock_offset_ms: i64,
    pub skew_samples: Vec<i64>,
    /// Why a manual refresh was refused, etc.
    pub note: String,
    pub note_at: Option<DateTime<Utc>>,
}

impl SyncStatus {
    pub fn add_skew(&mut self, ms: i64) {
        self.skew_samples.push(ms);
        if self.skew_samples.len() > 7 {
            self.skew_samples.remove(0);
        }
        let mut v = self.skew_samples.clone();
        v.sort();
        let med = v[v.len() / 2];
        self.clock_skew_ms = Some(med);
        self.clock_offset_ms = if med.abs() > 1000 && v.len() >= 3 { med } else { 0 };
    }
}

/// Remove control characters (LL2 copy sometimes contains \r\n / tabs) so no
/// widget ever emits a raw newline into the terminal.
pub fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for c in s.chars() {
        let c = if c.is_control() { ' ' } else { c };
        if c == ' ' && last_space {
            continue;
        }
        last_space = c == ' ';
        out.push(c);
    }
    out.trim().to_string()
}

/// Index of the current milestone (last one at or before `t_rel` seconds) .
pub fn current_milestone(events: &[Milestone], t_rel: i64) -> Option<usize> {
    events.iter().rposition(|e| e.offset_secs <= t_rel)
}

/// Next milestone strictly after `t_rel`.
pub fn next_milestone(events: &[Milestone], t_rel: i64) -> Option<&Milestone> {
    events.iter().find(|e| e.offset_secs > t_rel)
}

pub fn fmt_rel(secs: i64) -> String {
    let a = secs.abs();
    let sign = if secs < 0 { '-' } else { '+' };
    if a >= 3600 { format!("T{sign}{:02}:{:02}:{:02}", a / 3600, (a % 3600) / 60, a % 60) } else { format!("T{sign}{:02}:{:02}", a / 60, a % 60) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ev(o: i64) -> Milestone {
        Milestone { offset_secs: o, label: o.to_string(), description: String::new() }
    }
    #[test]
    fn cache_json_carries_events_for_the_bar() {
        // Model.js reads cache.flight_plans[id].events[].{offset_secs,label} and launches[].milestones
        let mut c = Cache::default();
        c.launches.push(Launch { id: "a".into(), milestones: vec![ev(0)], ..Default::default() });
        c.flight_plans.insert("a".into(), FlightPlan { events: vec![ev(146)], ..Default::default() });
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["flight_plans"]["a"]["events"][0]["offset_secs"], 146);
        assert_eq!(v["launches"][0]["milestones"][0]["label"], "0");
    }
    #[test]
    fn skew_needs_three_samples_and_a_second() {
        let mut s = SyncStatus::default();
        s.add_skew(2500);
        assert_eq!(s.clock_offset_ms, 0);
        s.add_skew(2400);
        s.add_skew(9999);
        assert_eq!(s.clock_offset_ms, 2500);
        let mut t = SyncStatus::default();
        for _ in 0..3 {
            t.add_skew(600);
        }
        assert_eq!(t.clock_offset_ms, 0, "sub-second skew is within Date resolution");
    }
    #[test]
    fn clean_strips_controls() {
        assert_eq!(clean("a\r\nCargo\tDragon  x\n"), "a Cargo Dragon x");
    }
    #[test]
    fn milestone_lookup() {
        let e = vec![ev(-60), ev(0), ev(150), ev(500)];
        assert_eq!(current_milestone(&e, -100), None);
        assert_eq!(current_milestone(&e, 0), Some(1));
        assert_eq!(current_milestone(&e, 200), Some(2));
        assert_eq!(next_milestone(&e, 200).unwrap().offset_secs, 500);
        assert!(next_milestone(&e, 600).is_none());
        assert_eq!(fmt_rel(-125), "T-02:05");
        assert_eq!(fmt_rel(3725), "T+01:02:05");
    }
}
