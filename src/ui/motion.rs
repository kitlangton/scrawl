//! The capsule's motion plumbing: one animation clock that can run slow, springs and durations
//! that can be tuned live from `~/.config/scrawl/tune.json` (debug builds), and a per-frame trace
//! in motion-lint's format (`SCRAWL_MOTION_TRACE=<file>`). Each costs nothing when it's off.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use super::Spring;

// ---------------------------------------------------------------------------------------------
// Clock

struct Clock {
    anchor: Instant,
    /// Animation seconds at `anchor`.
    base: f64,
    scale: f64,
}

thread_local! {
    static CLOCK: RefCell<Clock> = RefCell::new(Clock { anchor: Instant::now(), base: 0.0, scale: 1.0 });
}

/// Animation time in seconds. Every capsule age reads this, so a time scale slows them all.
pub(crate) fn now() -> f64 {
    CLOCK.with_borrow(|c| c.base + c.anchor.elapsed().as_secs_f64() * c.scale)
}

/// Animation seconds since `since` (a `now()` reading).
pub(crate) fn age(since: f64) -> f32 {
    (now() - since).max(0.0) as f32
}

pub(crate) fn time_scale() -> f32 {
    CLOCK.with_borrow(|c| c.scale as f32)
}

/// Re-anchors so ages already running continue from where they are.
fn set_time_scale(scale: f64) {
    CLOCK.with_borrow_mut(|c| {
        let scale = scale.clamp(0.02, 4.0);
        if (scale - c.scale).abs() < 1e-9 {
            return;
        }
        c.base += c.anchor.elapsed().as_secs_f64() * c.scale;
        c.anchor = Instant::now();
        c.scale = scale;
    });
}

// ---------------------------------------------------------------------------------------------
// Tuning

/// Every value Kit judges by eye. `DEFAULT` is what ships; `tune.json` overrides it live.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tune {
    /// Width spring for a phase change (picker, recording, processing, done).
    pub island: Spring,
    /// Open/close scale and drop of the whole capsule.
    pub island_scale: Spring,
    /// Width spring as the switch chips open on a tap's recording.
    pub reveal_open: Spring,
    /// Width spring as they fold back into the recording pill.
    pub reveal_close: Spring,
    /// Width spring when a switch chip turns the recording into the picker.
    pub switch_width: Spring,
    /// The chips' slide to their place in the picker. Slightly slower than the width, which leads.
    pub switch_slide: Spring,
    /// Chips ↔ controls swap in place: incoming fade, outgoing fade, peak blur.
    pub reveal_enter_s: f32,
    pub reveal_exit_s: f32,
    pub reveal_blur_px: f32,
    pub time_scale: f32,
    /// Capsule style override (`-1.0` = use config, `0.0` = black, `1.0` = glass).
    pub pill_glass: f32,
}

impl Tune {
    pub const DEFAULT: Tune = Tune {
        island: Spring::visual(0.32, 0.0),
        island_scale: Spring::visual(0.22, 0.0),
        reveal_open: Spring::visual(0.36, 0.16),
        reveal_close: Spring::visual(0.32, 0.08),
        switch_width: Spring::visual(0.30, 0.0),
        switch_slide: Spring::visual(0.36, 0.0),
        reveal_enter_s: 0.16,
        reveal_exit_s: 0.13,
        reveal_blur_px: 2.5,
        time_scale: 1.0,
        pill_glass: -1.0,
    };
}

type Field = fn(&mut Tune) -> &mut f32;

