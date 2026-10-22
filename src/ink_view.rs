//! The ink overlay's view, shared by Windows and Linux: a click-through GPUI window over the
//! display being recorded that paints what `laser_overlay.metal` paints on macOS, from the same
//! telemetry: the picker's dim and cutout, the target's inset border, the start glow, Shift ink
//! with its fade, a pin's brief pulse and the Shift tip dot. Each platform's `InkOverlay` opens and places the
//! window (`win/overlay.rs`, `linux/overlay.rs`).

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use gpui::{
    App, Bounds, Context, Corners, Edges, Hsla, IntoElement, PathBuilder, PathStyle, Pixels, Point,
    Render, StrokeOptions, Window, canvas, point, prelude::*, px, size,
};

use crate::capture::SharedCaptureTelemetry;
use crate::sketch::{pin_pulse, slice_laser_window, smooth_ink_points, smooth_laser_points};

const TAIL_MS: u64 = 420;

/// The display an overlay covers: its rect in global pixels and its scale.
#[derive(Clone, Copy)]
pub(crate) struct Placement {
    pub rect: [f64; 4],
    pub scale: f32,
}

pub(crate) struct OverlayView {
    telemetry: Arc<SharedCaptureTelemetry>,
    /// The display it covers (moved by `InkOverlay::move_to`).
    placement: Arc<Mutex<Placement>>,
}

/// One frame's geometry, in the window's points.
struct Scene {
    target: Option<([f32; 4], f32)>,
    uniform_dim: Option<f32>,
    now_sec: f32,
    picker_fade: f32,
    glow: bool,
    ink: Vec<(Vec<[f32; 2]>, f32)>,
    laser: Vec<Vec<([f32; 2], f32)>>,
    /// Pin pulses: centre, ring radius and opacity.
    pins: Vec<([f32; 2], f32, f32)>,
    tip: Option<[f32; 2]>,
    screen: [f32; 2],
}

fn ink_color(alpha: f32) -> Hsla {
    gpui::Rgba {
        r: 1.0,
        g: 0.23,
        b: 0.19,
        a: alpha,
    }
    .into()
}

fn rgba(r: f32, g: f32, b: f32, a: f32) -> Hsla {
    gpui::Rgba { r, g, b, a }.into()
}

impl OverlayView {
    pub(crate) fn new(telemetry: Arc<SharedCaptureTelemetry>, placement: Arc<Mutex<Placement>>) -> Self {
        Self {
            telemetry,
            placement,
        }
    }

