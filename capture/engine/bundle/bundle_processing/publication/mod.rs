//! The final persistence boundary. No Engine terminal authority lives here.
//! Hold `coordinate` across optional post-build mutations, then call `checked`.
pub(crate) mod benchmark;
mod cleanup;
mod fs;
mod movie;
mod validate;

use crate::shared::model::ApparentBundle;
use anyhow::{Context, Result, ensure};
pub(crate) use fs::Directory;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fs::{File, Metadata},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    rc::{Rc, Weak},
};

pub const IN_PROGRESS: &str = ".publication-pending";
const LOCK: &str = ".publication.lock";
type Built = (PathBuf, ApparentBundle, String);
thread_local! { static HELD: RefCell<HashMap<PathBuf, Weak<Lease>>> = RefCell::new(HashMap::new()); }
thread_local! { static PUBLISHING: RefCell<HashSet<PathBuf>> = RefCell::new(HashSet::new()); }

/// Writer coordination, not capture admission. Reentrant only on the owning thread.
/// The persistent flock inode is never unlinked or replaced.
/// Every guard shares ownership; only the last guard releases coordination.
pub struct Coordination {
    path: PathBuf,
    lease: Rc<Lease>,
}

struct Lease {
    // Closing this descriptor releases flock after the last owning-thread guard.
    _lock: File,
    root: Metadata,
}

pub fn coordinate(path: &Path) -> Result<Coordination> {
    let path = fs::absolute(path)?;
    if let Some(lease) = HELD.with(|held| held.borrow().get(&path).and_then(Weak::upgrade)) {
        check_lease(&path, &lease)?;
        return Ok(Coordination { path, lease });
    }
    let root = Directory::open(&path, true)?;
    let lock = root.leaf(LOCK, true)?;
    // SAFETY: live owned descriptor. flock works on Linux and macOS.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let metadata = root.0.metadata()?;
    ensure!(
        fs::same_directory(&metadata, &Directory::open(&path, false)?.0.metadata()?),
        "bundle replaced while acquiring writer coordination"
    );
    ensure!(
        fs::same(&lock.metadata()?, &root.file(LOCK)?.metadata()?),
        "publication lock replaced"
    );
    let lease = Rc::new(Lease {
        _lock: lock,
        root: metadata,
    });
    HELD.with(|held| {
        held.borrow_mut()
            .insert(path.clone(), Rc::downgrade(&lease))
    });
    Ok(Coordination { path, lease })
}

fn check_lease(path: &Path, lease: &Lease) -> Result<()> {
    let root = Directory::open(path, false)?;
    ensure!(
        fs::same_directory(&lease.root, &root.0.metadata()?),
        "bundle replaced under writer lease"
    );
    ensure!(
        fs::same(&lease._lock.metadata()?, &root.file(LOCK)?.metadata()?),
        "publication lock replaced under writer lease"
    );
    Ok(())
}

impl Drop for Coordination {
    fn drop(&mut self) {
        if Rc::strong_count(&self.lease) == 1 {
            HELD.with(|held| held.borrow_mut().remove(&self.path));
        }
    }
}

/// Exact inputs for the existing Engine FinalizerSuccess constructor, NOT a receipt.
/// Remain under `coordinate` until the caller has consumed these inputs. Later edits
/// invalidate the proof; neither this layer nor Engine freezes all media forever.
pub struct PublicationInputs {
    pub path: PathBuf,
    pub bundle_id: String,
    pub manifest: Vec<u8>,
}

/// Run a production build and ALL optional post-mutations under one writer lease.
/// The existing tuple/result contract is preserved. Only this successful return is
/// eligible for clipboard/upload/Saved by a caller, never an intermediate tuple.
pub fn publish_after(path: &Path, build: impl FnOnce() -> Result<Built>) -> Result<Built> {
    let absolute = fs::absolute(path)?;
    if PUBLISHING.with(|held| held.borrow().contains(&absolute)) {
        return build();
    }
    let guard = coordinate(path)?;
    PUBLISHING.with(|held| held.borrow_mut().insert(absolute.clone()));
    struct Publishing(PathBuf);
    impl Drop for Publishing {
        fn drop(&mut self) {
            PUBLISHING.with(|held| held.borrow_mut().remove(&self.0));
        }
    }
    let publishing = Publishing(absolute);
    let root = Directory::open(path, false)?;
    ensure!(
        fs::same_directory(&guard.lease.root, &root.0.metadata()?),
        "bundle replaced before build"
    );
    root.write(IN_PROGRESS, b"publication has not completed\n")?;
    let result = build().and_then(|built| {
        ensure!(
            fs::absolute(&built.0)? == fs::absolute(path)?,
            "finalizer returned another bundle path"
        );
        checked(path, &built.1, &built.2)?;
        Ok(built)
    });
    if result.is_err() {
        // Preserve pending work and mark any already-written manifest incomplete.
        // Never clear recovery or transcription markers on a failed barrier.
        if let Err(error) = root.write(IN_PROGRESS, b"publication has not completed\n") {
            crate::capture::engine::runtime::apparent_log(format!(
                "Could not preserve publication marker: {error:#}"
            ));
        }
    }
    drop(publishing);
    drop(guard);
    if result
        .as_ref()
        .is_ok_and(|(_, bundle, _)| bundle.transcript.missing.as_ref().is_some_and(|m| m.retry))
    {
        crate::capture::engine::runtime::audio::transcription::speech_model::resume_pending(path);
    }
    result
}

