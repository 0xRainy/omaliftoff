//! Manual T-0 nudge, shared with the bar widget via a tiny JSON file.
//! Stored as an absolute T-0 so a later LL2 sync doesn't double-shift it; if
//! LL2's NET moves by more than RECYCLE (a hold / new attempt) the nudge is dropped.
use crate::config::paths;
use crate::model::Launch;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

pub const RECYCLE_SECS: i64 = 120;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Nudge {
    pub launch_id: String,
    /// Effective T-0 chosen by the user.
    pub t0: Option<DateTime<Utc>>,
    /// LL2 NET when the nudge was made.
    pub base_net: Option<DateTime<Utc>>,
    pub set_at: Option<DateTime<Utc>>,
}

pub fn file() -> std::path::PathBuf {
    paths::cache_dir().join("nudge.json")
}
pub fn load() -> Nudge {
    crate::store::load_json(&file())
}
pub fn save(n: &Nudge) -> anyhow::Result<()> {
    crate::store::save_json(&file(), n)
}

impl Nudge {
    /// Offset (s) of the effective T-0 vs LL2's NET for `l`, if this nudge applies.
    pub fn applies(&self, l: &Launch) -> Option<DateTime<Utc>> {
        if self.launch_id != l.id {
            return None;
        }
        let (t0, base) = (self.t0?, self.base_net?);
        ((l.net - base).num_seconds().abs() <= RECYCLE_SECS).then_some(t0)
    }
    pub fn offset_secs(&self, l: &Launch) -> Option<i64> {
        self.applies(l).map(|t0| (t0 - l.net).num_seconds())
    }
    /// Shift T-0 by `secs` (positive = later).
    pub fn shifted(&self, l: &Launch, secs: i64, now: DateTime<Utc>) -> Nudge {
        let cur = self.applies(l).unwrap_or(l.net);
        let base = if self.applies(l).is_some() { self.base_net } else { Some(l.net) };
        Nudge { launch_id: l.id.clone(), t0: Some(cur + Duration::seconds(secs)), base_net: base, set_at: Some(now) }
    }
    /// "Liftoff is now."
    pub fn mark(&self, l: &Launch, now: DateTime<Utc>) -> Nudge {
        let base = if self.applies(l).is_some() { self.base_net } else { Some(l.net) };
        Nudge { launch_id: l.id.clone(), t0: Some(now), base_net: base, set_at: Some(now) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn l(net: &str) -> Launch {
        Launch { id: "a".into(), net: net.parse().unwrap(), ..Default::default() }
    }
    #[test]
    fn shift_mark_and_recycle() {
        let now: DateTime<Utc> = "2026-10-13T18:00:00Z".parse().unwrap();
        let a = l("2026-10-13T18:00:00Z");
        let n = Nudge::default().shifted(&a, 1, now).shifted(&a, 1, now).shifted(&a, -3, now);
        assert_eq!(n.offset_secs(&a), Some(-1));
        // T-5s / T+90s sync revises NET by 2 s: absolute T-0 kept, offset re-expressed
        let a2 = l("2026-10-13T18:00:02Z");
        assert_eq!(n.applies(&a2), Some("2026-10-13T17:59:59Z".parse().unwrap()));
        // hold / recycle: NET moved 15 min -> nudge ignored
        assert_eq!(n.applies(&l("2026-10-13T18:15:00Z")), None);
        // mark liftoff 3.4 s late
        let m = n.mark(&a, now + Duration::milliseconds(3400));
        assert_eq!(m.applies(&a).unwrap(), now + Duration::milliseconds(3400));
        assert_eq!(m.base_net, n.base_net);
        // other launch unaffected
        let mut b = a.clone();
        b.id = "b".into();
        assert_eq!(m.applies(&b), None);
    }
}
