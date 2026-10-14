//! Cache + state files, written atomically (the QML widget may read at any time).
use crate::model::Cache;
use crate::schedule::State;
use anyhow::Result;
use serde::{Serialize, de::DeserializeOwned};
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

/// Write a file that may hold secrets: the temp file is created owner-only (0600) before any
/// byte is written, then atomically renamed over `path`. Never readable by others, not even
/// briefly (unlike write-then-chmod).
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn save_json<T: Serialize>(path: &Path, v: &T) -> Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(v)?)
}

pub fn load_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
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
    #[cfg(unix)]
    #[test]
    fn write_private_is_0600_from_creation_and_replaces_wider_files() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("c/config.toml");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "old").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&p, b"secret").unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read(&p).unwrap(), b"secret");
        assert_eq!(std::fs::read_dir(p.parent().unwrap()).unwrap().count(), 1, "no temp file left");
    }

    #[test]
    fn roundtrip_and_missing() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x/launches.json");
        assert_eq!(load_cache(&p), Cache::default());
        let c = Cache { source: "test".into(), ..Default::default() };
        save_json(&p, &c).unwrap();
        assert_eq!(load_cache(&p).source, "test");
    }
}
