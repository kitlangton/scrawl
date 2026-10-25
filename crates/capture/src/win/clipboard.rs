//! The Windows clipboard: Unicode text, and images as both the registered `PNG` format (what
//! browsers, Electron apps and Office read, alpha intact) and `CF_DIBV5` (what everything else
//! reads; Windows derives `CF_DIB` and `CF_BITMAP` from it).

use std::time::Duration;

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::Graphics::Gdi::{BI_BITFIELDS, BITMAPV5HEADER};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{CF_DIBV5, CF_UNICODETEXT};
use windows::core::PCWSTR;

use super::wide;

/// `LCS_sRGB`
const LCS_SRGB: u32 = 0x7352_4742;

/// Holds the clipboard open; closes it on drop.
struct Open;

impl Open {
    /// Another app may hold the clipboard for a moment; retry briefly.
    fn new() -> Result<Self> {
        for _ in 0..20 {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Ok(Self);
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        bail!("The clipboard is busy")
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

/// Copies `bytes` into movable global memory the clipboard takes over.
fn global(bytes: &[u8]) -> Result<HGLOBAL> {
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) }
        .context("Out of memory for the clipboard")?;
    let target = unsafe { GlobalLock(memory) };
    if target.is_null() {
        let _ = unsafe { GlobalFree(Some(memory)) };
        bail!("Could not lock clipboard memory");
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(memory);
    }
    Ok(memory)
}

fn put(format: u32, bytes: &[u8]) -> Result<()> {
    let memory = global(bytes)?;
    if unsafe { SetClipboardData(format, Some(HANDLE(memory.0))) }.is_err() {
        let _ = unsafe { GlobalFree(Some(memory)) };
        bail!("Could not put data on the clipboard");
    }
    Ok(())
}

/// NUL-terminated UTF-16LE, as `CF_UNICODETEXT` holds it.
fn unicode_text(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

pub fn set_text(text: &str) -> Result<()> {
    let _open = Open::new()?;
    unsafe { EmptyClipboard() }.context("Could not clear the clipboard")?;
    put(u32::from(CF_UNICODETEXT.0), &unicode_text(text))
}

/// A top-down 32-bit BGRA `BITMAPV5HEADER` DIB of an RGBA image.
fn dib_v5(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let header = BITMAPV5HEADER {
        bV5Size: std::mem::size_of::<BITMAPV5HEADER>() as u32,
        bV5Width: width as i32,
        // Positive height: bottom-up rows, which every reader handles.
        bV5Height: height as i32,
        bV5Planes: 1,
        bV5BitCount: 32,
        bV5Compression: BI_BITFIELDS,
        bV5SizeImage: width * height * 4,
        bV5RedMask: 0x00FF_0000,
        bV5GreenMask: 0x0000_FF00,
        bV5BlueMask: 0x0000_00FF,
        bV5AlphaMask: 0xFF00_0000,
        bV5CSType: LCS_SRGB,
        bV5Intent: 4, // LCS_GM_IMAGES
        ..Default::default()
    };
    let header_bytes = unsafe {
        std::slice::from_raw_parts(
            (&raw const header).cast::<u8>(),
            std::mem::size_of::<BITMAPV5HEADER>(),
        )
    };
    let mut out = Vec::with_capacity(header_bytes.len() + rgba.len());
    out.extend_from_slice(header_bytes);
    let row = width as usize * 4;
    for y in (0..height as usize).rev() {
        for px in rgba[y * row..(y + 1) * row].chunks_exact(4) {
            out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    out
}

fn decode_png(png_bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let mut buffer = vec![0; reader.output_buffer_size().context("PNG too large")?];
    let info = reader.next_frame(&mut buffer)?;
    let pixels = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => pixels
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => bail!("Unexpected indexed PNG"),
    };
    Ok((info.width, info.height, rgba))
}

pub fn set_png(png_bytes: &[u8]) -> Result<()> {
    set_png_and_text(png_bytes, None)
}

/// The image and, with `text`, the text too, so one Ctrl+V gives each app what it takes: a
/// browser or Electron composer attaches the image and inserts the text, a terminal the text.
pub fn set_png_and_text(png_bytes: &[u8], text: Option<&str>) -> Result<()> {
    let (width, height, rgba) = decode_png(png_bytes)?;
    let dib = dib_v5(width, height, &rgba);
    let png_name = wide("PNG");
    let png_format = unsafe { RegisterClipboardFormatW(PCWSTR(png_name.as_ptr())) };
    let _open = Open::new()?;
    unsafe { EmptyClipboard() }.context("Could not clear the clipboard")?;
    if png_format != 0 {
        put(png_format, png_bytes)?;
    }
    put(u32::from(CF_DIBV5.0), &dib)?;
    if let Some(text) = text {
        put(u32::from(CF_UNICODETEXT.0), &unicode_text(text))?;
    }
    Ok(())
}
