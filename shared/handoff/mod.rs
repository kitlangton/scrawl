use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;

use crate::shared::model::ApparentBundle;

mod config_store;
mod config;
mod integration;
// Preserve the handoff API while each boundary owns its implementation.
pub use config::*;
pub use integration::*;
pub(crate) use config_store::ConfigPublicationUncertain;
pub(crate) mod capture_location;
pub use config_store::{default_config_path, set_config_value_in};
pub(crate) use config_store::{update_config_in, update_config_allowed_in};
#[cfg(test)]
pub(crate) use config_store::set_config_write_hook;
#[cfg(test)]
pub(crate) use config_store::set_config_directory_sync_failure;

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
const DEMO_VIDEO_REQUEST: &str = "Use make-it-apparent for complex visible changes; share verified video proof or explain the blocker.";
const CLI_INSTALL: &str =
    "Once per agent session, install or update to the latest Apparent CLI: https://github.com/tryapparent/apparent#agents";

/// Clipboard-only instructions precede the timeline so the last pasted lines identify
/// the user's request. A queued hosted identity, not sign-in alone, selects remote access.
pub(crate) fn clipboard_summary(bundle: &ApparentBundle, directory: &Path) -> String {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let hosted = crate::connect::hosted::handoff_pointer(directory);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let hosted: Option<String> = None;
    clipboard_summary_with_access(
        bundle,
        directory,
        experiment(AGENT_DEMO_VIDEO),
        hosted.as_deref(),
    )
}

fn clipboard_summary_with_access(
    bundle: &ApparentBundle,
    directory: &Path,
    request_video: bool,
    hosted: Option<&str>,
) -> String {
    let mut out = hosted.map(str::to_owned).unwrap_or_else(|| {
        format!(
            "apparent inspect {}\n",
            shell_quote(&directory.to_string_lossy())
        )
    });
    out.push_str(&format!("\n{CLI_INSTALL}\n\n"));
    if hosted.is_some() {
        out.push_str("Inspect relevant images before making changes.");
    } else {
        out.push_str(
            "Inspect contact-sheet.png and relevant frames/ in that folder before making changes.",
        );
        if let Some(crop) = &bundle.autocrop {
            out.push_str(&format!(" Uncropped images are in {}/.", crop.full_frames));
        }
    }
    if request_video {
        out.push(' ');
        out.push_str(DEMO_VIDEO_REQUEST);
    }
    out.push_str("\n\n");
    out.push_str(&crate::capture::engine::sketch::clipboard_markdown(
        bundle,
        directory,
        hosted.is_some(),
    ));
    out
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
