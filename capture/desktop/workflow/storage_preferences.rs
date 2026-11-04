//! Narrow Storage transaction executor; Settings policy and persistence intent live in TypeScript.
use crate::shared::handoff::{capture_location, update_config_allowed_in};
use std::hash::{Hash, Hasher};
use std::os::fd::{FromRawFd, IntoRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

fn field(raw: &serde_json::Value, name: &str) -> serde_json::Value {
    match raw
        .get("retention")
        .and_then(serde_json::Value::as_object)
        .and_then(|r| r.get(name))
    {
        Some(value) => serde_json::json!({"present":true,"value":value}),
        None => serde_json::json!({"present":false,"value":null}),
    }
}

fn revision(raw: &serde_json::Value) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    serde_json::json!({"mediaDays":field(raw,"mediaDays"),"bundleDays":field(raw,"bundleDays")})
        .to_string().hash(&mut hash);
    format!("{:016x}", hash.finish())
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SavePolicy {
    #[serde(rename = "mediaDays")]
    pub(super) media_days: serde_json::Value,
    #[serde(rename = "bundleDays")]
    pub(super) bundle_days: serde_json::Value,
}

impl SavePolicy {
    pub(super) fn valid(&self) -> bool {
        let valid = |days: &serde_json::Value| {
            days.is_null() || days.as_u64().is_some_and(|n| n <= u32::MAX as u64)
        };
        valid(&self.media_days) && valid(&self.bundle_days)
    }
}

pub(super) fn read(config: &Path) -> Result<serde_json::Value, &'static str> {
    let raw = capture_location::read_raw(config).map_err(|_| "config_read_failed")?;
    let stored = raw.get("retention").cloned().and_then(|value| serde_json::from_value::<crate::capture::engine::bundle::retention::Policy>(value).ok());
    let malformed = raw.get("retention").is_some_and(|value| value.is_object()) && stored.is_none();
    let (media_days, bundle_days) = if malformed {
        // Preserve the old cleanup reader's `unwrap_or_default()` behavior: one
        // malformed known value falls back to Week as a whole. Never project the
        // malformed value itself into the Settings renderer.
        (serde_json::json!({"present":false,"value":null}), serde_json::json!({"present":false,"value":null}))
    } else {
        (field(&raw, "mediaDays"), field(&raw, "bundleDays"))
    };
    let custom = raw["retention"].get("mediaDays").is_some()
        || raw["retention"].get("bundleDays").is_some();
    Ok(serde_json::json!({
        "retention": {"mediaDays":media_days,"bundleDays":bundle_days},
        "revision": revision(&raw),
        "custom": custom,
    }))
}

pub(super) fn save(
    config: &Path,
    policy: Option<&SavePolicy>,
    expected: &str,
    admit: impl Fn() -> bool,
) -> Result<serde_json::Value, &'static str> {
    if policy.is_some_and(|p| !p.valid()) {
        return Err("invalid_request");
    }
    let mut refusal = None;
    update_config_allowed_in(config, &admit, |raw| {
        if !admit() {
            refusal = Some("admission_expired");
            anyhow::bail!("expired");
        }
        if revision(raw) != expected {
            refusal = Some("conflict");
            anyhow::bail!("conflict");
        }
        if raw
            .get("retention")
            .is_some_and(|value| !value.is_object())
        {
            refusal = Some("config_write_failed");
            anyhow::bail!("invalid retention object");
        }
        match policy {
            Some(policy) => {
                // Keep unrecognized nested policy keys, too.
                if !raw["retention"].is_object() {
                    raw["retention"] = serde_json::json!({});
                }
                raw["retention"]["mediaDays"] = policy.media_days.clone();
                raw["retention"]["bundleDays"] = policy.bundle_days.clone();
            }
            None => {
                if let Some(fields) = raw["retention"].as_object_mut() {
                    fields.remove("mediaDays");
                    fields.remove("bundleDays");
                    if fields.is_empty() {
                        raw.as_object_mut().unwrap().remove("retention");
                    }
                } else {
                    raw.as_object_mut().unwrap().remove("retention");
                }
            }
        }
        Ok(())
    })
    .map_err(|error| {
        if error.is::<crate::shared::handoff::ConfigPublicationUncertain>() { return "outcome_unknown"; }
        refusal.unwrap_or_else(|| if !admit() { "admission_expired" } else { "config_write_failed" })
    })?;
    // Same timing as legacy Settings: schedule the existing worker, never run cleanup here.
    crate::capture::engine::bundle::retention::nudge();
    read(config)
}

pub(super) fn usage(
    config: &Path,
    admit: impl Fn() -> bool,
) -> Result<serde_json::Value, &'static str> {
    let before = capture_location::read_in(config).map_err(|_| "config_read_failed")?;
    let root = Path::new(&before.effective_path);
    let opened = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root);
    let (bytes, identity) = match opened {
        Ok(file) => {
            let meta = file.metadata().map_err(|_| "usage_unavailable")?;
            (
                scan_directory(file, &admit, &mut 0, 0)?,
                Some((meta.dev(), meta.ino())),
            )
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (0, None),
        Err(_) => return Err("usage_unavailable"),
    };
    let current = std::fs::symlink_metadata(root)
        .ok()
        .map(|m| (m.dev(), m.ino()));
    if current != identity {
        return Err("conflict");
    }
    let after = capture_location::read_in(config).map_err(|_| "config_read_failed")?;
    if before.revision != after.revision || before.effective_path != after.effective_path {
        return Err("conflict");
    }
    Ok(serde_json::json!({"usage": {
        "effectivePath": after.effective_path,
        "displayPath": after.display_path,
        "formatted": crate::capture::engine::bundle::retention::format_bytes(bytes),
    }}))
}