/// `tune.json` keys, in the order `scrawl tune list` prints them.
pub(super) const KEYS: &[(&str, &str, Field)] = &[
    ("island.duration", "Phase morph width spring (s)", |t| {
        &mut t.island.response
    }),
    ("island.bounce", "Phase morph width bounce", |t| {
        &mut t.island.bounce
    }),
    ("scale.duration", "Open/close scale spring (s)", |t| {
        &mut t.island_scale.response
    }),
    ("scale.bounce", "Open/close scale bounce", |t| {
        &mut t.island_scale.bounce
    }),
    ("reveal.open.duration", "Chips open width spring (s)", |t| {
        &mut t.reveal_open.response
    }),
    ("reveal.open.bounce", "Chips open bounce", |t| {
        &mut t.reveal_open.bounce
    }),
    (
        "reveal.close.duration",
        "Chips fold width spring (s)",
        |t| &mut t.reveal_close.response,
    ),
    ("reveal.close.bounce", "Chips fold bounce", |t| {
        &mut t.reveal_close.bounce
    }),
    (
        "switch.width.duration",
        "Chip → picker width spring (s)",
        |t| &mut t.switch_width.response,
    ),
    ("switch.width.bounce", "Chip → picker width bounce", |t| {
        &mut t.switch_width.bounce
    }),
    (
        "switch.slide.duration",
        "Chip → picker slide spring (s)",
        |t| &mut t.switch_slide.response,
    ),
    ("switch.slide.bounce", "Chip → picker slide bounce", |t| {
        &mut t.switch_slide.bounce
    }),
    ("reveal.enter", "Chips ↔ controls fade in (s)", |t| {
        &mut t.reveal_enter_s
    }),
    ("reveal.exit", "Chips ↔ controls fade out (s)", |t| {
        &mut t.reveal_exit_s
    }),
    ("reveal.blur", "Chips ↔ controls peak blur (px)", |t| {
        &mut t.reveal_blur_px
    }),
    (
        "timeScale",
        "Animation time scale (0.25 = slow motion)",
        |t| &mut t.time_scale,
    ),
    (
        "pill.glass",
        "Capsule style (-1 = config, 0 = black, 1 = glass)",
        |t| &mut t.pill_glass,
    ),
];

/// Numbered presets; 1 is what ships and names nothing.
pub(super) const PRESETS: &[(&str, &[(&str, f32)])] = &[
    ("Today", &[]),
    (
        "Before",
        &[
            ("island.duration", 0.28),
            ("island.bounce", 0.18),
            ("scale.bounce", 0.14),
            ("switch.width.duration", 0.28),
            ("switch.width.bounce", 0.18),
            ("switch.slide.duration", 0.28),
            ("switch.slide.bounce", 0.18),
        ],
    ),
    (
        "Calm",
        &[
            ("island.duration", 0.4),
            ("switch.width.duration", 0.38),
            ("switch.slide.duration", 0.45),
            ("reveal.open.duration", 0.42),
            ("reveal.close.duration", 0.38),
        ],
    ),
    (
        "Brisk",
        &[
            ("island.duration", 0.25),
            ("switch.width.duration", 0.22),
            ("switch.slide.duration", 0.28),
            ("reveal.open.duration", 0.3),
            ("reveal.close.duration", 0.26),
        ],
    ),
    (
        "Shape",
        &[
            ("switch.width.duration", 0.42),
            ("switch.width.bounce", 0.16),
            ("switch.slide.duration", 0.42),
            ("switch.slide.bounce", 0.08),
        ],
    ),
    ("Glass", &[("pill.glass", 1.0)]),
];

impl Tune {
    pub fn pill_style_override(&self) -> Option<crate::native::pill::PillStyle> {
        if self.pill_glass >= 0.5 {
            Some(crate::native::pill::PillStyle::Glass)
        } else if self.pill_glass >= -0.5 {
            Some(crate::native::pill::PillStyle::Black)
        } else {
            None
        }
    }

    pub fn get(&self, key: &str) -> Option<f32> {
        let mut copy = *self;
        KEYS.iter()
            .find(|(k, ..)| *k == key)
            .map(|(_, _, f)| *f(&mut copy))
    }

    fn set(&mut self, key: &str, value: f32) -> bool {
        match KEYS.iter().find(|(k, ..)| *k == key) {
            Some((_, _, f)) => {
                *f(self) = value;
                true
            }
            None => false,
        }
    }