/// Validate exact final bytes, flush every artifact and nested directory, then
/// remove recovery/in-progress markers and flush root and its parent. Fail closed.
pub fn checked(path: &Path, expected: &ApparentBundle, summary: &str) -> Result<PublicationInputs> {
    let guard = coordinate(path)?;
    ensure!(
        fs::same_directory(
            &guard.lease.root,
            &Directory::open(path, false)?.0.metadata()?
        ),
        "bundle replaced before validation"
    );
    checked_with(path, expected, summary, || Ok(()))
}

fn checked_with(
    path: &Path,
    expected: &ApparentBundle,
    summary: &str,
    after_validation: impl FnOnce() -> Result<()>,
) -> Result<PublicationInputs> {
    let guard = coordinate(path)?;
    let root = Directory::open(path, false)?;
    let before = root.0.metadata()?;
    let inventory = validate::Inventory::read(&root)?;
    let manifest = inventory.validate(path, expected, summary)?;
    after_validation()?;
    check_lease(path, &guard.lease)?;
    inventory.sync_and_check(&root)?;
    ensure!(
        fs::same_directory(&before, &Directory::open(path, false)?.0.metadata()?),
        "bundle directory changed during publication"
    );
    let absolute = fs::absolute(path)?;
    let parent = Directory::open_sync_parent(absolute.parent().context("bundle has no parent")?)?;
    cleanup::finish(&root, &parent, || {
        check_lease(path, &guard.lease)?;
        ensure!(
            fs::same_directory(&before, &Directory::open(path, false)?.0.metadata()?),
            "bundle directory changed after durability barrier"
        );
        Ok(())
    })?;
    Ok(PublicationInputs {
        path: absolute,
        bundle_id: expected.id.clone(),
        manifest,
    })
}

/// Checked persistence used by the screen journal and the pending-transcript marker.
pub(crate) fn write(path: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    Directory::open(path, true)?.write(name, bytes)
}

pub(crate) fn read_text(path: &Path, name: &str) -> Result<String> {
    let root = Directory::open(path, false)?;
    Ok(String::from_utf8(fs::bytes(
        &root.file(name)?,
        validate::TEXT_CAP,
    )?)?)
}

/// Check declared input paths before a decoder/transcriber can consume them.
/// Cooperative writers stay under the outer lease; this is not a perpetual path capability.
pub(crate) fn preflight_sources(path: &Path, video: Option<&Path>) -> Result<()> {
    let root = Directory::open(path, false)?;
    if let Some(video) = video {
        ensure!(video.parent() == Some(path), "movie is outside its bundle");
        let leaf = video
            .file_name()
            .and_then(|n| n.to_str())
            .context("invalid movie name")?;
        root.file(leaf)?;
    }
    if root.names()?.iter().any(|name| name == "audio.wav") {
        root.file("audio.wav")?;
    }
    Ok(())
}

/// Write already encoded images without pathname traversal or symlink writes.
pub(crate) fn write_artifacts(
    path: &Path,
    bundle: &ApparentBundle,
    frames: &[(crate::shared::model::Keyframe, crate::shared::model::RgbaFrame)],
    sheet: &crate::shared::model::RgbaFrame,
    summary: &str,
) -> Result<()> {
    let root = Directory::open(path, true)?;
    for (key, frame) in frames {
        root.write(&key.file, &crate::capture::engine::render::encode_png(frame)?)?;
    }
    root.write(&bundle.contact_sheet, &crate::capture::engine::render::encode_png(sheet)?)?;
    root.write("apparent.json", &serde_json::to_vec_pretty(bundle)?)?;
    root.write("summary.md", summary.as_bytes())
}

pub(crate) fn write_full(
    path: &Path,
    info: &crate::shared::model::AutocropInfo,
    frames: &[(crate::shared::model::Keyframe, crate::shared::model::RgbaFrame)],
    sheet: &crate::shared::model::RgbaFrame,
) -> Result<()> {
    let root = Directory::open(path, true)?;
    fs::relative(&info.full_frames)?;
    for (key, frame) in frames {
        let leaf = Path::new(&key.file)
            .file_name()
            .and_then(|p| p.to_str())
            .context("bad full frame path")?;
        root.write(
            &format!("{}/{leaf}", info.full_frames),
            &crate::capture::engine::render::encode_png(frame)?,
        )?;
    }
    root.write(&info.full_contact_sheet, &crate::capture::engine::render::encode_png(sheet)?)
}

#[cfg(test)]
mod tests;
