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

/// Config keys whose values are secrets. They can only be set through stdin
/// (`config set-secret`): a command-line argument is visible to every local user
/// via /proc/<pid>/cmdline and `ps`.
pub const SECRET_KEYS: &[&str] = &["notify.ntfy_topic", "notify.ntfy_token", "api_key"];

pub fn is_secret_key(key: &str) -> bool {
    SECRET_KEYS.contains(&key)
}

/// The single way to set a secret: value comes from `input` (stdin), one line, stored as
/// a TOML string. Never echoes the value.
pub fn set_secret(cfg: &Config, key: &str, mut input: impl std::io::BufRead) -> Result<Config> {
    if !is_secret_key(key) {
        return Err(anyhow!("{key} is not a secret; use `liftoff config set {key} <value>`"));
    }
    let mut b = String::new();
    input.read_line(&mut b)?;
    let v = b.trim_end_matches(['\n', '\r']).trim();
    let s = toml::Value::String(v.to_string()).to_string();
    set_key(cfg, key, &s)
}

pub fn config_cmd(args: &[String]) -> Result<()> {
    let cfg = Config::load()?;
    match (args.first().map(String::as_str), args.get(1), args.get(2)) {
        (Some("get"), Some(k), _) => println!("{}", get_key(&cfg, k)?),
        (Some("set-secret"), Some(k), None) => {
            set_secret(&cfg, k, std::io::stdin().lock())?.save()?;
            println!("{k} updated (restart: systemctl --user restart liftoff)");
        }
        // the value already sits in our argv, so refuse rather than silently accept it
        (Some("set"), Some(k), Some(_)) if is_secret_key(k) => {
            return Err(anyhow!(
                "{k} is a secret: pipe it on stdin instead, e.g. `liftoff config set-secret {k} < file` (command-line arguments are visible to other users)"
            ));
        }
        (Some("set"), Some(k), Some(v)) => {
            set_key(&cfg, k, v)?.save()?;
            println!("{k} updated (restart: systemctl --user restart liftoff)");
        }
        (Some("path"), _, _) => println!("{}", paths::config_file().display()),
        _ => return Err(anyhow!("usage: liftoff config get <key> | set <key> <value> | set-secret <key> (value on stdin) | path")),
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
                    "{} launches, last successful fetch {age} min ago{}",
                    cache.launches.len(),
                    cache.last_error.as_ref().map(|e| format!("; latest attempt failed: {e}")).unwrap_or_default()
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
    fn secrets_only_via_stdin() {
        let c = Config::default();
        let c = set_secret(&c, "notify.ntfy_topic", &b"liftoff-sentinel\n"[..]).unwrap();
        assert_eq!(c.notify.ntfy_topic, "liftoff-sentinel");
        // quotes/backslashes/TOML syntax are stored literally, not parsed
        let c = set_secret(&c, "notify.ntfy_token", &br#"tk_"a\b = [1]"#[..]).unwrap();
        assert_eq!(c.notify.ntfy_token, r#"tk_"a\b = [1]"#);
        let c = set_secret(&c, "api_key", &b"k\r\n"[..]).unwrap();
        assert_eq!(c.api_key, "k");
        assert!(set_secret(&c, "notify.desktop", &b"true\n"[..]).is_err(), "non-secret via set-secret");
        for k in SECRET_KEYS {
            assert!(is_secret_key(k));
            assert!(get_key(&c, k).is_ok(), "{k} must be a real config key");
        }
    }

    #[test]
    fn config_set_refuses_secret_in_argv() {
        let e = config_cmd(&["set".into(), "notify.ntfy_topic".into(), "liftoff-sentinel".into()]).unwrap_err().to_string();
        assert!(e.contains("set-secret") && !e.contains("liftoff-sentinel"), "{e}");
    }
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