    /// `DEFAULT`, then the preset the file names (1-based), then the file's own keys.
    pub(super) fn from_json(value: &serde_json::Value) -> Tune {
        let mut tune = Tune::DEFAULT;
        let preset = value.get("preset").and_then(serde_json::Value::as_u64);
        if let Some((_, values)) = preset.and_then(|n| PRESETS.get((n as usize).checked_sub(1)?)) {
            for (key, v) in *values {
                tune.set(key, *v);
            }
        }
        if let Some(map) = value.as_object() {
            for (key, v) in map {
                if key == "pillStyle"
                    && let Some(s) = v.as_str()
                    && let Some(style) = crate::native::pill::PillStyle::from_str_opt(s)
                {
                    tune.pill_glass = match style {
                        crate::native::pill::PillStyle::Black => 0.0,
                        crate::native::pill::PillStyle::Glass => 1.0,
                    };
                } else if let Some(v) = v.as_f64() {
                    tune.set(key, v as f32);
                }
            }
        }
        tune.sanitize();
        tune
    }

    fn sanitize(&mut self) {
        for spring in [
            &mut self.island,
            &mut self.island_scale,
            &mut self.reveal_open,
            &mut self.reveal_close,
            &mut self.switch_width,
            &mut self.switch_slide,
        ] {
            spring.response = spring.response.clamp(0.04, 3.0);
            spring.bounce = spring.bounce.clamp(0.0, 0.9);
        }
        self.reveal_enter_s = self.reveal_enter_s.clamp(0.0, 2.0);
        self.reveal_exit_s = self.reveal_exit_s.clamp(0.0, 2.0);
        self.reveal_blur_px = self.reveal_blur_px.clamp(0.0, 8.0);
        self.time_scale = self.time_scale.clamp(0.02, 4.0);
        self.pill_glass = self.pill_glass.clamp(-1.0, 1.0);
    }
}

pub(super) fn tune_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SCRAWL_TUNE") {
        return Some(PathBuf::from(path));
    }
    dirs::home_dir().map(|home| home.join(".config/scrawl/tune.json"))
}

struct TuneState {
    tune: Tune,
    checked: Option<Instant>,
    stamp: Option<std::time::SystemTime>,
}

thread_local! {
    static TUNE: RefCell<TuneState> = const {
        RefCell::new(TuneState { tune: Tune::DEFAULT, checked: None, stamp: None })
    };
    /// Cached copy for the hot path.
    static CURRENT: Cell<Tune> = const { Cell::new(Tune::DEFAULT) };
}

/// The live values. One thread-local read; `DEFAULT` unless a tune file is loaded.
pub(crate) fn tune() -> Tune {
    CURRENT.get()
}

/// Re-reads `tune.json` if it changed (checked at most four times a second). Debug builds only,
/// so a release never picks up a stray file.
pub(crate) fn poll_tune() {
    if !cfg!(debug_assertions) {
        return;
    }
    TUNE.with_borrow_mut(|state| {
        let first = state.checked.is_none();
        if state
            .checked
            .is_some_and(|at| at.elapsed() < Duration::from_millis(250))
        {
            return;
        }
        state.checked = Some(Instant::now());
        let Some(path) = tune_path() else { return };
        let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if stamp == state.stamp && !first {
            return;
        }
        state.stamp = stamp;
        let mut tune = stamp
            .and_then(|_| std::fs::read_to_string(&path).ok())
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .map_or(Tune::DEFAULT, |value| Tune::from_json(&value));
        if let Some(scale) = std::env::var("SCRAWL_TIME_SCALE")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
        {
            tune.time_scale = scale.clamp(0.02, 4.0);
        }
        if tune != state.tune {
            crate::capture::scrawl_log(format!(
                "tune: {} ({})",
                if stamp.is_some() {
                    "loaded"
                } else {
                    "defaults"
                },
                path.display()
            ));
        }
        state.tune = tune;
        CURRENT.set(tune);
        set_time_scale(f64::from(tune.time_scale));
    });
}

/// `scrawl tune …`: edits `tune.json`, which debug builds (the lab, the installed app) re-read
/// within a quarter second.
pub(crate) enum TuneAction {
    List,
    Set(String, f32),
    Preset(usize),
    Reset,
    Rust,
}

