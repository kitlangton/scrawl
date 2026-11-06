//! Policy-free pixel composition for an explicitly laid out contact sheet.
//!
//! Record supplies tile geometry and the exact layout hash. This module only
//! places visual pixels; it does not choose rows, add keyframe badges, or
//! reinterpret a sheet as a decoded video frame.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type Result<T> = std::result::Result<T, &'static str>;

macro_rules! ensure {
    ($condition:expr, $message:literal) => {
        if !$condition {
            return Err($message);
        }
    };
    ($condition:expr) => {
        if !$condition {
            return Err("invalid contact sheet input");
        }
    };
}

const MAX_EDGE: u32 = 4096;
const MAX_PIXELS: u64 = 64_000_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TileRect {
    pub request_index: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Layout {
    pub schema: String,
    pub width: u32,
    pub height: u32,
    pub background_rgb: [u8; 3],
    pub fit: String,
    pub sampling: String,
    pub clipping: String,
    pub tiles: Vec<TileRect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub struct ComposedSheet {
    pub image: RgbaImage,
    pub layout_hash: String,
}

impl Layout {
    /// Hash the normalized serde representation shared with the caller.
    pub fn sha256(&self) -> Result<String> {
        Ok(hex(&Sha256::digest(
            serde_json::to_vec(self).map_err(|_| "invalid layout")?,
        )))
    }

    pub fn validate(&self, frame_count: usize) -> Result<()> {
        ensure!(
            self.schema == "apparent.contact-sheet-layout/1"
                && self.fit == "stretch"
                && self.sampling == "legacy-bilinear"
                && self.clipping == "reject",
            "unsupported contact sheet pixel execution"
        );
        ensure!(
            self.width > 0
                && self.height > 0
                && self.width <= MAX_EDGE
                && self.height <= MAX_EDGE
                && u64::from(self.width) * u64::from(self.height) <= MAX_PIXELS,
            "contact sheet canvas exceeds bounds"
        );
        ensure!(!self.tiles.is_empty() && self.tiles.len() <= 48);
        for (index, tile) in self.tiles.iter().enumerate() {
            ensure!(
                tile.request_index > 0
                    && tile.request_index <= frame_count
                    && tile.width > 0
                    && tile.height > 0
                    && tile
                        .x
                        .checked_add(tile.width)
                        .is_some_and(|x| x <= self.width)
                    && tile
                        .y
                        .checked_add(tile.height)
                        .is_some_and(|y| y <= self.height),
                "invalid contact sheet tile geometry"
            );
            for previous in &self.tiles[..index] {
                ensure!(!overlap(tile, previous), "contact sheet tiles overlap");
            }
        }
        Ok(())
    }
}

/// Compose frame-slot pixels into explicit output rectangles. Record owns the
/// layout; this applies the bounded legacy stretch/bilinear pixel operation.
pub fn compose(
    frames: &[RgbaImage],
    layout: &Layout,
    expected_layout_hash: &str,
) -> Result<ComposedSheet> {
    layout.validate(frames.len())?;
    let layout_hash = layout.sha256()?;
    ensure!(
        layout_hash == expected_layout_hash,
        "contact sheet layout hash mismatch"
    );
    let mut canvas = RgbaImage {
        width: layout.width,
        height: layout.height,
        pixels: vec![0; layout.width as usize * layout.height as usize * 4],
    };
    for pixel in canvas.pixels.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[
            layout.background_rgb[0],
            layout.background_rgb[1],
            layout.background_rgb[2],
            255,
        ]);
    }
    for tile in &layout.tiles {
        let source = &frames[tile.request_index - 1];
        ensure!(
            source.width > 0
                && source.height > 0
                && source.width.max(source.height) <= 16_384
                && u64::from(source.width) * u64::from(source.height) <= 64_000_000
                && source.pixels.len() == source.width as usize * source.height as usize * 4,
            "invalid source frame pixels"
        );
        blit_stretch_legacy(&mut canvas, source, tile);
    }
    Ok(ComposedSheet {
        image: canvas,
        layout_hash,
    })
}

