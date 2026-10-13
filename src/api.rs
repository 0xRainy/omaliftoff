//! HTTP: Launch Library 2 + Spaceflight News API. Parsing is tolerant
//! (serde_json::Value) so LL2 schema drift doesn't break the bar.
use crate::config::{Config, NEWS_API};
use crate::model::{clean, status, Article, Booster, Launch, Milestone, ProviderStats, RocketStats, Throttle};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::time::Duration;

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(25))
        .user_agent(concat!("liftoff/", env!("CARGO_PKG_VERSION"), " (personal launch tracker)"))
        .build()
}

pub(crate) fn get_json(url: &str, api_key: &str) -> Result<Value> {
    get_json_timed(url, api_key).map(|(v, _)| v)
}

/// Like get_json, plus a clock-skew sample (server time − local time, ms) from the
/// HTTP Date header: Date is truncated to whole seconds, so add 500 ms and compare
/// to the request midpoint. Error ≈ ±0.5 s + RTT/2.
pub(crate) fn get_json_timed(url: &str, api_key: &str) -> Result<(Value, Option<i64>)> {
    let t_send = Utc::now();
    let mut req = agent().get(url);
    if !api_key.is_empty() {
        req = req.set("Authorization", &format!("Token {api_key}"));
    }
    match req.call() {
        Ok(r) => {
            let t_recv = Utc::now();
            let skew = r.header("Date").and_then(|d| skew_ms(d, t_send, t_recv));
            Ok((r.into_json()?, skew))
        }
        Err(ureq::Error::Status(429, r)) => {
            let body = r.into_string().unwrap_or_default();
            Err(anyhow!("rate limited (429): {}", body.chars().take(200).collect::<String>()))
        }
        Err(e) => Err(anyhow!("GET {url}: {e}")),
    }
}

pub fn skew_ms(date_header: &str, t_send: DateTime<Utc>, t_recv: DateTime<Utc>) -> Option<i64> {
    let server = DateTime::parse_from_rfc2822(date_header).ok()?.with_timezone(&Utc) + chrono::Duration::milliseconds(500);
    let mid = t_send + (t_recv - t_send) / 2;
    Some((server - mid).num_milliseconds())
}

/// One LL2 call for a single launch (used for reserved final syncs).
pub fn fetch_launch(cfg: &Config, id: &str) -> Result<(Launch, Option<i64>)> {
    let url = format!("{}/launches/{id}/?format=json&mode=detailed", cfg.api_base.trim_end_matches('/'));
    let (v, skew) = get_json_timed(&url, &cfg.api_key)?;
    Ok((parse_launch(&v).ok_or_else(|| anyhow!("LL2 launch {id}: unparseable"))?, skew))
}

pub fn upcoming_url(cfg: &Config) -> String {
    format!(
        "{}/launches/upcoming/?format=json&mode=detailed&limit={}&ordering=net",
        cfg.api_base.trim_end_matches('/'),
        cfg.upcoming_limit.clamp(1, 100)
    )
}

/// One LL2 call (counts against the 15/hr budget).
pub fn fetch_upcoming_timed(cfg: &Config) -> Result<(Vec<Launch>, Option<i64>)> {
    let (v, skew) = get_json_timed(&upcoming_url(cfg), &cfg.api_key)?;
    Ok((parse_upcoming(&v)?, skew))
}

/// /api-throttle/ — does not count against the limit.
pub fn fetch_throttle(cfg: &Config) -> Result<Throttle> {
    let v = get_json(&format!("{}/api-throttle/?format=json", cfg.api_base.trim_end_matches('/')), &cfg.api_key)?;
    Ok(Throttle {
        limit: v["your_request_limit"].as_i64().unwrap_or(15),
        current_use: v["current_use"].as_i64().unwrap_or(0),
        next_use_secs: v["next_use_secs"].as_i64().unwrap_or(0),
        checked_at: Some(Utc::now()),
    })
}

pub fn fetch_news(launch_id: &str, limit: u32) -> Result<Vec<Article>> {
    let url = format!("{NEWS_API}/articles/?launch={launch_id}&limit={limit}&ordering=-published_at");
    let v = get_json(&url, "").context("spaceflight news")?;
    Ok(parse_news(&v))
}

