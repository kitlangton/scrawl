//! Serialized JSON config storage shared by CLI and GUI writers.
use std::fs;
use std::path::Path;
use anyhow::{Context, Result, ensure};

#[derive(Debug)]
pub struct ConfigPublicationUncertain;

impl std::fmt::Display for ConfigPublicationUncertain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Config was published but durability was not confirmed")
    }
}

impl std::error::Error for ConfigPublicationUncertain {}

/// Sets one top-level key in `path` (`null` removes it), keeping the rest (and its `0600` mode,
/// since it holds the ingest token).
pub fn set_config_value_in(path: &Path, key: &str, value: serde_json::Value) -> Result<()> {
    ensure!(key != ONBOARDING_WRITER_OWNER_KEY, "onboarding writer ownership is host-managed");
    update_config_checked_in(path, None, ConfigMutationAuthority::Ordinary, |config| {
        ensure!(key != "onboarding" || !onboarding_legacy_writer_retired(config)?, "legacy onboarding progress writer has been retired");
        if value.is_null() {
            config.as_object_mut().unwrap().remove(key);
        } else {
            config[key] = value;
        }
        Ok(true)
    })
}

const ONBOARDING_WRITER_OWNER_KEY: &str = "onboardingLegacyWriter";

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfigMutationAuthority { Ordinary, OnboardingOwnership }

fn onboarding_legacy_writer_retired(config: &serde_json::Value) -> Result<bool> {
    let top = config.as_object().context("Config must be an object")?;
    match top.get(ONBOARDING_WRITER_OWNER_KEY) {
        None => Ok(false),
        Some(value) => {
            ensure!(value == &serde_json::json!({"state":"retired","schema":1}), "Invalid onboarding writer ownership marker");
            Ok(true)
        }
    }
}

/// Permanently transfers the legacy onboarding key to the owner store.
pub fn retire_onboarding_legacy_writer_in(path: &Path) -> Result<()> {
    update_config_checked_in(path, None, ConfigMutationAuthority::OnboardingOwnership, |config| {
        let top = config.as_object_mut().context("Config must be an object")?;
        match top.get(ONBOARDING_WRITER_OWNER_KEY) {
            None => { top.insert(ONBOARDING_WRITER_OWNER_KEY.to_owned(), serde_json::json!({"state":"retired","schema":1})); Ok(true) }
            Some(value) => { ensure!(value == &serde_json::json!({"state":"retired","schema":1}), "Invalid onboarding writer ownership marker"); Ok(false) }
        }
    })
}

// One sidecar inode for every CLI/GUI writer, never the replaced config inode.
// Hosted operations take queue.lock first; this transaction never acquires a
// queue lock or performs network work. Keep nested edits inside this closure.
pub fn update_config_in(
    path: &Path,
    edit: impl FnOnce(&mut serde_json::Value) -> Result<()>,
) -> Result<()> {
    update_config_checked_in(path, None, ConfigMutationAuthority::Ordinary, |config| { edit(config)?; Ok(true) })
}

/// Connect's background authorization writes must recheck cancellation after
/// acquiring the shared writer lock. Other settings keep their existing wait policy.
pub fn update_config_allowed_in(
    path: &Path,
    allowed: &dyn Fn() -> bool,
    edit: impl FnOnce(&mut serde_json::Value) -> Result<()>,
) -> Result<()> {
    update_config_checked_in(path, Some(allowed), ConfigMutationAuthority::Ordinary, |config| { edit(config)?; Ok(true) })
}

