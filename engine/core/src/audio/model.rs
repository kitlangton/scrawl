//! Explicit local model bytes, not model/inference readiness or a downloader.
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::fs::{File, Metadata};
use std::os::{
    fd::{AsRawFd, RawFd},
    unix::fs::{FileExt, MetadataExt},
};
use std::time::Instant;

const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl Identity {
    fn read(m: &Metadata) -> Self {
        Self {
            device: m.dev(),
            inode: m.ino(),
            size: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        }
    }
}

/// An immutable Linux memfd snapshot of explicitly declared local model bytes.
/// The original source is never chmodded or rewritten. Its owner may mutate it:
/// native loading uses only the copied, expected-hash-verified, write-sealed snapshot.
/// The caller must admit model-size snapshot memory PLUS native working memory.
pub struct ModelFile {
    file: File,
    identity: Identity,
    sha256: String,
}
impl ModelFile {
    pub fn verify(source: File, bytes: u64, sha256: &str, deadline: Instant) -> Result<Self> {
        use std::os::fd::FromRawFd;
        ensure!(Instant::now() < deadline, "model verification deadline");
        ensure!(bytes > 0 && bytes <= MAX_BYTES, "model byte bound");
        ensure!(
            sha256.len() == 64
                && sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "model digest must be lowercase SHA256"
        );
        let m = source.metadata()?;
        let mode = unsafe { libc::fcntl(source.as_raw_fd(), libc::F_GETFL) };
        ensure!(
            mode >= 0 && mode & libc::O_ACCMODE == libc::O_RDONLY,
            "model source descriptor must be read-only"
        );
        ensure!(
            m.is_file()
                && m.nlink() == 1
                && m.len() == bytes
                && (m.uid() == unsafe { libc::geteuid() } || m.uid() == 0)
                && m.mode() & 0o022 == 0,
            "model source identity/mode/size mismatch"
        );
        let source_identity = Identity::read(&m);
        let fd = unsafe {
            libc::memfd_create(
                c"apparent-speech-model".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let mut offset = 0;
        let mut chunk = [0u8; 64 * 1024];
        while offset < bytes {
            ensure!(Instant::now() < deadline, "model snapshot deadline");
            let size = chunk.len().min((bytes - offset) as usize);
            let n = source.read_at(&mut chunk[..size], offset)?;
            ensure!(n > 0, "model source truncated");
            file.write_all_at(&chunk[..n], offset)?;
            offset += n as u64;
            ensure!(Instant::now() < deadline, "model snapshot deadline");
        }
        ensure!(
            Identity::read(&source.metadata()?) == source_identity,
            "model source changed during snapshot"
        );
        if unsafe { libc::fchmod(fd, 0o400) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, required_seals()) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut digest = Sha256::new();
        let mut offset = 0;
        // Verify the actual already-sealed bytes, not a claim about the mutable source.
        while offset < bytes {
            ensure!(Instant::now() < deadline, "model verification deadline");
            let size = chunk.len().min((bytes - offset) as usize);
            let n = file.read_at(&mut chunk[..size], offset)?;
            ensure!(n > 0, "model snapshot truncated");
            digest.update(&chunk[..n]);
            offset += n as u64;
            ensure!(Instant::now() < deadline, "model verification deadline");
        }
        ensure!(
            format!("{:x}", digest.finalize()) == sha256,
            "model checksum mismatch"
        );
        let pin = Self {
            identity: Identity::read(&file.metadata()?),
            file,
            sha256: sha256.to_owned(),
        };
        pin.revalidate(deadline)?;
        Ok(pin)
    }

    /// Check kernel immutability and identity without rehashing immutable bytes each run.
    /// This detects a late return; hard interruption remains the supervised child's job.
    pub fn revalidate(&self, deadline: Instant) -> Result<()> {
        ensure!(Instant::now() < deadline, "model verification deadline");
        let seals = unsafe { libc::fcntl(self.file.as_raw_fd(), libc::F_GET_SEALS) };
        ensure!(
            seals >= 0 && seals & required_seals() == required_seals(),
            "model snapshot seals missing"
        );
        let meta = self.file.metadata()?;
        ensure!(
            meta.is_file() && meta.mode() & 0o222 == 0 && Identity::read(&meta) == self.identity,
            "model snapshot changed"
        );
        ensure!(Instant::now() < deadline, "model verification deadline");
        Ok(())
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn byte_len(&self) -> u64 {
        self.identity.size
    }
}
fn required_seals() -> libc::c_int {
    libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL
}
impl AsRawFd for ModelFile {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Seek, SeekFrom, Write},
        os::unix::fs::PermissionsExt,
        time::Duration,
    };
    const HASH: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(2)
    }
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("apparent-model-{}", crate::new_id().unwrap()));
            std::fs::create_dir(&root).unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::write(root.join("model"), b"hello").unwrap();
            std::fs::set_permissions(root.join("model"), std::fs::Permissions::from_mode(0o600))
                .unwrap();
            Self(root)
        }
        fn path(&self) -> std::path::PathBuf {
            self.0.join("model")
        }
        fn file(&self) -> File {
            File::open(self.path()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn pin_preserves_offset_and_path_replacement_does_not_rebind_it() {
        let f = Fixture::new();
        let mut file = f.file();
        file.seek(SeekFrom::Start(3)).unwrap();
        let mut original_cursor = file.try_clone().unwrap();
        let pin = ModelFile::verify(file, 5, HASH, deadline()).unwrap();
        assert_eq!(original_cursor.stream_position().unwrap(), 3);
        // Snapshot has its own offset; the original descriptor's cursor is untouched.
        assert_eq!(pin.byte_len(), 5);
        assert_eq!(pin.sha256(), HASH);
        std::fs::rename(f.path(), f.0.join("original")).unwrap();
        std::fs::write(f.path(), b"other").unwrap();
        // The original held FD remains authoritative; rename ctime behavior differs by FS.
        // A new opener must verify its new bytes and cannot silently inherit this pin.
        assert!(ModelFile::verify(f.file(), 5, HASH, deadline()).is_err());
        let mut bytes = [0; 5];
        pin.file.read_exact_at(&mut bytes, 0).unwrap();
        assert_eq!(&bytes, b"hello");
    }
    #[test]
    fn mutable_source_changes_cannot_change_the_verified_native_snapshot() {
        let f = Fixture::new();
        let pin = ModelFile::verify(f.file(), 5, HASH, deadline()).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(f.path())
            .unwrap()
            .write_all(b"world")
            .unwrap();
        pin.revalidate(deadline()).unwrap();
        let mut observed = [0; 5];
        pin.file.read_exact_at(&mut observed, 0).unwrap();
        assert_eq!(&observed, b"hello");
        assert!(pin.file.write_all_at(b"world", 0).is_err());
        assert_eq!(unsafe { libc::ftruncate(pin.as_raw_fd(), 0) }, -1);
        assert_eq!(
            unsafe { libc::fcntl(pin.as_raw_fd(), libc::F_ADD_SEALS, 0) },
            -1
        );
    }
    #[test]
    fn malformed_missing_and_untrusted_model_bytes_refuse() {
        let f = Fixture::new();
        assert!(ModelFile::verify(f.file(), 5, &"0".repeat(64), deadline()).is_err());
        assert!(ModelFile::verify(f.file(), 6, HASH, deadline()).is_err());
        assert!(ModelFile::verify(f.file(), 5, &HASH.to_uppercase(), deadline()).is_err());
        assert!(
            ModelFile::verify(
                std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(f.path())
                    .unwrap(),
                5,
                HASH,
                deadline()
            )
            .is_err()
        );
        std::fs::set_permissions(f.path(), std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(ModelFile::verify(f.file(), 5, HASH, deadline()).is_err());
    }
    #[test]
    fn expired_budget_never_returns_a_verified_file() {
        let f = Fixture::new();
        assert!(ModelFile::verify(f.file(), 5, HASH, Instant::now()).is_err());
    }
}
