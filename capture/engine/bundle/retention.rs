//! Keeps the captures folder from growing forever.
//!
//! A bundle's prompt (`summary.md`, `apparent.json`, `contact-sheet.png`, `frames/`) is small; its
//! video, audio and full-size frames are nearly all of its size. After `mediaDays` the heavy
//! media (and the `zoom/` sheets made from it) goes to the Trash and the prompt stays; after
//! `bundleDays` the whole bundle goes. Both live under `"retention"` in `~/.config/apparent/config.json` (`null` keeps forever). A bundle
//! younger than a day or changed within the last day, one whose media is still copying
//! (`.media-pending`, `.media-sending`), one Apparent is working on, a pinned one (`.pinned`), one
//! still waiting to reach OpenCode (`.deliver.json`), and one `apparent recover` hasn't built yet
//! are never touched.
//!
//! Cabled-iPhone videos are also re-encoded to HEVC at the size keyframes use (`shrink`), which
//! makes them about ten times smaller.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::capture::engine::runtime::apparent_log;

/// Marks a bundle to keep forever.
pub const PINNED: &str = ".pinned";
/// Present on this Mac while a bundle's heavy media streams to another one.
pub const MEDIA_SENDING: &str = ".media-sending";
/// Written when the heavy media went to the Trash: when, and after how many days.
pub const MEDIA_CLEANED: &str = ".media-cleaned";

const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// A sync marker older than this belongs to a copy that died.
const STALE_MARKER: Duration = Duration::from_secs(24 * 60 * 60);
/// Long edge iPhone video is re-encoded to: the size its keyframes are decoded at.
const SHRINK_EDGE: u32 = crate::capture::engine::runtime::phone::PHONE_FRAME_EDGE;
/// Videos above this rate are worth re-encoding (native iPhone mirror video is ~18 Mbps; a
/// shrunk one ~2).
const SHRINK_ABOVE_BPS: u64 = 6_000_000;
/// How long after a capture its video is shrunk, so pasting and sending go first.
const SHRINK_DELAY: Duration = Duration::from_secs(60);
/// The daily shrink sweep picks up bundles left alone this long (a capture whose shrink was
/// missed, or one copied from another Mac).
const SHRINK_SETTLE_MS: u64 = 10 * 60 * 1000;
/// How often cleanup runs. Hourly, so media goes within an hour of `mediaDays`, not a day late.
const PASS_EVERY: Duration = Duration::from_secs(3600);
/// How often a pass also re-encodes missed iPhone videos (each try can take a while).
const SHRINK_EVERY: Duration = Duration::from_secs(24 * 3600);
/// Made from the heavy media (`apparent zoom` sheets and frame ranges), so it goes with it.
const DERIVED_MEDIA: [&str; 1] = ["zoom"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    /// Days to keep video, audio and full-size frames; `None` keeps them.
    #[serde(default = "default_media_days")]
    pub media_days: Option<u32>,
    /// Days to keep the bundle itself; `None` keeps it.
    #[serde(default = "default_bundle_days")]
    pub bundle_days: Option<u32>,
}

fn default_media_days() -> Option<u32> {
    Some(7)
}

fn default_bundle_days() -> Option<u32> {
    Some(30)
}

impl Default for Policy {
    fn default() -> Self {
        Preset::Week.policy()
    }
}

/// The choices in the menu bar's Keep Recordings menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Week,
    Month,
    Forever,
}

impl Preset {
    pub const ALL: [Self; 3] = [Self::Week, Self::Month, Self::Forever];

    pub fn label(self) -> &'static str {
        match self {
            Self::Week => "Video for 7 Days",
            Self::Month => "Video for 30 Days",
            Self::Forever => "Forever",
        }
    }

    pub fn policy(self) -> Policy {
        let (media_days, bundle_days) = match self {
            Self::Week => (Some(7), Some(30)),
            Self::Month => (Some(30), Some(90)),
            Self::Forever => (None, None),
        };
        Policy {
            media_days,
            bundle_days,
        }
    }
}

impl Policy {
    pub fn preset(&self) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.policy() == *self)
    }

    pub fn load() -> Self {
        Self::load_from(&crate::shared::handoff::default_config_path())
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|config| config.get("retention").cloned())
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default()
    }

    /// Writes `"retention"`, keeping the rest of the config (and its `0600` mode) as it is.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        crate::shared::handoff::set_config_value_in(path, "retention", serde_json::to_value(self)?)
    }
}

// MARK: What Apparent is busy with

