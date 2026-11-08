//! Text and PNG through X11 selections, except on KDE Wayland and Hyprland: neither reliably
//! bridges a background XWayland owner's selection to native apps. There we use wl-copy's
//! data-control protocol (bundled in the AppImage). Both backends keep an owner alive to serve
//! the data.

use std::io::{BufRead as _, Read as _, Write as _};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};

static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>) -> Result<T> {
    let mut slot = CLIPBOARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.is_none() {
        *slot = Some(arboard::Clipboard::new().map_err(|e| anyhow!("{e}"))?);
    }
    let clipboard = slot.as_mut().context("No clipboard")?;
    f(clipboard).map_err(|e| anyhow!("Clipboard: {e}"))
}

/// Desktops whose XWayland selections don't reach native Wayland apps.
fn native_wayland_clipboard() -> bool {
    (super::super::kde() || super::super::hyprland()) && super::super::wayland_session()
}

/// This is a preflight only. A failed offer after this check must not replace
/// clipboard contents: another process may have become the selection owner.
pub fn combined_offer_supported() -> bool {
    native_wayland_clipboard()
        && super::super::wayland_display().is_some_and(|display| helper_ready(&restore_helper(), &display))
}

fn helper_ready(helper: &std::path::Path, display: &std::ffi::OsStr) -> bool {
    if !helper.is_file() {
        return false;
    }
    Command::new("timeout")
        .args(["3s", "python3"])
        .arg(helper)
        .arg("--check")
        .env("GDK_BACKEND", "wayland")
        .env("WAYLAND_DISPLAY", display)
        .output()
        .is_ok_and(|check| check.status.success() && check.stdout == b"READY\n")
}

pub struct TextSnapshot {
    entries: Vec<(String, Vec<u8>)>,
}

fn plain_text_mime(types: &str) -> Result<&str> {
    let offered: Vec<_> = types.lines().collect();
    anyhow::ensure!(!offered.is_empty(), "Clipboard has no offered MIME types");
    offered
        .iter()
        .copied()
        .find(|mime| *mime == "text/plain;charset=utf-8")
        .or_else(|| offered.iter().copied().find(|mime| *mime == "text/plain"))
        .or_else(|| offered.first().copied())
        .context("Clipboard has no MIME type")
}

pub fn snapshot_text_wayland() -> Result<TextSnapshot> {
    anyhow::ensure!(
        native_wayland_clipboard(),
        "Draft clipboard restoration requires KDE or Hyprland Wayland"
    );
    let types = read_wayland(&["--list-types"], 8192)?;
    let types = std::str::from_utf8(&types).context("Invalid clipboard types")?;
    if types.is_empty() {
        return Ok(TextSnapshot {
            entries: Vec::new(),
        });
    }
    let _ = plain_text_mime(types)?;
    let mimes: Vec<_> = types.lines().collect();
    anyhow::ensure!(
        mimes.len() <= 32,
        "Clipboard has too many MIME types to preserve"
    );
    let mut entries = Vec::with_capacity(mimes.len());
    let mut size = 0usize;
    for mime in mimes {
        anyhow::ensure!(
            !mime.is_empty() && mime.len() <= 256,
            "Invalid clipboard MIME type"
        );
        let bytes = read_wayland(&["--no-newline", "--type", mime], 16 * 1024 * 1024 - size)?;
        size += bytes.len();
        entries.push((mime.to_owned(), bytes));
    }
    anyhow::ensure!(
        read_wayland(&["--list-types"], 8192)? == types.as_bytes(),
        "Clipboard changed during snapshot"
    );
    if entries.len() > 1 {
        let helper = restore_helper();
        anyhow::ensure!(
            helper.is_file(),
            "Rich clipboard restore helper is missing; original clipboard untouched"
        );
        let check = Command::new("timeout")
            .args(["3s", "python3"])
            .arg(&helper)
            .arg("--check")
            .env("GDK_BACKEND", "wayland")
            .env(
                "WAYLAND_DISPLAY",
                super::super::wayland_display().context("No Wayland display")?,
            )
            .output()
            .context("Checking rich clipboard restoration")?;
        anyhow::ensure!(
            check.status.success() && check.stdout == b"READY\n",
            "Rich clipboard restoration is unavailable; original clipboard untouched"
        );
    }
    Ok(TextSnapshot { entries })
}

