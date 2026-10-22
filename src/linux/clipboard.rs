//! Text and PNG through X11 selections, except on KDE Wayland: KWin does not reliably bridge
//! a background XWayland owner's selection to native apps. There we use wl-copy's data-control
//! protocol (bundled in the AppImage). Both backends keep an owner alive to serve the data.

use std::io::Write as _;
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

pub fn set_text(text: &str) -> Result<()> {
    if super::kde() && super::wayland_session() {
        return copy_wayland("text/plain;charset=utf-8", text.as_bytes());
    }
    with(|c| c.set_text(text.to_owned()))
}

pub fn set_png(png_bytes: &[u8]) -> Result<()> {
    if super::kde() && super::wayland_session() {
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

fn copy_wayland(mime: &str, bytes: &[u8]) -> Result<()> {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("wl-copy")))
        .filter(|path| path.is_file());
    let mut child = Command::new(bundled.unwrap_or_else(|| "wl-copy".into()))
        .args(["--type", mime])
        .env(
            "WAYLAND_DISPLAY",
            super::wayland_display().context("No Wayland display")?,
        )
        .stdin(Stdio::piped())
        // wl-copy forks a clipboard owner after acquiring the selection. It must not keep
        // pipes to Scrawl open; waiting for captured stdout/stderr would never finish.
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
