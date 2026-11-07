//! Descriptor-checked SCK recording output for a trusted native caller.
//!
//! SCK's recording API accepts a URL and may replace a precreated file, so an open-file
//! descriptor URL cannot preserve the inode created by `SecureStore::file`. This adapter
//! gives SCK an absent leaf below the global file-ID URL of a private Store directory, then
//! pins the regular file that appears and checks its identity again after native finalization.
//! The caller must create the Store beneath a trusted parent, keep this guard alive through
//! successful SCK finish and source readback, and ensure the native writer creates a 0600 leaf.
//! The adapter never chmods output and does not clean up a failed or uncertain leaf.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::SecureStore;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    AwaitingNativeLeaf,
    PinnedNativeLeaf,
    Refused,
}

impl FileIdentity {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }

    fn read(file: &File) -> io::Result<Self> {
        Ok(Self::from_metadata(&file.metadata()?))
    }
}

/// Owns the Store guard and a pinned native output file for the whole capture/readback scope.
pub struct SckOutputGuard {
    store: Arc<SecureStore>,
    leaf: String,
    output_url: PathBuf,
    root_identity: FileIdentity,
    state: State,
    file: Option<File>,
    file_identity: Option<FileIdentity>,
}

impl SckOutputGuard {
    /// Build an absent-leaf SCK URL below an already-created private Store.
    ///
    /// `stage` must have been created by the trusted native caller with `SecureStore::create`
    /// under a caller-owned trusted parent. Existing leaves, symlinks and unsafe names are
    /// refused before SCK starts. SCK may create the leaf only after this check; the private
    /// stage and trusted-caller serialization are the assumption that excludes an unrelated
    /// same-user writer from racing that creation.
    ///
    /// `SecureStore::file` accepts only mode 0600. The caller/native setup must establish a
    /// creation mask that yields that mode for the actual SCK writer; the caller's umask alone
    /// is insufficient if a daemon owns file creation. This adapter never changes the process
    /// umask. If SCK creates another mode, observation fails closed.
    pub fn new(stage: Arc<SecureStore>, leaf: &str) -> io::Result<Self> {
        validate_leaf(leaf)?;
        stage.check()?;
        let root = stage.directory_file()?;
        let root_metadata = root.metadata()?;
        if root_metadata.uid() != unsafe { libc::geteuid() }
            || root_metadata.mode() & 0o777 != 0o700
        {
            return Err(invalid());
        }
        let root_identity = FileIdentity::read(&root)?;
        let alias = volume_id_alias(&root_metadata);
        let alias_root = File::open(&alias)?;
        if FileIdentity::read(&alias_root)? != root_identity {
            return Err(invalid());
        }
        drop(alias_root);
        drop(root);

        match stage.file(leaf, false) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "SCK leaf exists",
                ));
            }
            Err(error) => return Err(error),
        }

        Ok(Self {
            store: stage,
            leaf: leaf.to_owned(),
            output_url: alias.join(leaf),
            root_identity,
            state: State::AwaitingNativeLeaf,
            file: None,
            file_identity: None,
        })
    }

    /// URL passed only as the native SCK writer argument, never as a sealed output reference.
    pub fn native_output_url(&self) -> &Path {
        &self.output_url
    }

    /// Pin the single-link 0600 regular file SCK created, or verify an already-pinned inode.
    /// `None` means the writer has not created its leaf yet. A mode or identity mismatch fails
    /// closed; this code does not change permissions or adopt a replacement.
    pub fn observe_created(&mut self) -> io::Result<Option<&File>> {
        if self.state == State::Refused {
            return Err(invalid());
        }
        if let Err(error) = self.check_stage() {
            self.state = State::Refused;
            return Err(error);
        }
        if self.state == State::PinnedNativeLeaf {
            if let Err(error) = self.verify_named_file() {
                self.state = State::Refused;
                return Err(error);
            }
            return self.file.as_ref().map(Some).ok_or_else(invalid);
        }
        let file = match self.store.file(&self.leaf, false) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                self.state = State::Refused;
                return Err(error);
            }
        };
        let identity = match self.validate_native_file(&file) {
            Ok(identity) => identity,
            Err(error) => {
                self.state = State::Refused;
                return Err(error);
            }
        };
        self.file_identity = Some(identity);
        self.file = Some(file);
        self.state = State::PinnedNativeLeaf;
        Ok(self.file.as_ref())
    }

    /// Recheck the leaf pinned during capture after the caller has observed a successful SCK
    /// finish callback. The returned descriptor is the original observed inode; callers keep
    /// this guard alive while validating and reading its bytes. Call only after native finish.
    pub fn verify_finalized(&mut self) -> io::Result<&File> {
        if self.state == State::Refused {
            return Err(invalid());
        }
        if self.state == State::AwaitingNativeLeaf {
            self.state = State::Refused;
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "SCK output was never pinned during capture",
            ));
        }
        if self.state != State::PinnedNativeLeaf {
            return Err(invalid());
        }
        let result = self.verify_finalized_inner();
        if let Err(error) = result {
            self.state = State::Refused;
            return Err(error);
        }
        Ok(self.file.as_ref().expect("pinned output checked above"))
    }

    fn verify_finalized_inner(&self) -> io::Result<()> {
        let pinned = self.file.as_ref().ok_or_else(invalid)?;
        let expected = self.file_identity.ok_or_else(invalid)?;
        if self.validate_native_file(pinned)? != expected {
            return Err(invalid());
        }

        // Sync the pinned output and its named directory entry before returning readback access.
        pinned.sync_all()?;
        self.store.directory_file()?.sync_all()?;
        self.verify_named_file()?;
        self.check_stage()
    }

    fn check_stage(&self) -> io::Result<()> {
        self.store.check()?;
        let metadata = self.store.directory_metadata()?;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o700
            || FileIdentity::from_metadata(&metadata) != self.root_identity
        {
            return Err(invalid());
        }
        Ok(())
    }

    fn verify_named_file(&self) -> io::Result<()> {
        let expected = self.file_identity.ok_or_else(invalid)?;
        let current = self.store.file(&self.leaf, false)?;
        if self.validate_native_file(&current)? != expected {
            return Err(invalid());
        }
        Ok(())
    }

    fn validate_native_file(&self, file: &File) -> io::Result<FileIdentity> {
        self.check_stage()?;
        self.store.validate(file)?;
        let metadata = file.metadata()?;
        if metadata.nlink() != 1 || metadata.dev() != self.root_identity.device {
            return Err(invalid());
        }

        // Confirm the URL handed to SCK resolves to the same regular inode. Nonblocking and
        // no-follow flags make a raced FIFO or symlink harmless to this read-only identity check.
        let alias_file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&self.output_url)?;
        if !alias_file.metadata()?.is_file()
            || FileIdentity::read(&alias_file)? != FileIdentity::read(file)?
        {
            return Err(invalid());
        }
        Ok(FileIdentity::read(file)?)
    }
}