static BUSY: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());
static RECORDING: AtomicUsize = AtomicUsize::new(0);

/// Holds a bundle out of cleanup while Apparent works on it.
pub struct Claim(PathBuf);

/// Claims `dir` unless something else already has.
pub fn claim(dir: &Path) -> Option<Claim> {
    let key = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let inserted = BUSY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.clone());
    // Built after the lock is released: a refused `Claim` would unlock on drop.
    inserted.then(|| Claim(key))
}

fn is_claimed(dir: &Path) -> bool {
    let key = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    BUSY.lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&key)
}

impl Drop for Claim {
    fn drop(&mut self) {
        BUSY.lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

/// Held by a running capture; cleanup and re-encoding wait until none is.
pub struct Recording(());

impl Recording {
    pub fn start() -> Self {
        RECORDING.fetch_add(1, Ordering::AcqRel);
        Self(())
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        RECORDING.fetch_sub(1, Ordering::AcqRel);
    }
}

fn recording() -> bool {
    RECORDING.load(Ordering::Acquire) > 0
}

// MARK: Planning

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    #[serde(default)]
    created_at_ms: Option<u64>,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    video: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    device: Option<ManifestDevice>,
}

#[derive(Debug, Default, Deserialize)]
struct ManifestDevice {
    #[serde(default)]
    capture: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Move these heavy files out of the bundle and into the Trash.
    TrashMedia {
        bundle: PathBuf,
        items: Vec<PathBuf>,
    },
    /// Move the whole bundle to the Trash.
    TrashBundle(PathBuf),
    /// Re-encode the bundle's iPhone video.
    Shrink { bundle: PathBuf, video: PathBuf },
}

fn modified_ago(path: &Path, now: SystemTime) -> Option<Duration> {
    let modified = path.metadata().ok()?.modified().ok()?;
    Some(now.duration_since(modified).unwrap_or_default())
}

/// When anything last changed in the folder itself: an entry added, removed or renamed (a copy
/// arriving, a zoom, a re-encode). Uses the change time, which `tar` can't backdate.
#[cfg(unix)]
fn changed_ago(path: &Path, now: SystemTime) -> Option<Duration> {
    use std::os::unix::fs::MetadataExt;
    let meta = path.metadata().ok()?;
    let changed = UNIX_EPOCH + Duration::new(meta.ctime().max(0) as u64, meta.ctime_nsec() as u32);
    Some(now.duration_since(changed).unwrap_or_default())
}

/// NTFS's change time for the folder (`ChangeTime`), which moves when an entry is added, removed
/// or renamed, like the Unix ctime.
#[cfg(windows)]
fn changed_ago(path: &Path, now: SystemTime) -> Option<Duration> {
    let changed =
        crate::capture::engine::runtime::platform::legacy_windows::platform::shell::change_time(path).or_else(|| path.metadata().ok()?.modified().ok())?;
    Some(now.duration_since(changed).unwrap_or_default())
}

/// A sync marker that is recent enough to mean a copy is still running.
fn live_marker(dir: &Path, name: &str, now: SystemTime) -> bool {
    modified_ago(&dir.join(name), now).is_some_and(|ago| ago < STALE_MARKER)
}

fn created_ms(dir: &Path, manifest: &Manifest) -> Option<u64> {
    manifest.created_at_ms.or_else(|| {
        // `apparent-<ms>-…`
        let name = dir.file_stem()?.to_str()?;
        name.strip_prefix("apparent-")?
            .split('-')
            .next()?
            .parse()
            .ok()
    })
}

/// What one cleanup pass would do to the bundles in `captures`, given `now`.
pub fn plan(captures: &Path, policy: &Policy, now: SystemTime) -> Vec<Action> {
    let now_ms = now
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let Ok(entries) = std::fs::read_dir(captures) else {
        return Vec::new();
    };
    let mut bundles: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| (p.extension().is_some_and(|ext| ext == "apparent")
            || p.file_name().is_some_and(|name| name.to_string_lossy().starts_with(".cancelled-"))) && p.is_dir())
        .collect();
    bundles.sort();
    let mut actions = Vec::new();
    for dir in bundles {
        if dir.join(PINNED).exists()
            || is_claimed(&dir)
            || crate::capture::engine::runtime::audio::persistence::live(&dir)
            || crate::capture::engine::bundle::recover::owed(&dir)
            || live_marker(&dir, crate::shared::handoff::MEDIA_PENDING, now)
            || live_marker(&dir, MEDIA_SENDING, now)
        {
            continue;
        }
        let manifest: Manifest = std::fs::read(dir.join("apparent.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let Some(created) = created_ms(&dir, &manifest).or_else(|| {
            let ago = modified_ago(&dir, now)?;
            Some(now_ms.saturating_sub(ago.as_millis() as u64))
        }) else {
            continue;
        };
        let age_ms = now_ms.saturating_sub(created);
        let unchanged_ms = changed_ago(&dir, now).map_or(0, |ago| ago.as_millis() as u64);
        let settled = |ms: u64| age_ms >= ms && unchanged_ms >= ms;
        if !settled(SHRINK_SETTLE_MS) {
            continue;
        }
        if !settled(DAY_MS) {
            if let Some(video) = shrinkable(&dir, &manifest) {
                actions.push(Action::Shrink { bundle: dir, video });
            }
            continue;
        }
        let due = |days: Option<u32>| days.is_some_and(|d| age_ms >= u64::from(d.max(1)) * DAY_MS);
        if due(policy.bundle_days) {
            actions.push(Action::TrashBundle(dir));
            continue;
        }
        if due(policy.media_days) {
            let items: Vec<PathBuf> = crate::shared::handoff::HEAVY_MEDIA
                .iter()
                .chain(&DERIVED_MEDIA)
                .map(|rel| dir.join(rel))
                .filter(|p| p.exists())
                .collect();
            if !items.is_empty() {
                actions.push(Action::TrashMedia { bundle: dir, items });
            }
            continue;
        }
        if let Some(video) = shrinkable(&dir, &manifest) {
            actions.push(Action::Shrink { bundle: dir, video });
        }
    }
    actions
}

