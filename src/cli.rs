//! `liftoff config`, `liftoff setup`, `liftoff doctor`.
use crate::config::{Config, paths};
use crate::{api, store};
use anyhow::{Result, anyhow};
use chrono::Utc;
use std::process::Command;

pub const PLUGIN_ID: &str = "liftoff.tracker";

/// Parse a CLI value as TOML (bool, number, array, quoted string); bare words become strings.
fn parse_value(raw: &str) -> toml::Value {
    toml::from_str::<toml::Table>(&format!("v = {raw}")).ok().and_then(|mut t| t.remove("v")).unwrap_or_else(|| toml::Value::String(raw.to_string()))
}

/// Set a dotted key (e.g. `notify.ntfy_topic`) on a config, validating the result.
pub fn set_key(cfg: &Config, key: &str, raw: &str) -> Result<Config> {
    let mut root = toml::Value::try_from(cfg)?;
    let (parent, last) = key.rsplit_once('.').map(|(a, b)| (Some(a), b)).unwrap_or((None, key));
    let mut cur = &mut root;
    for p in parent.into_iter().flat_map(|p| p.split('.')) {
        cur = cur.get_mut(p).ok_or_else(|| anyhow!("unknown key {key}"))?;
    }
    let t = cur.as_table_mut().ok_or_else(|| anyhow!("{key}: not a table"))?;
    if !t.contains_key(last) {
        return Err(anyhow!("unknown key {key}"));
    }
    t.insert(last.to_string(), parse_value(raw));
    Ok(root.try_into()?)
}

pub fn get_key(cfg: &Config, key: &str) -> Result<String> {
    let root = toml::Value::try_from(cfg)?;
    let mut cur = &root;
    for p in key.split('.') {
        cur = cur.get(p).ok_or_else(|| anyhow!("unknown key {key}"))?;
    }
    Ok(match cur {
        toml::Value::String(s) => s.clone(),
        v => v.to_string(),
    })
}

pub fn config_cmd(args: &[String]) -> Result<()> {
    let cfg = Config::load()?;
    match (args.first().map(String::as_str), args.get(1), args.get(2)) {
        (Some("get"), Some(k), _) => println!("{}", get_key(&cfg, k)?),
        (Some("set"), Some(k), Some(v)) => {
            // "-" reads the value from stdin (keeps tokens out of the process list)
            let v = if v == "-" {
                let mut b = String::new();
                std::io::stdin().read_line(&mut b)?;
                format!("{:?}", b.trim())
            } else {
                v.clone()
            };
            set_key(&cfg, k, &v)?.save()?;
            println!("{k} updated (restart: systemctl --user restart liftoff)");
        }
        (Some("path"), _, _) => println!("{}", paths::config_file().display()),
        _ => return Err(anyhow!("usage: liftoff config get <key> | set <key> <value> | path")),
    }
    Ok(())
}

/// The wizard lives in install.sh (gum UI); the installer keeps a copy here.
pub fn setup() -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_default();
    let data = std::env::var("XDG_DATA_HOME").unwrap_or(format!("{home}/.local/share"));
    let cfg_home = std::env::var("XDG_CONFIG_HOME").unwrap_or(format!("{home}/.config"));
    // prefer the plugin checkout (`omarchy plugin add`), else the installer's saved copy
    let script = [
        std::path::PathBuf::from(&cfg_home).join("omarchy/plugins").join(PLUGIN_ID).join("install.sh"),
        std::path::PathBuf::from(data).join("liftoff/install.sh"),
    ]
    .into_iter()
    .find(|p| p.exists())
    .unwrap_or_default();
    if !script.exists() {
        return Err(anyhow!(
            "{} not found; re-run the installer, or edit {} (notify.ntfy, notify.ntfy_topic)",
            script.display(),
            paths::config_file().display()
        ));
    }
    let st = Command::new("bash").arg(&script).arg("--setup").status()?;
    if !st.success() {
        return Err(anyhow!("setup exited with {st}"));
    }
    Ok(())
}

fn have(cmd: &str) -> bool {
    Command::new("sh").arg("-c").arg(format!("command -v {cmd} >/dev/null 2>&1")).status().map(|s| s.success()).unwrap_or(false)
}
fn out(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).output().ok()?;
    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

