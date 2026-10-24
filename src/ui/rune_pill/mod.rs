//! `scrawl lab pill <scene> --rune`: the capsule drawn by Rune instead of the Metal panel, on the
//! lab's timeline and pretend recording. It runs Rune's event loop (not GPUI's) in a transparent
//! borderless window placed where the capsule's panel goes, so moving it into an overlay later
//! changes only placement.
//!
//! What it shows and where everything rests is decided in `surface` (one `Surface`, Rune's
//! state); the view below only draws it, and Rune's springs animate between commits. The red
//! dot and the spinner animate through `Live` values a pretend meter thread publishes, and the
//! timer through a `LiveText`, without re-running the view.

pub(crate) mod grid;
mod proto;
pub(crate) mod style;
pub(crate) mod surface;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use rune::color::srgb_decode;
use rune::{
    App, AppSpec, Color, Compositor, Cx, DisplayTarget, El, Frame, LineCap, Live, LiveText, NativeDisplayId,
    OverlayHandle, OverlayInput, OverlayLevel, OverlayOptions, OverlayPlacement, OverlaySpec, Path, RunOptions, Shape,
    SpringPair, TextStyle, Transition, Tx, WindowSpec, canvas, el, row, text,
};

use self::style::style;
pub(crate) use self::surface::{PANEL_H, Shown, Surface};
use self::surface::{DONE_TEXT_X, LEAD_TEXT_X, PROCESSING_TEXT_X};
use super::lab::{BACKDROP, Scene, Step, Timeline};
use super::{SelectionMode, motion};
use crate::native::pill::PillAction;
pub(crate) use crate::native::pill::model::{PANEL_W, PANEL_Y};
use crate::native::pill::model::{CHIP_H, ElemId, Pres};

pub struct Options {
    pub scene: Scene,
    pub repeat: u32,
    pub backdrop: bool,
    /// Headless goldens: a `.png` renders `state`; a directory renders every state into it.
    pub snap: Option<PathBuf>,
    pub state: Option<String>,
    /// A real Screen capture made while the scene plays (`Capture`).
    pub capture: Option<PathBuf>,
}

/// The states `--snap` renders, as they rest.
const STATES: &[&str] = &[
    "picker",
    "picker-area",
    "recording-chips",
    "recording",
    "processing",
    "completed",
    "completed-routes",
];

fn state_pres(name: &str) -> Option<(Pres, &'static str)> {
    let pres = match name {
        "picker" => Pres::Selecting { mode: SelectionMode::Screen, has_iphone: true },
        "picker-area" => Pres::Selecting { mode: SelectionMode::Area, has_iphone: true },
        "recording-chips" => Pres::Recording { chips: true, has_iphone: true },
        "recording" => Pres::Recording { chips: false, has_iphone: true },
        "processing" => Pres::Processing,
        "completed" => Pres::Completed { label: "Copied".into(), routes: Vec::new() },
        "completed-routes" => Pres::Completed {
            label: "Copied".into(),
            routes: vec!["Send to scrawl".into(), "Send to rune".into()],
        },
        _ => return None,
    };
    Some((pres, "0:07"))
}

// ─── Colours (SwiftUI `PillColor`) ───────────────────────────────────────────────────────────

fn srgb(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color { r: srgb_decode(r), g: srgb_decode(g), b: srgb_decode(b), a }
}

fn white(level: f32) -> Color {
    srgb(level, level, level, 1.0)
}

/// White at `alpha` over the capsule, as AppKit composites it (in sRGB): Rune blends in linear
/// light, where the same alpha reads far brighter.
fn veil(alpha: f32) -> Color {
    over_shell(Color::WHITE, alpha)
}

/// `color` at `alpha` over the capsule, blended as AppKit would (see `veil`).
fn over_shell(color: Color, alpha: f32) -> Color {
    rune::theme::srgb_equivalent(color.alpha(alpha), rgb3(style().shell))
}

fn rgb3(c: [f32; 3]) -> Color {
    srgb(c[0], c[1], c[2], 1.0)
}

fn text_color() -> Color {
    rgb3(style().text)
}

fn red() -> Color {
    srgb(0.941, 0.267, 0.220, 1.0)
}

fn green() -> Color {
    rgb3(style().green)
}

fn accent(hover: bool) -> Color {
    let s = style();
    rgb3(if hover { s.accent_hover } else { s.accent })
}

fn medium() -> TextStyle {
    style().medium_text()
}

fn semibold() -> TextStyle {
    style().semibold_text()
}

/// The timer: SF's tabular digits, so a new second never changes its width (`monospacedDigit()`).
fn timer_style() -> TextStyle {
    semibold().features("'tnum' 1")
}

// ─── Text ────────────────────────────────────────────────────────────────────────────────────

static FONTS: std::sync::OnceLock<&'static [&'static [u8]]> = std::sync::OnceLock::new();

thread_local! {
    static TEXT: std::cell::RefCell<rune::text::TextSystem> = std::cell::RefCell::new({
        let fonts = FONTS.get().copied().unwrap_or(&[]);
        rune::text::TextSystem::new(fonts, families(fonts))
    });
}

pub(crate) fn families(fonts: &[&[u8]]) -> (&'static str, &'static str) {
    if fonts.is_empty() { ("Inter", "Inter") } else { ("System Font", "System Font") }
}

/// `text`'s width in points as Rune draws it (13 pt, medium or semibold), so chips hug the
/// labels they show.
pub fn measure(text: &str, bold: bool) -> f32 {
    let style = if bold { semibold() } else { medium() };
    TEXT.with_borrow_mut(|t| t.measure(text, style, 2.0).0)
}

fn clock_width(clock: &str) -> f32 {
    TEXT.with_borrow_mut(|t| t.measure(clock, timer_style(), 2.0).0)
}

fn metrics() -> crate::native::pill::model::Metrics {
    crate::native::pill::model::Metrics {
        area: measure("Area", false),
        window: measure("Window", false),
        screen: measure("Screen", false),
        iphone: measure("iPhone", false),
        record: measure("Record", true),
        cancel: measure("Cancel", false),
        finish: measure("Finish", true),
        processing: measure("Processing…", true),
        paste: measure("Paste", true),
        inspect: measure("Inspect", false),
        clock_000: clock_width("0:00"),
        clock_0000: clock_width("00:00"),
    }
}

const PROCESSING: &str = "Processing…";
const PROCESSING_GLYPHS: usize = 11;

/// "Processing…" split into glyphs, each with its x and width as the whole label shapes them.
fn processing_glyphs() -> [(char, f32, f32); PROCESSING_GLYPHS] {
    let mut out = [(' ', 0.0, 0.0); PROCESSING_GLYPHS];
    for (k, (at, c)) in PROCESSING.char_indices().enumerate() {
        let end = at + c.len_utf8();
        let x = if at == 0 { 0.0 } else { measure(&PROCESSING[..at], true) };
        out[k] = (c, x, measure(&PROCESSING[..end], true) - x);
    }
    out
}

