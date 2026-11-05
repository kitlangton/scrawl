//! Descriptor-relative paths and pinned input/output identities. No symlink traversal.
use super::protocol::{Decode, Identity, Result};
use std::ffi::CString;
use std::fs::{File, Metadata};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
use std::path::{Component, Path, PathBuf};

fn owned(fd: i32) -> Result<File> {
    if fd < 0 {
        Err("source_mismatch")
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    struct Root(PathBuf);
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn swapped_temporary_manifest_cannot_change_the_published_inode() {
        let root = Root(std::env::temp_dir().canonicalize().unwrap().join(format!(
            "engine-manifest-swap-{}",
            apparent_capture_engine::new_id().unwrap()
        )));
        std::fs::create_dir(&root.0).unwrap();
        std::fs::set_permissions(&root.0, std::fs::Permissions::from_mode(0o700)).unwrap();
        let dir = Directory::open(&root.0, true).unwrap();
        let claim = dir.create(".engine-media-job").unwrap();
        let output = Output {
            dir,
            files: vec![],
            claim,
        };
        let mut original = output.dir.create(".manifest.tmp").unwrap();
        original.write_all(b"verified manifest").unwrap();
        original.sync_all().unwrap();
        std::fs::rename(
            root.0.join(".manifest.tmp"),
            root.0.join("retained-original"),
        )
        .unwrap();
        let mut replacement = output.dir.create(".manifest.tmp").unwrap();
        replacement.write_all(b"foreign manifest").unwrap();
        // The former pathname primitive publishes the replacement at this boundary.
        assert_eq!(
            unsafe {
                libc::linkat(
                    output.dir.file.as_raw_fd(),
                    c".manifest.tmp".as_ptr(),
                    output.dir.file.as_raw_fd(),
                    c"unsafe.json".as_ptr(),
                    0,
                )
            },
            0
        );
        assert_eq!(
            std::fs::read(root.0.join("unsafe.json")).unwrap(),
            b"foreign manifest"
        );
        output.link_manifest(&original).unwrap();
        assert_eq!(
            std::fs::read(root.0.join("manifest.json")).unwrap(),
            b"verified manifest"
        );
        assert!(!output.dir.unlink_owned(".manifest.tmp", &original));
        assert_eq!(
            std::fs::read(root.0.join(".manifest.tmp")).unwrap(),
            b"foreign manifest"
        );
    }
}
fn name(s: &str) -> Result<CString> {
    if s.is_empty() || s.contains('/') || matches!(s, "." | "..") {
        return Err("source_mismatch");
    }
    CString::new(s).map_err(|_| "source_mismatch")
}
pub fn same(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev() && a.ino() == b.ino()
}
pub fn identity(m: &Metadata) -> Identity {
    Identity {
        device: m.dev().to_string(),
        inode: m.ino().to_string(),
        size: m.len(),
        modified_ns: (i128::from(m.mtime()) * 1_000_000_000 + i128::from(m.mtime_nsec()))
            .to_string(),
    }
}
pub struct Directory {
    pub file: File,
    pub path: PathBuf,
}
impl Directory {
    fn has_only(&self, allowed: &[&str]) -> Result<bool> {
        // A new directory description avoids sharing a consumed readdir offset.
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                c".".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err("invalid_output");
        }
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            unsafe {
                libc::close(fd);
            }
            return Err("invalid_output");
        }
        struct Entries(*mut libc::DIR);
        impl Drop for Entries {
            fn drop(&mut self) {
                unsafe {
                    libc::closedir(self.0);
                }
            }
        }
        let stream = Entries(stream);
        loop {
            #[cfg(target_os = "linux")]
            let errno = unsafe { libc::__errno_location() };
            #[cfg(target_os = "macos")]
            let errno = unsafe { libc::__error() };
            unsafe {
                *errno = 0;
            }
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                return if unsafe { *errno } == 0 {
                    Ok(true)
                } else {
                    Err("invalid_output")
                };
            }
            let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name != b"." && name != b".." && !allowed.iter().any(|n| n.as_bytes() == name) {
                return Ok(false);
            }
        }
    }
    pub fn open(path: &Path, private: bool) -> Result<Self> {
        if !path.is_absolute() {
            return Err("source_mismatch");
        }
        let mut d = File::open("/").map_err(|_| "source_mismatch")?;
        let uid = unsafe { libc::geteuid() };
        for p in path.components() {
            let Component::Normal(p) = p else {
                if p == Component::RootDir {
                    continue;
                }
                return Err("source_mismatch");
            };
            let p = CString::new(p.as_bytes()).map_err(|_| "source_mismatch")?;
            d = owned(unsafe {
                libc::openat(
                    d.as_raw_fd(),
                    p.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            })?;
            let m = d.metadata().map_err(|_| "source_mismatch")?;
            let sticky = m.uid() == 0 && m.mode() & libc::S_ISVTX as u32 != 0;
            if (m.uid() != uid && m.uid() != 0) || (m.mode() & 0o022 != 0 && !sticky) {
                return Err("source_mismatch");
            }
        }
        let m = d.metadata().map_err(|_| "source_mismatch")?;
        if m.uid() != uid || m.mode() & 0o022 != 0 || (private && m.mode() & 0o777 != 0o700) {
            return Err("source_mismatch");
        }
        Ok(Self {
            file: d,
            path: path.to_owned(),
        })
    }
    pub fn check(&self) -> Result<()> {
        let d = Self::open(&self.path, false)?;
        if same(
            &self.file.metadata().map_err(|_| "source_mismatch")?,
            &d.file.metadata().map_err(|_| "source_mismatch")?,
        ) {
            Ok(())
        } else {
            Err("source_mismatch")
        }
    }
    pub fn read(&self, n: &str) -> Result<File> {
        let n = name(n)?;
        let f = owned(unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                n.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        })?;
        let m = f.metadata().map_err(|_| "source_mismatch")?;
        if !m.is_file()
            || m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o022 != 0
            || m.nlink() != 1
        {
            return Err("source_mismatch");
        }
        Ok(f)
    }
    pub fn create(&self, n: &str) -> Result<File> {
        let n = name(n)?;
        owned(unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                n.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        })
        .map_err(|_| "invalid_output")
    }
    pub fn exists(&self, n: &str) -> bool {
        let Ok(n) = name(n) else {
            return true;
        };
        let mut m = std::mem::MaybeUninit::<libc::stat>::uninit();
        let rc = unsafe {
            libc::fstatat(
                self.file.as_raw_fd(),
                n.as_ptr(),
                m.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        rc == 0 || std::io::Error::last_os_error().kind() != std::io::ErrorKind::NotFound
    }
    pub fn sync(&self) -> Result<()> {
        self.file.sync_all().map_err(|_| "invalid_output")
    }
    pub fn unlink_owned(&self, n: &str, original: &File) -> bool {
        let Ok(n) = name(n) else {
            return false;
        };
        // Atomic publication temporarily gives our manifest two hard links.
        // Deletion checks the retained inode, not the source-reader link policy.
        let current = owned(unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                n.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        });
        if let Ok(current) = current {
            if let (Ok(a), Ok(b)) = (current.metadata(), original.metadata()) {
                if same(&a, &b) {
                    return unsafe { libc::unlinkat(self.file.as_raw_fd(), n.as_ptr(), 0) } == 0;
                }
            }
        }
        false
    }
}
pub struct SourcePin {
    pub video: File,
    manifest: File,
    metadata: Identity,
    manifest_identity: Identity,
    bundle: Directory,
    video_dir: Directory,
    video_name: String,
}
impl SourcePin {
    pub fn open(d: &Decode) -> Result<Self> {
        let bundle = Directory::open(Path::new(&d.source.bundle_path), false)?;
        for marker in [
            "unfinished.json",
            ".unsafe-video",
            ".audio-recording",
            ".recovering",
        ] {
            if bundle.exists(marker) {
                return Err("source_incomplete");
            }
        }
        let mut manifest = bundle
            .read("apparent.json")
            .map_err(|_| "source_incomplete")?;
        let mi = identity(&manifest.metadata().map_err(|_| "source_mismatch")?);
        if mi.size > 4 * 1024 * 1024 {
            return Err("source_incomplete");
        }
        let mut bytes = Vec::new();
        (&mut manifest)
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "source_incomplete")?;
        let m: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "source_incomplete")?;
        let relative = Path::new(&d.source.video_path)
            .strip_prefix(&bundle.path)
            .map_err(|_| "source_mismatch")?;
        if m["version"].as_u64() != Some(1)
            || m["id"].as_str() != Some(&d.source.bundle_id)
            || m["video"].as_str() != relative.to_str()
            || m["videoOffsetMs"].as_u64().unwrap_or(0) != d.source.video_offset_ms
            || m["durationMs"].as_u64().unwrap_or(0) == 0
        {
            return Err("source_mismatch");
        }
        let vp = Path::new(&d.source.video_path);
        let video_dir = Directory::open(vp.parent().ok_or("source_mismatch")?, false)?;
        let video_name = vp
            .file_name()
            .and_then(|p| p.to_str())
            .ok_or("source_mismatch")?
            .to_owned();
        let video = video_dir.read(&video_name)?;
        let metadata = identity(&video.metadata().map_err(|_| "source_mismatch")?);
        if metadata != d.source.identity {
            return Err("source_mismatch");
        }
        let pin = Self {
            video,
            manifest,
            metadata,
            manifest_identity: mi,
            bundle,
            video_dir,
            video_name,
        };
        pin.check()?;
        Ok(pin)
    }
    pub fn check(&self) -> Result<()> {
        self.bundle.check()?;
        self.video_dir.check()?;
        for (file, expected, dir, n) in [
            (
                &self.video,
                &self.metadata,
                &self.video_dir,
                self.video_name.as_str(),
            ),
            (
                &self.manifest,
                &self.manifest_identity,
                &self.bundle,
                "apparent.json",
            ),
        ] {
            if identity(&file.metadata().map_err(|_| "source_mismatch")?) != *expected
                || identity(&dir.read(n)?.metadata().map_err(|_| "source_mismatch")?) != *expected
            {
                return Err("source_mismatch");
            }
        }
        for marker in [
            "unfinished.json",
            ".unsafe-video",
            ".audio-recording",
            ".recovering",
        ] {
            if self.bundle.exists(marker) {
                return Err("source_incomplete");
            }
        }
        Ok(())
    }
}
pub struct Output {
    pub dir: Directory,
    pub files: Vec<File>,
    claim: File,
}
impl Output {
    fn link_manifest(&self, file: &File) -> Result<()> {
        let to = name("manifest.json")?;
        // Publish the retained inode, not the mutable temporary pathname.
        #[cfg(target_os = "linux")]
        let (from_dir, from, flags) = (
            libc::AT_FDCWD,
            CString::new(format!("/proc/self/fd/{}", file.as_raw_fd()))
                .map_err(|_| "invalid_output")?,
            libc::AT_SYMLINK_FOLLOW,
        );
        #[cfg(target_os = "macos")]
        let (from_dir, from, flags) = (self.dir.file.as_raw_fd(), name(".manifest.tmp")?, 0);
        if unsafe {
            libc::linkat(
                from_dir,
                from.as_ptr(),
                self.dir.file.as_raw_fd(),
                to.as_ptr(),
                flags,
            )
        } != 0
        {
            return Err("invalid_output");
        }
        let published = owned(unsafe {
            libc::openat(
                self.dir.file.as_raw_fd(),
                to.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        })
        .map_err(|_| "invalid_output")?;
        if !same(
            &published.metadata().map_err(|_| "invalid_output")?,
            &file.metadata().map_err(|_| "invalid_output")?,
        ) {
            return Err("invalid_output");
        }
        Ok(())
    }
    pub fn create(d: &Decode) -> Result<Self> {
        let p = Path::new(&d.output_directory);
        let parent = Directory::open(p.parent().ok_or("invalid_output")?, true)
            .map_err(|_| "invalid_output")?;
        let leaf = name(
            p.file_name()
                .and_then(|n| n.to_str())
                .ok_or("invalid_output")?,
        )?;
        if unsafe { libc::mkdirat(parent.file.as_raw_fd(), leaf.as_ptr(), 0o700) } != 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err("invalid_output");
        }
        let dir = Directory::open(p, true).map_err(|_| "invalid_output")?;
        parent.check().map_err(|_| "invalid_output")?;
        if !dir.has_only(&[])? {
            return Err("invalid_output");
        }
        // Racing providers cannot both claim a caller-provisioned empty directory.
        let claim = dir.create(".engine-media-job")?;
        let mut out = Self {
            dir,
            files: vec![],
            claim,
        };
        let prepared = (|| {
            out.claim
                .write_all(d.job_id.as_bytes())
                .map_err(|_| "invalid_output")?;
            out.claim.sync_all().map_err(|_| "invalid_output")?;
            if !out.dir.has_only(&[".engine-media-job"])? {
                return Err("invalid_output");
            }
            for i in 1..=d.requested_bundle_ms.len() {
                out.files
                    .push(out.dir.create(&format!("frame-{i:03}.png"))?);
            }
            out.dir.sync()?;
            parent.sync()
        })();
        if let Err(e) = prepared {
            out.cleanup();
            return Err(e);
        }
        Ok(out)
    }
    pub fn cleanup(&self) {
        for (i, f) in self.files.iter().enumerate() {
            self.dir.unlink_owned(&format!("frame-{:03}.png", i + 1), f);
        }
        self.dir.unlink_owned(".engine-media-job", &self.claim);
        let _ = self.dir.sync();
    }
    pub fn publish(&self, value: &serde_json::Value) -> Result<()> {
        self.dir.check().map_err(|_| "invalid_output")?;
        let claim = self
            .dir
            .read(".engine-media-job")
            .map_err(|_| "invalid_output")?;
        if identity(&claim.metadata().map_err(|_| "invalid_output")?)
            != identity(&self.claim.metadata().map_err(|_| "invalid_output")?)
        {
            return Err("invalid_output");
        }
        let mut f = self.dir.create(".manifest.tmp")?;
        let result = (|| {
            let bytes = serde_json::to_vec(value).map_err(|_| "internal")?;
            f.write_all(&bytes).map_err(|_| "invalid_output")?;
            f.sync_all().map_err(|_| "invalid_output")?;
            self.link_manifest(&f)?;
            if !self.dir.unlink_owned(".manifest.tmp", &f)
                || !self.dir.unlink_owned(".engine-media-job", &self.claim)
            {
                return Err("invalid_output");
            }
            self.dir.sync()?;
            let mut published = self
                .dir
                .read("manifest.json")
                .map_err(|_| "invalid_output")?;
            if !same(
                &published.metadata().map_err(|_| "invalid_output")?,
                &f.metadata().map_err(|_| "invalid_output")?,
            ) {
                return Err("invalid_output");
            }
            let mut actual = Vec::new();
            (&mut published)
                .take(bytes.len() as u64 + 1)
                .read_to_end(&mut actual)
                .map_err(|_| "invalid_output")?;
            if actual != bytes {
                return Err("invalid_output");
            }
            Ok(())
        })();
        if result.is_err() {
            self.dir.unlink_owned(".manifest.tmp", &f);
            self.dir.unlink_owned("manifest.json", &f);
            let _ = self.dir.sync();
        }
        result
    }
}
