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
    update_config_in(path, |config| {
        if value.is_null() {
            config.as_object_mut().unwrap().remove(key);
        } else {
            config[key] = value;
        }
        Ok(())
    })
}

// One sidecar inode for every CLI/GUI writer, never the replaced config inode.
// Hosted operations take queue.lock first; this transaction never acquires a
// queue lock or performs network work. Keep nested edits inside this closure.
pub fn update_config_in(
    path: &Path,
    edit: impl FnOnce(&mut serde_json::Value) -> Result<()>,
) -> Result<()> {
    update_config_checked_in(path, None, edit)
}

/// Connect's background authorization writes must recheck cancellation after
/// acquiring the shared writer lock. Other settings keep their existing wait policy.
pub fn update_config_allowed_in(
    path: &Path,
    allowed: &dyn Fn() -> bool,
    edit: impl FnOnce(&mut serde_json::Value) -> Result<()>,
) -> Result<()> {
    update_config_checked_in(path, Some(allowed), edit)
}

fn update_config_checked_in(
    path: &Path,
    allowed: Option<&dyn Fn() -> bool>,
    edit: impl FnOnce(&mut serde_json::Value) -> Result<()>,
) -> Result<()> {
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    #[cfg(any(test, feature = "test-support"))]
    config_write_hook("before_lock", path);
    let writer = ConfigWriter::acquire(path, allowed)?;
    let mut config: serde_json::Value =
        match writer.open(&writer.name, libc::O_RDONLY | libc::O_NONBLOCK) {
            Ok(mut file) => {
                ensure!(file.metadata()?.is_file(), "Config must be a regular file");
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                let parsed: serde_json::Value = serde_json::from_slice(&bytes)
                    .with_context(|| format!("Invalid config at {}", path.display()))?;
                anyhow::ensure!(
                    parsed.is_object(),
                    "Config at {} must be an object",
                    path.display()
                );
                parsed
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
            Err(error) => {
                return Err(error).with_context(|| format!("Could not read {}", path.display()));
            }
        };
    #[cfg(any(test, feature = "test-support"))]
    config_write_hook("snapshot", path);
    ensure!(allowed.is_none_or(|check| check()), "cancelled");
    edit(&mut config)?;
    let tmp = std::ffi::CString::new(format!(".config-{}.tmp", crate::random_id()?))?;
    let mut body = serde_json::to_vec_pretty(&config)?;
    body.push(b'\n');
    let mut published = false;
    let result = (|| -> Result<()> {
        let mut file = writer.open(&tmp, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)?;
        file.write_all(&body)?;
        file.sync_all()?;
        #[cfg(any(test, feature = "test-support"))]
        config_write_hook("before_publish", path);
        ensure!(allowed.is_none_or(|check| check()), "cancelled");
        let fd = writer.directory.as_raw_fd();
        if unsafe { libc::renameat(fd, tmp.as_ptr(), fd, writer.name.as_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error()).context("Could not publish config");
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
        writer.directory.sync_all().context("Could not persist config directory")?;
        Ok(())
    })();
    if result.is_err() {
        unsafe {
            libc::unlinkat(writer.directory.as_raw_fd(), tmp.as_ptr(), 0);
        }
    }
    if published { result.context(ConfigPublicationUncertain) } else { result }
}

struct ConfigWriter {
    directory: fs::File,
    // Closing this descriptor releases flock, including on error/unwind.
    _lock: fs::File,
    name: std::ffi::CString,
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
            .create(parent)?;
        let parent = parent.canonicalize()?;
        let mut directory = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/")?;
        let mut private_ancestor = false;
        for component in parent.components().skip(1) {
            let name = std::ffi::CString::new(component.as_os_str().as_bytes())?;
            directory = Self::open_at(&directory, &name, libc::O_RDONLY | libc::O_DIRECTORY)?;
            let meta = directory.metadata()?;
            ensure!(
                (meta.uid() == 0 || meta.uid() == unsafe { libc::geteuid() })
                    && (meta.mode() & 0o022 == 0
                        || meta.mode() & libc::S_ISVTX as u32 != 0
                        || private_ancestor),
                "Config ancestry must be trusted or sticky"
            );
            private_ancestor |= meta.mode() & 0o077 == 0;
        }
        let meta = directory.metadata()?;
        ensure!(
            meta.uid() == unsafe { libc::geteuid() }
                && (meta.mode() & 0o022 == 0 || private_ancestor),
            "Config directory must be owned and not writable by other users"
        );
        let name = path.file_name().context("Config has no filename")?;
        let name = std::ffi::CString::new(name.as_bytes())?;
        let lock_name = std::ffi::CString::new([name.as_bytes(), b".lock"].concat())?;
        let lock = Self::open_at(
            &directory,
            &lock_name,
            libc::O_RDWR | libc::O_CREAT | libc::O_NONBLOCK,
        )?;
        let meta = lock.metadata()?;
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
                return Err(error.into());
            }
        }
        let current =
            Self::open_at(&directory, &lock_name, libc::O_RDONLY | libc::O_NONBLOCK)?.metadata()?;
        ensure!(
            current.dev() == meta.dev()
                && current.ino() == meta.ino()
                && lock.metadata()?.nlink() == 1,
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
