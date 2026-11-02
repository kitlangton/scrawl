use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::shared::model::ApparentBundle;

mod config_store;
pub(crate) mod capture_location;
pub use config_store::{default_config_path, set_config_value_in};
pub(crate) use config_store::update_config_in;
#[cfg(test)]
pub(crate) use config_store::set_config_write_hook;

/// Where captures go: `APPARENT_CAPTURES_DIR`, else `"capturesDir"` in `config.json` (Settings ›
/// Storage), else `builtin_captures_dir`. Tests never see the config file.
pub fn default_captures_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("APPARENT_CAPTURES_DIR") {
        return PathBuf::from(dir);
    }
    let configured = if cfg!(test) {
        None
    } else {
        config().captures_dir
    };
    resolve_captures_dir(configured.as_deref(), std::env::var_os("APPARENT_CAPTURES_DIR").map(PathBuf::from)).0
}

fn resolve_captures_dir(configured: Option<&str>, environment: Option<PathBuf>) -> (PathBuf, &'static str) {
    if let Some(dir) = environment { return (dir, "environment"); }
    if let Some(dir) = configured.and_then(configured_captures_dir) {
        if usable_captures_dir(&dir) { return (dir, "configured"); }
        return (builtin_captures_dir(), "fallback");
    }
    (builtin_captures_dir(), "builtin")
}

/// A chosen folder that exists (or can be made) and takes writes; otherwise it's logged once and the
/// built-in folder is used, so a capture is never lost to a missing or unplugged drive.
fn usable_captures_dir(dir: &Path) -> bool {
    let usable = fs::create_dir_all(dir).is_ok()
        && fs::metadata(dir).is_ok_and(|meta| meta.is_dir() && !meta.permissions().readonly());
    if !usable {
        log_config_once(format!(
            "config.json: \"capturesDir\" {} can't be used (missing or not writable); captures go to {}",
            dir.display(),
            builtin_captures_dir().display()
        ));
    }
    usable
}

/// A `"capturesDir"` value as a folder: `~` is the home folder, and only an absolute path counts
/// (anything else is logged and skipped, so captures never land relative to the app's cwd).
pub fn configured_captures_dir(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let path = match value.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            dirs::home_dir()?.join(rest.trim_start_matches(['/', '\\']))
        }
        _ => PathBuf::from(value),
    };
    if path.is_absolute() {
        Some(path)
    } else {
        log_config_once(format!(
            "config.json: ignoring \"capturesDir\": {value} is not an absolute path"
        ));
        None
    }
}

/// `~/Library/Application Support/apparent/captures` on macOS, `%LOCALAPPDATA%\apparent\captures`
/// on Windows (local, not roaming: the movies are large), `~/.local/share/apparent/captures` on
/// Linux.
pub fn builtin_captures_dir() -> PathBuf {
    if cfg!(not(target_os = "macos")) {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("apparent")
            .join("captures")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library/Application Support/apparent/captures")
    }
}

/// Opens a file, folder or URL with its default app (Finder, Explorer, the browser).
pub fn open_path(target: &Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        crate::capture::engine::runtime::platform::legacy_windows::platform::shell::open(target)
    }
    #[cfg(target_os = "linux")]
    {
        crate::capture::desktop::linux::shell::open(target)
    }
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("/usr/bin/open").arg(target).status()?;
        ensure!(status.success(), "open {} failed", target.display());
        Ok(())
    }
}

/// Shows a file selected in Finder, Explorer or the Linux file manager.
pub fn reveal_path(target: &Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        crate::capture::engine::runtime::platform::legacy_windows::platform::shell::reveal(target)
    }
    #[cfg(target_os = "linux")]
    {
        crate::capture::desktop::linux::shell::reveal(target)
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("/usr/bin/open")
            .arg("-R")
            .arg(target)
            .spawn()?;
        Ok(())
    }
}

pub fn resolve_bundle_dir(path_or_id: &Path) -> Result<PathBuf> {
    if path_or_id == Path::new("latest") {
        return newest_bundle(&default_captures_dir());
    }
    let mut path = path_or_id.to_owned();
    if path.file_name().is_some_and(|name| name == "apparent.json") {
        path = path
            .parent()
            .context("Missing bundle directory")?
            .to_owned();
        if path.as_os_str().is_empty() {
            path = PathBuf::from(".");
        }
    }
    if !path.exists() && path.components().count() == 1 {
        path = default_captures_dir().join(&path);
        if path.extension().is_none() {
            path.set_extension("apparent");
        }
    }
    ensure!(
        path.is_dir(),
        "Bundle directory not found: {}",
        path.display()
    );
    let path = plain_path(
        path.canonicalize()
            .with_context(|| format!("Could not resolve {}", path.display()))?,
    );
    ensure!(
        path.extension().is_some_and(|ext| ext == "apparent"),
        "Expected a .apparent directory: {}",
        path.display()
    );
    Ok(path)
}

fn newest_bundle(captures: &Path) -> Result<PathBuf> {
    capture_history(captures)?
        .into_iter()
        .next()
        .context("No .apparent captures found")
}

