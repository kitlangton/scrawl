#![allow(dead_code)]
//!
//! Ported from Kit Motion study 03 (`island/model.js` and `island/NATIVE.md`):
//! - Springs are parameterized by `spring(duration, bounce)` with `ω = 2π / duration` and
//!   `ζ = 1 − bounce`, integrated at fixed 480 Hz sub-steps so retargets preserve velocity (`C1`).
//! - Only the outline's width (`L`, `R`) bounces (`0.50 / 0.28` open, `0.45 / 0.18` close,
//!   `0.35 / 0.45` tap squish); the top edge `TOP` is anchored so it never bounces upward into the
//!   top of the screen, and content / opacity / shared travel use `bounce = 0`.
//! - Room gating (`CLEAR = 2 pt` + `EDGE_BAND = 3 pt` smoothstep), `0.10 s` look-ahead exit cap
//!   sampled at 240 Hz, `0.10 s` collapse wait, `0.50` growth gate, `0.30 s` content-in fade from
//!   `8 pt` blur and `0.82` scale with blur on its own critically damped clock at `0.55×` lead,
//!   and `0.013 s` / `1.75 pt` directional stagger.
//! - Persistent shared elements (`lead` dot + timer across recording states, and mode chips + `✕`
//!   across `Recording (chips) -> Selecting`) travel on a `0.24 s` zero-bounce spring with up to
//!   `4 pt` blur in transit that clears before landing.

use crate::ui::SelectionMode;

// ─── Geometry (pt in the fixed 640 × 84 panel) ──────────────────────────────────────────────

pub const PANEL_W: f32 = 640.0;
pub const PANEL_H: f32 = 84.0;
pub const PANEL_Y: f32 = 34.0;
pub const CX: f32 = PANEL_W * 0.5;
/// Top edge of the resting 40 pt capsule inside the 84 pt panel (vertical centre y = 42, screen y = 76).
pub const TOP: f32 = 22.0;
pub const ISLAND_H: f32 = 40.0;
pub const ISLAND_RADIUS: f32 = ISLAND_H * 0.5;
pub const ISLAND_INSET: f32 = 4.0;
pub const ISLAND_OPEN_FROM_W: f32 = 120.0;
pub const IDLE_W: f32 = 96.0;
pub const IDLE_H: f32 = 24.0;

pub const CHIP_H: f32 = ISLAND_H - 2.0 * ISLAND_INSET; // 32.0
pub const CHIP_RADIUS: f32 = CHIP_H * 0.5; // 16.0
pub const CHIP_PX: f32 = 12.0;
pub const CHIP_GAP: f32 = 2.0;
pub const DIVIDER_MX: f32 = 6.0;
pub const DIVIDER_W: f32 = 1.0 + 2.0 * DIVIDER_MX; // 13.0
pub const LEAD_TRAIL: f32 = 8.0;
pub const SEG_PX: f32 = 10.0;
pub const SEG_ICON: f32 = 14.0;
pub const SEG_ICON_GAP: f32 = 5.0;
pub const HUD_TEXT: f32 = 13.0;

/// Core cap radius of a 32 pt chip for clearance checks (3 pt inside the 16 pt chip pill and 7 pt
/// inside the 20 pt outer capsule wall at rest, so at rest `d = 7 pt >= CLEAR + EDGE_BAND = 5 pt`).
pub const CHIP_CORE_ROUND: f32 = 13.0;

// ─── Physics & Timing Constants ─────────────────────────────────────────────────────────────

pub const CLEAR: f32 = 2.0;
pub const EDGE_BAND: f32 = 3.0;
pub const EXIT_LEAD_S: f32 = 0.08;
/// Short pause before narrowing between two visible states so edge items start fading before
/// the capsule wall moves across them. Never used on Entry or Exit (`Pres::Hidden`).
pub const COLLAPSE_WAIT_S: f32 = 0.04;
/// Fraction of a visible width expansion before incoming items begin fading in.
pub const CONTENT_DELAY: f32 = 0.2;
/// Whole-pill and per-element scale at `alpha = 0` on Entry and Exit (`0.94 ↔ 1.0`).
pub const ENTRY_EXIT_SCALE: f32 = 0.94;
pub const CONTENT_SCALE_FROM: f32 = 0.94;
/// Maximum Gaussian blur sigma (in points) for entering/exiting content and shell.
pub const CONTENT_BLUR_MAX: f32 = 5.0;
pub const SHELL_BLUR_MAX: f32 = 4.0;
/// Blur resolves ahead of opacity (`0.55×` of the fade duration).
pub const BLUR_LEAD: f32 = 0.55;
pub const CONTENT_STAGGER_S: f32 = 0.008;
/// Slight lead (`16 ms`) of the capsule shell ahead of content on initial Entry from Hidden.
pub const ENTRY_CONTENT_DELAY_S: f32 = 0.016;
pub const CONTENT_PUSH_PT: f32 = 0.5;
pub const TRAVEL_BLUR_MAX: f32 = 3.5;
/// Reduce Motion cross-fade duration (`150 ms`).
pub const REDUCED_FADE_S: f32 = 0.15;
pub const RADIUS_BLEND: f32 = 8.0;
pub const PRESS_GROW_PT: f32 = 2.5;
pub const TAP_SQUISH: f32 = 0.04;
const REF_DIST: f32 = 140.0;
const DISTANCE_FLOOR: f32 = 0.96;
const AHEAD_STEP: f32 = 1.0 / 240.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringCfg {
    pub duration: f32,
    pub bounce: f32,
}

impl SpringCfg {
    pub const fn new(duration: f32, bounce: f32) -> Self {
        Self { duration, bounce }
    }
}

/// Entry from Hidden: `~235–250 ms` critically damped (`0.94 → 1.0` scale, `4 pt → 0` blur, `0 → 1` alpha).
pub const ENTRY_SHELL_SPRING: SpringCfg = SpringCfg::new(0.25, 0.0);
pub const ENTRY_ALPHA_SPRING: SpringCfg = SpringCfg::new(0.22, 0.0);
/// Exit to Hidden: `~195 ms` critically damped (`1.0 → 0.94` scale, `0 → 4 pt` blur, `1 → 0` alpha).
pub const EXIT_SHELL_SPRING: SpringCfg = SpringCfg::new(0.21, 0.0);
/// Visible state-to-state width expansion (`≤ 340 ms` settled, `0.14` width-only morph bounce per `kit-motion`).
pub const OPEN_SPRING: SpringCfg = SpringCfg::new(0.30, 0.14);
/// Visible state-to-state width contraction (`≤ 340 ms` settled with `40 ms` collapse wait, critically damped `0.30 / 0`).
pub const CLOSE_SPRING: SpringCfg = SpringCfg::new(0.30, 0.0);
pub const TAP_SPRING: SpringCfg = SpringCfg::new(0.30, 0.35);
pub const PRESS_SPRING: SpringCfg = SpringCfg::new(0.20, 0.0);
/// Shared-element slide between visible states (`≤ 230 ms` settled, critically damped).
pub const SHARED_SPRING: SpringCfg = SpringCfg::new(0.22, 0.0);
pub const CONTENT_IN_SPRING: SpringCfg = SpringCfg::new(0.24, 0.0);
pub const CONTENT_OUT_SPRING: SpringCfg = SpringCfg::new(0.17, 0.0);

#[inline]
pub fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
pub fn smooth(a: f32, b: f32, v: f32) -> f32 {
    let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringVal {
    pub x: f32,
    pub v: f32,
    pub target: f32,
    pub duration: f32,
    pub bounce: f32,
}

impl SpringVal {
    pub const fn new(x: f32, cfg: SpringCfg) -> Self {
        Self {
            x,
            v: 0.0,
            target: x,
            duration: cfg.duration,
            bounce: cfg.bounce,
        }
    }

    #[inline]
    pub fn to(&mut self, target: f32, cfg: SpringCfg) {
        self.target = target;
        self.duration = cfg.duration;
        self.bounce = cfg.bounce;
    }

    #[inline]
    pub fn snap(&mut self, x: f32) {
        self.x = x;
        self.target = x;
        self.v = 0.0;
    }

    #[inline]
    pub fn is_resting(&self) -> bool {
        (self.x - self.target).abs() < 1e-4 && self.v.abs() < 4e-4
    }

    /// SwiftUI's `spring(duration:bounce:)`, using the exact analytical closed-form solution
    /// so integration is frame-rate independent and retargets are C1 continuous.
    pub fn step(&mut self, dt: f32) -> bool {
        if dt <= 0.0 {
            return false;
        }
        if self.is_resting() {
            self.x = self.target;
            self.v = 0.0;
            return false;
        }
        let w = std::f32::consts::TAU / self.duration.max(0.02);
        let zeta = (1.0 - self.bounce).clamp(0.05, 1.0);
        let d0 = self.x - self.target;
        let v0 = self.v;
        if zeta >= 0.9999 {
            let e = (-w * dt).exp();
            let c1 = v0 + w * d0;
            let next_x = self.target + (d0 + c1 * dt) * e;
            let next_v = (v0 - c1 * w * dt) * e;
            if d0 * (next_x - self.target) < 0.0 {
                self.x = self.target;
                self.v = 0.0;
            } else {
                self.x = next_x;
                self.v = next_v;
            }
        } else {
            let wd = w * (1.0 - zeta * zeta).sqrt();
            let e = (-zeta * w * dt).exp();
            let (s, c) = (wd * dt).sin_cos();
            let b = (v0 + zeta * w * d0) / wd;
            self.x = self.target + e * (d0 * c + b * s);
            self.v = e * ((b * wd - zeta * w * d0) * c - (d0 * wd + zeta * w * b) * s);
        }
        if self.is_resting() {
            self.x = self.target;
            self.v = 0.0;
        }
        true
    }

    /// Critically damped 0-bounce step that never crosses `target` (`status-roll.js`).
    pub fn step_critical(&mut self, dt: f32, visual_duration: f32, _unit: f32) {
        self.duration = visual_duration.max(0.03);
        self.bounce = 0.0;
        self.step(dt);
    }
}

// ─── Silhouette & Clearance Math ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Silhouette {
    pub l: f32,
    pub t: f32,
    pub r: f32,
    pub b: f32,
    pub radius: f32,
    pub alpha: f32,
    pub blur: f32,
}