fn update_config_checked_in(
    path: &Path,
    allowed: Option<&dyn Fn() -> bool>,
    authority: ConfigMutationAuthority,
    edit: impl FnOnce(&mut serde_json::Value) -> Result<bool>,
) -> Result<()> {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    #[cfg(any(test, feature = "test-support"))]
    config_write_hook("before_lock", path);
    let writer = ConfigWriter::acquire(path, allowed)?;
    let mut config = read_snapshot(&writer, path)?;
    let previous_onboarding_marker = config.get(ONBOARDING_WRITER_OWNER_KEY).cloned();
    let previous_onboarding = config.get("onboarding").cloned();
    #[cfg(any(test, feature = "test-support"))]
    config_write_hook("snapshot", path);
    ensure!(allowed.is_none_or(|check| check()), "cancelled");
    if !edit(&mut config)? { return Ok(()); }
    ensure!(authority == ConfigMutationAuthority::OnboardingOwnership || config.get(ONBOARDING_WRITER_OWNER_KEY).cloned() == previous_onboarding_marker, "onboarding writer ownership is host-managed");
    if authority == ConfigMutationAuthority::Ordinary && previous_onboarding_marker.is_some() {
        ensure!(config.get("onboarding").cloned() == previous_onboarding, "legacy onboarding progress writer has been retired");
    }
    let tmp = std::ffi::CString::new(format!(".config-{}.tmp", random_id()?))?;
    let mut body = serde_json::to_vec_pretty(&config)?;
    body.push(b'\n');
    let mut published = false;
    let result = (|| -> Result<()> {
        let mut file = writer.open(&tmp, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)
            .context("config-temp-open")?;
        file.write_all(&body).context("config-temp-write")?;
        file.sync_all().context("config-temp-sync")?;
        #[cfg(any(test, feature = "test-support"))]
        config_write_hook("before_publish", path);
        ensure!(allowed.is_none_or(|check| check()), "cancelled");
        let fd = writer.directory.as_raw_fd();
        if unsafe { libc::renameat(fd, tmp.as_ptr(), fd, writer.name.as_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error())
                .context("config-rename")
                .context("Could not publish config");
        }
        published = true;
        #[cfg(any(test, feature = "test-support"))]
        {
            config_write_hook("before_directory_sync", path);
            anyhow::ensure!(
                !CONFIG_DIRECTORY_SYNC_FAILURE.with(|failure| failure.replace(false)),
                "Injected config directory sync failure"
            );
        }
        // A revocation intent may be removed only after this directory entry is
        // durable, not merely after the new JSON file is visible to this process.
        writer.directory.sync_all()
            .context("config-directory-sync")
            .context("Could not persist config directory")?;
        Ok(())
    })();
    if result.is_err() {
        unsafe {
            libc::unlinkat(writer.directory.as_raw_fd(), tmp.as_ptr(), 0);
        }
    }
    if published { result.context(ConfigPublicationUncertain) } else { result }
}


/// Read a config snapshot while holding the same writer lock used by updates.
pub fn read_config_in(path: &Path) -> Result<serde_json::Value> {
    let writer = ConfigWriter::acquire(path, None)?;
    read_snapshot(&writer, path)
}

pub fn read_stream_state_in(path: &Path, owner: &str) -> Result<(u64, serde_json::Value)> {
    let config = read_config_in(path)?;
    let Some(stream) = config.get("streamState") else { return Ok((0, serde_json::Value::Null)); };
    let map = stream.as_object().context("streamState must be an object")?;
    match map.get(owner) {
        Some(value) => { let entry = parse_stream_entry(value)?; Ok((entry.revision, entry.value)) }
        None => Ok((0, serde_json::Value::Null)),
    }
}

/// Read the selected owner entry and its identically named top-level legacy key.
/// Presence is explicit so a missing key remains distinct from JSON null.
pub fn read_for_adoption_in(path: &Path, owner: &str) -> Result<AdoptionSnapshot> {
    let writer = ConfigWriter::acquire(path, None)?;
    let config = read_snapshot(&writer, path)?;
    adoption_snapshot(&config, owner)
}

/// Read Record's bounded legacy pen color under the shared config writer lock.
pub fn read_pen_color_in(path: &Path) -> Result<LegacyValue> {
    let writer = ConfigWriter::acquire(path, None)?;
    let config = read_snapshot(&writer, path)?;
    top_level_value(&config, "penColor")
}

/// Presence/value CAS for Record's existing top-level `penColor` setting.
/// A missing desired value removes the key; stored JSON null remains present.
pub fn compare_and_swap_pen_color_in(
    path: &Path,
    expected: LegacyValue,
    next: LegacyValue,
) -> Result<LegacyCasResult> {
    validate_legacy_value(&expected)?;
    validate_legacy_value(&next)?;
    let mut outcome = None;
    update_config_checked_in(path, None, ConfigMutationAuthority::Ordinary, |config| {
        let current = top_level_value(config, "penColor")?;
        if current != expected {
            outcome = Some(LegacyCasResult::Conflict(current));
            return Ok(false);
        }
        let top = config.as_object_mut().context("Config must be an object")?;
        if next.present { top.insert("penColor".to_owned(), next.value.clone()); }
        else { top.remove("penColor"); }
        outcome = Some(LegacyCasResult::Saved);
        Ok(true)
    })?;
    Ok(outcome.expect("transaction always sets outcome"))
}

