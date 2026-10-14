//! Normalised data model shared by daemon, TUI and (via JSON) the shell plugin.
use chrono::{DateTime, Datelike, Local, NaiveDate, Utc};
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
    /// LL2 net_precision abbrev (`net_precision.abbrev`, not the name), as listed by
    /// /2.3.0/config/net_precisions/: SEC MIN HR AM PM DAY WK M Q1-Q4 H1 H2 Y FY DEC.
    /// Interpret it through [`Launch::precision`].
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
        matches!(self.precision(), Precision::Exact | Precision::Hour)
    }
    pub fn precision(&self) -> Precision {
        Precision::from_abbrev(&self.net_precision)
    }

    /// The NET's calendar date. Exact/hour NETs are instants, so this is the local date.
    /// Day and coarser NETs are calendar *labels*: their date is the UTC date, in every zone.
    ///
    /// Evidence (LL2 2.3.0 responses, 2026-10-14; 557 launches from lldev + ll upcoming):
    /// DAY nets are all `YYYY-MM-DDT00:00:00Z` of the day itself (9/9, e.g. Amazon Leo LV-01
    /// "net": "2026-10-29T00:00:00Z"); M/Q1-Q4/H1/H2/Y nets are 00:00Z of (usually) the last day
    /// of the period (Y 96/96 "12-31T00:00:00Z", Q4 18/18 "12-31", H1 6/6 "06-30"; Q1/Q2 have a
    /// few non-period-end dates, so quarter/half come from the code, not the date); FY 6/6
    /// "2027-06-30T00:00:00Z" (US launches). No AM, PM, WK or DEC launch exists in the data, so
    /// those encodings are unverified and rendered conservatively (approximate, "~").
    pub fn net_date(&self) -> NaiveDate {
        if self.precise() { self.net.with_timezone(&Local).date_naive() } else { self.net.date_naive() }
    }

    /// THE only "is this launch overdue?" rule (status line, bar, TUI, refresh pacing). Returns the
    /// earliest instant at which the NET can be in the past. Overdue never hides a launch: LL2's
    /// upcoming list is the ground truth for what is shown; overdue launches read "awaiting update".
    ///
    /// Exact/hour NETs are instants: current until NET + 30 min (unchanged). Day and coarser NETs
    /// are calendar *labels* (see [`Launch::net_date`]): LL2 stores them as 00:00 UTC of a day in
    /// the period, so the NET instant says nothing about when the launch is late. Such a launch
    /// stays current through the end of its labelled UTC period:
    /// DAY/AM/PM -> end of that UTC day; M -> end of the month; Q1-Q4/H1/H2 -> end of the quarter/
    /// half *named by the code* (LL2 2.3.0 lldev upcoming, 2026-10-14, 178 launches: Q1 nets on
    /// 01-31 and 02-28 as well as 03-31, so the date alone would end Q1 early); Y -> end of year.
    /// Unverified encodings (no LL2 sample) are kept current longest: WK -> 7 days after the
    /// date, FY -> end of that calendar year (US FY samples sit at 06-30), DEC -> end of decade,
    /// unknown codes -> end of the day.
    pub fn current_until(&self) -> DateTime<Utc> {
        use Precision as P;
        let d = self.net.date_naive();
        let start = |y: i32, m: u32| NaiveDate::from_ymd_opt(y, m, 1).map(|n| n.and_hms_opt(0, 0, 0).unwrap().and_utc());
        // first instant of the month after (y, m)
        let after = |y: i32, m: u32| if m >= 12 { start(y + 1, 1) } else { start(y, m + 1) };
        let day_end = (d + chrono::Duration::days(1)).and_hms_opt(0, 0, 0).unwrap().and_utc();
        let end = match self.precision() {
            P::Exact | P::Hour => return self.net + chrono::Duration::minutes(30),
            P::Day | P::Morning | P::Afternoon | P::Unknown => Some(day_end),
            P::Week => Some(day_end + chrono::Duration::days(6)),
            P::Month => after(d.year(), d.month()),
            P::Quarter(0) => after(d.year(), ((d.month() - 1) / 3 + 1) * 3),
            P::Quarter(q) => after(d.year(), u32::from(q) * 3),
            P::Half(h) => after(d.year(), u32::from(h) * 6),
            P::Year | P::FiscalYear => start(d.year() + 1, 1),
            P::Decade => start(d.year() / 10 * 10 + 10, 1),
        };
        // never earlier than the NET itself (e.g. a Q1 code on a date in May)
        end.unwrap_or(day_end).max(day_end)
    }

    /// Past its [`Launch::current_until`] and not in flight: LL2 still lists it, but its NET has
    /// passed without LL2 confirming a liftoff or a new date.
    pub fn overdue(&self, now: DateTime<Utc>) -> bool {
        self.status_id != status::IN_FLIGHT && now >= self.current_until()
    }

    /// THE only place a NET becomes text (alerts, TUI, bar, status). Converts to local time
    /// only for exact/hour precision; day and coarser use LL2's UTC calendar fields.
    pub fn net_text(&self, f: NetFmt) -> String {
        use Precision as P;
        let t = self.net.with_timezone(&Local);
        let u = self.net;
        let day = |long: bool| u.format(if long { "%a %b %-d" } else { "%b %-d" }).to_string();
        match (self.precision(), f) {
            (P::Exact, NetFmt::Clock) => t.format("%H:%M").to_string(),
            (P::Exact, NetFmt::Long) => t.format("%a %b %-d %H:%M").to_string(),
            (P::Exact, _) => t.format("%b %-d %H:%M").to_string(),
            (P::Hour, NetFmt::Clock) => t.format("~%H:%M ±1h").to_string(),
            (P::Hour, NetFmt::Long) => t.format("%a %b %-d ~%H:%M ±1h").to_string(),
            (P::Hour, _) => t.format("%b %-d ~%H:00 (±1h)").to_string(),
            (P::Day, NetFmt::Column) => day(false),
            (P::Day, f) => day(f == NetFmt::Long),
            // AM/PM (LL2 docs: "expected in the morning/afternoon (local time)", i.e. launch-site
            // time), WK and unknown codes: no sample in LL2 data, so the timestamp's day is not
            // verified -> approximate date only, no weekday.
            (P::Morning | P::Afternoon | P::Week | P::Unknown, NetFmt::Column) => format!("~{}", day(false)),
            (P::Morning, _) => format!("~{} (morning, site time)", day(false)),
            (P::Afternoon, _) => format!("~{} (afternoon, site time)", day(false)),
            (P::Week, _) => format!("~{} (week)", day(false)),
            (P::Unknown, _) => format!("~{}", day(false)),
            (P::Month, _) => u.format("%b %Y").to_string(),
            (P::Quarter(0), _) => format!("Q{} {}", (u.month() - 1) / 3 + 1, u.year()),
            (P::Quarter(q), _) => format!("Q{q} {}", u.year()),
            (P::Half(h), _) => format!("H{h} {}", u.year()),
            (P::Year, _) => u.year().to_string(),
            // FY: "fiscal year of the launch country"; which FY a timestamp denotes is not
            // documented (US samples sit at 06-30) -> approximate calendar year only.
            (P::FiscalYear, NetFmt::Column) => format!("~{}", u.year()),
            (P::FiscalYear, _) => format!("~{} (fiscal year)", u.year()),
            // DEC: no sample in LL2 data -> approximate.
            (P::Decade, _) => format!("~{}s", u.year() / 10 * 10),
        }
    }
}