pub(crate) fn tune_command(action: TuneAction) -> anyhow::Result<()> {
    let path = tune_path().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    let read = || -> serde_json::Map<String, serde_json::Value> {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    };
    let write = |map: &serde_json::Map<String, serde_json::Value>| -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(map)? + "\n")?;
        Ok(())
    };
    match action {
        TuneAction::List => {
            let file = read();
            let tune = Tune::from_json(&serde_json::Value::Object(file.clone()));
            let preset = file.get("preset").and_then(serde_json::Value::as_u64);
            println!("{}", path.display());
            for (i, (name, _)) in PRESETS.iter().enumerate() {
                let mark = if preset == Some(i as u64 + 1) || (preset.is_none() && i == 0) {
                    "●"
                } else {
                    " "
                };
                print!("{mark} {} {name}   ", i + 1);
            }
            println!();
            for (key, label, _) in KEYS {
                let now = tune.get(key).unwrap_or_default();
                let default = Tune::DEFAULT.get(key).unwrap_or_default();
                let changed = if (now - default).abs() > 1e-6 {
                    "*"
                } else {
                    " "
                };
                println!("{changed} {key:<24} {now:<8} (default {default})  {label}");
            }
        }
        TuneAction::Set(key, value) => {
            anyhow::ensure!(
                Tune::DEFAULT.get(&key).is_some(),
                "unknown key {key}; `scrawl tune list` shows them"
            );
            let mut map = read();
            map.insert(key.clone(), serde_json::json!(value));
            write(&map)?;
            let tune = Tune::from_json(&serde_json::Value::Object(map));
            println!("{key} = {}", tune.get(&key).unwrap_or_default());
        }
        TuneAction::Preset(n) => {
            let (name, _) = PRESETS
                .get(n.wrapping_sub(1))
                .ok_or_else(|| anyhow::anyhow!("presets are 1–{}", PRESETS.len()))?;
            // A preset replaces individual keys, as picking one does on the phone tune page.
            let mut map = serde_json::Map::new();
            if let Some(scale) = read().get("timeScale") {
                map.insert("timeScale".into(), scale.clone());
            }
            map.insert("preset".into(), n.into());
            write(&map)?;
            println!("{n} {name}");
        }
        TuneAction::Reset => match std::fs::remove_file(&path) {
            Ok(()) => println!("removed {}", path.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => println!("nothing to reset"),
            Err(e) => return Err(e.into()),
        },
        TuneAction::Rust => {
            let t = Tune::from_json(&serde_json::Value::Object(read()));
            let spring = |s: Spring| format!("Spring::visual({:?}, {:?})", s.response, s.bounce);
            println!("// Paste into `Tune::DEFAULT` in src/ui/motion.rs.");
            println!("island: {},", spring(t.island));
            println!("island_scale: {},", spring(t.island_scale));
            println!("reveal_open: {},", spring(t.reveal_open));
            println!("reveal_close: {},", spring(t.reveal_close));
            println!("switch_width: {},", spring(t.switch_width));
            println!("switch_slide: {},", spring(t.switch_slide));
            println!("reveal_enter_s: {:?},", t.reveal_enter_s);
            println!("reveal_exit_s: {:?},", t.reveal_exit_s);
            println!("reveal_blur_px: {:?},", t.reveal_blur_px);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Trace

/// Maps the capsule window's GPUI coordinates to what's drawn on screen: the content layer's
/// scale about the window centre and its drop, then the window's screen origin. `clip` is the
/// shell (the capsule clips its content), in screen points.
#[derive(Clone, Copy, Debug)]
pub(super) struct Xf {
    pub s: f32,
    pub cx: f32,
    pub cy: f32,
    pub ty: f32,
    pub ox: f32,
    pub oy: f32,
    pub alpha: f32,
    pub clip: [f32; 4],
}

impl Xf {
    pub fn map(&self, [x, y, w, h]: [f32; 4]) -> [f32; 4] {
        [
            self.ox + self.cx + (x - self.cx) * self.s,
            self.oy + self.cy + (y - self.cy) * self.s + self.ty,
            w * self.s,
            h * self.s,
        ]
    }
}

/// A prepaint listener that traces a row's children under `keys` (`None` skips a child), each
/// at its own opacity times the capsule's.
pub(super) fn trace_children(
    keys: Vec<Option<(String, f32)>>,
    xf: Xf,
    parent: &'static str,
) -> impl Fn(Vec<gpui::Bounds<gpui::Pixels>>, &mut gpui::Window, &mut gpui::App) + 'static {
    move |bounds, _, _| {
        for (key, b) in keys.iter().zip(bounds) {
            let Some((key, alpha)) = key else { continue };
            let rect = xf.map([
                f32::from(b.origin.x),
                f32::from(b.origin.y),
                f32::from(b.size.width),
                f32::from(b.size.height),
            ]);
            trace_element(
                key.clone(),
                rect,
                alpha * xf.alpha,
                0.0,
                Some(parent),
                Some(xf.clip),
            );
        }
    }
}

type PendingElement = (
    String,
    [f32; 4],
    f32,
    f32,
    Option<&'static str>,
    Option<[f32; 4]>,
);

#[derive(Default)]
struct Pending {
    t_ms: f64,
    elements: Vec<PendingElement>,
    channels: Vec<(&'static str, &'static str, f32)>,
    events: Vec<String>,
}

struct Trace {
    out: std::io::BufWriter<std::fs::File>,
    start: Instant,
    /// One display frame: a line's `n` counts these, as motion-lint reads them at `hz`.
    frame_ms: f64,
    last_t: Option<f64>,
    ids: HashMap<String, u32>,
    described: HashSet<u32>,
    states: HashMap<u32, String>,
    channel_ids: HashMap<&'static str, u32>,
    channel_values: HashMap<u32, String>,
    pending: Option<Pending>,
    queued_events: Vec<String>,
}

thread_local! {
    static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) };
}