pub fn restore_text_wayland(snapshot: TextSnapshot, offered: Option<(&str, &[u8])>) -> Result<()> {
    let Some((mime, expected)) = offered else {
        return Ok(());
    };
    anyhow::ensure!(
        still_ours(mime, expected)?,
        "Clipboard changed during handoff; leaving the newer content untouched"
    );
    match snapshot.entries.as_slice() {
        [] => clear_wayland(),
        [(mime, bytes)] => copy_wayland(mime, bytes),
        _ => copy_multi_wayland(&snapshot.entries),
    }
}

fn restore_helper() -> std::path::PathBuf {
    let adjacent = std::env::current_exe().ok().and_then(|exe| {
        exe.parent()
            .map(|dir| dir.join("apparent-restore-clipboard.py"))
    });
    if let Some(path) = adjacent.filter(|path| path.is_file()) {
        return path;
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capture/desktop/scripts/support/apparent-restore-clipboard.py")
}

fn copy_multi_wayland(entries: &[(String, Vec<u8>)]) -> Result<()> {
    let mut child = Command::new("python3")
        .arg(restore_helper())
        .env("GDK_BACKEND", "wayland")
        .env(
            "WAYLAND_DISPLAY",
            super::super::wayland_display().context("No Wayland display")?,
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Rich clipboard restore requires Python GTK4")?;
    let write = (|| -> Result<()> {
        let mut stdin = child.stdin.take().context("No restore input")?;
        stdin.write_all(&(entries.len() as u32).to_be_bytes())?;
        for (mime, bytes) in entries {
            stdin.write_all(&(mime.len() as u32).to_be_bytes())?;
            stdin.write_all(mime.as_bytes())?;
            stdin.write_all(&(bytes.len() as u32).to_be_bytes())?;
            stdin.write_all(bytes)?;
        }
        Ok(())
    })();
    if let Err(error) = write {
        let _ = child.kill();
        return Err(error);
    }
    let mut stdout = child.stdout.take().context("No restore confirmation")?;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let read = std::io::BufReader::new(&mut stdout).read_line(&mut line);
        let _ = sender.send((read, line));
    });
    let confirmation = receiver.recv_timeout(std::time::Duration::from_secs(4));
    if !matches!(confirmation, Ok((Ok(_), line)) if line == "READY\n") {
        let _ = child.kill();
        anyhow::bail!("Rich clipboard restore was not confirmed");
    }
    let offered = (|| -> Result<bool> {
        for (mime, bytes) in entries {
            if !still_ours(mime, bytes)? {
                return Ok(false);
            }
        }
        Ok(true)
    })();
    if !matches!(offered, Ok(true)) {
        let _ = child.kill();
        anyhow::bail!("Rich clipboard restore did not own the expected data");
    }
    Ok(())
}

fn clear_wayland() -> Result<()> {
    let status = Command::new("wl-copy")
        .arg("--clear")
        .env(
            "WAYLAND_DISPLAY",
            super::super::wayland_display().context("No Wayland display")?,
        )
        .status()
        .context("Clearing original empty clipboard")?;
    anyhow::ensure!(status.success(), "Could not restore empty clipboard");
    Ok(())
}

pub fn still_ours(mime: &str, expected: &[u8]) -> Result<bool> {
    let offered = read_wayland(&["--list-types"], 8192)?;
    let offered = std::str::from_utf8(&offered).context("Invalid clipboard types")?;
    if !offered.lines().any(|item| item == mime) {
        return Ok(false);
    }
    Ok(read_wayland(&["--no-newline", "--type", mime], 16 * 1024 * 1024)? == expected)
}

fn read_wayland(args: &[&str], limit: usize) -> Result<Vec<u8>> {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("wl-paste")))
        .filter(|path| path.is_file());
    let mut child = Command::new("timeout")
        .arg("3s")
        .arg(bundled.unwrap_or_else(|| "wl-paste".into()))
        .args(args)
        .env(
            "WAYLAND_DISPLAY",
            super::super::wayland_display().context("No Wayland display")?,
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Native Wayland clipboard needs wl-paste and timeout")?;
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()
        .context("No clipboard output")?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes);
    if bytes.len() > limit || read.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().context("Waiting for wl-paste")?;
    read.context("Reading clipboard")?;
    anyhow::ensure!(
        bytes.len() <= limit,
        "Clipboard too large to restore safely"
    );
    if args == ["--list-types"] && status.code() == Some(1) && bytes.is_empty() {
        return Ok(bytes);
    }
    anyhow::ensure!(
        status.success(),
        "Could not read clipboard (wl-paste: {status})"
    );
    Ok(bytes)
}

