use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, ensure};

pub use scrawl_router::handoff::*;

/// Opens a file, folder or URL with its default app (Finder, Explorer, the browser).
pub fn open_path(target: &Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        crate::win::shell::open(target)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let status = Command::new("/usr/bin/open").arg(target).status()?;
        ensure!(status.success(), "open {} failed", target.display());
        Ok(())
    }
}

/// Shows a file selected in Finder or Explorer.
pub fn reveal_path(target: &Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        crate::win::shell::reveal(target)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Command::new("/usr/bin/open")
            .arg("-R")
            .arg(target)
            .spawn()?;
        Ok(())
    }
}

/// Host clipboard and paste operations. Scrawl's app uses `SystemClipboard`; an embedding host
/// such as Hex can supply its own implementation.
pub trait Clipboard: Send + Sync {
    fn copy_bundle(&self, bundle_dir: &Path, bundle: &crate::model::ScrawlBundle) -> Result<()>;
    fn copy_text(&self, text: &str) -> Result<()>;
    fn paste_bundle(
        &self,
        bundle_dir: &Path,
        bundle: &crate::model::ScrawlBundle,
        terminal: bool,
    ) -> Result<()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClipboard;

impl Clipboard for SystemClipboard {
    fn copy_bundle(&self, bundle_dir: &Path, bundle: &crate::model::ScrawlBundle) -> Result<()> {
        copy_loaded_bundle_to_clipboard(bundle_dir, bundle)
    }

    fn copy_text(&self, text: &str) -> Result<()> {
        copy_string_to_clipboard(text)
    }