/// A `Surface` laid out with the capsule's real label widths (call `fonts` first).
pub(crate) fn new_surface() -> Surface {
    Surface::new(motion::tune(), metrics())
}

/// What the capsule draws beside its `Surface`: the live values (the dot, the spinner and the
/// edge glow, from a meter thread) and the timer's text. Whoever owns the surface owns one of
/// these: publish the clock through `set_clock`, call `sync` after changing the surface, and
/// draw with `capsule`.
pub(crate) struct Capsule {
    meter: Meter,
    /// The timer, so a new second redraws without re-running the view; only a change of
    /// length (9:59 → 10:00) re-lays the capsule out.
    clock: LiveText,
    /// `surface.morphs` as last passed to the meter's edge glow.
    morphs: u32,
}

impl Capsule {
    /// Starts the thread that animates the live values (one per capsule).
    pub(crate) fn new(surface: &Surface) -> Self {
        let capsule = Self::with_meter(surface, Meter::new());
        capsule.meter.clone().spawn();
        capsule
    }

    fn with_meter(surface: &Surface, meter: Meter) -> Self {
        Self { meter, clock: LiveText::new(surface.clock.clone()), morphs: surface.morphs }
    }

    /// Shows `clock`; the surface re-lays out only when its length changes. True when the view
    /// must re-run.
    pub(crate) fn set_clock(&self, surface: &mut Surface, clock: &str, now: f64) -> bool {
        self.clock.set(clock.to_owned());
        surface.set_clock(clock, now)
    }

    /// Tells the meter what the surface shows: which animations run, at what speed, and when a
    /// morph lights the edge.
    pub(crate) fn sync(&mut self, surface: &Surface) {
        let activity = match surface.pres {
            Pres::Recording { .. } => 1,
            Pres::Processing => 2,
            _ => 0,
        };
        self.meter.activity.store(activity, Ordering::Relaxed);
        self.meter.time_scale.store(surface.time_scale.to_bits(), Ordering::Relaxed);
        if surface.morphs != self.morphs {
            self.morphs = surface.morphs;
            self.meter.kick.store(surface.morph_settle.max(0.2).to_bits(), Ordering::Release);
        }
    }
}

/// What a press on `id` asks the app to do, as `NativePill::dispatch_click` decides it.
#[allow(dead_code, reason = "the app mounts the capsule on rune/app; the lab plays presses itself")]
pub(crate) fn action(pres: &Pres, id: ElemId) -> Option<PillAction> {
    let selecting = matches!(pres, Pres::Selecting { .. });
    let mode = |m| if selecting { PillAction::SelectMode(m) } else { PillAction::SwitchRecording(m) };
    Some(match id {
        ElemId::ChipArea => mode(SelectionMode::Area),
        ElemId::ChipWindow => mode(SelectionMode::Window),
        ElemId::ChipScreen => mode(SelectionMode::Screen),
        ElemId::ChipIphone => mode(SelectionMode::Iphone),
        ElemId::Record => PillAction::RecordSelected,
        ElemId::Cancel => PillAction::Cancel,
        ElemId::Finish => PillAction::Finish,
        ElemId::Close if matches!(pres, Pres::Completed { .. }) => PillAction::DismissCompleted,
        ElemId::Close => PillAction::Cancel,
        ElemId::Paste => PillAction::Paste(None),
        ElemId::Inspect => PillAction::Inspect,
        ElemId::Route0 => PillAction::Route(0),
        ElemId::Route1 => PillAction::Route(1),
        _ => return None,
    })
}

// ─── Live values ─────────────────────────────────────────────────────────────────────────────

/// What the pretend meter thread animates.
#[derive(Clone)]
struct Meter {
    /// The dot's core breath, its ripple ring's scale and fade (one 1.6 s cycle).
    core: Live<f32>,
    ring_scale: Live<f32>,
    ring_alpha: Live<f32>,
    /// The spinner's eight dots.
    spin: [Live<f32>; 8],
    /// "Processing…" one glyph each, lit by a band of light passing across (`pillShimmer`).
    shimmer: [Live<f32>; PROCESSING_GLYPHS],
    /// Each glyph's centre across the label, 0…1.
    shimmer_at: [f32; PROCESSING_GLYPHS],
    /// The glow inside the capsule's edge while it morphs (SwiftUI's `pillEdgeGlow` energy).
    glow: Live<f32>,
    /// A morph's settle time in seconds (`f32` bits), for the thread to start a glow; 0 when none.
    kick: Arc<AtomicU32>,
    /// 0 idle, 1 recording, 2 processing (written by the UI, read by the thread).
    activity: Arc<AtomicU8>,
    time_scale: Arc<AtomicU32>,
}

/// The edge glow `dt` seconds into a morph that settles in `settle`: from `from` up to 1 over
/// 0.05 s, then a critically damped spring back to 0 (SwiftUI's keyframes).
fn glow_at(dt: f32, settle: f32, from: f32) -> f32 {
    const RISE: f32 = 0.05;
    if dt < RISE {
        return from + (1.0 - from) * dt / RISE;
    }
    let w = std::f32::consts::TAU / settle.max(0.2);
    let t = dt - RISE;
    (1.0 + w * t) * (-w * t).exp()
}

impl Meter {
    fn new() -> Self {
        let meter = Self {
            core: Live::new(1.0),
            ring_scale: Live::new(0.4),
            ring_alpha: Live::new(0.0),
            spin: std::array::from_fn(|_| Live::new(0.18)),
            shimmer: std::array::from_fn(|_| Live::new(1.0)),
            shimmer_at: {
                let total = measure(PROCESSING, true).max(1.0);
                let glyphs = processing_glyphs();
                std::array::from_fn(|k| (glyphs[k].1 + glyphs[k].2 * 0.5) / total)
            },
            glow: Live::new(0.0),
            kick: Arc::default(),
            activity: Arc::default(),
            time_scale: Arc::new(AtomicU32::new(1.0_f32.to_bits())),
        };
        meter.publish(0.0);
        meter
    }

    /// The values at animation second `t` (the SwiftUI `pillDot` shader and `PillSpinner`).
    fn publish(&self, t: f64) {
        let phase = (t / 1.6).fract() as f32;
        let breath = 0.5 + 0.5 * (std::f32::consts::TAU * phase).cos();
        self.core.set(0.78 + 0.22 * breath);
        self.ring_scale.set((4.0 + 6.0 * phase) / 10.0);
        self.ring_alpha.set(0.34 * (1.0 - phase) * (1.0 - phase));
        let head = (t * 8.0).rem_euclid(8.0) as f32;
        for (k, dot) in self.spin.iter().enumerate() {
            let phase = (head - k as f32).rem_euclid(8.0);
            dot.set((1.0 - phase / 7.5).clamp(0.18, 1.0));
        }
        // A 1.8 s ease-in-out sweep from left of the label to right of it, repeating.
        let sweep = (t / 1.8).fract() as f32;
        let centre = -0.3 + 1.6 * (0.5 - 0.5 * (std::f32::consts::PI * sweep).cos());
        for (glyph, x) in self.shimmer.iter().zip(self.shimmer_at) {
            let d = (x - centre) / 0.14;
            glyph.set(0.62 + 0.38 * (-d * d).exp());
        }
    }

