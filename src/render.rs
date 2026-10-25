//! PNG output and deterministic, CPU-rendered review images.
use crate::model::*;
use crate::sketch::{FreehandOptions, freehand_outline, slice_laser_window, smooth_laser_points};
use std::path::Path;

type Color = [u8; 4];
const RED: Color = [255, 59, 48, 255];
const WHITE: Color = [255, 255, 255, 255];
const TEXT: Color = [242, 242, 242, 255];
const MUTED: Color = [150, 150, 150, 255];

pub fn encode_png(frame: &RgbaFrame) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        frame.width > 0 && frame.height > 0,
        "PNG dimensions must be nonzero"
    );
    let expected = (frame.width as usize)
        .checked_mul(frame.height as usize)
        .and_then(|n| n.checked_mul(4));
    anyhow::ensure!(
        expected == Some(frame.rgba.len()),
        "Invalid RGBA buffer length"
    );
    let mut bytes = Vec::with_capacity((frame.rgba.len() / 8).max(16_384));
    {
        let mut encoder = png::Encoder::new(&mut bytes, frame.width, frame.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder.set_filter(png::Filter::Sub);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&frame.rgba)?;
        writer.finish()?;
    }
    Ok(bytes)
}

pub fn save_png(frame: &RgbaFrame, path: &Path) -> anyhow::Result<()> {
    std::fs::write(path, encode_png(frame)?)?;
    Ok(())
}

pub fn decode_png<R: std::io::BufRead + std::io::Seek>(source: R) -> anyhow::Result<RgbaFrame> {
    let mut decoder = png::Decoder::new(source);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| anyhow::anyhow!("PNG is too large"))?;
    let mut data = vec![0; size];
    let info = reader.next_frame(&mut data)?;
    anyhow::ensure!(
        info.bit_depth == png::BitDepth::Eight,
        "Unsupported PNG bit depth"
    );
    let data = &data[..info.buffer_size()];
    let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(data),
        png::ColorType::Rgb => {
            for p in data.chunks_exact(3) {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for &v in data {
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for p in data.chunks_exact(2) {
                rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
            }
        }
        png::ColorType::Indexed => anyhow::bail!("PNG palette was not expanded"),
    }
    Ok(RgbaFrame::new(0, info.width, info.height, rgba))
}

/// Box-filtered copy at most `max_w` wide.
pub fn downscale(frame: &RgbaFrame, max_w: u32) -> RgbaFrame {
    let f = frame.width.div_ceil(max_w.max(1)).max(1);
    if f == 1 {
        return frame.clone();
    }
    let (w, h) = (frame.width / f, frame.height / f);
    let mut rgba = vec![0_u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0_u32; 4];
            for dy in 0..f {
                let row = ((y * f + dy) * frame.width + x * f) as usize * 4;
                for px in frame.rgba[row..row + f as usize * 4].as_chunks::<4>().0 {
                    for (a, v) in acc.iter_mut().zip(px) {
                        *a += u32::from(*v);
                    }
                }
            }
            let i = ((y * w + x) * 4) as usize;
            for c in 0..4 {
                rgba[i + c] = (acc[c] / (f * f)) as u8;
            }
        }
    }
    RgbaFrame::new(frame.t_ms, w, h, rgba)
}

fn solid(width: u32, height: u32, color: Color) -> RgbaFrame {
    let mut rgba = vec![0; width as usize * height as usize * 4];
    for p in rgba.chunks_exact_mut(4) {
        p.copy_from_slice(&color);
    }
    RgbaFrame::new(0, width, height, rgba)
}

/// Source-over, retaining straight alpha even when the input is transparent.
fn blend(frame: &mut RgbaFrame, x: i32, y: i32, color: Color, coverage: f32) {
    if x < 0 || y < 0 || x >= frame.width as i32 || y >= frame.height as i32 {
        return;
    }
    let i = (y as usize * frame.width as usize + x as usize) * 4;
    let Some(dst) = frame.rgba.get_mut(i..i + 4) else {
        return;
    };
    let a = (color[3] as f32 / 255.0 * coverage).clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let out = a + da * (1.0 - a);
    for c in 0..3 {
        dst[c] = ((color[c] as f32 * a + dst[c] as f32 * da * (1.0 - a)) / out).round() as u8;
    }
    dst[3] = (out * 255.0).round() as u8;
}

fn opacity(mut color: Color, alpha: f32) -> Color {
    color[3] = (color[3] as f32 * alpha.clamp(0.0, 1.0)).round() as u8;
    color
}

