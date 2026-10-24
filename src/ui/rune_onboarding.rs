//! Scrawl's welcome window drawn with Rune, in Scrawl's own process (`"ui": "rune"`). A port of
//! `helper/Onboarding.swift` and `helper/OnboardingDemo.swift`: the same JSON state in, the same
//! actions out, the same sizes, colours, copy and timings. `onboarding::Flow` still decides
//! everything.
//!
//! The how-it-works demo is a function of one clock, as in Swift: every frame is built from the
//! demo time alone, so a render at any moment looks the same as the live loop there.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use rune::{
    Align, App, AppHandle, AppSpec, Chrome, Color, Cx, El, LineCap, Live, PointerPress, Shape, Span,
    Spring, SpringPair, Transition, Tx, WindowOptions, WindowSpec, canvas, col, el, rich, row, spacer, text,
};
use serde::Deserialize;
use serde_json::Value;

use super::rune_settings::{
    ROUNDED, Rgba, SpeechStatus, app_icon_px, families, fonts, icons, measure, permission_symbol, reduce_motion,
    style,
};
use super::rune_session::{Attached, Host, Link};
use super::rune_symbols::{symbol, weight};

type Events = Arc<dyn Fn(crate::onboarding::Event) + Send + Sync>;

const TITLE: &str = "Welcome to Scrawl";
const WIDTH: f32 = 620.0;
const HEIGHT: f32 = 500.0;

// MARK: Palette (matches the review window and HUD)

const CANVAS: Rgba = Rgba::hex(0x161616);
const SURFACE: Rgba = Rgba::hex(0x1D1D1D);
const RAISED: Rgba = Rgba::hex(0x252525);
const LINE: Rgba = Rgba::WHITE.alpha(0.09);
const TEXT: Rgba = Rgba::hex(0xF2F2F2);
const SOFT: Rgba = Rgba::hex(0xD0D0D0);
const MUTED: Rgba = Rgba::hex(0x969696);
const INK: Rgba = Rgba::rgb(1.0, 0x3B as f32 / 255.0, 0x30 as f32 / 255.0);
const POSITIVE: Rgba = Rgba::hex(0x69D89F);

// The pretend page: light, so the red ink reads as it does on a real site.
const PAGE: Rgba = Rgba::rgb(0.972, 0.972, 0.965);
const TITLE_BAR: Rgba = Rgba::rgb(0.93, 0.93, 0.925);
const D_TEXT: Rgba = Rgba::rgb(0.11, 0.11, 0.12);
const D_SECONDARY: Rgba = Rgba::rgb(0.36, 0.36, 0.38);
const PILL: Rgba = Rgba::rgb(20.0 / 255.0, 20.0 / 255.0, 22.0 / 255.0).alpha(0.96);
const ACCENT: Rgba = Rgba::hex(0x3B5CF6);
const AGENT: Rgba = Rgba::hex(0x232325);
const SHEET: Rgba = Rgba::hex(0x0E0E0E);
const STAGE_TOP: Rgba = Rgba::rgb(0.13, 0.13, 0.135);
const STAGE_BOTTOM: Rgba = Rgba::rgb(0.10, 0.10, 0.105);

/// SF's line height (ascender + descender) at `size`, plus `spacing` between lines.
fn lh(size: f32, spacing: f32) -> f32 {
    size * 1.193 + spacing
}

/// SwiftUI's measured line pitch for SF, which isn't a constant multiple at text sizes.
fn line_pitch(size: f32) -> f32 {
    match size {
        12.0 => 15.0,
        13.0 => 16.0,
        _ => size * 1.193,
    }
}

fn label(s: &str, size: f32, weight: f32, color: Color) -> El {
    text(s).font(style(size, weight)).color(color).single_line()
}

/// A stage label drawn at `k`: set at `size * k` but with `size`'s optical size, so a scaled-down
/// stage reads like SwiftUI's `scaleEffect` and not like SF's wider small optical sizes.
fn label_k(s: &str, size: f32, k: f32, weight: f32, color: Color) -> El {
    text(s).font(style(size * k, weight).optical_size(size)).color(color).single_line()
}

// MARK: State (`OnboardingState`)

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
struct Row {
    permission: String,
    status: String,
    requested: bool,
    required: bool,
    reopening: bool,
    relaunch: bool,
}

impl Default for Row {
    fn default() -> Self {
        Self {
            permission: String::new(),
            status: "not-determined".into(),
            requested: false,
            required: true,
            reopening: false,
            relaunch: false,
        }
    }
}

impl Row {
    fn granted(&self) -> bool {
        self.status == "granted"
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
struct State {
    step: String,
    index: usize,
    count: usize,
    rows: Vec<Row>,
    ready: bool,
    fix: bool,
    picker_note: bool,
    speech: Option<SpeechStatus>,
    /// The demo on how-it-works: nothing, "skipped", "watched" or "practiced".
    tutorial: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            step: "welcome".into(),
            index: 0,
            count: 4,
            rows: Vec::new(),
            ready: false,
            fix: false,
            picker_note: false,
            speech: None,
            tutorial: None,
        }
    }
}

impl State {
    /// The demo has played through (or been drawn on): its button reads Continue, not Skip.
    fn tutorial_seen(&self) -> bool {
        matches!(self.tutorial.as_deref(), Some("watched" | "practiced"))
    }

    /// A fix is shorter than the tour: it has no art or page dots.
    fn height(&self) -> f32 {
        if self.fix { 318.0 + 58.0 * self.rows.len() as f32 } else { HEIGHT }
    }

    fn speech_not_ready(&self) -> bool {
        self.speech.as_ref().is_some_and(SpeechStatus::not_ready)
    }
}

const STEP_ORDER: [&str; 5] = ["welcome", "how-it-works", "permissions", "try-it", "done"];

fn step_index(step: &str) -> usize {
    STEP_ORDER.iter().position(|s| *s == step).unwrap_or(0)
}

// MARK: Motion (springs by duration, no bounce; reduced motion keeps a short fade)

fn page_transition(forward: bool, reduce: bool) -> Transition {
    if reduce {
        return Transition::opacity().springs(SpringPair::new(Spring::new(0.18, 0.0), Spring::new(0.18, 0.0)));
    }
    let d = if forward { 24.0 } else { -24.0 };
    Transition::asymmetric(
        Transition::offset(d, 0.0).combined(Transition::opacity()),
        Transition::offset(-d, 0.0).combined(Transition::opacity()),
    )
    .springs(SpringPair::new(Spring::new(0.4, 0.0), Spring::new(0.4, 0.0)))
}

/// A status replaced in place: a slight scale and fade.
fn swap(reduce: bool) -> Transition {
    let t = if reduce { Transition::opacity() } else { Transition::scale(0.96) };
    t.springs(SpringPair::new(Spring::new(0.3, 0.0), Spring::new(0.3, 0.0)))
}

fn fade() -> Transition {
    Transition::opacity().springs(SpringPair::new(Spring::new(0.3, 0.0), Spring::new(0.3, 0.0)))
}

/// Uniform scale about the element's centre, as a presentation (layout keeps its size).
fn scaled(e: El, k: f32) -> El {
    if (k - 1.0).abs() < 1e-4 { e } else { e.live_scale(&Live::new(k)) }
}

fn clock() -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

// MARK: Demo script and helpers (`DemoScript`)

mod script {
    pub const LOOP: f64 = 11.0;
    pub const START_KEYS: f64 = 0.5;
    pub const PILL_OPEN: f64 = 0.62;
    pub const RECORD_START: f64 = 0.7;
    pub const APPROACH: (f64, f64) = (1.0, 1.9);
    pub const MARK: (f64, f64) = (2.6, 3.9);
    pub const TO_TOTAL: (f64, f64) = (4.2, 4.8);
    pub const CLICK: f64 = 4.9;
    pub const FINISH_KEYS: f64 = 5.8;
    pub const FINISH: f64 = 6.0;
    pub const TO_COMPOSER: (f64, f64) = (6.2, 6.85);
    pub const COMPOSER_CLICK: f64 = 6.9;
    pub const PASTE_KEYS: f64 = 7.0;
    pub const FLY: f64 = 7.05;
    pub const PILL_CLOSE: f64 = 7.55;
    pub const SEND: f64 = 7.95;
    pub const REPLY: f64 = 8.7;
    pub const FIX: f64 = 9.0;
    pub const END: f64 = 10.4;
    pub const RESET: f64 = 10.4;
    pub const RESET_CAPTION: f64 = 10.6;

    pub const SENTENCES: [((f64, f64), &[(&str, f64)]); 3] = [
        ((1.2, 2.5), &[("This", 1.25), ("button", 1.42), ("is", 1.62), ("cut", 1.75), ("off", 1.92)]),
        ((2.6, 4.15), &[("and", 2.7), ("too", 2.9), ("far", 3.08), ("right.", 3.28)]),
        ((4.3, 5.9), &[("Line", 4.35), ("it", 4.5), ("up", 4.62), ("with", 4.78), ("this.", 4.95)]),
    ];
    pub const TRANSCRIPT: &str = "This button is cut off and too far right. Line it up with this.";
    pub const REPLY_TEXT: &str = "Moved the button back in line.";
}

fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

fn ramp(t: f64, a: f64, b: f64) -> f64 {
    clamp01((t - a) / (b - a))
}

fn smooth(p: f64) -> f64 {
    p * p * (3.0 - 2.0 * p)
}

fn ease_in_out(p: f64) -> f64 {
    if p < 0.5 { 4.0 * p * p * p } else { 1.0 - (-2.0 * p + 2.0).powi(3) / 2.0 }
}

/// A critically damped spring (bounce 0) released at `start`, 0 → 1, settled after `duration`.
fn spring_step(t: f64, start: f64, duration: f64) -> f64 {
    let tau = t - start;
    if tau <= 0.0 {
        return 0.0;
    }
    let w = 2.0 * std::f64::consts::PI / duration;
    (1.0 - (1.0 + w * tau) * (-w * tau).exp()).min(1.0)
}

fn mixf(a: f32, b: f32, p: f64) -> f32 {
    a + (b - a) * p as f32
}

fn mixp(a: [f32; 2], b: [f32; 2], p: f64) -> [f32; 2] {
    [mixf(a[0], b[0], p), mixf(a[1], b[1], p)]
}

fn mixr(a: [f32; 4], b: [f32; 4], p: f64) -> [f32; 4] {
    [mixf(a[0], b[0], p), mixf(a[1], b[1], p), mixf(a[2], b[2], p), mixf(a[3], b[3], p)]
}

// MARK: Geometry (stage points, top-left origin; rects are x, y, w, h)

mod geo {
    pub const SIZE: [f32; 2] = [540.0, 244.0];
    pub const APP: [f32; 4] = [14.0, 50.0, 300.0, 180.0];
    pub const AGENT: [f32; 4] = [326.0, 50.0, 200.0, 180.0];
    pub const BUTTON_FIXED: [f32; 4] = [200.0, 122.0, 84.0, 26.0];
    pub const BUTTON_BROKEN: [f32; 4] = [250.0, 126.0, 84.0, 26.0];
    pub const MARK_CENTER: [f32; 2] = [292.0, 189.0];
    pub const MARK_RADIUS: [f32; 2] = [40.0, 23.0];
    pub const CLICK_POINT: [f32; 2] = [297.0, 156.0];
    pub const REST: [f32; 2] = [170.0, 118.0];
    pub const COMPOSER: [f32; 4] = [334.0, 194.0, 184.0, 28.0];
    pub const COMPOSER_POINT: [f32; 2] = [452.0, 210.0];
    pub const COMPOSER_THUMB: [f32; 4] = [339.0, 198.0, 33.3, 20.0];
    pub const BUBBLE: [f32; 4] = [348.0, 82.0, 170.0, 84.0];
    pub const BUBBLE_THUMB: [f32; 4] = [354.0, 88.0, 72.0, 43.2];
    pub const PILL_TOP: f32 = 10.0;
    pub const PILL_HEIGHT: f32 = 30.0;
    pub const PILL_CLOSED: f32 = 30.0;
    pub const PILL_RECORDING: f32 = 116.0;
    pub const PILL_COPIED: f32 = 128.0;

    /// A hand-drawn loop around the cut-off end of the button, a little more than one turn.
    pub fn mark_point(p: f64) -> [f32; 2] {
        let theta = 200.0 / 180.0 * std::f64::consts::PI + p * 2.25 * std::f64::consts::PI;
        let wobble = 1.0 + 0.07 * (3.0 * theta + 0.6).sin() + 0.05 * p;
        [
            MARK_CENTER[0] + MARK_RADIUS[0] * (theta.cos() * wobble) as f32,
            MARK_CENTER[1] + MARK_RADIUS[1] * (theta.sin() * wobble) as f32,
        ]
    }

