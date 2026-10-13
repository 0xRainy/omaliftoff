//! Fetch loop + notification loop. The daemon is the single writer of the
//! cache and state files; other front-ends request refreshes via a flag file.
use crate::config::{Config, paths};
use crate::model::{CACHE_VERSION, Cache};
use crate::schedule::{self, State};
use crate::{api, notify, planner, spacex, store};
use anyhow::Result;
use chrono::{Duration, Utc};

pub fn refresh_request_file() -> std::path::PathBuf {
    paths::cache_dir().join("refresh.request")
}

fn unlimited(cfg: &Config) -> bool {
    cfg.api_base.contains("lldev") || !cfg.api_key.is_empty()
}

/// One fetch cycle. Returns Ok(true) if LL2 was actually called.
pub fn fetch_once(cfg: &Config, cache: &mut Cache, state: &mut State, verbose: bool) -> Result<bool> {
    let now = Utc::now();
    state.budget.prune(now);
    // Never eat into calls reserved for final pre-launch syncs.
    if !unlimited(cfg) {
        let slots = reserved_slots(cfg, cache, state, now);
        if !planner::allowed_now(&state.budget.calls, &slots, cfg.max_calls_per_hour, now) {
            let next = planner::next_unreserved(&state.budget.calls, &slots, cfg.max_calls_per_hour, now, now);
            cache.sync.note = format!("refresh held: calls reserved for final syncs; next free {}", next.with_timezone(&chrono::Local).format("%H:%M:%S"));
            cache.sync.note_at = Some(now);
            return Ok(false);
        }
    }
    if !unlimited(cfg) && state.budget.remaining(now, cfg.max_calls_per_hour) == 0 {
        cache.last_error =
            Some(format!("local budget exhausted ({} calls/hr); next at {}", cfg.max_calls_per_hour, state.budget.next_allowed(now, cfg.max_calls_per_hour)));
        return Ok(false);
    }
    // api-throttle is free; use it to avoid burning a call into a 429.
    match api::fetch_throttle(cfg) {
        Ok(t) => {
            let blocked = t.remaining() == 0 && t.next_use_secs > 0;
            cache.throttle = t;
            if blocked {
                cache.last_error = Some(format!("LL2 throttled; retry in {}s", cache.throttle.next_use_secs));
                return Ok(false);
            }
        }
        Err(e) if verbose => eprintln!("api-throttle: {e}"),
        Err(_) => {}
    }
    state.budget.record(now);
    let res = api::fetch_upcoming_timed(cfg);
    match res {
        Ok((launches, skew)) => {
            if let Some(ms) = skew {
                cache.sync.add_skew(ms);
            }
            cache.launches = launches;
            cache.fetched_at = Some(now);
            cache.last_error = None;
            cache.throttle.current_use += 1;
        }
        Err(e) => {
            cache.last_error = Some(e.to_string());
            if verbose {
                eprintln!("fetch: {e}");
            }
        }
    }
    cache.version = CACHE_VERSION;
    cache.source = cfg.api_base.clone();
    cache.starred = cfg.starred_providers.clone();

    // News: SNAPI has no hard limit but be polite (every 30 min).
    if cache.news_fetched_at.is_none_or(|t| now - t > Duration::minutes(30)) {
        let mut news = std::collections::BTreeMap::new();
        let ids: Vec<String> =
            cache.launches.iter().filter(|l| l.net > now - Duration::hours(1)).take(cfg.news_for_next as usize).map(|l| l.id.clone()).collect();
        for id in ids {
            match api::fetch_news(&id, 6) {
                Ok(a) if !a.is_empty() => {
                    news.insert(id, a);
                }
                Ok(_) => {}
                Err(e) if verbose => eprintln!("news: {e}"),
                Err(_) => {}
            }
        }
        cache.news = news;
        cache.news_fetched_at = Some(now);
    }
    refresh_flight_plans(cfg, cache, verbose);
    Ok(true)
}