    /// Publishes at 120 Hz while something animates, the way an audio callback would.
    fn spawn(self) {
        std::thread::Builder::new()
            .name("rune-pill-meter".into())
            .spawn(move || {
                let mut t = 0.0_f64;
                let mut last = Instant::now();
                let mut morph: Option<(f64, f32, f32)> = None;
                let mut glow = 0.0_f32;
                loop {
                    std::thread::sleep(Duration::from_millis(8));
                    let now = Instant::now();
                    let scale = f32::from_bits(self.time_scale.load(Ordering::Relaxed));
                    t += (now - last).as_secs_f64() * f64::from(scale);
                    last = now;
                    if self.activity.load(Ordering::Relaxed) != 0 {
                        self.publish(t);
                    }
                    let kick = f32::from_bits(self.kick.swap(0, Ordering::AcqRel));
                    if kick > 0.0 {
                        morph = Some((t, kick, glow));
                    }
                    if let Some((since, settle, from)) = morph {
                        glow = glow_at((t - since) as f32, settle, from);
                        if t - since > 0.05 && glow < 0.002 {
                            glow = 0.0;
                            morph = None;
                        }
                        self.glow.set(glow);
                    }
                }
            })
            .expect("meter thread");
    }
}

// ─── State ───────────────────────────────────────────────────────────────────────────────────

pub struct Lab {
    /// This window is the plain backdrop behind the capsule.
    backdrop: Option<Color>,
    /// A headless golden: the state rests, nothing plays.
    still: bool,
    surface: Surface,
    live: Capsule,
    /// The scene's steps, shared with the Metal lab.
    timeline: Timeline,
    recording_since: Option<f64>,
    /// Animation seconds the capsule went to `Hidden` at, so the overlay hides after the exit.
    hidden_since: Option<f64>,
    now: f64,
    ctl: Arc<Ctl>,
}

/// What `poll` tells the thread driving the overlay (`drive`), which has no view of the state.
#[derive(Default)]
struct Ctl {
    /// `drive` has started the scene (after any capture): until then `poll` does nothing, so
    /// prewarm frames don't start the timeline.
    go: AtomicBool,
    /// The last run's tail has passed.
    done: AtomicBool,
    /// `poll` has shown the overlay: the capsule is up or still leaving.
    shown: AtomicBool,
    /// The capsule's overlay, which `poll` shows and hides.
    pill: std::sync::OnceLock<OverlayHandle<Lab>>,
    /// The picker is open, so the capsule follows the pointer between displays (the Metal
    /// capsule's `follow_pointer_display`).
    follow: AtomicBool,
    driver: std::sync::OnceLock<std::thread::Thread>,
}

impl Ctl {
    fn notify(&self) {
        if let Some(driver) = self.driver.get() {
            driver.unpark();
        }
    }
}

impl Lab {
    /// `animate`: the live values run (the capsule overlay's; a backdrop or a golden has none).
    fn capsule(scene: Scene, repeat: u32, animate: bool) -> Self {
        motion::poll_tune();
        let surface = new_surface();
        let live = if animate { Capsule::new(&surface) } else { Capsule::with_meter(&surface, Meter::new()) };
        Self {
            backdrop: None,
            still: false,
            surface,
            live,
            timeline: Timeline::new(scene, repeat),
            recording_since: None,
            // Hidden since long before the run, so the overlay starts hidden.
            hidden_since: Some(f64::NEG_INFINITY),
            now: 0.0,
            ctl: Arc::default(),
        }
    }

    fn set_clock(&mut self, clock: &str, now: f64) -> bool {
        self.live.set_clock(&mut self.surface, clock, now)
    }

    /// A click on the capsule, as the lab plays it (the app sends `action` instead).
    fn press(&mut self, id: ElemId) {
        let now = self.now;
        let anim = self.surface.anim(now);
        match id {
            ElemId::Close | ElemId::Cancel | ElemId::Paste | ElemId::Inspect | ElemId::Route0 | ElemId::Route1 => {
                self.surface.set(Pres::Hidden, now);
            }
            _ => self.surface.click(id, now),
        }
        self.track_recording(anim);
    }

    fn apply(&mut self, step: Step, now: f64) {
        motion::trace_event(format!("lab:{step:?}"));
        let anim = self.surface.anim(now);
        let s = &mut self.surface;
        match step {
            Step::Tap => s.set(Pres::Recording { chips: true, has_iphone: false }, now),
            Step::Chip(mode) | Step::Click(mode) => s.click(ElemId::from_mode(mode), now),
            Step::Open => s.set(Pres::Selecting { mode: SelectionMode::Screen, has_iphone: false }, now),
            Step::Cancel | Step::Dismiss => s.set(Pres::Hidden, now),
            Step::Finish => s.finish(now),
        }
        self.track_recording(anim);
    }

    /// A recording that starts (or is discarded) resets the clock.
    fn track_recording(&mut self, anim: f64) {
        match self.surface.pres {
            Pres::Recording { .. } => {
                self.recording_since.get_or_insert(anim);
            }
            _ => self.recording_since = None,
        }
        self.live.sync(&self.surface);
    }
}

fn init() -> Lab {
    Lab::capsule(Scene::Picker, 1, false)
}

/// How long the capsule's exit gets before its overlay hides, in animation seconds.
/// Inside the scene's tail, so the last run hides before it quits.
const HIDE_AFTER_S: f64 = super::lab::TAIL_S as f64 - 0.1;
/// How often a hidden capsule polls for `Ctl::go`, in real seconds.
const WAIT_FOR_GO_S: f64 = 0.01;