    pub fn mark_path(upto: f64) -> Vec<[f32; 2]> {
        let steps = ((80.0 * upto) as usize).max(2);
        (0..=steps).map(|i| mark_point(upto * i as f64 / steps as f64)).collect()
    }
}

fn mid_x(r: [f32; 4]) -> f32 {
    r[0] + r[2] / 2.0
}

fn mid_y(r: [f32; 4]) -> f32 {
    r[1] + r[3] / 2.0
}

/// The stage's background at `y` (its top-to-bottom gradient), for compositing over it.
fn stage_bg(y: f32) -> Rgba {
    STAGE_TOP.mix(STAGE_BOTTOM, y / geo::SIZE[1])
}

// MARK: Drawing the stage

/// Text anchored at a point as SwiftUI's `GraphicsContext.draw(_:at:anchor:)` places it.
#[derive(Clone, Copy)]
enum Anchor {
    Leading,
    Center,
    Trailing,
}

fn at(child: El, x: f32, y: f32, anchor: Anchor, k: f32) -> El {
    let (w, h) = (320.0 * k.max(0.4), 40.0 * k.max(0.4));
    let (left, line) = match anchor {
        Anchor::Leading => (x, row().justify_start()),
        Anchor::Center => (x - w / 2.0, row().justify_center()),
        Anchor::Trailing => (x - w, row().justify_end()),
    };
    line.absolute(left, y - h / 2.0).size(w, h).items_center().child(child)
}

fn rect_el(r: [f32; 4], k: f32) -> El {
    el().absolute(r[0] * k, r[1] * k).size(r[2] * k, r[3] * k)
}

/// A polygon approximating an ellipse inside `r`.
fn ellipse(r: [f32; 4], k: f32) -> Vec<[f32; 2]> {
    let (cx, cy, rx, ry) = ((r[0] + r[2] / 2.0) * k, (r[1] + r[3] / 2.0) * k, r[2] / 2.0 * k, r[3] / 2.0 * k);
    let n = 48;
    (0..n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            [cx + rx * a.cos(), cy + ry * a.sin()]
        })
        .collect()
}

struct Scene {
    t: f64,
}

impl Scene {
    /// 0 while the button is broken, 1 once the agent has fixed it.
    fn fixed(&self) -> f64 {
        spring_step(self.t, script::FIX, 0.45) * (1.0 - spring_step(self.t, script::RESET, 0.45))
    }

    /// 0 … 1 while the loop clears back to its first frame.
    fn reset(&self) -> f64 {
        smooth(ramp(self.t, script::RESET, script::RESET + 0.35))
    }

    fn mark_progress(&self) -> f64 {
        smooth(ramp(self.t, script::MARK.0, script::MARK.1))
    }

    /// The arrow the demo moves (the person's pointer), and whether it's pressed.
    fn cursor(&self) -> ([f32; 2], bool) {
        use script::*;
        let t = self.t;
        let (mark_start, mark_end) = (geo::mark_point(0.0), geo::mark_point(1.0));
        let pressed = (CLICK..CLICK + 0.14).contains(&t) || (COMPOSER_CLICK..COMPOSER_CLICK + 0.14).contains(&t);
        let point = if t < APPROACH.0 {
            geo::REST
        } else if t < APPROACH.1 {
            mixp(geo::REST, mark_start, ease_in_out(ramp(t, APPROACH.0, APPROACH.1)))
        } else if t < MARK.0 {
            mark_start
        } else if t < MARK.1 {
            geo::mark_point(smooth(ramp(t, MARK.0, MARK.1)))
        } else if t < TO_TOTAL.0 {
            mark_end
        } else if t < TO_TOTAL.1 {
            mixp(mark_end, geo::CLICK_POINT, ease_in_out(ramp(t, TO_TOTAL.0, TO_TOTAL.1)))
        } else if t < TO_COMPOSER.0 {
            geo::CLICK_POINT
        } else if t < RESET {
            mixp(geo::CLICK_POINT, geo::COMPOSER_POINT, ease_in_out(ramp(t, TO_COMPOSER.0, TO_COMPOSER.1)))
        } else {
            mixp(geo::COMPOSER_POINT, geo::REST, ease_in_out(ramp(t, RESET, LOOP - 0.05)))
        };
        (point, pressed)
    }

    /// The lifted frame's rect on the stage, or nothing before ⌥⇧S finishes.
    fn frame_rect(&self) -> Option<[f32; 4]> {
        use script::*;
        let t = self.t;
        if t < FINISH {
            return None;
        }
        let lift = spring_step(t, FINISH, 0.38);
        let scale = 1.0 - 0.1 * lift as f32;
        let app = geo::APP;
        let lifted = [
            mid_x(app) - app[2] * scale / 2.0,
            mid_y(app) - app[3] * scale / 2.0,
            app[2] * scale,
            app[3] * scale,
        ];
        if t < FLY {
            return Some(lifted);
        }
        // Along an arc into the composer, shrinking as it goes.
        let q = spring_step(t, FLY, 0.6);
        let thumb = geo::COMPOSER_THUMB;
        let a = [mid_x(lifted), mid_y(lifted)];
        let b = [mid_x(thumb), mid_y(thumb)];
        let control = [(a[0] + b[0]) / 2.0, a[1].min(b[1]) - 70.0];
        let u = q as f32;
        let center = [
            (1.0 - u) * (1.0 - u) * a[0] + 2.0 * (1.0 - u) * u * control[0] + u * u * b[0],
            (1.0 - u) * (1.0 - u) * a[1] + 2.0 * (1.0 - u) * u * control[1] + u * u * b[1],
        ];
        let w = mixf(lifted[2], thumb[2], q);
        let h = mixf(lifted[3], thumb[3], q);
        let in_composer = [center[0] - w / 2.0, center[1] - h / 2.0, w, h];
        if t < SEND {
            return Some(in_composer);
        }
        Some(mixr(in_composer, geo::BUBBLE_THUMB, spring_step(t, SEND, 0.42)))
    }

    /// Everything on the stage at this moment, in stage points times `k`, back to front.
    fn draw(&self, show_cursor: bool, k: f32) -> Vec<El> {
        use script::*;
        let t = self.t;
        let mut out = vec![self.agent(k)];
        out.push(
            rect_el(geo::APP, k)
                .radius(8.0 * k)
                .fill(PAGE.color())
                .shadow(3.0 * k, 9.0 * k, Rgba::BLACK.alpha(0.45).color()),
        );
        out.push(page(geo::APP[0] * k, geo::APP[1] * k, k, self.fixed(), false, 0.0));

        // Live ink and the click ripple, until the frame lifts off and takes them with it.
        if t < FINISH {
            let mut shape = Shape::new();
            let mark = self.mark_progress();
            if mark > 0.0 {
                let points: Vec<[f32; 2]> = geo::mark_path(mark).iter().map(|p| [p[0] * k, p[1] * k]).collect();
                shape = shape.stroke(points, 3.2 * k, INK.color());
            }
            let r = ramp(t, CLICK, CLICK + 0.6);
            if r > 0.0 && r < 1.0 {
                let radius = 4.0 + 13.0 * (1.0 - (1.0 - r).powi(3)) as f32;
                let p = geo::CLICK_POINT;
                let ring = ellipse([p[0] - radius, p[1] - radius, radius * 2.0, radius * 2.0], k);
                shape = shape.stroke_with(ring, true, 2.0 * k, LineCap::Butt, INK.alpha(1.0 - r as f32).color());
            }
            if !shape.is_empty() {
                out.push(canvas(shape).absolute(0.0, 0.0).size(geo::SIZE[0] * k, geo::SIZE[1] * k));
            }
        }

        out.extend(self.subtitle(k));

        let reset = self.reset();
        if let Some(rect) = self.frame_rect()
            && reset < 1.0
        {
            let mut layer = el().absolute(0.0, 0.0).size(geo::SIZE[0] * k, geo::SIZE[1] * k).children(self.frame(rect, k));
            if reset > 0.0 {
                layer = layer.opacity(1.0 - reset as f32).blur(3.0 * reset as f32 * k);
            }
            out.push(layer);
        }

        if let Some(pill) = self.pill(k) {
            out.push(pill);
        }

        // The start cue: a brief red glow around the edge of the screen being recorded.
        let glow = 0.7 * ramp(t, PILL_OPEN, PILL_OPEN + 0.1) * (1.0 - smooth(ramp(t, PILL_OPEN + 0.15, PILL_OPEN + 0.8)));
        if glow > 0.0 {
            out.push(
                el().absolute(1.0 * k, 1.0 * k)
                    .size((geo::SIZE[0] - 2.0) * k, (geo::SIZE[1] - 2.0) * k)
                    .radius(11.0 * k)
                    .border(4.0 * k, INK.alpha(0.85 * glow as f32).color())
                    .blur(3.0 * k),
            );
        }

        if show_cursor {
            out.extend(cursor(self.cursor(), k));
        }
        out
    }

    fn agent(&self, k: f32) -> El {
        use script::*;
        let t = self.t;
        let g = geo::AGENT;
        let c = geo::COMPOSER;
        let mut panel = rect_el(g, k)
            .radius(8.0 * k)
            .fill(AGENT.color())
            .border(0.5 * k, Rgba::WHITE.alpha(0.09).over(AGENT))
            .shadow(3.0 * k, 8.0 * k, Rgba::BLACK.alpha(0.4).color());
        // Children are placed in the panel's own points.
        let local = |r: [f32; 4]| [r[0] - g[0], r[1] - g[1], r[2], r[3]];
        panel = panel
            .child(rect_el([11.0, 9.0, 5.0, 5.0], k).radius(2.5 * k).fill(POSITIVE.alpha(0.9).over(AGENT)))
            .child(at(label_k("Agent", 9.5, k, 600.0, SOFT.over(AGENT)), 21.0 * k, 11.5 * k, Anchor::Leading, k))
            .child(rect_el([0.0, 23.0, g[2], 0.5], k).fill(Rgba::WHITE.alpha(0.07).over(AGENT)));

        // The composer: a placeholder, then the pasted frame and words, which leave when sent.
        let focused = ramp(t, COMPOSER_CLICK, COMPOSER_CLICK + 0.15) * (1.0 - ramp(t, SEND + 0.3, SEND + 0.5));
        let field = Rgba::WHITE.alpha(0.06).over_rgba(AGENT);
        panel = panel.child(
            rect_el(local(c), k)
                .radius(8.0 * k)
                .fill(field.color())
                .border(0.75 * k, Rgba::WHITE.alpha(0.10 + 0.14 * focused as f32).over(field)),
        );
        // The placeholder is gone before the pasted words arrive, so the two never overlap.
        let pasted = ramp(t, FLY + 0.48, FLY + 0.63) * (1.0 - ramp(t, SEND, SEND + 0.12));
        let placeholder = 1.0 - ramp(t, FLY + 0.28, FLY + 0.4) * (1.0 - ramp(t, SEND + 0.3, SEND + 0.5));
        if placeholder > 0.0 {
            panel = panel.child(at(
                label_k("Ask anything", 8.5, k, 400.0, MUTED.alpha(placeholder as f32).over(field)),
                (c[0] + 10.0 - g[0]) * k,
                (mid_y(c) - g[1]) * k,
                Anchor::Leading,
                k,
            ));
        }
        if pasted > 0.0 {
            panel = panel.child(at(
                label_k("This button is cut off and too…", 8.5, k, 400.0, TEXT.alpha(pasted as f32).over(field)),
                (geo::COMPOSER_THUMB[0] + geo::COMPOSER_THUMB[2] + 6.0 - g[0]) * k,
                (mid_y(c) - g[1]) * k,
                Anchor::Leading,
                k,
            ));
        }

        // The sent message: a bubble grows in behind the frame as it moves up, then the words.
        // At the end of the loop the whole conversation clears in place.
        let bubble = spring_step(t, SEND + 0.05, 0.4);
        let reset = self.reset();
        if bubble > 0.0 && reset < 1.0 {
            let mut convo = el().absolute(0.0, 0.0).size(g[2] * k, g[3] * k).children(self.conversation(bubble, k));
            if reset > 0.0 {
                convo = convo.opacity(1.0 - reset as f32).blur(3.0 * reset as f32 * k);
            }
            panel = panel.child(convo);
        }
        panel
    }