/// Free-text news fallback (provider / rocket) when nothing is linked to the launch yet.
pub fn search_news(q: &str, limit: u32) -> Result<Vec<Article>> {
    let q: String = q.chars().map(|c| if c.is_alphanumeric() { c } else { '+' }).collect();
    let since = (Utc::now() - chrono::Duration::days(45)).format("%Y-%m-%dT%H:%M:%SZ");
    let url = format!("{NEWS_API}/articles/?search={q}&limit={limit}&ordering=-published_at&published_at_gte={since}");
    Ok(parse_news(&get_json(&url, "")?))
}

fn s(v: &Value) -> String {
    clean(v.as_str().unwrap_or(""))
}
fn os(v: &Value) -> Option<String> {
    v.as_str().map(clean).filter(|x| !x.is_empty())
}
fn int(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))
}

/// ISO-8601 duration ("-PT38M", "P36DT8H33M6S", "PT1H2M3.5S") -> seconds.
pub fn iso_duration_secs(d: &str) -> Option<i64> {
    let d = d.trim();
    let (neg, d) = match d.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, d.strip_prefix('+').unwrap_or(d)),
    };
    let d = d.strip_prefix('P')?;
    let mut total = 0f64;
    let mut num = String::new();
    let mut in_time = false;
    for c in d.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' | '.' => num.push(c),
            _ => {
                let n: f64 = num.parse().ok()?;
                num.clear();
                total += n * match (c, in_time) {
                    ('W', false) => 604800.0,
                    ('D', false) => 86400.0,
                    ('H', true) => 3600.0,
                    ('M', true) => 60.0,
                    ('S', true) => 1.0,
                    _ => return None,
                };
            }
        }
    }
    if !num.is_empty() {
        return None;
    }
    let t = total.round() as i64;
    Some(if neg { -t } else { t })
}

/// "36d 8h" style for turnaround durations.
pub fn human_duration(secs: i64) -> String {
    let (d, h, m) = (secs / 86400, (secs % 86400) / 3600, (secs % 3600) / 60);
    if d > 0 { format!("{d}d {h}h") } else if h > 0 { format!("{h}h {m}m") } else { format!("{m}m") }
}