pub fn set_text(text: &str) -> Result<()> {
    if native_wayland_clipboard() {
        return copy_wayland("text/plain;charset=utf-8", text.as_bytes());
    }
    with(|c| c.set_text(text.to_owned()))
}

pub fn set_png(png_bytes: &[u8]) -> Result<()> {
    if native_wayland_clipboard() {
        return copy_wayland("image/png", png_bytes);
    }
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().context("Not a PNG")?;
    let mut buffer = vec![0; reader.output_buffer_size().context("PNG too large")?];
    let info = reader.next_frame(&mut buffer)?;
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buffer[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buffer[..info.buffer_size()]
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        other => anyhow::bail!("Unsupported PNG colour type {other:?}"),
    };
    with(|c| {
        c.set_image(arboard::ImageData {
            width: info.width as usize,
            height: info.height as usize,
            bytes: rgba.into(),
        })
    })
}

/// Offer both formats on one Wayland selection. arboard replaces its previous
/// format on X11, so the caller falls back to text there.
pub fn set_png_and_text(png_bytes: &[u8], text: &str) -> Result<()> {
    anyhow::ensure!(
        native_wayland_clipboard(),
        "Combined image and text clipboard is unavailable on this X11 session"
    );
    anyhow::ensure!(
        png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "Contact sheet is not a PNG"
    );
    copy_multi_wayland(&[
        ("image/png".into(), png_bytes.to_vec()),
        ("text/plain;charset=utf-8".into(), text.as_bytes().to_vec()),
    ])
}

