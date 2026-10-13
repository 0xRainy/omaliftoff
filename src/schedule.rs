//! Pure, clock-injected logic: countdown text, refresh cadence, API budget and
//! notification planning. Everything here is unit tested; nothing does I/O.
use crate::config::NotifyConfig;
use crate::model::{Launch, Throttle, status};
use chrono::{DateTime, Duration, Local, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ---------------------------------------------------------------- countdown

/// "T-02:14:07", "T-3d 04h", "T+00:05:12".
/// Whole seconds since T-0, floored (negative before launch). This is the one
/// countdown rule shared with the bar (Model.js `relSecs`): the display flips
/// exactly on whole-second boundaries of (now - net), T-1 lasts until T-0.
pub fn rel_secs(net: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    (now - net).num_milliseconds().div_euclid(1000)
}

/// Milliseconds until just after the next whole second of `now` (pass the
/// skew-corrected clock); same as Model.js `msToNextSecond`.
pub fn ms_to_next_second(now: DateTime<Utc>) -> u64 {
    1000 - now.timestamp_millis().rem_euclid(1000) as u64 + 5
}

pub fn format_countdown(net: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = -rel_secs(net, now);
    let (sign, s) = if secs >= 0 { ('-', secs) } else { ('+', -secs) };
    let (d, h, m, sec) = (s / 86400, (s % 86400) / 3600, (s % 3600) / 60, s % 60);
    if d > 0 { format!("T{sign}{d}d {h:02}h") } else { format!("T{sign}{h:02}:{m:02}:{sec:02}") }
}

/// Countdown that does not invent precision: coarse NETs show a date instead.
pub fn display_countdown(l: &Launch, now: DateTime<Utc>) -> String {
    if l.precise() || (l.net - now).num_hours().abs() < 1 {
        format_countdown(l.net, now)
    } else {
        let local = l.net.with_timezone(&Local);
        match l.net_precision.as_str() {
            "HALF" | "TOD" | "DAY" => format!("NET {}", local.format("%a %b %-d")),
            "W" | "M" => format!("NET {}", local.format("%b %Y")),
            _ => format!("NET {}", local.format("%Y")),
        }
    }
}

/// Human "in 2h 05m" / "in 9m" text used by notifications.
pub fn human_lead(net: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let m = ((net - now).num_seconds() + 30) / 60;
    if m <= 0 {
        return "now".into();
    }
    let (d, h, mm) = (m / 1440, (m % 1440) / 60, m % 60);
    if d > 0 {
        format!("in {d}d {h}h")
    } else if h > 0 {
        format!("in {h}h {mm:02}m")
    } else {
        format!("in {mm}m")
    }
}

/// The next launch that is still worth counting down to (not finished, and
/// not more than `grace` past its NET unless it is in flight).
pub fn next_launch(launches: &[Launch], now: DateTime<Utc>) -> Option<&Launch> {
    launches
        .iter()
        .filter(|l| !status::is_finished(l.status_id))
        .filter(|l| l.status_id == status::IN_FLIGHT || l.net > now - Duration::minutes(30))
        .min_by_key(|l| l.net)
}

// ---------------------------------------------------------------- refresh

/// Adaptive refresh: ~10 min normally, faster as T-0 approaches.
pub fn refresh_interval(launches: &[Launch], now: DateTime<Utc>) -> Duration {
    let Some(l) = next_launch(launches, now) else {
        return Duration::minutes(10);
    };
    let lead = l.net - now;
    if l.status_id == status::IN_FLIGHT || lead <= Duration::minutes(15) {
        Duration::minutes(4)
    } else if lead <= Duration::hours(1) {
        Duration::minutes(5)
    } else if lead <= Duration::hours(6) {
        Duration::minutes(8)
    } else {
        Duration::minutes(10)
    }
}

/// Rolling one-hour call ledger so we never exceed LL2's 15 req/hr.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Budget {
    pub calls: Vec<DateTime<Utc>>,
}

