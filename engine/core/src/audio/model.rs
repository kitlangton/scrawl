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

/// A read-only descriptor whose declared bytes were verified, with unchanged file metadata.
/// The caller owns model selection and the independently trusted expected size/hash.
/// Holding this FD defeats pathname replacement; it does NOT make writable bytes immutable.
/// Revalidate after native use, before using its result in original material sealing.
pub struct ModelFile {
    file: File,
    identity: Identity,
    sha256: String,
}
impl ModelFile {
    pub fn verify(file: File, bytes: u64, sha256: &str, deadline: Instant) -> Result<Self> {
        ensure!(bytes > 0 && bytes <= MAX_BYTES, "model byte bound");
        ensure!(
            sha256.len() == 64
                && sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "model digest must be lowercase SHA256"
        );
        let m = file.metadata()?;
        let mode = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
        ensure!(
            mode >= 0 && mode & libc::O_ACCMODE == libc::O_RDONLY,
            "model descriptor must be read-only"
        );
        ensure!(
            m.is_file()
                && m.nlink() == 1
                && m.len() == bytes
                && (m.uid() == unsafe { libc::geteuid() } || m.uid() == 0)
                && m.mode() & 0o022 == 0,
            "model file identity/mode/size mismatch"
        );
        let pin = Self {
            file,
            identity: Identity::read(&m),
            sha256: sha256.to_owned(),
        };
        pin.revalidate(deadline)?;
        Ok(pin)
    }

    /// Bounded-memory byte verification; preserves the caller's descriptor offset.
    /// A late/blocking filesystem read cannot yield success. Hard interruption still belongs
    /// to the caller's existing supervised process deadline, not this synchronous file API.
    pub fn revalidate(&self, deadline: Instant) -> Result<()> {
        ensure!(Instant::now() < deadline, "model verification deadline");
        let before = self.file.metadata()?;
        ensure!(
            before.nlink() == 1
                && before.mode() & 0o022 == 0
                && (before.uid() == unsafe { libc::geteuid() } || before.uid() == 0),
            "model ownership/mode changed"
        );
        ensure!(Identity::read(&before) == self.identity, "model changed");
        let mut digest = Sha256::new();
        let mut offset = 0u64;
        let mut chunk = [0u8; 64 * 1024];
        while offset < self.identity.size {
            ensure!(Instant::now() < deadline, "model verification deadline");
            let size = chunk.len().min((self.identity.size - offset) as usize);
            let n = self.file.read_at(&mut chunk[..size], offset)?;
            ensure!(Instant::now() < deadline, "model verification deadline");
            ensure!(n > 0, "model truncated");
            digest.update(&chunk[..n]);
            offset += n as u64;
        }
        let after = self.file.metadata()?;
        ensure!(
            after.nlink() == 1
                && after.mode() & 0o022 == 0
                && (after.uid() == unsafe { libc::geteuid() } || after.uid() == 0),
            "model ownership/mode changed during verification"
        );
        ensure!(
            Identity::read(&after) == self.identity,
            "model changed during verification"
        );
        ensure!(
            format!("{:x}", digest.finalize()) == self.sha256,
            "model checksum mismatch"
        );
        ensure!(Instant::now() < deadline, "model verification deadline");
        Ok(())
    }
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
        let pin = ModelFile::verify(file, 5, HASH, deadline()).unwrap();
        assert_eq!((&pin.file).stream_position().unwrap(), 3);
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
    fn changed_same_length_bytes_never_revalidate() {
        let f = Fixture::new();
        let pin = ModelFile::verify(f.file(), 5, HASH, deadline()).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(f.path())
            .unwrap()
            .write_all(b"world")
            .unwrap();
        assert!(pin.revalidate(deadline()).is_err());
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