/// The lab's frame hook: tune reload, due surface steps, the scene's steps (the same `Timeline`
/// the Metal lab steps), the clock, and what the overlay's driver should do.
fn poll(s: &mut Lab, now: f64, tx: &mut Tx) -> bool {
    if s.backdrop.is_some() || s.still {
        return false;
    }
    if !s.ctl.go.load(Ordering::Acquire) {
        // A poll that only schedules keeps the prewarmed frame current (an update wouldn't).
        tx.wake_after(Duration::from_secs_f64(WAIT_FOR_GO_S));
        return false;
    }
    s.now = now;
    motion::poll_tune();
    let tune = motion::tune();
    s.surface.tune = tune;
    s.surface.set_time_scale(now, tune.time_scale);
    let scale = f64::from(s.surface.time_scale);
    let anim = s.surface.anim(now);

    let mut changed = s.surface.tick(now);
    let due = s.timeline.advance(anim);
    for step in due.steps {
        s.apply(step, now);
        changed = true;
    }
    if changed {
        s.track_recording(anim);
    }
    s.live.sync(&s.surface);
    if due.done {
        motion::trace_close();
        s.ctl.done.store(true, Ordering::Release);
        s.ctl.notify();
        return changed;
    }

    // The clock reads off the animation time, like the app's pretend recording.
    let mut wake = s.surface.next_due().map(|due| due - now);
    if let Some(since) = s.recording_since {
        let secs = (anim - since).max(0.0);
        let whole = secs as u64;
        let clock = format!("{}:{:02}", whole / 60, whole % 60);
        changed |= s.set_clock(&clock, now);
        let tick = (whole as f64 + 1.0 - secs) / scale;
        wake = Some(wake.map_or(tick, |w| w.min(tick)));
    }
    if let Some(at) = s.timeline.next_at() {
        let step = (at - anim) / scale;
        wake = Some(wake.map_or(step, |w| w.min(step)));
    }
    // The overlay stays up while the capsule leaves, then hides (ordered in at alpha 0).
    let hidden = matches!(s.surface.pres, Pres::Hidden);
    let since = match (hidden, s.hidden_since) {
        (true, None) => *s.hidden_since.insert(anim),
        (true, Some(since)) => since,
        (false, _) => {
            s.hidden_since = None;
            anim
        }
    };
    let leaving = (since + HIDE_AFTER_S - anim) / scale;
    if hidden && leaving > 0.0 {
        wake = Some(wake.map_or(leaving, |w| w.min(leaving)));
    }
    // Shown in the same poll as the step that brings the capsule in, so Rune reveals the
    // prewarmed hidden pose and animates the entry from it.
    let shown = !hidden || leaving > 0.0;
    if s.ctl.shown.swap(shown, Ordering::AcqRel) != shown
        && let Some(pill) = s.ctl.pill.get()
    {
        if shown { pill.show() } else { pill.hide() }
    }
    s.ctl.follow.store(
        matches!(s.surface.pres, Pres::Selecting { mode, .. } if mode != SelectionMode::Iphone),
        Ordering::Release,
    );
    if let Some(wake) = wake {
        tx.wake_after(Duration::from_secs_f64(wake.max(0.0)));
    }
    s.ctl.notify();

    for event in s.surface.events.drain(..) {
        motion::trace_event(event);
    }
    if std::mem::take(&mut s.surface.instant) {
        tx.instant();
    }
    changed
}

// ─── View ────────────────────────────────────────────────────────────────────────────────────

fn view(s: &Lab, cx: &mut Cx<Lab>) -> El {
    if let Some(fill) = s.backdrop {
        return el().w_full().h_full().fill(fill);
    }
    let capsule = capsule(&s.surface, &s.live, cx, Lab::press);
    if s.still {
        // Goldens show the capsule over the lab's backdrop, so its shadow and rim read.
        return el().size(PANEL_W, PANEL_H).fill(srgb(0.851, 0.851, 0.871, 1.0)).child(capsule);
    }
    capsule
}

/// The capsule in its `PANEL_W` × `PANEL_H` panel, as `surface` and `live` say. A press on a
/// chip, button or ✕ calls `on_press` (`action` says what it means to the app); the shell and
/// those are the only `.hit()` boxes, for `OverlayInput::Regions`.
pub(crate) fn capsule<S: 'static>(
    surface: &Surface,
    live: &Capsule,
    cx: &mut Cx<S>,
    on_press: impl Fn(&mut S, ElemId) + Clone + 'static,
) -> El {
    cx.time_scale(surface.time_scale);
    cx.reduced_motion(surface.reduced);
    let mut root = el().size(PANEL_W, PANEL_H);
    root = root.child(shell(surface, &live.meter));
    let blur = if surface.reduced { 0.0 } else { PRESENCE_BLUR };
    let presence = Transition::blur(blur)
        .combined(Transition::scale(0.96))
        .springs(SpringPair::new(surface.presence(), surface::PRESENCE_EXIT));
    // Everything inside is masked by the capsule as it moves (SwiftUI's `.mask`): a box on the
    // shell's rect and spring clips to its rounded shape, and the box inside it moves back by
    // the same amount on the same spring, so the content stays where it's laid out in panel
    // points while the walls pass over it.
    let [x, y, w, h] = surface.shell;
    let mut inner = el().key("inner").absolute(-x, -y).size(PANEL_W, PANEL_H).motion(surface.shell_motion);
    if let Some(h) = &surface.highlight {
        inner = inner.child(
            placed(el(), h)
                .radius(CHIP_H * 0.5)
                .gradient(veil(0.216), veil(0.16))
                .transition(presence),
        );
    }
    // Content entering over content that leaves (lead → Processing → done, chips → Finish)
    // crossfades additively, without a midpoint alpha dip. The highlight is a shared background,
    // so it stays outside; nothing inside overlaps at rest.
    let mut content = el().key("content").absolute(0.0, 0.0).size(PANEL_W, PANEL_H).dissolve();
    for shown in &surface.items {
        content = content.child(item(surface, live, shown, on_press.clone()).transition(presence));
    }
    let clip = el().key("clip").absolute(x, y).size(w, h).motion(surface.shell_motion).clip_rounded(h * 0.5);
    root.child(clip.child(inner.child(content)))
}

/// Peak blur sigma (pt) of content entering or leaving in place: the SwiftUI capsule's `pill.blur`.
const PRESENCE_BLUR: f32 = 1.5;

fn placed(e: El, shown: &Shown) -> El {
    let [x, y, w, h] = shown.rect;
    e.key(shown.key.clone()).absolute(x, y).size(w, h).motion(shown.motion)
}

fn shell(surface: &Surface, meter: &Meter) -> El {
    let [x, y, w, h] = surface.shell;
    let completed = matches!(surface.pres, Pres::Completed { .. });
    let inset = |y: f32, blur: f32, spread: f32, color: Color| rune::Shadow { x: 0.0, y, blur, spread, color, inset: true };
    shell_body(el().key("shell").absolute(x, y).size(w, h), h)
        // The glow inside the edge while the shape morphs, stronger at the top.
        .child(
            el().key("glow").absolute(0.0, 0.0).size(w, h).radius(h * 0.5)
                .box_shadow(inset(1.5, 8.0, 0.0, veil(0.07)))
                .live_opacity(&meter.glow)
                .motion(surface.shell_motion),
        )
        .child(rim(w, h, completed).motion(surface.shell_motion))
        .opacity(if surface.shell_on { 1.0 } else { 0.0 })
        .motion(surface.shell_motion)
        .opacity_motion(surface.shell_fade)
        // The overlay takes clicks only on `.hit()` boxes: the island, so a click on it never
        // reaches the app below, and the chips in it.
        .hit()
}

/// The capsule's fill and drop shadow on `e`, `h` tall.
fn shell_body(e: El, h: f32) -> El {
    let s = style();
    e.radius(h * 0.5)
        // SwiftUI's 0.96, blended in sRGB there; in Rune's linear blend a light desktop would
        // lift it visibly, so nearly opaque.
        .fill(srgb(s.shell[0], s.shell[1], s.shell[2], s.shell_alpha))
        .shadow(4.0, 18.0, Color::BLACK.alpha(s.shadow_alpha))
}