impl Budget {
    pub fn prune(&mut self, now: DateTime<Utc>) {
        self.calls.retain(|t| *t > now - Duration::hours(1));
    }
    pub fn used(&self, now: DateTime<Utc>) -> usize {
        self.calls.iter().filter(|t| **t > now - Duration::hours(1)).count()
    }
    pub fn remaining(&self, now: DateTime<Utc>, max: u32) -> u32 {
        (max as usize).saturating_sub(self.used(now)) as u32
    }
    /// Earliest instant a new call is allowed.
    pub fn next_allowed(&self, now: DateTime<Utc>, max: u32) -> DateTime<Utc> {
        let mut recent: Vec<_> = self.calls.iter().filter(|t| **t > now - Duration::hours(1)).copied().collect();
        if recent.len() < max as usize {
            return now;
        }
        recent.sort();
        // the call that has to age out
        recent[recent.len() - max as usize] + Duration::hours(1)
    }
    /// Union with a ledger another process wrote (TUI fallback fetch, `liftoff fetch`).
    pub fn merge(&mut self, other: &Budget) {
        for t in &other.calls {
            if !self.calls.contains(t) {
                self.calls.push(*t);
            }
        }
        self.calls.sort();
    }
    pub fn record(&mut self, now: DateTime<Utc>) {
        self.prune(now);
        self.calls.push(now);
    }
}

/// When should the next LL2 fetch happen?
pub fn next_fetch_at(
    last_fetch: Option<DateTime<Utc>>,
    launches: &[Launch],
    budget: &Budget,
    throttle: &Throttle,
    max_per_hour: u32,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    let wanted = match last_fetch {
        None => now,
        Some(t) => t + refresh_interval(launches, now),
    };
    let mut at = wanted.max(budget.next_allowed(now, max_per_hour));
    if let Some(checked) = throttle.checked_at {
        if throttle.next_use_secs > 0 {
            at = at.max(checked + Duration::seconds(throttle.next_use_secs));
        }
    }
    at
}