/// SpaceX flight plans (content.spacex.com; separate host, not LL2 budget).
/// Hourly, every 20 min when a SpaceX launch is within 3 h. Old plans survive errors.
pub fn refresh_flight_plans(cfg: &Config, cache: &mut Cache, verbose: bool) {
    if !cfg.spacex_flight_plans {
        return;
    }
    let now = Utc::now();
    let soon = cache.launches.iter().any(|l| spacex::is_spacex(l) && (l.net - now).num_minutes().abs() < 180);
    let every = Duration::minutes(if soon { 20 } else { 60 });
    if cache.flight_plans_fetched_at.is_some_and(|t| now - t < every) {
        return;
    }
    cache.flight_plans_fetched_at = Some(now);
    match spacex::fetch_plans(&cache.launches, 5) {
        Ok(v) => {
            for (id, p) in v {
                cache.flight_plans.insert(id, p);
            }
        }
        Err(e) if verbose => eprintln!("spacex: {e}"),
        Err(_) => {}
    }
    let ids: std::collections::HashSet<&str> = cache.launches.iter().map(|l| l.id.as_str()).collect();
    cache.flight_plans.retain(|k, _| ids.contains(k.as_str()));
}

pub fn reserved_slots(cfg: &Config, cache: &Cache, state: &State, now: chrono::DateTime<Utc>) -> Vec<planner::Slot> {
    planner::plan(&cache.launches, &cfg.starred_providers, &state.budget.calls, &state.synced, cfg.max_calls_per_hour, now)
}

/// Replace one launch with a freshly synced copy. Returns the NET shift (s), which
/// re-anchors the live timeline (holds, or LL2 setting NET to the actual liftoff).
pub fn apply_sync(cache: &mut Cache, mut fresh: crate::model::Launch) -> Option<i64> {
    let old = cache.launches.iter_mut().find(|l| l.id == fresh.id)?;
    let shift = Some((fresh.net - old.net).num_seconds()).filter(|s| *s != 0);
    if fresh.milestones.is_empty() {
        fresh.milestones = old.milestones.clone();
    }
    *old = fresh;
    shift
}

/// One reserved single-launch sync: refresh that launch in place (re-anchors NET).
pub fn run_sync(cfg: &Config, cache: &mut Cache, state: &mut State, slot: &planner::Slot) {
    let now = Utc::now();
    state.budget.record(now);
    state.synced.insert(slot.key(), now);
    match api::fetch_launch(cfg, &slot.launch_id) {
        Ok((mut fresh, skew)) => {
            if let Some(ms) = skew {
                cache.sync.add_skew(ms);
            }
            let shift = apply_sync(cache, std::mem::take(&mut fresh));
            cache.throttle.current_use += 1;
            cache.sync.last_label = format!("{} · {}", slot.kind.label(), slot.launch_name);
            cache.sync.last_at = Some(now);
            cache.sync.last_net_shift_secs = shift;
            eprintln!("sync {} {}: ok{}", slot.kind.label(), slot.launch_name, shift.map(|s| format!(", NET moved {s:+}s")).unwrap_or_default());
        }
        Err(e) => {
            cache.sync.last_label = format!("{} · {} failed", slot.kind.label(), slot.launch_name);
            cache.sync.last_at = Some(now);
            eprintln!("sync {}: {e}", slot.kind.label());
        }
    }
}

fn finish(cfg: &Config, cache: &mut Cache, state: &State) {
    let now = Utc::now();
    let max = if unlimited(cfg) { 1000 } else { cfg.max_calls_per_hour };
    let wanted = schedule::next_fetch_at(cache.fetched_at, &cache.launches, &state.budget, &cache.throttle, max, now);
    let slots = if unlimited(cfg) { vec![] } else { reserved_slots(cfg, cache, state, now) };
    let at = planner::next_unreserved(&state.budget.calls, &slots, max, wanted, now);
    cache.next_fetch_at = Some(at);
    cache.sync.next_routine_at = Some(at);
    cache.sync.reserved = slots.len();
    match slots.iter().find(|s| s.at + s.kind.validity() >= now) {
        Some(s) => {
            cache.sync.next_label = format!("{} · {}", s.kind.label(), s.launch_name);
            cache.sync.next_at = Some(s.at);
        }
        None => {
            cache.sync.next_label.clear();
            cache.sync.next_at = None;
        }
    }
}

pub fn fetch_and_save(cfg: &Config, verbose: bool) -> Result<Cache> {
    let (cp, sp) = (paths::cache_file(), paths::state_file());
    let mut cache = store::load_cache(&cp);
    let mut state = store::load_state(&sp);
    fetch_once(cfg, &mut cache, &mut state, verbose)?;
    finish(cfg, &mut cache, &state);
    store::save_json(&cp, &cache)?;
    store::save_json(&sp, &state)?;
    Ok(cache)
}