    fn scene(&self, screen: [f32; 2]) -> Option<Scene> {
        let t = &self.telemetry;
        let active = t.active.load(Ordering::Acquire);
        let selecting = t.selecting.load(Ordering::Acquire);
        let mode = t.capture_mode.load(Ordering::Acquire);
        if !active && !selecting && mode != 3 {
            return None;
        }
        let now_ms = t.elapsed_ms();
        let now_sec = if selecting {
            0.0
        } else {
            now_ms as f32 / 1000.0
        };
        let picker_fade = if selecting {
            let elapsed = t
                .picker_started_at
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .elapsed()
                .as_secs_f32();
            1.0 - (1.0 - (elapsed / 0.15).clamp(0.0, 1.0)).powi(3)
        } else {
            1.0
        };
        let placement = *self
            .placement
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let k = 1.0 / placement.scale;
        let [ox, oy] = [placement.rect[0] as f32, placement.rect[1] as f32];
        let local = |x: f32, y: f32| [(x - ox) * k, (y - oy) * k];
        let picker = if selecting {
            *t.picker_rect
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        } else {
            None
        };
        let frame = *t
            .target_frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (target, uniform_dim) = if selecting {
            match picker {
                Some(([x, y, w, h], kind)) => {
                    let [lx, ly] = local(x, y);
                    (
                        Some(([lx, ly, w.max(1.0) * k, h.max(1.0) * k], f32::from(kind))),
                        None,
                    )
                }
                None => (None, Some(picker_fade)),
            }
        } else {
            let [lx, ly] = local(frame.x as f32, frame.y as f32);
            let constrained = mode > 0 && frame.x > -10_000.0;
            let rect = [
                lx,
                ly,
                (frame.width as f32).max(1.0) * k,
                (frame.height as f32).max(1.0) * k,
            ];
            (constrained.then_some((rect, mode as f32)), None)
        };
        let to_screen = |nx: f32, ny: f32| -> [f32; 2] {
            let [lx, ly] = local(frame.x as f32, frame.y as f32);
            [
                lx + nx * frame.width as f32 * k,
                ly + ny * frame.height as f32 * k,
            ]
        };

        let mut strokes: Vec<(crate::model::Stroke, bool)> = t
            .strokes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .map(|s| (s, false))
            .collect();
        if let Some(laser) = t
            .active_laser
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            strokes.push((laser, true));
        }
        if let Some(ink) = t
            .live_ink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            strokes.push((ink, true));
        }
        let mut ink = Vec::new();
        let mut laser = Vec::new();
        let mut pins = Vec::new();
        if active {
            for (stroke, live) in &strokes {
                if stroke.pin.is_some() {
                    if let Some([nx, ny]) = stroke.normalized_points().first().copied()
                        && let Some((radius, alpha)) = pin_pulse(now_ms.saturating_sub(stroke.t1))
                    {
                        pins.push((to_screen(nx, ny), radius, alpha));
                    }
                    continue;
                }
                if stroke.laser {
                    if stroke.t1 + TAIL_MS < now_ms {
                        continue;
                    }
                    let raw =
                        slice_laser_window(&stroke.timed_points(), now_ms, TAIL_MS, to_screen);
                    laser.push(smooth_laser_points(&raw));
                    continue;
                }
                let alpha = if *live {
                    1.0
                } else {
                    let age = now_ms.saturating_sub(stroke.t1);
                    if age >= 1_300 {
                        continue;
                    } else if age <= 650 {
                        1.0
                    } else {
                        let t = ((age - 650) as f32 / 650.0).clamp(0.0, 1.0);
                        (1.0 - t) * (1.0 - t)
                    }
                };
                let pts: Vec<[f32; 2]> = stroke
                    .normalized_points()
                    .into_iter()
                    .map(|[nx, ny]| to_screen(nx, ny))
                    .collect();
                let smoothed: Vec<[f32; 2]> = smooth_ink_points(&pts)
                    .into_iter()
                    .map(|(p, _)| p)
                    .collect();
                ink.push((smoothed, alpha));
            }
        }
        let tip = (active && t.shift_held.load(Ordering::Acquire)).then(|| {
            let [x, y] = *t
                .pointer_screen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            local(x, y)
        });
        Some(Scene {
            target,
            uniform_dim,
            now_sec,
            picker_fade,
            glow: active && t.start_glow.load(Ordering::Acquire),
            ink,
            laser,
            pins,
            tip,
            screen,
        })
    }
}

fn pt(p: [f32; 2], origin: Point<Pixels>) -> Point<Pixels> {
    point(origin.x + px(p[0]), origin.y + px(p[1]))
}

fn rect_bounds(r: [f32; 4], origin: Point<Pixels>) -> Bounds<Pixels> {
    Bounds {
        origin: pt([r[0], r[1]], origin),
        size: size(px(r[2].max(0.0)), px(r[3].max(0.0))),
    }
}