// ---------------------------------------------------------------- notifications

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Fired {
    pub net: DateTime<Utc>,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct State {
    /// key -> when/at-which-NET it fired. Keys: "<id>:lead:<min>", "<id>:hold:<net>", "<id>:slip:<net>".
    pub fired: BTreeMap<String, Fired>,
    pub budget: Budget,
    /// Reserved syncs done: Slot::key() -> when.
    #[serde(default)]
    pub synced: BTreeMap<String, DateTime<Utc>>,
    /// Launch ids seen in any fetch -> last seen (for "new launch" alerts).
    #[serde(default)]
    pub seen: BTreeMap<String, DateTime<Utc>>,
    #[serde(default)]
    pub seen_seeded: bool,
}

impl State {
    pub fn prune(&mut self, now: DateTime<Utc>) {
        self.fired.retain(|_, f| f.at > now - Duration::days(7));
        self.budget.prune(now);
        self.synced.retain(|_, t| *t > now - Duration::days(1));
    }
    pub fn mark(&mut self, a: &Alert, now: DateTime<Utc>) {
        self.fired.insert(a.key.clone(), Fired { net: a.net, at: now });
        for k in &a.also_covers {
            self.fired.insert(k.clone(), Fired { net: a.net, at: now });
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AlertKind {
    Lead(i64),
    Hold,
    Slip { from: DateTime<Utc> },
    New,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub key: String,
    pub launch_id: String,
    pub kind: AlertKind,
    pub net: DateTime<Utc>,
    pub title: String,
    pub body: String,
    pub url: Option<String>,
    pub critical: bool,
    /// Lead keys this alert also satisfies (a "delayed" notice replaces the
    /// lead alert that would otherwise fire in the same instant).
    pub also_covers: Vec<String>,
}

fn precision_ok(prec: &str, minutes: i64) -> bool {
    let rank = match prec {
        "SEC" | "MIN" | "" => 0,
        "HR" => 1,
        "HALF" | "TOD" | "DAY" => 2,
        _ => 3,
    };
    match minutes {
        m if m <= 30 => rank == 0,
        m if m <= 180 => rank <= 1,
        _ => rank <= 2,
    }
}

/// How far the NET may move before an already-fired lead alert re-arms.
fn rearm_tolerance(minutes: i64) -> Duration {
    Duration::seconds((minutes * 60 / 4).max(120))
}

fn describe(l: &Launch) -> String {
    let mut parts = vec![l.status_name.clone()];
    if !l.pad.is_empty() {
        let loc = l.location.split(',').next().unwrap_or("").trim();
        parts.push(if loc.is_empty() { l.pad.clone() } else { format!("{} · {}", l.pad, loc) });
    }
    if let Some(p) = l.probability {
        parts.push(format!("{p}% go"));
    }
    parts.join(" · ")
}

fn title_name(l: &Launch) -> String {
    if l.mission.is_empty() { l.name.clone() } else { format!("{} · {}", l.short_rocket(), l.mission) }
}

/// Decide which alerts are due now. Pure: caller marks them in `state` after sending.
pub fn plan_alerts(launches: &[Launch], cfg: &NotifyConfig, starred: &[String], now: DateTime<Utc>, state: &State) -> Vec<Alert> {
    let mut out = Vec::new();
    for l in launches {
        if status::is_finished(l.status_id) || l.status_id == status::IN_FLIGHT {
            continue;
        }
        let is_star = crate::config::is_starred(starred, &l.provider, &l.provider_abbrev);
        let mut th: Vec<i64> = cfg.all_minutes.clone();
        if is_star {
            th.extend(cfg.starred_minutes.iter().copied());
        }
        th.retain(|m| *m > 0);
        th.sort_unstable();
        th.dedup();
        let local = l.net.with_timezone(&Local).format("%H:%M").to_string();

        // ---- change alerts (only for launches we already told the user about)
        if cfg.notify_changes {
            let prev: Vec<&Fired> =
                th.iter().filter_map(|m| state.fired.get(&format!("{}:lead:{m}", l.id))).filter(|f| f.at > now - Duration::hours(48)).collect();
            if let Some(last) = prev.iter().max_by_key(|f| f.at) {
                if l.status_id == status::HOLD {
                    let key = format!("{}:hold:{}", l.id, l.net.timestamp());
                    if !state.fired.contains_key(&key) {
                        out.push(Alert {
                            key,
                            launch_id: l.id.clone(),
                            kind: AlertKind::Hold,
                            net: l.net,
                            title: format!("⏸ Hold · {}", title_name(l)),
                            body: format!("{} is on hold. NET {}", l.name, local),
                            url: l.webcast_url.clone(),
                            critical: false,
                            also_covers: vec![],
                        });
                    }
                } else if l.net - last.net > Duration::minutes(5) {
                    let key = format!("{}:slip:{}", l.id, l.net.timestamp());
                    if !state.fired.contains_key(&key) {
                        let when =
                            if (l.net - now) > Duration::hours(20) { l.net.with_timezone(&Local).format("%a %b %-d %H:%M").to_string() } else { local.clone() };
                        out.push(Alert {
                            key,
                            launch_id: l.id.clone(),
                            kind: AlertKind::Slip { from: last.net },
                            net: l.net,
                            title: format!("🕒 Delayed · {}", title_name(l)),
                            body: format!("New NET {when} ({}). {}", human_lead(l.net, now), describe(l)),
                            url: l.webcast_url.clone(),
                            critical: false,
                            also_covers: vec![],
                        });
                    }
                }
            }
        }

        // ---- lead-time alerts: only the most specific threshold that applies
        let change_idx = out.iter().rposition(|a| a.launch_id == l.id && matches!(a.kind, AlertKind::Slip { .. }));
        let lead = l.net - now;
        if lead <= Duration::zero() || l.status_id == status::HOLD {
            continue;
        }
        let Some(&m) = th.iter().find(|m| lead <= Duration::minutes(**m)) else {
            continue;
        };
        if !precision_ok(&l.net_precision, m) {
            continue;
        }
        let key = format!("{}:lead:{m}", l.id);
        if let Some(f) = state.fired.get(&key) {
            if (f.net - l.net).abs() <= rearm_tolerance(m) {
                continue;
            }
        }
        // Don't fire a coarser alert if a finer one already fired for this same NET.
        let finer_fired =
            th.iter().filter(|x| **x < m).any(|x| state.fired.get(&format!("{}:lead:{x}", l.id)).is_some_and(|f| (f.net - l.net).abs() <= rearm_tolerance(*x)));
        if finer_fired {
            continue;
        }
        if let Some(i) = change_idx {
            out[i].also_covers.push(key);
            continue;
        }
        let label = if m >= 60 && m % 60 == 0 { format!("T-{}h", m / 60) } else { format!("T-{m}m") };
        out.push(Alert {
            key,
            launch_id: l.id.clone(),
            kind: AlertKind::Lead(m),
            net: l.net,
            title: format!("🚀 {label} · {}", title_name(l)),
            body: format!("Liftoff {} ({local}). {}", human_lead(l.net, now), describe(l)),
            url: l.webcast_url.clone().or_else(|| l.info_url.clone()),
            critical: m <= 10,
            also_covers: vec![],
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 13, 18, 0, 0).unwrap()
    }
    fn launch(id: &str, provider: &str, net: DateTime<Utc>) -> Launch {
        Launch {
            id: id.into(),
            name: format!("Rocket | {id}"),
            mission: id.into(),
            provider: provider.into(),
            rocket: "Falcon 9".into(),
            status_id: status::GO,
            status_name: "Go for Launch".into(),
            net,
            net_precision: "MIN".into(),
            webcast_url: Some("https://example.com/live".into()),
            ..Default::default()
        }
    }
    fn cfg() -> NotifyConfig {
        NotifyConfig::default()
    }
    fn star() -> Vec<String> {
        vec!["SpaceX".into()]
    }
    /// Run the planner, mark everything, return the keys.
    fn step(ls: &[Launch], now: DateTime<Utc>, st: &mut State) -> Vec<String> {
        let a = plan_alerts(ls, &cfg(), &star(), now, st);
        for x in &a {
            st.mark(x, now);
        }
        a.into_iter().map(|a| a.key).collect()
    }

    #[test]
    fn countdown_formats() {
        let n = t0();
        assert_eq!(format_countdown(n + Duration::seconds(2 * 3600 + 14 * 60 + 7), n), "T-02:14:07");
        assert_eq!(format_countdown(n + Duration::hours(76), n), "T-3d 04h");
        assert_eq!(format_countdown(n - Duration::seconds(312), n), "T+00:05:12");
        assert_eq!(format_countdown(n, n), "T-00:00:00");
    }

    /// Same instants and expected strings as omarchy-plugin/tests/model.test.js
    /// ("countdown matches the TUI rule"); keep the two tables identical.
    #[test]
    fn countdown_rule_matches_bar() {
        let n = t0();
        let ms = |v: i64| n + Duration::milliseconds(v);
        for (off_ms, want) in [
            (8_534_700i64, "T-02:22:15"),
            (8_534_000, "T-02:22:14"),
            (1_999, "T-00:00:02"),
            (1_000, "T-00:00:01"),
            (999, "T-00:00:01"),
            (1, "T-00:00:01"),
            (0, "T-00:00:00"),
            (-1, "T-00:00:00"),
            (-999, "T-00:00:00"),
            (-1_000, "T+00:00:01"),
            (-1_001, "T+00:00:01"),
            (-312_400, "T+00:05:12"),
        ] {
            assert_eq!(format_countdown(ms(off_ms), n), want, "net = now + {off_ms} ms");
        }
        // with the daemon's clock offset applied to now (TUI) == applied to net (bar)
        assert_eq!(format_countdown(n + Duration::seconds(60), n + Duration::milliseconds(2947)), "T-00:00:58");
        assert_eq!(ms_to_next_second(n + Duration::milliseconds(2947)), 58);
        assert_eq!(ms_to_next_second(n), 1005);
    }

    #[test]
    fn coarse_precision_shows_date() {
        let mut l = launch("a", "X", t0() + Duration::days(3));
        l.net_precision = "DAY".into();
        assert!(display_countdown(&l, t0()).starts_with("NET "));
        l.net_precision = "MIN".into();
        assert!(display_countdown(&l, t0()).starts_with("T-3d"));
    }

    #[test]
    fn human_lead_text() {
        assert_eq!(human_lead(t0() + Duration::minutes(10), t0()), "in 10m");
        assert_eq!(human_lead(t0() + Duration::minutes(125), t0()), "in 2h 05m");
        assert_eq!(human_lead(t0() + Duration::hours(25), t0()), "in 1d 1h");
    }

    #[test]
    fn next_launch_skips_finished() {
        let mut done = launch("done", "X", t0() + Duration::minutes(5));
        done.status_id = status::SUCCESS;
        let later = launch("later", "X", t0() + Duration::hours(3));
        let ls = vec![done, later];
        assert_eq!(next_launch(&ls, t0()).unwrap().id, "later");
    }

    #[test]
    fn refresh_speeds_up_near_t0() {
        let far = vec![launch("a", "X", t0() + Duration::days(2))];
        let near = vec![launch("a", "X", t0() + Duration::minutes(40))];
        let imminent = vec![launch("a", "X", t0() + Duration::minutes(5))];
        assert_eq!(refresh_interval(&far, t0()), Duration::minutes(10));
        assert_eq!(refresh_interval(&near, t0()), Duration::minutes(5));
        assert_eq!(refresh_interval(&imminent, t0()), Duration::minutes(4));
        assert_eq!(refresh_interval(&[], t0()), Duration::minutes(10));
    }

    #[test]
    fn adaptive_polling_never_exceeds_budget() {
        // Simulate 3 hours with an imminent launch; daemon fetches whenever allowed.
        let ls = vec![launch("a", "X", t0() + Duration::minutes(20))];
        let mut b = Budget::default();
        let mut last = None;
        let mut now = t0() - Duration::hours(2);
        let end = t0() + Duration::hours(1);
        while now < end {
            let at = next_fetch_at(last, &ls, &b, &Throttle::default(), 14, now);
            if at <= now {
                b.record(now);
                last = Some(now);
            }
            assert!(b.used(now) <= 14);
            now += Duration::seconds(15);
        }
        assert!(b.used(now) >= 11, "should poll quickly near T-0");
    }

    #[test]
    fn budget_blocks_when_exhausted_and_honours_throttle() {
        let mut b = Budget::default();
        for i in 0..14 {
            b.record(t0() + Duration::minutes(i));
        }
        let now = t0() + Duration::minutes(20);
        assert_eq!(b.remaining(now, 14), 0);
        assert_eq!(b.next_allowed(now, 14), t0() + Duration::hours(1));
        let thr = Throttle { limit: 15, current_use: 15, next_use_secs: 600, checked_at: Some(now) };
        let at = next_fetch_at(None, &[], &Budget::default(), &thr, 14, now);
        assert_eq!(at, now + Duration::minutes(10));
    }

    #[test]
    fn t10_fires_once_for_every_launch() {
        let ls = vec![launch("rl", "Rocket Lab", t0() + Duration::minutes(9))];
        let mut st = State::default();
        assert_eq!(step(&ls, t0(), &mut st), vec!["rl:lead:10"]);
        assert!(step(&ls, t0() + Duration::minutes(1), &mut st).is_empty());
        assert!(step(&ls, t0() + Duration::minutes(5), &mut st).is_empty());
    }

    #[test]
    fn non_starred_gets_no_24h_or_1h() {
        let ls = vec![launch("rl", "Rocket Lab", t0() + Duration::minutes(50))];
        let mut st = State::default();
        assert!(step(&ls, t0(), &mut st).is_empty());
        let ls = vec![launch("rl", "Rocket Lab", t0() + Duration::hours(20))];
        assert!(step(&ls, t0(), &mut st).is_empty());
    }

    #[test]
    fn starred_gets_24h_1h_10m_sequence() {
        let net = t0() + Duration::hours(30);
        let ls = vec![launch("sx", "SpaceX", net)];
        let mut st = State::default();
        let mut fired = vec![];
        let mut now = t0();
        while now < net + Duration::minutes(5) {
            fired.extend(step(&ls, now, &mut st));
            now += Duration::seconds(30);
        }
        assert_eq!(fired, vec!["sx:lead:1440", "sx:lead:60", "sx:lead:10"]);
    }

    #[test]
    fn late_start_fires_only_most_specific() {
        // daemon starts 30 min before a starred launch: only T-1h, then T-10m.
        let ls = vec![launch("sx", "SpaceX", t0() + Duration::minutes(30))];
        let mut st = State::default();
        assert_eq!(step(&ls, t0(), &mut st), vec!["sx:lead:60"]);
        assert_eq!(step(&ls, t0() + Duration::minutes(21), &mut st), vec!["sx:lead:10"]);
    }

    #[test]
    fn coarse_net_does_not_fire_minute_alerts() {
        let mut l = launch("sx", "SpaceX", t0() + Duration::minutes(8));
        l.net_precision = "DAY".into();
        let mut st = State::default();
        assert!(step(&[l.clone()], t0(), &mut st).is_empty());
        l.net = t0() + Duration::hours(20);
        assert_eq!(step(&[l], t0(), &mut st), vec!["sx:lead:1440"]);
    }

    #[test]
    fn small_slip_does_not_refire_but_scrub_recycle_does() {
        let mut l = launch("sx", "SpaceX", t0() + Duration::minutes(8));
        let mut st = State::default();
        assert_eq!(step(&[l.clone()], t0(), &mut st), vec!["sx:lead:10"]);
        // NET slips by 1 minute: no duplicate T-10
        l.net += Duration::minutes(1);
        assert!(step(&[l.clone()], t0() + Duration::minutes(1), &mut st).is_empty());
        // Scrub: recycle +40 min -> one "delayed" notice now ...
        l.net = t0() + Duration::minutes(49);
        let k = step(&[l.clone()], t0() + Duration::minutes(5), &mut st);
        assert_eq!(k.len(), 1);
        assert!(k[0].starts_with("sx:slip:"));
        // ... which also covers the new T-1h; T-10 re-arms and fires again.
        let mut all = vec![];
        let mut now = t0() + Duration::minutes(6);
        while now < l.net {
            all.extend(step(&[l.clone()], now, &mut st));
            now += Duration::seconds(30);
        }
        assert_eq!(all, vec!["sx:lead:10".to_string()]);
    }

    #[test]
    fn hold_notifies_once_and_suppresses_lead() {
        let mut l = launch("sx", "SpaceX", t0() + Duration::minutes(50));
        let mut st = State::default();
        assert_eq!(step(&[l.clone()], t0(), &mut st), vec!["sx:lead:60"]);
        l.status_id = status::HOLD;
        l.status_name = "On Hold".into();
        let k = step(&[l.clone()], t0() + Duration::minutes(42), &mut st);
        assert_eq!(k.len(), 1);
        assert!(k[0].starts_with("sx:hold:"));
        assert!(step(&[l.clone()], t0() + Duration::minutes(43), &mut st).is_empty());
    }

    #[test]
    fn no_change_alerts_for_launches_never_alerted() {
        let mut l = launch("rl", "Rocket Lab", t0() + Duration::days(3));
        l.status_id = status::HOLD;
        let mut st = State::default();
        assert!(step(&[l], t0(), &mut st).is_empty());
    }

    #[test]
    fn finished_and_past_launches_are_ignored() {
        let mut a = launch("a", "SpaceX", t0() + Duration::minutes(5));
        a.status_id = status::SUCCESS;
        let b = launch("b", "SpaceX", t0() - Duration::minutes(5));
        let mut st = State::default();
        assert!(step(&[a, b], t0(), &mut st).is_empty());
    }

    #[test]
    fn alert_content_has_title_and_webcast() {
        let mut l = launch("sx", "SpaceX", t0() + Duration::minutes(10));
        l.probability = Some(90);
        let a = plan_alerts(&[l], &cfg(), &star(), t0(), &State::default());
        assert_eq!(a.len(), 1);
        assert!(a[0].title.contains("T-10m"));
        assert!(a[0].title.contains("Falcon 9"));
        assert!(a[0].body.contains("90% go"));
        assert!(a[0].critical);
        assert_eq!(a[0].url.as_deref(), Some("https://example.com/live"));
    }

    #[test]
    fn state_prunes_old_entries() {
        let mut st = State::default();
        st.fired.insert("old".into(), Fired { net: t0(), at: t0() - Duration::days(8) });
        st.fired.insert("new".into(), Fired { net: t0(), at: t0() });
        st.prune(t0());
        assert!(st.fired.contains_key("new") && !st.fired.contains_key("old"));
    }
}
