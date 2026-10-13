//! SpaceX flight plans from the spacex.com website's content API.
//!
//! spacex.com/launches/<slug> is a client-rendered page backed by a public
//! Strapi CMS (no auth, JSON):
//!   GET https://content.spacex.com/api/spacex-website/launches-page-tiles/upcoming
//!       -> [{ link: "<slug>", title, vehicle, launchSite, returnSite, launchDate, launchTime, ... }]
//!   GET https://content.spacex.com/api/spacex-website/missions/<slug>
//!       -> { title, preLaunchTimeline{timelineEntries[{time:"HH:MM:SS",description}]},
//!            postLaunchTimeline{...}, paragraphs[{content(html)}], webcasts, ... }
//! It is undocumented: treat every field as optional and fail soft.
use crate::api::get_json;
use crate::model::{FlightPlan, Launch, Milestone, clean};
use anyhow::{Result, anyhow};
use chrono::{NaiveDate, Utc};
use serde_json::Value;

pub const CONTENT_API: &str = "https://content.spacex.com/api/spacex-website";

pub fn is_spacex(l: &Launch) -> bool {
    l.provider.to_lowercase().contains("spacex") || l.provider_abbrev.eq_ignore_ascii_case("SpX")
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tile {
    pub slug: String,
    pub title: String,
    pub vehicle: String,
    pub launch_site: String,
    pub return_site: String,
    pub date: Option<NaiveDate>,
}

pub fn parse_tiles(v: &Value) -> Vec<Tile> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    let slug = t["link"].as_str()?.trim().to_string();
                    if slug.is_empty() || slug.contains('/') {
                        return None;
                    }
                    Some(Tile {
                        slug,
                        title: clean(t["title"].as_str().unwrap_or("")),
                        vehicle: clean(t["vehicle"].as_str().unwrap_or("")),
                        launch_site: clean(t["launchSite"].as_str().unwrap_or("")),
                        return_site: clean(t["returnSite"].as_str().unwrap_or("")),
                        date: t["launchDate"].as_str().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Lowercase alphanumeric tokens, split at letter/digit boundaries ("USSF-481" -> ussf, 481).
fn tokens(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut prev_digit = None;
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            let d = c.is_ascii_digit();
            if prev_digit.is_some_and(|p| p != d) && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(c);
            prev_digit = Some(d);
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_digit = None;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    // strip leading zeros so "sl-6-07" == "Group 6-7"
    out.into_iter().map(|t| if t.chars().all(|c| c.is_ascii_digit()) { t.trim_start_matches('0').to_string().max("0".into()) } else { t }).collect()
}

fn mission_tokens(l: &Launch) -> Vec<String> {
    let src = if l.mission.is_empty() { l.name.split('|').nth(1).unwrap_or(&l.name).to_string() } else { l.mission.clone() };
    let mut t = tokens(&src);
    if t.iter().any(|x| x == "starlink") {
        t.push("sl".into());
    }
    t
}

/// Every slug token must appear in the LL2 mission tokens (multiset), at least one
/// must be numeric or long, and dates (when SpaceX lists one) must be within 3 days.
pub fn match_tile<'a>(l: &Launch, tiles: &'a [Tile]) -> Option<&'a Tile> {
    let mt = mission_tokens(l);
    tiles
        .iter()
        .filter(|t| {
            if let Some(d) = t.date {
                // LL2 NETs coarser than a day (week/month/…) drift a lot; be lenient there.
                let tol = if matches!(l.net_precision.as_str(), "SEC" | "MIN" | "HR" | "HALF" | "TOD" | "DAY" | "") { 3 } else { 45 };
                if (d - l.net.date_naive()).num_days().abs() > tol {
                    return false;
                }
            }
            let st = tokens(&t.slug);
            if st.is_empty() || !st.iter().any(|x| x.len() >= 5 || x.chars().all(|c| c.is_ascii_digit())) {
                return false;
            }
            let mut pool = mt.clone();
            st.iter().all(|x| match pool.iter().position(|y| y == x) {
                Some(i) => {
                    pool.remove(i);
                    true
                }
                None => false,
            })
        })
        .max_by_key(|t| tokens(&t.slug).len())
}

/// "00:02:29 " / "02:29" / "\t1:02:03" -> seconds.
pub fn parse_hms(s: &str) -> Option<i64> {
    let parts: Vec<i64> = s.trim().split(':').map(|p| p.trim().parse::<i64>()).collect::<Result<_, _>>().ok()?;
    match parts.as_slice() {
        [h, m, s] => Some(h * 3600 + m * 60 + s),
        [m, s] => Some(m * 60 + s),
        [s] => Some(*s),
        _ => None,
    }
}

pub fn strip_html(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    clean(&out.replace("&nbsp;", " ").replace("&amp;", "&").replace("&#39;", "'").replace("&quot;", "\""))
}

/// Short label for a flight event ("1st stage main engine cutoff (MECO)" -> "MECO").
pub fn short_label(desc: &str) -> String {
    if let (Some(a), Some(b)) = (desc.rfind('('), desc.rfind(')')) {
        let inner = &desc[a + 1..b];
        if b > a && inner.len() <= 8 && inner.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-') {
            return inner.to_string();
        }
    }
    let d = desc.to_lowercase();
    let map = [
        ("liftoff", "Liftoff"),
        ("max q", "Max Q"),
        ("stages separate", "Stage sep"),
        ("fairing", "Fairing sep"),
        ("entry burn begin", "Entry burn"),
        ("entry burn start", "Entry burn"),
        ("entry burn end", "Entry end"),
        ("landing burn", "Landing burn"),
        ("lands", "Landing"),
        ("landing", "Landing"),
        ("deploy", "Deploy"),
        ("engine chill", "Engine chill"),
        ("propellant load", "Go prop load"),
        ("go for launch", "Go for launch"),
        ("ignition", "Ignition"),
        ("rp-1", "RP-1 load"),
        ("1st stage lox", "S1 LOX load"),
        ("2nd stage lox", "S2 LOX load"),
        ("pressurization", "Pressurize"),
        ("final prelaunch", "Startup"),
    ];
    for (k, v) in map {
        if d.contains(k) {
            return v.into();
        }
    }
    desc.chars().take(14).collect()
}