fn validate_leaf(leaf: &str) -> io::Result<()> {
    if leaf.is_empty() || leaf == "." || leaf == ".." || leaf.contains('/') || leaf.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe SCK output leaf",
        ));
    }
    Ok(())
}

fn volume_id_alias(metadata: &std::fs::Metadata) -> PathBuf {
    PathBuf::from(format!("/.vol/{}/{}", metadata.dev(), metadata.ino()))
}

fn invalid() -> io::Error {
    io::Error::other("SCK output identity or private-stage validation failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    struct Fixture {
        root: PathBuf,
        stage_path: PathBuf,
        root_identity: FileIdentity,
        stage_identity: FileIdentity,
        stage: Arc<SecureStore>,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("apparent-sck-output-{}", crate::new_id().unwrap()));
            fs::create_dir(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            let root_identity = FileIdentity::read(&File::open(&root).unwrap()).unwrap();
            let stage_path = root.join("stage");
            let stage = Arc::new(SecureStore::create(&stage_path).unwrap());
            let stage_identity = FileIdentity::read(&stage.directory_file().unwrap()).unwrap();
            Self {
                root,
                stage_path,
                root_identity,
                stage_identity,
                stage,
            }
        }

        fn cleanup(self, leaves: &[(&str, FileIdentity)]) {
            for (leaf, expected) in leaves {
                let path = self.stage_path.join(leaf);
                match fs::symlink_metadata(&path) {
                    Ok(metadata) => {
                        assert_eq!(FileIdentity::from_metadata(&metadata), *expected);
                        // Symlinks are unlinked, never followed.
                        if metadata.file_type().is_symlink() || metadata.is_file() {
                            fs::remove_file(path).unwrap();
                        } else {
                            panic!("unexpected fixture leaf type");
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => panic!("fixture leaf cleanup failed: {error}"),
                }
            }
            assert_eq!(
                FileIdentity::read(&File::open(&self.stage_path).unwrap()).unwrap(),
                self.stage_identity
            );
            drop(self.stage);
            fs::remove_dir(&self.stage_path).unwrap();
            assert_eq!(
                FileIdentity::read(&File::open(&self.root).unwrap()).unwrap(),
                self.root_identity
            );
            fs::remove_dir(&self.root).unwrap();
        }
    }

    #[test]
    fn refuses_existing_symlink_and_unsafe_leaves_without_touching_targets() {
        let fixture = Fixture::new();
        let existing = fixture.stage.file("existing.mp4", true).unwrap();
        let existing_identity = FileIdentity::read(&existing).unwrap();
        drop(existing);
        assert!(SckOutputGuard::new(fixture.stage.clone(), "existing.mp4").is_err());

        let target = fixture.root.join("target");
        let target_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&target)
            .unwrap();
        fs::write(&target, b"leave unchanged").unwrap();
        let target_identity = FileIdentity::read(&target_file).unwrap();
        drop(target_file);
        let link_path = fixture.stage_path.join("linked.mp4");
        symlink(&target, &link_path).unwrap();
        let link_identity = FileIdentity::from_metadata(&fs::symlink_metadata(&link_path).unwrap());
        assert!(SckOutputGuard::new(fixture.stage.clone(), "linked.mp4").is_err());
        assert!(SckOutputGuard::new(fixture.stage.clone(), "../escape.mp4").is_err());
        assert!(SckOutputGuard::new(fixture.stage.clone(), "nested/escape.mp4").is_err());
        assert_eq!(fs::read(&target).unwrap(), b"leave unchanged");
        assert_eq!(
            FileIdentity::from_metadata(&fs::metadata(&target).unwrap()),
            target_identity
        );
        fs::remove_file(&target).unwrap();
        fixture.cleanup(&[
            ("existing.mp4", existing_identity),
            ("linked.mp4", link_identity),
        ]);
    }

    #[test]
    fn pins_store_created_file_and_rejects_leaf_replacement() {
        let fixture = Fixture::new();
        let mut guard = SckOutputGuard::new(fixture.stage.clone(), "movie.mp4").unwrap();
        assert!(guard.observe_created().unwrap().is_none());
        let native_file = fixture.stage.file("movie.mp4", true).unwrap();
        drop(native_file);
        let pinned_identity =
            FileIdentity::read(guard.observe_created().unwrap().unwrap()).unwrap();
        let finalized = guard.verify_finalized().unwrap();
        assert_eq!(FileIdentity::read(finalized).unwrap(), pinned_identity);

        fs::remove_file(fixture.stage_path.join("movie.mp4")).unwrap();
        let replacement = fixture.stage.file("movie.mp4", true).unwrap();
        let replacement_identity = FileIdentity::read(&replacement).unwrap();
        assert_ne!(replacement_identity, pinned_identity);
        assert!(guard.verify_finalized().is_err());
        drop(replacement);
        drop(guard);
        fixture.cleanup(&[("movie.mp4", replacement_identity)]);
    }

    #[test]
    fn stage_ancestry_replacement_permanently_refuses_sealing() {
        let fixture = Fixture::new();
        let mut guard = SckOutputGuard::new(fixture.stage.clone(), "movie.mp4").unwrap();
        let file = fixture.stage.file("movie.mp4", true).unwrap();
        let identity = FileIdentity::read(&file).unwrap();
        drop(file);
        assert!(guard.observe_created().unwrap().is_some());

        fs::rename(&fixture.stage_path, fixture.root.join("stage-moved")).unwrap();
        assert!(guard.verify_finalized().is_err());
        fs::rename(fixture.root.join("stage-moved"), &fixture.stage_path).unwrap();
        assert!(guard.verify_finalized().is_err());
        drop(guard);
        fixture.cleanup(&[("movie.mp4", identity)]);
    }

    #[test]
    fn refuses_to_adopt_leaf_only_after_capture_has_finished() {
        let fixture = Fixture::new();
        let mut guard = SckOutputGuard::new(fixture.stage.clone(), "late.mp4").unwrap();
        let file = fixture.stage.file("late.mp4", true).unwrap();
        let identity = FileIdentity::read(&file).unwrap();
        drop(file);
        assert!(guard.verify_finalized().is_err());
        assert!(guard.observe_created().is_err());
        drop(guard);
        fixture.cleanup(&[("late.mp4", identity)]);
    }

    #[test]
    fn rejects_native_output_mode_without_chmod_or_reopen_fallback() {
        let fixture = Fixture::new();
        let mut guard = SckOutputGuard::new(fixture.stage.clone(), "mode.mp4").unwrap();
        let path = fixture.stage_path.join("mode.mp4");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .open(&path)
            .unwrap();
        drop(file);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(guard.observe_created().is_err());
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o644);
        let rejected_identity = FileIdentity::from_metadata(&fs::metadata(&path).unwrap());
        fs::remove_file(&path).unwrap();
        let replacement = fixture.stage.file("mode.mp4", true).unwrap();
        let replacement_identity = FileIdentity::read(&replacement).unwrap();
        assert_ne!(rejected_identity, replacement_identity);
        assert!(
            guard.observe_created().is_err(),
            "refused guard must not adopt a later file"
        );
        drop(replacement);
        drop(guard);
        fixture.cleanup(&[("mode.mp4", replacement_identity)]);
    }
}