    fn conversation(&self, bubble: f64, k: f32) -> Vec<El> {
        use script::*;
        let t = self.t;
        let g = geo::AGENT;
        let b = geo::BUBBLE;
        let fill = Rgba::WHITE.alpha(0.09).over_rgba(AGENT);
        // Grows from its top-right corner.
        let s = 0.96 + 0.04 * bubble as f32;
        let grown = [b[0] + b[2] * (1.0 - s) - g[0], b[1] - g[1], b[2] * s, b[3] * s];
        let mut out = vec![rect_el(grown, k).radius(9.0 * k).fill(fill.color()).opacity(bubble as f32)];
        let words = ramp(t, SEND + 0.3, SEND + 0.5);
        if words > 0.0 {
            out.push(
                el().absolute((b[0] + 6.0 - g[0]) * k, (geo::BUBBLE_THUMB[1] + geo::BUBBLE_THUMB[3] + 4.0 + 3.0 * (1.0 - words) as f32 - g[1]) * k)
                    .w((b[2] - 12.0) * k)
                    .child(
                        text(TRANSCRIPT)
                            .font(style(8.0 * k, 400.0).optical_size(8.0).lh(lh(8.0 * k, 0.0)))
                            .color(TEXT.alpha(words as f32).over(fill))
                            .wrap()
                            .w_full(),
                    ),
            );
        }
        let reply = ramp(t, REPLY, REPLY + 0.25);
        if reply > 0.0 {
            out.push(
                el().absolute(10.0 * k, (171.0 + 4.0 * (1.0 - smooth(reply)) as f32 - g[1]) * k).w(160.0 * k).child(
                    text(REPLY_TEXT)
                        .font(style(8.0 * k, 400.0).optical_size(8.0).lh(lh(8.0 * k, 0.0)))
                        .color(SOFT.alpha(reply as f32).over(AGENT))
                        .wrap()
                        .w_full(),
                ),
            );
        }
        out
    }

    /// What the person is saying, word by word, over the bottom of the page.
    fn subtitle(&self, k: f32) -> Vec<El> {
        let t = self.t;
        let mut out = Vec::new();
        for ((a, b), words) in script::SENTENCES {
            let alpha = ramp(t, a, a + 0.15) * (1.0 - ramp(t, b - 0.12, b));
            if alpha <= 0.0 {
                continue;
            }
            let box_bg = Rgba::BLACK.alpha(0.78).over_rgba(PAGE);
            let mut line = String::new();
            let mut spans = Vec::new();
            for (i, (word, said_at)) in words.iter().enumerate() {
                if i > 0 {
                    line.push(' ');
                }
                let start = line.len();
                line.push_str(word);
                let said = 0.3 + 0.7 * ramp(t, said_at - 0.04, said_at + 0.08);
                let mut span = Span::new(start, line.len());
                span.color = Some(Rgba::WHITE.alpha(said as f32).over(box_bg));
                spans.push(span);
            }
            let font = style(9.5, 500.0);
            let width = measure(&line, font) + 32.0;
            let center = [mid_x(geo::APP), 214.0];
            let boxed = [center[0] - width / 2.0, center[1] - 9.0, width, 18.0];
            let mic = el()
                .absolute(9.0 * k, 0.0)
                .size(6.0 * k, 18.0 * k)
                .center()
                .child(symbol(&icons::MIC_FILL, 7.5 * k, weight::REGULAR, INK.color()));
            let words = el().absolute(21.0 * k, 0.0).h(18.0 * k).items_center().child(
                rich(line, spans.into_iter().map(|mut s| {
                    s.size = Some(9.5 * k);
                    s
                }).collect())
                .font(style(9.5 * k, 500.0).optical_size(9.5))
                .color(Rgba::WHITE.over(box_bg))
                .single_line(),
            );
            let mut e = rect_el(boxed, k).radius(9.0 * k).fill(box_bg.color()).child(mic).child(words).opacity(alpha as f32);
            if alpha < 1.0 {
                e = e.blur(3.0 * (1.0 - alpha as f32) * k);
            }
            out.push(scaled(e, 0.96 + 0.04 * alpha as f32));
        }
        out
    }

    /// The captured frame: it starts exactly over the page, then lifts off as a one-frame
    /// contact sheet (dark margin, `1` chip) and travels into the agent's chat.
    fn frame(&self, rect: [f32; 4], k: f32) -> Vec<El> {
        use script::*;
        let t = self.t;
        let lift = spring_step(t, FINISH, 0.38) as f32;
        let scale = rect[2] / geo::APP[2];
        let margin = 6.0;
        let sheet = [
            rect[0] - margin * scale,
            rect[1] - margin * scale,
            (geo::APP[2] + 2.0 * margin) * scale,
            (geo::APP[3] + 2.0 * margin) * scale,
        ];
        let badge = ramp(t, FINISH + 0.1, FINISH + 0.3) as f32;
        vec![
            rect_el(sheet, k)
                .radius(3.0 * scale * k)
                .fill(SHEET.alpha(lift).color())
                .shadow(5.0 * scale * k, 12.0 * scale * k, Rgba::BLACK.alpha(0.5 * lift).color()),
            page(rect[0] * k, rect[1] * k, scale * k, 0.0, true, badge),
        ]
    }

    /// The recording pill, drawn like the real one: a black capsule that opens out of a dot,
    /// counts while recording, turns to Copied when finished and closes after the paste.
    fn pill(&self, k: f32) -> Option<El> {
        use script::*;
        let t = self.t;
        if t < PILL_OPEN {
            return None;
        }
        let open = spring_step(t, PILL_OPEN, 0.42);
        let copied = if t >= FINISH { spring_step(t, FINISH, 0.38) } else { 0.0 };
        // Closing, the contents fade first and the capsule shrinks right after, so nothing is cut.
        let close = smooth(ramp(t, PILL_CLOSE + 0.06, PILL_CLOSE + 0.3));
        let contents_out = ramp(t, PILL_CLOSE, PILL_CLOSE + 0.08);
        if close >= 1.0 {
            return None;
        }
        let mut width = mixf(geo::PILL_CLOSED, geo::PILL_RECORDING, open);
        width = mixf(width, geo::PILL_COPIED, copied);
        width = mixf(width, geo::PILL_CLOSED, close);
        let alpha = ramp(t, PILL_OPEN, PILL_OPEN + 0.15) * (1.0 - close);
        let scale = ((0.9 + 0.1 * open) * (1.0 - 0.1 * close)) as f32;
        let h = geo::PILL_HEIGHT;
        let cx = geo::SIZE[0] / 2.0;
        let shell = [cx - width / 2.0, geo::PILL_TOP, width, h];
        let bg = stage_bg(geo::PILL_TOP + h / 2.0);
        let fill = PILL.over_rgba(bg);
        let edge = Rgba::WHITE.alpha(0.12 * (1.0 - copied as f32)).mix(POSITIVE.alpha(0.34), copied as f32);

        let mut capsule = rect_el(shell, k)
            .radius(h / 2.0 * k)
            .fill(fill.color())
            .border(1.0 * k, edge.over(fill))
            .shadow(2.5 * k, 7.0 * k, Rgba::BLACK.alpha(0.4).color())
            .clip_rounded(h / 2.0 * k);
        let out = if t >= FINISH { ramp(t, FINISH, FINISH + 0.12) } else { 0.0 };
        let into = if t >= FINISH { ramp(t, FINISH + 0.04, FINISH + 0.22) } else { 0.0 };
        if out < 1.0 {
            // Opening, the contents come in once the capsule is nearly wide enough for them.
            let a = (1.0 - out) * ramp(t, PILL_OPEN + 0.14, PILL_OPEN + 0.3);
            capsule = capsule.child(pill_content(t, false, width, a, fill, k));
        }
        if into > 0.0 && contents_out < 1.0 {
            capsule = capsule.child(pill_content(t, true, width, into * (1.0 - contents_out), fill, k));
        }
        Some(scaled(capsule.opacity(alpha as f32), scale))
    }
}

fn pill_content(t: f64, copied: bool, width: f32, alpha: f64, fill: Rgba, k: f32) -> El {
    use script::*;
    let full = if copied { geo::PILL_COPIED } else { geo::PILL_RECORDING };
    // In the capsule's own points: the contents are laid out for its full width, centred.
    let x0 = (width - full) / 2.0;
    let cy = geo::PILL_HEIGHT / 2.0;
    let slot = [x0 + 3.0 + 12.0, cy];
    let mut layer = el().absolute(0.0, 0.0).size(width * k, geo::PILL_HEIGHT * k);
    let mut shape = Shape::new();
    if copied {
        shape = shape.fill(ellipse([slot[0] - 7.0, slot[1] - 7.0, 14.0, 14.0], k), POSITIVE.alpha(0.18).over(fill));
        let check = vec![
            [(slot[0] - 3.2) * k, (slot[1] + 0.2) * k],
            [(slot[0] - 0.9) * k, (slot[1] + 2.5) * k],
            [(slot[0] + 3.4) * k, (slot[1] - 2.4) * k],
        ];
        shape = shape.stroke(check, 1.4 * k, POSITIVE.color());
        layer = layer.child(at(label_k("Copied", 10.0, k, 600.0, TEXT.over(fill)), (x0 + 27.0) * k, cy * k, Anchor::Leading, k));
    } else {
        let beat = ((t - RECORD_START).rem_euclid(1.6) / 1.6) as f32;
        let halo = 7.5 * (0.4 + 0.6 * beat);
        shape = shape
            .fill(ellipse([slot[0] - halo, slot[1] - halo, halo * 2.0, halo * 2.0], k), INK.alpha(0.34 * (1.0 - beat)).over(fill))
            .fill(ellipse([slot[0] - 3.0, slot[1] - 3.0, 6.0, 6.0], k), INK.color());
        let seconds = (t.min(FINISH) - RECORD_START).max(0.0) as i64;
        layer = layer.child(at(
            text(format!("0:0{seconds}")).font(style(10.0 * k, 600.0).optical_size(10.0).features("'tnum' 1")).color(TEXT.over(fill)).single_line(),
            (x0 + 27.0) * k,
            cy * k,
            Anchor::Leading,
            k,
        ));
    }
    let divider_x = x0 + 3.0 + 24.0 + if copied { 38.0 } else { 24.0 } + 6.0 + 4.5;
    layer = layer
        .child(canvas(shape).absolute(0.0, 0.0).size(width * k, geo::PILL_HEIGHT * k))
        .child(rect_el([divider_x, cy - 6.0, 1.0, 12.0], k).fill(Rgba::WHITE.alpha(0.1).over(fill)));
    let chip_w = if copied { 44.0 } else { 46.0 };
    let pressed_at = if copied { PASTE_KEYS } else { FINISH_KEYS };
    let press = if (pressed_at..pressed_at + 0.18).contains(&t) { 0.92 } else { 1.0 };
    let chip = [x0 + full - 3.0 - chip_w, cy - 12.0, chip_w, 24.0];
    let pressed = [
        mid_x(chip) - chip[2] * press / 2.0,
        mid_y(chip) - chip[3] * press / 2.0,
        chip[2] * press,
        chip[3] * press,
    ];
    layer = layer.child(
        rect_el(pressed, k)
            .radius(12.0 * press * k)
            .fill(ACCENT.color())
            .center()
            .child(label_k(if copied { "Paste" } else { "Finish" }, 9.5 * press, k, 600.0, TEXT.over(ACCENT))),
    );
    let alpha = alpha as f32;
    if alpha < 1.0 {
        layer = scaled(layer.opacity(alpha).blur(3.0 * (1.0 - alpha) * k), 0.96 + 0.04 * alpha);
    }
    layer
}

