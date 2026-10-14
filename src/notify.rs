//! Delivery: notify-send (mako) and ntfy.
use crate::config::NotifyConfig;
use crate::schedule::Alert;
use anyhow::{Result, anyhow};
use std::process::Command;

/// App name for notify-send and the ntfy title prefix.
pub const APP_NAME: &str = "OmaLiftoff";

pub fn send(cfg: &NotifyConfig, a: &Alert) -> Vec<String> {
    let mut errs = vec![];
    if cfg.desktop {
        if let Err(e) = desktop(a) {
            errs.push(format!("notify-send: {e}"));
        }
    }
    if cfg.ntfy && !cfg.ntfy_topic.is_empty() {
        if let Err(e) = ntfy(cfg, a) {
            errs.push(format!("ntfy: {e}"));
        }
    }
    errs
}

/// notify-send with a "Watch" action. notify-send blocks while an action is
/// pending, so the wait happens on a detached thread.
pub fn desktop(a: &Alert) -> Result<()> {
    let mut args: Vec<String> = vec![
        "-a".into(),
        APP_NAME.into(),
        "-i".into(),
        "rocket".into(),
        "-u".into(),
        if a.critical { "critical" } else { "normal" }.into(),
        // replace earlier notifications for the same launch instead of stacking
        "-h".into(),
        format!("string:x-canonical-private-synchronous:liftoff-{}", a.launch_id),
        "-h".into(),
        format!("string:x-dunst-stack-tag:liftoff-{}", a.launch_id),
    ];
    if a.url.is_some() {
        args.push("-A".into());
        args.push("watch=Watch".into());
    }
    args.push(a.title.clone());
    args.push(a.body.clone());
    let url = a.url.clone();
    let mut child =
        Command::new("notify-send").args(&args).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().map_err(|e| anyhow!("{e}"))?;
    std::thread::spawn(move || {
        use std::io::Read;
        let mut out = String::new();
        if let Some(mut so) = child.stdout.take() {
            let _ = so.read_to_string(&mut out);
        }
        let _ = child.wait();
        if out.trim() == "watch" {
            if let Some(u) = url {
                let _ = Command::new("xdg-open").arg(u).spawn();
            }
        }
    });
    Ok(())
}

pub fn ntfy_payload(cfg: &NotifyConfig, a: &Alert) -> serde_json::Value {
    let mut v = serde_json::json!({
        "topic": cfg.ntfy_topic,
        "title": format!("{APP_NAME} · {}", a.title),
        "message": a.body,
        "tags": ["rocket"],
        "priority": if a.critical { 5 } else { 3 },
    });
    if let Some(u) = &a.url {
        v["click"] = u.clone().into();
        v["actions"] = serde_json::json!([{ "action": "view", "label": "Watch", "url": u, "clear": true }]);
    }
    v
}

/// JSON publish (POST to the server root) so UTF-8 titles/emoji survive.
pub fn ntfy(cfg: &NotifyConfig, a: &Alert) -> Result<()> {
    if cfg.ntfy_topic.trim().is_empty() {
        return Err(anyhow!("ntfy: no topic configured (run `liftoff setup`)"));
    }
    let url = cfg.ntfy_server.trim_end_matches('/').to_string();
    let mut req = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(15)).build().post(&url);
    if !cfg.ntfy_token.trim().is_empty() {
        req = req.set("Authorization", &format!("Bearer {}", cfg.ntfy_token.trim()));
    }
    req.send_json(ntfy_payload(cfg, a)).map_err(|e| crate::api::http_error("POST", &url, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::AlertKind;
    #[test]
    fn ntfy_payload_has_click_and_action() {
        let a = Alert {
            key: "k".into(),
            launch_id: "id".into(),
            kind: AlertKind::Lead(10),
            net: chrono::Utc::now(),
            title: "🚀 T-10m · Falcon 9".into(),
            body: "b".into(),
            url: Some("https://x.com/live".into()),
            critical: true,
            also_covers: vec![],
        };
        let cfg = NotifyConfig { ntfy_topic: "example-topic".into(), ..Default::default() };
        let p = ntfy_payload(&cfg, &a);
        assert_eq!(p["topic"], "example-topic");
        assert_eq!(p["title"], "OmaLiftoff · 🚀 T-10m · Falcon 9");
        assert_eq!(p["click"], "https://x.com/live");
        assert_eq!(p["actions"][0]["label"], "Watch");
        assert_eq!(p["priority"], 5);
        assert!(ntfy(&NotifyConfig::default(), &a).is_err()); // no topic -> refused, no network
    }
}
