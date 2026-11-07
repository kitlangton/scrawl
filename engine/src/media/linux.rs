//! Bounded, file-descriptor-pinned Linux GStreamer decoder.

use apparent_capture_engine::linux::media::{IncompleteVideo, PinnedVideoReader};
use std::fs::File;

const SOURCE_EDGE_LIMIT: u32 = 16_384;
const SOURCE_BYTES_LIMIT: u64 = 256 * 1024 * 1024;

pub struct Decoded {
    pub actual_ms: u64,
    pub clamped_to_end: bool,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn bus_error_code(error: &anyhow::Error) -> Option<i32> {
    error.chain().find_map(|cause| {
        cause
            .downcast_ref::<gstreamer::glib::Error>()
            .map(gstreamer::glib::Error::code)
    })
}

fn shrink(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    edge: u32,
) -> Result<(u32, u32, Vec<u8>), &'static str> {
    let long = width.max(height);
    if long <= edge {
        return Ok((width, height, rgba));
    }
    let scale = f64::from(edge) / f64::from(long);
    let (ow, oh) = (
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    );
    let (sw, sh) = (width as usize, height as usize);
    let mut out = vec![
        0u8;
        (ow as usize)
            .checked_mul(oh as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or("limits_exceeded")?
    ];
    for y in 0..oh as usize {
        let (y0, y1) = (
            y * sh / oh as usize,
            ((y + 1) * sh / oh as usize).max(y * sh / oh as usize + 1),
        );
        for x in 0..ow as usize {
            let (x0, x1) = (
                x * sw / ow as usize,
                ((x + 1) * sw / ow as usize).max(x * sw / ow as usize + 1),
            );
            let mut sums = [0u64; 3];
            let mut count = 0u64;
            for sy in y0..y1 {
                for px in rgba[(sy * sw + x0) * 4..(sy * sw + x1) * 4].chunks_exact(4) {
                    for c in 0..3 {
                        sums[c] += u64::from(px[c]);
                    }
                    count += 1;
                }
            }
            let offset = (y * ow as usize + x) * 4;
            out[offset..offset + 4].copy_from_slice(&[
                (sums[0] / count) as u8,
                (sums[1] / count) as u8,
                (sums[2] / count) as u8,
                255,
            ]);
        }
    }
    Ok((ow, oh, out))
}

pub fn decode(
    source: &File,
    times: &[u64],
    max_edge: u32,
    max_pixels: u64,
    mut emit: impl FnMut(Decoded) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    if max_edge == 0 || max_edge > 4096 || max_pixels == 0 || max_pixels > 64_000_000 {
        return Err("limits_exceeded");
    }
    let metadata = source.metadata().map_err(|_| "source_incomplete")?;
    if metadata.len() == 0 {
        return Err("source_incomplete");
    }
    let reader = match PinnedVideoReader::open(source) {
        Ok(reader) => reader,
        Err(error) => {
            let code = if error.is::<IncompleteVideo>() {
                "source_incomplete"
            } else {
                "decode_failed"
            };
            super::job::diagnostic_failure("reader-open", code, bus_error_code(&error));
            return Err(code);
        }
    };
    let mut total = 0u64;
    for &requested in times {
        let clamped_to_end = requested >= reader.duration_ms();
        let frame = match reader.frame(requested, SOURCE_EDGE_LIMIT, SOURCE_BYTES_LIMIT) {
            Ok(frame) => frame,
            Err(error) => {
                let message = error.to_string();
                let code = if message.contains("limit") {
                    "limits_exceeded"
                } else if message.contains("No frame") || message.contains("frame") {
                    "source_incomplete"
                } else {
                    "decode_failed"
                };
                super::job::diagnostic_failure("frame-decode", code, bus_error_code(&error));
                return Err(code);
            }
        };
        let (actual_ms, w, h, raw) = (frame.t_ms, frame.width, frame.height, frame.rgba);
        let (width, height, rgba) = shrink(w, h, raw, max_edge)?;
        total = total
            .checked_add(u64::from(width) * u64::from(height))
            .ok_or("limits_exceeded")?;
        if total > max_pixels {
            return Err("limits_exceeded");
        }
        emit(Decoded {
            actual_ms,
            clamped_to_end,
            width,
            height,
            rgba,
        })?;
    }
    Ok(())
}
