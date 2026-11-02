//! Descriptor-relative access. Never follow a managed ancestor or leaf symlink.
use anyhow::{Context, Result, ensure};
use std::ffi::{CStr, CString};
use std::fs::{File, Metadata};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};

use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

pub struct Directory(pub File);

fn owned(fd: i32) -> Result<File> {
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: a successful open/dup returns one new owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

pub fn relative(path: &str) -> Result<Vec<&str>> {
    ensure!(
        !path.is_empty() && path.len() <= 1024 && !path.contains('\\'),
        "unsafe artifact path"
    );
    let parts: Vec<_> = path.split('/').collect();
    ensure!(
        parts.len() <= 16
            && parts
                .iter()
                .all(|p| !p.is_empty() && !matches!(*p, "." | "..") && !p.contains('\0')),
        "unsafe artifact path"
    );
    Ok(parts)
}

fn cstr(name: &str) -> Result<CString> {
    ensure!(relative(name)?.len() == 1, "expected an artifact leaf");
    Ok(CString::new(name)?)
}

impl Directory {
    pub fn open(path: &Path, create: bool) -> Result<Self> {
        Self::open_with_parent_policy(path, create, false)
    }

    /// Pin an existing parent for durability sync, accepting root-owned sticky
    /// directories such as /tmp. Bundle and managed artifact checks stay strict.
    pub(super) fn open_sync_parent(path: &Path) -> Result<Self> {
        Self::open_with_parent_policy(path, false, true)
    }

    fn open_with_parent_policy(path: &Path, create: bool, sync_parent: bool) -> Result<Self> {
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        let mut dir = Self(File::open("/")?);
        let mut private_ancestor = false;
        for component in path.components() {
            match component {
                Component::RootDir | Component::CurDir => continue,
                Component::Normal(name) => {
                    let name = name.to_str().context("non-UTF8 bundle path")?;
                    dir = dir.child(name, create)?;
                    let m = dir.0.metadata()?;
                    let uid = unsafe { libc::geteuid() };
                    let sticky = m.uid() == 0 && m.mode() & libc::S_ISVTX as u32 != 0;
                    ensure!(
                        (m.uid() == 0 || m.uid() == uid)
                            && (m.mode() & 0o022 == 0 || sticky || private_ancestor),
                        "untrusted bundle ancestor"
                    );
                    private_ancestor |= m.uid() == uid && m.mode() & 0o077 == 0;
                }
                _ => anyhow::bail!("unsafe bundle directory path"),
            }
        }
        let m = dir.0.metadata()?;
        ensure!(
            (m.uid() == unsafe { libc::geteuid() } || m.uid() == 0)
                && (m.mode() & 0o022 == 0
                    || (sync_parent && m.uid() == 0 && m.mode() & libc::S_ISVTX as u32 != 0)),
            "untrusted bundle directory owner or permissions"
        );
        if create {
            ensure!(
                m.uid() == unsafe { libc::geteuid() },
                "bundle is not owned by this user"
            );
        }
        Ok(dir)
    }

    pub fn child(&self, leaf: &str, create: bool) -> Result<Self> {
        let name = cstr(leaf)?;
        if create {
            // SAFETY: pinned parent and NUL-free leaf, no traversal.
            let rc = unsafe { libc::mkdirat(self.0.as_raw_fd(), name.as_ptr(), 0o700) };
            if rc != 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        Ok(Self(owned(unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?))
    }

    pub fn leaf(&self, leaf: &str, write: bool) -> Result<File> {
        let name = cstr(leaf)?;
        let flags = libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if write {
                libc::O_RDWR | libc::O_CREAT
            } else {
                libc::O_RDONLY
            };
        let file = owned(unsafe { libc::openat(self.0.as_raw_fd(), name.as_ptr(), flags, 0o600) })?;
        let m = file.metadata()?;
        ensure!(
            m.is_file() && m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o022 == 0,
            "unsafe artifact type, owner or permissions: {leaf}"
        );
        ensure!(m.nlink() == 1, "refusing a hard-linked artifact");
        Ok(file)
    }

    pub fn file(&self, path: &str) -> Result<File> {
        let parts = relative(path)?;
        let mut dir = Self(self.0.try_clone()?);
        for part in &parts[..parts.len() - 1] {
            dir = dir.managed_child(part, false)?;
        }
        dir.leaf(parts[parts.len() - 1], false)
    }

    pub fn write(&self, path: &str, bytes: &[u8]) -> Result<()> {
        let parts = relative(path)?;
        let mut dir = Self(self.0.try_clone()?);
        for part in &parts[..parts.len() - 1] {
            dir = dir.managed_child(part, true)?;
        }
        let mut file = dir.leaf(parts[parts.len() - 1], true)?;
        file.set_len(0)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        dir.0.sync_all()?;
        Ok(())
    }

    fn managed_child(&self, name: &str, create: bool) -> Result<Self> {
        let dir = self.child(name, create)?;
        let m = dir.0.metadata()?;
        ensure!(
            m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o022 == 0,
            "unsafe managed directory"
        );
        Ok(dir)
    }

    fn parent_leaf<'a>(&self, path: &'a str) -> Result<(Self, &'a str)> {
        let parts = relative(path)?;
        let mut dir = Self(self.0.try_clone()?);
        for part in &parts[..parts.len() - 1] {
            dir = dir.managed_child(part, false)?;
        }
        Ok((dir, parts[parts.len() - 1]))
    }

    pub fn copy_to(&self, path: &str, destination: &Self, leaf: &str) -> Result<()> {
        let mut source = self.file(path)?;
        let mut target = destination.leaf(leaf, true)?;
        target.set_len(0)?;
        std::io::copy(&mut source, &mut target)?;
        target.sync_all()?;
        Ok(())
    }

    pub fn rename_to(&self, leaf: &str, destination: &Self, path: &str) -> Result<()> {
        let (parent, name) = destination.parent_leaf(path)?;
        if unsafe {
            libc::renameat(
                self.0.as_raw_fd(),
                cstr(leaf)?.as_ptr(),
                parent.0.as_raw_fd(),
                cstr(name)?.as_ptr(),
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }

    pub fn remove_path(&self, path: &str) -> Result<()> {
        let (parent, leaf) = self.parent_leaf(path)?;
        parent.remove(leaf)
    }

    pub fn remove_directory(&self, leaf: &str) -> Result<()> {
        if unsafe { libc::unlinkat(self.0.as_raw_fd(), cstr(leaf)?.as_ptr(), libc::AT_REMOVEDIR) }
            != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }

    pub fn remove(&self, leaf: &str) -> Result<()> {
        match self.leaf(leaf, false) {
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Ok(());
            }
            result => {
                result?;
            }
        }
        let name = cstr(leaf)?;
        if unsafe { libc::unlinkat(self.0.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }

    pub fn names(&self) -> Result<Vec<String>> {
        // A fresh open has its own directory offset; dup would share enumeration state.
        let fd = owned(unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                c".".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        })?;
        use std::os::fd::IntoRawFd;
        let fd = fd.into_raw_fd();
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            unsafe {
                libc::close(fd);
            }
            return Err(std::io::Error::last_os_error().into());
        }
        struct Stream(*mut libc::DIR);
        impl Drop for Stream {
            fn drop(&mut self) {
                unsafe {
                    libc::closedir(self.0);
                }
            }
        }
        let stream = Stream(stream);
        let mut names = Vec::new();
        loop {
            #[cfg(target_os = "linux")]
            unsafe {
                *libc::__errno_location() = 0;
            }
            #[cfg(target_os = "macos")]
            unsafe {
                *libc::__error() = 0;
            }
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                let error = std::io::Error::last_os_error();
                ensure!(
                    error.raw_os_error() == Some(0),
                    "could not enumerate bundle: {error}"
                );
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
                .to_str()?
                .to_owned();
            if name != "." && name != ".." {
                names.push(name);
            }
            ensure!(names.len() <= 4096, "too many bundle entries");
        }
        names.sort();
        Ok(names)
    }

    pub fn is_directory(&self, leaf: &str) -> Result<bool> {
        let name = cstr(leaf)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let mode = unsafe { stat.assume_init() }.st_mode;
        ensure!(
            mode & libc::S_IFMT != libc::S_IFLNK,
            "symlink in bundle: {leaf}"
        );
        Ok(mode & libc::S_IFMT == libc::S_IFDIR)
    }
}

pub fn bytes(file: &File, cap: u64) -> Result<Vec<u8>> {
    use std::io::{Seek, SeekFrom};
    ensure!(
        file.metadata()?.len() <= cap,
        "artifact exceeds validation budget"
    );
    let mut file = file.try_clone()?;
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= cap,
        "artifact grew beyond validation budget"
    );
    Ok(bytes)
}

pub fn same(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
        && a.mode() == b.mode()
        && a.uid() == b.uid()
        && a.nlink() == b.nlink()
}

pub fn same_directory(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.mode() == b.mode()
        && a.uid() == b.uid()
        && b.nlink() > 0
}

pub fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    })
}
