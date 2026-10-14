//! "New launch added" alerts: a launch id we haven't seen before appears in a fetch.
use crate::config::NotifyConfig;
use crate::model::Launch;
use crate::schedule::{Alert, AlertKind, State};
use chrono::{DateTime, Duration, Utc};

/// More than this many new launches in one fetch → one summary notification.
pub const BATCH_OVER: usize = 3;

pub fn describe(l: &Launch) -> String {
    let mission = if l.mission.is_empty() { l.name.as_str() } else { l.mission.as_str() };
    format!("{} · {} · NET {}", l.short_rocket(), mission, l.net_text(crate::model::NetFmt::Short))
}

/// Updates `state.seen`; returns the alerts to send (0, 1 per launch, or 1 summary).
/// First run (nothing seeded yet) records every id silently.
pub fn plan(launches: &[Launch], cfg: &NotifyConfig, starred: &[String], now: DateTime<Utc>, state: &mut State) -> Vec<Alert> {
    let first = !state.seen_seeded;
    let mut fresh: Vec<&Launch> = vec![];
    for l in launches {
        if l.id.is_empty() || l.id.starts_with("test") {
            continue;
        }
        if state.seen.insert(l.id.clone(), now).is_none() && !first {
            fresh.push(l);
        } else {
            state.seen.insert(l.id.clone(), now); // refresh last-seen
        }
    }
    state.seen_seeded = true;
    state.seen.retain(|_, t| *t > now - Duration::days(90));
    if !cfg.new_launch_alerts {
        return vec![];
    }
    if cfg.new_launch_scope.eq_ignore_ascii_case("starred") {
        fresh.retain(|l| crate::config::is_starred(starred, &l.provider, &l.provider_abbrev));
    }
    let mk = |key: String, id: String, net, title: String, body: String, url| Alert {
        key,
        launch_id: id,
        kind: AlertKind::New,
        net,
        title,
        body,
        url,
        critical: false,
        also_covers: vec![],
    };
    if fresh.len() > BATCH_OVER {
        let mut body: Vec<String> = fresh.iter().take(6).map(|l| describe(l)).collect();
        if fresh.len() > 6 {
            body.push(format!("…and {} more (liftoff tui)", fresh.len() - 6));
        }
        return vec![mk(
            format!("new:batch:{}", now.timestamp()),
            "new-batch".into(),
            fresh[0].net,
            format!("󱓞 {} new launches added", fresh.len()),
            body.join("\n"),
            None,
        )];
    }
    fresh.iter().map(|l| mk(format!("{}:new", l.id), l.id.clone(), l.net, "󱓞 New launch added".into(), describe(l), l.webcast_url.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn l(id: &str, prov: &str) -> Launch {
        Launch {
            id: id.into(),
            name: format!("Rocket | {id}"),
            mission: id.into(),
            provider: prov.into(),
            rocket: "Falcon 9 Block 5".into(),
            net: Utc::now(),
            net_precision: "MIN".into(),
            ..Default::default()
        }
    }
    fn cfg() -> NotifyConfig {
        NotifyConfig::default()
    }

    #[test]
    fn coarse_net_shows_only_known_precision() {
        use chrono::TimeZone;
        let mut x = l("a", "SpaceX");
        x.net = Utc.with_ymd_and_hms(2026, 12, 30, 23, 0, 0).unwrap();
        let y = x.net.with_timezone(&chrono::Local).format("%Y").to_string();
        for (p, bad) in [("Y", "(year)"), ("M", "(month)"), ("Q4", "(quarter)")] {
            x.net_precision = p.into();
            let d = describe(&x);
            assert!(!d.contains(bad) && !d.contains(':'), "{p}: {d}");
            assert!(d.contains(&y), "{p}: {d}");
        }
        x.net_precision = "Y".into();
        assert!(describe(&x).ends_with(&format!("NET {y}")));
    }

    #[test]
    fn coarse_net_uses_utc_calendar() {
        use chrono::TimeZone;
        let mut x = l("a", "SpaceX");
        // midnight UTC on Jan 1 is still Dec 31 in every zone west of UTC
        x.net = Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
        for (p, want) in [("Y", "2027"), ("M", "Jan 2027"), ("Q1", "Q1 2027"), ("H1", "H1 2027")] {
            x.net_precision = p.into();
            assert_eq!(x.net_text(crate::model::NetFmt::Short), want, "{p}");
        }
        x.net = Utc.with_ymd_and_hms(2026, 12, 31, 23, 59, 59).unwrap();
        for (p, want) in [("Y", "2026"), ("M", "Dec 2026"), ("Q4", "Q4 2026"), ("H2", "H2 2026")] {
            x.net_precision = p.into();
            assert_eq!(x.net_text(crate::model::NetFmt::Short), want, "{p}");
        }
    }

    #[test]
    fn every_ll2_precision_code_renders_without_fake_time() {
        use chrono::TimeZone;
        // LL2 2.3.0 /config/net_precisions/ (17 codes), dated the way LL2 stores them
        let d = |y, m, dd| Utc.with_ymd_and_hms(y, m, dd, 0, 0, 0).unwrap();
        let cases = [
            ("AM", d(2026, 11, 3), "~Nov 3 (morning, site time)"),
            ("PM", d(2026, 11, 3), "~Nov 3 (afternoon, site time)"),
            ("DAY", d(2026, 11, 7), "Nov 7"),
            ("WK", d(2026, 11, 9), "~Nov 9 (week)"),
            ("M", d(2026, 12, 31), "Dec 2026"),
            ("Q1", d(2027, 3, 31), "Q1 2027"),
            ("Q2", d(2027, 6, 30), "Q2 2027"),
            ("Q3", d(2027, 9, 30), "Q3 2027"),
            ("Q4", d(2026, 12, 31), "Q4 2026"),
            ("H1", d(2027, 6, 30), "H1 2027"),
            ("H2", d(2026, 12, 31), "H2 2026"),
            ("Y", d(2026, 12, 31), "2026"),
            ("FY", d(2027, 6, 30), "~2027 (fiscal year)"),
            ("DEC", d(2029, 12, 31), "~2020s"),
            // legacy/fake codes stay harmless
            ("W", d(2026, 11, 9), "~Nov 9 (week)"),
            ("TOD", d(2026, 11, 7), "Nov 7"),
            ("Q", d(2026, 12, 31), "Q4 2026"),
            ("ZZZ", d(2026, 11, 7), "~Nov 7"),
        ];
        let mut x = l("a", "SpaceX");
        for (p, net, want) in cases {
            x.net_precision = p.into();
            x.net = net;
            assert_eq!(x.net_text(crate::model::NetFmt::Short), want, "{p}");
            assert!(!describe(&x).contains(':'), "{p}: {}", describe(&x));
        }
        // minute / second / hour keep local clock time
        x.net = d(2026, 11, 5) + chrono::Duration::minutes(210);
        let local = x.net.with_timezone(&chrono::Local);
        for p in ["SEC", "MIN", ""] {
            x.net_precision = p.into();
            assert_eq!(x.net_text(crate::model::NetFmt::Short), local.format("%b %-d %H:%M").to_string(), "{p}");
        }
        x.net_precision = "HR".into();
        assert!(x.net_text(crate::model::NetFmt::Short).ends_with("(±1h)"));
    }

    #[test]
    fn first_run_seeds_silently() {
        let mut st = State::default();
        let ls: Vec<_> = (0..40).map(|i| l(&format!("id{i}"), "SpaceX")).collect();
        assert!(plan(&ls, &cfg(), &[], Utc::now(), &mut st).is_empty());
        assert_eq!(st.seen.len(), 40);
        assert!(st.seen_seeded);
    }

    #[test]
    fn dedupes_and_alerts_once() {
        let mut st = State::default();
        let now = Utc::now();
        plan(&[l("a", "SpaceX")], &cfg(), &[], now, &mut st);
        let a = plan(&[l("a", "SpaceX"), l("b", "Rocket Lab")], &cfg(), &[], now, &mut st);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].launch_id, "b");
        assert!(a[0].body.contains("Falcon 9") && a[0].body.contains("NET"));
        assert!(plan(&[l("a", "SpaceX"), l("b", "Rocket Lab")], &cfg(), &[], now, &mut st).is_empty());
        // empty fetch after seeding does not reseed
        plan(&[], &cfg(), &[], now, &mut st);
        assert_eq!(plan(&[l("c", "SpaceX")], &cfg(), &[], now, &mut st).len(), 1);
    }

    #[test]
    fn batches_more_than_three() {
        let mut st = State::default();
        let now = Utc::now();
        plan(&[l("a", "x")], &cfg(), &[], now, &mut st);
        let three: Vec<_> = ["b", "c", "d"].iter().map(|i| l(i, "x")).collect();
        assert_eq!(plan(&three, &cfg(), &[], now, &mut st).len(), 3);
        let four: Vec<_> = ["e", "f", "g", "h"].iter().map(|i| l(i, "x")).collect();
        let a = plan(&four, &cfg(), &[], now, &mut st);
        assert_eq!(a.len(), 1);
        assert!(a[0].title.contains("4 new launches"));
        assert_eq!(a[0].body.lines().count(), 4);
    }

    #[test]
    fn scope_and_toggle() {
        let now = Utc::now();
        let mut st = State::default();
        let mut c = cfg();
        c.new_launch_scope = "starred".into();
        plan(&[], &c, &[], now, &mut st);
        let a = plan(&[l("a", "SpaceX"), l("b", "Rocket Lab")], &c, &["SpaceX".into()], now, &mut st);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].launch_id, "a");
        c.new_launch_alerts = false;
        assert!(plan(&[l("z", "SpaceX")], &c, &["SpaceX".into()], now, &mut st).is_empty());
        assert!(st.seen.contains_key("z"), "still recorded while off: no flood when re-enabled");
        assert!(plan(&[l("test-flight", "SpaceX")], &cfg(), &[], now, &mut st).is_empty());
    }
}
