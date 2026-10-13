//! LL2 call planner. LL2 allows 15 calls per rolling hour per IP; we keep
//! `max_calls_per_hour` (14) as our own cap. Final syncs before a launch are
//! *reserved*: any other call (routine refresh, manual refresh, TUI fallback)
//! is only allowed if, after making it, every reserved sync still fits in its
//! own rolling hour window.
use crate::config::is_starred;
use crate::model::{Launch, status};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SyncKind {
    /// T-5 min: catch holds / recycled NET before the terminal count.
    TMinus5m,
    /// T-60 s: last chance to see a hold.
    TMinus60s,
    /// T-5 s: final anchor for the live timeline.
    TMinus5s,
    /// T+90 s: confirm liftoff (status In Flight / revised NET = actual liftoff).
    TPlus90s,
}

impl SyncKind {
    /// Importance order used when the budget can't fit everything.
    pub const PRIORITY: [SyncKind; 4] = [SyncKind::TMinus5s, SyncKind::TMinus60s, SyncKind::TPlus90s, SyncKind::TMinus5m];
    pub fn offset(self) -> Duration {
        match self {
            SyncKind::TMinus5m => Duration::minutes(-5),
            SyncKind::TMinus60s => Duration::seconds(-60),
            SyncKind::TMinus5s => Duration::seconds(-5),
            SyncKind::TPlus90s => Duration::seconds(90),
        }
    }
    /// How long after its slot a missed sync is still worth doing.
    pub fn validity(self) -> Duration {
        match self {
            SyncKind::TMinus5m => Duration::minutes(3),
            SyncKind::TMinus60s => Duration::seconds(45),
            SyncKind::TMinus5s => Duration::seconds(30),
            SyncKind::TPlus90s => Duration::minutes(5),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            SyncKind::TMinus5m => "T-5m",
            SyncKind::TMinus60s => "T-60s",
            SyncKind::TMinus5s => "T-5s",
            SyncKind::TPlus90s => "T+90s",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub launch_id: String,
    pub launch_name: String,
    pub kind: SyncKind,
    pub at: DateTime<Utc>,
}

impl Slot {
    /// Done-key includes the NET minute, so a recycled NET gets fresh syncs.
    pub fn key(&self) -> String {
        let net = self.at - self.kind.offset();
        format!("{}:{:?}:{}", self.launch_id, self.kind, net.timestamp() / 60)
    }
}

/// Launches that get final syncs: precise NET, not finished, inside the planning
/// horizon. Starred providers first, then soonest.
pub fn targets<'a>(launches: &'a [Launch], starred: &[String], now: DateTime<Utc>) -> Vec<&'a Launch> {
    let mut v: Vec<&Launch> = launches
        .iter()
        .filter(|l| !status::is_finished(l.status_id) && matches!(l.net_precision.as_str(), "SEC" | "MIN" | ""))
        .filter(|l| l.net > now - Duration::minutes(7) && l.net < now + Duration::minutes(75))
        .collect();
    v.sort_by_key(|l| (!is_starred(starred, &l.provider, &l.provider_abbrev), l.net));
    v
}

fn count_in(times: &[DateTime<Utc>], end: DateTime<Utc>) -> usize {
    times.iter().filter(|t| **t > end - Duration::hours(1) && **t <= end).count()
}

/// Would `calls` (made) + `planned` (future, clamped to >= now) respect `max`
/// in every rolling hour that matters?
pub fn feasible(calls: &[DateTime<Utc>], planned: &[DateTime<Utc>], max: u32, now: DateTime<Utc>) -> bool {
    let planned: Vec<DateTime<Utc>> = planned.iter().map(|t| (*t).max(now)).collect();
    let mut all: Vec<DateTime<Utc>> = calls.iter().filter(|t| **t > now - Duration::hours(1)).copied().collect();
    all.extend(planned.iter().copied());
    // a window's count only increases at a call instant, so checking ends at each future call suffices
    planned.iter().chain(std::iter::once(&now)).all(|end| count_in(&all, *end) <= max as usize)
}

