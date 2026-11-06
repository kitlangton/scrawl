use crate::secure::{invalid, trusted_directory};
use std::os::{
    fd::{AsRawFd, FromRawFd},
    unix::fs::MetadataExt,
};
use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};
/// Only a successful production finalizer may construct this. This verifies identity and
/// exact published manifest bytes, NOT media quality or the full Apparent bundle schema.
/// Caller must first validate all required artifacts and complete their durability barrier.
pub struct FinalizerSuccess {
    pub(crate) request: String,
    pub(crate) bundle: String,
    pub(crate) path: PathBuf,
    dir: File,
    manifest: Vec<u8>,
}
// Production serializes events, stroke points, transcript and window/context
// histories, not a small receipt. Keep a separate bounded manifest budget.
const MAX_MANIFEST_BYTES: usize = 64 * 1024 * 1024;
impl FinalizerSuccess {
    pub fn validate(request: &str, bundle: &str, path: &Path, manifest: &[u8]) -> io::Result<Self> {
        crate::identity::validate(request, 32)?;
        crate::identity::validate_bundle(bundle)?;
        if manifest.len() > MAX_MANIFEST_BYTES {
            return Err(invalid());
        }
        let value: serde_json::Value = serde_json::from_slice(manifest).map_err(|_| invalid())?;
        if value["id"].as_str() != Some(bundle) || value["version"].as_u64().is_none() {
            return Err(invalid());
        }
        let result = Self {
            request: request.into(),
            bundle: bundle.into(),
            path: path.into(),
            dir: trusted_directory(path)?,
            manifest: manifest.into(),
        };
        result.check()?;
        Ok(result)
    }
    pub(crate) fn check(&self) -> io::Result<()> {
        let current = trusted_directory(&self.path)?.metadata()?;
        let pinned = self.dir.metadata()?;
        if current.dev() != pinned.dev()
            || current.ino() != pinned.ino()
            || pinned.uid() != unsafe { libc::geteuid() }
            || pinned.mode() & 0o022 != 0
            || pinned.nlink() == 0
        {
            return Err(invalid());
        }
        for marker in [c".unsafe-video", c"unfinished.json"] {
            let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: pinned directory, static C strings, writable stat buffer.
            let rc = unsafe {
                libc::fstatat(
                    self.dir.as_raw_fd(),
                    marker.as_ptr(),
                    st.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if rc == 0 || io::Error::last_os_error().kind() != io::ErrorKind::NotFound {
                return Err(invalid());
            }
        }
        // SAFETY: pinned directory, static leaf; descriptor ownership transferred once.
        let fd = unsafe {
            libc::openat(
                self.dir.as_raw_fd(),
                c"apparent.json".as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let m = file.metadata()?;
        if !m.is_file()
            || m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o022 != 0
            || m.nlink() != 1
        {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        (&file)
            .take((MAX_MANIFEST_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes != self.manifest {
            return Err(invalid());
        }
        file.sync_all()?;
        self.dir.sync_all()
    }
}