fn trace_target() -> Option<&'static PathBuf> {
    static TARGET: OnceLock<Option<PathBuf>> = OnceLock::new();
    TARGET
        .get_or_init(|| std::env::var_os("SCRAWL_MOTION_TRACE").map(PathBuf::from))
        .as_ref()
}

/// Whether a trace is being written. One `OnceLock` read when off.
pub(crate) fn tracing() -> bool {
    trace_target().is_some()
}

/// The fastest display's refresh rate (120 on ProMotion), so a frame at 120 Hz counts as one
/// and a missed one as two. Main thread; 60 when it can't tell.
fn refresh_hz() -> f64 {
    #[cfg(target_os = "macos")]
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        let fastest = objc2_app_kit::NSScreen::screens(mtm).iter().map(|s| s.maximumFramesPerSecond()).max();
        if let Some(hz) = fastest.filter(|hz| *hz > 0) {
            return hz as f64;
        }
    }
    60.0
}

/// Opens the trace (once): a motion-lint header sized to the main display in points.
pub(crate) fn trace_open(launch: &str, display: [f32; 2]) {
    let Some(path) = trace_target() else { return };
    TRACE.with_borrow_mut(|slot| {
        if slot.is_some() {
            return;
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Ok(file) = std::fs::File::create(path) else {
            crate::capture::scrawl_log(format!("motion trace: can't write {}", path.display()));
            return;
        };
        let mut out = std::io::BufWriter::new(file);
        let hz = refresh_hz();
        let header = serde_json::json!({
            "type": "header", "launch": launch, "width": display[0], "height": display[1],
            "hz": hz, "scale": 2, "wall": crate::now_epoch_ms() as f64 / 1000.0,
            "platform": "gpui", "units": "pt", "clock": "wall",
        });
        let _ = writeln!(out, "{header}");
        *slot = Some(Trace {
            out,
            start: Instant::now(),
            frame_ms: 1000.0 / hz,
            last_t: None,
            ids: HashMap::new(),
            described: HashSet::new(),
            states: HashMap::new(),
            channel_ids: HashMap::new(),
            channel_values: HashMap::new(),
            pending: None,
            queued_events: Vec::new(),
        });
    });
}

/// An app event (a phase change, a chip click), written with the next frame.
pub(crate) fn trace_event(event: impl Into<String>) {
    if !tracing() {
        return;
    }
    TRACE.with_borrow_mut(|slot| {
        if let Some(trace) = slot.as_mut() {
            trace.queued_events.push(event.into());
        }
    });
}

/// Starts a frame: the previous one (now complete, its prepaint bounds included) is written.
/// Ends the frame in progress without starting one: what is drawn until the next
/// `trace_frame` isn't recorded (a new window's first frame, drawn fully transparent).
pub(crate) fn trace_skip_frame() {
    TRACE.with_borrow_mut(|slot| {
        if let Some(trace) = slot.as_mut() {
            trace.flush_pending();
        }
    });
}

pub(crate) fn trace_frame() {
    trace_frame_at(|wall_ms| wall_ms);
}

/// Starts a frame stamped `stamp(wall_ms)`: a renderer whose values are sampled at its display
/// link's target time stamps that, not the moment it finished drawing.
pub(crate) fn trace_frame_at(stamp: impl FnOnce(f64) -> f64) {
    TRACE.with_borrow_mut(|slot| {
        let Some(trace) = slot.as_mut() else { return };
        trace.flush_pending();
        trace.pending = Some(Pending {
            t_ms: stamp(trace.start.elapsed().as_secs_f64() * 1000.0),
            events: std::mem::take(&mut trace.queued_events),
            ..Pending::default()
        });
    });
}

/// An element's box in screen points (top-left origin), drawn opacity and corner radius, and
/// the clip it's drawn inside.
pub(crate) fn trace_element(
    key: impl Into<String>,
    rect: [f32; 4],
    alpha: f32,
    radius: f32,
    parent: Option<&'static str>,
    clip: Option<[f32; 4]>,
) {
    TRACE.with_borrow_mut(|slot| {
        if let Some(pending) = slot.as_mut().and_then(|t| t.pending.as_mut()) {
            pending
                .elements
                .push((key.into(), rect, alpha, radius, parent, clip));
        }
    });
}

/// The part of `rect` inside `clip`, when the clip cuts it.
fn visible(rect: [f32; 4], clip: [f32; 4]) -> Option<[f32; 4]> {
    let x0 = rect[0].max(clip[0]);
    let y0 = rect[1].max(clip[1]);
    let x1 = (rect[0] + rect[2]).min(clip[0] + clip[2]);
    let y1 = (rect[1] + rect[3]).min(clip[1] + clip[3]);
    let vis = [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)];
    let cut = (0..4).any(|i| (vis[i] - rect[i]).abs() > 0.25);
    cut.then_some(vis)
}