/// How much room a NET label gets. Coarse precisions ignore the difference where it
/// would add invented detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetFmt {
    /// "Nov 4 20:30", "Nov 7", "Q4 2026" (alerts)
    Short,
    /// "Wed Nov 4 20:30", "Sat Nov 7" (TUI detail/hero, bar, long alert form)
    Long,
    /// "20:30" for exact; coarse NETs fall back to their label
    Clock,
    /// at most 9 chars (TUI list column)
    Column,
}

/// LL2 NET precision, from `net_precision.abbrev` (LL2 2.3.0 /config/net_precisions/).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    /// SEC, MIN (and missing → treated as MIN by the parser)
    Exact,
    Hour,
    /// AM / PM: morning / afternoon of the day (launch-site local)
    Morning,
    Afternoon,
    Day,
    Week,
    Month,
    /// Q1..Q4
    Quarter(u8),
    /// H1 / H2
    Half(u8),
    Year,
    FiscalYear,
    Decade,
    /// Anything else: shown as an approximate date, never with a time.
    Unknown,
}

impl Precision {
    pub fn from_abbrev(a: &str) -> Self {
        match a {
            "SEC" | "MIN" | "" => Self::Exact,
            "HR" => Self::Hour,
            "AM" => Self::Morning,
            "PM" => Self::Afternoon,
            // "HALF"/"TOD" were never LL2 codes; kept harmless as day precision
            "DAY" | "HALF" | "TOD" => Self::Day,
            // "W" is a legacy alias of LL2's "WK"
            "WK" | "W" => Self::Week,
            "M" => Self::Month,
            "Q1" => Self::Quarter(1),
            "Q2" => Self::Quarter(2),
            "Q3" => Self::Quarter(3),
            "Q4" => Self::Quarter(4),
            // legacy bare "Q": quarter derived from the date
            "Q" => Self::Quarter(0),
            "H1" => Self::Half(1),
            "H2" => Self::Half(2),
            "Y" => Self::Year,
            "FY" => Self::FiscalYear,
            "DEC" => Self::Decade,
            _ => Self::Unknown,
        }
    }
    /// 0 exact, 1 hour, 2 within a day, 3 coarser (used by alert gating).
    pub fn rank(self) -> u8 {
        match self {
            Self::Exact => 0,
            Self::Hour => 1,
            Self::Morning | Self::Afternoon | Self::Day => 2,
            _ => 3,
        }
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

#[cfg(test)]
mod net_text_tests {
    use super::*;
    use chrono::TimeZone;

    /// Coarse NETs are calendar labels: their text must be byte-identical in every zone.
    /// Run under TZ=America/Phoenix, UTC and Asia/Tokyo (CI/test script); any `Local`
    /// conversion of a coarse NET makes one of them fail. Instants are the real LL2 encoding
    /// (00:00Z) plus 23:59Z to catch the other direction.
    #[test]
    fn coarse_net_text_is_timezone_independent() {
        let at = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ").unwrap().and_utc();
        let cases: &[(&str, &str, [&str; 4])] = &[
            // code, net, [Short, Long, Clock, Column]
            ("DAY", "2026-11-07T00:00:00Z", ["Nov 7", "Sat Nov 7", "Nov 7", "Nov 7"]),
            ("DAY", "2026-11-07T23:59:00Z", ["Nov 7", "Sat Nov 7", "Nov 7", "Nov 7"]),
            ("M", "2026-12-31T00:00:00Z", ["Dec 2026", "Dec 2026", "Dec 2026", "Dec 2026"]),
            ("Q4", "2026-12-31T00:00:00Z", ["Q4 2026", "Q4 2026", "Q4 2026", "Q4 2026"]),
            ("Q1", "2027-01-01T00:00:00Z", ["Q1 2027", "Q1 2027", "Q1 2027", "Q1 2027"]),
            ("H1", "2027-06-30T00:00:00Z", ["H1 2027", "H1 2027", "H1 2027", "H1 2027"]),
            ("Y", "2026-12-31T00:00:00Z", ["2026", "2026", "2026", "2026"]),
            ("Y", "2027-01-01T00:00:00Z", ["2027", "2027", "2027", "2027"]),
            ("FY", "2027-06-30T00:00:00Z", ["~2027 (fiscal year)", "~2027 (fiscal year)", "~2027 (fiscal year)", "~2027"]),
            ("DEC", "2029-12-31T00:00:00Z", ["~2020s", "~2020s", "~2020s", "~2020s"]),
            ("AM", "2026-11-03T00:00:00Z", ["~Nov 3 (morning, site time)", "~Nov 3 (morning, site time)", "~Nov 3 (morning, site time)", "~Nov 3"]),
            ("PM", "2026-11-03T23:59:00Z", ["~Nov 3 (afternoon, site time)", "~Nov 3 (afternoon, site time)", "~Nov 3 (afternoon, site time)", "~Nov 3"]),
            ("WK", "2026-11-09T00:00:00Z", ["~Nov 9 (week)", "~Nov 9 (week)", "~Nov 9 (week)", "~Nov 9"]),
        ];
        for (code, net, want) in cases {
            let l = Launch { net: at(net), net_precision: (*code).into(), ..Default::default() };
            for (f, w) in [NetFmt::Short, NetFmt::Long, NetFmt::Clock, NetFmt::Column].iter().zip(want) {
                assert_eq!(l.net_text(*f), *w, "{code} {net} {f:?}");
            }
            assert!(l.net_text(NetFmt::Column).chars().count() <= 9, "{code}");
            assert_eq!(l.net_date(), at(net).date_naive(), "{code}");
        }
    }

    #[test]
    fn exact_and_hour_are_local_instants() {
        let net = Utc.with_ymd_and_hms(2026, 11, 5, 3, 30, 0).unwrap();
        let t = net.with_timezone(&Local);
        let l = Launch { net, net_precision: "MIN".into(), ..Default::default() };
        assert_eq!(l.net_text(NetFmt::Clock), t.format("%H:%M").to_string());
        assert_eq!(l.net_text(NetFmt::Long), t.format("%a %b %-d %H:%M").to_string());
        assert_eq!(l.net_date(), t.date_naive());
        let h = Launch { net, net_precision: "HR".into(), ..Default::default() };
        assert!(h.net_text(NetFmt::Clock).ends_with("±1h"));
    }
}

#[cfg(test)]
mod expiry_tests {
    use super::*;

    /// Shared with omarchy-plugin/tests/model.test.js (same rule in Rust and the widget).
    #[test]
    fn listed_launches_stay_shown_and_overdue_ones_say_so() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/overdue_cases.json")).unwrap();
        for c in v["cases"].as_array().unwrap() {
            let l = Launch {
                net: c["net"].as_str().unwrap().parse().unwrap(),
                net_precision: c["p"].as_str().unwrap().into(),
                status_id: c["status_id"].as_i64().unwrap_or(1),
                ..Default::default()
            };
            let now: DateTime<Utc> = c["now"].as_str().unwrap().parse().unwrap();
            assert_eq!(l.overdue(now), c["overdue"].as_bool().unwrap(), "{c}");
            // LL2 is the ground truth: a listed, unfinished launch is never hidden, overdue or not
            // (through the same choke point the status line, TUI and refresh pacing use)
            let shown = crate::schedule::next_launch(std::slice::from_ref(&l)).is_some();
            assert!(shown, "listed launch hidden: {c}");
            let text = crate::schedule::display_countdown(&l, now);
            assert_eq!(text.ends_with("awaiting update"), c["overdue"].as_bool().unwrap(), "{text} {c}");
        }
    }

    #[test]
    fn coarse_nets_do_not_speed_up_refresh() {
        let l = Launch { net: "2026-10-14T00:00:00Z".parse().unwrap(), net_precision: "DAY".into(), status_id: 1, ..Default::default() };
        let now: DateTime<Utc> = "2026-10-14T00:05:00Z".parse().unwrap();
        assert_eq!(crate::schedule::refresh_interval(std::slice::from_ref(&l), now), chrono::Duration::minutes(10));
        // an overdue exact NET (LL2 hasn't updated it) must not keep the 4-minute pace forever
        let m = Launch { net: "2026-10-14T00:00:00Z".parse().unwrap(), net_precision: "MIN".into(), status_id: 1, ..Default::default() };
        assert_eq!(crate::schedule::refresh_interval(std::slice::from_ref(&m), "2026-10-15T00:00:00Z".parse().unwrap()), chrono::Duration::minutes(10));
        // finished per LL2 status (3 Success, 4 Failure, 7 Partial Failure, 9 Payload Deployed): not shown
        for id in [3, 4, 7, 9] {
            let f = Launch { status_id: id, ..m.clone() };
            assert!(crate::schedule::next_launch(std::slice::from_ref(&f)).is_none(), "status {id}");
        }
    }
}
