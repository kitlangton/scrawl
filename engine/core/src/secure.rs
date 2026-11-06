//! Descriptor-relative filesystem boundary. No operation follows a managed symlink.
use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

#[cfg(all(feature = "publication", target_os = "macos"))]
unsafe extern "C" {
    fn renameatx_np(
        from_fd: i32,
        from: *const libc::c_char,
        to_fd: i32,
        to: *const libc::c_char,
        flags: u32,
    ) -> i32;
}

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
pub fn directory(path: &Path) -> io::Result<File> {
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
pub struct Store {
    dir: File,
    path: PathBuf,
    parent: Option<std::sync::Arc<Store>>,
    private: bool,
}
impl Store {
    /// Open or provision one 0700 directory beneath an already existing parent.
    pub fn ensure(path: &Path) -> io::Result<Self> {
        Self::provision(path, false)
    }
    pub fn create(path: &Path) -> io::Result<Self> {
        Self::provision(path, true)
    }
    fn provision(path: &Path, exclusive: bool) -> io::Result<Self> {
        let parent_path = path.parent().ok_or_else(invalid)?;
        let parent = directory(parent_path)?;
        let leaf = name(
            path.file_name()
                .ok_or_else(invalid)?
                .to_str()
                .ok_or_else(invalid)?,
        )?;
        let rc = unsafe { libc::mkdirat(parent.as_raw_fd(), leaf.as_ptr(), 0o700) };
        let created = rc == 0;
        if rc != 0 {
            let error = io::Error::last_os_error();
            if exclusive || error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
        let store = Self::open(path)?;
        store.check()?;
        if created {
            store.dir.sync_all()?;
            parent.sync_all()?;
        }
        Ok(store)
    }
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
            private: true,
        })
    }
    /// Open a pinned output parent whose full path has trusted ownership and no
    /// group/other write access. Child Stores remain private 0700 directories.
    #[cfg(feature = "publication")]
    pub(crate) fn open_trusted_parent(path: &Path) -> io::Result<Self> {
        let dir = trusted_directory(path)?;
        let metadata = dir.metadata()?;
        if metadata.mode() & 0o022 != 0 {
            return Err(invalid());
        }
        Ok(Self {
            dir,
            path: path.into(),
            parent: None,
            private: false,
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
            private: true,
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
            || if self.private {
                m.mode() & 0o777 != 0o700
            } else {
                m.mode() & 0o022 != 0
            }
            || m.uid() != unsafe { libc::geteuid() }
            || m.nlink() == 0
        {
            return Err(invalid());
        }
        Ok(())
    }
    /// Clone the pinned directory descriptor after revalidating its path identity.
    pub fn directory_file(&self) -> io::Result<File> {
        self.check()?;
        self.dir.try_clone()
    }
    #[cfg(feature = "publication")]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    /// Open a directory leaf without following symlinks and pin its identity.
    #[cfg(feature = "publication")]
    pub(crate) fn open_child_directory(&self, key: &str) -> io::Result<File> {
        self.check()?;
        let key = name(key)?;
        cvt(unsafe {
            libc::openat(
                self.dir.as_raw_fd(),
                key.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })
    }
    #[cfg(feature = "publication")]
    pub(crate) fn entry_exists(&self, key: &str) -> io::Result<bool> {
        self.check()?;
        let key = name(key)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let rc = unsafe {
            libc::fstatat(
                self.dir.as_raw_fd(),
                key.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if rc == 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::NotFound {
            Ok(false)
        } else {
            Err(error)
        }
    }
    /// Remove one failed private snapshot leaf only if its named inode still
    /// matches the pinned, single-link file opened by this Store.
    #[cfg(feature = "publication")]
    pub(crate) fn remove_owned_file(&self, key: &str) -> io::Result<()> {
        use std::os::unix::fs::MetadataExt;
        self.check()?;
        let key_c = name(key)?;
        let file = self.file(key, false)?;
        let before = file.metadata()?;
        if before.uid() != unsafe { libc::geteuid() }
            || before.mode() & 0o777 != 0o600
            || before.nlink() != 1
        {
            return Err(invalid());
        }
        let named = cvt(unsafe {
            libc::openat(
                self.dir.as_raw_fd(),
                key_c.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        })?;
        let after = named.metadata()?;
        if before.dev() != after.dev() || before.ino() != after.ino() || after.nlink() != 1 {
            return Err(invalid());
        }
        if unsafe { libc::unlinkat(self.dir.as_raw_fd(), key_c.as_ptr(), 0) } != 0 {
            return Err(io::Error::last_os_error());
        }
        self.dir.sync_all()
    }
    /// Rename a private directory to an absent sibling using the platform's
    /// atomic exclusive primitive. `expected` pins the source inode being moved.
    #[cfg(feature = "publication")]
    pub(crate) fn rename_directory_noreplace(
        &self,
        from: &str,
        to: &str,
        expected: &File,
    ) -> io::Result<()> {
        use std::os::unix::fs::MetadataExt;
        self.check()?;
        let (from_name, to_name) = (name(from)?, name(to)?);
        let expected_meta = expected.metadata()?;
        if !expected_meta.is_dir()
            || expected_meta.uid() != unsafe { libc::geteuid() }
            || expected_meta.mode() & 0o777 != 0o700
        {
            return Err(invalid());
        }
        let source = self.open_child_directory(from)?;
        let source_meta = source.metadata()?;
        if source_meta.dev() != expected_meta.dev() || source_meta.ino() != expected_meta.ino() {
            return Err(invalid());
        }
        #[cfg(target_os = "linux")]
        let rc = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                self.dir.as_raw_fd(),
                from_name.as_ptr(),
                self.dir.as_raw_fd(),
                to_name.as_ptr(),
                1u32,
            ) as i32
        };
        #[cfg(target_os = "macos")]
        let rc = unsafe {
            renameatx_np(
                self.dir.as_raw_fd(),
                from_name.as_ptr(),
                self.dir.as_raw_fd(),
                to_name.as_ptr(),
                0x0000_0004,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        let published = self.open_child_directory(to)?;
        let published_meta = published.metadata()?;
        if published_meta.dev() != expected_meta.dev()
            || published_meta.ino() != expected_meta.ino()
        {
            return Err(invalid());
        }
        match self.open_child_directory(from) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.dir.sync_all(),
            _ => Err(invalid()),
        }
    }
    pub fn directory_metadata(&self) -> io::Result<std::fs::Metadata> {
        self.check()?;
        self.dir.metadata()
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
    /// Publish a new record atomically. Existing names are never replaced.
    pub fn write_new(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.check()?;
        let tmp = format!("tmp-{}", crate::identity::random_hex(16)?);
        let from = name(&tmp)?;
        let _to = name(key)?;
        let mut f = self.file(&tmp, true)?;
        let result = (|| {
            f.write_all(bytes)?;
            f.sync_all()?;
            self.link(&tmp, key)?;
            let rc = unsafe { libc::unlinkat(self.dir.as_raw_fd(), from.as_ptr(), 0) };
            if rc != 0 {
                return Err(io::Error::last_os_error());
            }
            self.dir.sync_all()
        })();
        if result.is_err() {
            if let (Ok(expected), Ok(candidate)) = (f.metadata(), self.file(&tmp, false)) {
                if candidate.metadata().is_ok_and(|actual| {
                    actual.dev() == expected.dev() && actual.ino() == expected.ino()
                }) {
                    unsafe { libc::unlinkat(self.dir.as_raw_fd(), from.as_ptr(), 0) };
                }
            }
        }
        result
    }
    /// Replace a journal only if the currently named inode is the one observed by
    /// `read`; Linux exchange lets us verify the displaced identity before unlink.
    #[cfg(target_os = "linux")]
    pub fn replace_checked(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        use std::os::unix::fs::MetadataExt;
        self.check()?;
        let old = self.file(key, false)?;
        if old.metadata()?.nlink() != 1 {
            return Err(invalid());
        }
        let before = old.metadata()?;
        let tmp = format!("tmp-{}", crate::identity::random_hex(16)?);
        let (from, to) = (name(&tmp)?, name(key)?);
        let mut new = self.file(&tmp, true)?;
        let result = (|| {
            new.write_all(bytes)?;
            new.sync_all()?;
            let rc = unsafe {
                libc::syscall(
                    libc::SYS_renameat2,
                    self.dir.as_raw_fd(),
                    from.as_ptr(),
                    self.dir.as_raw_fd(),
                    to.as_ptr(),
                    2u32,
                )
            };
            if rc != 0 {
                return Err(io::Error::last_os_error());
            }
            let displaced = self.file(&tmp, false)?;
            let after = displaced.metadata()?;
            if before.dev() != after.dev() || before.ino() != after.ino() {
                unsafe {
                    libc::syscall(
                        libc::SYS_renameat2,
                        self.dir.as_raw_fd(),
                        from.as_ptr(),
                        self.dir.as_raw_fd(),
                        to.as_ptr(),
                        2u32,
                    );
                }
                return Err(invalid());
            }
            let published = self.file(key, false)?.metadata()?;
            let created = new.metadata()?;
            if published.dev() != created.dev() || published.ino() != created.ino() {
                return Err(invalid());
            }
            if unsafe { libc::unlinkat(self.dir.as_raw_fd(), from.as_ptr(), 0) } != 0 {
                return Err(io::Error::last_os_error());
            }
            self.dir.sync_all()
        })();
        if result.is_err() {
            if let (Ok(expected), Ok(candidate)) = (new.metadata(), self.file(&tmp, false)) {
                if candidate.metadata().is_ok_and(|actual| {
                    actual.dev() == expected.dev() && actual.ino() == expected.ino()
                }) {
                    unsafe { libc::unlinkat(self.dir.as_raw_fd(), from.as_ptr(), 0) };
                }
            }
        }
        result
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
impl AsRawFd for Store {
    fn as_raw_fd(&self) -> i32 {
        self.dir.as_raw_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::Store;
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

    fn temp() -> PathBuf {
        // macOS may spell its temporary root through the /var symlink.
        // Resolve the fixture root; Store must still reject aliased paths.
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "apparent-secure-{}",
            crate::identity::new_id().unwrap()
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn store_new_record_is_bounded_private_and_non_replacing() {
        let path = temp();
        let store = Store::open(&path).unwrap();
        store.write_new("record.json", b"first").unwrap();
        assert!(store.write_new("record.json", b"second").is_err());
        assert_eq!(
            store.read("record.json").unwrap().as_deref(),
            Some(&b"first"[..])
        );
        fs::remove_dir_all(path).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn checked_replace_preserves_symlink_and_hardlink_targets() {
        let path = temp();
        let store = Store::open(&path).unwrap();
        store.write_new("record.json", b"first").unwrap();
        store.replace_checked("record.json", b"second").unwrap();
        assert_eq!(
            store.read("record.json").unwrap().as_deref(),
            Some(&b"second"[..])
        );
        fs::hard_link(path.join("record.json"), path.join("alias.json")).unwrap();
        assert!(store.replace_checked("record.json", b"third").is_err());
        assert_eq!(fs::read(path.join("alias.json")).unwrap(), b"second");
        fs::remove_file(path.join("alias.json")).unwrap();
        let outside = path.join("outside");
        fs::write(&outside, b"outside").unwrap();
        fs::remove_file(path.join("record.json")).unwrap();
        std::os::unix::fs::symlink(&outside, path.join("record.json")).unwrap();
        assert!(store.replace_checked("record.json", b"third").is_err());
        assert!(
            fs::symlink_metadata(path.join("record.json"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_dir_all(path).unwrap();
    }
}