/// SwiftUI's rim, a `strokeBorder` over the fill: a 0.5 pt hairline lit from above (white 0.15
/// at the top, 0.04 at the bottom), or the done state's 0.75 pt green. A Rune border replaces
/// its own fill under the ring, so the ring goes on an unfilled child to sit over it.
fn rim(w: f32, h: f32, completed: bool) -> El {
    let bw = if completed { 0.75 } else { 0.5 };
    let rim = el().key("rim").absolute(0.0, 0.0).size(w, h).radius(h * 0.5);
    if completed {
        rim.border(bw, over_shell(green(), 0.5))
    } else {
        rim.border_gradient(bw, veil(0.15), veil(0.04))
    }
}

fn label(content: &str, style: TextStyle, color: Color) -> El {
    text(content).font(style).color(color)
}

fn item<S: 'static>(surface: &Surface, live: &Capsule, shown: &Shown, on_press: impl Fn(&mut S, ElemId) + 'static) -> El {
    let id = shown.id;
    let press = move |state: &mut S| on_press(state, id);
    let base = placed(row().center(), shown);
    match id {
        ElemId::ChipArea | ElemId::ChipWindow | ElemId::ChipScreen | ElemId::ChipIphone => {
            let fg = white(if shown.active { 0.961 } else { 0.608 });
            let name = id.accessibility_label();
            let mut chip = base.radius(CHIP_H * 0.5).gap(5.0)
                .child(canvas(icon(id, 14.0 * 2.5 / 24.0, fg)).size(14.0, 14.0))
                .child(label(name, medium(), fg))
                .press(veil(0.06));
            if !shown.active {
                chip = chip.hover(veil(0.08));
            }
            chip.hit().on_press(press)
        }
        ElemId::Record | ElemId::Finish | ElemId::Paste => {
            let name = id.accessibility_label();
            base.radius(CHIP_H * 0.5)
                .fill(accent(false))
                .hover(accent(true))
                .child(label(name, semibold(), srgb(0.95, 0.95, 0.96, 1.0)))
                .hit().on_press(press)
        }
        ElemId::Cancel | ElemId::Inspect | ElemId::Route0 | ElemId::Route1 => {
            let name = match (id, &surface.pres) {
                (ElemId::Route0, Pres::Completed { routes, .. }) => routes.first().cloned().unwrap_or_default(),
                (ElemId::Route1, Pres::Completed { routes, .. }) => routes.get(1).cloned().unwrap_or_default(),
                _ => id.accessibility_label().to_owned(),
            };
            let fill = style().chip_fill;
            let chip = if fill > 0.0 { base.fill(veil(fill)) } else { base };
            chip.radius(CHIP_H * 0.5)
                .hover(veil(0.08 + fill))
                .press(veil(0.06 + fill))
                .child(label(&name, medium(), rgb3(style().secondary)))
                .hit().on_press(press)
        }
        ElemId::Close => base.child(
            row().center().size(28.0, 28.0).radius(14.0).hover(veil(0.10)).press(veil(0.06))
                .child(canvas(icon(id, 12.0 * 2.4 / 24.0, white(0.61))).size(12.0, 12.0))
                .hit().on_press(press),
        ),
        ElemId::Lead => placed(el(), shown).child(dot(&live.meter)).child(timer(surface, live)),
        ElemId::Processing => placed(el(), shown)
            .child(spinner(&live.meter))
            .child(
                processing_label(&live.meter),
            ),
        ElemId::DoneLead => {
            let done = match &surface.pres {
                Pres::Completed { label, .. } => label.clone(),
                _ => String::new(),
            };
            placed(el(), shown)
                .child(el().absolute(5.0, 5.0).size(22.0, 22.0).radius(11.0).fill(over_shell(green(), 0.22)))
                .child(
                    canvas(icon(id, 13.0 * 2.6 / 24.0, green())).absolute(9.5, 9.5).size(13.0, 13.0),
                )
                .child(
                    row().center_y().absolute(DONE_TEXT_X, 0.0).h(CHIP_H)
                        .child(label(&done, semibold(), text_color())),
                )
        }
        ElemId::DividerLead | ElemId::DividerClose => placed(el(), shown),
    }
}

/// The timer in tabular digits, bound to its `LiveText` in a box as wide as the clock.
fn timer(surface: &Surface, live: &Capsule) -> El {
    row().center_y().absolute(LEAD_TEXT_X, 0.0).h(CHIP_H).child(
        label(&surface.clock, timer_style(), text_color()).live_text(&live.clock).w(clock_width(&surface.clock)),
    )
}

/// "Processing…", each glyph at its place in the shaped label and lit by the shimmer.
fn processing_label(meter: &Meter) -> El {
    let mut glyphs = el().absolute(PROCESSING_TEXT_X, 0.0).h(CHIP_H).w(measure(PROCESSING, true));
    for ((c, x, w), live) in processing_glyphs().into_iter().zip(&meter.shimmer) {
        glyphs = glyphs.child(
            row().center_y().absolute(x, 0.0).h(CHIP_H).w(w)
                .child(label(c.encode_utf8(&mut [0; 4]), semibold(), text_color()))
                .live_opacity(live),
        );
    }
    glyphs
}

/// The recording dot: a breathing core with a glow, and a ripple ring every 1.6 s.
fn dot(meter: &Meter) -> El {
    let ring = 20.0;
    let core = 8.0;
    el().absolute(0.0, 0.0).size(CHIP_H, CHIP_H)
        .child(
            el().absolute((CHIP_H - ring) * 0.5, (CHIP_H - ring) * 0.5).size(ring, ring).radius(ring * 0.5)
                .fill(over_shell(red(), 0.5))
                .live_scale(&meter.ring_scale)
                .live_opacity(&meter.ring_alpha),
        )
        .child(
            el().absolute((CHIP_H - core) * 0.5, (CHIP_H - core) * 0.5).size(core, core).radius(core * 0.5)
                .fill(red())
                .shadow(0.0, 7.0, red().alpha(0.55))
                .live_opacity(&meter.core),
        )
}

/// Eight dots around a 6.2 pt circle with a bright head going round.
fn spinner(meter: &Meter) -> El {
    let mut box_ = el().absolute(0.0, 0.0).size(CHIP_H, CHIP_H);
    let d = 2.7;
    for (k, live) in meter.spin.iter().enumerate() {
        let ang = k as f32 * std::f32::consts::FRAC_PI_4 - std::f32::consts::FRAC_PI_2;
        let (cx, cy) = (CHIP_H * 0.5 + 6.2 * ang.cos(), CHIP_H * 0.5 + 6.2 * ang.sin());
        box_ = box_.child(
            el().absolute(cx - d * 0.5, cy - d * 0.5).size(d, d).radius(d * 0.5).fill(text_color())
                .live_opacity(live),
        );
    }
    box_
}