/// The pretend checkout page at `scale`, its top-left at (`x`, `y`): the fix's progress, and for
/// the lifted frame the mark and the keyframe's `1` chip.
fn page(x: f32, y: f32, k: f32, fixed: f64, frame: bool, badge: f32) -> El {
    let (w, h) = (geo::APP[2], geo::APP[3]);
    let mut p = el().absolute(x, y).size(w * k, h * k).radius(8.0 * k).fill(PAGE.color()).clip_rounded(8.0 * k);
    let secondary = D_SECONDARY.over(PAGE);
    let dark = D_TEXT.over(PAGE);
    p = p
        .child(rect_el([0.0, 0.0, w, 22.0], k).fill(TITLE_BAR.color()))
        .child(rect_el([0.0, 22.0, w, 0.5], k).fill(Rgba::BLACK.alpha(0.1).over(TITLE_BAR)));
    let mut dots = Shape::new();
    for (i, c) in [Rgba::rgb(1.0, 0.37, 0.34), Rgba::rgb(1.0, 0.74, 0.18), Rgba::rgb(0.16, 0.78, 0.25)].iter().enumerate() {
        let cx = 11.0 + i as f32 * 10.0;
        dots = dots.fill(ellipse([cx - 3.2, 11.0 - 3.2, 6.4, 6.4], k), c.alpha(0.9).over(TITLE_BAR));
    }
    p = p
        .child(canvas(dots).absolute(0.0, 0.0).size(w * k, 22.0 * k))
        .child(at(label_k("Checkout", 9.0, k, 500.0, D_SECONDARY.over(TITLE_BAR)), w / 2.0 * k, 11.0 * k, Anchor::Center, k))
        .child(at(label_k("Order summary", 11.0, k, 600.0, dark), 16.0 * k, 40.0 * k, Anchor::Leading, k));
    let tnum = |size: f32, weight: f32, color: Color, s: &str| {
        text(s).font(style(size * k, weight).optical_size(size).features("'tnum' 1")).color(color).single_line()
    };
    for (name, price, ry) in [("Notebook", "$24.00", 62.0), ("Shipping", "$4.00", 80.0)] {
        p = p
            .child(at(label_k(name, 9.5, k, 400.0, secondary), 16.0 * k, ry * k, Anchor::Leading, k))
            .child(at(tnum(9.5, 400.0, secondary, price), 284.0 * k, ry * k, Anchor::Trailing, k));
    }
    p = p
        .child(rect_el([16.0, 92.0, 268.0, 0.5], k).fill(Rgba::BLACK.alpha(0.12).over(PAGE)))
        .child(at(label_k("Total", 10.0, k, 600.0, dark), 16.0 * k, 106.0 * k, Anchor::Leading, k))
        .child(at(tnum(10.0, 600.0, dark, "$28.00"), 284.0 * k, 106.0 * k, Anchor::Trailing, k));
    let button = mixr(geo::BUTTON_BROKEN, geo::BUTTON_FIXED, fixed);
    p = p.child(
        rect_el(button, k)
            .radius(7.0 * k)
            .fill(D_TEXT.color())
            .center()
            .child(label_k("Pay now", 9.5, k, 600.0, Rgba::WHITE.over(D_TEXT))),
    );
    if frame {
        let points: Vec<[f32; 2]> =
            geo::mark_path(1.0).iter().map(|q| [(q[0] - geo::APP[0]) * k, (q[1] - geo::APP[1]) * k]).collect();
        p = p.child(canvas(Shape::new().stroke(points, 3.2 * k, INK.color())).absolute(0.0, 0.0).size(w * k, h * k));
        if badge > 0.0 {
            // The keyframe's number in a small white chip with a thin dark ring, top left, like
            // render.rs.
            p = p.child(
                rect_el([4.0 - 1.2, 4.0 - 1.2, 17.4, 17.4], k)
                    .radius(4.2 * k)
                    .fill(Rgba::gray(0.07).color())
                    .center()
                    .opacity(badge)
                    .child(
                        el().size(15.0 * k, 15.0 * k)
                            .radius(3.0 * k)
                            .fill(Rgba::gray(0.98).color())
                            .center()
                            .child(label_k("1", 10.0, k, 700.0, Rgba::gray(0.07).color())),
                    ),
            );
        }
    }
    p.child(rect_el([0.0, 0.0, w, h], k).radius(8.0 * k).border(0.5 * k, Rgba::BLACK.alpha(0.25).over(PAGE)))
}

/// The macOS arrow, tip at the origin.
const ARROW: [[f32; 2]; 7] = [[0.0, 0.0], [0.0, 15.5], [3.7, 12.0], [6.3, 17.8], [8.8, 16.7], [6.3, 11.1], [11.3, 10.9]];

fn cursor((point, pressed): ([f32; 2], bool), k: f32) -> Vec<El> {
    let s = if pressed { 0.88 } else { 1.0 } * k;
    let pad = 2.0;
    let pts: Vec<[f32; 2]> = ARROW.iter().map(|p| [pad + p[0] * s, pad + p[1] * s]).collect();
    let (w, h) = (12.0 * k + 2.0 * pad, 18.0 * k + 2.0 * pad);
    let (x, y) = (point[0] * k - pad, point[1] * k - pad);
    vec![
        canvas(Shape::new().fill(pts.clone(), Rgba::BLACK.alpha(0.35).color()))
            .absolute(x, y + 1.0 * k)
            .size(w, h)
            .blur(1.0 * k),
        canvas(
            Shape::new()
                .fill(pts.clone(), Rgba::BLACK.color())
                .stroke_with(pts, true, 1.2 * k, LineCap::Round, Rgba::WHITE.color()),
        )
        .absolute(x, y)
        .size(w, h),
    ]
}

// MARK: Captions (`DemoCaptions`)