/// A cabled-iPhone video recorded at native size and rate.
fn shrinkable(dir: &Path, manifest: &Manifest) -> Option<PathBuf> {
    if manifest.source.as_deref() != Some("iphone")
        || manifest
            .device
            .as_ref()
            .is_none_or(|d| d.capture != "mirror")
        || manifest.duration_ms == 0
    {
        return None;
    }
    let video = dir.join(manifest.video.as_deref()?);
    let bytes = video.metadata().ok()?.len();
    let bps = bytes.saturating_mul(8_000) / manifest.duration_ms;
    (bps > SHRINK_ABOVE_BPS).then_some(video)
}

// MARK: Doing it

/// Moves `path` to the Trash.
pub type Trasher<'a> = &'a dyn Fn(&Path) -> Result<()>;

#[cfg(windows)]
pub fn trash(path: &Path) -> Result<()> {
    crate::capture::engine::runtime::platform::legacy_windows::platform::shell::recycle(path)?;
    ensure!(
        !path.exists(),
        "Could not move {} to the Recycle Bin",
        path.display()
    );
    Ok(())
}

/// The freedesktop Trash (`~/.local/share/Trash`), as the file manager uses it.
#[cfg(target_os = "linux")]
pub fn trash(path: &Path) -> Result<()> {
    crate::capture::desktop::linux::shell::trash(path)?;
    ensure!(
        !path.exists(),
        "Could not move {} to the Trash",
        path.display()
    );
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn trash(path: &Path) -> Result<()> {
    let output = Command::new("/usr/bin/trash")
        .arg(path)
        .output()
        .context("Could not run /usr/bin/trash")?;
    ensure!(
        output.status.success() && !path.exists(),
        "Could not move {} to the Trash: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

fn size_of(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_dir() {
        std::fs::read_dir(path)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|e| size_of(&e.path()))
                    .sum()
            })
            .unwrap_or(0)
    } else {
        meta.len()
    }
}

/// Everything in the captures folder, in bytes.
pub fn captures_size(captures: &Path) -> u64 {
    size_of(captures)
}