fn overlap(a: &TileRect, b: &TileRect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

fn blit_stretch_legacy(canvas: &mut RgbaImage, source: &RgbaImage, tile: &TileRect) {
    let inv_w = source.width as f32 / tile.width as f32;
    let inv_h = source.height as f32 / tile.height as f32;
    let supersample = inv_w > 1.35 || inv_h > 1.35;
    for dy in 0..tile.height {
        for dx in 0..tile.width {
            let cx = dx as f32 + 0.5;
            let cy = dy as f32 + 0.5;
            let rgba: [u8; 4] = if supersample {
                let sx0 = (cx - 0.25) * inv_w - 0.5;
                let sx1 = (cx + 0.25) * inv_w - 0.5;
                let sy0 = (cy - 0.25) * inv_h - 0.5;
                let sy1 = (cy + 0.25) * inv_h - 0.5;
                let samples = [
                    sample_bilinear(source, sx0, sy0),
                    sample_bilinear(source, sx1, sy0),
                    sample_bilinear(source, sx0, sy1),
                    sample_bilinear(source, sx1, sy1),
                ];
                std::array::from_fn(|channel| {
                    ((samples[0][channel]
                        + samples[1][channel]
                        + samples[2][channel]
                        + samples[3][channel])
                        * 0.25)
                        .round()
                        .clamp(0.0, 255.0) as u8
                })
            } else {
                let sample = sample_bilinear(source, cx * inv_w - 0.5, cy * inv_h - 0.5);
                std::array::from_fn(|channel| sample[channel].round().clamp(0.0, 255.0) as u8)
            };
            let dst = (((tile.y + dy) * canvas.width + tile.x + dx) * 4) as usize;
            if rgba[3] == 255 {
                canvas.pixels[dst..dst + 4].copy_from_slice(&rgba);
            } else if rgba[3] > 0 {
                let alpha = rgba[3] as f32 / 255.0;
                for channel in 0..3 {
                    canvas.pixels[dst + channel] = (rgba[channel] as f32 * alpha
                        + canvas.pixels[dst + channel] as f32 * (1.0 - alpha))
                        .round() as u8;
                }
                canvas.pixels[dst + 3] = 255;
            }
        }
    }
}

fn sample_bilinear(source: &RgbaImage, sx: f32, sy: f32) -> [f32; 4] {
    let sx = sx.clamp(0.0, source.width.saturating_sub(1) as f32);
    let sy = sy.clamp(0.0, source.height.saturating_sub(1) as f32);
    let x0 = sx.floor() as u32;
    let y0 = sy.floor() as u32;
    let x1 = (x0 + 1).min(source.width - 1);
    let y1 = (y0 + 1).min(source.height - 1);
    let tx = sx - x0 as f32;
    let ty = sy - y0 as f32;
    let weights = [
        (1.0 - tx) * (1.0 - ty),
        tx * (1.0 - ty),
        (1.0 - tx) * ty,
        tx * ty,
    ];
    let offsets = [
        ((y0 * source.width + x0) * 4) as usize,
        ((y0 * source.width + x1) * 4) as usize,
        ((y1 * source.width + x0) * 4) as usize,
        ((y1 * source.width + x1) * 4) as usize,
    ];
    let colors: [[u8; 4]; 4] = std::array::from_fn(|index| {
        source.pixels[offsets[index]..offsets[index] + 4]
            .try_into()
            .unwrap()
    });
    if colors.iter().all(|pixel| pixel[3] == 255) {
        return std::array::from_fn(|channel| {
            colors[0][channel] as f32 * weights[0]
                + colors[1][channel] as f32 * weights[1]
                + colors[2][channel] as f32 * weights[2]
                + colors[3][channel] as f32 * weights[3]
        });
    }
    let mut accum = [0.0f32; 4];
    for (pixel, weight) in colors.iter().zip(weights) {
        let alpha = pixel[3] as f32 / 255.0;
        for channel in 0..3 {
            accum[channel] += pixel[channel] as f32 * alpha * weight;
        }
        accum[3] += alpha * weight;
    }
    if accum[3] > 0.0 {
        [
            accum[0] / accum[3],
            accum[1] / accum[3],
            accum[2] / accum[3],
            accum[3] * 255.0,
        ]
    } else {
        [0.0; 4]
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, color: [u8; 4]) -> RgbaImage {
        let mut pixels = vec![0; width as usize * height as usize * 4];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&color);
        }
        RgbaImage {
            width,
            height,
            pixels,
        }
    }

    #[test]
    fn explicit_layout_composes_frame_slots_without_policy_or_captions() {
        let frames = [solid(2, 1, [255, 0, 0, 255]), solid(1, 2, [0, 0, 255, 255])];
        let layout = Layout {
            schema: "apparent.contact-sheet-layout/1".into(),
            width: 8,
            height: 4,
            background_rgb: [9, 8, 7],
            fit: "stretch".into(),
            sampling: "legacy-bilinear".into(),
            clipping: "reject".into(),
            tiles: vec![
                TileRect {
                    request_index: 1,
                    x: 1,
                    y: 0,
                    width: 3,
                    height: 4,
                },
                TileRect {
                    request_index: 2,
                    x: 4,
                    y: 0,
                    width: 4,
                    height: 4,
                },
            ],
        };
        let composed = compose(&frames, &layout, &layout.sha256().unwrap()).unwrap();
        assert_eq!((composed.image.width, composed.image.height), (8, 4));
        assert_eq!(composed.image.pixels[..4], [9, 8, 7, 255]);
        assert_eq!(
            composed.image.pixels[((1 * 8 + 1) * 4)..((1 * 8 + 1) * 4 + 4)],
            [255, 0, 0, 255]
        );
        assert_eq!(
            composed.image.pixels[((1 * 8 + 6) * 4)..((1 * 8 + 6) * 4 + 4)],
            [0, 0, 255, 255]
        );
        assert_eq!(composed.layout_hash, layout.sha256().unwrap());
    }

    #[test]
    fn rejects_layout_hash_alias_overlap_and_out_of_bounds_geometry() {
        let frames = [solid(1, 1, [1, 2, 3, 255])];
        let mut layout = Layout {
            schema: "apparent.contact-sheet-layout/1".into(),
            width: 4,
            height: 4,
            background_rgb: [0, 0, 0],
            fit: "stretch".into(),
            sampling: "legacy-bilinear".into(),
            clipping: "reject".into(),
            tiles: vec![TileRect {
                request_index: 1,
                x: 0,
                y: 0,
                width: 3,
                height: 3,
            }],
        };
        assert!(compose(&frames, &layout, &"0".repeat(64)).is_err());
        layout.tiles.push(TileRect {
            request_index: 1,
            x: 2,
            y: 2,
            width: 2,
            height: 2,
        });
        let hash = layout.sha256().unwrap();
        assert!(compose(&frames, &layout, &hash).is_err());
        layout.tiles.pop();
        layout.tiles[0].x = 3;
        assert!(compose(&frames, &layout, &layout.sha256().unwrap()).is_err());
    }

    #[test]
    fn stretches_and_source_over_composites_to_an_opaque_canvas() {
        let source = RgbaImage {
            width: 2,
            height: 1,
            pixels: vec![255, 0, 0, 255, 0, 0, 255, 128],
        };
        let layout = Layout {
            schema: "apparent.contact-sheet-layout/1".into(),
            width: 3,
            height: 1,
            background_rgb: [0, 255, 0],
            fit: "stretch".into(),
            sampling: "legacy-bilinear".into(),
            clipping: "reject".into(),
            tiles: vec![TileRect {
                request_index: 1,
                x: 0,
                y: 0,
                width: 3,
                height: 1,
            }],
        };
        let composed = compose(&[source], &layout, &layout.sha256().unwrap()).unwrap();
        assert_eq!(&composed.image.pixels[..4], &[255, 0, 0, 255]);
        assert_eq!(&composed.image.pixels[4..8], &[128, 63, 64, 255]);
        assert_eq!(&composed.image.pixels[8..12], &[0, 127, 128, 255]);
    }
}
