//! Descriptor-relative filesystem boundary. No operation follows a managed symlink.
use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

#[cfg(target_os = "macos")]
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
        let child = self.existing_child(key)?;
        child.dir.sync_all()?;
        self.dir.sync_all()?;
        Ok(child)
    }
    /// Exclusively create a private child directory. Unlike [`Self::child`],
    /// an existing name is never adopted as native-owned output.
    pub fn create_child(
        self: &std::sync::Arc<Self>,
        key: &str,
    ) -> io::Result<std::sync::Arc<Self>> {
        self.check()?;
        let leaf = name(key)?;
        if unsafe { libc::mkdirat(self.dir.as_raw_fd(), leaf.as_ptr(), 0o700) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let child = self.existing_child(key)?;
        child.dir.sync_all()?;
        self.dir.sync_all()?;
        Ok(child)
    }
    /// Open an existing private child without provisioning state on lookup.
    pub fn existing_child(
        self: &std::sync::Arc<Self>,
        key: &str,
    ) -> io::Result<std::sync::Arc<Self>> {
        self.check()?;
        let leaf = name(key)?;
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
    pub fn entry_exists(&self, key: &str) -> io::Result<bool> {
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
    /// Unlink a private regular file only when the current leaf still names
    /// the exact device/inode recorded by its native creator. Missing leaves
    /// are already clean; replacements, links and unsafe files fail closed.
    pub fn remove_file_if_identity(
        &self,
        key: &str,
        expected_device: u64,
        expected_inode: u64,
    ) -> io::Result<bool> {
        self.remove_file_if_identity_with(key, expected_device, expected_inode, || Ok(()))
    }

    fn remove_file_if_identity_with(
        &self,
        key: &str,
        expected_device: u64,
        expected_inode: u64,
        before_rename: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<bool> {
        use std::os::unix::fs::MetadataExt;
        self.check()?;
        let key = name(key)?;
        let file = match cvt(unsafe {
            libc::openat(
                self.dir.as_raw_fd(),
                key.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        }) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o600
            || metadata.nlink() != 1
            || metadata.dev() != expected_device
            || metadata.ino() != expected_inode
        {
            return Err(invalid());
        }

        // Atomically move the current leaf to a fresh private name before
        // deciding what may be removed. Checking an open FD and then unlinking
        // `key` can otherwise delete a replacement installed between them.
        let tmp = format!("remove-{}", crate::identity::random_hex(16)?);
        let from = name(&tmp)?;
        if let Err(error) = before_rename() {
            return Err(error);
        }
        if let Err(error) = self.rename_noreplace(&key, &from) {
            if error.kind() == io::ErrorKind::NotFound {
                return Ok(false);
            }
            return Err(error);
        }
        let moved_matches =
            self.named_identity_matches(&from, expected_device, expected_inode, libc::S_IFREG);
        if !moved_matches {
            self.rename_noreplace(&from, &key)?;
            return Err(invalid());
        }
        if unsafe { libc::unlinkat(self.dir.as_raw_fd(), from.as_ptr(), 0) } != 0 {
            let error = io::Error::last_os_error();
            let _ = self.rename_noreplace(&from, &key);
            return Err(error);
        }
        self.dir.sync_all()?;
        Ok(true)
    }
    /// Remove an empty private child directory only when its named device and
    /// inode still match the native-created directory recorded by its owner.
    pub fn remove_empty_child_if_identity(
        &self,
        key: &str,
        expected_device: u64,
        expected_inode: u64,
    ) -> io::Result<bool> {
        self.remove_empty_child_if_identity_with(key, expected_device, expected_inode, || Ok(()))
    }

    fn remove_empty_child_if_identity_with(
        &self,
        key: &str,
        expected_device: u64,
        expected_inode: u64,
        before_rename: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<bool> {
        use std::os::unix::fs::MetadataExt;
        self.check()?;
        let key = name(key)?;
        let directory = match cvt(unsafe {
            libc::openat(
                self.dir.as_raw_fd(),
                key.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        }) {
            Ok(directory) => directory,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        let metadata = directory.metadata()?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o700
            || metadata.dev() != expected_device
            || metadata.ino() != expected_inode
        {
            return Err(invalid());
        }

        let tmp = format!("remove-{}", crate::identity::random_hex(16)?);
        let from = name(&tmp)?;
        if let Err(error) = before_rename() {
            return Err(error);
        }
        if let Err(error) = self.rename_noreplace(&key, &from) {
            if error.kind() == io::ErrorKind::NotFound {
                return Ok(false);
            }
            return Err(error);
        }
        let moved_matches =
            self.named_identity_matches(&from, expected_device, expected_inode, libc::S_IFDIR);
        if !moved_matches {
            self.rename_noreplace(&from, &key)?;
            return Err(invalid());
        }
        if unsafe { libc::unlinkat(self.dir.as_raw_fd(), from.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
            let error = io::Error::last_os_error();
            let _ = self.rename_noreplace(&from, &key);
            return Err(error);
        }
        self.dir.sync_all()?;
        Ok(true)
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
    fn named_identity_matches(
        &self,
        key: &CString,
        expected_device: u64,
        expected_inode: u64,
        expected_type: libc::mode_t,
    ) -> bool {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        unsafe {
            libc::fstatat(
                self.dir.as_raw_fd(),
                key.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            ) == 0
                && (*stat.as_ptr()).st_dev as u64 == expected_device
                && (*stat.as_ptr()).st_ino as u64 == expected_inode
                && ((*stat.as_ptr()).st_mode & libc::S_IFMT) == expected_type
        }
    }

    /// Rename a leaf only when the destination is absent. Unlike check-then-
    /// rename, this cannot overwrite a caller entry that appears concurrently.
    fn rename_noreplace(&self, from: &CString, to: &CString) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        let rc = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                self.dir.as_raw_fd(),
                from.as_ptr(),
                self.dir.as_raw_fd(),
                to.as_ptr(),
                1u32, // RENAME_NOREPLACE
            ) as i32
        };
        #[cfg(target_os = "macos")]
        let rc = unsafe {
            renameatx_np(
                self.dir.as_raw_fd(),
                from.as_ptr(),
                self.dir.as_raw_fd(),
                to.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn exchange(&self, from: &CString, to: &CString) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        let rc = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                self.dir.as_raw_fd(),
                from.as_ptr(),
                self.dir.as_raw_fd(),
                to.as_ptr(),
                2u32, // RENAME_EXCHANGE
            ) as i32
        };
        #[cfg(target_os = "macos")]
        let rc = unsafe {
            renameatx_np(
                self.dir.as_raw_fd(),
                from.as_ptr(),
                self.dir.as_raw_fd(),
                to.as_ptr(),
                libc::RENAME_SWAP,
            )
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Replace a journal only if the named inode is the one observed before the
    /// exchange. Linux RENAME_EXCHANGE and Darwin RENAME_SWAP let us verify the
    /// displaced identity before unlink. Unsupported filesystems fail closed;
    /// there is no pathname rename fallback.
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
            self.exchange(&from, &to)?;
            let displaced = self.file(&tmp, false)?;
            let after = displaced.metadata()?;
            if before.dev() != after.dev() || before.ino() != after.ino() {
                let _ = self.exchange(&from, &to);
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
    use std::{
        fs,
        os::{fd::AsRawFd, unix::fs::PermissionsExt},
        path::PathBuf,
    };

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
    fn existing_child_never_provisions_and_rejects_replaced_links() {
        let path = temp();
        let store = std::sync::Arc::new(Store::open(&path).unwrap());
        assert_eq!(
            store.existing_child("missing").err().unwrap().kind(),
            std::io::ErrorKind::NotFound
        );
        assert!(!path.join("missing").exists());
        let child = store.child("owned").unwrap();
        child.write_new("record", b"retained").unwrap();
        assert_eq!(
            store
                .existing_child("owned")
                .unwrap()
                .read("record")
                .unwrap()
                .unwrap(),
            b"retained"
        );
        fs::rename(path.join("owned"), path.join("retained")).unwrap();
        std::os::unix::fs::symlink(path.join("retained"), path.join("owned")).unwrap();
        assert!(store.existing_child("owned").is_err());
        fs::remove_dir_all(path).unwrap();
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

    #[test]
    fn identity_checked_output_removal_preserves_replacements_and_unknown_entries() {
        use std::os::unix::fs::MetadataExt;

        let path = temp();
        let root = std::sync::Arc::new(Store::open(&path).unwrap());
        let frames = root.create_child("frames").unwrap();
        assert!(root.create_child("frames").is_err());

        frames.write_new("frame.png", b"native frame").unwrap();
        let original = frames.file("frame.png", false).unwrap().metadata().unwrap();
        assert!(
            frames
                .remove_file_if_identity("frame.png", original.dev(), original.ino())
                .unwrap()
        );
        assert!(
            !frames
                .remove_file_if_identity("frame.png", original.dev(), original.ino())
                .unwrap()
        );

        frames
            .write_new("replacement.png", b"foreign replacement")
            .unwrap();
        let replaced_file = frames.file("replacement.png", false).unwrap();
        let replaced = replaced_file.metadata().unwrap();
        fs::remove_file(path.join("frames/replacement.png")).unwrap();
        frames
            .write_new("replacement.png", b"new caller file")
            .unwrap();
        assert!(
            frames
                .remove_file_if_identity("replacement.png", replaced.dev(), replaced.ino())
                .is_err()
        );
        assert_eq!(
            fs::read(path.join("frames/replacement.png")).unwrap(),
            b"new caller file"
        );
        drop(replaced_file);

        fs::write(path.join("frames/unknown.txt"), b"keep me").unwrap();
        let frame_metadata = frames.directory_metadata().unwrap();
        assert!(
            root.remove_empty_child_if_identity(
                "frames",
                frame_metadata.dev(),
                frame_metadata.ino()
            )
            .is_err()
        );
        assert_eq!(
            fs::read(path.join("frames/unknown.txt")).unwrap(),
            b"keep me"
        );
        fs::remove_file(path.join("frames/unknown.txt")).unwrap();

        let original_frames = frames.directory_metadata().unwrap();
        drop(frames);
        fs::rename(path.join("frames"), path.join("retained-frames")).unwrap();
        let replacement = root.create_child("frames").unwrap();
        replacement
            .write_new("keep.txt", b"replacement directory")
            .unwrap();
        assert!(
            root.remove_empty_child_if_identity(
                "frames",
                original_frames.dev(),
                original_frames.ino()
            )
            .is_err()
        );
        assert_eq!(
            fs::read(path.join("frames/keep.txt")).unwrap(),
            b"replacement directory"
        );
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn checked_removal_noreplace_preserves_replacements_at_validation_boundary() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let path = temp();
        let root = std::sync::Arc::new(Store::open(&path).unwrap());

        // Faithfully reproduce the old open/fstat -> interleave -> unlink
        // sequence: the replacement installed after validation is lost.
        root.write_new("race.txt", b"original").unwrap();
        let validated = root.file("race.txt", false).unwrap();
        let before = validated.metadata().unwrap();
        fs::remove_file(path.join("race.txt")).unwrap();
        fs::write(path.join("race.txt"), b"replacement lost by old sequence").unwrap();
        assert_eq!(before.len(), 8);
        let race_name = std::ffi::CString::new("race.txt").unwrap();
        assert_eq!(
            unsafe { libc::unlinkat(root.dir.as_raw_fd(), race_name.as_ptr(), 0) },
            0
        );
        assert!(!path.join("race.txt").exists());

        root.write_new("race.txt", b"original").unwrap();
        let original = root.file("race.txt", false).unwrap().metadata().unwrap();
        let replacement = b"replacement preserved by no-replace rename";
        assert!(root
            .remove_file_if_identity_with("race.txt", original.dev(), original.ino(), || {
                fs::remove_file(path.join("race.txt"))?;
                fs::write(path.join("race.txt"), replacement)?;
                Ok(())
            })
            .is_err());
        assert_eq!(fs::read(path.join("race.txt")).unwrap(), replacement);

        let old_frames = root.create_child("old-frames").unwrap();
        let _validated_directory = old_frames.directory_metadata().unwrap();
        drop(old_frames);
        fs::remove_dir(path.join("old-frames")).unwrap();
        fs::create_dir(path.join("old-frames")).unwrap();
        fs::set_permissions(path.join("old-frames"), fs::Permissions::from_mode(0o700)).unwrap();
        let old_frames_name = std::ffi::CString::new("old-frames").unwrap();
        assert_eq!(
            unsafe {
                libc::unlinkat(
                    root.dir.as_raw_fd(),
                    old_frames_name.as_ptr(),
                    libc::AT_REMOVEDIR,
                )
            },
            0
        );
        assert!(!path.join("old-frames").exists());

        let frames = root.create_child("frames").unwrap();
        let original_directory = frames.directory_metadata().unwrap();
        drop(frames);
        assert!(root
            .remove_empty_child_if_identity_with(
                "frames",
                original_directory.dev(),
                original_directory.ino(),
                || {
                    fs::remove_dir(path.join("frames"))?;
                    fs::create_dir(path.join("frames"))?;
                    fs::set_permissions(path.join("frames"), fs::Permissions::from_mode(0o700))?;
                    Ok(())
                }
            )
            .is_err());
        let preserved = root.existing_child("frames").unwrap();
        let preserved_metadata = preserved.directory_metadata().unwrap();
        assert_ne!(preserved_metadata.ino(), original_directory.ino());
        preserved.write_new("caller.txt", b"preserve").unwrap();
        assert_eq!(
            fs::read(path.join("frames/caller.txt")).unwrap(),
            b"preserve"
        );

        fs::remove_dir_all(path).unwrap();
    }

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

    #[test]
    fn checked_replace_keeps_open_reader_on_original_inode_and_cleans_temporary_leaf() {
        use std::io::Read;
        let path = temp();
        let store = Store::open(&path).unwrap();
        store.write_new("record.json", b"original").unwrap();
        let mut reader = store.file("record.json", false).unwrap();
        store
            .replace_checked("record.json", b"replacement")
            .unwrap();
        let mut original = Vec::new();
        reader.read_to_end(&mut original).unwrap();
        assert_eq!(original, b"original");
        assert_eq!(store.read("record.json").unwrap().unwrap(), b"replacement");
        let entries: Vec<_> = fs::read_dir(&path)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries, ["record.json"]);
        fs::remove_dir_all(path).unwrap();
    }
}