#[derive(Clone)]
enum Part {
    Text(&'static str),
    Keys(String, (f64, f64)),
    Symbol(&'static rune::Icon),
}

struct Beat {
    start: f64,
    parts: Vec<Part>,
}

fn beats(shortcut: &str, paste: &str) -> Vec<Beat> {
    use script::*;
    let start = vec![Part::Text("Tap"), Part::Keys(shortcut.into(), (START_KEYS, START_KEYS + 0.35)), Part::Text("to start recording")];
    vec![
        Beat { start: 0.0, parts: start.clone() },
        Beat { start: 1.1, parts: vec![Part::Symbol(&icons::MIC_FILL), Part::Text("Talk as you go")] },
        Beat {
            start: 2.4,
            parts: vec![Part::Text("Hold"), Part::Keys("⇧".into(), (2.55, 3.95)), Part::Text("and move the mouse to mark")],
        },
        Beat { start: 4.2, parts: vec![Part::Symbol(&icons::CURSOR_CLICK), Part::Text("Click to point at something")] },
        Beat {
            start: 5.5,
            parts: vec![Part::Text("Tap"), Part::Keys(shortcut.into(), (FINISH_KEYS, FINISH_KEYS + 0.3)), Part::Text("again to finish")],
        },
        Beat {
            start: 6.6,
            parts: vec![Part::Text("Press"), Part::Keys(paste.into(), (PASTE_KEYS, PASTE_KEYS + 0.3)), Part::Text("to paste into your agent")],
        },
        Beat { start: 8.2, parts: vec![Part::Text("Your agent sees your mark and reads what you said")] },
        Beat { start: RESET_CAPTION, parts: start },
    ]
}

fn keycap(label_: &str, lit: bool, ring: bool, size: f32, bg: Rgba) -> El {
    let top = Rgba::WHITE.alpha(0.13).over_rgba(bg);
    let bottom = Rgba::WHITE.alpha(0.06).over_rgba(bg);
    let middle = top.mix(bottom, 0.5);
    let edge = if lit || ring { INK.alpha(0.7).over(middle) } else { Rgba::WHITE.alpha(0.14).over(middle) };
    let multi = label_.chars().count() > 1;
    el().h(size)
        .min_w(size)
        .px(if multi { 8.0 } else { 0.0 })
        .center()
        .radius(size * 0.24)
        .gradient(top.color(), bottom.color())
        .border(1.0, edge)
        .shadow(2.0, 3.0, Rgba::BLACK.alpha(0.45).color())
        .child(
            text(label_)
                .font(style(size * 0.44, 500.0).family(ROUNDED))
                .color(if lit { INK.over(middle) } else { TEXT.over(middle) })
                .single_line(),
        )
}

/// Each key of `shortcut` as a keycap; `lit_mods` lights all but the last.
fn keycaps(shortcut: &str, lit_mods: bool, size: f32, bg: Rgba) -> El {
    let chars: Vec<String> = shortcut.chars().map(String::from).collect();
    let n = chars.len();
    row().gap(size * 0.18).center_y().children(
        chars.iter().enumerate().map(|(i, c)| keycap(c, i + 1 != n && lit_mods, false, size, bg)).collect::<Vec<_>>(),
    )
}

fn caption_line(beat: &Beat, t: f64) -> El {
    let mut line = row().gap(7.0).center_y();
    for part in &beat.parts {
        line = line.child(match part {
            Part::Text(s) => label(s, 13.5, 400.0, SOFT.over(CANVAS)),
            Part::Symbol(icon) => symbol(icon, 12.0, weight::MEDIUM, MUTED.color()),
            Part::Keys(keys, (lo, hi)) => {
                let on = t >= *lo && t <= *hi;
                let caps = if keys.chars().count() == 1 {
                    // A single key (⇧) lights as a whole.
                    keycap(keys, false, on, 22.0, CANVAS)
                } else {
                    keycaps(keys, on, 22.0, CANVAS)
                };
                scaled(caps, if on && t - lo < 0.12 { 0.94 } else { 1.0 })
            }
        });
    }
    line
}

/// The current step's caption; a new one slides up from under the row's lower edge.
fn captions(beats: &[Beat], t: f64) -> El {
    let index = beats.iter().rposition(|b| b.start <= t).unwrap_or(0);
    let beat = &beats[index];
    let p = if index == 0 { 1.0 } else { spring_step(t, beat.start, 0.35) } as f32;
    let place = |e: El, dy: f32, o: f32| el().absolute(0.0, dy).w(540.0).h(28.0).center().opacity(o).child(e);
    let mut out = el().w(540.0).h(28.0).clip_fade(0.0, 5.0, 0.0, 5.0);
    if p < 1.0 && index > 0 {
        out = out.child(place(caption_line(&beats[index - 1], t), -16.0 * p, 1.0 - p));
    }
    out.child(place(caption_line(beat, t), 16.0 * (1.0 - p), p))
}

// MARK: The window's state

#[derive(Clone, Copy)]
struct Follow {
    started: f64,
    asking: bool,
    returned: bool,
    next_check: f64,
}

pub(crate) struct OnboardingWin {
    state: State,
    /// The last step change went forward, so pages slide in from the trailing edge.
    forward: bool,
    shortcut: String,
    paste: String,
    icon: Option<rune::Pixels>,
    events: Option<Events>,
    now: f64,
    /// Renders: loops hold this phase (0…1) instead of running.
    render: Option<f64>,
    reduce: bool,
    storyboard: bool,
    focused: bool,
    visible: bool,
    /// State changes animate; the demo's frames until then don't cut them short.
    calm_until: f64,
    // The demo's clock: `base` plus the time since `since` (nothing while paused).
    base: f64,
    since: Option<f64>,
    strokes: Vec<Vec<[f32; 2]>>,
    drawing: bool,
    practiced: bool,
    /// When the mark starts to fade, after the stroke ends.
    fade_at: Option<f64>,
    /// The moment a rewind left, and when, fading out over the one it went to.
    ghost: Option<(f64, f64)>,
    reported_watched: bool,
    /// The stage (or storyboard) in window points, for pointer positions.
    stage: [f32; 4],
    /// Each row button's frame in global screen points, where the drag panel flies from.
    buttons: HashMap<String, [f32; 4]>,
    follow: Option<Follow>,
    sized: bool,
    /// The session driving the window (`rune_session`).
    session: u64,
}

impl Attached for OnboardingWin {
    fn session(&self) -> u64 {
        self.session
    }

    fn set_session(&mut self, id: u64) {
        self.session = id;
    }
}

impl OnboardingWin {
    fn new() -> Self {
        Self {
            state: State::default(),
            forward: true,
            shortcut: "⌥⇧S".into(),
            paste: "⌥⇧V".into(),
            icon: app_icon_px(256),
            events: None,
            now: clock(),
            render: None,
            reduce: reduce_motion(),
            storyboard: false,
            focused: false,
            visible: false,
            calm_until: 0.0,
            base: 0.0,
            since: Some(clock()),
            strokes: Vec::new(),
            drawing: false,
            practiced: false,
            fade_at: None,
            ghost: None,
            reported_watched: false,
            stage: [0.0; 4],
            buttons: HashMap::new(),
            follow: None,
            sized: false,
            session: 0,
        }
    }

    fn storyboard(&self) -> bool {
        self.reduce || self.storyboard
    }

    fn send(&self, action: &str) {
        if let Some(events) = &self.events {
            events(crate::onboarding::Event::Action(action.to_owned()));
        }
    }

    /// The phase (0…1) of a loop `period` seconds long.
    fn phase(&self, period: f64) -> f64 {
        match self.render {
            Some(p) => p,
            None if self.reduce => 0.3,
            None => self.now.rem_euclid(period) / period,
        }
    }

    fn elapsed(&self) -> f64 {
        self.base + self.since.map_or(0.0, |s| self.now - s)
    }

    /// The demo's time in its loop.
    fn demo_t(&self) -> f64 {
        match self.render {
            Some(p) => p * script::LOOP,
            None => self.elapsed().rem_euclid(script::LOOP),
        }
    }

    fn restart_demo(&mut self) {
        self.base = 0.0;
        self.since = Some(self.now);
        self.strokes.clear();
        self.drawing = false;
        self.practiced = self.state.tutorial.as_deref() == Some("practiced");
        self.fade_at = None;
        self.ghost = None;
        self.reported_watched = false;
        if self.storyboard() && self.render.is_none() {
            self.report_watched(true);
        }
    }

    /// A message from `onboarding::Session`'s `state`: `Onboarding.apply`.
    fn apply(&mut self, state: State, tx: &mut Tx) {
        self.now = clock();
        #[cfg(target_os = "macos")]
        for row in state.rows.iter().filter(|r| r.granted()) {
            super::rune_permissions::close_guide(&row.permission);
        }
        // A grant usually lands in System Settings (or a macOS prompt): come back in front.
        let was_off: Vec<&str> = self.state.rows.iter().filter(|r| !r.granted()).map(|r| r.permission.as_str()).collect();
        let newly = state.rows.iter().any(|r| r.granted() && was_off.contains(&r.permission.as_str()));
        let entering_demo = state.step == "how-it-works" && self.state.step != "how-it-works";
        self.state = state;
        if entering_demo {
            self.restart_demo();
        }
        if newly {
            self.return_after_grant();
        }
        self.calm_until = self.now + 0.45;
        let spring = if self.reduce { Spring::new(0.18, 0.0) } else { Spring::new(0.3, 0.0) };
        tx.motion(spring);
        tx.color_motion(spring);
        tx.wake_after(Duration::from_millis(16));
    }

    fn present(&mut self, tx: &mut Tx) {
        self.visible = true;
        take_keys();
        tx.wake_after(Duration::from_millis(16));
    }

    /// `returnAfterGrant`: comes forward once macOS has stopped asking. A Screen Recording grant
    /// brings System Settings' own "Quit & Reopen" alert and ScreenCaptureKit's window picker
    /// question, and answering one hands the keys back to System Settings, so for five minutes
    /// after a grant the window comes forward again each time the last question on screen is
    /// answered, until the user presses another row.
    fn return_after_grant(&mut self) {
        let asking = macos_is_asking();
        let mut follow = Follow { started: self.now, asking, returned: false, next_check: self.now + 0.3 };
        if !asking && self.visible {
            self.bring_forward();
            follow.returned = true;
        }
        self.follow = Some(follow);
    }

    fn tick_follow(&mut self) {
        let Some(mut f) = self.follow else { return };
        if self.now < f.next_check {
            return;
        }
        f.next_check = self.now + 0.3;
        if self.now - f.started >= 300.0 || !self.visible {
            self.follow = None;
            return;
        }
        if macos_is_asking() {
            f.asking = true;
        } else if f.asking || !f.returned {
            f.asking = false;
            f.returned = true;
            if !self.focused {
                self.bring_forward();
            }
        }
        self.follow = Some(f);
    }

    fn bring_forward(&self) {
        take_keys();
        if let Some(handle) = HANDLE.get() {
            handle.open();
        }
    }

    /// The row button's frame on screen, in global top-left points.
    fn screen_frame(&self, permission: &str) -> Option<[f32; 4]> {
        self.buttons.get(permission).copied()
    }

    // MARK: Trying a mark

    fn pointer(&mut self, x: f32, y: f32) {
        let [sx, sy, sw, sh] = self.stage;
        let p = [x - sx, y - sy];
        if p[0] < 0.0 || p[1] < 0.0 || p[0] > sw || p[1] > sh {
            self.end_stroke();
            return;
        }
        if shift_down() {
            self.draw_to(p);
        } else {
            self.end_stroke();
        }
    }

    fn draw_to(&mut self, p: [f32; 2]) {
        if !self.drawing {
            if self.fade_at.is_some_and(|at| self.now >= at) {
                self.strokes.clear();
            }
            self.fade_at = None;
            self.drawing = true;
            self.strokes.push(vec![p]);
            self.pause();
            return;
        }
        let Some(stroke) = self.strokes.last_mut() else { return };
        let last = stroke[stroke.len() - 1];
        if (p[0] - last[0]).hypot(p[1] - last[1]) >= 1.5 {
            stroke.push(p);
        }
    }

    fn end_stroke(&mut self) {
        if !self.drawing {
            return;
        }
        self.drawing = false;
        let stroke = self.strokes.last().cloned().unwrap_or_default();
        let length: f32 = stroke.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
        if length >= 40.0 {
            self.practiced = true;
            if self.state.tutorial.as_deref() != Some("practiced") {
                self.send("practiced");
            }
        }
        // The mark stays a moment, fades, and the demo carries on from where it stopped.
        self.fade_at = Some(self.now + 2.2);
    }

    /// Stops the loop for a mark. While the first words show and nothing is drawn yet it stops
    /// where it is; anywhere else it goes back to that moment (2.1 s: recording, "This button is
    /// cut off"), softly, so the mark goes on the page rather than on the lifted frame or the chat.
    fn pause(&mut self) {
        if self.since.is_none() || self.storyboard() {
            return;
        }
        let total = self.elapsed();
        let t = total.rem_euclid(script::LOOP);
        self.since = None;
        if (1.4..2.35).contains(&t) {
            self.base = total;
        } else {
            self.base = total - t + 2.1;
            self.ghost = Some((t, self.now));
        }
    }

    fn ink_opacity(&self) -> f32 {
        match self.fade_at {
            Some(at) if self.now > at => {
                let p = ((self.now - at) / 0.25).clamp(0.0, 1.0);
                (1.0 - (1.0 - (1.0 - p).powi(2))) as f32
            }
            _ => 1.0,
        }
    }

    fn report_watched(&mut self, force: bool) {
        if self.reported_watched || self.state.tutorial_seen() {
            return;
        }
        if force || self.elapsed() >= script::END {
            self.reported_watched = true;
            self.send("watched");
        }
    }

    /// The demo between frames; whether anything changed.
    fn tick_demo(&mut self) -> bool {
        let mut changed = false;
        if self.drawing && !shift_down() {
            self.end_stroke();
            changed = true;
        }
        if let Some(at) = self.fade_at
            && self.now >= at + 0.27
        {
            if !self.drawing {
                self.strokes.clear();
                if !self.storyboard() && self.since.is_none() {
                    self.since = Some(self.now);
                }
            }
            self.fade_at = None;
            changed = true;
        }
        if self.ghost.is_some_and(|(_, at)| self.now - at >= 0.3) {
            self.ghost = None;
            changed = true;
        }
        self.report_watched(false);
        changed
    }

    /// Something on screen moves by itself this frame.
    fn animating(&self) -> bool {
        if !self.visible {
            return false;
        }
        let spinner = self.state.rows.iter().any(|r| {
            (r.granted() && r.reopening) || (!r.granted() && r.requested && (r.status != "denied" || draggable(&r.permission)))
        });
        let looping = !self.reduce && matches!(self.state.step.as_str(), "welcome" | "try-it");
        let demo = self.state.step == "how-it-works"
            && (!self.storyboard() || self.drawing || self.fade_at.is_some() || self.ghost.is_some());
        spinner || looping || demo || self.fade_at.is_some()
    }
}

fn draggable(permission: &str) -> bool {
    matches!(permission, "screen-recording" | "accessibility")
}

fn shift_down() -> bool {
    crate::capture::modifier_flags() & (1 << 17) != 0
}

fn macos_is_asking() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::windows::macos_is_asking(&objc2::rc::autoreleasepool(|_| crate::windows::raw_windows()))
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// What System Settings calls the Accessibility list. macOS 27 renamed it.
pub(crate) fn accessibility_name() -> &'static str {
    static NAME: OnceLock<&'static str> = OnceLock::new();
    NAME.get_or_init(|| if macos_major() >= 27 { "Device Control and Data Access" } else { "Accessibility" })
}

fn macos_major() -> u32 {
    let mut buf = [0u8; 32];
    let mut len = buf.len();
    // SAFETY: a fixed buffer and its length for a string sysctl.
    let ok = unsafe {
        libc::sysctlbyname(c"kern.osproductversion".as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0)
    } == 0;
    if !ok {
        return 0;
    }
    let s = String::from_utf8_lossy(&buf[..len.saturating_sub(1)]);
    s.split('.').next().and_then(|m| m.trim_end_matches('\0').parse().ok()).unwrap_or(0)
}

fn permission_info(permission: &str, paste: &str) -> (&'static rune::Icon, String, String) {
    let icon = permission_symbol(permission);
    let (name, why) = match permission {
        "screen-recording" => ("Screen & System Audio Recording".to_owned(), "See your screen while you record".to_owned()),
        "microphone" => ("Microphone".to_owned(), "Hear what you say while you record".to_owned()),
        "accessibility" => (accessibility_name().to_owned(), format!("Undo with ⌘Z while you record, and paste with {paste}")),
        "camera" => ("Camera".to_owned(), "Show and record a cabled iPhone".to_owned()),
        "speech-recognition" => ("Speech Recognition".to_owned(), "Write down what you say, on this Mac".to_owned()),
        other => (other.to_owned(), String::new()),
    };
    (icon, name, why)
}

// MARK: Building blocks

fn primary_button(title: &str, enabled: bool, quiet: bool, action: String, reduce: bool) -> El {
    let red = enabled && !quiet;
    let base = if red {
        INK
    } else if quiet {
        Rgba::WHITE.alpha(0.10).over_rgba(CANVAS)
    } else {
        RAISED
    };
    let edge = Rgba::WHITE.alpha(if red || quiet { 0.16 } else { 0.08 }).over_rgba(base);
    let ink = if quiet { TEXT } else { Rgba::WHITE };
    // Disabled: 0.55, then the plain button style's own half, composited in sRGB like SwiftUI.
    let dim = |c: Rgba| if enabled { c } else { c.alpha(0.55 * 0.5).over_rgba(CANVAS) };
    let fill = dim(base);
    el().key("primary")
        .h(34.0)
        .min_w(180.0)
        .px(22.0)
        .center()
        .radius(9.0)
        .fill(fill.color())
        .border(1.0, dim(edge).color())
        // SwiftUI's ink 0.35 at shadowRadius 10, which is CSS blur 20.
        .shadow(3.0, 20.0, INK.alpha(if red { 0.35 } else { 0.0 }).color())
        .cursor(if enabled { rune::Cursor::Pointer } else { rune::Cursor::Default })
        .child(
            el().key(format!("primary-{title}"))
                .child(label(title, 13.0, 600.0, dim(ink).over(fill)))
                .transition(swap(reduce)),
        )
        .on_press(move |s: &mut OnboardingWin| {
            if enabled {
                s.send(&action);
            }
        })
}

fn quiet_button(title: &str, action: &'static str) -> El {
    el().h(22.0)
        .px(10.0)
        .center()
        .cursor(rune::Cursor::Pointer)
        .child(label(title, 12.0, 500.0, MUTED.over(CANVAS)))
        .on_press(move |s: &mut OnboardingWin| s.send(action))
}

fn row_button(title: &str, prominent: bool, permission: &str) -> El {
    let fill = if prominent { INK } else { Rgba::WHITE.alpha(0.10).over_rgba(SURFACE) };
    let p = permission.to_owned();
    let layout = permission.to_owned();
    el().h(26.0)
        .px(12.0)
        .center()
        .radius(13.0)
        .fill(fill.color())
        .border(1.0, Rgba::WHITE.alpha(if prominent { 0.18 } else { 0.14 }).over(fill))
        .press(Rgba::WHITE.alpha(0.08).over(fill))
        .cursor(rune::Cursor::Pointer)
        .child(label(title, 12.0, 600.0, if prominent { Rgba::WHITE.over(fill) } else { TEXT.over(fill) }))
        .on_screen_layout(move |s: &mut OnboardingWin, r: [f32; 4]| {
            s.buttons.insert(layout.clone(), r);
        })
        .on_press(move |s: &mut OnboardingWin| s.send(&format!("grant:{p}")))
}

fn spinner(phase: f64, bg: Rgba) -> El {
    let start = phase as f32 * std::f32::consts::TAU;
    let n = 28;
    let r = 5.5 - 0.8;
    let points: Vec<[f32; 2]> = (0..=n)
        .map(|i| {
            let a = start + i as f32 / n as f32 * 0.7 * std::f32::consts::TAU;
            [5.5 + r * a.cos(), 5.5 + r * a.sin()]
        })
        .collect();
    canvas(Shape::new().stroke(points, 1.6, MUTED.over(bg))).size(11.0, 11.0)
}

fn note(s: &str) -> El {
    text(s)
        .font(style(12.0, 400.0).lh(line_pitch(12.0) + 2.0))
        .color(MUTED.over(CANVAS))
        .align(Align::Center)
        .wrap()
        .max_w(480.0)
        .mt(-1.0)
        .mb(-1.0)
}

// MARK: Illustrations

fn welcome_art(s: &OnboardingWin) -> El {
    let t = s.phase(4.0);
    let bob = -3.0 * (t * std::f64::consts::TAU).sin() as f32;
    let mut art = el()
        .size(150.0, 150.0)
        .center()
        .child(el().absolute(0.0, 0.0).size(150.0, 150.0).radius(75.0).fill(INK.alpha(0.22).color()).blur(46.0));
    art = match &s.icon {
        Some(icon) => art.child(el().size(128.0, 128.0).pixels(icon)),
        None => art.child(el().size(112.0, 112.0).radius(28.0).fill(SURFACE.color())),
    };
    art.live_offset(&Live::new([0.0, bob]))
}

fn try_art(s: &OnboardingWin) -> El {
    let t = s.phase(2.2) as f32;
    let ring_bg = CANVAS;
    el().w_full()
        .h(164.0)
        .center()
        .child(
            el().absolute(0.0, 0.0).w_full().h(164.0).center().child(
                el().size(200.0 + 60.0 * t, 86.0 + 60.0 * t)
                    .radius(22.0)
                    .border(2.0, INK.alpha(0.55 * (1.0 - t)).color()),
            ),
        )
        .child(keycaps(&s.shortcut, true, 52.0, ring_bg))
}

fn done_art(reduce: bool) -> El {
    let t = if reduce { Transition::opacity() } else { Transition::scale(0.96) };
    el().key("done-art")
        .w_full()
        .h(164.0)
        .center()
        .child(el().absolute(0.0, 0.0).w_full().h(164.0).center().child(
            el().size(150.0, 150.0).radius(75.0).fill(POSITIVE.alpha(0.16).color()).blur(30.0),
        ))
        .child(
            el().size(104.0, 104.0)
                .radius(52.0)
                .fill(POSITIVE.alpha(0.14).over(POSITIVE.alpha(0.16 * 0.95).over_rgba(CANVAS)))
                .border(1.0, POSITIVE.alpha(0.55).over(CANVAS))
                .center()
                .child(symbol(&icons::CHECK, 42.0, weight::SEMIBOLD, POSITIVE.color())),
        )
        .transition(t.springs(SpringPair::new(Spring::new(0.3, 0.0), Spring::new(0.3, 0.0))))
}

/// The radial glow from the top edge: ink at 10% fading out over 360 pt.
fn top_glow() -> El {
    const R: f32 = 360.0;
    el().absolute(0.0, 0.0).size(WIDTH, R).radial_gradient([0.5, 0.0], R, [(0.0, INK.alpha(0.10).color()), (1.0, INK.alpha(0.0).color())])
}

// MARK: Steps

fn centered(s: &OnboardingWin) -> El {
    let sc = &s.shortcut;
    let (title, body) = match s.state.step.as_str() {
        "try-it" => ("Make your first Scrawl".to_owned(), format!("Tap {sc} now. Talk, point at something, then tap {sc} again.")),
        "done" => (
            "It's on your clipboard".to_owned(),
            format!("Press ⌘V in your agent. Scrawl stays in the menu bar; tap {sc} whenever you want to show something."),
        ),
        _ => (
            "Welcome to Scrawl".to_owned(),
            "Record your screen and your voice, draw on what matters, and paste it all into your agent.".to_owned(),
        ),
    };
    let art = match s.state.step.as_str() {
        "try-it" => try_art(s),
        "done" => done_art(s.reduce),
        _ => welcome_art(s),
    };
    let mut page = col()
        .w_full()
        .items_center()
        .child(el().h(34.0))
        .child(el().w_full().h(200.0).center().child(art))
        .child(heading_text(&title).mt(18.0 - 0.5))
        .child(body_text(&body, 13.5, 440.0).mt(10.0 - 2.0));
    if s.state.step == "try-it" {
        if s.state.picker_note {
            page = page.child(note(PICKER_NOTE).mt(12.0 - 1.0).transition(fade()).key("picker-note"));
        }
        if let Some(speech) = s.state.speech.as_ref().filter(|sp| sp.not_ready())
            && let Some(line) = speech.line()
        {
            let still = matches!(speech.state.as_str(), "downloading" | "checking")
                && speech.engine.as_deref() != Some("SFSpeechRecognizer");
            let text_ = if still {
                format!(
                    "The speech model is still downloading{}. If it isn't done when you finish, Scrawl adds the transcript to your capture afterwards.",
                    speech.percent_in_parens()
                )
            } else {
                line
            };
            page = page.child(note(&text_).mt(12.0 - 1.0).transition(fade()).key("speech-note"));
        }
    }
    page
}

fn heading_text(s: &str) -> El {
    text(s).font(style(22.0, 600.0).lh(lh(22.0, 0.0))).color(TEXT.over(CANVAS)).align(Align::Center).wrap().max_w(540.0)
}

fn body_text(s: &str, size: f32, max_w: f32) -> El {
    text(s)
        .font(style(size, 400.0).lh(line_pitch(size) + 3.0))
        .color(SOFT.alpha(0.85).over(CANVAS))
        .align(Align::Center)
        .wrap()
        .max_w(max_w)
        .mb(-1.5)
}

const PICKER_NOTE: &str =
    "macOS may also ask whether Scrawl can bypass its private window picker. Choose Allow, so Scrawl can record the screen you're showing.";

fn heading(title: &str, body: Option<&str>) -> El {
    let mut c = col().w_full().items_center().pt(44.0 - 0.5).child(heading_text(title));
    if let Some(body) = body {
        c = c.child(body_text(body, 13.0, 480.0).mt(8.0 - 1.25));
    }
    c
}

fn permissions_copy(s: &OnboardingWin) -> (String, Option<String>) {
    let st = &s.state;
    if st.fix && st.rows.len() == 1 && st.rows[0].permission == "camera" {
        return (
            "Camera, for your iPhone".into(),
            Some("macOS treats a cabled iPhone's screen as a camera, so Scrawl needs Camera access to show and record it. Pick iPhone again once it's on.".into()),
        );
    }
    if st.fix {
        return ("Scrawl can't record yet".into(), Some(format!("Turn these on, then press {} again.", s.shortcut)));
    }
    let required = st.rows.iter().filter(|r| r.required).count();
    let count = ["zero", "one", "two", "three", "four", "five"][required.min(5)];
    (format!("Scrawl needs {count} permissions"), None)
}

fn permission_row(s: &OnboardingWin, row_: &Row, detail: Option<String>, next: bool) -> El {
    let (icon, name, why) = permission_info(&row_.permission, &s.paste);
    let granted = row_.granted();
    let disc = if granted { POSITIVE.alpha(0.14).over_rgba(SURFACE) } else { Rgba::WHITE.alpha(0.07).over_rgba(SURFACE) };
    let mut title = row().gap(6.0).center_y().child(label(&name, 13.0, 600.0, TEXT.over(SURFACE)));
    if !s.state.fix && !row_.required {
        let pill = Rgba::WHITE.alpha(0.07).over_rgba(SURFACE);
        title = title.child(
            el().px(6.0).h(15.0).center().radius(7.5).fill(pill.color()).child(label("Optional", 10.0, 600.0, MUTED.over(pill))),
        );
    }
    let reason = if granted { detail.unwrap_or(why.clone()) } else { why };
    let p = &row_.permission;
    let soft = SOFT.over(SURFACE);
    let (key, trailing) = if granted && row_.reopening {
        (
            "reopening",
            row().gap(6.0).center_y().child(spinner(s.phase(0.9), SURFACE)).child(label("Reopening Scrawl", 12.0, 500.0, soft)),
        )
    } else if granted && row_.relaunch {
        ("relaunch", row_button("Reopen Scrawl", true, p))
    } else if granted {
        (
            "allowed",
            row().gap(5.0)
                .center_y()
                .child(symbol(&icons::CHECK_CIRCLE_FILL, 12.0, weight::MEDIUM, POSITIVE.color()))
                .child(label("Allowed", 12.0, 500.0, soft)),
        )
    } else {
        // A list Scrawl can be dragged into gets the drag card either way, so it says Allow
        // until it has been pressed, even when macOS already lists Scrawl as switched off.
        let drag = draggable(p);
        let settings = row_.requested || (row_.status == "denied" && !drag);
        let mut r = row().gap(8.0).center_y();
        if row_.requested && (row_.status != "denied" || drag) {
            r = r.child(spinner(s.phase(0.9), SURFACE));
        }
        let title = if settings { "Open Settings" } else { "Allow" };
        (if settings { "settings" } else { "allow" }, r.child(row_button(title, next && !row_.requested, p)))
    };
    row().w_full()
        .h(58.0)
        .px(14.0)
        .gap(12.0)
        .center_y()
        .child(
            el().size(32.0, 32.0)
                .radius(16.0)
                .fill(disc.color())
                .center()
                .child(symbol(icon, 14.0, weight::MEDIUM, if granted { POSITIVE.color() } else { TEXT.over(disc) })),
        )
        .child(
            col().gap(2.0).shrink().min_w(0.0).child(title).child(
                text(reason).font(style(12.0, 400.0)).color(MUTED.over(SURFACE)).single_line().truncate(),
            ),
        )
        .child(spacer().min_w(8.0))
        .child(el().key(format!("trail-{p}-{key}")).child(trailing).transition(swap(s.reduce)))
}

fn permissions(s: &OnboardingWin) -> El {
    let (title, body) = permissions_copy(s);
    let st = &s.state;
    let next = st.rows.iter().find(|r| !r.granted()).map(|r| r.permission.clone());
    let speech_row = if st.rows.iter().any(|r| r.permission == "speech-recognition") { "speech-recognition" } else { "microphone" };
    let mut card = col().w_full().radius(12.0).fill(SURFACE.color()).border(1.0, LINE.over(SURFACE)).clip_rounded(12.0);
    for (i, r) in st.rows.iter().enumerate() {
        if i > 0 {
            card = card.child(row().w_full().pl(16.0).child(el().h(1.0).grow(1.0).fill(LINE.over(SURFACE))));
        }
        let detail = (r.permission == speech_row && st.speech_not_ready()).then(|| st.speech.as_ref().and_then(SpeechStatus::line)).flatten();
        card = card.child(permission_row(s, r, detail, next.as_deref() == Some(r.permission.as_str())));
    }
    let mut page = col().w_full().items_center().child(heading(&title, body.as_deref())).child(card.mt(22.0 - 0.5));
    if st.picker_note {
        page = page.child(note(PICKER_NOTE).mt(14.0 - 0.5).key("picker-note").transition(fade()));
    }
    page
}

fn how_it_works(s: &OnboardingWin) -> El {
    col().w_full()
        .items_center()
        .child(heading_text("How Scrawl works").mt(34.0 - 0.5))
        .child(demo(s).mt(16.0))
}

fn hint(s: &OnboardingWin) -> El {
    let storyboard = s.storyboard();
    let (text_, soft) = if s.drawing {
        ("Let go of ⇧ to finish", false)
    } else if s.practiced && !s.strokes.is_empty() {
        ("That's a mark", true)
    } else if storyboard {
        ("Try it: hold ⇧ over the pictures and move the mouse", false)
    } else {
        ("Try it: hold ⇧ and move", false)
    };
    let bg = if storyboard { CANVAS } else { stage_bg(25.0) };
    el().key(format!("hint-{text_}"))
        .child(label(text_, if storyboard { 11.5 } else { 11.0 }, 400.0, if soft { SOFT.over(bg) } else { MUTED.over(bg) }))
        .transition(swap(s.reduce))
}

fn practice_layer(s: &OnboardingWin, w: f32, h: f32) -> El {
    let mut shape = Shape::new();
    for stroke in s.strokes.iter().filter(|st| st.len() > 1) {
        shape = shape.stroke(stroke.clone(), 3.2, INK.color());
    }
    canvas(shape)
        .absolute(0.0, 0.0)
        .size(w, h)
        .opacity(s.ink_opacity())
        .on_pointer_move(|s: &mut OnboardingWin, p: PointerPress| s.pointer(p.x, p.y))
        .on_hover_change(|s: &mut OnboardingWin, inside: bool, _tx: &mut Tx| {
            if !inside {
                s.end_stroke();
            }
        })
        .on_layout(|s: &mut OnboardingWin, r: [f32; 4]| s.stage = r)
}

fn stage_box(t: f64, show_cursor: bool, radius: f32) -> El {
    el().size(geo::SIZE[0], geo::SIZE[1])
        .radius(radius)
        .gradient(STAGE_TOP.color(), STAGE_BOTTOM.color())
        .clip_rounded(radius)
        .children(Scene { t }.draw(show_cursor, 1.0))
}

fn demo(s: &OnboardingWin) -> El {
    let beats = beats(&s.shortcut, &s.paste);
    if s.storyboard() {
        let frames: [(f64, String); 4] = [
            (3.95, format!("Tap {}, talk, and hold ⇧ to mark", s.shortcut)),
            (5.0, "Click to point at something".to_owned()),
            (6.5, format!("Tap {} again to finish", s.shortcut)),
            (9.4, format!("Press {} to paste into your agent", s.paste)),
        ];
        let k = 0.44;
        let (cw, ch) = (geo::SIZE[0] * k, geo::SIZE[1] * k);
        let cell = |(t, caption): &(f64, String)| {
            col().gap(5.0)
                .items_center()
                .child(
                    // SwiftUI's `scaleEffect(0.44, anchor: .topLeading)`: the full-size stage,
                    // scaled about its centre and moved so its top-left sits on the cell's.
                    el().size(cw, ch)
                        .clip_rounded(7.0)
                        .child(
                            stage_box(*t, true, 0.0)
                                .absolute(-(geo::SIZE[0] - cw) / 2.0, -(geo::SIZE[1] - ch) / 2.0)
                                .scale(k),
                        )
                        .child(el().absolute(0.0, 0.0).size(cw, ch).radius(7.0).border(1.0, LINE.over(STAGE_TOP))),
                )
                .child(label(caption, 11.0, 400.0, SOFT.over(CANVAS)))
        };
        let grid = col()
            .gap(6.0)
            .items_center()
            .child(row().gap(10.0).child(cell(&frames[0])).child(cell(&frames[1])))
            .child(row().gap(10.0).child(cell(&frames[2])).child(cell(&frames[3])));
        let height = 2.0 * (ch + 5.0 + lh(11.0, 0.0) * 1.05) + 6.0;
        return col()
            .w(geo::SIZE[0])
            .items_center()
            .child(el().w(geo::SIZE[0]).h(height).mt(0.5).center().child(grid).child(practice_layer(s, geo::SIZE[0], height)))
            .child(el().h(16.0).mt(8.5).center().child(hint(s)));
    }

    let t = s.demo_t();
    let practicing = s.drawing || !s.strokes.is_empty();
    let show_cursor = !practicing;
    let mut stage = el().size(geo::SIZE[0], geo::SIZE[1]).child(stage_box(t, show_cursor, 12.0));
    if let Some((ghost_t, at)) = s.ghost {
        // Only the old moment fades, on top of the new one, so the page both share never dims.
        let p = ((s.now - at) / 0.22).clamp(0.0, 1.0);
        let o = (1.0 - (1.0 - (1.0 - p).powi(2))) as f32;
        stage = stage.child(stage_box(ghost_t, false, 12.0).absolute(0.0, 0.0).opacity(o).blur(3.0 * (1.0 - o)));
    }
    stage = stage
        .child(el().absolute(0.0, 0.0).size(geo::SIZE[0], geo::SIZE[1]).radius(12.0).border(1.0, LINE.over(STAGE_TOP)))
        .child(practice_layer(s, geo::SIZE[0], geo::SIZE[1]))
        .child(
            el().absolute(geo::APP[0], geo::PILL_TOP).h(geo::PILL_HEIGHT).items_center().child(hint(s)),
        );
    // Drawing on the stage holds the ⇧ caption, lit while ⇧ is down.
    let caption_t = if practicing { if s.drawing { 3.0 } else { 3.98 } } else { t };
    col().gap(12.0).items_center().child(stage).child(
        el().key(format!("captions-{practicing}")).child(captions(&beats, caption_t)).transition(swap(s.reduce)),
    )
}

fn footer(s: &OnboardingWin) -> El {
    let st = &s.state;
    let primary: Option<(&str, &str, bool)> = match st.step.as_str() {
        "welcome" => Some(("Get Started", "continue", true)),
        "how-it-works" => Some(if st.tutorial_seen() { ("Continue", "continue", true) } else { ("Skip", "skip", true) }),
        "permissions" => Some((if st.fix { "Done" } else { "Continue" }, "continue", st.ready)),
        "done" => Some(("Done", "continue", true)),
        _ => None,
    };
    let secondary: Option<(&str, &'static str)> = match st.step.as_str() {
        "permissions" => Some((if st.fix { "Not now" } else { "Set up later" }, "later")),
        "try-it" => Some(("Close", "continue")),
        "done" => Some(("Open Settings", "settings")),
        _ => None,
    };
    let mut f = col().w_full().items_center().gap(10.0);
    if let Some((title, action, enabled)) = primary {
        f = f.child(primary_button(title, enabled, action == "skip", action.to_owned(), s.reduce));
    }
    f = f.child(match secondary {
        Some((title, action)) => quiet_button(title, action),
        None => el().h(22.0),
    });
    let mut bottom = el().w_full().h(22.0).mt(if st.step == "done" { 7.0 } else { 8.0 }).center();
    if st.step != "done" && !st.fix {
        let dots = (0..st.count.max(1))
            .map(|i| {
                let on = i == st.index;
                el().key(format!("dot-{i}"))
                    .w(if on { 16.0 } else { 6.0 })
                    .h(6.0)
                    .radius(3.0)
                    .fill(if on { INK.color() } else { Rgba::WHITE.alpha(0.18).over(CANVAS) })
            })
            .collect::<Vec<_>>();
        bottom = bottom.child(row().gap(6.0).center_y().children(dots));
    }
    if st.index > 0 && !st.fix && st.step != "done" {
        bottom = bottom.child(quiet_button("Back", "back").absolute(0.0, 0.0));
    }
    f.child(bottom)
}

fn view(s: &OnboardingWin, _cx: &mut Cx<OnboardingWin>) -> El {
    let page = match s.state.step.as_str() {
        "how-it-works" => how_it_works(s),
        "permissions" => permissions(s),
        _ => centered(s),
    };
    col().w_full()
        .h_full()
        .fill(CANVAS.color())
        .child(top_glow())
        .child(
            col().w_full()
                .grow(1.0)
                .px(40.0)
                .pb(20.0)
                .window_drag()
                .child(page.key(format!("page-{}", s.state.step)).transition(page_transition(s.forward, s.reduce)))
                .child(spacer().grow(1.0))
                .child(footer(s)),
        )
}

// MARK: Clock, keys, focus

fn poll(s: &mut OnboardingWin, _t: f64, tx: &mut Tx) -> bool {
    if s.render.is_some() {
        return false;
    }
    s.now = clock();
    if s.follow.is_some() {
        s.tick_follow();
        tx.wake_after(Duration::from_millis(300));
    }
    let mut changed = false;
    if s.visible && s.state.step == "how-it-works" {
        changed |= s.tick_demo();
    }
    if s.animating() {
        changed = true;
        tx.wake_after(Duration::from_millis(16));
    }
    if changed && s.now >= s.calm_until {
        tx.instant();
    }
    changed
}

fn on_focus(s: &mut OnboardingWin, focused: bool, tx: &mut Tx) {
    s.focused = focused;
    if !focused {
        s.end_stroke();
    }
    tx.wake_after(Duration::from_millis(16));
}

fn on_close(s: &mut OnboardingWin, _tx: &mut Tx) {
    s.visible = false;
    s.follow = None;
    if let Some(events) = s.events.take() {
        events(crate::onboarding::Event::Action("close".into()));
        events(crate::onboarding::Event::Exited);
    }
    hand_back_keys();
}

/// Takes the keys, remembering the app in front, and holds them while the window shows.
fn take_keys() {
    #[cfg(target_os = "macos")]
    {
        super::platform::hold_keys(super::platform::KeyHolder::Onboarding, true);
        super::platform::take_keys_now();
    }
}

/// Gives the remembered app its keys back, unless another Scrawl window still has them.
fn hand_back_keys() {
    #[cfg(target_os = "macos")]
    {
        super::platform::hold_keys(super::platform::KeyHolder::Onboarding, false);
        super::platform::hand_back_keys_now();
    }
}

fn on_key(s: &mut OnboardingWin, key: &rune::KeyEvent, tx: &mut Tx) -> bool {
    if matches!(key.key, rune::Key::Command('w')) || (key.mods.cmd && matches!(key.key, rune::Key::Text(ref c) if c == "w")) {
        tx.close_window();
        return true;
    }
    if matches!(key.key, rune::Key::Enter) && !key.mods.cmd {
        let st = &s.state;
        let action = match st.step.as_str() {
            "welcome" | "done" => Some("continue"),
            "how-it-works" => Some(if st.tutorial_seen() { "continue" } else { "skip" }),
            "permissions" if st.ready => Some("continue"),
            _ => None,
        };
        if let Some(action) = action {
            s.send(action);
            return true;
        }
    }
    false
}

fn init() -> OnboardingWin {
    OnboardingWin::new()
}

fn spec(fonts: &'static [&'static [u8]], height: f32) -> AppSpec<OnboardingWin> {
    AppSpec {
        title: TITLE,
        size: (WIDTH, height),
        min_size: Some((WIDTH, 300.0)),
        init,
        view,
        scenes: SCENES,
        fonts,
        families: families(fonts),
        setup: None,
        poll: Some(poll),
        on_key: Some(on_key),
    }
}