fn copy_wayland(mime: &str, bytes: &[u8]) -> Result<()> {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("wl-copy")))
        .filter(|path| path.is_file());
    let mut child = Command::new(bundled.unwrap_or_else(|| "wl-copy".into()))
        .args(["--type", mime])
        .env(
            "WAYLAND_DISPLAY",
            super::super::wayland_display().context("No Wayland display")?,
        )
        .stdin(Stdio::piped())
        // wl-copy forks a clipboard owner after acquiring the selection. It must not keep
        // pipes to Apparent open; waiting for captured stdout/stderr would never finish.
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Native Wayland clipboard needs wl-copy (install wl-clipboard)")?;
    let written = child
        .stdin
        .take()
        .context("No clipboard input")?
        .write_all(bytes);
    let status = child.wait().context("Waiting for wl-copy")?;
    written.context("Writing the Wayland clipboard")?;
    anyhow::ensure!(status.success(), "wl-copy failed: {status}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_helper_is_detected_before_the_clipboard_is_touched() {
        let root = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
        std::fs::create_dir(&root).unwrap();
        let helper = root.join("helper.py");
        assert!(!helper_ready(&helper, std::ffi::OsStr::new("wayland-0")));
        std::fs::write(&helper, "import sys; sys.exit(2)\n").unwrap();
        assert!(!helper_ready(&helper, std::ffi::OsStr::new("wayland-0")));
        std::fs::write(&helper, "print('READY')\n").unwrap();
        assert!(helper_ready(&helper, std::ffi::OsStr::new("wayland-0")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn snapshot_policy_accepts_rich_and_image_clipboards_without_losing_their_types() {
        assert_eq!(
            plain_text_mime("text/plain;charset=utf-8\ntext/plain\n").unwrap(),
            "text/plain;charset=utf-8"
        );
        assert!(plain_text_mime("text/plain\nimage/png\n").is_ok());
        assert!(plain_text_mime("text/html\ntext/plain\n").is_ok());
        assert!(plain_text_mime("image/png\n").is_ok());
        assert!(plain_text_mime("").is_err());
    }

    #[test]
    fn isolated_wayland_restores_exact_text_after_png_offer() {
        if std::env::var_os("APPARENT_TEST_WAYLAND_CLIPBOARD").is_none() {
            return;
        }
        let original = b"synthetic original\n";
        copy_wayland("text/plain;charset=utf-8", original).unwrap();
        let snapshot = snapshot_text_wayland().unwrap();
        assert!(
            snapshot
                .entries
                .iter()
                .any(|(mime, bytes)| mime == "text/plain;charset=utf-8" && bytes == original)
        );
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[255, 0, 0, 255])
                .unwrap();
        }
        copy_wayland("image/png", &png).unwrap();
        assert!(
            snapshot_text_wayland()
                .unwrap()
                .entries
                .iter()
                .any(|(mime, bytes)| mime == "image/png" && bytes == &png)
        );
        restore_text_wayland(snapshot, Some(("image/png", &png))).unwrap();
        assert_eq!(
            read_wayland(
                &["--no-newline", "--type", "text/plain;charset=utf-8"],
                1024
            )
            .unwrap(),
            original
        );
        let snapshot = snapshot_text_wayland().unwrap();
        copy_wayland("image/png", &png).unwrap();
        copy_wayland("text/plain;charset=utf-8", b"newer synthetic clipboard").unwrap();
        assert!(restore_text_wayland(snapshot, Some(("image/png", &png))).is_err());
        assert_eq!(
            read_wayland(
                &["--no-newline", "--type", "text/plain;charset=utf-8"],
                1024
            )
            .unwrap(),
            b"newer synthetic clipboard"
        );
    }

    #[test]
    fn isolated_combined_offer_has_both_mime_types_and_exact_bytes() {
        if std::env::var_os("APPARENT_TEST_WAYLAND_CLIPBOARD").is_none() {
            return;
        }
        let previous = snapshot_text_wayland().unwrap();
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[7, 8, 9, 255])
                .unwrap();
        }
        let text = "test summary";
        set_png_and_text(&png, text).unwrap();
        assert!(still_ours("image/png", &png).unwrap());
        assert!(still_ours("text/plain;charset=utf-8", text.as_bytes()).unwrap());
        restore_text_wayland(
            previous,
            Some(("text/plain;charset=utf-8", text.as_bytes())),
        )
        .unwrap();
    }

    #[test]
    fn isolated_wayland_preserves_rich_clipboard_after_two_offers() {
        if std::env::var_os("APPARENT_TEST_RICH_WAYLAND_CLIPBOARD").is_none() {
            return;
        }
        let original = snapshot_text_wayland().unwrap();
        let html = b"<strong>synthetic</strong>";
        let plain = b"synthetic";
        let marker = b"apparent-rich-restore-test-marker";
        let image = b"synthetic-image";
        let mut offered: (&str, &[u8]) = ("text/html", html);
        let probe = (|| -> Result<()> {
            copy_multi_wayland(&[
                ("text/html".into(), html.to_vec()),
                ("text/plain".into(), plain.to_vec()),
            ])?;
            let rich = snapshot_text_wayland()?;
            anyhow::ensure!(
                rich.entries
                    .iter()
                    .any(|(mime, bytes)| mime == "text/html" && bytes == html),
                "HTML not offered: {:?}",
                rich.entries
                    .iter()
                    .map(|(mime, bytes)| (mime, bytes.len()))
                    .collect::<Vec<_>>()
            );
            anyhow::ensure!(
                rich.entries
                    .iter()
                    .any(|(mime, bytes)| mime == "text/plain" && bytes == plain),
                "Plain text not offered"
            );
            offered = ("image/png", image);
            copy_wayland("image/png", image)?;
            offered = ("text/plain;charset=utf-8", marker);
            copy_wayland("text/plain;charset=utf-8", marker)?;
            restore_text_wayland(rich, Some(("text/plain;charset=utf-8", marker)))?;
            offered = ("text/html", html);
            std::thread::sleep(std::time::Duration::from_secs(1));
            anyhow::ensure!(
                read_wayland(&["--no-newline", "--type", "text/html"], 1024)? == html,
                "HTML not restored"
            );
            anyhow::ensure!(
                read_wayland(&["--no-newline", "--type", "text/plain"], 1024)? == plain,
                "Plain text not restored"
            );
            Ok(())
        })();
        if still_ours(offered.0, offered.1).unwrap_or(false) {
            restore_text_wayland(original, Some(offered)).unwrap();
        }
        probe.unwrap();
    }
}
