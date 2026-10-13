//! Cache + state files, written atomically (the QML widget may read at any time).
use crate::model::Cache;
use crate::schedule::State;
use anyhow::Result;
use serde::{de::DeserializeOwned, Serialize};
use std::path::Path;

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn save_json<T: Serialize>(path: &Path, v: &T) -> Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(v)?)
}

pub fn load_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn load_cache(path: &Path) -> Cache {
    load_json(path)
}
pub fn load_state(path: &Path) -> State {
    load_json(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_and_missing() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x/launches.json");
        assert_eq!(load_cache(&p), Cache::default());
        let mut c = Cache::default();
        c.source = "test".into();
        save_json(&p, &c).unwrap();
        assert_eq!(load_cache(&p).source, "test");
    }
}
