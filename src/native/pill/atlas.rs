//! CoreText & CoreGraphics Retina glyph and icon atlas for the native Metal recording pill.

use std::collections::HashMap;
use std::ffi::c_void;

use core_foundation::{
    attributed_string::CFMutableAttributedString,
    base::{CFRange, TCFType},
    boolean::CFBoolean,
    string::CFString,
};
use core_graphics::{
    base::kCGImageAlphaPremultipliedLast,
    color_space::CGColorSpace,
    context::{CGContext, CGLineCap, CGLineJoin, CGTextDrawingMode},
};
use core_text::{
    font::{self, CTFont},
    line::CTLine,
    string_attributes::{kCTFontAttributeName, kCTForegroundColorFromContextAttributeName},
};
use metal::{MTLOrigin, MTLPixelFormat, MTLRegion, MTLSize, MTLTextureUsage, TextureDescriptor};
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};

use super::model::{HUD_TEXT, Metrics};
use crate::ui::SelectionMode;

pub const ATLAS_W: u32 = 1024;
pub const ATLAS_H: u32 = 512;

pub const WEIGHT_MEDIUM: f64 = 0.23;
pub const WEIGHT_SEMIBOLD: f64 = 0.30;

#[derive(Clone, Copy, Debug, Default)]
pub struct Sprite {
    /// Width and height in logical points.
    pub size_pt: [f32; 2],
    /// UV bounds `[u0, v0, u1, v1]` in `[0.0 .. 1.0]`.
    pub uv: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconKind {
    Area,
    Window,
    Screen,
    Iphone,
    Close,
    Check,
}

impl IconKind {
    pub const fn from_mode(mode: SelectionMode) -> Self {
        match mode {
            SelectionMode::Area => Self::Area,
            SelectionMode::Window => Self::Window,
            SelectionMode::Screen => Self::Screen,
            SelectionMode::Iphone => Self::Iphone,
        }
    }
}

/// Creates a `CTFont` from AppKit's `NSFont` (SF Pro, optionally with monospaced digits for the timer).
fn system_ct_font(size_px: f64, weight: f64, monospaced_digits: bool) -> Option<CTFont> {
    if let Some(cls) = AnyClass::get(c"NSFont") {
        let ptr: *mut AnyObject = unsafe {
            if monospaced_digits {
                msg_send![cls, monospacedDigitSystemFontOfSize: size_px, weight: weight]
            } else {
                msg_send![cls, systemFontOfSize: size_px, weight: weight]
            }
        };
        if !ptr.is_null() {
            let ct = unsafe { CTFont::wrap_under_get_rule(ptr.cast()) };
            return Some(ct);
        }
    }
    font::new_from_name(".AppleSystemUIFont", size_px).ok()
}

fn shape_ct_line(
    text: &str,
    size_px: f64,
    weight: f64,
    monospaced_digits: bool,
) -> Option<(CTFont, CTLine)> {
    let font = system_ct_font(size_px, weight, monospaced_digits)?;
    let mut attr = CFMutableAttributedString::new();
    attr.replace_str(&CFString::new(text), CFRange::init(0, 0));
    unsafe {
        attr.set_attribute(
            CFRange::init(0, attr.char_len()),
            kCTFontAttributeName,
            &font,
        );
        attr.set_attribute(
            CFRange::init(0, attr.char_len()),
            kCTForegroundColorFromContextAttributeName,
            &CFBoolean::true_value(),
        );
    }
    let line = CTLine::new_with_attributed_string(attr.as_concrete_TypeRef());
    Some((font, line))
}

/// Measures `text` in logical points at 13 pt (or `size_pt`).
pub fn measure_text_pt(text: &str, size_pt: f32, weight: f64, monospaced_digits: bool) -> f32 {
    if let Some((_, line)) = shape_ct_line(text, f64::from(size_pt), weight, monospaced_digits) {
        (line.get_typographic_bounds().width as f32).ceil()
    } else {
        (text.chars().count() as f32 * size_pt * 0.58).ceil()
    }
}

/// Measures all standard capsule strings with CoreText to build a `Metrics` table.
pub fn measure_metrics() -> Metrics {
    Metrics {
        area: measure_text_pt("Area", HUD_TEXT, WEIGHT_MEDIUM, false),
        window: measure_text_pt("Window", HUD_TEXT, WEIGHT_MEDIUM, false),
        screen: measure_text_pt("Screen", HUD_TEXT, WEIGHT_MEDIUM, false),
        iphone: measure_text_pt("iPhone", HUD_TEXT, WEIGHT_MEDIUM, false),
        record: measure_text_pt("Record", HUD_TEXT, WEIGHT_SEMIBOLD, false),
        cancel: measure_text_pt("Cancel", HUD_TEXT, WEIGHT_MEDIUM, false),
        finish: measure_text_pt("Finish", HUD_TEXT, WEIGHT_SEMIBOLD, false),
        processing: measure_text_pt("Processing…", HUD_TEXT, WEIGHT_SEMIBOLD, false),
        paste: measure_text_pt("Paste", HUD_TEXT, WEIGHT_SEMIBOLD, false),
        inspect: measure_text_pt("Inspect", HUD_TEXT, WEIGHT_MEDIUM, false),
        clock_000: measure_text_pt("0:00", HUD_TEXT, WEIGHT_SEMIBOLD, true),
        clock_0000: measure_text_pt("00:00", HUD_TEXT, WEIGHT_SEMIBOLD, true),
    }
}

/// Rasterizes a text label at `scale` into premultiplied white RGBA8 pixels.
fn rasterize_text(
    text: &str,
    size_pt: f32,
    weight: f64,
    monospaced_digits: bool,
    scale: f32,
) -> (u32, u32, [f32; 2], Vec<u8>) {
    let size_px = f64::from(size_pt * scale);
    let Some((font, line)) = shape_ct_line(text, size_px, weight, monospaced_digits) else {
        return (4, 4, [2.0, 2.0], vec![0; 64]);
    };
    let bounds = line.get_typographic_bounds();
    let pad_px = (2.0 * scale).ceil() as f64;
    let w_px = (bounds.width.ceil() + 2.0 * pad_px).max(4.0) as usize;
    let ascent = font.ascent();
    let descent = font.descent();
    let h_px = ((ascent + descent).ceil() + 2.0 * pad_px).max(4.0) as usize;

    let space = CGColorSpace::create_device_rgb();
    let mut ctx = CGContext::create_bitmap_context(
        None,
        w_px,
        h_px,
        8,
        w_px * 4,
        &space,
        kCGImageAlphaPremultipliedLast,
    );
    ctx.data().fill(0);
    ctx.set_should_antialias(true);
    ctx.set_allows_antialiasing(true);
    ctx.set_should_smooth_fonts(false);
    ctx.set_rgb_fill_color(1.0, 1.0, 1.0, 1.0);
    ctx.set_text_drawing_mode(CGTextDrawingMode::CGTextFill);
    ctx.set_text_position(pad_px, pad_px + descent);
    line.draw(&ctx);
    ctx.flush();

    let size_pt = [w_px as f32 / scale, h_px as f32 / scale];
    (w_px as u32, h_px as u32, size_pt, ctx.data().to_vec())
}

/// Rasterizes one of Scrawl's line icons into premultiplied white RGBA8 pixels at `scale`.
fn rasterize_icon(kind: IconKind, size_pt: f32, scale: f32) -> (u32, u32, [f32; 2], Vec<u8>) {
    let w_px = (size_pt * scale).round().max(8.0) as usize;
    let h_px = w_px;
    let space = CGColorSpace::create_device_rgb();
    let mut ctx = CGContext::create_bitmap_context(
        None,
        w_px,
        h_px,
        8,
        w_px * 4,
        &space,
        kCGImageAlphaPremultipliedLast,
    );
    ctx.data().fill(0);
    ctx.set_should_antialias(true);
    ctx.set_allows_antialiasing(true);
    ctx.set_rgb_stroke_color(1.0, 1.0, 1.0, 1.0);
    ctx.set_line_cap(CGLineCap::CGLineCapRound);
    ctx.set_line_join(CGLineJoin::CGLineJoinRound);

    // Map 24×24 SVG coordinates (top-left origin) to CoreGraphics context (bottom-left origin).
    let s = w_px as f64 / 24.0;
    let map = |x: f64, y: f64| -> (f64, f64) { (x * s, (24.0 - y) * s) };
    let line_seg = |ctx: &CGContext, x0: f64, y0: f64, x1: f64, y1: f64| {
        let (ax, ay) = map(x0, y0);
        let (bx, by) = map(x1, y1);
        ctx.begin_path();
        ctx.move_to_point(ax, ay);
        ctx.add_line_to_point(bx, by);
        ctx.stroke_path();
    };
    let rrect = |ctx: &CGContext, x: f64, y: f64, w: f64, h: f64, r: f64| {
        let (x0, y1) = map(x, y);
        let (x1, y0) = map(x + w, y + h);
        let cr = (r * s).min((x1 - x0) * 0.5).min((y1 - y0) * 0.5);
        let c = cr * 0.552_284_75;
        ctx.begin_path();
        ctx.move_to_point(x0 + cr, y0);
        ctx.add_line_to_point(x1 - cr, y0);
        ctx.add_curve_to_point(x1 - cr + c, y0, x1, y0 + cr - c, x1, y0 + cr);
        ctx.add_line_to_point(x1, y1 - cr);
        ctx.add_curve_to_point(x1, y1 - cr + c, x1 - cr + c, y1, x1 - cr, y1);
        ctx.add_line_to_point(x0 + cr, y1);
        ctx.add_curve_to_point(x0 + cr - c, y1, x0, y1 - cr + c, x0, y1 - cr);
        ctx.add_line_to_point(x0, y0 + cr);
        ctx.add_curve_to_point(x0, y0 + cr - c, x0 + cr - c, y0, x0 + cr, y0);
        ctx.close_path();
        ctx.stroke_path();
    };

    ctx.set_line_width(2.5 * s);
    match kind {
        IconKind::Area => {
            // Four corner brackets from assets/icons/area.svg
            let corner = |ctx: &CGContext, ax: f64, ay: f64, cx: f64, cy: f64, bx: f64, by: f64| {
                let (p0x, p0y) = map(ax, ay);
                let (pcx, pcy) = map(cx, cy);
                let (p1x, p1y) = map(bx, by);
                ctx.begin_path();
                ctx.move_to_point(p0x, p0y);
                ctx.add_line_to_point(pcx, pcy);
                ctx.add_line_to_point(p1x, p1y);
                ctx.stroke_path();
            };
            corner(&ctx, 3.0, 8.0, 3.0, 3.0, 8.0, 3.0);
            corner(&ctx, 16.0, 3.0, 21.0, 3.0, 21.0, 8.0);
            corner(&ctx, 21.0, 16.0, 21.0, 21.0, 16.0, 21.0);
            corner(&ctx, 8.0, 21.0, 3.0, 21.0, 3.0, 16.0);
        }
        IconKind::Window => {
            rrect(&ctx, 2.5, 4.0, 19.0, 16.0, 2.5);
            line_seg(&ctx, 2.5, 9.5, 21.5, 9.5);
        }
        IconKind::Screen => {
            rrect(&ctx, 2.0, 3.0, 20.0, 14.0, 2.0);
            line_seg(&ctx, 8.0, 21.0, 16.0, 21.0);
            line_seg(&ctx, 12.0, 17.0, 12.0, 21.0);
        }
        IconKind::Iphone => {
            rrect(&ctx, 5.5, 2.0, 13.0, 20.0, 3.0);
            line_seg(&ctx, 10.5, 6.0, 13.5, 6.0);
        }
        IconKind::Close => {
            ctx.set_line_width(2.4 * s);
            line_seg(&ctx, 6.5, 6.5, 17.5, 17.5);
            line_seg(&ctx, 17.5, 6.5, 6.5, 17.5);
        }
        IconKind::Check => {
            ctx.set_line_width(2.6 * s);
            let (ax, ay) = map(5.5, 12.5);
            let (bx, by) = map(10.0, 17.0);
            let (cx, cy) = map(18.5, 7.5);
            ctx.begin_path();
            ctx.move_to_point(ax, ay);
            ctx.add_line_to_point(bx, by);
            ctx.add_line_to_point(cx, cy);
            ctx.stroke_path();
        }
    }
    ctx.flush();
    (
        w_px as u32,
        h_px as u32,
        [size_pt, size_pt],
        ctx.data().to_vec(),
    )
}

pub struct GlyphAtlas {
    pub texture: metal::Texture,
    pub scale: f32,
    icons: HashMap<IconKind, Sprite>,
    labels: HashMap<String, Sprite>,
    cursor_x: u32,
    cursor_y: u32,
    row_h: u32,
    /// Reserved slot for the 1 Hz timer string (`y = 400`, `x = 0..240`).
    clock_text: String,
    pub clock_sprite: Sprite,
    /// Reserved slot for the completed label string (`y = 400`, `x = 256..960`).
    completed_text: String,
    pub completed_sprite: Sprite,
    /// Reserved rows for the Completed island's session chips (`y = 424, 468`).
    route_text: [String; 2],
    pub route_sprites: [Sprite; 2],
}

/// The tallest a changing label's sprite may be, in pixels, so the reserved rows don't overlap.
const SLOT_H: u32 = 42;

impl GlyphAtlas {
    pub fn new(device: &metal::DeviceRef, scale: f32) -> Self {
        let desc = TextureDescriptor::new();
        desc.set_pixel_format(MTLPixelFormat::RGBA8Unorm);
        desc.set_width(u64::from(ATLAS_W));
        desc.set_height(u64::from(ATLAS_H));
        desc.set_usage(MTLTextureUsage::ShaderRead);
        let texture = device.new_texture(&desc);

        // Zero-initialize the atlas texture.
        let zeros = vec![0_u8; (ATLAS_W * ATLAS_H * 4) as usize];
        texture.replace_region(
            MTLRegion {
                origin: MTLOrigin { x: 0, y: 0, z: 0 },
                size: MTLSize {
                    width: u64::from(ATLAS_W),
                    height: u64::from(ATLAS_H),
                    depth: 1,
                },
            },
            0,
            zeros.as_ptr().cast::<c_void>(),
            u64::from(ATLAS_W * 4),
        );

        let mut atlas = Self {
            texture,
            scale,
            icons: HashMap::new(),
            labels: HashMap::new(),
            cursor_x: 2,
            cursor_y: 2,
            row_h: 0,
            clock_text: String::new(),
            clock_sprite: Sprite::default(),
            completed_text: String::new(),
            completed_sprite: Sprite::default(),
            route_text: [String::new(), String::new()],
            route_sprites: [Sprite::default(); 2],
        };

        for (kind, size_pt) in [
            (IconKind::Area, 14.0),
            (IconKind::Window, 14.0),
            (IconKind::Screen, 14.0),
            (IconKind::Iphone, 14.0),
            (IconKind::Close, 12.0),
            (IconKind::Check, 13.0),
        ] {
            let (w, h, size_pt, bytes) = rasterize_icon(kind, size_pt, scale);
            let sprite = atlas.pack(w, h, size_pt, &bytes);
            atlas.icons.insert(kind, sprite);
        }

        for (text, size_pt, weight) in [
            ("Area", HUD_TEXT, WEIGHT_MEDIUM),
            ("Window", HUD_TEXT, WEIGHT_MEDIUM),
            ("Screen", HUD_TEXT, WEIGHT_MEDIUM),
            ("iPhone", HUD_TEXT, WEIGHT_MEDIUM),
            ("Record", HUD_TEXT, WEIGHT_SEMIBOLD),
            ("Cancel", HUD_TEXT, WEIGHT_MEDIUM),
            ("Finish", HUD_TEXT, WEIGHT_SEMIBOLD),
            ("Processing…", HUD_TEXT, WEIGHT_SEMIBOLD),
            ("Paste", HUD_TEXT, WEIGHT_SEMIBOLD),
            ("Inspect", HUD_TEXT, WEIGHT_MEDIUM),
        ] {
            let (w, h, size_pt, bytes) = rasterize_text(text, size_pt, weight, false, scale);
            let sprite = atlas.pack(w, h, size_pt, &bytes);
            atlas.labels.insert(text.to_owned(), sprite);
        }

        atlas.update_clock("0:00");
        atlas.update_completed("Copied");
        atlas
    }

