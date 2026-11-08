//! Descriptor-relative filesystem boundary. No operation follows a managed symlink.
use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

pub(crate) fn invalid() -> io::Error {
    io::Error::other("unsafe or inconsistent engine state")
}
fn name(s: &str) -> io::Result<CString> {
    if s.is_empty() || matches!(s, "." | "..") || s.contains('/') {
        return Err(invalid());
    }
    CString::new(s).map_err(|_| invalid())
}
fn cvt(fd: i32) -> io::Result<File> {
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: successful open/openat returns a new owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
pub(crate) fn directory(path: &Path) -> io::Result<File> {
    walk_directory(path, false)
}
/// Output paths must descend through root/current-user-owned, non-writable
/// ancestors. Root-owned sticky directories (e.g. /tmp) protect owned entries.
/// The final directory must always be current-user-owned and non-writable.
pub(crate) fn trusted_directory(path: &Path) -> io::Result<File> {
    walk_directory(path, true)
}
fn walk_directory(path: &Path, trusted: bool) -> io::Result<File> {
    if !path.is_absolute() {
        return Err(invalid());
    }
    let mut dir = File::open("/")?;
    for part in path.components() {
        let Component::Normal(part) = part else {
            if part == Component::RootDir {
                continue;
            }
            return Err(invalid());
        };
        let part = CString::new(part.as_bytes()).map_err(|_| invalid())?;
        // SAFETY: valid C string and live parent descriptor; no symlink traversal.
        dir = cvt(unsafe {
            libc::openat(
                dir.as_raw_fd(),
                part.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        if trusted {
            let m = dir.metadata()?;
            let uid = unsafe { libc::geteuid() };
            let sticky_root = m.uid() == 0 && m.mode() & u32::from(libc::S_ISVTX) != 0;
            if (m.uid() != 0 && m.uid() != uid) || (m.mode() & 0o022 != 0 && !sticky_root) {
                return Err(invalid());
            }
        }
    }
    if trusted {
        let m = dir.metadata()?;
        if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o022 != 0 {
            return Err(invalid());
        }
    }
    Ok(dir)
}
pub(crate) struct Store {
    dir: File,
    path: PathBuf,
    parent: Option<std::sync::Arc<Store>>,
}
impl Store {
    /// The caller provisions the one stable runtime directory; never chmod unsafe state.
    pub fn open(path: &Path) -> io::Result<Self> {
        let dir = directory(path)?;
        let m = dir.metadata()?;
        // SAFETY: geteuid has no preconditions.
        if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o777 != 0o700 {
            return Err(invalid());
        }
        Ok(Self {
            dir,
            path: path.into(),
            parent: None,
        })
    }
    /// Permanent private namespace, created only while the root transaction is held.
    pub fn child(self: &std::sync::Arc<Self>, key: &str) -> io::Result<std::sync::Arc<Self>> {
        self.check()?;
        let leaf = name(key)?;
        let rc = unsafe { libc::mkdirat(self.dir.as_raw_fd(), leaf.as_ptr(), 0o700) };
        if rc != 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
            return Err(io::Error::last_os_error());
        }
        let dir = cvt(unsafe {
            libc::openat(
                self.dir.as_raw_fd(),
                leaf.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        let child = std::sync::Arc::new(Self {
            dir,
            path: self.path.join(key),
            parent: Some(self.clone()),
        });
        child.check()?;
        child.dir.sync_all()?;
        self.dir.sync_all()?;
        Ok(child)
    }
    pub fn check(&self) -> io::Result<()> {
        if let Some(parent) = &self.parent {
            parent.check()?;
        }
        let m = self.dir.metadata()?;
        let current = directory(&self.path)?.metadata()?;
        if m.dev() != current.dev()
            || m.ino() != current.ino()
            || m.mode() & 0o777 != 0o700
            || m.uid() != unsafe { libc::geteuid() }
            || m.nlink() == 0
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn file(&self, key: &str, create: bool) -> io::Result<File> {
        self.check()?;
        let key = name(key)?;
        let flags = libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
        // SAFETY: pinned directory and checked NUL-free leaf; O_EXCL never replaces.
        let file = cvt(unsafe {
            libc::openat(
                self.dir.as_raw_fd(),
                key.as_ptr(),
                flags
                    | if create {
                        libc::O_CREAT | libc::O_EXCL
                    } else {
                        0
                    },
                0o600,
            )
        })?;
        self.validate(&file)?;
        Ok(file)
    }
    pub fn validate(&self, f: &File) -> io::Result<()> {
        let m = f.metadata()?;
        // SAFETY: geteuid has no preconditions.
        if !m.is_file() || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o777 != 0o600 {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn link(&self, from: &str, to: &str) -> io::Result<()> {
        let (from, to) = (name(from)?, name(to)?);
        // SAFETY: both paths are NUL-free leaves relative to the pinned directory.
        let rc = unsafe {
            libc::linkat(
                self.dir.as_raw_fd(),
                from.as_ptr(),
                self.dir.as_raw_fd(),
                to.as_ptr(),
                0,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        self.dir.sync_all()
    }
    pub fn read(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        match self.file(key, false) {
            Ok(f) => {
                if f.metadata()?.nlink() != 1 {
                    return Err(invalid());
                }
                let mut bytes = Vec::new();
                f.take(65537).read_to_end(&mut bytes)?;
                if bytes.len() > 65536 {
                    return Err(invalid());
                }
                Ok(Some(bytes))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    /// Atomic replace + file/directory fsync; callers hold the metadata transaction lock.
    pub fn write(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.check()?;
        // Refuse a symlink, unsafe file or hardlink instead of replacing it silently.
        let _ = self.read(key)?;
        let tmp = format!("tmp-{}", crate::identity::random_hex(16)?);
        let (from, to) = (name(&tmp)?, name(key)?);
        let mut f = self.file(&tmp, true)?;
        let result = (|| {
            f.write_all(bytes)?;
            f.sync_all()?;
            // SAFETY: leaf names in the same pinned directory. Never moves lock files.
            let rc = unsafe {
                libc::renameat(
                    self.dir.as_raw_fd(),
                    from.as_ptr(),
                    self.dir.as_raw_fd(),
                    to.as_ptr(),
                )
            };
            if rc != 0 {
                return Err(io::Error::last_os_error());
            }
            self.dir.sync_all()
        })();
        if result.is_err() {
            // Best-effort cleanup of our exclusive temporary leaf. After rename
            // it is absent: never unlink the published record or any lock anchor.
            // Preserve the original publication error even if cleanup also fails.
            unsafe { libc::unlinkat(self.dir.as_raw_fd(), from.as_ptr(), 0) };
            let _ = self.dir.sync_all();
        }
        result
    }
}
