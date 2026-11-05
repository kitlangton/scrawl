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
    pub fn unlink_owned(&self, n: &str, original: &File) {
        if let Ok(current) = self.read(n) {
            if let (Ok(a), Ok(b)) = (current.metadata(), original.metadata()) {
                if same(&a, &b) {
                    if let Ok(n) = name(n) {
                        unsafe {
                            libc::unlinkat(self.file.as_raw_fd(), n.as_ptr(), 0);
                        }
                    }
                }
            }
        }
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
}
impl Output {
    pub fn create(d: &Decode) -> Result<Self> {
        let p = Path::new(&d.output_directory);
        let parent = Directory::open(p.parent().ok_or("invalid_output")?, true)
            .map_err(|_| "invalid_output")?;
        let leaf = name(
            p.file_name()
                .and_then(|n| n.to_str())
                .ok_or("invalid_output")?,
        )?;
        if unsafe { libc::mkdirat(parent.file.as_raw_fd(), leaf.as_ptr(), 0o700) } != 0 {
            return Err("invalid_output");
        }
        let dir = Directory::open(p, true).map_err(|_| "invalid_output")?;
        parent.check().map_err(|_| "invalid_output")?;
        let mut out = Self { dir, files: vec![] };
        let prepared = (|| {
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
        let _ = self.dir.sync();
    }
    pub fn publish(&self, value: &serde_json::Value) -> Result<()> {
        self.dir.check().map_err(|_| "invalid_output")?;
        let mut f = self.dir.create(".manifest.tmp")?;
        let result = (|| {
            let bytes = serde_json::to_vec(value).map_err(|_| "internal")?;
            f.write_all(&bytes).map_err(|_| "invalid_output")?;
            f.sync_all().map_err(|_| "invalid_output")?;
            let from = name(".manifest.tmp")?;
            let to = name("manifest.json")?;
            if unsafe {
                libc::linkat(
                    self.dir.file.as_raw_fd(),
                    from.as_ptr(),
                    self.dir.file.as_raw_fd(),
                    to.as_ptr(),
                    0,
                )
            } != 0
            {
                return Err("invalid_output");
            }
            self.dir.unlink_owned(".manifest.tmp", &f);
            self.dir.sync()
        })();
        if result.is_err() {
            self.dir.unlink_owned(".manifest.tmp", &f);
            self.dir.unlink_owned("manifest.json", &f);
            let _ = self.dir.sync();
        }
        result
    }
}