    fn pack(&mut self, w: u32, h: u32, size_pt: [f32; 2], bytes: &[u8]) -> Sprite {
        if self.cursor_x + w + 2 > ATLAS_W {
            self.cursor_x = 2;
            self.cursor_y += self.row_h + 2;
            self.row_h = 0;
        }
        let x = self.cursor_x;
        let y = self.cursor_y;
        self.cursor_x += w + 2;
        self.row_h = self.row_h.max(h);
        self.upload_at(x, y, w, h, bytes);
        Sprite {
            size_pt,
            uv: [
                x as f32 / ATLAS_W as f32,
                y as f32 / ATLAS_H as f32,
                (x + w) as f32 / ATLAS_W as f32,
                (y + h) as f32 / ATLAS_H as f32,
            ],
        }
    }

    fn upload_at(&self, x: u32, y: u32, w: u32, h: u32, bytes: &[u8]) {
        if x + w > ATLAS_W || y + h > ATLAS_H || w == 0 || h == 0 {
            return;
        }
        self.texture.replace_region(
            MTLRegion {
                origin: MTLOrigin {
                    x: u64::from(x),
                    y: u64::from(y),
                    z: 0,
                },
                size: MTLSize {
                    width: u64::from(w),
                    height: u64::from(h),
                    depth: 1,
                },
            },
            0,
            bytes.as_ptr().cast::<c_void>(),
            u64::from(w * 4),
        );
    }