fn parse_boosters(r: &Value) -> Vec<Booster> {
    r["rocket"]["launcher_stage"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|st| {
                    let ld = &st["landing"];
                    let loc = &ld["landing_location"];
                    Booster {
                        kind: s(&st["type"]),
                        serial: s(&st["launcher"]["serial_number"]),
                        flight_number: int(&st["launcher_flight_number"]),
                        reused: st["reused"].as_bool().unwrap_or(false),
                        turnaround: st["turn_around_time"].as_str().and_then(iso_duration_secs).map(human_duration).unwrap_or_default(),
                        landing_attempt: ld["attempt"].as_bool().unwrap_or(false),
                        landing_success: ld["success"].as_bool(),
                        landing_description: s(&ld["description"]),
                        landing_type: os(&ld["type"]["abbrev"]).unwrap_or_else(|| s(&ld["type"]["name"])),
                        landing_location: s(&loc["name"]),
                        landing_abbrev: s(&loc["abbrev"]),
                        landing_lat: num(&loc["latitude"]),
                        landing_lon: num(&loc["longitude"]),
                        downrange_km: num(&ld["downrange_distance"]),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_milestones(r: &Value) -> Vec<Milestone> {
    let mut v: Vec<Milestone> = r["timeline"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    Some(Milestone {
                        offset_secs: iso_duration_secs(e["relative_time"].as_str()?)?,
                        label: s(&e["type"]["abbrev"]),
                        description: s(&e["type"]["description"]),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|m| m.offset_secs);
    v
}
fn dt(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str().and_then(|x| DateTime::parse_from_rfc3339(x).ok()).map(|d| d.with_timezone(&Utc))
}
fn num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|x| x.parse().ok()))
}

pub fn parse_upcoming(v: &Value) -> Result<Vec<Launch>> {
    let results = v["results"].as_array().ok_or_else(|| anyhow!("LL2 response has no results[]"))?;
    Ok(results.iter().filter_map(parse_launch).collect())
}

pub fn parse_launch(r: &Value) -> Option<Launch> {
    let net = dt(&r["net"])?;
    let status_id = r["status"]["id"].as_i64().unwrap_or(status::TBD);
    // Best webcast: prefer live, then lowest priority number (LL2: lower = better? it's "priority"
    // where higher means more important in 2.3.0) — take live first, then highest priority.
    let mut vids: Vec<&Value> = r["vid_urls"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    vids.sort_by_key(|x| {
        (
            !x["live"].as_bool().unwrap_or(false),
            -(x["priority"].as_i64().unwrap_or(0)),
        )
    });
    let webcast = vids.first().and_then(|x| os(&x["url"]));
    let any_live = vids.iter().any(|x| x["live"].as_bool().unwrap_or(false));
    let mut patches: Vec<&Value> = r["mission_patches"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    patches.sort_by_key(|x| -(x["priority"].as_i64().unwrap_or(0)));
    let rocket = os(&r["rocket"]["configuration"]["name"])
        .or_else(|| os(&r["rocket"]["configuration"]["full_name"]))
        .unwrap_or_default();
    let location = s(&r["pad"]["location"]["name"]);
    Some(Launch {
        id: s(&r["id"]),
        name: s(&r["name"]),
        mission: s(&r["mission"]["name"]),
        mission_description: s(&r["mission"]["description"]),
        mission_type: s(&r["mission"]["type"]),
        orbit: os(&r["mission"]["orbit"]["abbrev"]).unwrap_or_else(|| s(&r["mission"]["orbit"]["name"])),
        provider: s(&r["launch_service_provider"]["name"]),
        provider_abbrev: s(&r["launch_service_provider"]["abbrev"]),
        rocket,
        status_id,
        status_abbrev: s(&r["status"]["abbrev"]),
        status_name: s(&r["status"]["name"]),
        status_class: status::class(status_id).into(),
        net,
        net_precision: os(&r["net_precision"]["abbrev"]).unwrap_or_else(|| "MIN".into()),
        window_start: dt(&r["window_start"]),
        window_end: dt(&r["window_end"]),
        probability: r["probability"].as_i64().filter(|p| *p >= 0),
        weather_concerns: os(&r["weather_concerns"]),
        pad: s(&r["pad"]["name"]),
        country: os(&r["pad"]["country"]["alpha_3_code"])
            .or_else(|| os(&r["pad"]["location"]["country"]["alpha_3_code"]))
            .unwrap_or_default(),
        location,
        lat: num(&r["pad"]["latitude"]),
        lon: num(&r["pad"]["longitude"]),
        webcast_url: webcast,
        webcast_live: r["webcast_live"].as_bool().unwrap_or(false) || any_live,
        patch_url: patches.first().and_then(|p| os(&p["image_url"])),
        image_url: os(&r["image"]["image_url"]).or_else(|| os(&r["image"])),
        info_url: r["info_urls"].as_array().and_then(|a| a.first()).and_then(|x| os(&x["url"])),
        last_updated: dt(&r["last_updated"]),
        rocket_full_name: s(&r["rocket"]["configuration"]["full_name"]),
        rocket_stats: {
            let c = &r["rocket"]["configuration"];
            RocketStats {
                description: s(&c["description"]),
                length_m: num(&c["length"]),
                diameter_m: num(&c["diameter"]),
                launch_mass_t: num(&c["launch_mass"]),
                leo_kg: num(&c["leo_capacity"]),
                gto_kg: num(&c["gto_capacity"]),
                thrust_kn: num(&c["to_thrust"]),
                total: int(&c["total_launch_count"]),
                successes: int(&c["successful_launches"]),
                failures: int(&c["failed_launches"]),
                consecutive: int(&c["consecutive_successful_launches"]),
                landings_ok: int(&c["successful_landings"]),
                landings_attempted: int(&c["attempted_landings"]),
                reusable: c["reusable"].as_bool().unwrap_or(false),
            }
        },
        provider_stats: {
            let p = &r["launch_service_provider"];
            ProviderStats {
                total: int(&p["total_launch_count"]),
                successes: int(&p["successful_launches"]),
                failures: int(&p["failed_launches"]),
                consecutive: int(&p["consecutive_successful_launches"]),
                pending: int(&p["pending_launches"]),
                landings_ok: int(&p["successful_landings"]),
                landings_attempted: int(&p["attempted_landings"]),
                founded: int(&p["founding_year"]),
            }
        },
        boosters: parse_boosters(r),
        milestones: parse_milestones(r),
        pad_launch_count: int(&r["pad"]["total_launch_count"]),
        orbital_attempt_of_year: int(&r["orbital_launch_attempt_count_year"]),
    })
}

pub fn parse_news(v: &Value) -> Vec<Article> {
    v["results"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| Article {
                    title: s(&x["title"]),
                    url: s(&x["url"]),
                    news_site: s(&x["news_site"]),
                    summary: s(&x["summary"]),
                    published_at: dt(&x["published_at"]),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_ll2_fixture() {
        let v: Value = serde_json::from_str(include_str!("../tests/fixtures/upcoming.json")).unwrap();
        let ls = parse_upcoming(&v).unwrap();
        assert!(!ls.is_empty());
        let l = &ls[0];
        assert!(!l.id.is_empty() && !l.provider.is_empty() && !l.rocket.is_empty());
        assert!(l.lat.is_some() && l.lon.is_some());
        assert!(ls.iter().all(|l| !l.status_class.is_empty()));
    }
    #[test]
    fn parses_detailed_extras() {
        let v: Value = serde_json::from_str(include_str!("../tests/fixtures/upcoming.json")).unwrap();
        let ls = parse_upcoming(&v).unwrap();
        let l = &ls[0];
        assert_eq!(l.rocket_full_name, "Falcon 9 Block 5");
        assert!(l.rocket_stats.successes.unwrap() > 600);
        assert!(l.provider_stats.total.unwrap() > 700);
        let b = &l.boosters[0];
        assert_eq!(b.serial, "B1088");
        assert_eq!(b.landing_abbrev, "OCISLY");
        assert_eq!(b.flight_number, Some(20));
        assert!(!l.milestones.is_empty() && l.milestones[0].offset_secs < 0);
        assert!(l.milestones.windows(2).all(|w| w[0].offset_secs <= w[1].offset_secs));
        // no control characters anywhere (they corrupted the terminal)
        for l in &ls {
            assert!(!l.mission_description.chars().any(|c| c.is_control()), "{}", l.name);
        }
    }
    #[test]
    fn date_header_skew() {
        let t: DateTime<Utc> = "2026-10-13T18:00:00.200Z".parse().unwrap();
        // server says 18:00:03 (truncated) -> ~18:00:03.5 vs local midpoint 18:00:00.300
        assert_eq!(skew_ms("Tue, 13 Oct 2026 18:00:03 GMT", t, t + chrono::Duration::milliseconds(200)), Some(3200));
        assert_eq!(skew_ms("garbage", t, t), None);
    }
    #[test]
    fn iso_durations() {
        assert_eq!(iso_duration_secs("-PT38M"), Some(-2280));
        assert_eq!(iso_duration_secs("PT2M29S"), Some(149));
        assert_eq!(iso_duration_secs("P36DT8H33M6S"), Some(36 * 86400 + 8 * 3600 + 33 * 60 + 6));
        assert_eq!(iso_duration_secs("PT0S"), Some(0));
        assert_eq!(iso_duration_secs("garbage"), None);
        assert_eq!(human_duration(36 * 86400 + 8 * 3600), "36d 8h");
    }
    #[test]
    fn picks_live_webcast_first() {
        let v: Value = serde_json::json!({"id":"x","net":"2026-10-13T18:00:00Z","status":{"id":1},
          "vid_urls":[{"url":"a","priority":10,"live":false},{"url":"b","priority":1,"live":true}]});
        let l = parse_launch(&v).unwrap();
        assert_eq!(l.webcast_url.as_deref(), Some("b"));
        assert!(l.webcast_live);
    }
}