pub fn parse_mission(v: &Value, tile: &Tile) -> Result<FlightPlan> {
    if v["data"].is_null() && v["error"].is_object() {
        return Err(anyhow!("spacex.com: {}", v["error"]["message"].as_str().unwrap_or("error")));
    }
    let mut events = vec![];
    for (key, sign) in [("preLaunchTimeline", -1), ("postLaunchTimeline", 1)] {
        if let Some(a) = v[key]["timelineEntries"].as_array() {
            for e in a {
                let Some(t) = e["time"].as_str().and_then(parse_hms) else { continue };
                let desc = strip_html(e["description"].as_str().unwrap_or(""));
                if desc.is_empty() {
                    continue;
                }
                events.push(Milestone { offset_secs: sign * t, label: short_label(&desc), description: desc });
            }
        }
    }
    events.sort_by_key(|m| m.offset_secs);
    let summary: Vec<String> = v["paragraphs"]
        .as_array()
        .map(|a| a.iter().filter_map(|p| p["content"].as_str()).map(strip_html).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default();
    let recovery = summary
        .iter()
        .flat_map(|p| p.split(". "))
        .find(|s| {
            let s = s.to_lowercase();
            (s.contains("land") || s.contains("droneship") || s.contains("recover")) && !s.contains("launch window")
        })
        .map(|s| s.trim().trim_end_matches('.').to_string() + ".")
        .unwrap_or_default();
    Ok(FlightPlan {
        slug: tile.slug.clone(),
        url: format!("https://www.spacex.com/launches/{}", tile.slug),
        title: clean(v["title"].as_str().unwrap_or(&tile.title)),
        vehicle: tile.vehicle.clone(),
        launch_site: tile.launch_site.clone(),
        return_site: tile.return_site.clone(),
        recovery,
        summary,
        events,
        fetched_at: Some(Utc::now()),
    })
}

pub fn fetch_tiles() -> Result<Vec<Tile>> {
    Ok(parse_tiles(&get_json(&format!("{CONTENT_API}/launches-page-tiles/upcoming"), "")?))
}

pub fn fetch_plan(tile: &Tile) -> Result<FlightPlan> {
    parse_mission(&get_json(&format!("{CONTENT_API}/missions/{}", tile.slug), "")?, tile)
}

/// Fetch plans for the next `n` SpaceX launches. Errors per mission are skipped.
pub fn fetch_plans(launches: &[Launch], n: usize) -> Result<Vec<(String, FlightPlan)>> {
    let now = Utc::now();
    let want: Vec<&Launch> = launches.iter().filter(|l| is_spacex(l) && l.net > now - chrono::Duration::hours(3)).take(n).collect();
    if want.is_empty() {
        return Ok(vec![]);
    }
    let tiles = fetch_tiles()?;
    Ok(want
        .into_iter()
        .filter_map(|l| {
            let t = match_tile(l, &tiles)?;
            fetch_plan(t).ok().map(|p| (l.id.clone(), p))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn launch(mission: &str, net: &str) -> Launch {
        Launch { mission: mission.into(), provider: "SpaceX".into(), net: net.parse().unwrap(), ..Default::default() }
    }
    fn tiles() -> Vec<Tile> {
        parse_tiles(&serde_json::from_str(include_str!("../tests/fixtures/spacex_tiles_upcoming.json")).unwrap())
    }
    #[test]
    fn matches_ll2_to_spacex_slugs() {
        let t = tiles();
        assert!(t.len() >= 3);
        assert_eq!(match_tile(&launch("USSF-481", "2026-10-15T23:44:00Z"), &t).unwrap().slug, "ussf481");
        assert_eq!(match_tile(&launch("Starlink Group 15-15", "2026-10-17T00:00:00Z"), &t).unwrap().slug, "sl-15-15");
        assert_eq!(match_tile(&launch("Bandwagon 5 (Dedicated Mid-Inclination Rideshare)", "2026-10-19T06:30:00Z"), &t).unwrap().slug, "bandwagon5");
        // wrong group / far date / unrelated
        assert!(match_tile(&launch("Starlink Group 15-1", "2026-10-17T00:00:00Z"), &t).is_none());
        assert!(match_tile(&launch("USSF-481", "2026-12-15T00:00:00Z"), &t).is_none());
        let mut coarse = launch("Bandwagon 5 (Dedicated Mid-Inclination Rideshare)", "2026-10-31T00:00:00Z");
        coarse.net_precision = "M".into();
        assert_eq!(match_tile(&coarse, &t).unwrap().slug, "bandwagon5");
        assert!(match_tile(&launch("Unknown Payload", "2026-10-15T00:00:00Z"), &t).is_none());
    }
    #[test]
    fn parses_mission_timeline() {
        let v: Value = serde_json::from_str(include_str!("../tests/fixtures/spacex_mission_ussf481.json")).unwrap();
        let tile = tiles().into_iter().find(|t| t.slug == "ussf481").unwrap();
        let p = parse_mission(&v, &tile).unwrap();
        assert!(p.events.first().unwrap().offset_secs == -38 * 60);
        assert!(p.events.iter().any(|e| e.label == "MECO" && e.offset_secs == 146));
        assert!(p.events.iter().any(|e| e.label == "Liftoff" && e.offset_secs == 0));
        assert!(p.events.windows(2).all(|w| w[0].offset_secs <= w[1].offset_secs));
        assert!(p.recovery.to_lowercase().contains("land"), "{}", p.recovery);
        assert!(p.summary.iter().all(|s| !s.contains('<')));
        assert_eq!(p.url, "https://www.spacex.com/launches/ussf481");
    }
    #[test]
    fn hms_and_html() {
        assert_eq!(parse_hms("00:02:36\t"), Some(156));
        assert_eq!(parse_hms(" 01:04:09 "), Some(3849));
        assert_eq!(parse_hms("x"), None);
        assert_eq!(strip_html("a <a href=\"x\">X @SpaceX</a>. "), "a X @SpaceX.");
        assert_eq!(short_label("1st stage main engine cutoff (MECO)"), "MECO");
        assert_eq!(short_label("1st and 2nd stages separate"), "Stage sep");
        assert_eq!(parse_mission(&serde_json::json!({"data":null,"error":{"message":"Not Found"}}), &Tile::default()).is_err(), true);
    }
}