pub fn doctor() -> Result<()> {
    let mut bad = 0;
    let mut line = |ok: Option<bool>, what: &str, msg: String| {
        let tag = match ok {
            Some(true) => "\x1b[32m ok \x1b[0m",
            Some(false) => {
                bad += 1;
                "\x1b[31mFAIL\x1b[0m"
            }
            None => "\x1b[33mwarn\x1b[0m",
        };
        println!("[{tag}] {what:<12} {msg}");
    };
    let cfg = match Config::load() {
        Ok(c) => {
            line(Some(true), "config", paths::config_file().display().to_string());
            c
        }
        Err(e) => {
            line(Some(false), "config", format!("{e}"));
            return Err(anyhow!("config unreadable"));
        }
    };
    // service
    let active = out("systemctl", &["--user", "is-active", "liftoff.service"]).unwrap_or_default();
    line(Some(active == "active"), "service", format!("liftoff.service is {}", if active.is_empty() { "unknown" } else { &active }));
    // cache
    let cache = store::load_cache(&paths::cache_file());
    let now = Utc::now();
    match cache.fetched_at {
        Some(t) => {
            let age = (now - t).num_minutes();
            line(
                Some(age < 45),
                "cache",
                format!(
                    "{} launches, fetched {age} min ago{}",
                    cache.launches.len(),
                    cache.last_error.as_ref().map(|e| format!(" (last error: {e})")).unwrap_or_default()
                ),
            );
        }
        None => line(Some(false), "cache", "never fetched — run `liftoff fetch`".into()),
    }
    // budget
    let st = store::load_state(&paths::state_file());
    let used = st.budget.used(now);
    line(
        if used as u32 >= cfg.max_calls_per_hour { None } else { Some(true) },
        "budget",
        format!("{used}/{} LL2 calls in the last hour (local ledger)", cfg.max_calls_per_hour),
    );
    match api::fetch_throttle(&cfg) {
        Ok(t) => line(Some(true), "LL2", format!("reachable; server says {}/{} used", t.current_use, t.limit)),
        Err(e) => line(Some(false), "LL2", format!("{e}")),
    }
    // plugin
    if have("omarchy") {
        let list = out("omarchy", &["plugin", "list", "--json"]).unwrap_or_default();
        let enabled = serde_json::from_str::<serde_json::Value>(&list).ok().and_then(|v| {
            let arr = v.as_array().cloned().or_else(|| v.get("plugins").and_then(|p| p.as_array().cloned()))?;
            arr.iter().find(|p| p["id"] == PLUGIN_ID).map(|p| p["enabled"].as_bool().unwrap_or(false))
        });
        match enabled {
            Some(true) => line(Some(true), "plugin", format!("{PLUGIN_ID} enabled")),
            Some(false) => line(Some(false), "plugin", format!("{PLUGIN_ID} installed but disabled: omarchy plugin enable {PLUGIN_ID}")),
            None => line(Some(false), "plugin", format!("{PLUGIN_ID} not found (omarchy plugin list)")),
        }
    } else {
        line(None, "plugin", "omarchy CLI not found (TUI/daemon still work)".into());
    }
    // notifications
    line(
        if !cfg.notify.desktop { None } else { Some(have("notify-send")) },
        "desktop",
        if cfg.notify.desktop { "notify-send".into() } else { "desktop alerts disabled".into() },
    );
    if cfg.notify.ntfy && !cfg.notify.ntfy_topic.is_empty() {
        let url = format!("{}/v1/health", cfg.notify.ntfy_server.trim_end_matches('/'));
        let r = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(8)).build().get(&url).call();
        match r {
            Ok(_) => line(
                Some(true),
                "ntfy",
                format!("{} reachable (topic set{})", cfg.notify.ntfy_server, if cfg.notify.ntfy_token.is_empty() { "" } else { ", token" }),
            ),
            Err(ureq::Error::Status(c, _)) => line(None, "ntfy", format!("{url} answered {c} (server may not expose /v1/health)")),
            Err(e) => line(Some(false), "ntfy", format!("{e}")),
        }
    } else {
        line(None, "ntfy", "phone push off (enable with `liftoff setup`)".into());
    }
    // clock
    if let Some(s) = out("timedatectl", &["show", "-p", "NTPSynchronized", "--value"]).filter(|s| !s.is_empty()) {
        line(if s == "yes" { Some(true) } else { None }, "clock", format!("NTP synchronized: {s}"));
    }
    if bad > 0 { Err(anyhow!("{bad} check(s) failed")) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn set_and_get_keys() {
        let c = Config::default();
        let c = set_key(&c, "notify.ntfy", "true").unwrap();
        assert!(c.notify.ntfy);
        let c = set_key(&c, "notify.ntfy_topic", "liftoff-abc").unwrap();
        assert_eq!(get_key(&c, "notify.ntfy_topic").unwrap(), "liftoff-abc");
        let c = set_key(&c, "starred_providers", r#"["SpaceX", "Rocket Lab"]"#).unwrap();
        assert_eq!(c.starred_providers, vec!["SpaceX", "Rocket Lab"]);
        assert!(set_key(&c, "notify.nope", "1").is_err());
        assert!(set_key(&c, "notify.ntfy", "\"maybe\"").is_err()); // type checked
    }
}
