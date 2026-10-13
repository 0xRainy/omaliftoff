//! ~/.config/liftoff/config.toml
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PROD_API: &str = "https://ll.thespacedevs.com/2.3.0";
pub const DEV_API: &str = "https://lldev.thespacedevs.com/2.3.0";
pub const NEWS_API: &str = "https://api.spaceflightnewsapi.net/v4";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// LL2 base URL. Use DEV_API (lldev) while developing; it has no rate limit but stale data.
    pub api_base: String,
    /// Optional LL2 API key (raises the 15 req/hr limit).
    pub api_key: String,
    /// How many upcoming launches to fetch (worldwide, all providers).
    pub upcoming_limit: u32,
    /// Hard cap on LL2 calls per rolling hour (free tier = 15; we keep one spare).
    pub max_calls_per_hour: u32,
    /// Fetch SNAPI news for this many upcoming launches.
    pub news_for_next: u32,
    /// Providers that get the extra T-24h / T-1h alerts (matched case-insensitively
    /// against the LL2 provider name or abbreviation).
    pub starred_providers: Vec<String>,
    pub notify: NotifyConfig,
    /// Optional live telemetry JSON endpoint (best effort, e.g. a Launch Dashboard API relay).
    pub telemetry_url: String,
    /// Fetch SpaceX flight plans (countdown/flight timeline, recovery) from spacex.com.
    pub spacex_flight_plans: bool,
    pub tui: TuiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TuiConfig {
    /// Paint a solid background behind every pane (legible over transparent terminals).
    pub solid_background: bool,
    /// Pane background colour (hex) when solid_background is on.
    pub background: String,
    /// Render mission patches (half-block art; downloads the patch image once).
    pub mission_patch: bool,
    /// Use Nerd Font glyphs (rocket marker) in the live mission timeline.
    pub nerd_font: bool,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self { solid_background: true, background: "#12151b".into(), mission_patch: true, nerd_font: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NotifyConfig {
    pub enabled: bool,
    /// Desktop notifications via notify-send (mako on Omarchy).
    pub desktop: bool,
    /// Push via ntfy.
    pub ntfy: bool,
    pub ntfy_server: String,
    /// Empty = ntfy off. Use an unguessable name on public servers (anyone who knows it can read it).
    pub ntfy_topic: String,
    /// Optional access token (self-hosted / reserved topics). Sent as `Authorization: Bearer`.
    pub ntfy_token: String,
    /// Minutes before T-0, for every launch worldwide.
    pub all_minutes: Vec<i64>,
    /// Minutes before T-0, extra for starred providers.
    pub starred_minutes: Vec<i64>,
    /// Notify when a launch we already alerted on slips or goes on hold.
    pub notify_changes: bool,
    /// Alert when a launch id appears that wasn't in any earlier fetch.
    pub new_launch_alerts: bool,
    /// "all" or "starred".
    pub new_launch_scope: String,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            desktop: true,
            ntfy: false,
            ntfy_server: "https://ntfy.sh".into(),
            ntfy_topic: String::new(),
            ntfy_token: String::new(),
            new_launch_alerts: true,
            new_launch_scope: "all".into(),
            all_minutes: vec![10],
            starred_minutes: vec![24 * 60, 60],
            notify_changes: true,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_base: PROD_API.into(),
            api_key: String::new(),
            upcoming_limit: 40,
            max_calls_per_hour: 14,
            news_for_next: 5,
            starred_providers: vec!["SpaceX".into()],
            notify: NotifyConfig::default(),
            telemetry_url: String::new(),
            spacex_flight_plans: true,
            tui: TuiConfig::default(),
        }
    }
}

impl Config {
    pub fn is_starred(&self, provider: &str, abbrev: &str) -> bool {
        is_starred(&self.starred_providers, provider, abbrev)
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let s = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn load() -> Result<Self> {
        let p = paths::config_file();
        let mut c = Self::load_from(&p)?;
        if !p.exists() {
            let _ = c.save_to(&p);
        }
        if let Ok(b) = std::env::var("LIFTOFF_API") {
            c.api_base = match b.as_str() {
                "dev" => DEV_API.into(),
                "prod" => PROD_API.into(),
                other => other.into(),
            };
        }
        Ok(c)
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let body = format!(
            "# Liftoff configuration. Restart `systemctl --user restart liftoff` after edits.\n\
             # api_base: {PROD_API} (15 req/hr free) or {DEV_API} (dev, stale data)\n{}",
            toml::to_string_pretty(self)?
        );
        std::fs::write(path, body)?;
        // may hold an ntfy token
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&paths::config_file())
    }

    /// Toggle a provider in the starred list; returns new starred state.
    pub fn toggle_star(&mut self, provider: &str) -> bool {
        let before = self.starred_providers.len();
        self.starred_providers.retain(|p| !p.eq_ignore_ascii_case(provider));
        if self.starred_providers.len() == before {
            self.starred_providers.push(provider.to_string());
            true
        } else {
            false
        }
    }
}

pub fn is_starred(list: &[String], provider: &str, abbrev: &str) -> bool {
    list.iter().any(|s| {
        let s = s.trim();
        !s.is_empty() && (s.eq_ignore_ascii_case(provider) || (!abbrev.is_empty() && s.eq_ignore_ascii_case(abbrev)))
    })
}

pub mod paths {
    use super::*;
    fn base(env: &str, fallback: fn() -> Option<PathBuf>, tail: &str) -> PathBuf {
        if let Ok(v) = std::env::var(env) {
            if !v.is_empty() {
                return PathBuf::from(v);
            }
        }
        fallback().unwrap_or_else(|| PathBuf::from("/tmp")).join(tail)
    }
    pub fn config_dir() -> PathBuf {
        base("LIFTOFF_CONFIG_DIR", dirs::config_dir, "liftoff")
    }
    pub fn cache_dir() -> PathBuf {
        base("LIFTOFF_CACHE_DIR", dirs::cache_dir, "liftoff")
    }
    pub fn state_dir() -> PathBuf {
        base("LIFTOFF_STATE_DIR", || dirs::state_dir().or_else(dirs::cache_dir), "liftoff")
    }
    pub fn config_file() -> PathBuf {
        config_dir().join("config.toml")
    }
    pub fn cache_file() -> PathBuf {
        cache_dir().join("launches.json")
    }
    pub fn state_file() -> PathBuf {
        state_dir().join("state.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_roundtrip_and_star() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("c.toml");
        let mut c = Config::default();
        assert!(c.is_starred("SpaceX", "SpX"));
        assert!(!c.is_starred("Rocket Lab", "RL"));
        assert!(c.toggle_star("Rocket Lab"));
        c.save_to(&p).unwrap();
        let back = Config::load_from(&p).unwrap();
        assert_eq!(back, c);
        assert!(back.is_starred("rocket lab", ""));
    }
    #[test]
    fn partial_config_uses_defaults() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("c.toml");
        std::fs::write(&p, "starred_providers = [\"RL\"]\n[notify]\nntfy = false\n").unwrap();
        let c = Config::load_from(&p).unwrap();
        assert!(c.is_starred("Rocket Lab", "RL"));
        assert!(!c.notify.ntfy);
        assert_eq!(c.notify.all_minutes, vec![10]);
    }

    #[test]
    fn ntfy_off_and_no_topic_by_default() {
        let n = NotifyConfig::default();
        assert!(!n.ntfy);
        assert!(n.ntfy_topic.is_empty() && n.ntfy_token.is_empty());
        assert!(n.new_launch_alerts && n.new_launch_scope == "all");
    }
}