impl Silhouette {
    #[inline]
    pub fn width(&self) -> f32 {
        (self.r - self.l).max(0.0)
    }

    #[inline]
    pub fn height(&self) -> f32 {
        (self.b - self.t).max(0.0)
    }

    #[inline]
    pub fn rect(&self) -> [f32; 4] {
        [self.l, self.t, self.width(), self.height()]
    }
}

/// Polynomial smooth-min of `h / 2` and `max_r` (`radiusOf` in `island/model.js`).
#[inline]
pub fn radius_of(h: f32, max_r: f32, blend: f32) -> f32 {
    let a = h * 0.5;
    let b = max_r;
    if blend <= 0.0 {
        return a.min(b);
    }
    let t = (blend - (a - b).abs()).max(0.0) / blend;
    a.min(b) - t * t * blend * 0.25
}

/// Exact signed distance from `(x, y)` to a silhouette's rounded rectangle; negative inside.
#[inline]
pub fn outline_sdf(sh: &Silhouette, x: f32, y: f32) -> f32 {
    let hw = (sh.r - sh.l) * 0.5;
    let hh = (sh.b - sh.t) * 0.5;
    let r = sh.radius.min(hw).min(hh);
    let qx = (x - (sh.l + sh.r) * 0.5).abs() - (hw - r);
    let qy = (y - (sh.t + sh.b) * 0.5).abs() - (hh - r);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// How far a rounded box `[x, y, w, h]` (with corner radius `round`) sits inside `sh`, when
/// scaled by `k` about `origin` and shifted vertically by `dy`.
#[inline]
pub fn box_clearance(
    sh: &Silhouette,
    [x, y, w, h]: [f32; 4],
    k: f32,
    round: f32,
    [ox, oy]: [f32; 2],
    dy: f32,
) -> f32 {
    let cap = (w * 0.5).min(h * 0.5).max(0.0);
    let r = round.min(cap).max(0.0);
    let x0 = x + cap;
    let y0 = y + cap;
    let x1 = (x + w - cap).max(x0);
    let y1 = (y + h - cap).max(y0);
    let mut worst = f32::NEG_INFINITY;
    for cx in [x0, x1] {
        for cy in [y0, y1] {
            let px = ox + (cx - ox) * k;
            let py = oy + (cy - oy) * k + dy;
            worst = worst.max(outline_sdf(sh, px, py));
        }
    }
    -worst - r * k
}

#[inline]
pub fn room(d: f32) -> f32 {
    smooth(CLEAR, CLEAR + EDGE_BAND, d)
}

#[inline]
pub fn scale_of(a: f32, reduced: bool) -> f32 {
    if reduced {
        1.0
    } else {
        mix(CONTENT_SCALE_FROM, 1.0, a.clamp(0.0, 1.0))
    }
}

#[inline]
pub fn push_of(a: f32, reduced: bool) -> f32 {
    if reduced {
        0.0
    } else {
        -CONTENT_PUSH_PT * (1.0 - a.clamp(0.0, 1.0))
    }
}

// ─── Elements & Presentations ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum ElemId {
    Lead = 0,
    DividerLead = 1,
    ChipArea = 2,
    ChipWindow = 3,
    ChipScreen = 4,
    ChipIphone = 5,
    Record = 6,
    DividerClose = 7,
    Close = 8,
    Cancel = 9,
    Finish = 10,
    Processing = 11,
    DoneLead = 12,
    Paste = 13,
    Inspect = 14,
    /// The Completed island's sessions one click away (smart routing).
    Route0 = 15,
    Route1 = 16,
}

impl ElemId {
    pub const COUNT: usize = 17;
    pub const ROUTES: [ElemId; 2] = [Self::Route0, Self::Route1];
    pub const ALL: [ElemId; Self::COUNT] = [
        Self::Lead,
        Self::DividerLead,
        Self::ChipArea,
        Self::ChipWindow,
        Self::ChipScreen,
        Self::ChipIphone,
        Self::Record,
        Self::DividerClose,
        Self::Close,
        Self::Cancel,
        Self::Finish,
        Self::Processing,
        Self::DoneLead,
        Self::Paste,
        Self::Inspect,
        Self::Route0,
        Self::Route1,
    ];

    pub const fn trace_key(self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::DividerLead => "divider.lead",
            Self::ChipArea => "chip:Area",
            Self::ChipWindow => "chip:Window",
            Self::ChipScreen => "chip:Screen",
            Self::ChipIphone => "chip:iPhone",
            Self::Record => "record",
            Self::DividerClose => "divider.close",
            Self::Close => "close",
            Self::Cancel => "cancel",
            Self::Finish => "finish",
            Self::Processing => "processing",
            Self::DoneLead => "done.lead",
            Self::Paste => "paste",
            Self::Inspect => "inspect",
            Self::Route0 => "route:0",
            Self::Route1 => "route:1",
        }
    }

    pub const fn from_mode(mode: SelectionMode) -> Self {
        match mode {
            SelectionMode::Area => Self::ChipArea,
            SelectionMode::Window => Self::ChipWindow,
            SelectionMode::Screen => Self::ChipScreen,
            SelectionMode::Iphone => Self::ChipIphone,
        }
    }

    pub const fn mode(self) -> Option<SelectionMode> {
        match self {
            Self::ChipArea => Some(SelectionMode::Area),
            Self::ChipWindow => Some(SelectionMode::Window),
            Self::ChipScreen => Some(SelectionMode::Screen),
            Self::ChipIphone => Some(SelectionMode::Iphone),
            _ => None,
        }
    }

    pub const fn is_interactive(self) -> bool {
        !matches!(
            self,
            Self::DividerLead | Self::DividerClose | Self::Processing
        )
    }

    pub const fn accessibility_label(self) -> &'static str {
        match self {
            Self::Lead => "Recording timer",
            Self::DividerLead | Self::DividerClose => "",
            Self::ChipArea => "Area",
            Self::ChipWindow => "Window",
            Self::ChipScreen => "Screen",
            Self::ChipIphone => "iPhone",
            Self::Record => "Record",
            Self::Close | Self::Cancel => "Cancel",
            Self::Finish => "Finish",
            Self::Processing => "Processing",
            Self::DoneLead => "Completed",
            Self::Paste => "Paste",
            Self::Inspect => "Inspect",
            Self::Route0 | Self::Route1 => "Send to session",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pres {
    Hidden,
    Recording {
        chips: bool,
        has_iphone: bool,
    },
    Selecting {
        mode: SelectionMode,
        has_iphone: bool,
    },
    Processing,
    /// `routes`: the labels of sessions offered one click away, beside Paste.
    Completed {
        label: String,
        routes: Vec<String>,
    },
}

impl Pres {
    /// Whether two presentations share the same geometry/layout structure (ignoring minor state
    /// like the active mode highlight when `shows_record` doesn't change).
    pub fn same_layout(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Hidden, Self::Hidden) | (Self::Processing, Self::Processing) => true,
            (
                Self::Recording {
                    chips: a,
                    has_iphone: ia,
                },
                Self::Recording {
                    chips: b,
                    has_iphone: ib,
                },
            ) => a == b && (ia == ib || !*a),
            (
                Self::Selecting {
                    mode: ma,
                    has_iphone: ia,
                },
                Self::Selecting {
                    mode: mb,
                    has_iphone: ib,
                },
            ) => ia == ib && ma.shows_record() == mb.shows_record(),
            (
                Self::Completed {
                    label: a,
                    routes: ra,
                },
                Self::Completed {
                    label: b,
                    routes: rb,
                },
            ) => a == b && ra == rb,
            _ => false,
        }
    }
}

/// Measured text widths (in points at 13 pt SF Pro) used to lay out capsule rows.
#[derive(Clone, Debug)]
pub struct Metrics {
    pub area: f32,
    pub window: f32,
    pub screen: f32,
    pub iphone: f32,
    pub record: f32,
    pub cancel: f32,
    pub finish: f32,
    pub processing: f32,
    pub paste: f32,
    pub inspect: f32,
    pub clock_000: f32,
    pub clock_0000: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            area: 30.0,
            window: 50.0,
            screen: 44.0,
            iphone: 43.0,
            record: 46.0,
            cancel: 44.0,
            finish: 38.0,
            processing: 78.0,
            paste: 36.0,
            inspect: 47.0,
            clock_000: 29.0,
            clock_0000: 37.0,
        }
    }
}