const CHROME: Chrome = Chrome { full_size_content: true, traffic_lights: None, drag_regions: true };

// MARK: Hosting

static HANDLE: OnceLock<AppHandle<OnboardingWin>> = OnceLock::new();

/// Adds the hidden welcome window to Scrawl's Rune app; `request_open` shows it.
pub(crate) fn install<S: 'static>(app: &mut App<S>) {
    let fonts = fonts();
    let mut spec = WindowSpec::new("onboarding", spec(fonts, HEIGHT))
        .options(WindowOptions {
            visible: false,
            resizable: false,
            min_size: Some((WIDTH, 300.0)),
            max_size: Some((WIDTH, HEIGHT)),
            ..Default::default()
        })
        .chrome(CHROME)
        .srgb_blend(true)
        .on_focus(on_focus);
    spec.app = spec.app.on_close(on_close);
    let (handle, _window) = app.window(spec);
    let _ = HANDLE.set(handle);
}

/// Whether the Rune window is there to take `onboarding::Session`'s messages.
pub(crate) fn available() -> bool {
    HANDLE.get().is_some()
}

/// Attaches a session: its messages drive the window, its buttons go to `events`.
pub(crate) fn request_open(messages: flume::Receiver<Value>, shortcut: String, paste: String, events: Events) -> bool {
    let Some(handle) = HANDLE.get().cloned() else { return false };
    let link = attach(handle, shortcut, paste, events);
    std::thread::Builder::new()
        .name("rune-onboarding".into())
        .spawn(move || {
            let mut presented = false;
            while let Ok(message) = messages.recv() {
                if !on_message(&link, message, &mut presented) {
                    return;
                }
            }
            close(&link);
        })
        .is_ok()
}