/// A rounded rectangle as a closed path (for the dim's even-odd hole).
fn add_rounded_rect(builder: &mut PathBuilder, r: [f32; 4], radius: f32, origin: Point<Pixels>) {
    let [x, y, w, h] = r;
    let rad = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    if rad <= 0.0 {
        builder.add_polygon(
            &[
                pt([x, y], origin),
                pt([x + w, y], origin),
                pt([x + w, y + h], origin),
                pt([x, y + h], origin),
            ],
            true,
        );
        return;
    }
    builder.move_to(pt([x + rad, y], origin));
    builder.line_to(pt([x + w - rad, y], origin));
    builder.curve_to(pt([x + w, y + rad], origin), pt([x + w, y], origin));
    builder.line_to(pt([x + w, y + h - rad], origin));
    builder.curve_to(pt([x + w - rad, y + h], origin), pt([x + w, y + h], origin));
    builder.line_to(pt([x + rad, y + h], origin));
    builder.curve_to(pt([x, y + h - rad], origin), pt([x, y + h], origin));
    builder.line_to(pt([x, y + rad], origin));
    builder.curve_to(pt([x + rad, y], origin), pt([x, y], origin));
    builder.close();
}

fn round_stroke(width: f32) -> PathBuilder {
    let mut options = StrokeOptions::default().with_line_width(width);
    options.line_join = lyon::tessellation::LineJoin::Round;
    options.start_cap = lyon::tessellation::LineCap::Round;
    options.end_cap = lyon::tessellation::LineCap::Round;
    PathBuilder::stroke(px(width)).with_style(PathStyle::Stroke(options))
}