/// Pin every directory, including while enumerating its children. Path-based
/// read_dir after symlink_metadata has a race that can traverse outside captures.
fn scan_directory(
    file: std::fs::File,
    admit: &impl Fn() -> bool,
    entries: &mut usize,
    depth: usize,
) -> Result<u64, &'static str> {
    if !admit() || depth >= 256 {
        return Err("usage_unavailable");
    }
    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) {
            // fdopendir transferred the owned descriptor to DIR.
            unsafe {
                libc::closedir(self.0);
            }
        }
    }
    let fd = file.into_raw_fd();
    // SAFETY: fd is an exclusively owned open directory descriptor.
    let dir = unsafe { libc::fdopendir(fd) };
    if dir.is_null() {
        drop(unsafe { std::fs::File::from_raw_fd(fd) });
        return Err("usage_unavailable");
    }
    let dir = Directory(dir);
    let mut bytes = 0u64;
    loop {
        if !admit() {
            return Err("usage_unavailable");
        }
        // readdir returns null for both EOF and error; clear/check errno to
        // avoid presenting a failed scan as a successful partial total.
        let entry = unsafe {
            #[cfg(target_os = "linux")]
            {
                *libc::__errno_location() = 0;
            }
            #[cfg(target_os = "macos")]
            {
                *libc::__error() = 0;
            }
            libc::readdir(dir.0)
        };
        if entry.is_null() {
            if std::io::Error::last_os_error().raw_os_error() != Some(0) {
                return Err("usage_unavailable");
            }
            return Ok(bytes);
        }
        // SAFETY: readdir's name is NUL-terminated and valid until the next call
        // on this DIR; recursion uses a different DIR.
        let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) };
        if [b".".as_slice(), b"..".as_slice()].contains(&name.to_bytes()) {
            continue;
        }
        *entries += 1;
        if *entries > 100_000 {
            return Err("usage_unavailable");
        }
        let mut meta = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                fd,
                name.as_ptr(),
                meta.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err("usage_unavailable");
        }
        let meta = unsafe { meta.assume_init() };
        let size = if meta.st_mode & libc::S_IFMT == libc::S_IFDIR {
            // Parent fd is pinned; O_NOFOLLOW rejects a directory replaced by a
            // symlink after fstatat. Never reopen an absolute descendant path.
            let child = unsafe {
                libc::openat(
                    fd,
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if child < 0 {
                return Err("usage_unavailable");
            }
            let child = unsafe { std::fs::File::from_raw_fd(child) };
            scan_directory(child, admit, entries, depth + 1)?
        } else {
            u64::try_from(meta.st_size).map_err(|_| "usage_unavailable")?
        };
        bytes = bytes.checked_add(size).ok_or("usage_unavailable")?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_known_retention_values_use_week_defaults_without_echoing_them() {
        let root = std::env::temp_dir().join(format!("retention-{}", crate::shared::ids::make_bundle_id()));
        std::fs::create_dir(&root).unwrap();
        let config = root.join("config.json");
        std::fs::write(&config, r#"{"secret":"keep","retention":{"mediaDays":"private-invalid","bundleDays":90,"unknown":true}}"#).unwrap();
        let read = read(&config).unwrap();
        assert_eq!(read["retention"]["mediaDays"], serde_json::json!({"present":false,"value":null}));
        assert_eq!(read["retention"]["bundleDays"], serde_json::json!({"present":false,"value":null}));
        assert_eq!(read["custom"], true);
        assert!(read.to_string().find("private-invalid").is_none());
        assert_eq!(crate::capture::engine::bundle::retention::Policy::load_from(&config), Default::default());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn usage_pins_directories_and_never_follows_replaced_paths_or_child_links() {
        let root =
            std::env::temp_dir().join(format!("usage-{}", crate::shared::ids::make_bundle_id()));
        let captures = root.join("captures");
        let outside = root.join("outside");
        std::fs::create_dir_all(&captures).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(captures.join("keep"), b"123").unwrap();
        std::fs::write(outside.join("private"), vec![0; 1_000_000]).unwrap();
        std::os::unix::fs::symlink(&outside, captures.join("link")).unwrap();
        let link_size = std::fs::symlink_metadata(captures.join("link"))
            .unwrap()
            .len();
        let pinned = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&captures)
            .unwrap();
        // Replace the pathname after opening. The actual traversal must stay on
        // the original directory, not follow the newly substituted symlink.
        std::fs::rename(&captures, root.join("moved")).unwrap();
        std::os::unix::fs::symlink(&outside, &captures).unwrap();
        assert_eq!(
            scan_directory(pinned, &|| true, &mut 0, 0),
            Ok(3 + link_size)
        );
        let denied = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&captures);
        assert!(denied.is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