/// Reserved syncs still to do, greedy by launch priority then sync importance.
pub fn plan(
    launches: &[Launch],
    starred: &[String],
    calls: &[DateTime<Utc>],
    done: &BTreeMap<String, DateTime<Utc>>,
    max: u32,
    now: DateTime<Utc>,
) -> Vec<Slot> {
    let mut accepted: Vec<Slot> = vec![];
    // Starred launches claim all their syncs first; the rest share what's left
    // round-robin by importance (every launch's T-5s before anyone's T-60s, ...).
    let ts = targets(launches, starred, now);
    let (star, rest): (Vec<&Launch>, Vec<&Launch>) = ts.into_iter().partition(|l| is_starred(starred, &l.provider, &l.provider_abbrev));
    let mut order: Vec<(&Launch, SyncKind)> = star.iter().flat_map(|l| SyncKind::PRIORITY.map(|k| (*l, k))).collect();
    for k in SyncKind::PRIORITY {
        order.extend(rest.iter().map(|l| (*l, k)));
    }
    for (l, kind) in order {
        {
            let s = Slot { launch_id: l.id.clone(), launch_name: l.name.clone(), kind, at: l.net + kind.offset() };
            if now > s.at + kind.validity() || done.contains_key(&s.key()) {
                continue;
            }
            let mut times: Vec<DateTime<Utc>> = accepted.iter().map(|a| a.at).collect();
            times.push(s.at);
            if feasible(calls, &times, max, now) {
                accepted.push(s);
            }
        }
    }
    accepted.sort_by_key(|s| s.at);
    accepted
}

/// May a non-reserved call (routine/manual/TUI) happen now?
pub fn allowed_now(calls: &[DateTime<Utc>], slots: &[Slot], max: u32, now: DateTime<Utc>) -> bool {
    let mut times: Vec<DateTime<Utc>> = slots.iter().map(|s| s.at).collect();
    times.push(now);
    feasible(calls, &times, max, now)
}