/// A rounded rectangle on the icon grid.
fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    let k = 0.552_284_8 * r;
    let (x1, y1) = (x + w, y + h);
    Path::new()
        .move_to([x + r, y])
        .line_to([x1 - r, y])
        .cubic_to([x1 - r + k, y], [x1, y + r - k], [x1, y + r])
        .line_to([x1, y1 - r])
        .cubic_to([x1, y1 - r + k], [x1 - r + k, y1], [x1 - r, y1])
        .line_to([x + r, y1])
        .cubic_to([x + r - k, y1], [x, y1 - r + k], [x, y1 - r])
        .line_to([x, y + r])
        .cubic_to([x, y + r - k], [x + r - k, y], [x + r, y])
        .close()
}

/// Scrawl's line icons on their 24 × 24 grid (`PillIcon`).
fn icon(id: ElemId, width: f32, color: Color) -> Shape {
    let mut paths: Vec<Path> = Vec::new();
    let line = |a: [f32; 2], b: [f32; 2]| Path::new().move_to(a).line_to(b);
    match id {
        ElemId::ChipArea => {
            for (a, c, b) in [
                ([3.0, 8.0], [3.0, 3.0], [8.0, 3.0]),
                ([16.0, 3.0], [21.0, 3.0], [21.0, 8.0]),
                ([21.0, 16.0], [21.0, 21.0], [16.0, 21.0]),
                ([8.0, 21.0], [3.0, 21.0], [3.0, 16.0]),
            ] {
                paths.push(Path::new().move_to(a).line_to(c).line_to(b));
            }
        }
        ElemId::ChipWindow => {
            paths.push(rounded(2.5, 4.0, 19.0, 16.0, 2.5));
            paths.push(line([2.5, 9.5], [21.5, 9.5]));
        }
        ElemId::ChipScreen => {
            paths.push(rounded(2.0, 3.0, 20.0, 14.0, 2.0));
            paths.push(line([8.0, 21.0], [16.0, 21.0]));
            paths.push(line([12.0, 17.0], [12.0, 21.0]));
        }
        ElemId::ChipIphone => {
            paths.push(rounded(5.5, 2.0, 13.0, 20.0, 3.0));
            paths.push(line([10.5, 6.0], [13.5, 6.0]));
        }
        ElemId::Close => {
            paths.push(line([6.5, 6.5], [17.5, 17.5]));
            paths.push(line([17.5, 6.5], [6.5, 17.5]));
        }
        ElemId::DoneLead => paths.push(Path::new().move_to([5.5, 12.5]).line_to([10.0, 17.0]).line_to([18.5, 7.5])),
        _ => {}
    }
    paths
        .iter()
        .fold(Shape::new().view_box(24.0, 24.0), |shape, path| shape.stroke_path(path, width, LineCap::Round, color))
}

// ─── Motion trace ────────────────────────────────────────────────────────────────────────────

/// Draws Rune's frame, then writes it to the motion-lint trace with the Metal capsule's names
/// (`shell`, `lead`, `chip:Area`, … in screen points), so the two traces compare.
struct TraceFrames {
    origin: [f32; 2],
    /// The last frame's stamp and the wall time it was written (ms on the trace's clock).
    last: Option<(f64, f64)>,
}

impl TraceFrames {
    fn new(origin: [f32; 2]) -> Self {
        Self { origin, last: None }
    }

    /// Rune samples a frame at its display link's target time and steps by `dt` between targets,
    /// so back-to-back frames are stamped `dt` apart; after a pause or drift, the wall clock.
    fn stamp(&mut self, wall_ms: f64, dt: f64) -> f64 {
        let t = match self.last {
            Some((t, wall)) if wall_ms - wall < 50.0 && dt > 0.0 && (t + dt * 1000.0 - wall_ms).abs() < 40.0 => {
                t + dt * 1000.0
            }
            _ => wall_ms,
        };
        self.last = Some((t, wall_ms));
        t
    }
}

impl Compositor<Lab> for TraceFrames {
    fn render(&mut self, f: &mut Frame<'_, Lab>) {
        f.draw_ui();
        if !motion::tracing() || f.rt.state.backdrop.is_some() {
            return;
        }
        let [ox, oy] = self.origin;
        let at = |r: [f32; 4]| [ox + r[0], oy + r[1], r[2], r[3]];
        let dt = f.dt;
        motion::trace_frame_at(|wall_ms| self.stamp(wall_ms, dt));
        let elements = f.rt.elements();
        motion::trace_element("window", [ox, oy, PANEL_W, PANEL_H], 0.0, 0.0, None, None);
        let Some(shell) = elements.iter().find(|e| e.key.as_deref() == Some("shell")) else { return };
        let shell_rect = at(shell.rect);
        motion::trace_element("shell", shell_rect, shell.opacity, shell.rect[3] * 0.5, None, None);
        // The blur anyone sees: an item fading out keeps its full blur until it unmounts, at
        // opacity 0, so weight each by how visible it is.
        let blur = elements
            .iter()
            .filter(|e| e.key.as_deref().is_some_and(|k| k.contains('#')))
            .map(|e| e.blur * e.opacity.clamp(0.0, 1.0));
        motion::trace_channel("content.blur", "level", blur.fold(0.0, f32::max));
        motion::trace_channel("content.alpha", "level", shell.opacity);
        // Entry lands its pose while invisible: a width jump there isn't motion anyone sees.
        if shell.opacity > 0.001 {
            motion::trace_channel("shell.width", "position", shell.rect[2]);
        }
        // Newest generation first, so an element and its exiting predecessor don't share a name.
        let mut items: Vec<_> = elements
            .iter()
            .filter_map(|e| {
                let (name, generation) = e.key.as_deref()?.split_once('#')?;
                (name != "highlight").then(|| (name, generation.parse::<u64>().unwrap_or(0), e))
            })
            .collect();
        items.sort_by(|a, b| b.1.cmp(&a.1));
        let mut seen: Vec<&str> = Vec::new();
        for (name, _, e) in items {
            let key = if seen.contains(&name) { format!("{name}.old") } else { name.to_owned() };
            seen.push(name);
            motion::trace_element(key, at(e.rect), e.opacity, 0.0, Some("row"), Some(shell_rect));
        }
    }
}

// ─── Running ───────────────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────────────────

fn spec(size: (f32, f32), fonts: &'static [&'static [u8]]) -> AppSpec<Lab> {
    AppSpec {
        title: "Scrawl capsule (Rune)",
        size,
        min_size: Some((0.0, 0.0)),
        init,
        view,
        scenes: &[],
        fonts,
        families: families(fonts),
        setup: None,
        poll: Some(poll),
        on_key: None,
    }
}

/// SF (the capsule's face), read from the system once; Rune's Inter when it's missing. The
/// capsule measures its labels with these, so load them before the first `new_surface`, and give
/// them and `families` to the app that draws it.
pub(crate) fn fonts() -> &'static [&'static [u8]] {
    FONTS.get_or_init(|| match std::fs::read("/System/Library/Fonts/SFNS.ttf") {
        Ok(bytes) => Box::leak(vec![&*Box::leak(bytes.into_boxed_slice())].into_boxed_slice()),
        Err(_) => &[],
    })
}

