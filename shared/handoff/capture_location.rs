//! Captures-only preference transaction. Never moves a bundle or changes active capture state.
use super::*;
use std::hash::{Hash, Hasher};
use std::os::unix::fs::OpenOptionsExt;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub saved_path: Option<String>,
    pub effective_path: String,
    pub display_path: String,
    pub custom: bool,
    pub source: &'static str,
    pub revision: String,
}

fn revision(raw: &serde_json::Value) -> String {
    // An opaque stale-edit fingerprint, not an authentication token.
    // DefaultHasher is available on both platforms without a crypto dependency.
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_vec(&raw["capturesDir"]).unwrap().hash(&mut hash);
    format!("{:016x}", hash.finish())
}

fn read_raw(path: &Path) -> Result<serde_json::Value> {
    use std::io::Read;
    let file = match fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(serde_json::json!({})),
        Err(e) => return Err(e.into()),
    };
    ensure!(file.metadata()?.is_file(), "invalid config");
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "config too large");
    let raw: serde_json::Value = serde_json::from_slice(&bytes)?;
    ensure!(raw.is_object(), "invalid config");
    Ok(raw)
}

pub fn read_in(path: &Path) -> Result<Snapshot> {
    snapshot(&read_raw(path)?, std::env::var_os("APPARENT_CAPTURES_DIR").map(PathBuf::from))
}

pub fn enabled_in(path: &Path) -> Result<bool> {
    // A failed read is not a successful read of a disabled flag.
    let config = config_from_value(read_raw(path)?);
    Ok(experiment_overridden("electronCaptureLocation") || experiment_enabled_in(&config, "electronCaptureLocation"))
}

fn snapshot(raw: &serde_json::Value, environment: Option<PathBuf>) -> Result<Snapshot> {
    let saved_path = raw["capturesDir"].as_str().filter(|s| configured_captures_dir(s).is_some())
        .map(|s| s.trim().to_owned());
    let (effective, source) = resolve_captures_dir(saved_path.as_deref(), environment);
    let effective_path = effective.to_str().context("non UTF-8 path")?.to_owned();
    ensure!(effective_path.len() <= 4096 && !effective_path.contains('\0'), "invalid effective path");
    ensure!(saved_path.as_ref().is_none_or(|s| s.len() <= 4096), "invalid saved path");
    Ok(Snapshot {
        custom: saved_path.is_some(), saved_path,
        display_path: crate::composition::command_line::tilde(&effective),
        effective_path, source, revision: revision(raw),
    })
}

/// `admit` runs after acquiring the existing config flock, against the fresh raw object.
/// Errors are fixed codes; callers must never forward config-store diagnostics to a renderer.
pub fn set_in(
    path: &Path, value: Option<&str>, expected: Option<&str>, admit: impl FnOnce() -> bool,
) -> std::result::Result<(), &'static str> {
    let value = value.map(str::trim).filter(|s| !s.is_empty());
    if value.is_some_and(|s| s.len() > 4096 || s.contains('\0') || configured_captures_dir(s).is_none()) {
        return Err("invalid_path");
    }
    let mut refusal = None;
    let result = update_config_in(path, |raw| {
        if !admit() { refusal = Some("admission_expired"); bail!("expired"); }
        if expected.is_some_and(|expected| revision(raw) != expected) {
            refusal = Some("conflict"); bail!("conflict");
        }
        match value {
            Some(value) => raw["capturesDir"] = value.into(),
            None => { raw.as_object_mut().unwrap().remove("capturesDir"); }
        }
        Ok(())
    });
    result.map_err(|_| refusal.unwrap_or("config_write_failed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn capture_location_transaction_preserves_secrets_conflicts_expires_and_resets() {
        let dir = std::env::temp_dir().join(format!("cl-{}", crate::shared::ids::make_bundle_id()));
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.join("config.json");
        fs::write(&path, r#"{"secret":"keep","unknown":{"nested":true}}"#).unwrap();
        let first = read_in(&path).unwrap();
        // Preserve an unrelated edit made while a picker is open.
        set_config_value_in(&path, "otherSetting", "new".into()).unwrap();
        let chosen = dir.join("captures with é");
        set_in(&path, chosen.to_str(), Some(&first.revision), || true).unwrap();
        let saved = read_in(&path).unwrap();
        assert_eq!(saved.saved_path.as_deref(), chosen.to_str());
        assert_eq!(saved.source, "configured");
        let mut raw = read_raw(&path).unwrap();
        assert_eq!(raw["secret"], "keep");
        assert_eq!(raw["unknown"]["nested"], true);
        assert_eq!(raw["otherSetting"], "new");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(set_in(&path, None, Some(&first.revision), || true), Err("conflict"));
        assert_eq!(set_in(&path, None, Some(&saved.revision), || false), Err("admission_expired"));
        for invalid in ["relative", "~someone", "/bad\0path"] {
            assert_eq!(set_in(&path, Some(invalid), None, || true), Err("invalid_path"));
        }
        let overridden = snapshot(&raw, Some(PathBuf::from("relative-env"))).unwrap();
        assert_eq!(overridden.source, "environment");
        assert_eq!(overridden.effective_path, "relative-env");
        assert_eq!(overridden.saved_path, saved.saved_path);
        let blocked = dir.join("file");
        fs::write(&blocked, "not a folder").unwrap();
        raw["capturesDir"] = blocked.to_str().unwrap().into();
        assert_eq!(snapshot(&raw, None).unwrap().source, "fallback");
        set_in(&path, None, Some(&saved.revision), || true).unwrap();
        assert!(!read_raw(&path).unwrap().as_object().unwrap().contains_key("capturesDir"));
        assert_eq!(read_in(&path).unwrap().source, "builtin");
        fs::write(&path, "malformed").unwrap();
        assert_eq!(set_in(&path, None, None, || true), Err("config_write_failed"));
        assert!(read_in(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "malformed");
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(dir.join("target"), &path).unwrap();
        assert_eq!(set_in(&path, None, None, || true), Err("config_write_failed"));
        assert!(read_in(&path).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
