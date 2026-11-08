//! Narrow experimental Storage bridge; policy and config transactions remain Rust-owned.
use crate::capture::engine::bundle::retention::{Policy, Preset};
use crate::shared::handoff::{capture_location, update_config_in};
use std::hash::{Hash, Hasher};
use std::os::fd::{FromRawFd, IntoRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

fn revision(raw: &serde_json::Value) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    raw["retention"].to_string().hash(&mut hash);
    format!("{:016x}", hash.finish())
}

pub(in crate::capture::desktop) fn read(config: &Path) -> Result<serde_json::Value, &'static str> {
    let raw = capture_location::read_raw(config).map_err(|_| "config_read_failed")?;
    let policy: Policy = serde_json::from_value(raw["retention"].clone()).unwrap_or_default();
    Ok(serde_json::json!({
        "retention": crate::capture::desktop::settings::settings::retention_view(&policy),
        "revision": revision(&raw),
        "custom": raw["retention"].get("mediaDays").is_some() || raw["retention"].get("bundleDays").is_some(),
    }))
}

pub(in crate::capture::desktop) fn save(
    config: &Path,
    preset: Option<&str>,
    expected: &str,
    admit: impl FnOnce() -> bool,
) -> Result<serde_json::Value, &'static str> {
    let policy = preset
        .map(|id| {
            Preset::ALL
                .into_iter()
                .find(|p| crate::capture::desktop::settings::settings::preset_id(*p) == id)
                .map(|p| p.policy())
                .ok_or("invalid_request")
        })
        .transpose()?;
    let mut refusal = None;
    update_config_in(config, |raw| {
        if !admit() {
            refusal = Some("admission_expired");
            anyhow::bail!("expired");
        }
        if revision(raw) != expected {
            refusal = Some("conflict");
            anyhow::bail!("conflict");
        }
        match policy {
            Some(policy) => {
                // Keep unrecognized nested policy keys, too.
                if !raw["retention"].is_object() {
                    raw["retention"] = serde_json::json!({});
                }
                raw["retention"]["mediaDays"] = serde_json::json!(policy.media_days);
                raw["retention"]["bundleDays"] = serde_json::json!(policy.bundle_days);
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
    .map_err(|_| refusal.unwrap_or("config_write_failed"))?;
    // Same timing as legacy Settings: schedule the existing worker, never run cleanup here.
    crate::capture::engine::bundle::retention::nudge();
    read(config)
}

pub(in crate::capture::desktop) fn usage(
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