fn attach<H: Host<OnboardingWin>>(host: H, shortcut: String, paste: String, events: Events) -> Link<OnboardingWin, H> {
    Link::attach(host, move |s: &mut OnboardingWin, _tx| {
        s.events = Some(events);
        s.reduce = reduce_motion();
        s.shortcut = shortcut;
        s.paste = paste;
        s.sized = false;
        s.buttons.clear();
    })
}

/// One message from the session. False once it has closed the window.
fn on_message<H: Host<OnboardingWin>>(link: &Link<OnboardingWin, H>, message: Value, presented: &mut bool) -> bool {
    let kind = message["type"].as_str().unwrap_or_default().to_owned();
    let permission = message["permission"].as_str().unwrap_or_default().to_owned();
    match kind.as_str() {
        "state" => {
            let Ok(state) = serde_json::from_value::<State>(message) else {
                crate::capture::scrawl_log("onboarding (rune): bad state");
                return true;
            };
            // The page transition reads the direction, so it changes a commit before the step.
            let step = state.step.clone();
            link.update(move |s, _tx| {
                if s.state.step != step {
                    s.forward = step_index(&step) >= step_index(&s.state.step);
                }
            });
            let first = !*presented;
            link.update(move |s, tx| {
                if first {
                    s.state = state.clone();
                    s.now = clock();
                    if s.state.step == "how-it-works" {
                        s.restart_demo();
                    }
                    tx.instant();
                    if !s.sized {
                        s.sized = true;
                        // Swift's first state sizes the window to its step and centres it.
                        tx.resize_window_instant((WIDTH, s.state.height()));
                        tx.center_window();
                    }
                    s.present(tx);
                } else {
                    s.apply(state, tx);
                }
            });
            if first {
                *presented = true;
                link.open();
            }
        }
        // The user has moved on to another row: answering an earlier question mustn't pull this
        // window over the System Settings pane they're working in.
        "request" => link.update(move |s, _| {
            s.follow = None;
            #[cfg(target_os = "macos")]
            super::rune_permissions::request(&permission);
        }),
        "open-settings" => link.update(move |s, _| {
            s.follow = None;
            #[cfg(target_os = "macos")]
            super::rune_permissions::open_settings(&permission, s.screen_frame(&permission));
        }),
        "hide" => {
            link.update(|s, _| {
                s.visible = false;
                #[cfg(target_os = "macos")]
                super::platform::hold_keys(super::platform::KeyHolder::Onboarding, false);
            });
            link.hide();
        }
        "show" => {
            link.update(|s, tx| {
                s.reduce = reduce_motion();
                s.present(tx);
            });
            link.open();
        }
        "close" => {
            close(link);
            return false;
        }
        _ => {}
    }
    true
}