impl Metrics {
    pub fn mode_label(&self, mode: SelectionMode) -> f32 {
        match mode {
            SelectionMode::Area => self.area,
            SelectionMode::Window => self.window,
            SelectionMode::Screen => self.screen,
            SelectionMode::Iphone => self.iphone,
        }
    }

    pub fn segment_width(&self, mode: SelectionMode) -> f32 {
        self.mode_label(mode) + 2.0 * SEG_PX + SEG_ICON + SEG_ICON_GAP
    }

    pub fn chip_width(label_w: f32) -> f32 {
        label_w + 2.0 * CHIP_PX
    }

    pub fn clock_width(&self, clock: &str) -> f32 {
        if clock.len() <= 4 {
            self.clock_000
        } else {
            self.clock_0000
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LayoutItem {
    pub id: ElemId,
    pub rect: [f32; 4],
    pub round: f32,
    pub rank: usize,
    pub last: usize,
    pub active: f32,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub l: f32,
    pub r: f32,
    pub t: f32,
    pub b: f32,
}

impl Layout {
    pub fn centered(w: f32, h: f32) -> Self {
        Self {
            l: CX - w * 0.5,
            r: CX + w * 0.5,
            t: TOP,
            b: TOP + h,
        }
    }

    pub fn width(&self) -> f32 {
        self.r - self.l
    }

    pub fn height(&self) -> f32 {
        self.b - self.t
    }
}

/// Widths of the Completed island's changing labels, measured as their sprites are drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelWidths {
    pub completed: f32,
    pub routes: [f32; 2],
}

impl Default for LabelWidths {
    fn default() -> Self {
        Self {
            completed: 46.0,
            routes: [0.0; 2],
        }
    }
}

/// Computes the resting capsule bounds and item boxes for `pres`.
pub fn layout_of(
    pres: &Pres,
    clock: &str,
    labels: LabelWidths,
    m: &Metrics,
) -> (Layout, Vec<LayoutItem>) {
    let y0 = TOP + ISLAND_INSET;
    let mut raw: Vec<(ElemId, f32, f32, f32, bool)> = Vec::with_capacity(8);
    // (id, width, round, active_level, enabled)
    match pres {
        Pres::Hidden => {
            return (Layout::centered(IDLE_W, IDLE_H), Vec::new());
        }
        Pres::Selecting { mode, has_iphone } => {
            for &seg in SelectionMode::available(*has_iphone) {
                raw.push((
                    ElemId::from_mode(seg),
                    m.segment_width(seg),
                    CHIP_CORE_ROUND,
                    f32::from(u8::from(seg == *mode)),
                    true,
                ));
            }
            if mode.shows_record() {
                raw.push((
                    ElemId::Record,
                    Metrics::chip_width(m.record),
                    CHIP_CORE_ROUND,
                    0.0,
                    true,
                ));
            }
            raw.push((ElemId::DividerClose, DIVIDER_W, 0.0, 0.0, false));
            raw.push((ElemId::Close, CHIP_H, CHIP_CORE_ROUND, 0.0, true));
        }
        Pres::Recording { chips, has_iphone } => {
            let clock_w = m.clock_width(clock);
            let lead_w = CHIP_H + clock_w + LEAD_TRAIL;
            raw.push((ElemId::Lead, lead_w, CHIP_CORE_ROUND, 0.0, true));
            raw.push((ElemId::DividerLead, DIVIDER_W, 0.0, 0.0, false));
            if *chips {
                for &seg in SelectionMode::available(*has_iphone) {
                    raw.push((
                        ElemId::from_mode(seg),
                        m.segment_width(seg),
                        CHIP_CORE_ROUND,
                        f32::from(u8::from(seg == SelectionMode::Screen)),
                        true,
                    ));
                }
                raw.push((ElemId::DividerClose, DIVIDER_W, 0.0, 0.0, false));
                raw.push((ElemId::Close, CHIP_H, CHIP_CORE_ROUND, 0.0, true));
            } else {
                raw.push((
                    ElemId::Cancel,
                    Metrics::chip_width(m.cancel),
                    CHIP_CORE_ROUND,
                    0.0,
                    true,
                ));
                raw.push((
                    ElemId::Finish,
                    Metrics::chip_width(m.finish),
                    CHIP_CORE_ROUND,
                    0.0,
                    true,
                ));
            }
        }
        Pres::Processing => {
            let w = CHIP_H + 6.0 + m.processing + CHIP_PX;
            raw.push((ElemId::Processing, w, CHIP_CORE_ROUND, 0.0, false));
        }
        Pres::Completed { routes, .. } => {
            let lead_w = CHIP_H + 8.0 + labels.completed + LEAD_TRAIL;
            raw.push((ElemId::DoneLead, lead_w, CHIP_CORE_ROUND, 0.0, true));
            raw.push((ElemId::DividerLead, DIVIDER_W, 0.0, 0.0, false));
            raw.push((
                ElemId::Paste,
                Metrics::chip_width(m.paste),
                CHIP_CORE_ROUND,
                0.0,
                true,
            ));
            for (i, id) in ElemId::ROUTES.into_iter().enumerate().take(routes.len()) {
                raw.push((
                    id,
                    Metrics::chip_width(labels.routes[i]),
                    CHIP_CORE_ROUND,
                    0.0,
                    true,
                ));
            }
            raw.push((
                ElemId::Inspect,
                Metrics::chip_width(m.inspect),
                CHIP_CORE_ROUND,
                0.0,
                true,
            ));
            raw.push((ElemId::Close, CHIP_H, CHIP_CORE_ROUND, 0.0, true));
        }
    }

    let inner_w: f32 = raw.iter().map(|(_, w, ..)| *w).sum::<f32>()
        + raw.len().saturating_sub(1) as f32 * CHIP_GAP;
    let outer_w = inner_w + 2.0 * ISLAND_INSET;
    let lay = Layout::centered(outer_w, ISLAND_H);
    let last = raw.len().saturating_sub(1);
    let mut x = lay.l + ISLAND_INSET;
    let items = raw
        .into_iter()
        .enumerate()
        .map(|(rank, (id, w, round, active, enabled))| {
            let item = LayoutItem {
                id,
                rect: [x, y0, w, CHIP_H],
                round,
                rank,
                last,
                active,
                enabled,
            };
            x += w + CHIP_GAP;
            item
        })
        .collect();
    (lay, items)
}

// ─── Model State ────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
struct ElemState {
    x: SpringVal,
    y: SpringVal,
    w: f32,
    h: f32,
    round: f32,
    rank: usize,
    last: usize,
    a: SpringVal,
    blur: SpringVal,
    go_blur: SpringVal,
    cap: f32,
    trip: f32,
    travelling: bool,
    want: bool,
    want_since: f32,
    active: SpringVal,
    hover: SpringVal,
    press: SpringVal,
    enabled: bool,
    initialized: bool,
}

impl ElemState {
    const fn new() -> Self {
        Self {
            x: SpringVal::new(CX, SHARED_SPRING),
            y: SpringVal::new(TOP + ISLAND_INSET, SHARED_SPRING),
            w: CHIP_H,
            h: CHIP_H,
            round: CHIP_CORE_ROUND,
            rank: 0,
            last: 0,
            a: SpringVal::new(0.0, CONTENT_IN_SPRING),
            blur: SpringVal::new(1.0, CONTENT_IN_SPRING),
            go_blur: SpringVal::new(0.0, CONTENT_OUT_SPRING),
            cap: 1.0,
            trip: 0.0,
            travelling: false,
            want: false,
            want_since: 0.0,
            active: SpringVal::new(0.0, SpringCfg::new(0.18, 0.0)),
            hover: SpringVal::new(0.0, SpringCfg::new(0.14, 0.0)),
            press: SpringVal::new(0.0, SpringCfg::new(0.12, 0.0)),
            enabled: true,
            initialized: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FrameElem {
    pub id: ElemId,
    /// Drawn bounding box `[x, y, w, h]` in panel points.
    pub rect: [f32; 4],
    /// Unscaled resting width and height `[w, h]` for texture tile layout.
    pub base_size: [f32; 2],
    pub alpha: f32,
    pub blur: f32,
    pub scale: f32,
    pub active: f32,
    pub hover: f32,
    pub press: f32,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub struct FrameState {
    pub t: f32,
    pub shape: Silhouette,
    pub ghost: Option<Silhouette>,
    pub is_completed: bool,
    pub content_blur: f32,
    pub content_alpha: f32,
    pub elems: Vec<FrameElem>,
}

pub struct PillModel {
    pub t: f32,
    pub pres: Pres,
    pub geo_pres: Pres,
    pub geo_at: f32,
    pub reduced: bool,
    pub l: SpringVal,
    pub r: SpringVal,
    pub b: SpringVal,
    pub press: SpringVal,
    pub squish: SpringVal,
    pub shell_scale: SpringVal,
    pub shell_alpha: SpringVal,
    pub shell_blur: SpringVal,
    entering_from_hidden: bool,
    morph_start_w: f32,
    morph_start_h: f32,
    elems: [ElemState; ElemId::COUNT],
    ghost: Option<Silhouette>,
    dis: f32,
    pub hovered: Option<ElemId>,
    pub pressed: Option<ElemId>,
    pub clock: String,
    pub labels: LabelWidths,
    pub metrics: Metrics,
}

impl PillModel {
    pub fn new(metrics: Metrics) -> Self {
        let idle = Layout::centered(ISLAND_OPEN_FROM_W, ISLAND_H);
        Self {
            t: 0.0,
            pres: Pres::Hidden,
            geo_pres: Pres::Hidden,
            geo_at: 0.0,
            reduced: false,
            l: SpringVal::new(idle.l, OPEN_SPRING),
            r: SpringVal::new(idle.r, OPEN_SPRING),
            b: SpringVal::new(idle.b, SpringCfg::new(0.24, 0.0)),
            press: SpringVal::new(0.0, PRESS_SPRING),
            squish: SpringVal::new(0.0, TAP_SPRING),
            shell_scale: SpringVal::new(ENTRY_EXIT_SCALE, ENTRY_SHELL_SPRING),
            shell_alpha: SpringVal::new(0.0, ENTRY_ALPHA_SPRING),
            shell_blur: SpringVal::new(1.0, ENTRY_SHELL_SPRING),
            entering_from_hidden: false,
            morph_start_w: ISLAND_OPEN_FROM_W,
            morph_start_h: ISLAND_H,
            elems: [ElemState::new(); ElemId::COUNT],
            ghost: None,
            dis: 1.0,
            hovered: None,
            pressed: None,
            clock: "0:00".to_owned(),
            labels: LabelWidths::default(),
            metrics,
        }
    }

    /// Sets the target presentation. Retargets are C1 continuous: positions and velocities carry over.
    pub fn set_pres(&mut self, next: Pres) {
        if self.pres == next {
            return;
        }
        let layout_changed = !self.pres.same_layout(&next);
        let was_visible = !matches!(self.pres, Pres::Hidden);
        let now_hidden = matches!(next, Pres::Hidden);
        let (prev_lay, prev_items) =
            layout_of(&self.geo_pres, &self.clock, self.labels, &self.metrics);
        let (next_lay, next_items) = layout_of(&next, &self.clock, self.labels, &self.metrics);

        // Update element metadata for the new presentation immediately (e.g. highlight or enabled state).
        if !now_hidden {
            for item in &next_items {
                let st = &mut self.elems[item.id as usize];
                st.enabled = item.enabled;
                if !st.initialized {
                    st.active.snap(item.active);
                } else {
                    st.active.to(item.active, SpringCfg::new(0.16, 0.0));
                }
            }
        }

        self.pres = next.clone();

        if !layout_changed {
            self.geo_pres = next;
            return;
        }

        // Collapse wait: ONLY when narrowing between two visible states that have leaving items
        // (`was_visible && !now_hidden`). On Exit (`now_hidden`), content and shell leave together
        // immediately in place (`wait = 0.0`) without collapsing into a lozenge.
        let shrinking_between_visible = was_visible
            && !now_hidden
            && !self.reduced
            && next_lay.width() < prev_lay.width() - 1.0
            && prev_items
                .iter()
                .any(|p| !next_items.iter().any(|n| n.id == p.id));
        let wait = if shrinking_between_visible {
            COLLAPSE_WAIT_S
        } else {
            0.0
        };
        self.geo_at = self.t + wait;
        if wait == 0.0 {
            self.apply_geometry();
        }
    }

    fn apply_geometry(&mut self) {
        let next = self.pres.clone();
        let landing_idle = matches!(next, Pres::Hidden);
        let from_hidden = !landing_idle
            && (self.shell_alpha.x < 0.02
                || matches!(self.geo_pres, Pres::Hidden) && self.shell_alpha.x < 0.08);
        self.entering_from_hidden = from_hidden;
        self.geo_pres = next.clone();
        let (lay, items) = layout_of(&next, &self.clock, self.labels, &self.metrics);

        if from_hidden && !self.reduced {
            // Entry in place (no lozenge growth): snap width/height to the entering state's layout
            // while invisible (`alpha ≈ 0`) and scale in from `0.94 → 1.0` with `4 pt → 0` blur.
            self.l.snap(lay.l);
            self.r.snap(lay.r);
            self.b.snap(lay.b);
            if self.shell_alpha.x < 0.02 {
                self.shell_scale.snap(ENTRY_EXIT_SCALE);
                self.shell_blur.snap(1.0);
            }
        }

        let before_l = self.l.x;
        let before_r = self.r.x;
        let before_b = self.b.x;
        self.morph_start_w = (before_r - before_l).max(1.0);
        self.morph_start_h = (before_b - TOP).max(1.0);

        if self.reduced {
            let current = self.silhouette();
            if self.ghost.is_none() || self.dis >= 0.5 {
                self.ghost = Some(current);
                self.dis = 0.0;
            } else {
                self.dis = 1.0 - self.dis.max(1.0 - self.dis);
            }
            if !landing_idle {
                self.l.snap(lay.l);
                self.r.snap(lay.r);
                self.b.snap(lay.b);
            }
            self.shell_scale.snap(1.0);
            self.shell_alpha.to(
                if landing_idle { 0.0 } else { 1.0 },
                SpringCfg::new(REDUCED_FADE_S, 0.0),
            );
            self.shell_blur.snap(0.0);
        } else {
            // Fold any active press swell into the outline's position & velocity (`foldPress`).
            if self.press.x.abs() > 1e-4 || self.press.v.abs() > 1e-4 {
                let swell = self.press.x * PRESS_GROW_PT;
                let rate = self.press.v * PRESS_GROW_PT;
                self.l.x -= swell;
                self.r.x += swell;
                self.b.x += swell * 0.5;
                self.l.v -= rate;
                self.r.v += rate;
                self.b.v += rate * 0.5;
                self.press.snap(0.0);
            }

            if landing_idle {
                // Exit in place (no collapse into a 64×28 / 96×24 lozenge): keep the current
                // layout's target width/height while shrinking `shell_scale` `1.0 → 0.94`,
                // blurring `0 → 9 pt` (leading opacity), and fading `1.0 → 0.0` in `~190–200 ms`.
                let cur_l = self.l.target;
                let cur_r = self.r.target;
                let cur_b = self.b.target;
                self.l.to(cur_l, CLOSE_SPRING);
                self.r.to(cur_r, CLOSE_SPRING);
                self.b.to(cur_b, CLOSE_SPRING);
                self.shell_scale.to(ENTRY_EXIT_SCALE, EXIT_SHELL_SPRING);
                self.shell_alpha.to(0.0, EXIT_SHELL_SPRING);
                self.shell_blur.to(
                    1.0,
                    SpringCfg::new((EXIT_SHELL_SPRING.duration * BLUR_LEAD).max(0.04), 0.0),
                );
            } else if from_hidden {
                // Entry from Hidden: `0.94 → 1.0` scale, `4 pt → 0` blur, `0 → 1` alpha in `~230–250 ms`.
                self.shell_scale.to(1.0, ENTRY_SHELL_SPRING);
                self.shell_alpha.to(1.0, ENTRY_ALPHA_SPRING);
                self.shell_blur.to(
                    0.0,
                    SpringCfg::new((ENTRY_SHELL_SPRING.duration * BLUR_LEAD).max(0.04), 0.0),
                );
            } else {
                // Visible state-to-state width-led morph (`≤ 350 ms` settled, velocity preserved).
                let dw = (lay.l - before_l).abs().max((lay.r - before_r).abs());
                let dh = (lay.b - before_b).abs();
                let opening =
                    lay.width() * lay.height() >= (before_r - before_l) * (before_b - TOP) - 1.0;
                let base = if opening { OPEN_SPRING } else { CLOSE_SPRING };
                let f = (dw.hypot(dh) / REF_DIST).clamp(0.0, 1.0);
                let dur = (base.duration * mix(DISTANCE_FLOOR, 1.0, f)).max(0.14);
                let w_bounce = base.bounce;

                self.l.to(lay.l, SpringCfg::new(dur, w_bounce));
                self.r.to(lay.r, SpringCfg::new(dur, w_bounce));
                self.b.to(lay.b, SpringCfg::new(dur, 0.0));
                self.shell_scale.to(1.0, ENTRY_SHELL_SPRING);
                self.shell_alpha.to(1.0, ENTRY_ALPHA_SPRING);
                self.shell_blur.to(
                    0.0,
                    SpringCfg::new((ENTRY_SHELL_SPRING.duration * BLUR_LEAD).max(0.04), 0.0),
                );
            }
        }

        // Configure elements for the new layout: elements already visible (`a.x > 0.03`) that
        // also exist in `items` travel on `SHARED_SPRING` (`0.24 s`, bounce 0) with `travelBlur`;
        // newly entering elements snap to their resting spot while invisible and wait for room.
        for item in &items {
            let st = &mut self.elems[item.id as usize];
            let [ix, iy, iw, ih] = item.rect;
            st.w = iw;
            st.h = ih;
            st.round = item.round;
            st.rank = item.rank;
            st.last = item.last;
            st.enabled = item.enabled;
            if !st.initialized || st.a.x <= 0.03 || self.reduced {
                st.x.snap(ix);
                st.y.snap(iy);
                st.trip = 0.0;
                st.travelling = false;
                st.active.snap(item.active);
                st.initialized = true;
            } else {
                let dist = (st.x.x - ix).hypot(st.y.x - iy);
                if dist > 0.5 {
                    st.trip = dist;
                    st.travelling = true;
                }
                st.x.to(ix, SHARED_SPRING);
                st.y.to(iy, SHARED_SPRING);
                st.active.to(item.active, SpringCfg::new(0.18, 0.0));
            }
        }
    }

    /// Updates the recording clock string (`M:SS`); if the digit count changed (e.g. `9:59 -> 10:00`),
    /// updates layout widths smoothly.
    pub fn set_clock(&mut self, clock: &str) {
        if self.clock == clock {
            return;
        }
        let old_w = self.metrics.clock_width(&self.clock);
        let new_w = self.metrics.clock_width(clock);
        self.clock.clear();
        self.clock.push_str(clock);
        if (old_w - new_w).abs() > 0.5 && matches!(self.pres, Pres::Recording { .. }) {
            self.apply_geometry();
        }
    }

    /// Takes the Completed island's measured label widths; a change while it shows re-lays it
    /// out smoothly.
    /// `next` is the presentation about to be set: when it's already showing, only the widths
    /// changed and the layout follows them now; otherwise `set_pres` lays it out.
    pub fn set_label_widths(&mut self, labels: LabelWidths, next: &Pres) {
        let changed = (self.labels.completed - labels.completed).abs() > 0.5
            || (0..2).any(|i| (self.labels.routes[i] - labels.routes[i]).abs() > 0.5);
        self.labels = labels;
        if changed && self.pres == *next && matches!(next, Pres::Completed { .. }) {
            self.apply_geometry();
        }
    }

    /// Pointer hover/press updates in panel coordinates `(x, y)` (top-left origin `[0..640, 0..84]`).
    pub fn set_pointer(&mut self, pos: Option<[f32; 2]>, down: bool) {
        let hit = pos.and_then(|[x, y]| self.hit_element(x, y));
        self.hovered = hit;
        if down {
            if self.pressed.is_none() {
                self.pressed = hit;
                if hit.is_some() && !self.reduced {
                    self.press.to(1.0, PRESS_SPRING);
                }
            }
        } else {
            if self.pressed.is_some() && !self.reduced {
                let w = std::f32::consts::TAU / TAP_SPRING.duration;
                self.squish.v = (self.squish.v + TAP_SQUISH * w * 1.4)
                    .clamp(-TAP_SQUISH * w * 3.0, TAP_SQUISH * w * 3.0);
            }
            self.pressed = None;
            self.press.to(0.0, PRESS_SPRING);
        }
    }

    /// Hit-tests against the model's target capsule shape (`hitIsland` in `island/model.js`),
    /// plus a `6 pt` slop margin so reaching for chips keeps mouse events enabled.
    pub fn hit_island(&self, x: f32, y: f32) -> bool {
        if matches!(self.pres, Pres::Hidden) {
            return false;
        }
        let (lay, _) = layout_of(&self.pres, &self.clock, self.labels, &self.metrics);
        let slop = 6.0;
        x >= lay.l - slop && x <= lay.r + slop && y >= lay.t - slop && y <= lay.b + slop
    }

    /// Hit-tests interactive elements at `(x, y)` in panel points.
    pub fn hit_element(&self, x: f32, y: f32) -> Option<ElemId> {
        if matches!(self.pres, Pres::Hidden) {
            return None;
        }
        let (_, items) = layout_of(&self.pres, &self.clock, self.labels, &self.metrics);
        for item in &items {
            if !item.id.is_interactive() || !item.enabled {
                continue;
            }
            let st = &self.elems[item.id as usize];
            if st.a.x < 0.15 {
                continue;
            }
            let [rx, ry, rw, rh] = [st.x.x, st.y.x, st.w, st.h];
            let slop = 2.0;
            if x >= rx - slop && x <= rx + rw + slop && y >= ry - slop && y <= ry + rh + slop {
                return Some(item.id);
            }
        }
        None
    }

    /// Current capsule silhouette (`L, T, R, B, radius, alpha, blur`), including entry/exit scale,
    /// press swell, and tap squish. `T` is anchored at `TOP` so it never bounces upward into the
    /// top of the screen.
    pub fn silhouette(&self) -> Silhouette {
        self.silhouette_from(self.l.x, self.r.x, self.b.x, self.shell_scale.x)
    }

    fn silhouette_from(&self, mut l: f32, mut r: f32, mut b: f32, scale: f32) -> Silhouette {
        let swell = self.press.x * PRESS_GROW_PT;
        l -= swell;
        r += swell;
        b += swell * 0.5;
        let sq = self.squish.x;
        if sq.abs() > 1e-6 {
            let mid = (l + r) * 0.5;
            let w = (r - l) * (1.0 + sq);
            let h = (b - TOP) * (1.0 - sq * 1.2);
            l = mid - w * 0.5;
            r = mid + w * 0.5;
            b = TOP + h;
        }
        let k_shell = if self.reduced {
            1.0
        } else {
            scale.clamp(ENTRY_EXIT_SCALE * 0.9, 1.05)
        };
        let base_w = (r - l).max(IDLE_W * 0.5);
        let base_h = (b - TOP).max(IDLE_H * 0.5);
        let mid = CX + ((l + r) * 0.5 - CX) * k_shell;
        let w = base_w * k_shell;
        let h = base_h * k_shell;
        l = mid - w * 0.5;
        r = mid + w * 0.5;
        b = TOP + h;
        let radius = radius_of(h, ISLAND_RADIUS * k_shell, RADIUS_BLEND * k_shell);
        let alpha = self.shell_alpha.x.clamp(0.0, 1.0);
        let blur = if self.reduced {
            0.0
        } else {
            self.shell_blur.x.clamp(0.0, 1.0) * SHELL_BLUR_MAX
        };
        Silhouette {
            l,
            t: TOP,
            r,
            b,
            radius,
            alpha,
            blur,
        }
    }

    /// Predicts the capsule silhouette over the next `lead` seconds on its current springs at 240 Hz.
    fn predict(&self, lead: f32) -> Vec<(f32, Silhouette)> {
        let mut l = self.l;
        let mut r = self.r;
        let mut b = self.b;
        let mut s = self.shell_scale;
        let mut out = Vec::with_capacity(28);
        let mut t = AHEAD_STEP;
        while t <= lead + 1e-9 {
            l.step(AHEAD_STEP);
            r.step(AHEAD_STEP);
            b.step(AHEAD_STEP);
            s.step(AHEAD_STEP);
            out.push((t, self.silhouette_from(l.x, r.x, b.x, s.x)));
            t += AHEAD_STEP;
        }
        out
    }

    fn grown_enough(&self) -> bool {
        let target_w = self.r.target - self.l.target;
        let cur_w = self.r.x - self.l.x;
        let span = target_w - self.morph_start_w;
        if span <= 0.5 {
            return true;
        }
        (cur_w - self.morph_start_w) / span >= CONTENT_DELAY
    }

    /// Advances the simulation by `dt` seconds.
    pub fn advance(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, 0.1);
        if dt <= 0.0 {
            return;
        }
        self.integrate(dt);
    }

    fn integrate(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        self.t += dt;
        if self.geo_at <= self.t + 1e-3 && self.geo_pres != self.pres {
            self.apply_geometry();
        }

        let reduced = self.reduced;
        if reduced {
            self.l.snap(self.l.target);
            self.r.snap(self.r.target);
            self.b.snap(self.b.target);
            self.press.snap(0.0);
            self.squish.snap(0.0);
            self.shell_scale.snap(1.0);
            self.shell_alpha.step_critical(dt, REDUCED_FADE_S, 100.0);
            self.shell_blur.snap(0.0);
            self.dis = (self.dis + dt / REDUCED_FADE_S).min(1.0);
            if self.dis >= 1.0 {
                self.ghost = None;
            }
        } else {
            self.l.step(dt);
            self.r.step(dt);
            self.b.step(dt);
            self.press.step(dt);
            self.squish.step(dt);
            self.shell_scale
                .step_critical(dt, self.shell_scale.duration, 100.0);
            self.shell_alpha
                .step_critical(dt, self.shell_alpha.duration, 100.0);
            self.shell_blur
                .step_critical(dt, self.shell_blur.duration, 100.0);
            if self.shell_scale.x > 0.99 && self.shell_alpha.x > 0.95 {
                self.entering_from_hidden = false;
            }
        }

        let now = self.silhouette();
        let k_shell = if reduced {
            1.0
        } else {
            self.shell_scale.x.clamp(ENTRY_EXIT_SCALE * 0.9, 1.05)
        };
        let ahead = if reduced {
            Vec::new()
        } else {
            self.predict(EXIT_LEAD_S)
        };
        let settled_geo = self.geo_pres == self.pres;
        let grown = self.grown_enough();
        let (_, geo_items) = layout_of(&self.geo_pres, &self.clock, self.labels, &self.metrics);
        let (_, active_items) = layout_of(&self.pres, &self.clock, self.labels, &self.metrics);

        let inn = if reduced {
            SpringCfg::new(REDUCED_FADE_S, 0.0)
        } else {
            CONTENT_IN_SPRING
        };
        let out = if reduced {
            SpringCfg::new(REDUCED_FADE_S, 0.0)
        } else {
            CONTENT_OUT_SPRING
        };

        // Boxes of shared items still travelling: a session chip inserted between them waits until
        // they've cleared its spot, so it never fades in underneath one.
        let movers: Vec<[f32; 4]> = ElemId::ALL
            .iter()
            .map(|id| &self.elems[*id as usize])
            .filter(|st| st.travelling && st.a.x > 0.03)
            .map(|st| [st.x.x, st.y.x, st.w, st.h])
            .collect();

        for id in ElemId::ALL {
            let idx = id as usize;
            let in_active = active_items.iter().any(|item| item.id == id);
            let in_geo = geo_items.iter().any(|item| item.id == id);
            // During `COLLAPSE_WAIT_S` (`!settled_geo`), shared elements that exist in both the
            // old and new presentation stay visible while outgoing elements fade out.
            let in_pres = in_active && (settled_geo || in_geo);
            let is_hovered = self.hovered == Some(id);
            let is_pressed = self.pressed == Some(id);

            let st = &mut self.elems[idx];
            st.hover
                .to(f32::from(u8::from(is_hovered)), SpringCfg::new(0.14, 0.0));
            st.press.to(
                f32::from(u8::from(is_pressed)),
                SpringCfg::new(if is_pressed { 0.10 } else { 0.20 }, 0.0),
            );
            st.hover.step_critical(dt, 0.14, 100.0);
            st.press.step_critical(dt, 0.14, 100.0);
            st.active.step_critical(dt, 0.16, 100.0);

            if st.travelling && in_pres && !reduced {
                st.x.step(dt);
                st.y.step(dt);
                // Inelastic containment: keep travelling shared elements at least CLEAR inside the
                // current unscaled outline (`contain` in `island/model.js`).
                let min_x = self.l.x + ISLAND_INSET;
                let max_x = (self.r.x - ISLAND_INSET - st.w).max(min_x);
                if st.x.x < min_x {
                    st.x.x = min_x;
                    if st.x.v < 0.0 {
                        st.x.v = 0.0;
                    }
                } else if st.x.x > max_x {
                    st.x.x = max_x;
                    if st.x.v > 0.0 {
                        st.x.v = 0.0;
                    }
                }
                let left = (st.x.x - st.x.target).hypot(st.y.x - st.y.target);
                let going = st.trip > 2.0 && left > st.trip * 0.22;
                st.go_blur.target = f32::from(u8::from(going));
                let blur_dur = if going {
                    0.035
                } else {
                    (out.duration * BLUR_LEAD).max(0.03)
                };
                st.go_blur.step_critical(dt, blur_dur, 100.0);
                if left < 1e-3 && st.x.v.abs() < 1e-2 && st.go_blur.x < 1e-3 {
                    st.x.snap(st.x.target);
                    st.y.snap(st.y.target);
                    st.travelling = false;
                    st.trip = 0.0;
                }
            } else {
                st.x.snap(st.x.target);
                st.y.snap(st.y.target);
                st.travelling = false;
                st.go_blur.target = 0.0;
                st.go_blur.step_critical(dt, 0.08, 100.0);
            }

            let k = if st.travelling {
                k_shell
            } else {
                scale_of(st.a.x, reduced).min(k_shell)
            };
            let dy = if st.travelling {
                0.0
            } else {
                push_of(st.a.x, reduced)
            };
            let box_rect = [st.x.x, st.y.x, st.w, st.h];
            let d = box_clearance(&now, box_rect, k, st.round, [CX, TOP], dy);
            let d_rest = box_clearance(&now, box_rect, k_shell, st.round, [CX, TOP], 0.0);

            let covered = ElemId::ROUTES.contains(&id)
                && movers
                    .iter()
                    .any(|m| m[0] < box_rect[0] + box_rect[2] && box_rect[0] < m[0] + m[2]);
            let gate = reduced
                || st.travelling
                || st.want
                || (settled_geo && grown && !covered && d_rest >= CLEAR + EDGE_BAND - 0.8);
            let want = in_pres && gate;
            if st.want != want {
                st.want = want;
                st.want_since = self.t - dt;
            }
            let turn = if reduced || st.travelling {
                0.0
            } else if want {
                let lead_delay = if self.entering_from_hidden {
                    ENTRY_CONTENT_DELAY_S
                } else {
                    0.0
                };
                lead_delay + CONTENT_STAGGER_S * st.rank as f32
            } else if matches!(self.pres, Pres::Hidden) {
                // On Exit to Hidden, content and shell leave together without stagger lingering.
                0.0
            } else {
                // Outgoing items nearest either shrinking outer cap leave first (0 delay).
                let from_cap = st.rank.min(st.last.saturating_sub(st.rank));
                CONTENT_STAGGER_S * from_cap as f32 * 0.5
            };
            if self.t - st.want_since >= turn - 1e-6 {
                st.a.to(f32::from(u8::from(want)), if want { inn } else { out });
            }
            let on = st.a.target >= 0.5;
            st.a.step(dt);

            // Blur runs on its own critically damped clock at `BLUR_LEAD` (0.55×) of the fade.
            st.blur.target = if on { 0.0 } else { 1.0 };
            let blur_dur = if reduced {
                0.05
            } else {
                ((if on { inn.duration } else { out.duration }) * BLUR_LEAD).max(0.03)
            };
            st.blur.step_critical(dt, blur_dur, 100.0);

            // Look-ahead exit cap (`stepCap` in `island/model.js`): only for non-travelling elements
            // during visible morphs (on Exit to Hidden, the capsule shrinks in place so edges don't clip).
            let hidden = st.a.x <= 1e-3;
            if !reduced && !(st.travelling && in_pres) && !matches!(self.pres, Pres::Hidden) {
                let contact = if hidden {
                    f32::INFINITY
                } else {
                    let round = st.round;
                    ahead
                        .iter()
                        .find(|(_, sh)| {
                            box_clearance(sh, box_rect, k, round, [CX, TOP], dy) < CLEAR
                        })
                        .map_or(f32::INFINITY, |(at_t, _)| *at_t)
                };
                if d < CLEAR {
                    st.cap = if hidden { 1.0 } else { 0.0 };
                } else if contact.is_finite() {
                    st.cap -= st.cap * (dt / contact.max(dt)).min(1.0);
                } else if hidden {
                    st.cap = 1.0;
                } else {
                    st.cap += dt / EXIT_LEAD_S;
                }
                st.cap = st.cap.clamp(0.0, 1.0);
            } else {
                st.cap = 1.0;
            }
        }
    }

    /// Evaluates the complete frame state for rendering and motion-lint tracing.
    pub fn frame(&self) -> FrameState {
        let reduced = self.reduced;
        let mut shape = self.silhouette();
        let k_shell = if reduced {
            1.0
        } else {
            self.shell_scale.x.clamp(ENTRY_EXIT_SCALE * 0.9, 1.05)
        };
        let ghost = if reduced
            && let Some(mut g) = self.ghost
            && self.dis < 1.0
        {
            g.alpha *= 1.0 - self.dis;
            shape.alpha *= self.dis;
            Some(g)
        } else {
            None
        };

        let mut elems = Vec::with_capacity(10);
        let mut max_blur = 0.0_f32;
        let mut max_alpha = 0.0_f32;

        for id in ElemId::ALL {
            let st = &self.elems[id as usize];
            if !st.initialized {
                continue;
            }
            let base_k = if st.travelling {
                k_shell
            } else {
                scale_of(st.a.x, reduced).min(k_shell)
            };
            let press_k = 1.0 - 0.05 * st.press.x.clamp(0.0, 1.0);
            let dy = if st.travelling {
                0.0
            } else {
                push_of(st.a.x, reduced)
            };
            let box_rect = [st.x.x, st.y.x, st.w, st.h];
            let d = box_clearance(&shape, box_rect, base_k, st.round, [CX, TOP], dy);
            let edge = if !reduced {
                st.cap.clamp(0.0, 1.0).min(room(d))
            } else if d >= CLEAR {
                1.0
            } else if let Some(ref g) = ghost
                && box_clearance(g, box_rect, base_k, st.round, [CX, TOP], dy) >= CLEAR
            {
                g.alpha
            } else {
                0.0
            };
            let alpha = (st.a.x.clamp(0.0, 1.0).min(edge) * shape.alpha).clamp(0.0, 1.0);
            if alpha <= 1e-4 {
                continue;
            }
            let enter_exit_blur = if reduced {
                0.0
            } else {
                st.blur
                    .x
                    .clamp(0.0, 1.0)
                    .max(self.shell_blur.x.clamp(0.0, 1.0))
                    .max(((1.0 - st.cap) / BLUR_LEAD).clamp(0.0, 1.0))
                    * CONTENT_BLUR_MAX
            };
            let travel_blur = if reduced {
                0.0
            } else {
                st.go_blur.x.clamp(0.0, 1.0) * TRAVEL_BLUR_MAX
            };
            let blur = enter_exit_blur.max(travel_blur);

            // Transform `[x, y, w, h]` by `base_k` about `(CX, TOP)` + `dy`, then `press_k` about item center.
            let sx = CX + (st.x.x - CX) * base_k;
            let sy = TOP + (st.y.x - TOP) * base_k + dy;
            let sw = st.w * base_k;
            let sh = st.h * base_k;
            let icx = sx + sw * 0.5;
            let icy = sy + sh * 0.5;
            let fw = sw * press_k;
            let fh = sh * press_k;
            let rect = [icx - fw * 0.5, icy - fh * 0.5, fw, fh];

            max_blur = max_blur.max(blur * alpha);
            max_alpha = max_alpha.max(alpha);
            elems.push(FrameElem {
                id,
                rect,
                base_size: [st.w, st.h],
                alpha,
                blur,
                scale: base_k * press_k,
                active: st.active.x.clamp(0.0, 1.0),
                hover: st.hover.x.clamp(0.0, 1.0),
                press: st.press.x.clamp(0.0, 1.0),
                enabled: st.enabled,
            });
        }

        FrameState {
            t: self.t,
            shape,
            ghost,
            is_completed: matches!(self.geo_pres, Pres::Completed { .. }),
            content_blur: max_blur,
            content_alpha: max_alpha,
            elems,
        }
    }

    /// True once every spring has settled and no delayed geometry transition is pending.
    pub fn is_at_rest(&self) -> bool {
        if self.geo_pres != self.pres {
            return false;
        }
        if self.reduced && self.dis < 1.0 {
            return false;
        }
        let outline_rest = [
            &self.l,
            &self.r,
            &self.b,
            &self.press,
            &self.squish,
            &self.shell_scale,
            &self.shell_alpha,
            &self.shell_blur,
        ]
        .into_iter()
        .all(|s| (s.x - s.target).abs() < 1e-3 && s.v.abs() < 1e-2);
        if !outline_rest {
            return false;
        }
        self.elems.iter().all(|e| {
            !e.initialized
                || ((e.cap <= 1e-3 || e.cap >= 1.0 - 1e-3)
                    && [
                        &e.x, &e.y, &e.a, &e.blur, &e.go_blur, &e.active, &e.hover, &e.press,
                    ]
                    .into_iter()
                    .all(|s| (s.x - s.target).abs() < 1e-3 && s.v.abs() < 1e-2))
        })
    }

    /// True when the capsule is in `Pres::Hidden` and its outline and alpha have settled to rest.
    pub fn is_fully_hidden(&self) -> bool {
        matches!(self.pres, Pres::Hidden)
            && matches!(self.geo_pres, Pres::Hidden)
            && self.shell_alpha.x < 0.005
            && self.is_at_rest()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME_DT: f32 = 1.0 / 120.0;

    /// Independent rounded-rect inside distance check (from `island/model.test.js`), written
    /// without sharing `outline_sdf` or `box_clearance` with the implementation under test.
    fn independent_inside(sh: &Silhouette, x: f32, y: f32) -> f32 {
        let r = sh.radius.min((sh.r - sh.l) * 0.5).min((sh.b - sh.t) * 0.5);
        let nx = x.clamp(sh.l + r, sh.r - r);
        let ny = y.clamp(sh.t + r, sh.b - r);
        let from_core = (x - nx).hypot(y - ny);
        if from_core > 0.0 {
            r - from_core
        } else {
            (x - sh.l).min(sh.r - x).min(y - sh.t).min(sh.b - y)
        }
    }

    fn assert_frame_room_invariants(model: &PillModel, f: &FrameState) {
        assert!(f.shape.l.is_finite() && f.shape.r.is_finite() && f.shape.b.is_finite());
        // Outline never bounces upward above TOP.
        assert!(
            (f.shape.t - TOP).abs() < 1e-5 && f.shape.b >= TOP,
            "outline bounced above TOP: {:?}",
            f.shape
        );
        for elem in &f.elems {
            assert!(
                elem.alpha >= 0.0 && elem.alpha <= 1.0 + 1e-5,
                "element {:?} alpha out of bounds: {}",
                elem.id,
                elem.alpha
            );
            if elem.alpha > 1e-3 && !model.reduced {
                let st = &model.elems[elem.id as usize];
                let cap = (elem.rect[2] * 0.5).min(elem.rect[3] * 0.5);
                let r = (st.round * elem.scale).min(cap);
                let x0 = elem.rect[0] + cap;
                let y0 = elem.rect[1] + cap;
                let x1 = (elem.rect[0] + elem.rect[2] - cap).max(x0);
                let y1 = (elem.rect[1] + elem.rect[3] - cap).max(y0);
                let mut min_inside = f32::INFINITY;
                for cx in [x0, x1] {
                    for cy in [y0, y1] {
                        min_inside = min_inside.min(independent_inside(&f.shape, cx, cy) - r);
                    }
                }
                assert!(
                    min_inside >= CLEAR - 0.15,
                    "element {:?} visible (alpha={:.3}) with clearance {:.2} < CLEAR at t={:.3}",
                    elem.id,
                    elem.alpha,
                    min_inside,
                    f.t
                );
            }
        }
    }

    #[test]
    fn content_alpha_never_exceeds_room_on_any_frame() {
        let scenes: Vec<Vec<(f32, Pres)>> = vec![
            // tap-then-area
            vec![
                (
                    0.0,
                    Pres::Recording {
                        chips: true,
                        has_iphone: false,
                    },
                ),
                (
                    1.2,
                    Pres::Selecting {
                        mode: SelectionMode::Area,
                        has_iphone: false,
                    },
                ),
                (2.4, Pres::Hidden),
            ],
            // tap-then-window
            vec![
                (
                    0.0,
                    Pres::Recording {
                        chips: true,
                        has_iphone: false,
                    },
                ),
                (
                    1.2,
                    Pres::Selecting {
                        mode: SelectionMode::Window,
                        has_iphone: false,
                    },
                ),
                (2.4, Pres::Hidden),
            ],
            // tap-then-screen
            vec![
                (
                    0.0,
                    Pres::Recording {
                        chips: true,
                        has_iphone: false,
                    },
                ),
                (
                    1.2,
                    Pres::Recording {
                        chips: false,
                        has_iphone: false,
                    },
                ),
                (2.4, Pres::Hidden),
            ],
            // picker
            vec![
                (
                    0.0,
                    Pres::Selecting {
                        mode: SelectionMode::Screen,
                        has_iphone: false,
                    },
                ),
                (
                    0.9,
                    Pres::Selecting {
                        mode: SelectionMode::Area,
                        has_iphone: false,
                    },
                ),
                (
                    1.8,
                    Pres::Selecting {
                        mode: SelectionMode::Screen,
                        has_iphone: false,
                    },
                ),
                (
                    2.7,
                    Pres::Recording {
                        chips: false,
                        has_iphone: false,
                    },
                ),
                (3.9, Pres::Hidden),
            ],
            // finish
            vec![
                (
                    0.0,
                    Pres::Recording {
                        chips: true,
                        has_iphone: false,
                    },
                ),
                (
                    0.9,
                    Pres::Recording {
                        chips: false,
                        has_iphone: false,
                    },
                ),
                (2.0, Pres::Processing),
                (
                    2.8,
                    Pres::Completed {
                        label: "Copied".into(),
                        routes: Vec::new(),
                    },
                ),
                (4.2, Pres::Hidden),
            ],
        ];

        for steps in scenes {
            let mut m = PillModel::new(Metrics::default());
            let mut step_idx = 0;
            let total_s = steps.last().unwrap().0 + 1.5;
            while m.t < total_s {
                while step_idx < steps.len() && m.t >= steps[step_idx].0 - 1e-6 {
                    m.set_pres(steps[step_idx].1.clone());
                    step_idx += 1;
                }
                m.advance(FRAME_DT);
                let f = m.frame();
                assert_frame_room_invariants(&m, &f);
            }
            assert!(m.is_at_rest());
            assert!(m.is_fully_hidden());
        }
    }

    #[test]
    fn retargeting_mid_flight_is_c1_continuous() {
        let mut m = PillModel::new(Metrics::default());
        m.set_pres(Pres::Processing);
        for _ in 0..35 {
            m.advance(FRAME_DT);
        }
        // Start a visible width expansion (`Processing` -> `Recording(chips)`).
        m.set_pres(Pres::Recording {
            chips: true,
            has_iphone: false,
        });
        for _ in 0..8 {
            m.advance(FRAME_DT);
        }
        let before = (m.l.x, m.l.v, m.r.x, m.r.v, m.b.x, m.b.v);
        assert!(
            before.3 > 50.0,
            "expected active outward velocity, got {}",
            before.3
        );

        // Retarget mid-expansion to Selecting(Area): position and velocity must carry over unchanged.
        m.set_pres(Pres::Selecting {
            mode: SelectionMode::Area,
            has_iphone: false,
        });
        assert_eq!((m.l.x, m.l.v, m.r.x, m.r.v, m.b.x, m.b.v), before);

        // Next frame continues smoothly without a jump.
        m.advance(FRAME_DT);
        assert!((m.l.x - before.0).abs() < 12.0);
        assert!((m.r.x - before.2).abs() < 12.0);
    }

    #[test]
    fn every_scene_can_interrupt_every_other_and_settle() {
        let all_states = [
            Pres::Hidden,
            Pres::Recording {
                chips: true,
                has_iphone: false,
            },
            Pres::Recording {
                chips: false,
                has_iphone: false,
            },
            Pres::Selecting {
                mode: SelectionMode::Area,
                has_iphone: false,
            },
            Pres::Selecting {
                mode: SelectionMode::Screen,
                has_iphone: true,
            },
            Pres::Processing,
            Pres::Completed {
                label: "Copied".into(),
                routes: Vec::new(),
            },
        ];

        for from in &all_states {
            for to in &all_states {
                for interrupt_s in [0.04, 0.11, 0.22] {
                    let mut m = PillModel::new(Metrics::default());
                    m.set_pres(from.clone());
                    let mut elapsed = 0.0;
                    while elapsed < interrupt_s {
                        m.advance(FRAME_DT);
                        assert_frame_room_invariants(&m, &m.frame());
                        elapsed += FRAME_DT;
                    }
                    let prev_f = m.frame();
                    let was_vis = prev_f.shape.alpha > 0.08;
                    m.set_pres(to.clone());
                    m.advance(FRAME_DT);
                    let next_f = m.frame();
                    if was_vis && next_f.shape.alpha > 0.08 {
                        assert!(
                            (next_f.shape.l - prev_f.shape.l).abs() <= 16.0
                                && (next_f.shape.r - prev_f.shape.r).abs() <= 16.0,
                            "jump on visible interrupt {:?} -> {:?} at {:.2}s",
                            from,
                            to,
                            interrupt_s
                        );
                    }
                    for _ in 0..180 {
                        m.advance(FRAME_DT);
                        assert_frame_room_invariants(&m, &m.frame());
                    }
                    assert!(
                        m.is_at_rest(),
                        "failed to settle after {:?} -> {:?}",
                        from,
                        to
                    );
                }
            }
        }
    }

    #[test]
    fn blur_resolves_ahead_of_opacity_and_shared_lead_travels_with_blur() {
        let mut m = PillModel::new(Metrics::default());
        m.set_pres(Pres::Recording {
            chips: true,
            has_iphone: false,
        });
        let mut saw_sharp_before_opaque = false;
        for _ in 0..120 {
            m.advance(FRAME_DT);
            let st = &m.elems[ElemId::ChipArea as usize];
            if st.a.x > 0.35 && st.a.x < 0.92 && st.blur.x < 0.10 {
                saw_sharp_before_opaque = true;
            }
        }
        assert!(
            saw_sharp_before_opaque,
            "blur should resolve ahead of opacity (blurLead = 0.55)"
        );

        // Fold chips into RecordingControls: `Lead` is shared and travels with travelBlur > 0,
        // then clears its blur before landing.
        m.set_pres(Pres::Recording {
            chips: false,
            has_iphone: false,
        });
        let mut peak_travel_blur = 0.0_f32;
        for _ in 0..120 {
            m.advance(FRAME_DT);
            let f = m.frame();
            if let Some(lead) = f.elems.iter().find(|e| e.id == ElemId::Lead) {
                peak_travel_blur = peak_travel_blur.max(lead.blur);
            }
        }
        assert!(
            peak_travel_blur > 1.0,
            "expected shared lead travel blur, got {peak_travel_blur}"
        );
        let settled = m.frame();
        let lead = settled
            .elems
            .iter()
            .find(|e| e.id == ElemId::Lead)
            .expect("lead visible");
        assert!(lead.blur < 1e-3 && (lead.alpha - 1.0).abs() < 1e-3);
    }

    #[test]
    fn outline_width_bounces_on_open_and_close() {
        // 1. Entry from Hidden (`~0.94` scale + `~4 pt` blur -> sharp `1.0` in ≤ 260 ms, no lozenge).
        let mut m = PillModel::new(Metrics::default());
        m.set_pres(Pres::Recording {
            chips: true,
            has_iphone: false,
        });
        let target_w = m.r.target - m.l.target;
        m.advance(FRAME_DT);
        let f0 = m.frame();
        assert!(
            f0.shape.width() >= target_w * ENTRY_EXIT_SCALE - 1.0,
            "entry should start at ~0.94 scale of target width (not a narrow lozenge), got {}",
            f0.shape.width()
        );
        assert!(
            f0.shape.blur > SHELL_BLUR_MAX * 0.6,
            "expected blurred shell on entry start"
        );

        for _ in 0..60 {
            m.advance(FRAME_DT);
        }

        // 2. Visible state-to-state width expansion (`Processing` -> `Completed`) has width overshoot
        //    and settles in ≤ 350 ms while `t` stays anchored at `TOP`.
        m.set_pres(Pres::Processing);
        for _ in 0..60 {
            m.advance(FRAME_DT);
        }
        m.set_pres(Pres::Completed {
            label: "Copied".into(),
            routes: Vec::new(),
        });
        let done_target_w = m.r.target - m.l.target;
        let mut max_w = 0.0_f32;
        for _ in 0..80 {
            m.advance(FRAME_DT);
            let f = m.frame();
            max_w = max_w.max(f.shape.width());
            assert_eq!(f.shape.t, TOP);
        }
        assert!(
            max_w > done_target_w + 0.5,
            "expected width cushion on visible widening morph, max_w={max_w} target_w={done_target_w}"
        );

        // 3. Exit to Hidden shrinks in place (`1.0 -> 0.94` scale, blur up to ~4 pt, fade in ~200 ms)
        //    without collapsing into a lozenge.
        m.set_pres(Pres::Hidden);
        let mut min_vis_w = f32::INFINITY;
        let mut exit_done_s = None;
        let exit_start = m.t;
        for _ in 0..60 {
            m.advance(FRAME_DT);
            let f = m.frame();
            if f.shape.alpha > 0.02 {
                min_vis_w = min_vis_w.min(f.shape.width());
            } else if exit_done_s.is_none() {
                exit_done_s = Some(m.t - exit_start);
            }
        }
        assert!(
            min_vis_w >= done_target_w * ENTRY_EXIT_SCALE - 2.0,
            "exit should shrink in place to ~0.94 scale, not a lozenge: min_vis_w={min_vis_w}"
        );
        let dur = exit_done_s.expect("exit finished");
        assert!(
            (0.15..=0.24).contains(&dur),
            "expected exit to finish in ~180–220 ms, got {:.3}s",
            dur
        );
    }

    #[test]
    fn opening_content_follows_half_the_growth_inner_first() {
        let mut m = PillModel::new(Metrics::default());
        m.set_pres(Pres::Processing);
        for _ in 0..60 {
            m.advance(FRAME_DT);
        }
        let start_w = m.frame().shape.width();
        m.set_pres(Pres::Recording {
            chips: true,
            has_iphone: false,
        });
        let target_w = m.r.target - m.l.target;
        let mut first: Vec<(ElemId, f32)> = Vec::new();
        for _ in 0..140 {
            m.advance(FRAME_DT);
            let f = m.frame();
            for elem in &f.elems {
                if elem.id != ElemId::Processing
                    && elem.alpha > 0.02
                    && !first.iter().any(|(id, _)| *id == elem.id)
                {
                    let grown = (f.shape.width() - start_w) / (target_w - start_w);
                    assert!(
                        grown >= CONTENT_DELAY - 0.02,
                        "{:?} appeared at grown={grown}",
                        elem.id
                    );
                    first.push((elem.id, m.t));
                }
            }
        }
        let at = |want: ElemId| first.iter().find(|(id, _)| *id == want).unwrap().1;
        assert!(
            at(ElemId::ChipWindow) <= at(ElemId::Close),
            "inner chip enters before outer close chip"
        );
    }

    #[test]
    fn recording_controls_are_cancel_and_finish() {
        let pres = Pres::Recording {
            chips: false,
            has_iphone: false,
        };
        let (layout, items) = layout_of(&pres, "0:00", LabelWidths::default(), &Metrics::default());
        let ids: Vec<ElemId> = items.iter().map(|i| i.id).collect();
        assert_eq!(
            ids,
            [
                ElemId::Lead,
                ElemId::DividerLead,
                ElemId::Cancel,
                ElemId::Finish
            ]
        );
        let last = items.last().unwrap().rect;
        assert!((layout.r - (last[0] + last[2]) - ISLAND_INSET).abs() < 0.5);
    }

    #[test]
    fn reduce_motion_dissolves_without_travel() {
        let mut m = PillModel::new(Metrics::default());
        m.reduced = true;
        m.set_pres(Pres::Recording {
            chips: true,
            has_iphone: false,
        });
        for _ in 0..60 {
            m.advance(FRAME_DT);
        }
        let w0 = m.frame().shape.width();
        m.set_pres(Pres::Selecting {
            mode: SelectionMode::Area,
            has_iphone: false,
        });
        m.advance(FRAME_DT);
        let f = m.frame();
        assert!(
            f.ghost
                .as_ref()
                .is_some_and(|g| (g.width() - w0).abs() < 1e-3 && g.alpha > 0.8),
            "expected dissolving ghost silhouette in Reduce Motion"
        );
        assert_eq!(f.content_blur, 0.0);
        for _ in 0..60 {
            m.advance(FRAME_DT);
        }
        assert!(m.frame().ghost.is_none());
    }
}