/// Send due alerts for the launches in `cache`; persist dedupe state.
pub fn run_alerts(cfg: &Config, cache: &Cache, state: &mut State, dry: bool) -> Vec<schedule::Alert> {
    if !cfg.notify.enabled {
        return vec![];
    }
    let now = Utc::now();
    let mut alerts = schedule::plan_alerts(&cache.launches, &cfg.notify, &cfg.starred_providers, now, state);
    if cache.fetched_at.is_some() {
        alerts.extend(crate::newlaunch::plan(&cache.launches, &cfg.notify, &cfg.starred_providers, now, state));
    }
    for a in &alerts {
        eprintln!("alert: {} | {}", a.title, a.body);
        if !dry {
            for e in notify::send(&cfg.notify, a) {
                eprintln!("  delivery error: {e}");
            }
        }
        // mark even on delivery failure: never spam on a broken notifier
        state.mark(a, now);
    }
    alerts
}

pub fn run() -> Result<()> {
    let (cp, sp) = (paths::cache_file(), paths::state_file());
    let mut cache = store::load_cache(&cp);
    let mut state = store::load_state(&sp);
    eprintln!("liftoff daemon: cache {} state {}", cp.display(), sp.display());
    let mut first = true;
    loop {
        let cfg = Config::load().unwrap_or_else(|e| {
            eprintln!("config error ({e}); using defaults");
            Config::default()
        });
        // other processes (TUI fallback, `liftoff fetch`) may have spent calls
        state.budget.merge(&store::load_state(&sp).budget);
        let now = Utc::now();
        let req = refresh_request_file();
        let requested = req.exists();
        let slots = if unlimited(&cfg) { vec![] } else { reserved_slots(&cfg, &cache, &state, now) };
        if let Some(slot) = slots.iter().find(|s| s.at <= now).cloned() {
            run_sync(&cfg, &mut cache, &mut state, &slot);
        } else {
            let min_gap_ok = cache.fetched_at.is_none_or(|t| now - t > Duration::seconds(60));
            let due = cache.next_fetch_at.is_none_or(|t| now >= t) || first;
            if due || (requested && min_gap_ok) {
                match fetch_once(&cfg, &mut cache, &mut state, true) {
                    Ok(true) => eprintln!(
                        "fetched {} launches ({}), budget used {}/{}",
                        cache.launches.len(),
                        cache.last_error.as_deref().unwrap_or("ok"),
                        state.budget.used(now),
                        cfg.max_calls_per_hour
                    ),
                    Ok(false) if requested => eprintln!("manual refresh refused: {}", cache.sync.note),
                    _ => {}
                }
                // a refused manual refresh is dropped (served from cache), not queued
                if requested {
                    let _ = std::fs::remove_file(&req);
                }
            }
        }
        first = false;
        cache.starred = cfg.starred_providers.clone();
        finish(&cfg, &mut cache, &state);
        let _ = store::save_json(&cp, &cache);
        run_alerts(&cfg, &cache, &mut state, false);
        state.prune(Utc::now());
        let _ = store::save_json(&sp, &state);
        // wake exactly for the next reserved sync (T-5s needs sub-second timing)
        let now = Utc::now();
        let slots = if unlimited(&cfg) { vec![] } else { reserved_slots(&cfg, &cache, &state, now) };
        let mut sleep = std::time::Duration::from_secs(15);
        if let Some(s) = slots.iter().find(|s| s.at > now) {
            let ms = (s.at - now).num_milliseconds().max(50) as u64;
            sleep = sleep.min(std::time::Duration::from_millis(ms));
        }
        std::thread::sleep(sleep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Launch;
    #[test]
    fn sync_reanchors_net() {
        let net: chrono::DateTime<Utc> = "2026-10-13T18:00:00Z".parse().unwrap();
        let mut cache =
            Cache { launches: vec![Launch { id: "a".into(), net, milestones: vec![Default::default()], ..Default::default() }], ..Default::default() };
        let fresh = Launch { id: "a".into(), net: net + Duration::seconds(7), status_id: 6, ..Default::default() };
        assert_eq!(apply_sync(&mut cache, fresh), Some(7));
        assert_eq!(cache.launches[0].net, net + Duration::seconds(7));
        assert_eq!(cache.launches[0].status_id, 6);
        assert_eq!(cache.launches[0].milestones.len(), 1, "keeps timeline if single view lacks it");
        let same = cache.launches[0].clone();
        assert_eq!(apply_sync(&mut cache, same), None);
        assert_eq!(apply_sync(&mut cache, Launch { id: "zz".into(), ..Default::default() }), None);
    }
}