/// Completed captures in reverse creation order. Do not use directory mtime: reprocessing and
/// zooming an old capture can change it without making that capture the newest one.
pub fn capture_history(captures: &Path) -> Result<Vec<PathBuf>> {
    let mut candidates = Vec::new();
    for entry in
        fs::read_dir(captures).with_context(|| format!("Could not read {}", captures.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "apparent") && path.is_dir() {
            // A cancelled or still-processing capture has a directory but no manifest.
            // Never let it hide the last completed capture from paste/inspect latest.
            let Some(bundle) = fs::read(path.join("apparent.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<ApparentBundle>(&bytes).ok())
            else {
                continue;
            };
            // Bundle creation time is stable even after later zoom-frame extraction.
            candidates.push((bundle.created_at_ms, path));
        }
    }
    candidates.sort_unstable_by(|a, b| b.cmp(a));
    Ok(candidates
        .into_iter()
        .filter_map(|(_, path)| path.canonicalize().ok().map(plain_path))
        .collect())
}

/// `canonicalize` on Windows returns `\\?\C:\…`; agents and terminals want `C:\…`.
fn plain_path(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

pub fn load_bundle(bundle_dir: &Path) -> Result<(PathBuf, ApparentBundle)> {
    let directory = resolve_bundle_dir(bundle_dir)?;
    let path = directory.join("apparent.json");
    let bundle = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("Could not read {}", path.display()))?,
    )
    .with_context(|| format!("Invalid bundle manifest: {}", path.display()))?;
    Ok((directory, bundle))
}

#[cfg(target_os = "macos")]
pub fn copy_string_to_clipboard(content: &str) -> Result<()> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::{NSArray, NSString};

    let text = NSString::from_str(content);
    let pasteboard = NSPasteboard::generalPasteboard();
    unsafe {
        let types = NSArray::from_slice(&[NSPasteboardTypeString]);
        pasteboard.clearContents();
        pasteboard.declareTypes_owner(&types, None);
        ensure!(
            pasteboard.setString_forType(&text, NSPasteboardTypeString),
            "Could not copy text to clipboard"
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn copy_png_to_clipboard(png_bytes: &[u8]) -> Result<()> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypePNG};
    use objc2_foundation::{NSArray, NSData};

    let data = NSData::with_bytes(png_bytes);
    let pasteboard = NSPasteboard::generalPasteboard();
    unsafe {
        let types = NSArray::from_slice(&[NSPasteboardTypePNG]);
        pasteboard.clearContents();
        pasteboard.declareTypes_owner(&types, None);
        ensure!(
            pasteboard.setData_forType(Some(&data), NSPasteboardTypePNG),
            "Could not copy PNG image to clipboard"
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn copy_png_and_text_to_clipboard(png_bytes: &[u8], text: &str) -> Result<bool> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeString};
    use objc2_foundation::{NSArray, NSData, NSString};

    let pasteboard = NSPasteboard::generalPasteboard();
    let image = NSData::with_bytes(png_bytes);
    let text = NSString::from_str(text);
    unsafe {
        let types = NSArray::from_slice(&[NSPasteboardTypePNG, NSPasteboardTypeString]);
        pasteboard.clearContents();
        pasteboard.declareTypes_owner(&types, None);
        ensure!(
            pasteboard.setData_forType(Some(&image), NSPasteboardTypePNG)
                && pasteboard.setString_forType(&text, NSPasteboardTypeString),
            "Could not copy image and text to clipboard"
        );
    }
    Ok(true)
}

#[cfg(target_os = "windows")]
fn copy_png_and_text_to_clipboard(png_bytes: &[u8], text: &str) -> Result<bool> {
    crate::capture::engine::runtime::platform::legacy_windows::platform::clipboard::set_png_and_text(png_bytes, Some(text))?;
    Ok(true)
}

#[cfg(target_os = "linux")]
fn copy_png_and_text_to_clipboard(png_bytes: &[u8], text: &str) -> Result<bool> {
    if !crate::capture::desktop::linux::clipboard::combined_offer_supported() {
        return Ok(false);
    }
    crate::capture::desktop::linux::clipboard::set_png_and_text(png_bytes, text)?;
    Ok(true)
}

#[cfg(target_os = "windows")]
pub fn copy_string_to_clipboard(content: &str) -> Result<()> {
    crate::capture::engine::runtime::platform::legacy_windows::platform::clipboard::set_text(content)
}

#[cfg(target_os = "windows")]
pub fn copy_png_to_clipboard(png_bytes: &[u8]) -> Result<()> {
    crate::capture::engine::runtime::platform::legacy_windows::platform::clipboard::set_png(png_bytes)
}

#[cfg(target_os = "linux")]
pub fn copy_string_to_clipboard(content: &str) -> Result<()> {
    crate::capture::desktop::linux::clipboard::set_text(content)
}

#[cfg(target_os = "linux")]
pub fn copy_png_to_clipboard(png_bytes: &[u8]) -> Result<()> {
    crate::capture::desktop::linux::clipboard::set_png(png_bytes)
}

pub fn copy_bundle_to_clipboard(bundle_dir: &Path) -> Result<()> {
    let (directory, bundle) = load_bundle(bundle_dir)?;
    let summary = clipboard_summary(&bundle, &directory);
    offer_image_handoff(
        experiment("clipboardImage"),
        || fs::read(directory.join(&bundle.contact_sheet)).map_err(Into::into),
        &summary,
        copy_png_and_text_to_clipboard,
        copy_string_to_clipboard,
    )
}

#[cfg(target_os = "linux")]
pub fn copy_bundle_text_to_clipboard(bundle_dir: &Path) -> Result<()> {
    let (directory, bundle) = load_bundle(bundle_dir)?;
    copy_string_to_clipboard(&clipboard_summary(&bundle, &directory))
}

// Only fall back if no image was offered. An error after publication may mean another
// app took the selection; replacing it with our summary would steal the user's copy.
fn offer_image_handoff(
    enabled: bool,
    image: impl FnOnce() -> Result<Vec<u8>>,
    summary: &str,
    offer_both: impl FnOnce(&[u8], &str) -> Result<bool>,
    offer_text: impl FnOnce(&str) -> Result<()>,
) -> Result<()> {
    if enabled && let Ok(image) = image() {
        if offer_both(&image, summary)? {
            return Ok(());
        }
    }
    offer_text(summary)
}

const AGENT_DEMO_VIDEO: &str = "agentDemoVideo";
const DEMO_VIDEO_REQUEST: &str = "Use make-it-apparent for complex visible changes. Share verified video proof or explain why you can't.";

/// The request belongs to copy/paste, not the archived summary or automatic session message.
/// Read config on each handoff so file edits take effect without a restart.
pub(crate) fn clipboard_summary(bundle: &ApparentBundle, directory: &Path) -> String {
    let mut summary = clipboard_summary_with_demo_video(bundle, directory, experiment(AGENT_DEMO_VIDEO));
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if let Some(pointer) = crate::connect::hosted::handoff_pointer(directory) {
        summary.push_str(&pointer);
    }
    summary
}

fn clipboard_summary_with_demo_video(
    bundle: &ApparentBundle,
    directory: &Path,
    request_video: bool,
) -> String {
    with_demo_video_request(
        crate::capture::engine::sketch::summary_markdown(bundle, directory),
        request_video,
    )
}

fn with_demo_video_request(mut summary: String, request_video: bool) -> String {
    if request_video {
        summary.push('\n');
        summary.push_str(DEMO_VIDEO_REQUEST);
        summary.push('\n');
    }
    summary
}

/// Pastes both the contact-sheet image and the transcript/keyframe text into the active app
/// as a two-step sequence (`⌘V` image, then `⌘V` text; Ctrl+V on Windows and Linux, Ctrl+Shift+V
/// in a Linux terminal). If `click_target` is
/// provided (from dragging the Paste chip onto a window/input), clicks at that screen coordinate
/// first to focus it.
pub fn paste_bundle_at_target(bundle_dir: &Path, click_target: Option<[f32; 2]>) -> Result<()> {
    if let Some([x, y]) = click_target {
        crate::capture::engine::runtime::post_synthetic_click(f64::from(x), f64::from(y));
        std::thread::sleep(std::time::Duration::from_millis(65));
    } else {
        crate::capture::engine::runtime::ensure_external_app_active();
    }
    paste_bundle_image_and_text(bundle_dir)
}

pub fn paste_bundle_image_and_text(bundle_dir: &Path) -> Result<()> {
    let (directory, bundle) = load_bundle(bundle_dir)?;
    let sheet_path = directory.join(&bundle.contact_sheet);
    let image = fs::read(&sheet_path).context("Could not read contact sheet")?;
    let summary = clipboard_summary(&bundle, &directory);

    crate::capture::engine::runtime::wait_for_modifiers_released(350);

    #[cfg(target_os = "linux")]
    if crate::capture::desktop::linux::hyprland::running() && crate::capture::desktop::linux::wayland_session() {
        if !crate::capture::engine::runtime::is_frontmost_terminal() {
            // On Hyprland the targeted shortcut works without the RemoteDesktop portal.
            // Use it for any focused GUI composer, not only one named application.
            return crate::capture::desktop::linux::auto_paste::paste_bundle(bundle_dir);
        }
    }

    #[cfg(target_os = "linux")]
    if experiment("draftHandoff")
        && crate::capture::desktop::linux::hyprland::running()
        && crate::capture::desktop::linux::wayland_session()
    {
        return run_draft_handoff(
            &image,
            &summary,
            &sheet_path,
            crate::capture::engine::runtime::is_frontmost_terminal(),
            &mut LinuxDraftHandoff,
        );
    }

    if crate::capture::engine::runtime::is_frontmost_terminal() {
        // In terminals (Ghostty / OpenCode TUI / Claude Code), pasting the image path first
        // resolves and attaches `[Image 1]`, and the second paste inserts the text summary.
        copy_string_to_clipboard(&sheet_path.display().to_string())?;
    } else {
        // In GUI apps (OpenCode Desktop, Slack, Linear, browsers), PNG-only attaches the image.
        copy_png_to_clipboard(&image)?;
    }
    std::thread::sleep(std::time::Duration::from_millis(20));
    paste_synthetic_checked()?;

    std::thread::sleep(std::time::Duration::from_millis(160));
    copy_string_to_clipboard(&summary)?;
    std::thread::sleep(std::time::Duration::from_millis(20));
    paste_synthetic_checked()?;
    Ok(())
}

/// Automatic delivery never activates another app, clicks a target, or submits the draft.
#[cfg(target_os = "linux")]
pub(crate) fn paste_bundle_into_original_draft(
    bundle_dir: &Path,
    original: &crate::capture::desktop::linux::hyprland::FocusedWindow,
) -> Result<()> {
    ensure!(
        crate::capture::desktop::linux::hyprland::running() && crate::capture::desktop::linux::wayland_session(),
        "Automatic draft paste requires Hyprland Wayland"
    );
    let current = crate::capture::desktop::linux::hyprland::focused_window()?;
    ensure!(
        crate::capture::desktop::linux::auto_paste::window_matches(original, &current),
        "Original chat window is no longer focused; manual paste remains available"
    );
    let (directory, bundle) = load_bundle(bundle_dir)?;
    let sheet_path = directory.join(&bundle.contact_sheet);
    let image = fs::read(&sheet_path).context("Could not read contact sheet")?;
    let summary = clipboard_summary(&bundle, &directory);
    run_draft_handoff_at(
        &image,
        &summary,
        &sheet_path,
        false,
        original,
        &mut LinuxDraftHandoff,
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn draft_receiver_preflight(
    original: &crate::capture::desktop::linux::hyprland::FocusedWindow,
) -> Result<()> {
    LinuxDraftHandoff
        .supported(original)
        .context("Use the normal Copy and routing path for this composer")
}

fn paste_synthetic_checked() -> Result<()> {
    #[cfg(target_os = "linux")]
    return crate::capture::engine::runtime::try_post_synthetic_cmd_v();
    #[cfg(not(target_os = "linux"))]
    {
        crate::capture::engine::runtime::post_synthetic_cmd_v();
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn run_draft_handoff(
    image: &[u8],
    summary: &str,
    sheet_path: &Path,
    terminal: bool,
    ops: &mut impl DraftHandoffOps,
) -> Result<()> {
    let target = ops.target()?;
    run_draft_handoff_at(image, summary, sheet_path, terminal, &target, ops)
}

#[cfg(target_os = "linux")]
fn run_draft_handoff_at<O: DraftHandoffOps>(
    image: &[u8],
    summary: &str,
    sheet_path: &Path,
    terminal: bool,
    target: &O::Target,
    ops: &mut O,
) -> Result<()> {
    ops.supported(target)
        .context("Use manual image upload and text paste for this composer")?;
    // The user may select the composer after the recording finishes. Wait without
    // changing focus or clipboard; never turn an unfocused Ctrl+V into a silent loss.
    let mut editable = false;
    for _ in 0..30 {
        ops.check_target(target)?;
        if ops.editor_ready(target)? {
            editable = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    ensure!(
        editable,
        "No writable field selected; use manual image upload and text paste"
    );
    let snapshot = ops.snapshot()?;
    let path_text = sheet_path.display().to_string();
    let mut offered: Option<(&str, &[u8])> = None;
    let result = (|| {
        ops.check_target(&target)?;
        if terminal {
            offered = Some(("text/plain;charset=utf-8", path_text.as_bytes()));
            ops.text(&path_text)?;
        } else {
            offered = Some(("image/png", image));
            ops.image(image)?;
        }
        ops.check_target(&target)?;
        ensure!(
            ops.editor_ready(target)?,
            "Writable field lost before image paste"
        );
        ops.paste()?;
        let Some((mime, bytes)) = offered else {
            bail!("No image was offered");
        };
        let mut attached = false;
        for _ in 0..20 {
            ops.check_target(&target)?;
            if ops.attachment_visible(&target, image)? {
                attached = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        ensure!(
            attached,
            "Image attachment was not confirmed; use manual upload and paste"
        );
        ops.check_target(&target)?;
        ensure!(
            ops.still_ours(mime, bytes)?,
            "Clipboard changed; text was not pasted"
        );
        offered = Some(("text/plain;charset=utf-8", summary.as_bytes()));
        ops.text(summary)?;
        ops.check_target(&target)?;
        ensure!(
            ops.editor_ready(target)?,
            "Writable field lost before text paste"
        );
        ensure!(
            ops.still_ours("text/plain;charset=utf-8", summary.as_bytes())?,
            "Clipboard changed; text was not pasted"
        );
        ops.paste()
    })();
    let result = result.and_then(|()| {
        ops.check_target(&target)?;
        ensure!(
            ops.text_visible(&target, summary)?,
            "Transcript was not confirmed in the draft"
        );
        Ok(())
    });
    let restored = ops.restore(snapshot, offered);
    match (result, restored) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error.context("Could not restore clipboard")),
        (Err(error), Err(restore)) => {
            Err(error.context(format!("Clipboard restore also failed: {restore:#}")))
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) trait DraftHandoffOps {
    type Snapshot;
    /// Include the focused editor identity, not only its containing window.
    type Target: Clone + Eq;
    fn target(&mut self) -> Result<Self::Target>;
    fn supported(&mut self, target: &Self::Target) -> Result<()>;
    /// Confirm that the exact target is an active, writable editor, not merely a window.
    fn editor_ready(&mut self, target: &Self::Target) -> Result<bool>;
    fn check_target(&mut self, target: &Self::Target) -> Result<()> {
        ensure!(
            self.target()? == *target,
            "Focus moved; refusing to paste into another window"
        );
        Ok(())
    }
    /// Do not treat matching bytes alone as proof of clipboard ownership.
    fn still_ours(&mut self, mime: &str, bytes: &[u8]) -> Result<bool>;
    fn attachment_visible(&mut self, target: &Self::Target, image: &[u8]) -> Result<bool>;
    fn text_visible(&mut self, target: &Self::Target, text: &str) -> Result<bool>;
    fn snapshot(&mut self) -> Result<Self::Snapshot>;
    /// Restore only when the handoff still owns the selection; never overwrite another owner.
    fn restore(&mut self, snapshot: Self::Snapshot, offered: Option<(&str, &[u8])>) -> Result<()>;
    fn image(&mut self, image: &[u8]) -> Result<()>;
    fn text(&mut self, text: &str) -> Result<()>;
    fn paste(&mut self) -> Result<()>;
}

#[cfg(target_os = "linux")]
struct LinuxDraftHandoff;

#[cfg(target_os = "linux")]
impl DraftHandoffOps for LinuxDraftHandoff {
    type Snapshot = crate::capture::desktop::linux::clipboard::TextSnapshot;
    type Target = crate::capture::desktop::linux::hyprland::FocusedWindow;
    fn target(&mut self) -> Result<Self::Target> {
        crate::capture::desktop::linux::hyprland::focused_window()
    }
    fn supported(&mut self, _target: &Self::Target) -> Result<()> {
        bail!("No verified attachment observer is available for this composer")
    }
    fn editor_ready(&mut self, _target: &Self::Target) -> Result<bool> {
        // Window identity alone cannot establish that a focused text field exists.
        Ok(false)
    }
    fn still_ours(&mut self, mime: &str, bytes: &[u8]) -> Result<bool> {
        crate::capture::desktop::linux::clipboard::still_ours(mime, bytes)
    }
    fn attachment_visible(&mut self, _target: &Self::Target, _image: &[u8]) -> Result<bool> {
        bail!("No attachment observer")
    }
    fn text_visible(&mut self, _target: &Self::Target, _text: &str) -> Result<bool> {
        bail!("No text observer")
    }
    fn snapshot(&mut self) -> Result<Self::Snapshot> {
        crate::capture::desktop::linux::clipboard::snapshot_text_wayland()
    }
    fn restore(&mut self, snapshot: Self::Snapshot, offered: Option<(&str, &[u8])>) -> Result<()> {
        crate::capture::desktop::linux::clipboard::restore_text_wayland(snapshot, offered)
    }
    fn image(&mut self, image: &[u8]) -> Result<()> {
        copy_png_to_clipboard(image)
    }
    fn text(&mut self, text: &str) -> Result<()> {
        copy_string_to_clipboard(text)
    }
    fn paste(&mut self) -> Result<()> {
        std::thread::sleep(std::time::Duration::from_millis(20));
        crate::capture::engine::runtime::try_post_synthetic_cmd_v()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenCodeSession {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, rename = "parentID")]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub time: OpenCodeSessionTime,
    /// V2 puts the session's working directory here; V1 used a top-level `directory`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<OpenCodeSessionLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
}

impl OpenCodeSession {
    pub fn directory(&self) -> Option<&str> {
        self.location
            .as_ref()
            .and_then(|l| l.directory.as_deref())
            .or(self.directory.as_deref())
            .filter(|d| !d.is_empty())
    }

    pub fn is_open_root(&self) -> bool {
        self.parent_id.is_none() && self.time.archived.is_none()
    }

    /// The title, or the ID when it has none.
    pub fn label(&self) -> &str {
        if self.title.is_empty() {
            &self.id
        } else {
            &self.title
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct OpenCodeSessionLocation {
    #[serde(default)]
    pub directory: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct OpenCodeSessionTime {
    #[serde(default)]
    pub updated: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewed: Option<u64>,
    #[serde(default)]
    pub archived: Option<u64>,
}

fn executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// `name` on `PATH`. On Windows `name.exe` and `name.cmd` (npm and bun shims) count too.
pub(crate) fn on_path(name: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) && Path::new(name).extension().is_none() {
        vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            name.to_owned(),
        ]
    } else {
        vec![name.to_owned()]
    };
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .flat_map(|path| names.iter().map(move |name| path.join(name)))
            .find(|path| executable(path))
    })
}

fn opencode_binary() -> Result<PathBuf> {
    if let Some(bin) = std::env::var_os("OPENCODE_BIN").filter(|bin| !bin.is_empty()) {
        let path = PathBuf::from(bin);
        if executable(&path) {
            return Ok(path);
        }
        if let Some(path) = path.to_str().and_then(on_path) {
            return Ok(path);
        }
        bail!("OPENCODE_BIN is not an executable");
    }
    if let Some(path) = on_path("opencode2") {
        return Ok(path);
    }
    if let Some(path) = dirs::home_dir()
        .map(|home| home.join("bin/opencode2"))
        .filter(|path| executable(path))
    {
        return Ok(path);
    }
    bail!("Could not find opencode2; set OPENCODE_BIN or add it to PATH")
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Hosted sync preference; credentials are separate private artifacts, not settings.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[serde(default)]
    pub cloud: Option<crate::connect::hosted::CloudConfig>,
    /// Desired OS login registration. Missing leaves the OS's existing choice alone.
    #[serde(default)]
    pub launch_at_login: Option<bool>,
    #[serde(default)]
    pub agent_host: Option<String>,
    #[serde(default)]
    pub agent_opencode: Option<String>,
    #[serde(default)]
    pub update_channel: Option<String>,
    /// `false` turns Screen-capture auto-crop off; an object overrides its settings.
    #[serde(default)]
    pub autocrop: Option<serde_json::Value>,
    /// `false` keeps Mac captures of a dev site on the clipboard instead of sending them to the
    /// site's OpenCode session (`routing`).
    #[serde(default)]
    pub auto_send: Option<bool>,
    /// Feature flag: shows **Export Last Capture as Video…** in the menu (default off).
    #[serde(default)]
    pub video_export: Option<bool>,
    /// `"apple"` (default), `"hex"` or `"command"`: which engine transcribes recordings
    /// (`transcribe`). The command's own keys are read by `transcribe_command`, not here, so a
    /// malformed one can't reset this struct.
    #[serde(default)]
    pub transcriber: Option<String>,
    /// Reuse Linux Parakeet across captures for 30 idle minutes (default true).
    #[serde(default)]
    pub warm_recognizer: Option<bool>,
    /// Lower the output volume while recording (`playback`) to `playbackLevel` percent of it.
    #[serde(default)]
    pub lower_playback: Option<bool>,
    #[serde(default)]
    pub playback_level: Option<u8>,
    /// The Hex model to use when `transcriber` is `"hex"` (default: the first ready one of
    /// `hex::PREFERRED_MODELS`).
    #[serde(default)]
    pub hex_model: Option<String>,
    /// Hex language code (`"en"` by default, `"auto"` for model language detection).
    #[serde(default)]
    pub hex_language: Option<String>,
    /// Apple Speech locale (BCP-47, e.g. "zh-TW"); missing means the Mac's own language.
    #[serde(default)]
    pub apple_locale: Option<String>,
    /// Whether to use the native Metal recording pill instead of GPUI's `HudPillWindow` (default `true`).
    #[serde(default)]
    pub native_pill: Option<bool>,
    /// Visual style of the native recording pill: `"black"` (default) or `"glass"` (`NSGlassEffectView`).
    #[serde(default)]
    pub pill_style: Option<String>,
    /// Behaviour Kit tries locally before it ships (`{"agentContext": true}`); see `experiment`.
    /// Read leniently (`lenient_experiments`), so a stray value never fails the file.
    #[serde(default, deserialize_with = "lenient_experiments")]
    pub experiments: std::collections::BTreeMap<String, bool>,
    /// The microphone Mac captures record from (`mic`); absent follows the macOS default.
    #[serde(default)]
    pub microphone: Option<crate::capture::engine::runtime::audio::mic::Choice>,
    /// Smart routing's settings (`routing::look`).
    #[serde(default)]
    pub routing: Option<RoutingConfig>,
    /// Where new captures go (`default_captures_dir`); absent keeps the built-in folder.
    #[serde(default)]
    pub captures_dir: Option<String>,
}

impl Config {
    pub fn warm_recognizer_enabled(&self) -> bool {
        self.warm_recognizer.unwrap_or(true)
    }

    /// The output volume while recording when `lowerPlayback` is on, in percent of what it was.
    pub fn recording_volume(&self) -> u8 {
        self.playback_level.unwrap_or(20).min(100)
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoutingConfig {
    /// The checkout each app is built from, by bundle ID or a prefix of one
    /// (`{"ai.opencode.desktop": "~/code/opencode"}`), for apps whose checkout isn't named like
    /// them.
    #[serde(default)]
    pub apps: std::collections::BTreeMap<String, String>,
    /// The folder a URL prefix serves, for file servers whose pages aren't their own project's
    /// (`{"https://studio.example.ts.net:8444/raw/": "~/code/home/artifacts"}`).
    #[serde(default)]
    pub servers: std::collections::BTreeMap<String, String>,
}

/// `true`/`false`, `"yes"`/`"on"`/`"1"` and their opposites, or a number (non-zero is on). Other
/// values are skipped and logged once, as is an `experiments` that isn't an object.
fn lenient_experiments<'de, D>(
    deserializer: D,
) -> std::result::Result<std::collections::BTreeMap<String, bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde_json::Value;
    let value = Value::deserialize(deserializer)?;
    let mut out = std::collections::BTreeMap::new();
    let Value::Object(map) = value else {
        if !value.is_null() {
            log_config_once(format!(
                "config.json: ignoring \"experiments\": {value} (expected an object)"
            ));
        }
        return Ok(out);
    };
    for (name, value) in map {
        let on = match &value {
            Value::Bool(b) => Some(*b),
            Value::Number(n) => n.as_f64().map(|n| n != 0.0),
            Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" | "1" => Some(true),
                "false" | "no" | "off" | "0" | "" => Some(false),
                _ => None,
            },
            _ => None,
        };
        match on {
            Some(on) => {
                out.insert(name, on);
            }
            None => log_config_once(format!(
                "config.json: ignoring experiment \"{name}\": {value} (expected true or false)"
            )),
        }
    }
    Ok(out)
}

/// Config problems are logged once per process, not on every read.
fn log_config_once(message: String) {
    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !seen.contains(&message) {
        crate::capture::engine::runtime::apparent_log(&message);
        seen.push(message);
    }
}

/// More of what Apparent already knows in the summary: marks, shortcuts, drags and windows inline
/// at their time, numbered marks on shared frames, where a screen changed.
pub const AGENT_CONTEXT: &str = "agentContext";

/// Transcribe Mac recordings while they record, so finishing only waits for the last words
/// (`live_transcribe`).
pub const LIVE_TRANSCRIBE: &str = "liveTranscribe";

static EXPERIMENT_OVERRIDES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Turns an experiment on for this process (`apparent reprocess --experiment <name>`).
pub fn enable_experiment(name: &str) {
    EXPERIMENT_OVERRIDES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(name.to_owned());
}

fn experiment_overridden(name: &str) -> bool {
    EXPERIMENT_OVERRIDES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|n| n == name)
}

/// Whether an experiment is on: `"experiments": {"<name>": true}` in `config.json`, or
/// `enable_experiment` in this process. Off by default; tests see only `enable_experiment`.
pub fn experiment(name: &str) -> bool {
    let overridden = experiment_overridden(name);
    if overridden || cfg!(test) {
        return overridden;
    }
    experiment_enabled_in(&config(), name)
}

/// Most experiments are opt-in. This one is on unless explicitly disabled.
pub(crate) fn experiment_enabled_in(config: &Config, name: &str) -> bool {
    config.experiments.get(name).copied().unwrap_or(name == AGENT_DEMO_VIDEO)
}

pub fn load_config_from(path: &Path) -> Config {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .map(config_from_value)
        .unwrap_or_default()
}

/// The config in `value`. A key whose value doesn't fit (`"autoSend": "maybe"`) is dropped and
/// logged on its own, so one bad setting never resets the others to their defaults.
fn config_from_value(value: serde_json::Value) -> Config {
    let serde_json::Value::Object(map) = value else {
        return Config::default();
    };
    if let Ok(config) = serde_json::from_value(serde_json::Value::Object(map.clone())) {
        return config;
    }
    // Add keys one at a time, keeping each only if everything so far still reads.
    let mut kept = serde_json::Map::new();
    for (key, value) in map {
        kept.insert(key.clone(), value.clone());
        if let Err(error) =
            serde_json::from_value::<Config>(serde_json::Value::Object(kept.clone()))
        {
            kept.remove(&key);
            log_config_once(format!("config.json: ignoring \"{key}\": {error}"));
        }
    }
    serde_json::from_value(serde_json::Value::Object(kept)).unwrap_or_default()
}

fn config() -> Config {
    load_config_from(&default_config_path())
}

/// Read at recording start; edits never interrupt an active capture. Tests use defaults.
pub fn warm_recognizer() -> bool {
    if cfg!(test) {
        Config::default().warm_recognizer_enabled()
    } else {
        config().warm_recognizer_enabled()
    }
}



/// `routing.apps` from config.json, `~` expanded. Tests see none.
pub fn routing_apps() -> Vec<(String, PathBuf)> {
    routing_paths(|routing| routing.apps)
}

/// `routing.servers` in config.json: URL prefixes and the folders they serve, `~` expanded.
pub fn routing_servers() -> Vec<(String, PathBuf)> {
    routing_paths(|routing| routing.servers)
}

fn routing_paths(
    pick: fn(RoutingConfig) -> std::collections::BTreeMap<String, String>,
) -> Vec<(String, PathBuf)> {
    if cfg!(test) {
        return Vec::new();
    }
    let home = dirs::home_dir().unwrap_or_default();
    config()
        .routing
        .map(pick)
        .unwrap_or_default()
        .into_iter()
        .map(|(id, path)| {
            let path = match path.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => PathBuf::from(path),
            };
            (id, path)
        })
        .collect()
}

/// Whether Mac captures of a dev site go straight to its session (`autoSend`, default on).
pub fn auto_send() -> bool {
    config().auto_send.unwrap_or(true)
}

/// Whether the menu offers video export (`videoExport`, default off). `apparent export` works
/// either way.
pub fn video_export() -> bool {
    config().video_export.unwrap_or(false)
}

/// The configured transcriber name (`transcriber`) as written, unvalidated.
pub fn transcriber() -> Option<String> {
    config().transcriber
}

/// The configured Hex model id (`hexModel`), if any.
pub fn hex_model() -> Option<String> {
    config()
        .hex_model
        .map(|model| model.trim().to_owned())
        .filter(|model| !model.is_empty())
}

/// The configured Hex language, defaulting to the previous English behavior.
pub fn hex_language() -> String {
    hex_language_from(&config())
}

pub fn hex_language_from(config: &Config) -> String {
    config
        .hex_language
        .as_deref()
        .and_then(crate::shared::hex::language_code)
        .unwrap_or_else(|| crate::shared::hex::DEFAULT_LANGUAGE.into())
}

/// The configured Apple Speech locale, or the Mac's own language when unset.
pub fn apple_locale() -> Option<String> {
    apple_locale_from(&config())
}

pub fn apple_locale_from(config: &Config) -> Option<String> {
    config
        .apple_locale
        .as_deref()
        .and_then(crate::capture::engine::runtime::audio::transcription::speech_model::locale_id)
}

/// The command transcriber's program (`transcribeCommand`, `transcribeTimeoutSeconds`), read
/// on its own from the config file.
pub fn transcribe_command()
-> Result<crate::capture::engine::runtime::audio::transcription::command_transcriber::Program, crate::capture::engine::runtime::audio::transcription::command_transcriber::Unusable> {
    crate::capture::engine::runtime::audio::transcription::command_transcriber::Program::load_from(&default_config_path())
}

/// Whether the native AppKit + Metal recording pill is enabled (`APPARENT_NATIVE_PILL` or
/// `nativePill` in `config.json`, default `true`). macOS only: elsewhere the GPUI capsule draws.
pub fn native_pill() -> bool {
    if cfg!(not(target_os = "macos")) {
        return false;
    }
    if let Ok(val) = std::env::var("APPARENT_NATIVE_PILL") {
        let s = val.trim().to_ascii_lowercase();
        if matches!(s.as_str(), "0" | "false" | "gpui" | "off" | "no") {
            return false;
        }
        if matches!(s.as_str(), "1" | "true" | "native" | "metal" | "on" | "yes") {
            return true;
        }
    }
    config().native_pill.unwrap_or(true)
}

/// Configured capsule style (`pillStyle` in `config.json`, default `"black"`).
pub fn pill_style() -> crate::capture::desktop::native::pill::PillStyle {
    config()
        .pill_style
        .as_deref()
        .and_then(crate::capture::desktop::native::pill::PillStyle::from_str_opt)
        .unwrap_or(crate::capture::desktop::native::pill::PillStyle::Black)
}

/// Change one experiment without replacing unrelated flags or secret-bearing config keys.
pub(crate) fn set_experiment_in(path: &Path, name: &str, on: bool) -> Result<()> {
    update_config_in(path, |config| {
        let experiments = config
            .as_object_mut()
            .unwrap()
            .entry("experiments")
            .or_insert_with(|| serde_json::json!({}));
        let experiments = experiments
            .as_object_mut()
            .with_context(|| format!("Invalid experiments at {}", path.display()))?;
        experiments.insert(name.to_owned(), on.into());
        Ok(())
    })
}



/// Safe to put unquoted in a remote shell command (host names, `~/bin/tool` paths).
fn safe_word(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.@/~".contains(&b))
}

/// The Mac whose OpenCode server holds Kit's sessions when it isn't this one: an SSH host from
/// `APPARENT_AGENT_HOST` or `~/.config/apparent/config.json` (`{"agentHost": "studio"}`).
pub fn agent_host() -> Option<String> {
    std::env::var("APPARENT_AGENT_HOST")
        .ok()
        .or_else(|| config().agent_host)
        .filter(|h| safe_word(h))
}

/// The `opencode2` command to run on the agent host (`agentOpencode`, default `opencode2`).
fn agent_opencode() -> String {
    config()
        .agent_opencode
        .filter(|c| safe_word(c))
        .unwrap_or_else(|| "opencode2".to_owned())
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// A system command: `/usr/bin/<name>` on macOS; on Windows the copy in System32 (OpenSSH and
/// bsdtar ship with Windows 10 and later), and on Linux the one on `PATH`.
fn system_tool(name: &str) -> PathBuf {
    if cfg!(not(target_os = "macos")) {
        on_path(name).unwrap_or_else(|| PathBuf::from(name))
    } else {
        Path::new("/usr/bin").join(name)
    }
}

fn ssh(host: &str) -> Command {
    let mut command = Command::new(system_tool("ssh"));
    command.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=8", host]);
    command
}

/// Bundle files too big to hold up queueing the session; they follow in the background.
pub const HEAVY_MEDIA: [&str; 5] = [
    "recording.mp4",
    "recording.mov",
    "audio.wav",
    "frames-full",
    "contact-sheet-full.png",
];
/// Present in a copied bundle while its heavy media is still arriving.
pub const MEDIA_PENDING: &str = ".media-pending";

/// Copies a bundle to the same path on `host`, so the agent there can open its files.
/// Syncs the contact sheet, keyframe PNGs, manifest, and summary synchronously so the session
/// can be queued immediately, then streams heavy media (`recording.mp4`/`.mov`, `frames-full`)
/// in the background for `apparent zoom`, marked by [`MEDIA_PENDING`] until it lands.
pub fn sync_bundle_to(host: &str, bundle_dir: &Path) -> Result<()> {
    use std::process::Stdio;
    let parent = bundle_dir.parent().context("Bundle has no parent")?;
    let name = bundle_dir.file_name().context("Bundle has no name")?;
    let mut tar = Command::new(system_tool("tar"))
        .arg("-C")
        .arg(parent)
        .args([
            "--exclude=*/recording.mp4",
            "--exclude=*/recording.mov",
            "--exclude=*/audio.wav",
            "--exclude=*/frames-full",
            "--exclude=*/contact-sheet-full.png",
            "--exclude=*/.media-sending",
        ])
        .arg("-cf")
        .arg("-")
        .arg(name)
        .stdout(Stdio::piped())
        .spawn()
        .context("Could not start tar")?;
    let parent_q = shell_quote(&parent.to_string_lossy());
    let name_str = name.to_string_lossy().into_owned();
    let heavy: Vec<String> = HEAVY_MEDIA
        .iter()
        .filter(|rel| bundle_dir.join(rel).exists())
        .map(|rel| format!("{name_str}/{rel}"))
        .collect();
    let bundle_q = shell_quote(&parent.join(&name_str).to_string_lossy());
    // The marker goes up with the light files, before the session is prompted, so `apparent zoom`
    // there knows the video is still on its way rather than missing.
    let mark_pending = if heavy.is_empty() {
        String::new()
    } else {
        format!(" && touch {bundle_q}/{MEDIA_PENDING}")
    };
    let output = ssh(host)
        .arg(format!(
            "mkdir -p {parent_q} && tar -C {parent_q} -xf -{mark_pending}"
        ))
        .stdin(tar.stdout.take().context("No tar output")?)
        .output()
        .context("Could not start ssh")?;
    let _ = tar.wait();
    ensure!(
        output.status.success(),
        "Copying the bundle to {host} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );

    if !heavy.is_empty() {
        let host_owned = host.to_owned();
        let parent_owned = parent.to_path_buf();
        // Unpack into a staging folder and move into the bundle only once complete, so nothing on
        // the other side ever reads a half-copied video; then clear the marker.
        let staging_q = shell_quote(
            &parent
                .join(format!(".{name_str}.incoming"))
                .to_string_lossy(),
        );
        let remote = format!(
            "rm -rf {staging_q} && mkdir -p {staging_q} && tar -C {staging_q} -xf - && \
             for f in {staging_q}/*/*; do rm -rf {bundle_q}/\"$(basename \"$f\")\" && mv \"$f\" {bundle_q}/; done; \
             rm -rf {staging_q} {bundle_q}/{MEDIA_PENDING}"
        );
        // Held until the media lands, so the iPhone video isn't re-encoded or cleaned up under
        // the copy (see `retention`).
        let sending = bundle_dir.join(crate::capture::engine::bundle::retention::MEDIA_SENDING);
        let _ = fs::write(&sending, host);
        std::thread::spawn(move || {
            let _done = RemoveOnDrop(sending);
            let Ok(mut bg_tar) = Command::new(system_tool("tar"))
                .arg("-C")
                .arg(&parent_owned)
                .arg("-cf")
                .arg("-")
                .args(&heavy)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            else {
                return;
            };
            if let Some(stdout) = bg_tar.stdout.take() {
                let _ = ssh(&host_owned)
                    .arg(remote)
                    .stdin(stdout)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let _ = bg_tar.wait();
        });
    }
    Ok(())
}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn opencode_api(method: &str, path: &str, data: Option<&serde_json::Value>) -> Result<Vec<u8>> {
    let mut command = if let Some(host) = agent_host() {
        let mut remote = format!(
            "PATH=\"$HOME/bin:$HOME/.opencode/bin:/opt/homebrew/bin:$PATH\" {} api {} {}",
            agent_opencode(),
            shell_quote(method),
            shell_quote(path)
        );
        if let Some(data) = data {
            remote.push_str(&format!(
                " --data {}",
                shell_quote(&serde_json::to_string(data)?)
            ));
        }
        let mut command = ssh(&host);
        command.arg(remote);
        command
    } else {
        let mut command = Command::new(opencode_binary()?);
        command.args(["api", method, path]);
        if let Some(data) = data {
            command.args(["--data", &serde_json::to_string(data)?]);
        }
        command
    };
    let output = command.output().context("Could not launch opencode2")?;
    ensure!(
        output.status.success(),
        "OpenCode API failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output.stdout)
}

pub(crate) fn query_escape(value: &str) -> String {
    let mut result = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            result.push(char::from(byte));
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}

/// One `GET` against OpenCode's API, returning the body. Swapped for a fake in tests.
pub type ApiGet<'a> = dyn FnMut(&str) -> Result<Vec<u8>> + 'a;

pub fn api_get(path: &str) -> Result<Vec<u8>> {
    if agent_host().is_none()
        && let Ok(body) = api_get_within(path, std::time::Duration::from_secs(5))
    {
        return Ok(body);
    }
    opencode_api("get", path, None)
}

/// How long a TCP connect to the local OpenCode server may take before giving up.
pub const SERVER_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(400);
const SERVER_RESPONSE_LIMIT: u64 = 16 * 1024 * 1024;
const DEFAULT_SERVER_PORT: u16 = 0xc0de;

/// Only the connection fields of OpenCode's `service.json` (never its `env` block).
#[derive(Default, Deserialize)]
struct ServiceFile {
    url: Option<String>,
    hostname: Option<String>,
    port: Option<u16>,
    password: Option<String>,
}

impl ServiceFile {
    fn load(path: &Path) -> Option<Self> {
        serde_json::from_slice(&fs::read(path).ok()?).ok()
    }

    fn address(&self) -> Option<std::net::SocketAddr> {
        self.url.as_deref().and_then(server_address).or_else(|| {
            let host = self.hostname.as_deref().unwrap_or("127.0.0.1");
            server_address(&format!("http://{host}:{}", self.port?))
        })
    }

    fn password(&self) -> Option<String> {
        non_empty(self.password.as_deref())
    }
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// The socket address in `http://127.0.0.1:4096…` when its host is an IP or `localhost` (no
/// lookups).
fn server_address(url: &str) -> Option<std::net::SocketAddr> {
    let parsed = crate::shared::routing::parse_url(url.split_whitespace().next()?)?;
    let ip = if parsed.host == "localhost" {
        std::net::IpAddr::from([127, 0, 0, 1])
    } else {
        parsed.host.parse().ok()?
    };
    Some(std::net::SocketAddr::new(ip, parsed.port))
}

fn resolve_server(
    files: &[ServiceFile],
    env_password: Option<&str>,
) -> (std::net::SocketAddr, Option<String>) {
    let (address, matched_password) = files
        .iter()
        .find_map(|f| Some((f.address()?, f.password())))
        .unwrap_or_else(|| {
            (
                std::net::SocketAddr::from(([127, 0, 0, 1], DEFAULT_SERVER_PORT)),
                None,
            )
        });
    let password = matched_password
        .or_else(|| non_empty(env_password))
        .or_else(|| files.iter().find_map(ServiceFile::password));
    (address, password)
}

/// Finds the local OpenCode V2 server from its state registration (`$XDG_STATE_HOME/opencode/service.json`
/// or `~/.local/state/opencode/service.json`), service config (`$OPENCODE_CONFIG_DIR/service.json`
/// or `~/.config/opencode/service.json`), and `OPENCODE_PASSWORD`.
fn local_server() -> (std::net::SocketAddr, Option<String>) {
    let env_dir = |name: &str| {
        std::env::var_os(name)
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
    };
    let home = dirs::home_dir();
    let files: Vec<ServiceFile> = [
        env_dir("XDG_STATE_HOME").map(|d| d.join("opencode/service.json")),
        home.as_ref()
            .map(|h| h.join(".local/state/opencode/service.json")),
        env_dir("OPENCODE_CONFIG_DIR").map(|d| d.join("service.json")),
        env_dir("XDG_CONFIG_HOME").map(|d| d.join("opencode/service.json")),
        home.as_ref()
            .map(|h| h.join(".config/opencode/service.json")),
    ]
    .into_iter()
    .flatten()
    .filter_map(|p| ServiceFile::load(&p))
    .collect();
    let env_password = std::env::var("OPENCODE_PASSWORD")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("OPENCODE_SERVER_PASSWORD").ok());
    resolve_server(&files, env_password.as_deref())
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = u32::from_be_bytes([0, c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)]);
        let ch = |shift: u32| char::from(TABLE[((n >> shift) & 63) as usize]);
        out.extend([
            ch(18),
            ch(12),
            if c.len() > 1 { ch(6) } else { '=' },
            if c.len() > 2 { ch(0) } else { '=' },
        ]);
    }
    out
}

fn http_get(
    address: std::net::SocketAddr,
    password: Option<&str>,
    path: &str,
    timeout: std::time::Duration,
) -> Result<Vec<u8>> {
    use std::io::{Read, Write};
    let connect = SERVER_CONNECT_TIMEOUT
        .min(timeout)
        .max(std::time::Duration::from_millis(1));
    let timeout = timeout.max(std::time::Duration::from_millis(1));
    let mut stream = std::net::TcpStream::connect_timeout(&address, connect)
        .with_context(|| format!("Could not reach server at http://{address}"))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.set_read_timeout(Some(timeout))?;
    let auth = match password {
        Some(password) if !password.contains(['\r', '\n']) => format!(
            "Authorization: Basic {}\r\n",
            base64(format!("opencode:{password}").as_bytes())
        ),
        _ => String::new(),
    };
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: {address}\r\n{auth}Connection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .with_context(|| format!("Could not write to server at http://{address}"))?;
    let mut bytes = Vec::new();
    stream
        .take(SERVER_RESPONSE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("OpenCode at http://{address} didn't answer in time"))?;
    ensure!(
        bytes.len() as u64 <= SERVER_RESPONSE_LIMIT,
        "OpenCode response too large"
    );
    let split = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("Invalid OpenCode HTTP response")?;
    let header = std::str::from_utf8(&bytes[..split]).context("Invalid OpenCode HTTP header")?;
    let status: u16 = header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .context("Missing OpenCode HTTP status")?
        .parse()
        .context("Invalid OpenCode HTTP status")?;
    let body = bytes[split + 4..].to_vec();
    if let Some(length) = header
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
    {
        ensure!(body.len() == length, "Incomplete OpenCode HTTP response");
    }
    ensure!(
        (200..300).contains(&status),
        "OpenCode API failed: HTTP {status}"
    );
    Ok(body)
}

/// [`api_get`] for routing: talks HTTP directly to the local OpenCode server with a
/// [`SERVER_CONNECT_TIMEOUT`] connect cap and `timeout` read deadline, instead of spawning
/// `opencode2 api` per request.
pub fn api_get_within(path: &str, timeout: std::time::Duration) -> Result<Vec<u8>> {
    if agent_host().is_some() {
        return opencode_api("get", path, None);
    }
    let (address, password) = local_server();
    http_get(address, password.as_deref(), path, timeout)
}

/// How many pages of root sessions (100 each, newest first) a search reads before giving up.
pub const ROOT_SEARCH_PAGES: usize = 5;
const ROOT_PAGE: usize = 100;

/// One session by ID (`GET /api/session/{id}`).
pub fn get_session(get: &mut ApiGet, id: &str) -> Result<OpenCodeSession> {
    let response: serde_json::Value =
        serde_json::from_slice(&get(&format!("/api/session/{}", query_escape(id)))?)
            .context("Invalid OpenCode session response")?;
    let session = response.get("data").unwrap_or(&response).clone();
    Ok(serde_json::from_value(session)?)
}

/// What a root-session search does with each open root session it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scan {
    Skip,
    Take,
    Stop,
}

/// Reads open root sessions newest first (`query` adds filters, e.g. `&directory=…`) and returns
/// the first that `scan` takes, reading at most `max_pages` pages.
pub fn find_root_session(
    get: &mut ApiGet,
    query: &str,
    max_pages: usize,
    mut scan: impl FnMut(&OpenCodeSession) -> Scan,
) -> Result<Option<OpenCodeSession>> {
    let mut path = format!("/api/session?parentID=null&limit={ROOT_PAGE}{query}");
    let mut cursors = std::collections::HashSet::new();
    for _ in 0..max_pages {
        let response: serde_json::Value =
            serde_json::from_slice(&get(&path)?).context("Invalid OpenCode session response")?;
        let data = response
            .as_array()
            .or_else(|| response.get("data").and_then(|data| data.as_array()))
            .context("Missing OpenCode session list")?;
        for session in data
            .iter()
            .filter_map(|s| serde_json::from_value::<OpenCodeSession>(s.clone()).ok())
        {
            // The API has no archived filter.
            if !session.is_open_root() {
                continue;
            }
            match scan(&session) {
                Scan::Skip => {}
                Scan::Take => return Ok(Some(session)),
                Scan::Stop => return Ok(None),
            }
        }
        let Some(next) = response.pointer("/cursor/next").and_then(|v| v.as_str()) else {
            break;
        };
        if data.is_empty() || !cursors.insert(next.to_owned()) {
            break;
        }
        // The cursor carries the filters.
        path = format!(
            "/api/session?cursor={}&limit={ROOT_PAGE}",
            query_escape(next)
        );
    }
    Ok(None)
}

pub(crate) fn normalize_slug(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Sends a bundle to `session_id`, else to the session its recording was addressed to, else the
/// session the app resolves to. Sending an addressed recording elsewhere needs `reroute`.
pub fn send_to_opencode(
    bundle_dir: &Path,
    session_id: Option<&str>,
    reroute: bool,
) -> Result<String> {
    let addressed = recorded_destination(&resolve_bundle_dir(bundle_dir)?);
    let session_id = choose_destination(session_id, addressed.as_deref(), reroute)?;
    let (id, _) = send_to_opencode_for_app(bundle_dir, session_id, None)?;
    Ok(id)
}

/// The session an older imported recording was addressed to: its timeline's top-level `session`,
/// else the `destination` a Mac capture of a dev site was routed to (`apparent.json`).
/// `context.session` only names the session that was on screen, and never chooses a destination.
pub fn recorded_destination(bundle_dir: &Path) -> Option<String> {
    let read = |name: &str| -> Option<serde_json::Value> {
        serde_json::from_slice(&fs::read(bundle_dir.join(name)).ok()?).ok()
    };
    let session = |value: Option<&serde_json::Value>| {
        value
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    read("timeline.json")
        .and_then(|timeline| session(timeline.get("session")))
        .or_else(|| read("apparent.json").and_then(|b| session(b.pointer("/destination/session"))))
}

fn choose_destination<'a>(
    requested: Option<&'a str>,
    addressed: Option<&'a str>,
    reroute: bool,
) -> Result<Option<&'a str>> {
    match (requested, addressed) {
        (Some(requested), Some(addressed)) if requested != addressed && !reroute => bail!(
            "This recording was addressed to {addressed}; pass --reroute to send it to {requested} instead"
        ),
        (Some(requested), _) => Ok(Some(requested)),
        (None, addressed) => Ok(addressed),
    }
}

pub fn send_to_opencode_for_app(
    bundle_dir: &Path,
    session_id: Option<&str>,
    app_name: Option<&str>,
) -> Result<(String, String)> {
    let (_, bundle) = load_bundle(bundle_dir)?;
    let (session, title) =
        resolve_opencode_session(session_id, app_name.or(bundle.target.app.as_deref()))?;
    send_bundle_to_session(bundle_dir, &session)?;
    Ok((session, title))
}

/// Picks the session a capture goes to, and its title: an explicit ID, else the root session
/// whose title matches the app ("UpDown" → "UP DOWN APP"), else `OPENCODE_SESSION_ID`, else the
/// most recently updated root session.
pub fn resolve_opencode_session(
    session_id: Option<&str>,
    app_name: Option<&str>,
) -> Result<(String, String)> {
    let env_session = std::env::var("OPENCODE_SESSION_ID").ok();
    resolve_opencode_session_with(&mut api_get, session_id, app_name, env_session.as_deref())
}

/// [`resolve_opencode_session`] with the API and `OPENCODE_SESSION_ID` passed in. An explicit ID
/// costs one request; an app title match reads the newest [`ROOT_SEARCH_PAGES`] pages of root
/// sessions at most.
pub fn resolve_opencode_session_with(
    get: &mut ApiGet,
    session_id: Option<&str>,
    app_name: Option<&str>,
    env_session: Option<&str>,
) -> Result<(String, String)> {
    if let Some(id) = session_id.filter(|s| !s.trim().is_empty()) {
        let title = get_session(get, id)
            .ok()
            .map(|s| s.title)
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| id.to_owned());
        return Ok((id.to_owned(), title));
    }

    // Match against the last component if a bundle identifier was passed (com.kitlangton.UpDown -> updown).
    let short = app_name
        .and_then(|a| a.rsplit('.').next())
        .map(normalize_slug)
        .filter(|s| !s.is_empty())
        .or_else(|| app_name.map(normalize_slug).filter(|s| !s.is_empty()));
    let mut newest = None;
    let matched = find_root_session(get, "", ROOT_SEARCH_PAGES, |s| {
        if newest.is_none() {
            newest = Some(s.clone());
        }
        let Some(short) = short.as_deref() else {
            return Scan::Stop;
        };
        let title_slug = normalize_slug(&s.title);
        if !title_slug.is_empty() && (title_slug.contains(short) || short.contains(&title_slug)) {
            Scan::Take
        } else {
            Scan::Skip
        }
    })?;
    let matched = matched
        .or_else(|| {
            let id = env_session.filter(|id| !id.trim().is_empty())?;
            get_session(get, id)
                .ok()
                .filter(OpenCodeSession::is_open_root)
        })
        .or(newest)
        .context("No root OpenCode session found; pass a session ID")?;
    let title = matched.label().to_owned();
    Ok((matched.id, title))
}

/// Queues a bundle's summary and pointers into an OpenCode session.
pub fn send_bundle_to_session(bundle_dir: &Path, session: &str) -> Result<()> {
    let (directory, bundle) = load_bundle(bundle_dir)?;
    ensure!(
        session.starts_with("ses")
            && session
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "Invalid OpenCode session ID"
    );
    if let Some(host) = agent_host() {
        sync_bundle_to(&host, &directory)?;
    }
    opencode_api(
        "post",
        &format!("/api/session/{session}/prompt"),
        Some(&session_prompt(&directory, &bundle)),
    )?;
    Ok(())
}

fn session_prompt(directory: &Path, bundle: &ApparentBundle) -> serde_json::Value {
    let summary = crate::capture::engine::sketch::summary_markdown(bundle, directory);
    let quoted_directory = shell_quote(&directory.to_string_lossy());
    let contact_sheet = directory.join(&bundle.contact_sheet);
    let clicked = bundle.has_screen_click();
    let visual_note = if clicked {
        format!("The contact sheet is attached ({}).", contact_sheet.display())
    } else {
        format!("No image is attached; the recorded visuals remain in {}. Look them up only if needed.", directory.display())
    };
    let text = format!(
        "{}\n\n{} Read {} for timestamped transcript, marks, and interactions.\nTo look closely at a moment (an animation, a glitch, what the narration points at), run:\napparent zoom {} --at <M:SS or \"a phrase that was said\"> [--span 2] [--frames 12]\nIt saves a dense contact sheet of that moment with each frame's time and prints what was said. This capture is {} ms long.",
        summary.trim_end(),
        visual_note,
        directory.join("apparent.json").display(),
        quoted_directory,
        bundle.duration_ms,
    );
    // The server reads `file://` attachments from its own disk; the bundle is at the same path
    // there (copied above when the agents run on another Mac).
    let files = if clicked {
        serde_json::json!([{
                "uri": file_uri(&contact_sheet),
                "name": "contact-sheet.png",
                "description": "Apparent contact sheet: the capture's keyframes in order",
            }])
    } else {
        serde_json::json!([])
    };
    serde_json::json!({ "text": text, "files": files, "delivery": "queue" })
}

/// A `file://` URL for an absolute path, percent-encoding everything but unreserved
/// characters and `/` ("Application Support" → "Application%20Support"). A Windows path
/// (`C:\Users\…`, or `\\?\C:\…` after `canonicalize`) becomes `file:///C:/Users/…`.
pub(crate) fn file_uri(path: &Path) -> String {
    let mut text = path.to_string_lossy().into_owned();
    let mut uri = String::from("file://");
    let windows_path = cfg!(windows) || text.as_bytes().get(1) == Some(&b':');
    if windows_path {
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            text = rest.to_owned();
        }
        text = text.replace('\\', "/");
        if !text.starts_with('/') {
            uri.push('/');
        }
    }
    for (index, byte) in text.bytes().enumerate() {
        let drive_colon = windows_path && index == 1 && byte == b':';
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) || drive_colon {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

pub fn extract_frames_range(
    bundle_dir: &Path,
    from_ms: u64,
    to_ms: u64,
    fps: u32,
) -> Result<Vec<PathBuf>> {
    ensure!(from_ms <= to_ms, "Frame range ends before it starts");
    ensure!(fps > 0 && fps <= 240, "FPS must be between 1 and 240");
    let directory = resolve_bundle_dir(bundle_dir)?;
    let loaded = load_bundle(&directory).ok().map(|(_, bundle)| bundle);
    let offset_ms = loaded.as_ref().map_or(0, |b| b.video_offset_ms);
    let video = loaded
        .and_then(|bundle| bundle.video)
        .map_or_else(|| directory.join("recording.mp4"), |v| directory.join(v));
    if video.is_file() && from_ms < to_ms {
        let output_dir = directory
            .join("zoom")
            .join(format!("range-{from_ms}-{to_ms}"));
        if let Some(ffmpeg) = on_path("ffmpeg") {
            fs::create_dir_all(&output_dir)?;
            // Only remove our generated files, so repeating at a lower FPS cannot
            // return leftover frames from a previous extraction.
            for entry in fs::read_dir(&output_dir)? {
                let entry = entry?;
                if generated_frame(&entry.path()) {
                    fs::remove_file(entry.path())?;
                }
            }
            let output = Command::new(ffmpeg)
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-nostdin",
                    "-y",
                    "-ss",
                ])
                .arg(format!("{:.3}", from_ms as f64 / 1_000.0))
                .args(["-threads", "1", "-filter_threads", "1"])
                .arg("-i")
                .arg(&video)
                .arg("-t")
                .arg(format!("{:.3}", (to_ms - from_ms) as f64 / 1_000.0))
                .arg("-vf")
                .arg(format!("fps={fps}"))
                .args(["-threads", "1"])
                .arg(output_dir.join("frame-%03d.png"))
                .output()
                .context("Could not start ffmpeg")?;
            ensure!(
                output.status.success(),
                "ffmpeg extraction failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            let mut frames = Vec::new();
            for entry in fs::read_dir(&output_dir)? {
                let path = entry?.path();
                if generated_frame(&path) {
                    frames.push(path);
                }
            }
            frames.sort();
            return Ok(frames);
        }

        let step_ms = (1_000 / u64::from(fps)).max(1);
        let times: Vec<u64> = (from_ms..=to_ms).step_by(step_ms as usize).collect();
        if !times.is_empty()
            && let Ok(decoded) = crate::capture::engine::bundle::bundle_processing::movie_frames_at(&video, &times, offset_ms, 0)
            && !decoded.is_empty()
        {
            fs::create_dir_all(&output_dir)?;
            for entry in fs::read_dir(&output_dir)? {
                let entry = entry?;
                if generated_frame(&entry.path()) {
                    fs::remove_file(entry.path())?;
                }
            }
            let mut written = Vec::with_capacity(decoded.len());
            for (idx, frame) in decoded.iter().enumerate() {
                let out_path = output_dir.join(format!("frame-{:03}.png", idx + 1));
                crate::capture::engine::render::save_png(frame, &out_path)?;
                written.push(out_path);
            }
            return Ok(written);
        }
    }
    existing_keyframes(&directory, from_ms, to_ms, fps)
}

fn generated_frame(path: &Path) -> bool {
    path.is_file()
        && path.extension().is_some_and(|ext| ext == "png")
        && path
            .file_stem()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("frame-"))
            .is_some_and(|digits| {
                !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
            })
}

fn existing_keyframes(
    directory: &Path,
    from_ms: u64,
    to_ms: u64,
    fps: u32,
) -> Result<Vec<PathBuf>> {
    let (_, bundle) = load_bundle(directory)?;
    let margin = 1_000_u64.div_ceil(u64::from(fps)) / 2;
    let mut frames = Vec::new();
    for frame in bundle.keyframes {
        if frame.t < from_ms.saturating_sub(margin) || frame.t > to_ms.saturating_add(margin) {
            continue;
        }
        let relative = Path::new(&frame.file);
        ensure!(
            !relative.is_absolute()
                && relative.components().all(|part| matches!(
                    part,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )),
            "Invalid keyframe path"
        );
        let path = directory.join(relative);
        if generated_frame(&path) {
            let canonical = plain_path(path.canonicalize()?);
            ensure!(
                canonical.starts_with(directory.join("frames")),
                "Keyframe is outside bundle frames directory"
            );
            frames.push((frame.t, canonical));
        }
    }
    frames.sort_unstable();
    frames.dedup_by(|a, b| a.1 == b.1);
    Ok(frames.into_iter().map(|(_, path)| path).collect())
}

#[cfg(test)]
mod test_modules;
#[cfg(test)]
pub(crate) use test_modules::fake_api;