fn paint_polyline(
    window: &mut Window,
    points: &[[f32; 2]],
    width: f32,
    color: Hsla,
    o: Point<Pixels>,
) {
    if points.len() < 2 {
        if let Some(&p) = points.first() {
            paint_dot(window, p, width / 2.0, color, o);
        }
        return;
    }
    let mut builder = round_stroke(width);
    builder.move_to(pt(points[0], o));
    for &p in &points[1..] {
        builder.line_to(pt(p, o));
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn paint_dot(window: &mut Window, p: [f32; 2], radius: f32, color: Hsla, o: Point<Pixels>) {
    window.paint_quad(gpui::quad(
        rect_bounds(
            [p[0] - radius, p[1] - radius, radius * 2.0, radius * 2.0],
            o,
        ),
        Corners::all(px(radius)),
        color,
        Edges::default(),
        gpui::transparent_black(),
        gpui::BorderStyle::Solid,
    ));
}

fn paint_scene(scene: &Scene, bounds: Bounds<Pixels>, window: &mut Window) {
    let o = bounds.origin;
    let [sw, sh] = scene.screen;
    let intro = 1.0 - (1.0 - (scene.now_sec / 0.24).clamp(0.0, 1.0)).powi(3);
    let intro_fade = 1.0 - intro;
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;

    if let Some((rect, kind)) = scene.target {
        let device = kind > 2.5;
        let window_mode = kind > 1.5 && !device;
        let radius = if device {
            rect[2].min(rect[3]) * 0.135
        } else if window_mode {
            10.0
        } else {
            0.0
        };
        // Outside the target: 0.32 dim while picking, easing to 0.22 while recording.
        let dim = mix(0.32, 0.22, intro) * scene.picker_fade;
        let mut hole = PathBuilder::fill();
        add_rounded_rect(&mut hole, [0.0, 0.0, sw, sh], 0.0, o);
        add_rounded_rect(&mut hole, rect, radius, o);
        if let Ok(path) = hole.build() {
            window.paint_path(path, rgba(0.0, 0.0, 0.0, dim));
        }
        // Inside a picked window: the picker's blue tint, dissolving as recording starts.
        if window_mode && intro_fade > 0.002 {
            window.paint_quad(gpui::quad(
                rect_bounds(rect, o),
                Corners::all(px(radius)),
                rgba(0.231, 0.361, 0.965, 0.13 * intro_fade * scene.picker_fade),
                Edges::default(),
                gpui::transparent_black(),
                gpui::BorderStyle::Solid,
            ));
        }
        if !device {
            let width = if window_mode {
                mix(2.0, 1.0, intro)
            } else {
                1.0
            };
            let picker_border = if window_mode {
                [0.357, 0.486, 0.980]
            } else {
                [1.0, 1.0, 1.0]
            };
            let alpha = mix(if window_mode { 0.93 } else { 0.92 }, 0.28, intro) * scene.picker_fade;
            let c = [
                mix(picker_border[0], 1.0, intro),
                mix(picker_border[1], 1.0, intro),
                mix(picker_border[2], 1.0, intro),
            ];
            window.paint_quad(gpui::quad(
                rect_bounds(rect, o),
                Corners::all(px(radius)),
                gpui::transparent_black(),
                Edges::all(px(width)),
                rgba(c[0], c[1], c[2], alpha),
                gpui::BorderStyle::Solid,
            ));
        }
    } else if let Some(fade) = scene.uniform_dim {
        window.paint_quad(gpui::fill(bounds, rgba(0.0, 0.0, 0.0, 0.32 * fade)));
    } else if scene.glow {
        // A shortcut recording: a red glow along the display's edges, in by 0.12 s, gone by 1.1 s.
        let t = scene.now_sec;
        let smooth = |a: f32, b: f32, x: f32| {
            let s = ((x - a) / (b - a)).clamp(0.0, 1.0);
            s * s * (3.0 - 2.0 * s)
        };
        let env = smooth(0.0, 0.12, t) * (1.0 - smooth(0.35, 1.1, t));
        if env > 0.002 {
            for step in 0..24 {
                let d = step as f32 * 3.0;
                let a = ((-d / 22.0).exp() - (-(d + 3.0) / 22.0).exp()) * 0.45 * env;
                window.paint_quad(gpui::quad(
                    rect_bounds([d, d, sw - 2.0 * d, sh - 2.0 * d], o),
                    Corners::default(),
                    gpui::transparent_black(),
                    Edges::all(px(3.0)),
                    ink_color(a.max(0.0) * 1.8),
                    gpui::BorderStyle::Solid,
                ));
            }
        }
    } else if intro_fade > 0.002 {
        window.paint_quad(gpui::fill(bounds, rgba(0.0, 0.0, 0.0, 0.32 * intro_fade)));
    }

    // A pin is a 1.5 pt ring centred on its radius, no fill, as `laser_overlay.metal` draws it.
    for &(p, radius, alpha) in &scene.pins {
        let outer = radius + 0.75;
        window.paint_quad(gpui::quad(
            rect_bounds([p[0] - outer, p[1] - outer, outer * 2.0, outer * 2.0], o),
            Corners::all(px(outer)),
            gpui::transparent_black(),
            Edges::all(px(1.5)),
            ink_color(alpha),
            gpui::BorderStyle::Solid,
        ));
    }
    for (points, alpha) in &scene.ink {
        paint_polyline(window, points, 4.5, ink_color(*alpha), o);
    }
    for points in &scene.laser {
        for pair in points.windows(2) {
            let w = (pair[0].1 + pair[1].1) / 2.0;
            if w <= 0.002 {
                continue;
            }
            let glow = rgba(1.0, 0.16, 0.19, 0.18 * w);
            paint_polyline(
                window,
                &[pair[0].0, pair[1].0],
                12.0 * (0.28 + 0.72 * w),
                glow,
                o,
            );
            paint_polyline(
                window,
                &[pair[0].0, pair[1].0],
                4.9 * w.powf(0.68),
                rgba(1.0, 0.16, 0.19, 0.88 * w.powf(0.4)),
                o,
            );
            paint_polyline(
                window,
                &[pair[0].0, pair[1].0],
                1.8 * w.powf(0.82),
                rgba(1.0, 0.97, 0.97, 0.95 * w.powf(0.72)),
                o,
            );
        }
    }
    if let Some(tip) = scene.tip {
        paint_dot(window, tip, 2.0, ink_color(1.0), o);
    }
}

impl Render for OverlayView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let scene = self.scene([f32::from(size.width), f32::from(size.height)]);
        // Telemetry changes on other threads; follow it every frame while the overlay is up.
        window.request_animation_frame();
        canvas(
            |_, _, _| {},
            move |bounds, (), window: &mut Window, _: &mut App| {
                if let Some(scene) = scene {
                    paint_scene(&scene, bounds, window);
                }
            },
        )
        .size_full()
    }
}