/// Earliest time >= `wanted` at which a non-reserved call is allowed (30 s steps, 2 h cap).
pub fn next_unreserved(calls: &[DateTime<Utc>], slots: &[Slot], max: u32, wanted: DateTime<Utc>, now: DateTime<Utc>) -> DateTime<Utc> {
    let mut t = wanted.max(now);
    let end = t + Duration::hours(2);
    while t < end {
        // simulate "now = t": calls older than an hour before t no longer count
        let pending: Vec<Slot> = slots.iter().filter(|s| s.at + s.kind.validity() >= t).cloned().collect();
        let mut past: Vec<DateTime<Utc>> = calls.to_vec();
        past.extend(slots.iter().filter(|s| s.at < t).map(|s| s.at));
        let fut: Vec<Slot> = pending.into_iter().filter(|s| s.at >= t).collect();
        if allowed_now(&past, &fut, max, t) {
            return t;
        }
        t += Duration::seconds(30);
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t0() -> DateTime<Utc> {
        "2026-10-13T18:00:00Z".parse().unwrap()
    }
    fn l(id: &str, provider: &str, net: DateTime<Utc>) -> Launch {
        Launch { id: id.into(), name: id.into(), provider: provider.into(), net, net_precision: "MIN".into(), status_id: 1, ..Default::default() }
    }
    fn starred() -> Vec<String> {
        vec!["SpaceX".into()]
    }

    #[test]
    fn single_launch_gets_all_four_syncs() {
        let now = t0();
        let ls = vec![l("a", "SpaceX", now + Duration::minutes(30))];
        let p = plan(&ls, &starred(), &[], &BTreeMap::new(), 14, now);
        assert_eq!(p.len(), 4);
        assert_eq!(p.iter().find(|s| s.kind == SyncKind::TMinus5s).unwrap().at, now + Duration::minutes(30) - Duration::seconds(5));
    }

    #[test]
    fn reserve_is_never_consumed_by_routine_or_manual() {
        let net = t0() + Duration::minutes(40);
        let ls = vec![l("a", "SpaceX", net)];
        let mut calls: Vec<DateTime<Utc>> = vec![];
        // hammer: try a non-reserved call every 30 s for 50 min; execute slots when due
        let mut done = BTreeMap::new();
        let mut t = t0();
        let mut syncs = 0;
        while t < net + Duration::minutes(10) {
            let slots = plan(&ls, &starred(), &calls, &done, 14, t);
            if let Some(s) = slots.iter().find(|s| s.at <= t) {
                calls.push(t);
                done.insert(s.key(), t);
                syncs += 1;
            } else if allowed_now(&calls, &slots, 14, t) {
                calls.push(t);
            }
            // invariant: never more than 14 in any rolling hour
            assert!(count_in(&calls, t) <= 14, "at {t}");
            t += Duration::seconds(5);
        }
        assert_eq!(syncs, 4, "all reserved syncs happened");
        for k in SyncKind::PRIORITY {
            let at = net + k.offset();
            assert!(calls.iter().any(|c| *c >= at && *c < at + Duration::seconds(5)), "{k:?} on time");
        }
    }

    #[test]
    fn many_launches_in_one_hour_prioritise_starred_and_t5s() {
        let now = t0();
        let ls: Vec<Launch> = (0..6).map(|i| l(&format!("x{i}"), if i == 5 { "SpaceX" } else { "Other" }, now + Duration::minutes(20 + i * 5))).collect();
        let p = plan(&ls, &starred(), &[], &BTreeMap::new(), 14, now);
        assert!(p.len() <= 14);
        // starred launch keeps all four
        assert_eq!(p.iter().filter(|s| s.launch_id == "x5").count(), 4);
        // every launch gets at least its T-5s sync before lower-value slots of others
        for i in 0..6 {
            assert!(p.iter().any(|s| s.launch_id == format!("x{i}") && s.kind == SyncKind::TMinus5s), "x{i}");
        }
        let mut times: Vec<_> = p.iter().map(|s| s.at).collect();
        times.sort();
        assert!(feasible(&[], &times, 14, now));
    }

    #[test]
    fn exhausted_budget_degrades_gracefully() {
        let now = t0();
        let calls: Vec<DateTime<Utc>> = (0..13).map(|i| now - Duration::minutes(50) + Duration::minutes(i)).collect();
        let ls = vec![l("a", "SpaceX", now + Duration::minutes(3))];
        let p = plan(&ls, &starred(), &calls, &BTreeMap::new(), 14, now);
        // only one call left in the window ending at T-5s -> that's the one kept
        assert_eq!(p.iter().filter(|s| s.at < now + Duration::minutes(10)).map(|s| s.kind).next(), Some(SyncKind::TMinus5s));
        assert!(!allowed_now(&calls, &p, 14, now), "manual refresh denied");
    }

    #[test]
    fn far_or_coarse_launches_reserve_nothing() {
        let now = t0();
        let mut c = l("c", "SpaceX", now + Duration::minutes(30));
        c.net_precision = "DAY".into();
        let ls = vec![l("far", "SpaceX", now + Duration::hours(3)), c];
        assert!(plan(&ls, &starred(), &[], &BTreeMap::new(), 14, now).is_empty());
        assert!(allowed_now(&[], &[], 14, now));
    }

    #[test]
    fn next_unreserved_waits_for_room() {
        let now = t0();
        let calls: Vec<DateTime<Utc>> = (0..14).map(|i| now - Duration::minutes(59) + Duration::minutes(i)).collect();
        let at = next_unreserved(&calls, &[], 14, now, now);
        assert!(at >= now + Duration::minutes(1) && at <= now + Duration::seconds(90));
    }

    #[test]
    fn recycled_net_gets_new_syncs() {
        let now = t0();
        let a = l("a", "SpaceX", now + Duration::minutes(10));
        let s = &plan(std::slice::from_ref(&a), &starred(), &[], &BTreeMap::new(), 14, now)[0];
        let mut done = BTreeMap::new();
        done.insert(s.key(), now);
        let mut b = a.clone();
        b.net += Duration::minutes(15);
        let p = plan(&[b], &starred(), &[], &done, 14, now);
        assert!(p.iter().any(|x| x.kind == s.kind));
    }
}