/// `--snap DIR`: one headless run per state, each `--snap DIR/<state>.png --state <state>`.
fn snap_all(options: &Options, dir: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let exe = std::env::current_exe()?;
    for state in STATES {
        let out = dir.join(format!("{state}.png"));
        let status = std::process::Command::new(&exe)
            .args(["lab", "pill", options.scene.name(), "--rune", "--state", state, "--snap"])
            .arg(&out)
            .env_remove("SCRAWL_MOTION_TRACE")
            .status()?;
        anyhow::ensure!(status.success(), "snap {state} failed ({status})");
        println!("{}", out.display());
    }
    Ok(())
}

pub fn run(options: Options) -> anyhow::Result<()> {
    if options.scene == Scene::Grid {
        return grid::run(options.snap);
    }
    if let Some(snap) = &options.snap
        && snap.extension().is_none_or(|e| e != "png")
    {
        return snap_all(&options, snap);
    }
    let fonts = fonts();
    let still = match options.snap.as_ref().map(|_| options.state.as_deref().unwrap_or("picker")) {
        Some(name) => Some(state_pres(name).ok_or_else(|| {
            anyhow::anyhow!("unknown state {name}; one of {}", STATES.join(", "))
        })?),
        None => None,
    };
    let Options { scene, repeat, backdrop, capture, snap, .. } = options;
    if let Some((pres, clock)) = still {
        // Headless (Rune draws no overlays there): one window with the state at rest. Rune gets
        // only its own flag, never Scrawl's.
        let args = ["--snap".to_owned(), snap.unwrap_or_default().to_string_lossy().into_owned()];
        rune::run_windows_with_args(RunOptions { accessory: true }, args, move |_| {
            let mut lab = Lab::capsule(scene, repeat, false);
            lab.still = true;
            lab.set_clock(clock, 0.0);
            let surface = std::mem::replace(&mut lab.surface, Surface::new(motion::tune(), Default::default()));
            lab.surface = surface.settled(pres);
            let app = App::new(spec((PANEL_W, PANEL_H), fonts));
            vec![WindowSpec::from_app("capsule", app).state(lab)]
        });
        return Ok(());
    }
    rune::run_windows_with_args(RunOptions { accessory: true }, Vec::new(), move |_| {
        let places = places();
        let start = crate::screens::pointer()
            .and_then(|p| crate::screens::index_at(&crate::screens::current(), p))
            .unwrap_or(0);
        let Some(here) = places.get(start).cloned() else {
            eprintln!("rune lab: no display");
            std::process::exit(1);
        };
        let exclude = Exclude::from_env();
        let frame = [here.frame[2] as f32, here.frame[3] as f32];
        motion::trace_open(&format!("pill.{}", scene.name()), frame);
        // For screen recordings (`perf/pill/record.sh`): the backdrop's rect in global points.
        println!(
            "region {},{},{},{}",
            here.frame[0] as f32 + here.backdrop[0],
            here.frame[1] as f32 + here.backdrop[1],
            BACKDROP[2],
            BACKDROP[3]
        );

        // Only the capsule polls (`ConfiguredOverlay::poll`): overlays whose state type matches
        // inherit the app's poll, and the backdrop is a `Lab` too.
        let mut app = App::new(AppSpec { poll: None, ..spec((PANEL_W, PANEL_H), fonts) }).windowless();
        let backdrop = backdrop.then(|| {
            let mut state = Lab::capsule(scene, repeat, false);
            state.backdrop = Some(backdrop_fill());
            let overlay = app.overlay(
                OverlaySpec::new("Scrawl capsule backdrop (Rune)", (BACKDROP[2], BACKDROP[3]), view).options(
                    OverlayOptions { level: OverlayLevel::Status, input: OverlayInput::None, ..Default::default() },
                ),
                state,
            );
            place(&overlay, &here, here.backdrop, [BACKDROP[2], BACKDROP[3]]);
            overlay.show();
            overlay
        });

        let lab = Lab::capsule(scene, repeat, true);
        let ctl = lab.ctl.clone();
        let excluded: Arc<std::sync::Mutex<Vec<u32>>> = Arc::default();
        let capture = capture.map(|dir| Arc::new(Capture::new(dir, excluded.clone())));
        let created = capture.clone();
        let origin = [here.frame[0] as f32 + here.pill[0], here.frame[1] as f32 + here.pill[1]];
        let pill = app.overlay(
            OverlaySpec::new("Scrawl capsule (Rune)", (PANEL_W, PANEL_H), view)
                .options(OverlayOptions {
                    // Above the picker (`AboveStatus`) and the backdrop.
                    level: OverlayLevel::AboveOverlays,
                    input: OverlayInput::Regions,
                    exclude_from_capture: exclude.native,
                    prewarm: true,
                    ..Default::default()
                })
                .poll(poll)
                .compositor(TraceFrames::new(origin))
                .on_created(move |made| {
                    eprintln!(
                        "rune lab: capsule overlay window {:?}, native capture exclusion {:?}",
                        made.window_number, made.capabilities.capture_exclusion
                    );
                    if let Some(id) = made.window_number
                        && exclude.filter
                    {
                        excluded.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(id);
                    }
                    if let Some(capture) = created {
                        std::thread::spawn(move || capture.start());
                    }
                }),
            lab,
        );
        place(&pill, &here, here.pill, [PANEL_W, PANEL_H]);
        let _ = ctl.pill.set(pill.clone());
        let quit = app.handle();
        std::thread::Builder::new()
            .name("rune-pill-driver".into())
            .spawn(move || drive(Driver { pill, backdrop, ctl, places, at: start, capture }, quit))
            .expect("driver thread");
        vec![WindowSpec::from_app("lab", app)]
    });
    Ok(())
}

/// `SCRAWL_LAB_BACKDROP`: grey (default), light or dark.
fn backdrop_fill() -> Color {
    let theme = std::env::var("SCRAWL_LAB_BACKDROP").unwrap_or_default().to_ascii_lowercase();
    match theme.trim() {
        "dark" | "busy-dark" => srgb(0.059, 0.067, 0.090, 1.0),
        "light" | "white" => srgb(0.980, 0.980, 0.988, 1.0),
        _ => srgb(0.851, 0.851, 0.871, 1.0),
    }
}

/// How the capsule stays out of screen captures (`SCRAWL_LAB_EXCLUDE`): `both` (the default, as
/// the app does), `filter` (Scrawl's ScreenCaptureKit filter only), `native` (the panel's
/// `sharingType` only) or `none` (so another recorder can film it). Scrawl's own capture
/// (`--capture`) leaves out every window of this process above layer 0 anyway, the overlays
/// included, so the modes only change what other processes' captures see.
#[derive(Clone, Copy)]
struct Exclude {
    native: bool,
    filter: bool,
}