/// The session has ended: its window goes, unless a newer session has it by now.
fn close<H: Host<OnboardingWin>>(link: &Link<OnboardingWin, H>) {
    link.update(|s, _| {
        s.events = None;
        s.follow = None;
        s.visible = false;
        hand_back_keys();
    });
    link.hide();
}

// MARK: Renders

macro_rules! scenes {
    ($($name:literal),* $(,)?) => {
        const SCENE_NAMES: &[&str] = &[$($name),*];
        static SCENES: &[(&str, fn(&mut OnboardingWin))] = &[$(($name, {
            fn apply(s: &mut OnboardingWin) { sample(s, $name) }
            apply as fn(&mut OnboardingWin)
        })),*];
    };
}

// The order `scrawl-helper onboarding --render` numbers them.
scenes!(
    "welcome",
    "how-it-works",
    "how-it-works-recording",
    "how-it-works-mark",
    "how-it-works-click",
    "how-it-works-finish",
    "how-it-works-paste",
    "how-it-works-fixed",
    "how-it-works-trying",
    "how-it-works-tried",
    "how-it-works-reduced-motion",
    "how-it-works-skipped-earlier",
    "permissions-none",
    "permissions-screen-waiting",
    "permissions-screen-reopening",
    "permissions-resumed",
    "permissions-some",
    "permissions-denied",
    "permissions-all",
    "permissions-reopen-fallback",
    "permissions-picker-note",
    "permissions-macos15",
    "permissions-macos15-no-dictation",
    "try-it",
    "try-it-downloading",
    "done",
    "fix-required",
    "fix-screen-reopening",
    "fix-camera",
);

/// The state, loop phase, and demo practice of one render, as Swift's `render` builds them.
fn sample_state(name: &str) -> (State, f64) {
    let row = |p: &str, status: &str, requested: bool, reopening: bool, relaunch: bool| Row {
        permission: p.into(),
        status: status.into(),
        requested,
        required: matches!(p, "screen-recording" | "microphone" | "accessibility"),
        reopening,
        relaunch,
    };
    let nd = "not-determined";
    let g = "granted";
    let none = vec![row("screen-recording", nd, false, false, false), row("microphone", nd, false, false, false), row("accessibility", nd, false, false, false)];
    let waiting = vec![row("screen-recording", nd, true, false, false), row("microphone", nd, false, false, false), row("accessibility", nd, false, false, false)];
    let reopening = vec![row("screen-recording", g, false, true, false), row("microphone", nd, false, false, false), row("accessibility", nd, false, false, false)];
    let resumed = vec![row("screen-recording", g, false, false, false), row("microphone", nd, false, false, false), row("accessibility", nd, false, false, false)];
    let some = vec![row("screen-recording", g, false, false, false), row("microphone", g, false, false, false), row("accessibility", nd, true, false, false)];
    let denied = vec![row("screen-recording", g, false, false, false), row("microphone", "denied", false, false, false), row("accessibility", g, false, false, false)];
    let all = vec![row("screen-recording", g, false, false, false), row("microphone", g, false, false, false), row("accessibility", g, false, false, false)];
    let fallback = vec![row("screen-recording", g, false, false, true), row("microphone", g, false, false, false), row("accessibility", g, false, false, false)];
    let lost = vec![row("screen-recording", g, false, false, false), row("microphone", g, false, false, false), row("accessibility", nd, false, false, false)];
    let mut sequoia = none.clone();
    sequoia.push(row("speech-recognition", nd, false, false, false));
    let mut sequoia_no_dictation = all.clone();
    sequoia_no_dictation.push(row("speech-recognition", g, false, false, false));
    let speech = |state: &str, engine: Option<&str>, progress: f64, error: Option<&str>| SpeechStatus {
        state: state.into(),
        engine: engine.map(str::to_owned),
        locale: Some("en_US".into()),
        progress,
        locales: None,
        error: error.map(str::to_owned),
    };
    let st = |step: &str, index: usize| State { step: step.into(), index, ..State::default() };
    let perms = |rows: Vec<Row>| State { step: "permissions".into(), index: 2, rows, ..State::default() };
    let fix = |rows: Vec<Row>| State { step: "permissions".into(), index: 0, count: 1, rows, fix: true, ..State::default() };
    let how = |tutorial: Option<&str>| State { tutorial: tutorial.map(str::to_owned), ..st("how-it-works", 1) };
    let l = script::LOOP;
    match name {
        "welcome" => (st("welcome", 0), 0.25),
        "how-it-works" => (how(None), 0.0),
        "how-it-works-recording" => (how(None), 1.6 / l),
        "how-it-works-mark" => (how(None), 3.4 / l),
        "how-it-works-click" => (how(None), 4.95 / l),
        "how-it-works-finish" => (how(None), 6.4 / l),
        "how-it-works-paste" => (how(None), 7.35 / l),
        "how-it-works-fixed" => (how(Some("watched")), 9.6 / l),
        "how-it-works-trying" => (how(None), 2.0 / l),
        "how-it-works-tried" => (how(Some("practiced")), 2.0 / l),
        "how-it-works-reduced-motion" => (how(Some("watched")), 0.0),
        "how-it-works-skipped-earlier" => (how(Some("skipped")), 0.5 / l),
        "permissions-none" => (perms(none), 0.0),
        "permissions-screen-waiting" => (perms(waiting), 0.3),
        "permissions-screen-reopening" => (perms(reopening), 0.3),
        "permissions-resumed" => (perms(resumed), 0.0),
        "permissions-some" => (perms(some), 0.3),
        "permissions-denied" => (perms(denied), 0.0),
        "permissions-all" => (State { ready: true, ..perms(all) }, 0.0),
        "permissions-reopen-fallback" => (perms(fallback), 0.0),
        "permissions-picker-note" => (
            State { picker_note: true, speech: Some(speech("downloading", None, 0.42, None)), ..perms(some) },
            0.3,
        ),
        "permissions-macos15" => (
            State { speech: Some(speech("needsPermission", Some("SFSpeechRecognizer"), 0.0, None)), ..perms(sequoia) },
            0.0,
        ),
        "permissions-macos15-no-dictation" => (
            State {
                ready: true,
                speech: Some(speech(
                    "failed",
                    Some("SFSpeechRecognizer"),
                    0.0,
                    Some("Turn on Dictation in System Settings › Keyboard for transcripts."),
                )),
                ..perms(sequoia_no_dictation)
            },
            0.0,
        ),
        "try-it" => (State { count: 3, ..st("try-it", 2) }, 0.3),
        "try-it-downloading" => (
            State { count: 3, picker_note: true, speech: Some(speech("downloading", None, 0.42, None)), ..st("try-it", 2) },
            0.3,
        ),
        "done" => (st("done", 3), 0.0),
        "fix-required" => (fix(lost), 0.0),
        "fix-screen-reopening" => (fix(reopening), 0.3),
        "fix-camera" => (fix(vec![row("camera", nd, true, false, false)]), 0.3),
        _ => (State::default(), 0.0),
    }
}

/// `scrawl-helper onboarding --render`'s samples, from the same states.
fn sample(s: &mut OnboardingWin, name: &str) {
    let (state, phase) = sample_state(name);
    s.state = state;
    s.render = Some(phase);
    s.reduce = false;
    s.storyboard = name == "how-it-works-reduced-motion";
    s.shortcut = "⌥⇧S".into();
    s.paste = "⌥⇧V".into();
    let practice: Vec<[f32; 2]> = (0..=40)
        .map(|i| {
            let a = i as f32 / 40.0 * 2.1 * std::f32::consts::PI + 3.4;
            [292.0 + 44.0 * a.cos(), 189.0 + 25.0 * a.sin()]
        })
        .collect();
    (s.strokes, s.drawing, s.practiced) = match name {
        "how-it-works-trying" => (vec![practice[..26].to_vec()], true, false),
        "how-it-works-tried" => (vec![practice], false, true),
        _ => (Vec::new(), false, false),
    };
    s.fade_at = None;
    s.ghost = None;
}

/// `--snap` one sample (`onboarding-<name>`) headless.
pub(crate) fn run_headless(scene: &str, mut args: Vec<String>) {
    let name = scene.trim_start_matches("onboarding-");
    if let Some(i) = args.iter().position(|a| a == "--scene") {
        args[i + 1] = name.to_owned();
    }
    let height = sample_state(name).0.height();
    let fonts = fonts();
    App::new(spec(fonts, height)).chrome(CHROME).srgb_blend(true).args(args).run();
}

/// Every sample in the order `scrawl-helper onboarding --render` numbers them.
pub(crate) fn render_names() -> Vec<String> {
    SCENE_NAMES.iter().map(|n| format!("onboarding-{n}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_decode_as_rust_sends_them() {
        let view = crate::onboarding::View {
            step: crate::onboarding::Step::Permissions,
            index: 2,
            count: 4,
            rows: Vec::new(),
            ready: false,
            fix: false,
            tutorial: Some(crate::onboarding::Tutorial::Watched),
            picker_note: true,
        };
        let mut value = serde_json::to_value(&view).unwrap();
        value["type"] = "state".into();
        let state: State = serde_json::from_value(value).unwrap();
        assert_eq!(state.step, "permissions");
        assert!(state.picker_note);
        assert!(state.tutorial_seen());
        assert_eq!(state.height(), HEIGHT);
    }

    #[test]
    fn a_fix_is_as_tall_as_its_rows() {
        let (state, _) = sample_state("fix-camera");
        assert_eq!(state.height(), 376.0);
        let (state, _) = sample_state("fix-required");
        assert_eq!(state.height(), 492.0);
    }

    #[test]
    fn every_render_has_a_state() {
        assert_eq!(SCENE_NAMES.len(), 29);
        for name in SCENE_NAMES {
            if *name != "welcome" {
                assert_ne!(sample_state(name).0, State::default(), "{name}");
            }
        }
    }

    /// A replaced session's reader may still be handling its "close" after the new session
    /// has attached, whether before or after the new one's first state.
    #[test]
    fn a_replaced_session_leaves_the_new_one_alone() {
        use super::super::rune_session::test_host::Recorder;
        let mut state = serde_json::to_value(crate::onboarding::View {
            step: crate::onboarding::Step::Welcome,
            index: 0,
            count: 4,
            rows: Vec::new(),
            ready: false,
            fix: false,
            tutorial: None,
            picker_note: false,
        })
        .unwrap();
        state["type"] = "state".into();
        let close = serde_json::json!({"type": "close"});
        for close_first in [true, false] {
            let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
            let events = |tag: &'static str| -> Events {
                let seen = Arc::clone(&seen);
                Arc::new(move |_| seen.lock().unwrap().push(tag))
            };
            let host = Recorder::default();
            let (mut win, mut visible) = (OnboardingWin::new(), false);
            let old = attach(host.clone(), "⌥⇧S".into(), "⌥⇧V".into(), events("old"));
            assert!(on_message(&old, state.clone(), &mut false));
            host.run(&mut win, &mut visible);
            assert!(visible);

            let new = attach(host.clone(), "⌥⇧S".into(), "⌥⇧V".into(), events("new"));
            if close_first {
                assert!(!on_message(&old, close.clone(), &mut true));
                assert!(on_message(&new, state.clone(), &mut false));
            } else {
                assert!(on_message(&new, state.clone(), &mut false));
                assert!(!on_message(&old, close.clone(), &mut true));
            }
            host.run(&mut win, &mut visible);
            assert!(visible, "close_first: {close_first}");
            assert!(win.visible);
            (win.events.as_ref().expect("the new session's events"))(crate::onboarding::Event::Exited);
            assert_eq!(*seen.lock().unwrap(), ["new"]);

            assert!(!on_message(&new, close.clone(), &mut true));
            host.run(&mut win, &mut visible);
            assert!(!visible && win.events.is_none());
        }
    }

    #[test]
    fn the_demo_springs_settle() {
        assert_eq!(spring_step(0.0, 1.0, 0.4), 0.0);
        assert!(spring_step(2.0, 1.0, 0.4) > 0.999);
        let mark = geo::mark_path(1.0);
        assert_eq!(mark.len(), 81);
    }
}