fn top_level_value(config: &serde_json::Value, key: &str) -> Result<LegacyValue> {
    let top = config.as_object().context("Config must be an object")?;
    let value = top.get(key).cloned();
    let value = LegacyValue { present: value.is_some(), value: value.unwrap_or(serde_json::Value::Null) };
    validate_legacy_value(&value)?;
    Ok(value)
}

fn validate_legacy_value(value: &LegacyValue) -> Result<()> {
    ensure!(value.present || value.value.is_null(), "Invalid legacy config presence");
    ensure!(serde_json::to_vec(&value.value)?.len() <= 16 * 1024, "Legacy config value exceeds size limit");
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub enum LegacyCasResult { Saved, Conflict(LegacyValue) }

/// Run one bounded native effect only while Record's fixed pen color still
/// matches the observed presence/value. The shared config writer lock remains
/// held for the whole callback, ordering the effect after earlier settings CAS
/// operations and before later ones.
///
/// Lock order is config writer lock, then any runtime/native resource lock the
/// callback needs. The callback must not reenter config access, perform
/// filesystem or network work, or wait without its caller's deadline/admission
/// policy. An error is propagated and is never reported as `Applied`.
pub fn with_pen_color_current_in<T>(
    path: &Path,
    expected: LegacyValue,
    effect: impl FnOnce() -> Result<T>,
) -> Result<PenColorEffectResult<T>> {
    pen_color_effect_in(path, expected, None, effect)
}

/// Bounded hosts must use this variant and retain their admission deadline
/// through lock acquisition and the final check immediately before the effect.
/// The callback still owns cancellation/bounding once an effect has started.
pub fn with_pen_color_current_allowed_in<T>(
    path: &Path,
    expected: LegacyValue,
    allowed: &dyn Fn() -> bool,
    effect: impl FnOnce() -> Result<T>,
) -> Result<PenColorEffectResult<T>> {
    pen_color_effect_in(path, expected, Some(allowed), effect)
}

fn pen_color_effect_in<T>(
    path: &Path,
    expected: LegacyValue,
    allowed: Option<&dyn Fn() -> bool>,
    effect: impl FnOnce() -> Result<T>,
) -> Result<PenColorEffectResult<T>> {
    validate_legacy_value(&expected)?;
    let writer = ConfigWriter::acquire(path, allowed)?;
    let config = read_snapshot(&writer, path)?;
    let current = top_level_value(&config, "penColor")?;
    if current != expected {
        return Ok(PenColorEffectResult::Conflict(current));
    }
    ensure!(allowed.is_none_or(|check| check()), "cancelled");
    let result = effect()?;
    drop(writer);
    Ok(PenColorEffectResult::Applied(result))
}

#[derive(Debug, Clone, PartialEq)]
pub enum PenColorEffectResult<T> {
    Applied(T),
    Conflict(LegacyValue),
}

/// Adopt a legacy value only while both the owner entry is still absent and
/// the exact legacy key still matches the caller's observed presence/value.
pub fn adopt_legacy_in(
    path: &Path,
    owner: &str,
    expected_legacy: LegacyValue,
    value: serde_json::Value,
) -> Result<AdoptResult> {
    let mut outcome = None;
    let authority = if owner == "onboarding" { ConfigMutationAuthority::OnboardingOwnership } else { ConfigMutationAuthority::Ordinary };
    update_config_checked_in(path, None, authority, |config| {
        if owner == "onboarding" { let _ = onboarding_legacy_writer_retired(config)?; }
        let snapshot = adoption_snapshot(config, owner)?;
        if snapshot.owner.present || snapshot.legacy != expected_legacy {
            outcome = Some(AdoptResult::Conflict(snapshot));
            return Ok(false);
        }
        let top = config.as_object_mut().context("Config must be an object")?;
        let stream = top.entry("streamState").or_insert_with(|| serde_json::json!({}));
        let map = stream.as_object_mut().context("streamState must be an object")?;
        map.insert(owner.to_owned(), serde_json::json!({"revision":1,"value":value}));
        if owner == "onboarding" {
            top.insert(ONBOARDING_WRITER_OWNER_KEY.to_owned(), serde_json::json!({"state":"retired","schema":1}));
        }
        outcome = Some(AdoptResult::Saved { revision: 1 });
        Ok(true)
    })?;
    Ok(outcome.expect("transaction always sets outcome"))
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyValue { pub present: bool, pub value: serde_json::Value }

#[derive(Debug, Clone, PartialEq)]
pub struct OwnerAdoptionValue { pub present: bool, pub revision: u64, pub value: serde_json::Value }

#[derive(Debug, Clone, PartialEq)]
pub struct AdoptionSnapshot { pub owner: OwnerAdoptionValue, pub legacy: LegacyValue }

#[derive(Debug, Clone, PartialEq)]
pub enum AdoptResult { Saved { revision: u64 }, Conflict(AdoptionSnapshot) }

fn adoption_snapshot(config: &serde_json::Value, owner: &str) -> Result<AdoptionSnapshot> {
    let top = config.as_object().context("Config must be an object")?;
    let owner_entry = match top.get("streamState") {
        None => None,
        Some(stream) => {
            let map = stream.as_object().context("streamState must be an object")?;
            map.get(owner).map(parse_stream_entry).transpose()?
        }
    };
    let legacy = top.get(owner).cloned();
    Ok(AdoptionSnapshot {
        owner: OwnerAdoptionValue { present: owner_entry.is_some(), revision: owner_entry.as_ref().map_or(0, |entry| entry.revision), value: owner_entry.map_or(serde_json::Value::Null, |entry| entry.value) },
        legacy: LegacyValue { present: legacy.is_some(), value: legacy.unwrap_or(serde_json::Value::Null) },
    })
}

/// Compare and replace one owner entry under the config writer lock.
pub fn compare_and_swap_stream_state_in(
    path: &Path,
    owner: &str,
    expected: u64,
    value: serde_json::Value,
) -> Result<CasResult> {
    ensure!(expected <= 9_007_199_254_740_991, "Invalid expected revision");
    let mut outcome = None;
    update_config_checked_in(path, None, ConfigMutationAuthority::Ordinary, |config| {
        let top = config.as_object_mut().context("Config must be an object")?;
        let stream = top.entry("streamState").or_insert_with(|| serde_json::json!({}));
        let map = stream.as_object_mut().context("streamState must be an object")?;
        let current = match map.get(owner) {
            None => StreamEntry { revision: 0, value: serde_json::Value::Null },
            Some(v) => parse_stream_entry(v)?,
        };
        if current.revision != expected {
            outcome = Some(CasResult::Conflict { revision: current.revision, current: current.value });
            return Ok(false);
        }
        ensure!(expected < 9_007_199_254_740_991, "revision limit reached");
        let revision = expected + 1;
        map.insert(owner.to_owned(), serde_json::json!({"revision":revision,"value":value}));
        outcome = Some(CasResult::Saved { revision });
        Ok(true)
    })?;
    Ok(outcome.expect("transaction always sets outcome"))
}

#[derive(Debug, Clone, PartialEq)]
pub enum CasResult {
    Saved { revision: u64 },
    Conflict { revision: u64, current: serde_json::Value },
}
struct StreamEntry { revision: u64, value: serde_json::Value }

fn parse_stream_entry(value: &serde_json::Value) -> Result<StreamEntry> {
    let object = value.as_object().context("Invalid streamState entry")?;
    ensure!(object.len() == 2 && object.contains_key("revision") && object.contains_key("value"), "Invalid streamState entry");
    let revision = object["revision"].as_u64().context("Invalid streamState revision")?;
    ensure!(revision <= 9_007_199_254_740_991, "Invalid streamState revision");
    Ok(StreamEntry { revision, value: object["value"].clone() })
}

fn random_id() -> Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    fs::File::open("/dev/urandom").context("config-random-open")?
        .read_exact(&mut bytes).context("config-random-read")?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn read_snapshot(writer: &ConfigWriter, path: &Path) -> Result<serde_json::Value> {
    use std::io::Read;
    match writer.open(&writer.name, libc::O_RDONLY | libc::O_NONBLOCK) {
        Ok(mut file) => {
            ensure!(file.metadata().context("config-snapshot-stat")?.is_file(), "Config must be a regular file");
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).context("config-snapshot-read")?;
            let parsed: serde_json::Value = serde_json::from_slice(&bytes)
                .with_context(|| format!("Invalid config at {}", path.display()))?;
            ensure!(parsed.is_object(), "Config at {} must be an object", path.display());
            Ok(parsed)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(error) => Err(error)
            .context("config-snapshot-open")
            .with_context(|| format!("Could not read {}", path.display())),
    }
}

struct ConfigWriter {
    directory: fs::File,
    // Closing this descriptor releases flock, including on error/unwind.
    _lock: fs::File,
    name: std::ffi::CString,
}

fn open_lock_with(mut open: impl FnMut(i32) -> std::io::Result<fs::File>) -> std::io::Result<fs::File> {
    // Mac CI observed ENOENT during concurrent first creation in the combined
    // O_CREAT path. Create exclusively, or open the persistent existing inode.
    // If that inode disappears, fail closed: never create a replacement lock.
    match open(libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NONBLOCK) {
        Err(error) if error.raw_os_error() == Some(libc::EEXIST) => open(libc::O_RDWR | libc::O_NONBLOCK),
        result => result,
    }
}

impl ConfigWriter {
    fn acquire(path: &Path, allowed: Option<&dyn Fn() -> bool>) -> Result<Self> {
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent).context("config-parent-create")?;
        let parent = parent.canonicalize().context("config-parent-resolve")?;
        let mut directory = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/").context("config-root-open")?;
        let mut private_ancestor = false;
        for component in parent.components().skip(1) {
            let name = std::ffi::CString::new(component.as_os_str().as_bytes())?;
            directory = Self::open_at(&directory, &name, libc::O_RDONLY | libc::O_DIRECTORY).context("config-ancestor-open")?;
            let meta = directory.metadata().context("config-ancestor-stat")?;
            ensure!(
                (meta.uid() == 0 || meta.uid() == unsafe { libc::geteuid() })
                    && (meta.mode() & 0o022 == 0
                        || meta.mode() & libc::S_ISVTX as u32 != 0
                        || private_ancestor),
                "Config ancestry must be trusted or sticky"
            );
            private_ancestor |= meta.mode() & 0o077 == 0;
        }
        let meta = directory.metadata().context("config-directory-stat")?;
        ensure!(
            meta.uid() == unsafe { libc::geteuid() }
                && (meta.mode() & 0o022 == 0 || private_ancestor),
            "Config directory must be owned and not writable by other users"
        );
        let name = path.file_name().context("Config has no filename")?;
        let name = std::ffi::CString::new(name.as_bytes())?;
        let lock_name = std::ffi::CString::new([name.as_bytes(), b".lock"].concat())?;
        let lock = open_lock_with(|flags| Self::open_at(&directory, &lock_name, flags))
            .context("config-lock-open")?;
        let meta = lock.metadata().context("config-lock-stat")?;
        ensure!(
            meta.is_file()
                && meta.uid() == unsafe { libc::geteuid() }
                && meta.mode() & 0o077 == 0
                && meta.nlink() == 1,
            "Config lock must be an owned private regular file"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            ensure!(allowed.is_none_or(|check| check()), "cancelled");
            let flags = libc::LOCK_EX | if allowed.is_some() { libc::LOCK_NB } else { 0 };
            if unsafe { libc::flock(lock.as_raw_fd(), flags) } == 0 {
                break;
            }
            let error = std::io::Error::last_os_error();
            if allowed.is_some() && error.kind() == std::io::ErrorKind::WouldBlock {
                ensure!(std::time::Instant::now() < deadline, "Config lock wait timed out");
                std::thread::sleep(std::time::Duration::from_millis(20));
                continue;
            }
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error).context("config-lock-acquire");
            }
        }
        let current = Self::open_at(&directory, &lock_name, libc::O_RDONLY | libc::O_NONBLOCK)
            .context("config-lock-reopen")?.metadata().context("config-lock-restat")?;
        ensure!(
            current.dev() == meta.dev()
                && current.ino() == meta.ino()
                && lock.metadata().context("config-lock-restat")?.nlink() == 1,
            "Config lock changed while waiting"
        );
        Ok(Self {
            directory,
            _lock: lock,
            name,
        })
    }

    fn open(&self, name: &std::ffi::CStr, flags: i32) -> std::io::Result<fs::File> {
        Self::open_at(&self.directory, name, flags)
    }

    fn open_at(
        directory: &fs::File,
        name: &std::ffi::CStr,
        flags: i32,
    ) -> std::io::Result<fs::File> {
        use std::os::fd::{AsRawFd, FromRawFd};
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                // C variadic arguments require promotion: Darwin's mode_t is u16.
                0o600 as libc::c_uint,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { fs::File::from_raw_fd(fd) })
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static CONFIG_WRITE_HOOK: std::cell::RefCell<Option<Box<dyn FnMut(&str, &Path)>>> =
        const { std::cell::RefCell::new(None) };
    static CONFIG_DIRECTORY_SYNC_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_config_directory_sync_failure(fail: bool) {
    CONFIG_DIRECTORY_SYNC_FAILURE.with(|failure| failure.set(fail));
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_config_write_hook(hook: Option<Box<dyn FnMut(&str, &Path)>>) {
    CONFIG_WRITE_HOOK.with(|slot| *slot.borrow_mut() = hook);
}

#[cfg(any(test, feature = "test-support"))]
fn config_write_hook(point: &str, path: &Path) {
    CONFIG_WRITE_HOOK.with(|slot| {
        if let Ok(mut slot) = slot.try_borrow_mut()
            && let Some(hook) = slot.as_mut()
        {
            hook(point, path);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_creation_avoids_observed_combined_create_or_open_failure() {
        let mut calls = Vec::new();
        let lock = open_lock_with(|flags| {
            calls.push(flags);
            // Inject the ENOENT observed at the combined create/open boundary.
            // Exclusive create instead reports the peer's existing inode.
            if flags & libc::O_CREAT != 0 {
                let code = if flags & libc::O_EXCL == 0 { libc::ENOENT } else { libc::EEXIST };
                return Err(std::io::Error::from_raw_os_error(code));
            }
            fs::File::open("/dev/null")
        });
        assert!(lock.is_ok(), "{lock:?}");
        assert_eq!(calls, [libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NONBLOCK, libc::O_RDWR | libc::O_NONBLOCK]);
    }

    #[test]
    fn lock_removal_and_other_creation_errors_never_trigger_recreation() {
        let mut calls = 0;
        let error = open_lock_with(|_| {
            calls += 1;
            Err(std::io::Error::from_raw_os_error(if calls == 1 { libc::EEXIST } else { libc::ENOENT }))
        }).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ENOENT));
        assert_eq!(calls, 2, "never recreate a missing inode after observing an existing lock");
        calls = 0;
        let error = open_lock_with(|_| { calls += 1; Err(std::io::Error::from_raw_os_error(libc::EACCES)) }).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::EACCES));
        assert_eq!(calls, 1);
    }

    #[test]
    fn uncertain_publication_is_not_reported_as_saved_and_retry_observes_revision() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "apparent-config-native-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        let dir = dir.canonicalize().unwrap();
        let path = dir.join("config.json");

        set_config_directory_sync_failure(true);
        let error = compare_and_swap_stream_state_in(&path, "desktop", 0, serde_json::json!({"step":1})).unwrap_err();
        assert!(error.downcast_ref::<ConfigPublicationUncertain>().is_some());
        assert_eq!(read_stream_state_in(&path, "desktop").unwrap(), (1, serde_json::json!({"step":1})));
        assert_eq!(compare_and_swap_stream_state_in(&path, "desktop", 0, serde_json::json!({"step":2})).unwrap(), CasResult::Conflict { revision: 1, current: serde_json::json!({"step":1}) });
        fs::remove_dir_all(dir).unwrap();
    }
}