impl Exclude {
    fn from_env() -> Self {
        match std::env::var("SCRAWL_LAB_EXCLUDE").unwrap_or_default().trim() {
            "none" => Self { native: false, filter: false },
            "native" => Self { native: true, filter: false },
            "filter" => Self { native: false, filter: true },
            _ => Self { native: true, filter: true },
        }
    }
}

/// Where the capsule and the backdrop go on one display, in its local points (what
/// `OverlayPlacement::Rect` takes).
#[derive(Clone, Debug)]
struct Place {
    display: u32,
    /// Global points, to find the pointer's display.
    frame: [f64; 4],
    pill: [f32; 2],
    backdrop: [f32; 2],
}

/// Every display's places, from `screens::pill_origin` (below the menu bar or notch, centred).
/// Main thread.
fn places() -> Vec<Place> {
    crate::screens::current()
        .iter()
        .map(|s| {
            let [x, y] = crate::screens::pill_origin(s, f64::from(PANEL_W), f64::from(PANEL_Y));
            let f = s.frame;
            Place {
                display: s.id,
                frame: f,
                pill: [(x - f[0]) as f32, (y - f[1]) as f32],
                backdrop: [((f[2] as f32 - BACKDROP[2]) * 0.5).round(), BACKDROP[1]],
            }
        })
        .collect()
}

fn place<T>(overlay: &OverlayHandle<T>, place: &Place, at: [f32; 2], size: [f32; 2]) {
    overlay.set_display(DisplayTarget::Native(NativeDisplayId::MacOS(place.display)));
    overlay.set_placement(Some(OverlayPlacement::Rect { x: at[0], y: at[1], width: size[0], height: size[1] }));
}

/// `--capture DIR`: a real Screen capture of the capsule's display while the scene plays, with
/// the capsule's window in Scrawl's ScreenCaptureKit exclusion list (as the app excludes the
/// Metal panel), finished into a bundle.
struct Capture {
    dir: PathBuf,
    excluded: Arc<std::sync::Mutex<Vec<u32>>>,
    session: std::sync::Mutex<Option<(crate::model::TargetInfo, crate::capture::ScreenCaptureSession)>>,
    /// The capture has started (or failed to): the scene may begin.
    started: AtomicBool,
}

impl Capture {
    fn new(dir: PathBuf, excluded: Arc<std::sync::Mutex<Vec<u32>>>) -> Self {
        Self { dir, excluded, session: std::sync::Mutex::default(), started: AtomicBool::new(false) }
    }

    fn start(&self) {
        if let Err(error) = self.begin() {
            eprintln!("rune lab: capture didn't start: {error:#}");
        }
        self.started.store(true, Ordering::Release);
    }

    fn begin(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let target = crate::capture::discover_target(None);
        let session = crate::capture::ScreenCaptureSession::start(
            target.clone(),
            Some(self.dir.join("recording.mp4")),
            self.excluded.clone(),
        );
        let excluded = self.excluded.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        eprintln!("rune lab: capturing, excluding windows {excluded:?}");
        *self.session.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((target, session));
        Ok(())
    }

    /// Stops the capture and builds its bundle.
    fn finish(&self) -> anyhow::Result<()> {
        let Some((target, session)) = self.session.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
        else {
            anyhow::bail!("the capture never started");
        };
        let excluded = self.excluded.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        crate::finalize_capture_bundle(
            &self.dir,
            target,
            session.stop(),
            Vec::new(),
            Vec::new(),
            None,
            Some(format!("Rune capsule capture-exclusion check; excluded windows {excluded:?}")),
            crate::windows::ActivityLog::default(),
        )?;
        println!("bundle {}", self.dir.display());
        Ok(())
    }
}

/// Does what `poll` can't from inside the overlay: starts it once any capture is running, moves
/// it to the pointer's display while the picker is open, and quits (an overlay's `Tx::quit`
/// doesn't reach the app).
struct Driver {
    pill: OverlayHandle<Lab>,
    backdrop: Option<OverlayHandle<Lab>>,
    ctl: Arc<Ctl>,
    places: Vec<Place>,
    at: usize,
    capture: Option<Arc<Capture>>,
}

fn drive(mut d: Driver, quit: rune::AppHandle<Lab>) {
    let _ = d.ctl.driver.set(std::thread::current());
    // A capture records the whole scene: it starts first.
    let waiting = Instant::now();
    while d.capture.as_ref().is_some_and(|c| !c.started.load(Ordering::Acquire))
        && waiting.elapsed() < Duration::from_secs(10)
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    // The hidden capsule's next poll starts the timeline.
    d.ctl.go.store(true, Ordering::Release);
    loop {
        if d.ctl.done.load(Ordering::Acquire) {
            break;
        }
        if d.ctl.follow.load(Ordering::Acquire)
            && let Some(p) = crate::screens::pointer()
            && let Some(i) = d.places.iter().position(|pl| crate::screens::contains(pl.frame, p))
            && i != d.at
        {
            d.at = i;
            let here = &d.places[i];
            place(&d.pill, here, here.pill, [PANEL_W, PANEL_H]);
            if let Some(backdrop) = &d.backdrop {
                place(backdrop, here, here.backdrop, [BACKDROP[2], BACKDROP[3]]);
            }
        }
        std::thread::park_timeout(Duration::from_millis(50));
    }
    if let Some(capture) = &d.capture
        && let Err(error) = capture.finish()
    {
        eprintln!("rune lab: capture failed: {error:#}");
    }
    quit.quit();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timer_keeps_its_width_as_the_seconds_change() {
        fonts();
        let widths: Vec<f32> = ["0:00", "0:11", "1:11", "1:47"].map(clock_width).into();
        assert!(widths.iter().all(|w| (w - widths[0]).abs() < 0.01), "{widths:?}");
        // Without `tnum` SF's ones are narrower, so the check above means something.
        assert!(measure("1:11", true) < measure("0:00", true) - 1.0);
    }

    #[test]
    fn the_shimmer_has_a_glyph_per_character() {
        assert_eq!(PROCESSING.chars().count(), PROCESSING_GLYPHS);
    }

    #[test]
    fn presses_ask_for_what_the_metal_capsule_sends() {
        let picker = Pres::Selecting { mode: SelectionMode::Screen, has_iphone: false };
        let recording = Pres::Recording { chips: true, has_iphone: false };
        let done = Pres::Completed { label: "Copied".into(), routes: Vec::new() };
        assert!(matches!(action(&picker, ElemId::ChipArea), Some(PillAction::SelectMode(SelectionMode::Area))));
        assert!(matches!(
            action(&recording, ElemId::ChipWindow),
            Some(PillAction::SwitchRecording(SelectionMode::Window))
        ));
        assert!(matches!(action(&recording, ElemId::Close), Some(PillAction::Cancel)));
        assert!(matches!(action(&done, ElemId::Close), Some(PillAction::DismissCompleted)));
        assert!(matches!(action(&done, ElemId::Route1), Some(PillAction::Route(1))));
        assert!(action(&recording, ElemId::Lead).is_none());
    }
}