    fn paste_bundle(
        &self,
        bundle_dir: &Path,
        bundle: &crate::model::ScrawlBundle,
        terminal: bool,
    ) -> Result<()> {
        paste_loaded_bundle_image_and_text(bundle_dir, bundle, terminal)
    }
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

#[cfg(target_os = "windows")]
pub fn copy_string_to_clipboard(content: &str) -> Result<()> {
    crate::win::clipboard::set_text(content)
}

#[cfg(target_os = "windows")]
pub fn copy_png_to_clipboard(png_bytes: &[u8]) -> Result<()> {
    crate::win::clipboard::set_png(png_bytes)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn copy_bundle_to_clipboard(bundle_dir: &Path) -> Result<()> {
    let (directory, bundle) = load_bundle(bundle_dir)?;
    SystemClipboard.copy_bundle(&directory, &bundle)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn copy_loaded_bundle_to_clipboard(
    directory: &Path,
    bundle: &crate::model::ScrawlBundle,
) -> Result<()> {
    let summary = crate::sketch::summary_markdown(bundle, directory);
    #[cfg(target_os = "windows")]
    if experiment("clipboardImage")
        && let Ok(image) = fs::read(directory.join(&bundle.contact_sheet))
        && crate::win::clipboard::set_png_and_text(&image, Some(&summary)).is_ok()
    {
        return Ok(());
    }
    copy_string_to_clipboard(&summary)
}

/// Pastes both the contact-sheet image and the transcript/keyframe text into the active app
/// as a two-step sequence (`⌘V` image, then `⌘V` text; Ctrl+V on Windows). If `click_target` is
/// provided (from dragging the Paste chip onto a window/input), clicks at that screen coordinate
/// first to focus it.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn paste_bundle_at_target(bundle_dir: &Path, click_target: Option<[f32; 2]>) -> Result<()> {
    if let Some([x, y]) = click_target {
        crate::capture::post_synthetic_click(f64::from(x), f64::from(y));
        std::thread::sleep(std::time::Duration::from_millis(65));
    } else {
        crate::capture::ensure_external_app_active();
    }
    paste_bundle_image_and_text(bundle_dir)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn paste_bundle_image_and_text(bundle_dir: &Path) -> Result<()> {
    let (directory, bundle) = load_bundle(bundle_dir)?;
    SystemClipboard.paste_bundle(&directory, &bundle, crate::capture::is_frontmost_terminal())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn paste_loaded_bundle_image_and_text(
    directory: &Path,
    bundle: &crate::model::ScrawlBundle,
    terminal: bool,
) -> Result<()> {
    let sheet_path = directory.join(&bundle.contact_sheet);
    let image = fs::read(&sheet_path).context("Could not read contact sheet")?;
    let summary = crate::sketch::summary_markdown(bundle, directory);

    crate::capture::wait_for_modifiers_released(350);

    if terminal {
        // In terminals (Ghostty / OpenCode TUI / Claude Code), pasting the image path first
        // resolves and attaches `[Image 1]`, and the second paste inserts the text summary.
        copy_string_to_clipboard(&sheet_path.display().to_string())?;
    } else {
        // In GUI apps (OpenCode Desktop, Slack, Linear, browsers), PNG-only attaches the image.
        copy_png_to_clipboard(&image)?;
    }
    std::thread::sleep(std::time::Duration::from_millis(20));
    crate::capture::post_synthetic_cmd_v();

    std::thread::sleep(std::time::Duration::from_millis(160));
    copy_string_to_clipboard(&summary)?;
    std::thread::sleep(std::time::Duration::from_millis(20));
    crate::capture::post_synthetic_cmd_v();
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn copy_string_to_clipboard(_content: &str) -> Result<()> {
    bail!("Image and text clipboard handoff requires macOS or Windows")
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn copy_loaded_bundle_to_clipboard(
    _directory: &Path,
    _bundle: &crate::model::ScrawlBundle,
) -> Result<()> {
    bail!("Image and text clipboard handoff requires macOS or Windows")
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn copy_bundle_to_clipboard(_bundle_dir: &Path) -> Result<()> {
    bail!("Image and text clipboard handoff requires macOS or Windows")
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn paste_loaded_bundle_image_and_text(
    _directory: &Path,
    _bundle: &crate::model::ScrawlBundle,
    _terminal: bool,
) -> Result<()> {
    bail!("Image and text paste handoff requires macOS or Windows")
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn paste_bundle_image_and_text(_bundle_dir: &Path) -> Result<()> {
    bail!("Image and text paste handoff requires macOS or Windows")
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn paste_bundle_at_target(_bundle_dir: &Path, _click_target: Option<[f32; 2]>) -> Result<()> {
    bail!("Image and text paste handoff requires macOS or Windows")
}

/// The command transcriber's program (`transcribeCommand`, `transcribeTimeoutSeconds`), read
/// on its own from the config file.
pub fn transcribe_command()
-> Result<crate::command_transcriber::Program, crate::command_transcriber::Unusable> {
    crate::command_transcriber::Program::load_from(&default_config_path())
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
            && let Ok(decoded) = crate::pipeline::movie_frames_at(&video, &times, offset_ms, 0)
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
                crate::render::save_png(frame, &out_path)?;
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
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn frame_fallback_uses_manifest_times_not_file_indices() {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "scrawl-handoff-cap-{}-{}-{}",
            std::process::id(),
            crate::pipeline::now_epoch_ms(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let path = root.join("example.scrawl");
        fs::create_dir_all(path.join("frames")).unwrap();
        let keyframes: Vec<_> = [0, 1_900, 2_500, 4_000, 5_100, 6_000]
            .into_iter()
            .enumerate()
            .map(|(index, t)| {
                let file = format!("frames/frame-{index:03}.png");
                fs::write(path.join(&file), b"fixture").unwrap();
                serde_json::json!({"index": index, "t": t, "reason": "initial", "file": file, "event": ""})
            })
            .collect();
        let bundle = serde_json::json!({
            "version": 1, "id": "example", "createdAtMs": 123, "durationMs": 7_000,
            "target": {"mode": "display", "displayId": 1,
                "frame": {"x": 0.0, "y": 0.0, "width": 100.0, "height": 100.0, "scale": 1.0}},
            "transcript": {"source": "none", "text": "", "segments": []},
            "strokes": [], "marks": [], "events": [], "keyframes": keyframes,
            "contactSheet": "contact-sheet.png"
        });
        fs::write(
            path.join("scrawl.json"),
            serde_json::to_vec(&bundle).unwrap(),
        )
        .unwrap();

        let frames = extract_frames_range(&path, 2_000, 5_000, 4).unwrap();
        let names: Vec<_> = frames
            .iter()
            .map(|frame| frame.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec![
                "frame-001.png",
                "frame-002.png",
                "frame-003.png",
                "frame-004.png"
            ]
        );
        assert!(
            extract_frames_range(&path, 10_000, 11_000, 4)
                .unwrap()
                .is_empty()
        );
        assert!(extract_frames_range(&path, 5_000, 2_000, 4).is_err());
        assert!(extract_frames_range(&path, 0, 2_000, 0).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