    pub fn icon(&self, kind: IconKind) -> Sprite {
        self.icons.get(&kind).copied().unwrap_or_default()
    }

    pub fn label(&self, text: &str) -> Sprite {
        self.labels.get(text).copied().unwrap_or_default()
    }

    /// Updates the monospaced timer sprite only when `clock` changes (1 Hz during recording).
    pub fn update_clock(&mut self, clock: &str) -> Sprite {
        if self.clock_text == clock {
            return self.clock_sprite;
        }
        self.clock_text.clear();
        self.clock_text.push_str(clock);
        let (w, h, size_pt, bytes) =
            rasterize_text(clock, HUD_TEXT, WEIGHT_SEMIBOLD, true, self.scale);
        let (x, y) = (4_u32, 380_u32);
        let w = w.min(240);
        let h = h.min(SLOT_H);
        self.upload_at(x, y, w, h, &bytes);
        self.clock_sprite = Sprite {
            size_pt,
            uv: [
                x as f32 / ATLAS_W as f32,
                y as f32 / ATLAS_H as f32,
                (x + w) as f32 / ATLAS_W as f32,
                (y + h) as f32 / ATLAS_H as f32,
            ],
        };
        self.clock_sprite
    }

    /// Updates the Completed lead label sprite only when `text` changes.
    pub fn update_completed(&mut self, text: &str) -> Sprite {
        if self.completed_text == text {
            return self.completed_sprite;
        }
        self.completed_text.clear();
        self.completed_text.push_str(text);
        let (w, h, size_pt, bytes) =
            rasterize_text(text, HUD_TEXT, WEIGHT_SEMIBOLD, false, self.scale);
        let (x, y) = (256_u32, 380_u32);
        let w = w.min(ATLAS_W - x - 4);
        let h = h.min(SLOT_H);
        self.upload_at(x, y, w, h, &bytes);
        self.completed_sprite = Sprite {
            size_pt,
            uv: [
                x as f32 / ATLAS_W as f32,
                y as f32 / ATLAS_H as f32,
                (x + w) as f32 / ATLAS_W as f32,
                (y + h) as f32 / ATLAS_H as f32,
            ],
        };
        self.completed_sprite
    }