pub fn format_bytes(bytes: u64) -> String {
    const MB: f64 = 1_000_000.0;
    let mb = bytes as f64 / MB;
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else {
        format!("{mb:.0} MB")
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub media_trashed: usize,
    pub bundles_trashed: usize,
    pub freed: u64,
    pub shrunk: usize,
    pub errors: usize,
}

/// Puts a bundle's heavy media in the Trash as one folder named after the bundle, so it reads
/// (and restores) as a unit, then notes the cleanup in the bundle.
fn trash_media(bundle: &Path, items: &[PathBuf], days: u32, trasher: Trasher) -> Result<u64> {
    let name = bundle
        .file_stem()
        .context("Bundle has no name")?
        .to_string_lossy();
    let staging_root = bundle
        .parent()
        .context("Bundle has no parent")?
        .join(".trash-staging");
    let staging = staging_root.join(format!("{name} media"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let mut freed = 0;
    let mut moved = Vec::new();
    for item in items {
        let Some(file) = item.file_name() else {
            continue;
        };
        freed += size_of(item);
        std::fs::rename(item, staging.join(file))?;
        moved.push(file.to_string_lossy().into_owned());
    }
    let note = serde_json::json!({
        "cleanedAtMs": crate::shared::clock::now_epoch_ms(),
        "afterDays": days,
        "trashed": moved,
    });
    std::fs::write(
        bundle.join(MEDIA_CLEANED),
        serde_json::to_vec_pretty(&note)?,
    )?;
    trasher(&staging)?;
    let _ = std::fs::remove_dir(&staging_root);
    Ok(freed)
}

/// Runs a plan. Each step is checked again just before it runs, in case a capture or a copy
/// started meanwhile.
pub fn apply(actions: &[Action], policy: &Policy, trasher: Trasher, shrink: bool) -> Report {
    let mut report = Report::default();
    for action in actions {
        let bundle = match action {
            Action::TrashMedia { bundle, .. }
            | Action::TrashBundle(bundle)
            | Action::Shrink { bundle, .. } => bundle,
        };
        let Some(_claim) = claim(bundle) else {
            continue;
        };
        let now = SystemTime::now();
        if live_marker(bundle, crate::shared::handoff::MEDIA_PENDING, now)
            || live_marker(bundle, MEDIA_SENDING, now)
            || bundle.join(PINNED).exists()
        {
            continue;
        }
        let result = match action {
            Action::TrashMedia { bundle, items } => {
                trash_media(bundle, items, policy.media_days.unwrap_or(0), trasher).map(|freed| {
                    report.media_trashed += 1;
                    report.freed += freed;
                })
            }
            Action::TrashBundle(bundle) => {
                let size = size_of(bundle);
                trasher(bundle).map(|()| {
                    report.bundles_trashed += 1;
                    report.freed += size;
                })
            }
            Action::Shrink { bundle, video } if shrink => {
                if recording() {
                    continue;
                }
                shrink_video(bundle, video).map(|shrunk| {
                    if let Some(s) = shrunk {
                        report.shrunk += 1;
                        report.freed += s.before.saturating_sub(s.after);
                    }
                })
            }
            Action::Shrink { .. } => Ok(()),
        };
        if let Err(error) = result {
            report.errors += 1;
            apparent_log(format!("retention: {}: {error:#}", bundle.display()));
        }
    }
    report
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shrunk {
    pub before: u64,
    pub after: u64,
    pub width: u32,
    pub height: u32,
    pub ms: u64,
}

/// Re-encodes `video` (inside `bundle`) to HEVC at the keyframe size with the same timeline,
/// then swaps it in. The new movie is written beside the bundle, not in it, so a copy to
/// another Mac never picks up half of it. `None` when it was already small.
pub fn shrink_video(bundle: &Path, video: &Path) -> Result<Option<Shrunk>> {
    let probe = crate::capture::desktop::helper::probe(video)?;
    if probe.width.max(probe.height) <= SHRINK_EDGE * 115 / 100 {
        return Ok(None);
    }
    let name = bundle
        .file_stem()
        .context("Bundle has no name")?
        .to_string_lossy();
    let tmp = bundle
        .parent()
        .context("Bundle has no parent")?
        .join(format!(".{name}.shrinking.mov"));
    let result = crate::capture::desktop::helper::shrink(video, &tmp, SHRINK_EDGE);
    let shrunk = match result {
        Ok(s) if s.after > 0 && s.after < s.before => s,
        Ok(s) => {
            let _ = std::fs::remove_file(&tmp);
            apparent_log(format!(
                "shrink: {} kept ({} → {} bytes)",
                video.display(),
                s.before,
                s.after
            ));
            return Ok(None);
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    // A sync that started while encoding streams the original; let it finish with that.
    let now = SystemTime::now();
    if live_marker(bundle, MEDIA_SENDING, now)
        || live_marker(bundle, crate::shared::handoff::MEDIA_PENDING, now)
    {
        let _ = std::fs::remove_file(&tmp);
        anyhow::bail!("its media started copying while it was being shrunk");
    }
    std::fs::rename(&tmp, video)?;
    apparent_log(format!(
        "shrink: {} {} → {} ({}×{}, {:.1} s)",
        video.display(),
        format_bytes(shrunk.before),
        format_bytes(shrunk.after),
        shrunk.width,
        shrunk.height,
        shrunk.ms as f64 / 1000.0
    ));
    Ok(Some(shrunk))
}

/// Removes what a crash mid-cleanup or mid-shrink left beside the bundles.
fn sweep_leftovers(captures: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(captures) else {
        return;
    };
    for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let leftover = name.ends_with(".shrinking.mov") || name == ".trash-staging";
        if leftover
            && modified_ago(&path, now).is_some_and(|ago| ago > Duration::from_secs(6 * 3600))
        {
            let _ = std::fs::remove_dir_all(&path).or_else(|_| std::fs::remove_file(&path));
        }
    }
}

/// One cleanup pass over `captures`, planned as if `ahead` from now (for trying it out).
/// Without `shrink` it only trashes, so a video that doesn't get smaller isn't re-encoded hourly.
pub fn run_pass(
    captures: &Path,
    policy: &Policy,
    dry_run: bool,
    ahead: Duration,
    shrink: bool,
) -> (Vec<Action>, Report) {
    let now = SystemTime::now();
    let mut actions = plan(captures, policy, now + ahead);
    if !shrink {
        actions.retain(|a| !matches!(a, Action::Shrink { .. }));
    }
    if dry_run {
        return (actions, Report::default());
    }
    sweep_leftovers(captures, now);
    let report = apply(&actions, policy, &trash, true);
    if report != Report::default() {
        apparent_log(format!(
            "retention: media of {} bundle(s) and {} whole bundle(s) to the Trash, {} video(s) shrunk, {} freed, {} error(s)",
            report.media_trashed,
            report.bundles_trashed,
            report.shrunk,
            format_bytes(report.freed),
            report.errors
        ));
    }
    (actions, report)
}

/// Why a bundle's video is gone, for errors that need it.
pub fn missing_media_reason(dir: &Path) -> Option<String> {
    let note: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join(MEDIA_CLEANED)).ok()?).ok()?;
    let days = note
        .get("afterDays")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    Some(if days > 0 {
        format!(
            "The bundle's video was cleaned up after {days} days and is in the Trash \
             (Apparent menu › Keep Recordings sets how long video is kept)"
        )
    } else {
        "The bundle's video was cleaned up and is in the Trash".to_owned()
    })
}

// MARK: Background worker

enum Job {
    Pass,
    Shrink(PathBuf),
}

static WORKER: std::sync::OnceLock<flume::Sender<Job>> = std::sync::OnceLock::new();

fn lower_priority() {
    // SAFETY: only changes the calling thread's scheduling class.
    #[cfg(target_os = "macos")]
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
    #[cfg(windows)]
    crate::capture::engine::runtime::platform::legacy_windows::platform::shell::background_thread();
    // SAFETY: only lowers the calling thread's own priority.
    #[cfg(target_os = "linux")]
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t, 10);
    }
}

/// Starts the app's cleanup: a pass shortly after launch, then hourly, plus shrinking each
/// iPhone capture a minute after it's made.
pub fn start() {
    if WORKER.get().is_some() {
        return;
    }
    let (sender, receiver) = flume::unbounded();
    if WORKER.set(sender).is_err() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("apparent-retention".into())
        .spawn(move || worker(&receiver));
}

/// Runs a pass soon, after the policy changed.
pub fn nudge() {
    if let Some(worker) = WORKER.get() {
        let _ = worker.send(Job::Pass);
    }
}

/// Queues a just-finished iPhone capture for shrinking (only in the app).
pub fn after_phone_capture(bundle: &Path) {
    if let Some(worker) = WORKER.get() {
        let _ = worker.send(Job::Shrink(bundle.to_path_buf()));
    }
}

fn worker(jobs: &flume::Receiver<Job>) {
    lower_priority();
    let captures = crate::shared::handoff::default_captures_dir();
    let mut next_pass = Instant::now() + Duration::from_secs(90);
    let mut last_pass: Option<SystemTime> = None;
    let mut last_shrink: Option<SystemTime> = None;
    let mut shrinks: Vec<(Instant, Instant, PathBuf)> = Vec::new();
    loop {
        let now = Instant::now();
        let wake = shrinks
            .iter()
            .map(|(due, _, _)| *due)
            .chain([next_pass])
            .min()
            .unwrap_or(next_pass);
        match jobs.recv_timeout(
            wake.saturating_duration_since(now)
                .min(Duration::from_secs(3600)),
        ) {
            Ok(Job::Pass) => next_pass = Instant::now() + Duration::from_secs(5),
            Ok(Job::Shrink(dir)) => {
                let due = Instant::now() + SHRINK_DELAY;
                shrinks.push((due, due + Duration::from_secs(2 * 3600), dir));
            }
            Err(flume::RecvTimeoutError::Timeout) => {}
            Err(flume::RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        let mut later = Vec::new();
        for (due, give_up, dir) in shrinks.drain(..) {
            if due > now {
                later.push((due, give_up, dir));
                continue;
            }
            match shrink_now(&dir) {
                // Busy: a capture is running or the media is still copying. Try again soon; a
                // later pass catches anything that gives up.
                Err(()) if now < give_up => {
                    later.push((now + Duration::from_secs(30), give_up, dir))
                }
                _ => {}
            }
        }
        shrinks = later;
        // The Mac sleeps through `Instant`s, so passes also go by the wall clock.
        let pass_due = last_pass.is_none_or(|at| {
            SystemTime::now().duration_since(at).unwrap_or_default() >= PASS_EVERY
        });
        if Instant::now() >= next_pass || (last_pass.is_some() && pass_due) {
            if recording() {
                next_pass = Instant::now() + Duration::from_secs(60);
                continue;
            }
            let policy = Policy::load();
            let shrink = last_shrink.is_none_or(|at| {
                SystemTime::now().duration_since(at).unwrap_or_default() >= SHRINK_EVERY
            });
            let (actions, report) = run_pass(&captures, &policy, false, Duration::ZERO, shrink);
            if shrink {
                last_shrink = Some(SystemTime::now());
            }
            // Hourly passes that find nothing stay out of the log after the first.
            if last_pass.is_none() || !actions.is_empty() {
                apparent_log(format!(
                    "retention: pass over {} ({} action(s), policy {:?}/{:?} days), {} now",
                    captures.display(),
                    actions.len(),
                    policy.media_days,
                    policy.bundle_days,
                    format_bytes(captures_size(&captures)),
                ));
            }
            let _ = report;
            last_pass = Some(SystemTime::now());
            next_pass = Instant::now() + PASS_EVERY;
        }
    }
}

/// `Err` when the bundle is busy and should be retried.
fn shrink_now(dir: &Path) -> std::result::Result<(), ()> {
    let now = SystemTime::now();
    if recording()
        || live_marker(dir, crate::shared::handoff::MEDIA_PENDING, now)
        || live_marker(dir, MEDIA_SENDING, now)
    {
        return Err(());
    }
    let Some(_claim) = claim(dir) else {
        return Err(());
    };
    let manifest: Manifest = match std::fs::read(dir.join("apparent.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    {
        Some(m) => m,
        None => return Ok(()),
    };
    if dir.join(PINNED).exists() {
        return Ok(());
    }
    if let Some(video) = shrinkable(dir, &manifest)
        && let Err(error) = shrink_video(dir, &video)
    {
        apparent_log(format!("shrink: {}: {error:#}", dir.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "apparent-retention-{tag}-{}-{}",
                std::process::id(),
                crate::shared::ids::make_bundle_id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Plans run a little over a day ahead of the real clock, so folders made by the test
    /// count as unchanged for a day (their change time can't be backdated).
    fn later() -> SystemTime {
        SystemTime::now() + Duration::from_secs(25 * 3600)
    }

    fn ms(time: SystemTime) -> u64 {
        time.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
    }

    /// A bundle created `days` before `later()`.
    fn bundle(captures: &Path, name: &str, days: f64, iphone: bool) -> PathBuf {
        let dir = captures.join(format!("{name}.apparent"));
        std::fs::create_dir_all(dir.join("frames")).unwrap();
        let created = ms(later()) - (days * DAY_MS as f64) as u64;
        let video = if iphone {
            "recording.mov"
        } else {
            "recording.mp4"
        };
        let mut manifest = serde_json::json!({
            "version": 1,
            "id": name,
            "createdAtMs": created,
            "durationMs": 10_000,
            "video": video,
        });
        if iphone {
            manifest["source"] = "iphone".into();
            manifest["device"] = serde_json::json!({"name": "iPhone", "capture": "mirror", "width": 1320, "height": 2868});
        }
        std::fs::write(dir.join("apparent.json"), manifest.to_string()).unwrap();
        std::fs::write(dir.join("summary.md"), "summary").unwrap();
        std::fs::write(dir.join("contact-sheet.png"), "png").unwrap();
        std::fs::write(dir.join("frames/frame-01.png"), "png").unwrap();
        // 10 s at 20 Mbps, like a native iPhone mirror recording.
        std::fs::write(
            dir.join(video),
            vec![0u8; if iphone { 25_000_000 } else { 1000 }],
        )
        .unwrap();
        std::fs::write(dir.join("audio.wav"), vec![0u8; 100]).unwrap();
        dir
    }

    /// Marks `name` in `dir` as written at `at`.
    fn marker(dir: &Path, name: &str, at: SystemTime) {
        let file = std::fs::File::create(dir.join(name)).unwrap();
        file.set_modified(at).unwrap();
    }

    fn planned(captures: &Path, policy: &Policy) -> Vec<Action> {
        plan(captures, policy, later())
    }

    #[test]
    fn policy_reads_presets_nulls_and_keeps_other_keys() {
        let tmp = TempDir::new("config");
        let path = tmp.0.join("config.json");
        assert_eq!(Policy::load_from(&path), Preset::Week.policy());
        std::fs::write(
            &path,
            r#"{"ingestToken":"abc","agentHost":"studio","retention":{"mediaDays":null}}"#,
        )
        .unwrap();
        let loaded = Policy::load_from(&path);
        assert_eq!(
            loaded,
            Policy {
                media_days: None,
                bundle_days: Some(30)
            }
        );
        assert_eq!(loaded.preset(), None);
        Preset::Forever.policy().save_to(&path).unwrap();
        assert_eq!(Policy::load_from(&path).preset(), Some(Preset::Forever));
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(config["ingestToken"], "abc");
        assert_eq!(config["agentHost"], "studio");
        assert_eq!(config["retention"]["mediaDays"], serde_json::Value::Null);
        Preset::Month.policy().save_to(&path).unwrap();
        assert_eq!(
            Policy::load_from(&path),
            Policy {
                media_days: Some(30),
                bundle_days: Some(90)
            }
        );
    }

    #[test]
    fn old_media_and_older_bundles_are_planned_for_the_trash() {
        let tmp = TempDir::new("plan");
        bundle(&tmp.0, "apparent-fresh", 0.5, false);
        bundle(&tmp.0, "apparent-week", 3.0, false);
        let old = bundle(&tmp.0, "apparent-old", 8.0, false);
        std::fs::create_dir_all(old.join("frames-full")).unwrap();
        std::fs::create_dir_all(old.join("zoom")).unwrap();
        let ancient = bundle(&tmp.0, "apparent-ancient", 31.0, false);
        assert_eq!(
            planned(&tmp.0, &Policy::default()),
            vec![
                Action::TrashBundle(ancient),
                Action::TrashMedia {
                    bundle: old.clone(),
                    items: vec![
                        old.join("recording.mp4"),
                        old.join("audio.wav"),
                        old.join("frames-full"),
                        old.join("zoom")
                    ],
                },
            ]
        );
        assert!(planned(&tmp.0, &Preset::Forever.policy()).is_empty());
        // However short the policy, a day-old bundle is kept.
        let short = Policy {
            media_days: Some(0),
            bundle_days: None,
        };
        assert_eq!(planned(&tmp.0, &short).len(), 3);
    }

    #[test]
    fn pinned_syncing_and_busy_bundles_are_left_alone() {
        let tmp = TempDir::new("guards");
        let pinned = bundle(&tmp.0, "apparent-pinned", 40.0, false);
        std::fs::write(pinned.join(PINNED), "").unwrap();
        let pending = bundle(&tmp.0, "apparent-pending", 40.0, false);
        marker(
            &pending,
            crate::shared::handoff::MEDIA_PENDING,
            later() - Duration::from_secs(3600),
        );
        let sending = bundle(&tmp.0, "apparent-sending", 40.0, false);
        marker(&sending, MEDIA_SENDING, later() - Duration::from_secs(3600));
        let busy = bundle(&tmp.0, "apparent-busy", 40.0, false);
        let held = claim(&busy).unwrap();
        assert!(claim(&busy).is_none(), "one claim at a time");
        assert!(planned(&tmp.0, &Policy::default()).is_empty());
        drop(held);
        assert_eq!(
            planned(&tmp.0, &Policy::default()),
            vec![Action::TrashBundle(busy)]
        );
        // A marker left by a copy that died over a day ago no longer holds a bundle.
        marker(
            &pending,
            crate::shared::handoff::MEDIA_PENDING,
            later() - STALE_MARKER - Duration::from_secs(60),
        );
        assert!(planned(&tmp.0, &Policy::default()).contains(&Action::TrashBundle(pending)));
    }

    #[test]
    fn recently_changed_folders_wait_a_day() {
        let tmp = TempDir::new("changed");
        // Just copied from another Mac: made long ago, but the folder changed now.
        let arrived = bundle(&tmp.0, "apparent-arrived", 40.0, false);
        let today = later() - Duration::from_secs(24 * 3600);
        assert!(plan(&tmp.0, &Policy::default(), today).is_empty());
        assert_eq!(
            planned(&tmp.0, &Policy::default()),
            vec![Action::TrashBundle(arrived)]
        );
    }

    #[test]
    fn applying_moves_media_as_one_folder_and_keeps_the_prompt() {
        let tmp = TempDir::new("apply");
        let captures = tmp.0.join("captures");
        let trash_dir = tmp.0.join("Trash");
        std::fs::create_dir_all(&trash_dir).unwrap();
        let old = bundle(&captures, "apparent-old", 8.0, true);
        let ancient = bundle(&captures, "apparent-ancient", 31.0, false);
        let fake_trash = |path: &Path| -> Result<()> {
            std::fs::rename(path, trash_dir.join(path.file_name().unwrap()))?;
            Ok(())
        };
        let actions = planned(&captures, &Policy::default());
        assert_eq!(actions.len(), 2);
        let report = apply(&actions, &Policy::default(), &fake_trash, false);
        assert_eq!(
            (report.media_trashed, report.bundles_trashed, report.errors),
            (1, 1, 0)
        );
        assert!(report.freed > 25_000_000);
        assert!(!ancient.exists());
        assert!(
            trash_dir
                .join("apparent-ancient.apparent/apparent.json")
                .is_file()
        );
        let media = trash_dir.join("apparent-old media");
        assert!(media.join("recording.mov").is_file() && media.join("audio.wav").is_file());
        for kept in [
            "apparent.json",
            "summary.md",
            "contact-sheet.png",
            "frames/frame-01.png",
        ] {
            assert!(old.join(kept).is_file(), "{kept} kept");
        }
        assert!(!old.join("recording.mov").exists());
        assert!(!captures.join(".trash-staging").exists());
        let reason = missing_media_reason(&old).unwrap();
        assert!(reason.contains("after 7 days"), "{reason}");
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(old.join("apparent.json")).unwrap()).unwrap();
        assert_eq!(
            manifest["video"], "recording.mov",
            "the manifest is left as it was"
        );
        // Nothing left to clean, and the bundle changed today besides.
        let next_week = later() + Duration::from_secs(2 * 24 * 3600);
        assert!(plan(&captures, &Policy::default(), next_week).is_empty());
    }

    #[test]
    fn only_native_iphone_video_is_shrunk() {
        let tmp = TempDir::new("shrink");
        let phone = bundle(&tmp.0, "apparent-phone", 2.0, true);
        bundle(&tmp.0, "apparent-mac", 2.0, false);
        let small = bundle(&tmp.0, "apparent-small", 2.0, true);
        std::fs::write(small.join("recording.mov"), vec![0u8; 2_500_000]).unwrap();
        assert_eq!(
            planned(&tmp.0, &Policy::default()),
            vec![Action::Shrink {
                bundle: phone.clone(),
                video: phone.join("recording.mov")
            }]
        );
        // Hours old is enough to shrink; minutes old is left to the capture's own shrink.
        let hours = bundle(&tmp.0, "apparent-hours", 0.1, true);
        bundle(&tmp.0, "apparent-minutes", 0.003, true);
        let shrinks: Vec<PathBuf> = planned(&tmp.0, &Policy::default())
            .into_iter()
            .filter_map(|a| match a {
                Action::Shrink { bundle, .. } => Some(bundle),
                _ => None,
            })
            .collect();
        assert_eq!(shrinks, vec![hours, phone.clone()]);
        // Due for cleanup instead: not worth re-encoding.
        let short = Policy {
            media_days: Some(1),
            bundle_days: None,
        };
        assert!(
            planned(&tmp.0, &short)
                .iter()
                .all(|a| !matches!(a, Action::Shrink { bundle, .. } if *bundle == phone))
        );
    }

    #[test]
    fn sizes_read_plainly() {
        assert_eq!(format_bytes(72_247_194), "72 MB");
        assert_eq!(format_bytes(2_512_000_000), "2.5 GB");
    }
}