fn rounded_distance(x: f32, y: f32, rect: [f32; 4], radius: f32) -> f32 {
    let [left, top, width, height] = rect;
    let r = radius.max(0.0).min(width * 0.5).min(height * 0.5);
    let qx = (x - left - width * 0.5).abs() - width * 0.5 + r;
    let qy = (y - top - height * 0.5).abs() - height * 0.5 + r;
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

fn rounded(frame: &mut RgbaFrame, rect: [f32; 4], radius: f32, color: Color) {
    let [x, y, w, h] = rect;
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let left = (x.floor() as i32 - 1).max(0);
    let right = ((x + w).ceil() as i32 + 1).min(frame.width as i32);
    if left >= right {
        return;
    }
    let r = radius.max(0.0).min(w * 0.5).min(h * 0.5);
    for py in (y.floor() as i32 - 1).max(0)..((y + h).ceil() as i32 + 1).min(frame.height as i32) {
        // Only fully covered, opaque pixels can bypass source-over. Keep the
        // original distance/coverage calculation on all antialiased edges.
        let (start, end) =
            if color[3] == 255 && py >= y.ceil() as i32 && py < (y + h).floor() as i32 {
                let inset = if py >= (y + r).ceil() as i32 && py < (y + h - r).floor() as i32 {
                    0.0
                } else {
                    r
                };
                let start = ((x + inset).ceil() as i32).clamp(left, right);
                let end = ((x + w - inset).floor() as i32).clamp(start, right);
                (start, end)
            } else {
                (left, left)
            };
        let row = py as usize * frame.width as usize * 4;
        for pixel in
            frame.rgba[row + start as usize * 4..row + end as usize * 4].chunks_exact_mut(4)
        {
            pixel.copy_from_slice(&color);
        }
        for px in (left..start).chain(end..right) {
            let coverage = (0.5 - rounded_distance(px as f32 + 0.5, py as f32 + 0.5, rect, radius))
                .clamp(0.0, 1.0);
            blend(frame, px, py, color, coverage);
        }
    }
}

fn circle(frame: &mut RgbaFrame, center: [f32; 2], radius: f32, color: Color) {
    rounded(
        frame,
        [
            center[0] - radius,
            center[1] - radius,
            radius * 2.0,
            radius * 2.0,
        ],
        radius,
        color,
    );
}

fn ring(frame: &mut RgbaFrame, center: [f32; 2], radius: f32, thickness: f32, color: Color) {
    let [x, y] = center;
    let extent = radius + thickness * 0.5 + 1.0;
    for py in
        ((y - extent).floor() as i32).max(0)..((y + extent).ceil() as i32).min(frame.height as i32)
    {
        for px in ((x - extent).floor() as i32).max(0)
            ..((x + extent).ceil() as i32).min(frame.width as i32)
        {
            let d = (px as f32 + 0.5 - x).hypot(py as f32 + 0.5 - y);
            blend(
                frame,
                px,
                py,
                color,
                (thickness * 0.5 + 0.5 - (d - radius).abs()).clamp(0.0, 1.0),
            );
        }
    }
}

/// Four vertical subpixel scanlines, with exact horizontal span coverage.
fn polygon(frame: &mut RgbaFrame, points: &[[f32; 2]], color: Color) {
    if points.len() < 3 || points.iter().flatten().any(|v| !v.is_finite()) {
        return;
    }
    let top = points
        .iter()
        .map(|p| p[1])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as usize;
    let bottom = points
        .iter()
        .map(|p| p[1])
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(frame.height as f32)
        .max(0.0) as usize;
    let left = points
        .iter()
        .map(|p| p[0])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0)
        .min(frame.width as f32) as usize;
    let right = points
        .iter()
        .map(|p| p[0])
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .max(0.0)
        .min(frame.width as f32) as usize;
    if right <= left {
        return;
    }
    let mut coverage = vec![0.0f32; right - left];
    let mut crossings: Vec<(f32, i32)> = Vec::with_capacity(points.len());
    for y in top..bottom {
        coverage.fill(0.0);
        for sample in 0..4 {
            let sy = y as f32 + (sample as f32 + 0.5) / 4.0;
            crossings.clear();
            for i in 0..points.len() {
                let a = points[i];
                let b = points[(i + 1) % points.len()];
                if a[1] <= sy && b[1] > sy {
                    let x = a[0] + (sy - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                    crossings.push((x, 1));
                } else if b[1] <= sy && a[1] > sy {
                    let x = a[0] + (sy - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                    crossings.push((x, -1));
                }
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0_i32;
            let mut span_start = 0.0_f32;
            for &(x, dir) in &crossings {
                if winding == 0 {
                    span_start = x;
                }
                winding += dir;
                if winding == 0 {
                    let a = span_start.max(left as f32);
                    let b = x.min(right as f32);
                    if b > a {
                        for px in a.floor() as usize..(b.ceil() as usize).min(right) {
                            coverage[px - left] +=
                                ((px as f32 + 1.0).min(b) - (px as f32).max(a)).max(0.0) * 0.25;
                        }
                    }
                }
            }
        }
        for (x, &c) in coverage.iter().enumerate() {
            blend(frame, (left + x) as i32, y as i32, color, c);
        }
    }
}

fn line(frame: &mut RgbaFrame, a: [f32; 2], b: [f32; 2], r0: f32, r1: f32, color: Color) {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let length = dx.hypot(dy);
    if length > 0.001 {
        let nx = -dy / length;
        let ny = dx / length;
        polygon(
            frame,
            &[
                [a[0] + nx * r0, a[1] + ny * r0],
                [b[0] + nx * r1, b[1] + ny * r1],
                [b[0] - nx * r1, b[1] - ny * r1],
                [a[0] - nx * r0, a[1] - ny * r0],
            ],
            color,
        );
    }
    circle(frame, a, r0, color);
    circle(frame, b, r1, color);
}

#[cfg(test)]
pub fn render_annotated_keyframe(
    base: &RgbaFrame,
    keyframe: &Keyframe,
    strokes: &[Stroke],
    marks: &[Mark],
    events: &[InteractionEvent],
) -> RgbaFrame {
    let mut frame = base.clone();
    annotate_keyframe_in_place(&mut frame, keyframe, strokes, marks, events);
    frame
}

pub fn annotate_keyframe_in_place(
    frame: &mut RgbaFrame,
    keyframe: &Keyframe,
    strokes: &[Stroke],
    marks: &[Mark],
    events: &[InteractionEvent],
) {
    annotate_keyframe_marks_in_place(frame, keyframe, strokes, marks, events);
    stamp_keyframe_badge_in_place(frame, keyframe);
}

/// Sizes for a frame of `w × h` pixels: `(k, stroke_size, pin_scale)`.
/// Tuned for ~1280×800 screen frames; phone frames are taller and denser.
pub(crate) fn annotation_sizes(w: f32, h: f32) -> (f32, f32, f32) {
    let k = (w.min(h) / 800.0).max(w.max(h) / 1280.0).clamp(1.0, 2.0);
    let size = (w.min(h) * 0.0065).clamp(4.2, 6.0) * k;
    (k, size, (w.max(h) / 1280.0).clamp(0.75, 2.5))
}

pub(crate) fn is_long_press(label: Option<&str>) -> bool {
    label.is_some_and(|l| l == "long press" || l.starts_with("long press:"))
}

pub fn annotate_keyframe_marks_in_place(
    frame: &mut RgbaFrame,
    keyframe: &Keyframe,
    strokes: &[Stroke],
    marks: &[Mark],
    events: &[InteractionEvent],
) {
    frame.t_ms = keyframe.t;
    let w = frame.width as f32;
    let h = frame.height as f32;
    if w == 0.0 || h == 0.0 {
        return;
    }
    let (k, size, pin_scale) = annotation_sizes(w, h);
    // A mark keyframe shows every mark drawn on its screen. Flags carry no ink, so a flagged
    // keyframe still shows the strokes of its moment.
    let shown = crate::sketch::marks_on(keyframe, marks);
    for (index, stroke) in strokes.iter().enumerate() {
        if stroke.laser {
            continue;
        }
        let belongs = shown.iter().any(|m| m.strokes.contains(&index));
        let active = belongs
            || (shown.is_empty()
                && stroke.t0 <= keyframe.t
                && keyframe.t.saturating_sub(stroke.t1) <= MARK_GAP_MS);
        if !active {
            continue;
        }
        let points: Vec<[f32; 2]> = stroke
            .timed_points()
            .into_iter()
            .filter(|&(_, _, t)| belongs || t <= keyframe.t)
            .map(|(x, y, _)| [x * w, y * h])
            .collect();
        if let Some(number) = stroke.pin {
            if let Some(&point) = points.first() {
                draw_pin(frame, point, number, pin_scale);
            }
        } else if !points.is_empty() {
            let outline = freehand_outline(
                &points,
                &FreehandOptions {
                    size,
                    ..Default::default()
                },
            );
            polygon(frame, &outline, RED);
        }
    }
    for stroke in strokes.iter().filter(|s| s.laser) {
        draw_laser(frame, stroke, keyframe.t);
    }
    // Phone swipes that ended just before this moment (or produced it) leave a trail.
    for event in events {
        if let InteractionEvent::Drag {
            t,
            t1,
            x0,
            y0,
            x1,
            y1,
            path,
            button,
            label,
        } = event
            && button.as_deref() == Some("touch")
            && !is_long_press(label.as_deref())
            && *t <= keyframe.t
            && keyframe.t.saturating_sub(*t1) <= 900
        {
            let points: Vec<[f32; 2]> = if path.len() >= 2 {
                path.iter().map(|[x, y]| [x * w, y * h]).collect()
            } else {
                vec![[x0 * w, y0 * h], [x1 * w, y1 * h]]
            };
            draw_touch_trail(frame, &points, k);
        }
    }
    let click = events
        .iter()
        .filter_map(|event| match event {
            InteractionEvent::Click { t, x, y, .. } if t.abs_diff(keyframe.t) <= 600 => {
                Some((t.abs_diff(keyframe.t), [*x, *y]))
            }
            InteractionEvent::Drag {
                t,
                t1,
                x0,
                y0,
                label,
                ..
            } if is_long_press(label.as_deref())
                && ((*t <= keyframe.t && keyframe.t.saturating_sub(*t1) <= 600)
                    || t.abs_diff(keyframe.t) <= 600) =>
            {
                Some((t1.abs_diff(keyframe.t), [*x0, *y0]))
            }
            _ => None,
        })
        .min_by_key(|(delta, _)| *delta)
        .map(|(_, point)| point);
    if let Some([cx, cy]) = keyframe
        .cursor
        .or(click)
        .filter(|p| p.iter().all(|v| v.is_finite()))
    {
        let show_ripple = keyframe.reason == KeyframeReason::Click
            || (click.is_some() && keyframe.reason != KeyframeReason::Laser);
        if show_ripple {
            draw_click_ripple(frame, [cx * w, cy * h], k);
        }
    }

    let in_top_left = |[x, y]: [f32; 2]| x < 0.30 && y < 0.18;
    let in_top_right = |[x, y]: [f32; 2]| x > 0.70 && y < 0.18;
    let focus_pt = keyframe.cursor.or(click);
    let stroke_in_tl = strokes.iter().enumerate().any(|(idx, s)| {
        !s.laser
            && (shown.iter().any(|m| m.strokes.contains(&idx))
                || (shown.is_empty()
                    && s.t0 <= keyframe.t
                    && keyframe.t.saturating_sub(s.t1) <= MARK_GAP_MS))
            && s.timed_points()
                .into_iter()
                .any(|(x, y, t)| t <= keyframe.t && in_top_left([x, y]))
    });
    let busy_top_left = focus_pt.is_some_and(in_top_left) || stroke_in_tl;
    let busy_top_right = focus_pt.is_some_and(in_top_right);
    frame.badge_top_right = busy_top_left && !busy_top_right;
}

pub fn stamp_keyframe_badge_in_place(frame: &mut RgbaFrame, keyframe: &Keyframe) {
    let w = frame.width as f32;
    let h = frame.height as f32;
    if w == 0.0 || h == 0.0 {
        return;
    }
    let badge_k = (h / 480.0).min(w / 280.0).max(0.45);
    let dodge = frame.badge_top_right;
    draw_keyframe_badge(frame, [0.0, 0.0, w, h], keyframe, badge_k, dodge);
}

pub fn stamp_keyframe_badges(frames: &mut [(Keyframe, RgbaFrame)]) {
    for (kf, frame) in frames {
        stamp_keyframe_badge_in_place(frame, kf);
    }
}

/// Where the vertical middle of a digit sits below the top of a text mask, as a fraction of the
/// font size: Helvetica Neue through CoreText, Arial through `ab_glyph`, the 5×7 fallback.
const DIGIT_MIDDLE: f32 = if cfg!(target_os = "macos") {
    0.59
} else if cfg!(target_os = "windows") {
    0.55
} else {
    0.5
};

/// The keyframe's number, bold black on a small white rounded square with a thin dark ring,
/// inset from the top-left corner of `rect` (top-right when ink, a click or the cursor is there).
/// `k` scales it: 1 is a 24 px square. A shaped container keeps a model from reading it as a
/// number on the screen, and summary.md carries the time.
fn draw_keyframe_badge(
    dst: &mut RgbaFrame,
    rect: [f32; 4],
    keyframe: &Keyframe,
    k: f32,
    dodge_top_right: bool,
) {
    let [rx, ry, rw, rh] = rect;
    if rw <= 0.0 || rh <= 0.0 {
        return;
    }
    let k = k.max(0.45);
    let label = keyframe.index.to_string();
    let size = 16.0 * k;
    let text_w = text_width(&label, size, true);
    let h = size * 1.5;
    let w = (text_w + size * 0.62).max(h);
    let inset = 5.0 * k;
    let ring = (1.4 * k).max(1.0);
    let radius = size * 0.3;
    let dodge = dodge_top_right || keyframe.cursor.is_some_and(|[x, y]| x < 0.30 && y < 0.18);
    let x = if dodge {
        (rx + rw - w - inset).max(rx + inset)
    } else {
        rx + inset
    };
    let y = ry + inset;
    rounded(
        dst,
        [x - ring, y - ring, w + 2.0 * ring, h + 2.0 * ring],
        radius + ring,
        CHIP_RING,
    );
    rounded(dst, [x, y, w, h], radius, CHIP_FILL);
    draw_text(
        dst,
        &label,
        x + (w - text_w) * 0.5,
        y + h * 0.5 - size * DIGIT_MIDDLE,
        size,
        CHIP_INK,
        true,
    );
}

const CHIP_RING: Color = [16, 17, 20, 230];
const CHIP_FILL: Color = [250, 250, 252, 255];
const CHIP_INK: Color = [16, 17, 20, 255];

fn draw_pin(frame: &mut RgbaFrame, point: [f32; 2], number: u32, scale: f32) {
    let label = number.to_string();
    let radius = 13.0 * scale;
    circle(frame, point, radius + 2.5 * scale, WHITE);
    circle(frame, point, radius, RED);
    let mut size = 16.0 * scale;
    let width = text_width(&label, size, true);
    if width > radius * 1.65 {
        size *= radius * 1.65 / width;
    }
    let width = text_width(&label, size, true);
    draw_text(
        frame,
        &label,
        point[0] - width * 0.5,
        point[1] - size * 0.58,
        size,
        WHITE,
        true,
    );
}

fn draw_laser(frame: &mut RgbaFrame, stroke: &Stroke, time: u64) {
    let fw = frame.width as f32;
    let fh = frame.height as f32;
    let raw_points = slice_laser_window(&stroke.timed_points(), time, LASER_MS, |x, y| {
        [x * fw, y * fh]
    });
    if raw_points.is_empty() {
        return;
    }
    let points = smooth_laser_points(&raw_points);
    // Subtle, realistic optical laser trail: soft atmospheric scatter + delicate ruby beam + incandescent core.
    for (radius, alpha, power, color) in [
        (7.5, 0.08, 1.1, [255, 22, 38, 255]),
        (4.2, 0.22, 1.2, [255, 36, 48, 255]),
        (2.1, 0.78, 1.3, RED),
        (0.95, 0.92, 1.5, [255, 245, 245, 255]),
    ] {
        if points.len() == 1 {
            circle(
                frame,
                points[0].0,
                radius,
                opacity(color, alpha * points[0].1.powf(power)),
            );
        }
        for pair in points.windows(2) {
            let (a, fa) = pair[0];
            let (b, fb) = pair[1];
            let w0 = radius * fa.powf(0.70);
            let w1 = radius * fb.powf(0.70);
            let seg_alpha = alpha * ((fa + fb) * 0.5).powf(power);
            line(frame, a, b, w0, w1, opacity(color, seg_alpha));
        }
    }
    // Subtle pinpoint diode head at the tip of the beam.
    if let Some(&(tip, tip_age)) = points.last()
        && tip_age > 0.20
    {
        for (r, a, col) in [
            (9.5, 0.14 * tip_age, [255, 20, 36, 255]),
            (5.5, 0.34 * tip_age, [255, 36, 48, 255]),
            (3.0, 0.88 * tip_age, RED),
            (1.5, tip_age, WHITE),
        ] {
            circle(frame, tip, r, opacity(col, a));
        }
    }
}

const CLICK_BLUE: Color = [0, 136, 255, 255];

/// Where a click landed, without hiding what's under it: a thin translucent ring with a hairline
/// white edge (so it reads on dark and on blue) and a small centre dot. No fill.
fn draw_click_ripple(frame: &mut RgbaFrame, p: [f32; 2], k: f32) {
    let r = 9.0 * k;
    ring(frame, p, r + 1.1 * k, 0.9 * k, opacity(WHITE, 0.4));
    ring(frame, p, r, 1.3 * k, opacity(CLICK_BLUE, 0.8));
    circle(frame, p, 2.0 * k, opacity(WHITE, 0.5));
    circle(frame, p, 1.3 * k, opacity(CLICK_BLUE, 0.9));
}

/// A finger's path: a thin translucent blue line from a small dot to a slightly larger one where
/// it lifted (a tap's ring there, when the keyframe has one, stays the only ring).
fn draw_touch_trail(frame: &mut RgbaFrame, points: &[[f32; 2]], k: f32) {
    let (Some(&start), Some(&end)) = (points.first(), points.last()) else {
        return;
    };
    polyline(frame, points, 1.5 * k, opacity(WHITE, 0.22));
    polyline(frame, points, 0.8 * k, opacity(CLICK_BLUE, 0.7));
    for (point, r) in [(start, 1.3 * k), (end, 2.2 * k)] {
        circle(frame, point, r + 0.7 * k, opacity(WHITE, 0.5));
        circle(frame, point, r, opacity(CLICK_BLUE, 0.9));
    }
}

/// A round-capped polyline of half-width `radius`, blended once per pixel so a translucent line
/// doesn't darken where its segments meet.
fn polyline(frame: &mut RgbaFrame, points: &[[f32; 2]], radius: f32, color: Color) {
    if points.len() < 2 || points.iter().flatten().any(|v| !v.is_finite()) {
        return;
    }
    let reach = radius + 1.0;
    let [bx0, by0, bx1, by1] = crate::sketch::bounds(points);
    let x0 = ((bx0 - reach).floor() as i32).max(0);
    let x1 = ((bx1 + reach).ceil() as i32).min(frame.width as i32);
    let y0 = ((by0 - reach).floor() as i32).max(0);
    let y1 = ((by1 + reach).ceil() as i32).min(frame.height as i32);
    for py in y0..y1 {
        let y = py as f32 + 0.5;
        for px in x0..x1 {
            let x = px as f32 + 0.5;
            let mut d = f32::INFINITY;
            for seg in points.windows(2) {
                let (a, b) = (seg[0], seg[1]);
                if x < a[0].min(b[0]) - reach
                    || x > a[0].max(b[0]) + reach
                    || y < a[1].min(b[1]) - reach
                    || y > a[1].max(b[1]) + reach
                {
                    continue;
                }
                let ab = [b[0] - a[0], b[1] - a[1]];
                let len2 = (ab[0] * ab[0] + ab[1] * ab[1]).max(1e-6);
                let t = (((x - a[0]) * ab[0] + (y - a[1]) * ab[1]) / len2).clamp(0.0, 1.0);
                d = d.min((x - a[0] - ab[0] * t).hypot(y - a[1] - ab[1] * t));
            }
            let coverage = (radius + 0.5 - d).clamp(0.0, 1.0);
            if coverage > 0.0 {
                blend(frame, px, py, color, coverage);
            }
        }
    }
}

const SHEET_PAD: u32 = 6;
const SHEET_GAP: u32 = 6;
const SHEET_BG: Color = [28, 29, 34, 255];
/// One chip size on every tile, about 24 px square with its ring: the smallest that models still
/// read on a sheet shown at half size (20 px wasn't).
const SHEET_CHIP_K: f32 = 0.9;
const SHEET_MAX_EDGE: f32 = 1600.0;
const SHEET_MAX_PIXELS: f32 = 1_560_000.0;

/// Partition `aspects` (in chronological order) into justified rows of 1..=6 frames so that
/// per-frame displayed areas are balanced across wide and tall crops and the overall sheet aspect
/// ratio sits near the vision-model sweet spot.
fn plan_contact_sheet_rows(aspects: &[f32]) -> Vec<std::ops::Range<usize>> {
    let n = aspects.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![0..1];
    }
    if n == 2 {
        return if aspects[0] + aspects[1] > 3.6 {
            vec![0..1, 1..2]
        } else {
            vec![0..2]
        };
    }

    let avg_a = aspects.iter().sum::<f32>() / n as f32;
    let total_a = aspects.iter().sum::<f32>();
    let target_sheet = if avg_a < 0.7 { 1.05 } else { 1.25 };
    let target_row_a = (total_a * target_sheet).sqrt().max(avg_a);
    let max_cols = 6usize;

    let mut best_rows = Vec::new();
    let mut best_cost = f32::INFINITY;
    let mut current = Vec::with_capacity(n);

    fn search(
        idx: usize,
        n: usize,
        max_cols: usize,
        aspects: &[f32],
        avg_a: f32,
        target_sheet: f32,
        target_row_a: f32,
        current: &mut Vec<std::ops::Range<usize>>,
        best_cost: &mut f32,
        best_rows: &mut Vec<std::ops::Range<usize>>,
    ) {
        if idx == n {
            let r_count = current.len() as f32;
            let mut row_aspects = [0.0f32; MAX_KEYFRAMES];
            let mut inv_sum = 0.0f32;
            let mut log_row_sum = 0.0f32;
            for (r_idx, row) in current.iter().enumerate() {
                let a: f32 = aspects[row.clone()].iter().sum();
                row_aspects[r_idx] = a;
                inv_sum += 1.0 / a;
                log_row_sum += a.ln();
            }
            let sheet_aspect = 1.0 / inv_sum;
            let sheet_pen = (sheet_aspect / target_sheet).ln().powi(2) * 1.8;

            // Relative displayed area of frame `i` in row `r` is `a_i / A_r^2`.
            let mut areas = [0.0f32; MAX_KEYFRAMES];
            let mut log_areas = [0.0f32; MAX_KEYFRAMES];
            let mut log_area_sum = 0.0f32;
            for (r_idx, row) in current.iter().enumerate() {
                let a_r = row_aspects[r_idx];
                let inv_a_r2 = 1.0 / (a_r * a_r);
                for i in row.clone() {
                    let ar = aspects[i] * inv_a_r2;
                    areas[i] = ar;
                    let la = ar.ln();
                    log_areas[i] = la;
                    log_area_sum += la;
                }
            }
            let mean_log_area = log_area_sum / n as f32;
            let area_var = log_areas[..n]
                .iter()
                .map(|&la| (la - mean_log_area).powi(2))
                .sum::<f32>()
                / n as f32;

            let mut sorted_areas = areas;
            sorted_areas[..n].sort_by(f32::total_cmp);
            let median_area = (sorted_areas[(n - 1) / 2] + sorted_areas[n / 2]).max(1e-6) * 0.5;
            let mut under_pen = 0.0f32;
            for &ar in &areas[..n] {
                let ratio = ar / median_area;
                if ratio < 0.58 {
                    under_pen += ((0.58 - ratio) / 0.58).powi(2) * 18.0;
                }
                if ratio > 2.3 {
                    under_pen += ((ratio - 2.3) / 2.3).powi(2) * 4.0;
                }
            }

            let mean_log_a = log_row_sum / r_count;
            let mut row_var = 0.0f32;
            let mut row_pen = 0.0f32;
            let mut cramp_pen = 0.0f32;
            if current.len() > 4 {
                cramp_pen += (current.len() - 4) as f32 * 2.5;
            }
            let mut tie = 0.0f32;

            for (r_idx, row) in current.iter().enumerate() {
                let a_r = row_aspects[r_idx];
                let dl = a_r.ln() - mean_log_a;
                row_var += dl * dl;
                let dt = (a_r / target_row_a).ln();
                row_pen += dt * dt;

                for &a_i in &aspects[row.clone()] {
                    let w_frac = (a_i / a_r) * sheet_aspect.min(1.0);
                    if w_frac < 0.16 {
                        cramp_pen += ((0.16 - w_frac) / 0.16).powi(2) * 8.0;
                    }
                }
                let cols = row.len();
                if cols > 4 && avg_a > 0.75 {
                    cramp_pen += (cols - 4) as f32 * 2.0;
                }
                if cols > 3 && avg_a > 1.15 {
                    cramp_pen += (cols - 3) as f32 * 2.0;
                }
                tie += r_idx as f32 * 1e-4 * cols as f32;
            }
            let cost = sheet_pen
                + area_var * 8.0
                + under_pen
                + (row_var / r_count) * 0.6
                + (row_pen / r_count) * 0.4
                + cramp_pen
                + tie;
            if cost < *best_cost {
                *best_cost = cost;
                *best_rows = current.clone();
            }
            return;
        }

        let limit = max_cols.min(n - idx);
        for k in 1..=limit {
            if k == 1 && n >= 3 && aspects[idx] < 1.85 {
                continue;
            }
            current.push(idx..idx + k);
            search(
                idx + k,
                n,
                max_cols,
                aspects,
                avg_a,
                target_sheet,
                target_row_a,
                current,
                best_cost,
                best_rows,
            );
            current.pop();
        }
    }

    search(
        0,
        n,
        max_cols,
        aspects,
        avg_a,
        target_sheet,
        target_row_a,
        &mut current,
        &mut best_cost,
        &mut best_rows,
    );
    best_rows
}

pub fn render_contact_sheet_for_duration(
    duration_ms: u64,
    annotated_frames: &[(Keyframe, RgbaFrame)],
) -> RgbaFrame {
    let frames = &annotated_frames[..annotated_frames.len().min(MAX_KEYFRAMES)];
    if frames.is_empty() {
        let mut sheet = solid(640, 360, SHEET_BG);
        sheet.t_ms = duration_ms;
        return sheet;
    }

    let aspects: Vec<f32> = frames
        .iter()
        .map(|(_, f)| (f.width.max(1) as f32 / f.height.max(1) as f32).clamp(0.25, 4.5))
        .collect();
    let rows = plan_contact_sheet_rows(&aspects);
    let row_aspects: Vec<f32> = rows
        .iter()
        .map(|r| aspects[r.clone()].iter().sum::<f32>().max(0.1))
        .collect();
    let inv_sum: f32 = row_aspects.iter().map(|a| 1.0 / a).sum();
    let sheet_aspect = 1.0 / inv_sum.max(0.01);

    let pad = SHEET_PAD as f32;
    let gap = SHEET_GAP as f32;
    let native_row_w = rows
        .iter()
        .map(|r| {
            let sum_w: u32 = frames[r.clone()].iter().map(|(_, f)| f.width).sum();
            sum_w as f32 + gap * r.len().saturating_sub(1) as f32
        })
        .fold(0.0f32, f32::max);
    let min_inner_w = if frames.len() == 1 { 640.0 } else { 960.0 };
    let desired_inner_w = native_row_w.max(min_inner_w);

    let mut w_inner = if sheet_aspect >= 1.0 {
        (SHEET_MAX_EDGE - 2.0 * pad).min(desired_inner_w)
    } else {
        ((SHEET_MAX_EDGE - 2.0 * pad) * sheet_aspect).min(desired_inner_w)
    };

    let calc_h_inner = |wi: f32| -> f32 {
        rows.iter()
            .zip(&row_aspects)
            .map(|(r, &a)| {
                let avail = (wi - gap * r.len().saturating_sub(1) as f32).max(32.0);
                avail / a
            })
            .sum::<f32>()
            + gap * rows.len().saturating_sub(1) as f32
    };

    let h_inner = calc_h_inner(w_inner);
    let mut total_w = w_inner + 2.0 * pad;
    let total_h = h_inner + 2.0 * pad;

    let mut scale = 1.0f32;
    if total_w.max(total_h) > SHEET_MAX_EDGE {
        scale = scale.min(SHEET_MAX_EDGE / total_w.max(total_h));
    }
    if total_w * total_h > SHEET_MAX_PIXELS {
        scale = scale.min((SHEET_MAX_PIXELS / (total_w * total_h)).sqrt());
    }
    if scale < 1.0 {
        w_inner = ((total_w * scale) - 2.0 * pad).max(64.0);
        total_w = w_inner + 2.0 * pad;
    }

    let inner_w_px = total_w.round().max(64.0) as u32 - 2 * SHEET_PAD;
    let row_heights_px: Vec<u32> = rows
        .iter()
        .zip(&row_aspects)
        .map(|(r, &a)| {
            let total_gap = SHEET_GAP * r.len().saturating_sub(1) as u32;
            let avail_w = inner_w_px.saturating_sub(total_gap).max(1);
            (avail_w as f32 / a).round().max(1.0) as u32
        })
        .collect();

    let width = inner_w_px + 2 * SHEET_PAD;
    let height = 2 * SHEET_PAD
        + row_heights_px.iter().sum::<u32>()
        + SHEET_GAP * rows.len().saturating_sub(1) as u32;

    let mut sheet = solid(width, height, SHEET_BG);
    sheet.t_ms = duration_ms;

    let mut cells = Vec::with_capacity(frames.len());
    let mut row_y = SHEET_PAD;
    for (r_idx, row) in rows.iter().enumerate() {
        let row_h = row_heights_px[r_idx];
        let k = row.len();
        let total_gap = SHEET_GAP * k.saturating_sub(1) as u32;
        let avail_w = inner_w_px.saturating_sub(total_gap);
        let a_row = row_aspects[r_idx];
        let mut cum_a = 0.0f32;

        for (j, i) in row.clone().enumerate() {
            let x0 = (avail_w as f32 * (cum_a / a_row)).round() as u32;
            cum_a += aspects[i];
            let x1 = if j + 1 == k {
                avail_w
            } else {
                (avail_w as f32 * (cum_a / a_row)).round() as u32
            };
            let cell_w = x1.saturating_sub(x0).max(1);
            let cell_x = SHEET_PAD + x0 + j as u32 * SHEET_GAP;
            let rect = [cell_x as f32, row_y as f32, cell_w as f32, row_h as f32];
            cells.push((i, rect));
        }
        row_y += row_h + SHEET_GAP;
    }

    for (i, rect) in cells {
        let (kf, image) = &frames[i];
        blit_scaled(&mut sheet, image, rect, 0.0);
        draw_keyframe_badge(&mut sheet, rect, kf, SHEET_CHIP_K, image.badge_top_right);
    }
    sheet
}

/// An `M:SS.mmm` pill in the bottom-left corner, for `scrawl zoom` cells; `k` is how many frame
/// pixels make one pixel of the finished sheet.
pub fn draw_time_label(frame: &mut RgbaFrame, text: &str, k: f32) {
    let (w, h) = (frame.width as f32, frame.height as f32);
    if w == 0.0 || h == 0.0 {
        return;
    }
    let k = k.max(0.5);
    let font_size = 13.0 * k;
    let pill_h = 22.0 * k;
    let text_w = text_width(text, font_size, true);
    let pill_w = text_w + 14.0 * k;
    let margin = 6.0 * k;
    let (x, y) = (margin, h - pill_h - margin);
    rounded(
        frame,
        [x, y, pill_w, pill_h],
        pill_h * 0.5,
        [255, 255, 255, 42],
    );
    rounded(
        frame,
        [x + 1.0, y + 1.0, pill_w - 2.0, pill_h - 2.0],
        pill_h * 0.5 - 1.0,
        [18, 18, 20, 218],
    );
    draw_text(
        frame,
        text,
        x + 7.0 * k,
        y + 5.0 * k,
        font_size,
        WHITE,
        true,
    );
}

/// A tight grid of every frame of a `scrawl zoom` burst, in time order: four portrait or three
/// landscape frames to a row.
pub fn render_zoom_sheet(frames: &[RgbaFrame]) -> RgbaFrame {
    const PAD: f32 = 12.0;
    const GAP: f32 = 10.0;
    if frames.is_empty() {
        return solid(640, 360, [20, 20, 20, 255]);
    }
    let portrait = frames.iter().all(|f| f.height > f.width);
    let (cell, columns) = if portrait { (300.0, 4) } else { (480.0, 3) };
    let columns = frames.len().min(columns);
    let sizes: Vec<[f32; 2]> = frames
        .iter()
        .map(|f| {
            let scale = cell / f.width.max(1) as f32;
            [cell, f.height as f32 * scale]
        })
        .collect();
    let rows: Vec<f32> = sizes
        .chunks(columns)
        .map(|row| row.iter().map(|s| s[1]).fold(0.0, f32::max))
        .collect();
    let width = (PAD * 2.0 + cell * columns as f32 + GAP * (columns - 1) as f32) as u32;
    let height = (PAD * 2.0 + rows.iter().sum::<f32>() + GAP * rows.len().saturating_sub(1) as f32)
        .ceil() as u32;
    let mut sheet = solid(width, height, [20, 20, 20, 255]);
    let mut y = PAD;
    for (row, row_h) in rows.iter().enumerate() {
        for col in 0..columns {
            let i = row * columns + col;
            let (Some(size), Some(frame)) = (sizes.get(i), frames.get(i)) else {
                continue;
            };
            let x = PAD + col as f32 * (cell + GAP);
            rounded(&mut sheet, [x, y, size[0], size[1]], 8.0, [46, 46, 48, 255]);
            blit_scaled(
                &mut sheet,
                frame,
                [x + 1.0, y + 1.0, size[0] - 2.0, size[1] - 2.0],
                7.0,
            );
        }
        y += row_h + GAP;
    }
    sheet
}

fn sample_bilinear(
    src: &RgbaFrame,
    row0: usize,
    row1: usize,
    fy: f32,
    x0: usize,
    x1: usize,
    fx: f32,
) -> [f32; 4] {
    let w_y0 = 1.0 - fy;
    let w00 = (1.0 - fx) * w_y0;
    let w10 = fx * w_y0;
    let w01 = (1.0 - fx) * fy;
    let w11 = fx * fy;
    let i00 = row0 + x0 * 4;
    let i10 = row0 + x1 * 4;
    let i01 = row1 + x0 * 4;
    let i11 = row1 + x1 * 4;
    let p00 = &src.rgba[i00..i00 + 4];
    let p10 = &src.rgba[i10..i10 + 4];
    let p01 = &src.rgba[i01..i01 + 4];
    let p11 = &src.rgba[i11..i11 + 4];
    if (p00[3] & p10[3] & p01[3] & p11[3]) == 255 {
        return [
            p00[0] as f32 * w00 + p10[0] as f32 * w10 + p01[0] as f32 * w01 + p11[0] as f32 * w11,
            p00[1] as f32 * w00 + p10[1] as f32 * w10 + p01[1] as f32 * w01 + p11[1] as f32 * w11,
            p00[2] as f32 * w00 + p10[2] as f32 * w10 + p01[2] as f32 * w01 + p11[2] as f32 * w11,
            255.0,
        ];
    }
    let mut accum = [0.0f32; 4];
    for (p, weight) in [(p00, w00), (p10, w10), (p01, w01), (p11, w11)] {
        let a = p[3] as f32 / 255.0;
        for c in 0..3 {
            accum[c] += p[c] as f32 * a * weight;
        }
        accum[3] += a * weight;
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

fn blit_scaled(dst: &mut RgbaFrame, src: &RgbaFrame, rect: [f32; 4], radius: f32) {
    let [x, y, w, h] = rect;
    if src.width == 0 || src.height == 0 || w <= 0.0 || h <= 0.0 {
        return;
    }
    let r = radius.max(0.0).min(w * 0.5).min(h * 0.5);
    let src_w = src.width as usize;
    let src_h = src.height as usize;
    let dst_w = dst.width as usize;
    let px_start = (x.floor() as i32).max(0);
    let px_end = ((x + w).ceil() as i32).min(dst.width as i32);
    if px_start >= px_end {
        return;
    }
    let inv_w = src.width as f32 / w;
    let inv_h = src.height as f32 / h;
    let max_sx = src.width.saturating_sub(1) as f32;
    let max_sy = src.height.saturating_sub(1) as f32;
    let supersample = inv_w > 1.35 || inv_h > 1.35;

    let x_tap = |sample_x: f32| -> (usize, usize, f32) {
        let sx = sample_x.clamp(0.0, max_sx);
        let x0 = sx.floor() as usize;
        let x1 = (x0 + 1).min(src_w - 1);
        (x0, x1, sx - x0 as f32)
    };
    let y_tap = |sample_y: f32| -> (usize, usize, f32) {
        let sy = sample_y.clamp(0.0, max_sy);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(src_h - 1);
        (y0 * src_w * 4, y1 * src_w * 4, sy - y0 as f32)
    };

    // Precompute X sample taps across the horizontal span once.
    let span = (px_end - px_start) as usize;
    let mut x_taps = Vec::with_capacity(span);
    for px in px_start..px_end {
        let cx = px as f32 + 0.5;
        if supersample {
            let sx_a = (cx - 0.25 - x) * inv_w - 0.5;
            let sx_b = (cx + 0.25 - x) * inv_w - 0.5;
            x_taps.push((cx, x_tap(sx_a), x_tap(sx_b)));
        } else {
            let t = x_tap((cx - x) * inv_w - 0.5);
            x_taps.push((cx, t, t));
        }
    }

    for py in (y.floor() as i32).max(0)..((y + h).ceil() as i32).min(dst.height as i32) {
        let cy = py as f32 + 0.5;
        let (ya, yb) = if supersample {
            (
                y_tap((cy - 0.25 - y) * inv_h - 0.5),
                y_tap((cy + 0.25 - y) * inv_h - 0.5),
            )
        } else {
            let t = y_tap((cy - y) * inv_h - 0.5);
            (t, t)
        };
        let dst_row = py as usize * dst_w * 4;
        let y_inside_band = cy >= y + r + 0.5 && cy <= y + h - r - 0.5;
        let y_inside_box = cy >= y + 0.5 && cy <= y + h - 0.5;

        for (idx, &(cx, xa, xb)) in x_taps.iter().enumerate() {
            let px = px_start + idx as i32;
            let full_cov = (y_inside_band && cx >= x + 0.5 && cx <= x + w - 0.5)
                || (y_inside_box && cx >= x + r + 0.5 && cx <= x + w - r - 0.5);
            let coverage = if full_cov {
                1.0
            } else {
                let c = (0.5 - rounded_distance(cx, cy, rect, radius)).clamp(0.0, 1.0);
                if c <= 0.0 {
                    continue;
                }
                c
            };

            let rgba = if supersample {
                let s00 = sample_bilinear(src, ya.0, ya.1, ya.2, xa.0, xa.1, xa.2);
                let s10 = sample_bilinear(src, ya.0, ya.1, ya.2, xb.0, xb.1, xb.2);
                let s01 = sample_bilinear(src, yb.0, yb.1, yb.2, xa.0, xa.1, xa.2);
                let s11 = sample_bilinear(src, yb.0, yb.1, yb.2, xb.0, xb.1, xb.2);
                [
                    ((s00[0] + s10[0] + s01[0] + s11[0]) * 0.25).round() as u8,
                    ((s00[1] + s10[1] + s01[1] + s11[1]) * 0.25).round() as u8,
                    ((s00[2] + s10[2] + s01[2] + s11[2]) * 0.25).round() as u8,
                    ((s00[3] + s10[3] + s01[3] + s11[3]) * 0.25).round() as u8,
                ]
            } else {
                let s = sample_bilinear(src, ya.0, ya.1, ya.2, xa.0, xa.1, xa.2);
                [
                    s[0].round() as u8,
                    s[1].round() as u8,
                    s[2].round() as u8,
                    s[3].round() as u8,
                ]
            };

            if full_cov && rgba[3] == 255 {
                let out_i = dst_row + px as usize * 4;
                dst.rgba[out_i..out_i + 4].copy_from_slice(&rgba);
            } else if rgba[3] > 0 {
                blend(dst, px, py, rgba, coverage);
            }
        }
    }
}

/// Render at a stable design size, then scale to the requested capture geometry.
pub fn render_synthetic_demo_frame(width: u32, height: u32, t_ms: u64, phase: u32) -> RgbaFrame {
    let mut frame = solid(1280, 800, [22, 23, 26, 255]);
    rounded(&mut frame, [0.0, 0.0, 244.0, 800.0], 0.0, [28, 29, 33, 255]);
    rounded(&mut frame, [244.0, 0.0, 1.0, 800.0], 0.0, [48, 49, 54, 255]);
    for (x, color) in [
        (22.0, [255, 95, 87, 255]),
        (42.0, [254, 188, 46, 255]),
        (62.0, [40, 200, 64, 255]),
    ] {
        circle(&mut frame, [x, 24.0], 6.0, color);
    }
    draw_text(&mut frame, "Workspace", 20.0, 67.0, 20.0, TEXT, true);
    draw_text(
        &mut frame,
        "+  New session",
        20.0,
        116.0,
        16.0,
        MUTED,
        false,
    );
    draw_text(&mut frame, "SESSIONS", 20.0, 176.0, 11.0, MUTED, true);
    for (i, title) in [
        "Review tool activity",
        "Update settings panel",
        "Keyboard navigation",
        "Fix scroll position",
    ]
    .iter()
    .enumerate()
    {
        let y = 209.0 + i as f32 * 48.0;
        if i == 0 {
            rounded(
                &mut frame,
                [10.0, y - 8.0, 224.0, 38.0],
                7.0,
                [47, 48, 55, 255],
            );
        }
        draw_text(
            &mut frame,
            title,
            22.0,
            y,
            14.0,
            if i == 0 { TEXT } else { MUTED },
            false,
        );
    }
    draw_text(&mut frame, "Settings", 22.0, 754.0, 14.0, MUTED, false);
    rounded(
        &mut frame,
        [245.0, 0.0, 1035.0, 64.0],
        0.0,
        [26, 27, 31, 255],
    );
    draw_text(
        &mut frame,
        "Review tool activity",
        282.0,
        23.0,
        17.0,
        TEXT,
        true,
    );
    draw_text(&mut frame, "Share", 1169.0, 25.0, 14.0, MUTED, false);
    rounded(
        &mut frame,
        [292.0, 100.0, 928.0, 72.0],
        12.0,
        [36, 38, 43, 255],
    );
    draw_text(
        &mut frame,
        "Check the tool results and explain what changed.",
        313.0,
        126.0,
        18.0,
        TEXT,
        false,
    );
    circle(&mut frame, [304.0, 210.0], 10.0, [83, 114, 229, 255]);
    draw_text(&mut frame, "Assistant", 326.0, 200.0, 16.0, TEXT, true);
    draw_text(
        &mut frame,
        "I’ll inspect the files and compare the output.",
        294.0,
        242.0,
        17.0,
        [204, 206, 212, 255],
        false,
    );
    let expanded = phase >= 1;
    let band_y = if expanded { 304.0 } else { 286.0 };
    rounded(
        &mut frame,
        [290.0, band_y, 930.0, 48.0],
        8.0,
        [43, 45, 51, 255],
    );
    draw_text(
        &mut frame,
        if expanded {
            "−  Used 12 tools"
        } else {
            "+  Used 12 tools"
        },
        309.0,
        band_y + 14.0,
        16.0,
        TEXT,
        true,
    );
    draw_text(
        &mut frame,
        "12.4s",
        1145.0,
        band_y + 15.0,
        14.0,
        MUTED,
        false,
    );
    if expanded {
        for (i, (name, detail)) in [
            ("Read", "src/components/tool-panel.tsx"),
            ("Search", "tool results · 8 matches"),
            ("Read", "src/styles/layout.css"),
            ("Run", "Checks passed"),
        ]
        .iter()
        .enumerate()
        {
            let y = band_y + 60.0 + i as f32 * 43.0;
            circle(&mut frame, [313.0, y + 10.0], 3.0, [105, 190, 139, 255]);
            draw_text(&mut frame, name, 328.0, y, 14.0, TEXT, true);
            draw_text(&mut frame, detail, 402.0, y, 14.0, MUTED, false);
            rounded(
                &mut frame,
                [310.0, y + 31.0, 888.0, 1.0],
                0.0,
                [42, 44, 49, 255],
            );
        }
    }
    let response_y = if expanded { 569.0 } else { 363.0 };
    draw_text(
        &mut frame,
        "The results are ready.",
        294.0,
        response_y,
        18.0,
        TEXT,
        true,
    );
    draw_text(
        &mut frame,
        "Open a tool row to see its input and output.",
        294.0,
        response_y + 34.0,
        16.0,
        MUTED,
        false,
    );
    rounded(
        &mut frame,
        [284.0, 686.0, 946.0, 88.0],
        15.0,
        [64, 66, 74, 255],
    );
    rounded(
        &mut frame,
        [285.0, 687.0, 944.0, 86.0],
        14.0,
        [31, 33, 38, 255],
    );
    draw_text(
        &mut frame,
        "Ask a follow-up…",
        308.0,
        708.0,
        16.0,
        MUTED,
        false,
    );
    draw_text(&mut frame, "+", 309.0, 746.0, 16.0, MUTED, false);
    draw_text(
        &mut frame,
        "Local workspace",
        341.0,
        747.0,
        12.0,
        MUTED,
        false,
    );
    circle(&mut frame, [1200.0, 746.0], 13.0, [224, 226, 231, 255]);
    draw_text(
        &mut frame,
        "↑",
        1193.0,
        736.0,
        17.0,
        [28, 29, 33, 255],
        true,
    );
    if phase >= 2 {
        ring(&mut frame, [1200.0, 746.0], 16.0, 2.0, [93, 161, 255, 255]);
    }
    if width == 1280 && height == 800 {
        frame.t_ms = t_ms;
        return frame;
    }
    let mut out = solid(width, height, [22, 23, 26, 255]);
    blit_scaled(
        &mut out,
        &frame,
        [0.0, 0.0, width as f32, height as f32],
        0.0,
    );
    out.t_ms = t_ms;
    out
}

#[cfg(test)]
fn wrap_text(text: &str, size: f32, width: f32, bold: bool) -> Vec<String> {
    wrap_text_indented(text, size, width, width, bold)
}

#[cfg(test)]
fn wrap_text_indented(
    text: &str,
    size: f32,
    width: f32,
    first_width: f32,
    bold: bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            let available = if lines.is_empty() { first_width } else { width };
            if text_width(&candidate, size, bold) <= available {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // Character boundaries avoid slicing inside UTF-8, including emoji.
            for ch in word.chars() {
                let mut candidate = line.clone();
                candidate.push(ch);
                let available = if lines.is_empty() { first_width } else { width };
                if !line.is_empty() && text_width(&candidate, size, bold) > available {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(ch);
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    lines
}

#[cfg(target_os = "macos")]
mod native_text {
    use core_foundation::{
        attributed_string::CFMutableAttributedString,
        base::{CFRange, TCFType},
        boolean::CFBoolean,
        string::CFString,
    };
    use core_graphics::{
        base::kCGImageAlphaNone,
        color_space::CGColorSpace,
        context::{CGContext, CGTextDrawingMode},
    };
    use core_text::{
        font::{self, CTFont},
        line::CTLine,
        string_attributes::{kCTFontAttributeName, kCTForegroundColorFromContextAttributeName},
    };

    pub fn line(text: &str, size: f32, bold: bool) -> Option<(CTFont, CTLine)> {
        let font = font::new_from_name(
            if bold {
                "HelveticaNeue-Bold"
            } else {
                "HelveticaNeue"
            },
            size as f64,
        )
        .ok()?;
        let mut attributed = CFMutableAttributedString::new();
        attributed.replace_str(&CFString::new(text), CFRange::init(0, 0));
        unsafe {
            attributed.set_attribute(
                CFRange::init(0, attributed.char_len()),
                kCTFontAttributeName,
                &font,
            );
            attributed.set_attribute(
                CFRange::init(0, attributed.char_len()),
                kCTForegroundColorFromContextAttributeName,
                &CFBoolean::true_value(),
            );
        }
        let line = CTLine::new_with_attributed_string(attributed.as_concrete_TypeRef());
        Some((font, line))
    }

    pub fn mask(text: &str, size: f32, bold: bool) -> Option<(usize, usize, Vec<u8>)> {
        let (font, line) = line(text, size, bold)?;
        let width = line.get_typographic_bounds().width.ceil().max(1.0) as usize + 4;
        let height = (font.ascent() + font.descent() + font.leading())
            .ceil()
            .max(1.0) as usize
            + 4;
        let space = CGColorSpace::create_device_gray();
        let mut ctx = CGContext::create_bitmap_context(
            None,
            width,
            height,
            8,
            width,
            &space,
            kCGImageAlphaNone,
        );
        ctx.data().fill(0);
        ctx.set_should_antialias(true);
        ctx.set_gray_fill_color(1.0, 1.0);
        ctx.set_text_drawing_mode(CGTextDrawingMode::CGTextFill);
        ctx.set_text_position(2.0, height as f64 - 2.0 - font.ascent());
        line.draw(&ctx);
        ctx.flush();
        Some((width, height, ctx.data().to_vec()))
    }
}

/// Arial (Windows' Helvetica) from the system fonts, through `ab_glyph`.
#[cfg(target_os = "windows")]
mod native_text {
    use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
    use std::sync::OnceLock;

    fn font(bold: bool) -> Option<&'static FontVec> {
        static REGULAR: OnceLock<Option<FontVec>> = OnceLock::new();
        static BOLD: OnceLock<Option<FontVec>> = OnceLock::new();
        let load = |name: &str| {
            let dir = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
            let bytes = std::fs::read(std::path::Path::new(&dir).join("Fonts").join(name)).ok()?;
            FontVec::try_from_vec(bytes).ok()
        };
        if bold {
            BOLD.get_or_init(|| load("arialbd.ttf")).as_ref()
        } else {
            REGULAR.get_or_init(|| load("arial.ttf")).as_ref()
        }
    }

    /// Glyph positions along the baseline, kerned.
    fn layout(text: &str, size: f32, bold: bool) -> Option<(f32, Vec<ab_glyph::Glyph>)> {
        let font = font(bold)?.as_scaled(PxScale::from(size));
        let mut x = 0.0;
        let mut previous = None;
        let mut glyphs = Vec::new();
        for ch in text.chars() {
            let id = font.glyph_id(ch);
            if let Some(prev) = previous {
                x += font.kern(prev, id);
            }
            glyphs.push(id.with_scale_and_position(size, ab_glyph::point(x, font.ascent())));
            x += font.h_advance(id);
            previous = Some(id);
        }
        Some((x, glyphs))
    }

    pub fn width(text: &str, size: f32, bold: bool) -> Option<f32> {
        layout(text, size, bold).map(|(w, _)| w)
    }

    /// An 8-bit coverage mask with 2 px of padding, like the macOS CoreText one.
    pub fn mask(text: &str, size: f32, bold: bool) -> Option<(usize, usize, Vec<u8>)> {
        let font = font(bold)?;
        let scaled = font.as_scaled(PxScale::from(size));
        let (w, glyphs) = layout(text, size, bold)?;
        let width = w.ceil().max(1.0) as usize + 4;
        let height = (scaled.ascent() - scaled.descent() + scaled.line_gap())
            .ceil()
            .max(1.0) as usize
            + 4;
        let mut pixels = vec![0_u8; width * height];
        for glyph in glyphs {
            let Some(outline) = font.outline_glyph(glyph) else {
                continue;
            };
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                let x = bounds.min.x as i32 + gx as i32 + 2;
                let y = bounds.min.y as i32 + gy as i32 + 2;
                if x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height {
                    let p = &mut pixels[y as usize * width + x as usize];
                    *p = (*p).max((coverage.clamp(0.0, 1.0) * 255.0) as u8);
                }
            });
        }
        Some((width, height, pixels))
    }
}

fn text_width(text: &str, size: f32, bold: bool) -> f32 {
    #[cfg(target_os = "macos")]
    if let Some((_, line)) = native_text::line(text, size, bold) {
        return line.get_typographic_bounds().width as f32;
    }
    #[cfg(target_os = "windows")]
    if let Some(width) = native_text::width(text, size, bold) {
        return width;
    }
    let _ = bold;
    text.chars().count() as f32 * size * 6.0 / 7.0
}

fn draw_text(
    frame: &mut RgbaFrame,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    color: Color,
    bold: bool,
) {
    if text.is_empty() || size <= 0.0 {
        return;
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if let Some((width, height, pixels)) = native_text::mask(text, size, bold) {
        for py in 0..height {
            for px in 0..width {
                blend(
                    frame,
                    x.round() as i32 + px as i32 - 2,
                    y.round() as i32 + py as i32 - 2,
                    color,
                    pixels[py * width + px] as f32 / 255.0,
                );
            }
        }
        return;
    }
    let _ = bold;
    let scale = size / 7.0;
    for (index, ch) in text.chars().enumerate() {
        let rows = glyph(ch);
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    rounded(
                        frame,
                        [
                            x + (index * 6 + col) as f32 * scale,
                            y + row as f32 * scale,
                            scale,
                            scale,
                        ],
                        0.0,
                        color,
                    );
                }
            }
        }
    }
}

/// Portable 5×7 fallback. Unsupported Unicode is a visible replacement glyph.
fn glyph(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ' ' | '\t' | '\n' | '\r' => [0; 7],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        ',' => [0, 0, 0, 0, 4, 4, 8],
        ':' => [0, 12, 12, 0, 12, 12, 0],
        ';' => [0, 12, 12, 0, 4, 4, 8],
        '#' => [10, 31, 10, 10, 31, 10, 0],
        '·' | '•' => [0, 0, 0, 4, 0, 0, 0],
        '-' | '–' | '—' | '−' => [0, 0, 0, 31, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        '+' => [0, 4, 4, 31, 4, 4, 0],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        '\\' => [16, 8, 8, 4, 2, 2, 1],
        '[' => [14, 8, 8, 8, 8, 8, 14],
        ']' => [14, 2, 2, 2, 2, 2, 14],
        '(' => [2, 4, 8, 8, 8, 4, 2],
        ')' => [8, 4, 2, 2, 2, 4, 8],
        '{' => [2, 4, 4, 8, 4, 4, 2],
        '}' => [8, 4, 4, 2, 4, 4, 8],
        '\'' | '’' | '‘' | '`' => [4, 4, 8, 0, 0, 0, 0],
        '"' | '“' | '”' => [10, 10, 20, 0, 0, 0, 0],
        '!' => [4, 4, 4, 4, 4, 0, 4],
        '?' => [14, 17, 1, 2, 4, 0, 4],
        '=' => [0, 0, 31, 0, 31, 0, 0],
        '<' => [1, 2, 4, 8, 4, 2, 1],
        '>' => [16, 8, 4, 2, 4, 8, 16],
        '↑' => [4, 14, 21, 4, 4, 4, 4],
        '↓' => [4, 4, 4, 4, 21, 14, 4],
        '→' => [0, 4, 2, 31, 2, 4, 0],
        '←' => [0, 4, 8, 31, 8, 4, 0],
        '✎' | '✏' => [3, 7, 6, 12, 24, 16, 31],
        '…' => [0, 0, 0, 0, 0, 0, 21],
        '×' => [0, 17, 10, 4, 10, 17, 0],
        '*' => [0, 21, 14, 31, 14, 21, 0],
        '%' => [25, 25, 2, 4, 8, 19, 19],
        '&' => [12, 18, 20, 8, 21, 18, 13],
        '@' => [14, 17, 23, 21, 23, 16, 14],
        '|' => [4; 7],
        _ => [31, 17, 5, 4, 4, 0, 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn key(index: usize) -> Keyframe {
        Keyframe {
            index,
            t: 4000,
            reason: KeyframeReason::Mark,
            file: format!("frames/{index}.png"),
            event: "✎ circled region x 0.42–0.58, y 0.26–0.34 over “Band header”".into(),
            said: Some("When I click Used 12 tools…".into()),
            mark_id: None,
            cursor: None,
        }
    }

    #[test]
    fn opaque_rounded_spans_match_source_over_edges_and_clipping() {
        for rect in [
            [0.0, 0.0, 32.0, 24.0],
            [-4.25, -2.5, 21.75, 16.25],
            [8.75, 5.25, 31.5, 24.75],
            [2.25, 3.75, 0.5, 1.25],
            [-50.0, 0.0, 5.0, 5.0],
            [40.0, 0.0, 5.0, 5.0],
        ] {
            for radius in [0.0, 0.25, 2.5, 10.0, 100.0] {
                for alpha in [0, 127, 255] {
                    let color = [59, 130, 243, alpha];
                    let mut expected = solid(32, 24, [90, 60, 30, 137]);
                    let mut actual = expected.clone();
                    rounded(&mut actual, rect, radius, color);
                    let [x, y, w, h] = rect;
                    for py in (y.floor() as i32 - 1).max(0)..((y + h).ceil() as i32 + 1).min(24) {
                        for px in (x.floor() as i32 - 1).max(0)..((x + w).ceil() as i32 + 1).min(32)
                        {
                            let r = radius.max(0.0).min(w * 0.5).min(h * 0.5);
                            let qx = (px as f32 + 0.5 - x - w * 0.5).abs() - w * 0.5 + r;
                            let qy = (py as f32 + 0.5 - y - h * 0.5).abs() - h * 0.5 + r;
                            let distance = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
                            let coverage = (0.5 - distance).clamp(0.0, 1.0);
                            blend(&mut expected, px, py, color, coverage);
                        }
                    }
                    assert_eq!(
                        actual.rgba, expected.rgba,
                        "rect={rect:?} radius={radius} alpha={alpha}"
                    );
                }
            }
        }
    }

    #[test]
    fn swipes_leave_a_trail_on_phone_keyframes() {
        let base = solid(400, 860, [240, 240, 240, 255]);
        let swipe = InteractionEvent::Drag {
            t: 3000,
            t1: 3400,
            x0: 0.5,
            y0: 0.8,
            x1: 0.5,
            y1: 0.3,
            label: None,
            button: Some("touch".into()),
            path: vec![[0.5, 0.8], [0.52, 0.55], [0.5, 0.3]],
        };
        let blue = |frame: &RgbaFrame| {
            frame
                .rgba
                .chunks_exact(4)
                .filter(|p| p[2] > 200 && p[0] < 120)
                .count()
        };
        let after = render_annotated_keyframe(&base, &key(1), &[], &[], &[swipe.clone()]);
        assert!(blue(&after) > 500, "trail missing");
        let mut later = key(1);
        later.t = 6000;
        let stale = render_annotated_keyframe(&base, &later, &[], &[], &[swipe]);
        assert_eq!(blue(&stale), 0, "an old swipe should not be drawn");
    }

    /// A click is marked by a thin ring and a small dot: what was clicked stays visible.
    #[test]
    fn click_marks_leave_what_they_mark_visible() {
        let base = solid(1280, 800, [240, 240, 240, 255]);
        let mut keyframe = key(1);
        keyframe.reason = KeyframeReason::Click;
        keyframe.cursor = Some([0.5, 0.5]);
        let after = render_annotated_keyframe(&base, &keyframe, &[], &[], &[]);
        let changed: Vec<(usize, usize)> = (0..800)
            .flat_map(|y| (0..1280).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let i = (y * 1280 + x) * 4;
                after.rgba[i..i + 4] != base.rgba[i..i + 4]
            })
            .filter(|&(x, y)| x > 600 && x < 680 && y > 360 && y < 440)
            .collect();
        assert!(!changed.is_empty(), "click mark missing");
        // A quarter of the filled 18 px disc it replaces, and the band between dot and ring is
        // left alone.
        assert!(changed.len() < 260, "{} px painted", changed.len());
        for (x, y) in [(645, 400), (640, 395), (636, 400)] {
            let i = (y * 1280 + x) * 4;
            assert_eq!(
                after.rgba[i..i + 4],
                base.rgba[i..i + 4],
                "({x}, {y}) covered"
            );
        }
        // Nothing is opaque: the screen shows through every pixel of the mark.
        for (x, y) in changed {
            let i = (y * 1280 + x) * 4;
            assert!(after.rgba[i] > 20, "({x}, {y}) hides the screen");
        }
    }

    #[test]
    fn png_round_trip_preserves_rgba_including_alpha() {
        let frame = RgbaFrame::new(
            123,
            2,
            2,
            vec![
                255, 59, 48, 255, 3, 5, 7, 0, 23, 42, 91, 128, 255, 255, 255, 255,
            ],
        );
        let result = decode_png(Cursor::new(encode_png(&frame).unwrap())).unwrap();
        assert_eq!((result.width, result.height), (2, 2));
        assert_eq!(result.rgba, frame.rgba);
    }

    #[test]
    fn active_ink_paints_red_but_future_ink_is_hidden() {
        let base = solid(320, 200, [20, 20, 20, 255]);
        let stroke = Stroke {
            t0: 3500,
            t1: 3900,
            fw: 320,
            fh: 200,
            on: None,
            pin: None,
            laser: false,
            pts: vec![1500, 6000, 0, 5000, 6000, 200, 8500, 6000, 400],
        };
        let result = render_annotated_keyframe(&base, &key(1), &[stroke.clone()], &[], &[]);
        let red = result
            .rgba
            .chunks_exact(4)
            .enumerate()
            .filter(|(i, p)| i / 320 > 100 && p[0] == 255 && p[3] == 255)
            .count();
        assert!(
            red > 80,
            "Expected opaque red stroke pixels below the badge, got {red}"
        );
        let future = Stroke {
            t0: 5000,
            t1: 5400,
            ..stroke
        };
        let result = render_annotated_keyframe(&base, &key(1), &[future], &[], &[]);
        assert_eq!(&result.rgba[100 * 320 * 4..], &base.rgba[100 * 320 * 4..]);
        assert!(base.rgba.chunks_exact(4).all(|p| p == [20, 20, 20, 255]));
    }

    #[test]
    fn a_grouped_keyframe_shows_every_mark_drawn_on_its_screen() {
        let base = solid(320, 200, [20, 20, 20, 255]);
        // Three strokes across the lower half, drawn long before the keyframe.
        let stroke = |y: i32| Stroke {
            t0: 100,
            t1: 300,
            fw: 320,
            fh: 200,
            on: None,
            pin: None,
            laser: false,
            pts: vec![1500, y, 0, 5000, y, 100, 8500, y, 200],
        };
        let strokes = [stroke(6000), stroke(7500), stroke(9000)];
        let mark = |id: &str, stroke: usize, frame: &str| Mark {
            id: id.into(),
            strokes: vec![stroke],
            t0: 100,
            t1: 300,
            box_norm: [0.15, 0.6, 0.85, 0.9],
            shape: Shape::Underline,
            from: None,
            to: None,
            pin: None,
            on: None,
            description: String::new(),
            frame: Some(frame.into()),
        };
        let mut keyframe = key(1);
        keyframe.mark_id = Some("m1".into());
        let marks = [
            mark("m1", 0, &keyframe.file),
            mark("m2", 1, &keyframe.file),
            mark("m3", 2, "frames/2.png"),
        ];
        let result = render_annotated_keyframe(&base, &keyframe, &strokes, &marks, &[]);
        let red_rows = |rows: std::ops::Range<usize>| {
            rows.filter(|y| {
                result.rgba[y * 320 * 4..(y + 1) * 320 * 4]
                    .chunks_exact(4)
                    .any(|p| p[0] == 255 && p[1] < 80)
            })
            .count()
        };
        assert!(red_rows(115..125) > 0 && red_rows(145..155) > 0);
        assert_eq!(red_rows(175..185), 0, "another keyframe's mark");

        // No numbered badges beside the marks (the text numbers them): below the corner badge,
        // every pixel is the backdrop or red ink.
        let foreign = result.rgba[60 * 320 * 4..]
            .chunks_exact(4)
            .filter(|p| p[..3] != [20, 20, 20] && !(p[0] > p[1] && p[0] > p[2]))
            .count();
        assert_eq!(foreign, 0, "pixels that are neither backdrop nor ink");
        // Tests run with experiments off, so also check the images never depend on agentContext.
        assert!(!include_str!("render.rs").contains(concat!("handoff::", "AGENT_CONTEXT")));
    }

    /// The box of pixels in `frame` that differ from `base`, as `[x0, y0, x1, y1]` inclusive.
    fn changed_box(frame: &RgbaFrame, base: &RgbaFrame) -> Option<[usize; 4]> {
        let w = frame.width as usize;
        let mut found: Option<[usize; 4]> = None;
        for (i, (a, b)) in frame
            .rgba
            .chunks_exact(4)
            .zip(base.rgba.chunks_exact(4))
            .enumerate()
        {
            if a != b {
                let (x, y) = (i % w, i / w);
                let r = found.get_or_insert([x, y, x, y]);
                *r = [r[0].min(x), r[1].min(y), r[2].max(x), r[3].max(y)];
            }
        }
        found
    }

    #[test]
    fn a_keyframe_is_labelled_by_its_number_alone_in_a_small_white_chip() {
        let base = solid(1280, 800, [120, 130, 140, 255]);
        let mut frame = base.clone();
        stamp_keyframe_badge_in_place(&mut frame, &key(7));
        let [x0, y0, x1, y1] = changed_box(&frame, &base).expect("no chip");
        // k = 800 / 480: the chip sits 5k from the corner (its ring just outside) and is square
        // for one digit: no `#`, no time beside it.
        let k = 800.0 / 480.0;
        assert!((x0 as f32 - 3.6 * k).abs() <= 1.5 && (y0 as f32 - 3.6 * k).abs() <= 1.5);
        let (w, h) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
        assert!((h - 26.8 * k).abs() <= 2.0, "{w}x{h}");
        assert!(
            (w - h).abs() <= 1.0,
            "a one-digit chip should be square, got {w}x{h}"
        );
        let at = |x: usize, y: usize| &frame.rgba[(y * 1280 + x) * 4..][..4];
        // A white fill with dark digit strokes inside it, and a dark ring around it.
        let inner: Vec<_> = (y0 + 4..y1 - 3)
            .flat_map(|y| (x0 + 4..x1 - 3).map(move |x| (x, y)))
            .map(|(x, y)| at(x, y)[0])
            .collect();
        assert!(inner.iter().filter(|&&v| v > 240).count() > inner.len() / 2);
        assert!(inner.iter().filter(|&&v| v < 40).count() > 20, "no digit");
        assert!(at(x0 + 1, (y0 + y1) / 2)[0] < 40, "no ring");

        // Two digits widen it but keep it small.
        let mut wide = base.clone();
        stamp_keyframe_badge_in_place(&mut wide, &key(12));
        let [a0, _, a1, _] = changed_box(&wide, &base).unwrap();
        let w12 = (a1 - a0 + 1) as f32;
        assert!(w12 > w && w12 < 2.0 * h, "{w12}");
    }

    #[test]
    fn the_chip_moves_to_the_top_right_when_ink_or_the_cursor_is_top_left() {
        let base = solid(1280, 800, [120, 130, 140, 255]);
        let mut inked = base.clone();
        inked.badge_top_right = true;
        stamp_keyframe_badge_in_place(&mut inked, &key(3));
        let mut pointed = base.clone();
        let mut keyframe = key(3);
        keyframe.cursor = Some([0.1, 0.05]);
        stamp_keyframe_badge_in_place(&mut pointed, &keyframe);
        for frame in [&inked, &pointed] {
            let [x0, y0, x1, _] = changed_box(frame, &base).unwrap();
            let k = 800.0 / 480.0;
            assert!(x0 > 1200, "chip at x {x0}");
            assert!((1279.0 - x1 as f32 - 3.6 * k).abs() <= 1.5);
            assert!((y0 as f32 - 3.6 * k).abs() <= 1.5);
        }
    }

    #[test]
    fn contact_sheet_chips_are_one_readable_size_in_every_tile() {
        // Twelve tiles of mixed shapes: every chip is about 24 px whatever the tile's size.
        let dims = [(1280, 800), (736, 1600), (930, 975), (1054, 327)];
        let frames: Vec<_> = (1..=12)
            .map(|i| {
                let (w, h) = dims[i % dims.len()];
                let mut frame = solid(w, h, [120, 130, 140, 255]);
                frame.badge_top_right = i % 3 == 0;
                (key(i), frame)
            })
            .collect();
        let sheet = render_contact_sheet_for_duration(8000, &frames);
        // Chip pixels are the near-white fill; group them by tile through the gray around them.
        let w = sheet.width as usize;
        let white: Vec<(usize, usize)> = sheet
            .rgba
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, p)| p[0] > 240 && p[1] > 240 && p[2] > 240)
            .map(|(i, _)| (i % w, i / w))
            .collect();
        let mut chips: Vec<[usize; 4]> = Vec::new();
        for (x, y) in white {
            match chips
                .iter_mut()
                .find(|c| x + 4 >= c[0] && x <= c[2] + 4 && y + 4 >= c[1] && y <= c[3] + 4)
            {
                Some(c) => *c = [c[0].min(x), c[1].min(y), c[2].max(x), c[3].max(y)],
                None => chips.push([x, y, x, y]),
            }
        }
        assert_eq!(chips.len(), 12, "{chips:?}");
        for c in chips {
            // The white fill is the chip less its ring: 21.6 px, so about 24 px with the ring.
            let h = c[3] - c[1] + 1;
            assert!((20..=23).contains(&h), "chip fill {h} px tall: {c:?}");
        }
    }

    #[test]
    fn contact_sheets_lay_out_up_to_twelve_frames() {
        for count in [1, 2, 3, 4, 7, 9, 12] {
            for size in [(1280, 800), (736, 1600), (930, 975)] {
                let frames: Vec<_> = (1..=count)
                    .map(|i| {
                        (
                            key(i),
                            solid(size.0, size.1, [40 + i as u8 * 10, 60, 80, 255]),
                        )
                    })
                    .collect();
                let sheet = render_contact_sheet_for_duration(8000, &frames);
                assert!(
                    sheet.width <= SHEET_MAX_EDGE as u32 && sheet.height <= SHEET_MAX_EDGE as u32,
                    "Sheet {}x{} exceeded max edge for count={count} size={size:?}",
                    sheet.width,
                    sheet.height
                );
                assert!(
                    (sheet.width as f32 * sheet.height as f32) <= SHEET_MAX_PIXELS * 1.03,
                    "Sheet {}x{} exceeded pixel budget for count={count} size={size:?}",
                    sheet.width,
                    sheet.height
                );
                let mut centroids = Vec::with_capacity(count);
                for i in 1..=count {
                    let color = [40 + i as u8 * 10, 60, 80, 255];
                    let mut sum_x = 0usize;
                    let mut sum_y = 0usize;
                    let mut n_px = 0usize;
                    for (idx, p) in sheet.rgba.chunks_exact(4).enumerate() {
                        if p == color {
                            sum_x += idx % sheet.width as usize;
                            sum_y += idx / sheet.width as usize;
                            n_px += 1;
                        }
                    }
                    assert!(n_px > 500, "Missing card {i} of {count}");
                    centroids.push((sum_x as f32 / n_px as f32, sum_y as f32 / n_px as f32));
                }
                // Reading order: every frame is either to the right on the same row, or on a lower row.
                for pair in centroids.windows(2) {
                    let (x0, y0) = pair[0];
                    let (x1, y1) = pair[1];
                    let same_row = (y1 - y0).abs() < 20.0;
                    assert!(
                        (same_row && x1 > x0) || (!same_row && y1 > y0),
                        "Reading order violated for count={count} size={size:?}: {centroids:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn contact_sheets_pack_mixed_aspects_and_fill_width_without_letterboxing() {
        // Mixed aspect ratios like an auto-cropped screen capture: wide strip, landscape, square, tall portrait.
        let dims = [
            (1054, 327),
            (1183, 837),
            (1183, 837),
            (685, 648),
            (694, 1416),
            (898, 1416),
            (898, 1416),
        ];
        let frames: Vec<_> = dims
            .iter()
            .enumerate()
            .map(|(idx, &(w, h))| {
                let i = idx + 1;
                (key(i), solid(w, h, [50 + i as u8 * 15, 90, 130, 255]))
            })
            .collect();
        let sheet = render_contact_sheet_for_duration(8000, &frames);
        let w = sheet.width as usize;
        let h = sheet.height as usize;
        let pad = SHEET_PAD as usize;
        assert_eq!(sheet.rgba.len(), w * h * 4);
        assert!(sheet.rgba.chunks_exact(4).all(|p| p[3] == 255));

        // Every non-gutter scanline inside [pad..h-pad] must start with a frame pixel at x=pad
        // and end with a frame pixel at x=w-pad-1 (every row fills the inner width completely).
        let is_card = |p: &[u8]| p != SHEET_BG;
        let mut content_rows = 0usize;
        for y in pad..h - pad {
            let left = &sheet.rgba[(y * w + pad) * 4..(y * w + pad + 1) * 4];
            let right = &sheet.rgba[(y * w + w - pad - 1) * 4..(y * w + w - pad) * 4];
            if is_card(left) || is_card(right) {
                assert!(
                    is_card(left) && is_card(right),
                    "Row y={y} did not fill full inner width"
                );
                content_rows += 1;
                // Count non-gutter pixels in this scanline: only thin horizontal gutters may be SHEET_BG.
                let bg_px = (pad..w - pad)
                    .filter(|&x| !is_card(&sheet.rgba[(y * w + x) * 4..(y * w + x + 1) * 4]))
                    .count();
                assert!(
                    bg_px <= 5 * SHEET_GAP as usize,
                    "Unexpected blank space ({bg_px} px) on scanline y={y}"
                );
            }
        }
        assert!(content_rows > h * 4 / 5, "Too many empty scanlines");
    }

    #[test]
    fn contact_sheet_balances_per_frame_area_across_wide_and_tall_crops() {
        // Aspect mix from 17e3c-9: 5 landscape/square crops followed by 3 tall portrait crops.
        let dims = [
            (685, 648),
            (1183, 837),
            (1183, 837),
            (694, 726),
            (1183, 1095),
            (694, 1416),
            (898, 1416),
            (898, 1416),
        ];
        let aspects: Vec<f32> = dims.iter().map(|&(w, h)| w as f32 / h as f32).collect();
        let rows = plan_contact_sheet_rows(&aspects);
        let mut areas: Vec<f32> = rows
            .iter()
            .flat_map(|row| {
                let a_row: f32 = aspects[row.clone()].iter().sum();
                aspects[row.clone()]
                    .iter()
                    .map(move |&a_i| a_i / (a_row * a_row))
            })
            .collect();
        areas.sort_by(f32::total_cmp);
        let n = areas.len();
        let median = 0.5 * (areas[(n - 1) / 2] + areas[n / 2]);
        let min_ratio = areas[0] / median;
        assert!(
            min_ratio >= 0.55,
            "Tall crops were crushed relative to median frame area: min/median = {min_ratio:.2} (rows={rows:?})"
        );
    }

    #[test]
    fn portrait_frames_scale_without_smearing_lower_rows() {
        // Rows past the frame's width once blended with row `width - 1`.
        let mut frame = solid(100, 400, [200, 0, 0, 255]);
        frame.rgba[200 * 100 * 4..]
            .chunks_exact_mut(4)
            .for_each(|p| p.copy_from_slice(&[0, 0, 200, 255]));
        let mut sheet = solid(100, 400, [0, 0, 0, 255]);
        blit_scaled(&mut sheet, &frame, [0.0, 0.0, 100.0, 400.0], 0.0);
        let at = |x: usize, y: usize| &sheet.rgba[(y * 100 + x) * 4..][..4];
        assert_eq!(at(50, 150), [200, 0, 0, 255]);
        assert_eq!(at(50, 300), [0, 0, 200, 255]);
        assert_eq!(at(50, 399), [0, 0, 200, 255]);
    }

    #[test]
    fn unicode_wrapping_and_empty_sheet_are_safe() {
        let lines = wrap_text(
            "✎ · – “hello” 🦀 漢字 longwordwithoutspaces",
            16.0,
            80.0,
            false,
        );
        assert!(lines.len() > 2);
        assert!(lines.iter().all(|s| text_width(s, 16.0, false) <= 81.0));
        let sheet = render_contact_sheet_for_duration(8000, &[]);
        assert!(sheet.width > 0 && sheet.height > 0);
    }
}