    pub fn route_text(&self, index: usize) -> &str {
        self.route_text.get(index).map_or("", String::as_str)
    }

    /// Updates the `index`th session chip's label sprite only when `text` changes.
    pub fn update_route(&mut self, index: usize, text: &str) -> Sprite {
        let index = index.min(1);
        if self.route_text[index] == text {
            return self.route_sprites[index];
        }
        self.route_text[index] = text.to_owned();
        let (w, h, size_pt, bytes) =
            rasterize_text(text, HUD_TEXT, WEIGHT_MEDIUM, false, self.scale);
        let (x, y) = (4_u32, 424_u32 + 44 * index as u32);
        let w = w.min(ATLAS_W - x - 4);
        let h = h.min(SLOT_H);
        self.upload_at(x, y, w, h, &bytes);
        self.route_sprites[index] = Sprite {
            size_pt,
            uv: [
                x as f32 / ATLAS_W as f32,
                y as f32 / ATLAS_H as f32,
                (x + w) as f32 / ATLAS_W as f32,
                (y + h) as f32 / ATLAS_H as f32,
            ],
        };
        self.route_sprites[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coretext_metrics_and_monospaced_clock_digits_are_stable() {
        let m = measure_metrics();
        assert!(m.area >= 20.0 && m.area <= 45.0, "area={}", m.area);
        assert!(m.window > m.area, "window={} area={}", m.window, m.area);
        // Monospaced digits must give identical widths for 0:00, 1:11, and 4:59.
        let w_000 = measure_text_pt("0:00", HUD_TEXT, WEIGHT_SEMIBOLD, true);
        let w_111 = measure_text_pt("1:11", HUD_TEXT, WEIGHT_SEMIBOLD, true);
        let w_459 = measure_text_pt("4:59", HUD_TEXT, WEIGHT_SEMIBOLD, true);
        assert_eq!(w_000, w_111);
        assert_eq!(w_000, w_459);
    }

    #[test]
    fn icons_rasterize_with_non_empty_ink() {
        for kind in [
            IconKind::Area,
            IconKind::Window,
            IconKind::Screen,
            IconKind::Iphone,
            IconKind::Close,
            IconKind::Check,
        ] {
            let (w, h, _, rgba) = rasterize_icon(kind, 14.0, 2.0);
            assert_eq!(w, 28);
            assert_eq!(h, 28);
            assert!(
                rgba.chunks_exact(4).any(|px| px[3] > 180),
                "{kind:?} produced no opaque pixels"
            );
        }
    }
}