/// A scalar per frame: `kind` is motion-lint's `level`, `phase` or `position`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn trace_channel(name: &'static str, kind: &'static str, value: f32) {
    TRACE.with_borrow_mut(|slot| {
        if let Some(pending) = slot.as_mut().and_then(|t| t.pending.as_mut()) {
            pending.channels.push((name, kind, value));
        }
    });
}

/// Writes the last frame and closes the file.
pub(crate) fn trace_close() {
    TRACE.with_borrow_mut(|slot| {
        if let Some(mut trace) = slot.take() {
            trace.flush_pending();
            // An empty closing frame so the last change has a frame after it.
            trace.pending = Some(Pending {
                t_ms: trace.start.elapsed().as_secs_f64() * 1000.0,
                ..Pending::default()
            });
            trace.flush_pending();
            let _ = trace.out.flush();
        }
    });
}

fn r2(v: f32) -> f64 {
    (f64::from(v) * 100.0).round() / 100.0
}

impl Trace {
    fn id(&mut self, key: &str) -> u32 {
        let next = self.ids.len() as u32 + 1;
        *self.ids.entry(key.to_owned()).or_insert(next)
    }

    fn flush_pending(&mut self) {
        let Some(frame) = self.pending.take() else {
            return;
        };
        let n = self.last_t.map_or(1, |last| {
            ((frame.t_ms - last) / self.frame_ms).round().max(1.0) as u64
        });
        self.last_t = Some(frame.t_ms);
        let mut metas = Vec::new();
        let mut changed = Vec::new();
        let mut seen = HashSet::new();
        for (key, rect, a, r, parent, clip) in &frame.elements {
            let [x, y, w, h] = rect;
            let id = self.id(key);
            if !seen.insert(id) {
                continue;
            }
            if self.described.insert(id) {
                let mut meta =
                    serde_json::json!({ "id": id, "cls": "gpui", "nm": key, "host": "pill" });
                if let Some(parent) = parent {
                    meta["pv"] = self.id(parent).into();
                }
                metas.push(meta);
            }
            let mut e = vec![
                f64::from(id),
                r2(*x),
                r2(*y),
                r2(*w),
                r2(*h),
                (f64::from(*a) * 1000.0).round() / 1000.0,
                r2(*r),
                0.0,
            ];
            if let Some([vx, vy, vw, vh]) = clip.and_then(|clip| visible(*rect, clip)) {
                e.extend([r2(vx), r2(vy), r2(vw), r2(vh)]);
            }
            let e = serde_json::json!(e);
            let state = e.to_string();
            if self.states.get(&id) != Some(&state) {
                self.states.insert(id, state);
                changed.push(e);
            }
        }
        let gone: Vec<u32> = self
            .states
            .keys()
            .copied()
            .filter(|id| !seen.contains(id))
            .collect();
        for id in &gone {
            self.states.remove(id);
        }
        let mut channel_metas = Vec::new();
        let mut channel_values = Vec::new();
        for (name, kind, value) in &frame.channels {
            let next = self.channel_ids.len() as u32 + 1;
            let id = *self.channel_ids.entry(name).or_insert(next);
            if id == next {
                channel_metas.push(serde_json::json!({ "id": id, "name": name, "kind": kind }));
            }
            let v = (f64::from(*value) * 1e4).round() / 1e4;
            let text = v.to_string();
            if self.channel_values.get(&id) != Some(&text) {
                self.channel_values.insert(id, text);
                channel_values.push(serde_json::json!([id, v]));
            }
        }
        let mut line = serde_json::json!({ "t": (frame.t_ms * 1000.0).round() / 1000.0, "n": n });
        // In slow motion a display frame is a fraction of an animation frame; motion-lint
        // counts `n × sc`, so a 0.25× run measures like a 1× one.
        let scale = time_scale();
        if (scale - 1.0).abs() > 1e-6 {
            line["sc"] = f64::from(scale).into();
        }
        if !frame.events.is_empty() {
            line["ev"] = frame.events.into();
        }
        if !metas.is_empty() {
            line["m"] = metas.into();
        }
        if !changed.is_empty() {
            line["e"] = changed.into();
        }
        if !gone.is_empty() {
            line["g"] = gone.into();
        }
        if !channel_metas.is_empty() {
            line["cm"] = channel_metas.into();
        }
        if !channel_values.is_empty() {
            line["c"] = channel_values.into();
        }
        let _ = writeln!(self.out, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tune_json_layers_a_preset_and_keys_over_the_defaults() {
        let tune = Tune::from_json(&serde_json::json!({ "preset": 2, "reveal.blur": 3.0 }));
        assert_eq!(tune.island, Spring::visual(0.28, 0.18));
        assert_eq!(tune.reveal_blur_px, 3.0);
        assert_eq!(tune.reveal_open, Tune::DEFAULT.reveal_open);
        // Unknown keys are ignored and values are clamped.
        let tune = Tune::from_json(&serde_json::json!({ "nope": 1, "island.bounce": 5 }));
        assert_eq!(tune.island.bounce, 0.9);
        assert_eq!(Tune::from_json(&serde_json::json!({})), Tune::DEFAULT);
    }

    #[test]
    fn every_preset_names_real_keys() {
        for (name, values) in PRESETS {
            for (key, _) in *values {
                assert!(Tune::DEFAULT.get(key).is_some(), "{name}: {key}");
            }
        }
    }
}
