//! `scrawl lab pill grid --rune`: every capsule state in one ordinary window, one scene at a
//! time. The sidebar lists the scenes by group, the stage shows the selected one large over a
//! dark, light or screenshot backdrop, drawn by the real view (`capsule`) or, for the
//! post-record designs Kit is weighing, by `proto`, and the inspector holds playback, the
//! backdrop and the knobs.
//!
//! A scene starts held at rest; space (or a click on the selected scene) plays its entry,
//! transition or flow on a loop. Springs follow `scrawl tune` (tune.json) and spacing, type and
//! colours follow `pill-lab.json` (`style`): the inspector's sliders write both, and edits to
//! either file show within a quarter second.
//!
//! The lab's own chrome never animates: its presses and keys apply instantly, so only the
//! capsule on the stage moves.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use rune::{
    App, AppSpec, Color, Cx, Drag, El, Key, KeyEvent, Pixels, RunOptions, ScrollSpec, SpringPair, SurfaceFit,
    TextStyle, Transition, Tx, WindowSpec, col, el, row, text,
};

use super::proto::{self, Proto};
use super::style::{self, Style, lab_path, set_style};
use super::{Capsule, Meter, PANEL_H, PANEL_W, Surface, capsule, families, fonts, new_surface, srgb, surface};
use crate::native::pill::model::{ElemId, Pres};
use crate::ui::motion::{self, KEYS, Tune, tune_path};
use crate::ui::{SelectionMode, chip_title, island_title};

const SIDEBAR_W: f32 = 232.0;
const INSPECTOR_W: f32 = 288.0;
/// The window's title bar, drawn over the content.
const TITLE_BAR_H: f32 = 38.0;
/// A headless still's or film's window.
const SNAP_SIZE: (f32, f32) = (1600.0, 900.0);
/// The stage's largest zoom.
const MAX_ZOOM: f32 = 2.0;

/// A looping scene: hidden for `LEAD_S`, then its steps, then hidden again for `TAIL_S` (its
/// exit included), so it's gone for well under half a second a loop.
const LEAD_S: f64 = 0.15;
const HOLD_S: f64 = 2.2;
const TAIL_S: f64 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    AfterRecording,
    Earlier,
    States,
    Delivery,
    Motion,
}

impl Group {
    const ALL: [Group; 5] = [Self::AfterRecording, Self::Earlier, Self::States, Self::Delivery, Self::Motion];

    fn title(self) -> &'static str {
        match self {
            Self::AfterRecording => "After recording",
            Self::Earlier => "Earlier ideas",
            Self::States => "States",
            Self::Delivery => "Delivery",
            Self::Motion => "Motion",
        }
    }
}

#[derive(Clone, Debug)]
enum Act {
    Show(Pres),
    /// A design shows this frame.
    Frame(usize),
    Finish,
    Click(ElemId),
    Hide,
}

#[derive(Clone, Debug)]
enum Look {
    Real(Pres),
    Proto(Proto),
}

/// One scene: what it shows held, and its loop.
#[derive(Clone, Debug)]
struct Def {
    group: Group,
    name: String,
    about: String,
    look: Look,
    clock: &'static str,
    /// `Surface::routes`, for a finish that completes with route chips.
    routes: Vec<String>,
    script: Vec<(f64, Act)>,
    period: f64,
    panel_h: f32,
}

impl Def {
    /// `pres` entering from hidden, resting, and leaving.
    fn state(group: Group, name: impl Into<String>, pres: Pres) -> Self {
        Self {
            group,
            name: name.into(),
            about: String::new(),
            look: Look::Real(pres.clone()),
            clock: "0:07",
            routes: Vec::new(),
            script: vec![(LEAD_S, Act::Show(pres)), (LEAD_S + HOLD_S, Act::Hide)],
            period: LEAD_S + HOLD_S + TAIL_S,
            panel_h: PANEL_H,
        }
    }

    /// A design playing its frames from hidden and leaving after the last.
    fn proto(proto: Proto) -> Self {
        let script: Vec<(f64, Act)> = proto
            .steps(HOLD_S)
            .into_iter()
            .map(|(at, frame)| (LEAD_S + at, frame.map_or(Act::Hide, Act::Frame)))
            .collect();
        let end = script.last().map_or(LEAD_S, |(at, _)| *at);
        Self {
            group: if Proto::NEW.contains(&proto) { Group::AfterRecording } else { Group::Earlier },
            name: proto.name().into(),
            about: proto.about().into(),
            look: Look::Proto(proto),
            clock: "0:07",
            routes: Vec::new(),
            script,
            period: end + TAIL_S,
            panel_h: proto.panel_h(),
        }
    }

    /// Steps at seconds after the lead-in; held, it shows `hold`. It leaves after its last step.
    fn motion(name: impl Into<String>, hold: Pres, steps: Vec<(f64, Act)>) -> Self {
        let last = steps.last().map_or(0.0, |(at, _)| *at);
        let mut script: Vec<(f64, Act)> = steps.into_iter().map(|(at, act)| (LEAD_S + at, act)).collect();
        let ends_hidden = matches!(script.last(), Some((_, Act::Hide)));
        let mut end = LEAD_S + last;
        if !ends_hidden {
            end += 1.4;
            script.push((end, Act::Hide));
        }
        Self {
            group: Group::Motion,
            name: name.into(),
            about: String::new(),
            look: Look::Real(hold),
            clock: "0:07",
            routes: Vec::new(),
            script,
            period: end + TAIL_S,
            panel_h: PANEL_H,
        }
    }

    fn clock(mut self, clock: &'static str) -> Self {
        self.clock = clock;
        self
    }

    fn routes(mut self, routes: &[&str]) -> Self {
        self.routes = routes.iter().map(|r| (*r).to_owned()).collect();
        self
    }

    fn about(mut self, about: &str) -> Self {
        about.clone_into(&mut self.about);
        self
    }

    fn frames(&self) -> usize {
        match self.look {
            Look::Proto(proto) => proto.frames().len(),
            Look::Real(_) => 0,
        }
    }
}

fn done(label: &str, routes: &[&str]) -> Pres {
    Pres::Completed { label: label.into(), routes: routes.iter().map(|r| (*r).to_owned()).collect() }
}

fn defs() -> Vec<Def> {
    use Group::{AfterRecording, Delivery, States};
    use SelectionMode::{Area, Iphone, Screen, Window};
    let picker = |mode, has_iphone| Pres::Selecting { mode, has_iphone };
    let recording = |chips, has_iphone| Pres::Recording { chips, has_iphone };
    let long_title = "OpenCode Desktop · Problems board rewrite";
    let long_chip = "Scrawl · Rune pill lab follow-ups";

    let mut defs: Vec<Def> = Proto::NEW.map(Def::proto).into();
    defs.push(
        Def::state(AfterRecording, "a · Today", done("Copied", &["Send to CONCLAVE", "HEX"]))
            .about("What ships: Copied, Paste, and a chip per destination."),
    );
    defs.extend(Proto::OLD.map(Def::proto));
    defs.extend([
        Def::state(States, "Picker · Screen", picker(Screen, true)),
        Def::state(States, "Picker · Area", picker(Area, true)),
        Def::state(States, "Picker · Window", picker(Window, true)),
        Def::state(States, "Picker · iPhone", picker(Iphone, true)),
        Def::state(States, "Picker · no iPhone", picker(Screen, false)),
        Def::state(States, "Recording + chips", recording(true, true)),
        Def::state(States, "Recording + chips · no iPhone", recording(true, false)),
        Def::state(States, "Recording", recording(false, true)),
        Def::state(States, "Recording · 12:34", recording(false, true)).clock("12:34"),
        Def::state(States, "Processing", Pres::Processing),
        Def::state(States, "Copied", done("Copied", &[])),
        Def::state(States, "From iPhone", done("From iPhone", &[])),
        Def::state(States, "3 from iPhone", done("3 from iPhone", &[])),
        Def::state(Delivery, "Offer · one route", done("Copied", &["Send to CONCLAVE"])),
        Def::state(Delivery, "Offer · two routes", done("Copied", &["Send to CONCLAVE", "HEX"])),
        Def::state(Delivery, "Sending", done("Sending to CONCLAVE…", &["Send to HEX"]))
            .about("3 s to undo before it goes."),
        Def::state(Delivery, "Sent", done("Sent to CONCLAVE", &[])),
        Def::state(Delivery, "Queued", done("Copied · couldn't reach CONCLAVE", &[]))
            .about("A send that fails is queued and retried; there's no failed state."),
        Def::state(
            Delivery,
            "Long names, truncated",
            Pres::Completed {
                label: format!("Sending to {}…", island_title(long_title)),
                routes: vec![format!("Send to {}", chip_title(long_chip))],
            },
        )
        .about("Truncated as the app does."),
        Def::state(
            Delivery,
            "Long names, untruncated",
            Pres::Completed {
                label: format!("Sent to {long_title}"),
                routes: vec![format!("Send to {long_chip}"), "HEX".into()],
            },
        ),
        Def::motion("Tap → recording + chips", recording(true, true), vec![(0.0, Act::Show(recording(true, true)))]),
        Def::motion(
            "Chips → Area picker",
            picker(Area, true),
            vec![(0.0, Act::Show(recording(true, true))), (1.1, Act::Click(ElemId::ChipArea))],
        ),
        Def::motion(
            "Chips fold (Screen)",
            recording(false, true),
            vec![(0.0, Act::Show(recording(true, true))), (1.1, Act::Click(ElemId::ChipScreen))],
        ),
        Def::motion(
            "Picker: Area → Window → Screen",
            picker(Screen, true),
            vec![
                (0.0, Act::Show(picker(Area, true))),
                (0.9, Act::Click(ElemId::ChipWindow)),
                (1.8, Act::Click(ElemId::ChipScreen)),
            ],
        ),
        Def::motion(
            "Finish → Processing → Copied",
            done("Copied", &["Send to CONCLAVE", "HEX"]),
            vec![(0.0, Act::Show(recording(false, true))), (0.9, Act::Finish), (3.4, Act::Hide)],
        )
        .routes(&["Send to CONCLAVE", "HEX"]),
        Def::motion(
            "Sending → Sent",
            done("Sent to CONCLAVE", &[]),
            vec![
                (0.0, Act::Show(done("Sending to CONCLAVE…", &["Send to HEX"]))),
                (1.6, Act::Show(done("Sent to CONCLAVE", &[]))),
            ],
        ),
        Def::motion("Cancel", recording(true, true), vec![(0.0, Act::Show(recording(true, true))), (1.2, Act::Hide)]),
        Def::motion(
            "Dismiss",
            done("Copied", &["Send to CONCLAVE"]),
            vec![(0.0, Act::Show(done("Copied", &["Send to CONCLAVE"]))), (1.4, Act::Hide)],
        ),
        Def::motion(
            "iPhone cabled: chip on ↔ off",
            picker(Screen, true),
            vec![
                (0.0, Act::Show(picker(Screen, false))),
                (1.0, Act::Show(picker(Screen, true))),
                (2.0, Act::Show(picker(Screen, false))),
            ],
        ),
    ]);
    // The sidebar lists them group by group.
    defs.sort_by_key(|d| Group::ALL.iter().position(|g| *g == d.group));
    defs
}

struct Cell {
    def: Def,
    looping: bool,
    surface: Surface,
    live: Capsule,
    /// A design's capsule is up (designs have no `Surface`), at this frame.
    shown: bool,
    frame: usize,
    /// Bumped to remount the capsule: a held state lands at rest, and an entry lands its
    /// origin pose without animating from the last run (the app's `Tx::instant`).
    epoch: u32,
    start: f64,
    next: usize,
    /// Its capsule at rest runs past the panel's edges.
    overflows: bool,
}

impl Cell {
    fn new(def: Def, t: f64, animate: bool) -> Self {
        let surface = fresh(&def, t);
        let live = if animate { Capsule::new(&surface) } else { Capsule::with_meter(&surface, Meter::new()) };
        let mut cell =
            Self { def, looping: false, surface, live, shown: false, frame: 0, epoch: 0, start: t, next: 0, overflows: false };
        cell.reset(t);
        cell
    }

    /// Starts the loop over, or lands the held state at rest.
    fn reset(&mut self, t: f64) {
        self.epoch += 1;
        self.surface = fresh(&self.def, t);
        self.start = t;
        self.next = 0;
        self.shown = !self.looping;
        if let Look::Proto(proto) = self.def.look {
            self.frame = if self.looping { 0 } else { proto.held() };
        }
        if let Look::Real(pres) = &self.def.look {
            let [x, _, w, _] = fresh(&self.def, t).settled(pres.clone()).shell;
            self.overflows = x < 0.0 || x + w > PANEL_W;
            if !self.looping {
                let surface = std::mem::replace(&mut self.surface, fresh(&self.def, t));
                self.surface = surface.settled(pres.clone());
            }
        }
        self.live.clock.set(self.def.clock.to_owned());
        self.live.morphs = self.surface.morphs;
        self.sync(None);
    }

    fn apply(&mut self, act: Act, t: f64) {
        match act {
            Act::Show(pres) => {
                self.shown = true;
                if let Look::Real(_) = self.def.look {
                    self.surface.set(pres, t);
                }
            }
            Act::Frame(frame) => {
                self.shown = true;
                self.frame = frame;
            }
            Act::Hide => {
                self.shown = false;
                self.surface.set(Pres::Hidden, t);
            }
            Act::Finish => self.surface.finish(t),
            Act::Click(id) => self.surface.click(id, t),
        }
        if std::mem::take(&mut self.surface.instant) {
            self.epoch += 1;
        }
    }

    /// Runs what's due at lab time `t`. True when the cell changed.
    fn advance(&mut self, t: f64, rate: f32) -> bool {
        let mut changed = self.surface.tick(t);
        if self.looping {
            if t - self.start > self.def.period * 4.0 {
                self.start = t;
                self.next = 0;
            }
            loop {
                let rel = t - self.start;
                if let Some((at, act)) = self.def.script.get(self.next)
                    && rel >= *at
                {
                    let act = act.clone();
                    self.next += 1;
                    self.apply(act, t);
                    changed = true;
                } else if self.next >= self.def.script.len() && rel >= self.def.period {
                    self.start += self.def.period;
                    self.next = 0;
                } else {
                    break;
                }
            }
        }
        self.surface.events.clear();
        self.surface.instant = false;
        self.sync(Some(rate));
        changed
    }

    /// The next lab time something happens.
    fn next_due(&self) -> Option<f64> {
        let script = self.looping.then(|| {
            self.def.script.get(self.next).map_or(self.start + self.def.period, |(at, _)| self.start + at)
        });
        [self.surface.next_due(), script].into_iter().flatten().min_by(f64::total_cmp)
    }

    fn sync(&mut self, rate: Option<f32>) {
        self.live.sync(&self.surface);
        if let Look::Proto(proto) = self.def.look
            && proto.recording()
        {
            self.live.meter.activity.store(u8::from(self.shown), Ordering::Relaxed);
        }
        if let Some(rate) = rate {
            self.live.meter.time_scale.store(rate.to_bits(), Ordering::Relaxed);
        }
    }

    fn press(&mut self, id: ElemId, t: f64) {
        match id {
            ElemId::Close | ElemId::Cancel | ElemId::Paste | ElemId::Inspect | ElemId::Route0 | ElemId::Route1 => {
                self.apply(Act::Hide, t);
            }
            ElemId::Finish => self.apply(Act::Finish, t),
            _ => self.apply(Act::Click(id), t),
        }
    }
}

/// A hidden surface on the lab clock: the lab scales time itself, so the surface runs at 1×.
fn fresh(def: &Def, t: f64) -> Surface {
    let mut surface = new_surface();
    surface.clock = def.clock.to_owned();
    surface.routes = def.routes.clone();
    surface.set_time_scale(t, 1.0);
    surface
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backdrop {
    Dark,
    Light,
    Busy,
}

impl Backdrop {
    const ALL: [Backdrop; 3] = [Self::Dark, Self::Light, Self::Busy];

    fn from_env() -> Self {
        match std::env::var("SCRAWL_LAB_BACKDROP").unwrap_or_default().trim().to_ascii_lowercase().as_str() {
            "light" | "white" => Self::Light,
            "busy" | "screenshot" => Self::Busy,
            _ => Self::Dark,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::Busy => "Screenshot",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Busy,
            Self::Busy => Self::Dark,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Play {
    /// The scene rests on its state.
    Held,
    Playing,
    /// Its loop stops where it is (springs already moving settle).
    Paused,
}

pub struct Grid {
    cells: Vec<Cell>,
    selected: usize,
    play: Play,
    /// A headless still: everything rests, nothing plays.
    still: bool,
    /// Lab seconds: real seconds × `scale`, stopped while paused. The scene's script runs on
    /// it; Rune's springs run at `scale`.
    t: f64,
    last: Option<f64>,
    /// Rune's clock for the whole window, so it never goes to 0: a stopped scene clock freezes
    /// every exit, colour change and scroll. Pause stops the script instead.
    scale: f32,
    backdrop: Backdrop,
    /// The stage fits the scene (up to `MAX_ZOOM`), else shows it at real size.
    fit: bool,
    screenshot: Option<Pixels>,
    screenshot_from: Option<PathBuf>,
    /// The springs the scene draws with (the sliders set them before tune.json reads back).
    tune: Tune,
    /// tune.json as last read, so only a change there replaces `tune`.
    tune_seen: Tune,
}

impl Grid {
    fn new(still: bool) -> Self {
        if !still && !headless() {
            write_lab_file();
        }
        motion::poll_tune();
        style::poll_lab_file();
        let tune = motion::tune();
        let (screenshot, screenshot_from) = match screenshot() {
            Some((pixels, path)) => (Some(pixels), Some(path)),
            None => (None, None),
        };
        let defs = defs();
        let selected = scene_from_env(&defs);
        let cells = defs.into_iter().map(|def| Cell::new(def, 0.0, !still)).collect();
        let mut g = Self {
            cells,
            selected,
            play: Play::Held,
            still,
            t: 0.0,
            last: None,
            scale: tune.time_scale.clamp(0.05, 4.0),
            backdrop: Backdrop::from_env(),
            fit: true,
            screenshot,
            screenshot_from,
            tune,
            tune_seen: tune,
        };
        // 1-based, as the inspector numbers them.
        if let Some(frame) = std::env::var("SCRAWL_PILL_LAB_FRAME").ok().and_then(|f| f.parse::<usize>().ok()) {
            g.show_frame(frame.saturating_sub(1));
        }
        if std::env::var_os("SCRAWL_PILL_GRID_FILM").is_some() {
            g.toggle_play();
        }
        g
    }

    fn cell(&mut self) -> &mut Cell {
        &mut self.cells[self.selected]
    }

    /// Shows scene `i`, held, or playing from its start while the lab plays.
    fn select(&mut self, i: usize) {
        if i == self.selected || i >= self.cells.len() {
            return;
        }
        dbg_log(&format!("select {i}"));
        self.selected = i;
        if self.play == Play::Paused {
            self.play = Play::Held;
        }
        let (t, looping) = (self.t, self.play == Play::Playing);
        let cell = self.cell();
        cell.looping = looping;
        cell.reset(t);
    }

    fn step(&mut self, by: isize) {
        let n = self.cells.len() as isize;
        self.select((self.selected as isize + by).clamp(0, n - 1) as usize);
    }

    /// Plays the scene from its start, pauses it, or plays on from where it paused.
    fn toggle_play(&mut self) {
        dbg_log("play");
        self.play = match self.play {
            Play::Held => {
                let t = self.t;
                let cell = self.cell();
                cell.looping = true;
                cell.reset(t);
                Play::Playing
            }
            Play::Playing => Play::Paused,
            Play::Paused => Play::Playing,
        };
    }

    /// Back to resting on the scene's state.
    fn hold(&mut self) {
        dbg_log("hold");
        self.play = Play::Held;
        let t = self.t;
        let cell = self.cell();
        cell.looping = false;
        cell.reset(t);
    }

    /// Holds a design on one of its frames; the capsule moves there on its springs.
    fn show_frame(&mut self, frame: usize) {
        if frame >= self.cells[self.selected].def.frames() {
            return;
        }
        if self.play != Play::Held || !self.cells[self.selected].shown {
            self.hold();
        }
        self.cell().frame = frame;
    }

    fn speed(&mut self) {
        self.scale = if self.scale > 0.5 { 0.25 } else { 1.0 };
    }

    /// Rune's clock for the window: never 0 while it's live, or the chrome and scrolling stop.
    fn window_scale(&self) -> f32 {
        if self.still { 0.0 } else { 1.0 }
    }

    /// The scene's own clock (`El::time_scale`): paused freezes its springs where they are.
    fn scene_rate(&self) -> f32 {
        if self.play == Play::Paused { 0.0 } else { self.scale }
    }

    fn press(&mut self, id: ElemId) {
        let t = self.t;
        self.cell().press(id, t);
    }

    /// The springs the scene draws with, now; saved to tune.json.
    fn set_tune(&mut self, key: &str, value: f32) {
        let Some((_, _, field)) = KEYS.iter().find(|(k, ..)| *k == key) else { return };
        *field(&mut self.tune) = value;
        let tune = self.tune;
        for cell in &mut self.cells {
            cell.surface.tune = tune;
        }
        save_tune(&[(key, Some(value))]);
    }

    fn reset_tune(&mut self) {
        self.tune = Tune::DEFAULT;
        let tune = self.tune;
        for cell in &mut self.cells {
            cell.surface.tune = tune;
        }
        let keys: Vec<(&str, Option<f32>)> = SPRING_KNOBS.iter().map(|(k, ..)| (*k, None)).collect();
        save_tune(&keys);
    }

    /// New spacing, type or colours: a held real capsule lays out again (designs lay out as they
    /// draw, so theirs move on their springs).
    fn restyle(&mut self, next: Style) {
        set_style(next);
        if self.play == Play::Held && matches!(self.cells[self.selected].def.look, Look::Real(_)) {
            let t = self.t;
            self.cell().reset(t);
        }
    }
}

/// `SCRAWL_PILL_LAB_SCENE`: a scene's index, or part of its name.
fn scene_from_env(defs: &[Def]) -> usize {
    let Ok(want) = std::env::var("SCRAWL_PILL_LAB_SCENE") else { return 0 };
    if let Ok(i) = want.parse::<usize>() {
        return i.min(defs.len().saturating_sub(1));
    }
    let want = want.to_lowercase();
    defs.iter().position(|d| d.name.to_lowercase().contains(&want)).unwrap_or(0)
}

/// Sets (or, with `None`, removes) keys in tune.json, keeping the rest.
fn save_tune(keys: &[(&str, Option<f32>)]) {
    let Some(path) = tune_path() else { return };
    let mut map = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    for (key, value) in keys {
        match value {
            Some(v) => {
                map.insert((*key).to_owned(), serde_json::json!((f64::from(*v) * 1000.0).round() / 1000.0));
            }
            None => {
                map.remove(*key);
            }
        }
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let json = serde_json::to_string_pretty(&serde_json::Value::Object(map)).unwrap_or_default();
    let _ = std::fs::write(path, json + "\n");
}

fn poll(g: &mut Grid, now: f64, tx: &mut Tx) -> bool {
    if g.still {
        return false;
    }
    let mut changed = false;
    motion::poll_tune();
    let tune = motion::tune();
    if tune != g.tune_seen {
        if (tune.time_scale - g.tune_seen.time_scale).abs() > 1e-6 {
            g.scale = tune.time_scale.clamp(0.05, 4.0);
        }
        g.tune_seen = tune;
        if tune != g.tune {
            g.tune = tune;
            for cell in &mut g.cells {
                cell.surface.tune = tune;
            }
            changed = true;
        }
    }
    if style::poll_lab_file() {
        // New widths and type: the scene lays out again from scratch.
        let t = g.t;
        g.cell().reset(t);
        changed = true;
    }
    let dt = g.last.map_or(0.0, |last| (now - last).max(0.0));
    g.last = Some(now);
    let rate = g.scene_rate();
    g.t += dt * f64::from(rate);
    let t = g.t;
    changed |= g.cell().advance(t, rate);
    if rate > 0.0
        && let Some(due) = g.cells[g.selected].next_due()
    {
        tx.wake_after(Duration::from_secs_f64(((due - t) / f64::from(rate)).max(0.0)));
    }
    // The tune and lab files are polled at most four times a second.
    tx.wake_after(Duration::from_millis(250));
    changed
}

fn on_key(g: &mut Grid, key: &KeyEvent, tx: &mut Tx) -> bool {
    match &key.key {
        // Rune's default app menu has Quit but no Close, so ⌘W never reached the window.
        // Closing the only window ends the process, as the close button does.
        Key::Command('w') => {
            dbg_log("close");
            tx.close_window();
            return true;
        }
        Key::Command('q') => {
            dbg_log("quit");
            tx.quit();
            return true;
        }
        _ if key.mods.cmd || key.mods.ctrl => return false,
        Key::Up => g.step(-1),
        Key::Down => g.step(1),
        Key::Escape => g.hold(),
        Key::Text(text) => match text.to_ascii_lowercase().as_str() {
            "k" => g.step(-1),
            "j" => g.step(1),
            " " => g.toggle_play(),
            "r" => g.hold(),
            "s" => g.speed(),
            "b" => g.backdrop = g.backdrop.next(),
            "z" => g.fit = !g.fit,
            digit if digit.len() == 1 && (b'1'..=b'9').contains(&digit.as_bytes()[0]) => {
                // Frames move on the design's springs.
                g.show_frame(usize::from(digit.as_bytes()[0] - b'1'));
                return true;
            }
            _ => return false,
        },
        _ => return false,
    }
    // The lab's chrome never animates; only the capsule moves.
    tx.instant();
    true
}

// ─── View ────────────────────────────────────────────────────────────────────────────────────

fn ink(level: f32) -> Color {
    srgb(0.96, 0.96, 0.97, level)
}

fn chrome() -> Color {
    srgb(0.078, 0.082, 0.094, 1.0)
}

fn font(size: f32, weight: f32) -> TextStyle {
    TextStyle::new(size, weight)
}

fn accent() -> Color {
    srgb(0.38, 0.52, 1.0, 1.0)
}

fn caps(title: &str) -> El {
    text(title.to_uppercase()).font(font(10.5, 650.0).tracking(0.6)).color(ink(0.38))
}

/// A quiet text button: no box, a faint fill under the pointer. Chrome presses apply instantly.
fn action(key: &str, label: &str, help: &str, press: impl Fn(&mut Grid) + 'static) -> El {
    row().key(format!("act-{key}")).center().h(26.0).px(8.0).radius(6.0).hover(ink(0.07))
        .child(text(label).font(font(12.5, 550.0)).color(ink(0.88)))
        .help(help)
        .on_press_tx(move |g: &mut Grid, tx: &mut Tx| {
            tx.instant();
            press(g);
        })
}

/// One of a few choices, in full ink when it's the one.
fn choice(key: String, label: &str, on: bool, press: impl Fn(&mut Grid) + 'static) -> El {
    row().key(key).center().h(24.0).px(7.0).radius(6.0).hover(ink(0.06))
        .child(text(label).font(font(12.5, if on { 600.0 } else { 450.0 })).color(ink(if on { 0.95 } else { 0.45 })))
        .on_press_tx(move |g: &mut Grid, tx: &mut Tx| {
            tx.instant();
            press(g);
        })
}

fn setting(label: &str, choices: Vec<El>) -> El {
    let mut r = row().center_y().h(28.0).child(text(label).font(font(12.0, 450.0)).color(ink(0.5)).w(72.0));
    for c in choices {
        r = r.child(c);
    }
    r
}

fn section(title: &str, reset: Option<(&str, fn(&mut Grid))>) -> El {
    let mut header = row().center_y().h(40.0).pt(14.0).child(caps(title)).child(el().grow(1.0));
    if let Some((key, press)) = reset {
        header = header.child(
            row().key(format!("reset-{key}")).center().h(22.0).px(6.0).radius(5.0).hover(ink(0.06))
                .child(text("Reset").font(font(11.5, 500.0)).color(ink(0.45)))
                .on_press_tx(move |g: &mut Grid, tx: &mut Tx| {
                    tx.instant();
                    press(g);
                }),
        );
    }
    header
}

fn number(v: f32, step: f32) -> String {
    match step {
        s if s >= 1.0 => format!("{v:.0}"),
        s if s >= 0.1 => format!("{v:.1}"),
        s if s >= 0.01 => format!("{v:.2}"),
        _ => format!("{v:.3}"),
    }
}

/// A knob: its name and value over a track; a press or drag anywhere on it sets the value.
fn slider(key: String, name: &str, value: f32, default: f32, range: Range, set: impl Fn(&mut Grid, f32) + 'static) -> El {
    let (lo, hi, step) = range;
    let w = INSPECTOR_W - 32.0;
    let frac = ((value - lo) / (hi - lo)).clamp(0.0, 1.0);
    let tuned = (value - default).abs() > step * 0.5;
    let mark = if tuned { accent() } else { ink(0.5) };
    col().key(key).w(w).h(38.0).gap(7.0).pt(5.0)
        .child(
            row().center_y()
                .child(text(name).font(font(12.0, 450.0)).color(ink(0.72)))
                .child(el().grow(1.0))
                .child(text(number(value, step)).font(font(12.0, 500.0)).color(mark)),
        )
        .child(
            el().w(w).h(3.0).radius(1.5).fill(ink(0.1))
                .child(el().absolute(0.0, 0.0).w(w * frac).h(3.0).radius(1.5).fill(mark))
                .child(el().absolute((w * frac - 4.5).clamp(0.0, w - 9.0), -3.0).size(9.0, 9.0).radius(4.5).fill(ink(0.92))),
        )
        .on_drag(move |g: &mut Grid, d: &mut Drag, tx: &mut Tx| {
            tx.instant();
            let v = lo + (d.x / d.w.max(1.0)).clamp(0.0, 1.0) * (hi - lo);
            set(g, ((v / step).round() * step).clamp(lo, hi));
        })
}

/// (low, high, step).
type Range = (f32, f32, f32);
type StyleField = fn(&mut Style) -> &mut f32;

const STYLE_KNOBS: &[(&str, StyleField, Range)] = &[
    ("Chip gap", |s| &mut s.chip_gap, (0.0, 16.0, 0.5)),
    ("Group gap", |s| &mut s.group_gap, (0.0, 24.0, 0.5)),
    ("Chip padding", |s| &mut s.chip_px, (4.0, 24.0, 0.5)),
    ("After the lead", |s| &mut s.lead_trail, (0.0, 24.0, 0.5)),
    ("Text size", |s| &mut s.text_size, (10.0, 18.0, 0.25)),
    ("Tracking", |s| &mut s.tracking, (-0.5, 0.5, 0.01)),
    ("Weight", |s| &mut s.medium, (400.0, 700.0, 10.0)),
    ("Bold weight", |s| &mut s.semibold, (500.0, 800.0, 10.0)),
    ("Body opacity", |s| &mut s.shell_alpha, (0.5, 1.0, 0.01)),
    ("Shadow", |s| &mut s.shadow_alpha, (0.0, 0.8, 0.01)),
    ("Chip fill", |s| &mut s.chip_fill, (0.0, 0.2, 0.005)),
];

/// tune.json keys the real capsule's springs read.
const SPRING_KNOBS: &[(&str, &str, Range)] = &[
    ("island.duration", "Morph", (0.05, 1.5, 0.01)),
    ("island.bounce", "Morph bounce", (0.0, 0.6, 0.01)),
    ("scale.duration", "Open and close", (0.05, 1.5, 0.01)),
    ("scale.bounce", "Open and close bounce", (0.0, 0.6, 0.01)),
    ("reveal.open.duration", "Chips open", (0.05, 1.5, 0.01)),
    ("reveal.open.bounce", "Chips open bounce", (0.0, 0.6, 0.01)),
    ("reveal.close.duration", "Chips fold", (0.05, 1.5, 0.01)),
    ("reveal.close.bounce", "Chips fold bounce", (0.0, 0.6, 0.01)),
    ("switch.width.duration", "Chip to picker width", (0.05, 1.5, 0.01)),
    ("switch.width.bounce", "Chip to picker bounce", (0.0, 0.6, 0.01)),
    ("switch.slide.duration", "Chip to picker slide", (0.05, 1.5, 0.01)),
    ("switch.slide.bounce", "Chip to picker slide bounce", (0.0, 0.6, 0.01)),
    ("reveal.enter", "Fade in (s)", (0.0, 1.0, 0.01)),
    ("reveal.exit", "Fade out (s)", (0.0, 1.0, 0.01)),
    ("reveal.blur", "Fade blur (px)", (0.0, 8.0, 0.1)),
];

fn sidebar(g: &Grid) -> El {
    let mut list = col().w_full().px(8.0).pb(16.0);
    for group in Group::ALL {
        list = list.child(row().center_y().h(30.0).pt(8.0).px(10.0).child(caps(group.title())));
        for (i, cell) in g.cells.iter().enumerate().filter(|(_, c)| c.def.group == group) {
            let on = i == g.selected;
            list = list.child(
                row().key(format!("scene-{i}")).center_y().h(28.0).px(10.0).radius(6.0)
                    .fill(if on { ink(0.1) } else { Color::TRANSPARENT })
                    .hover(ink(if on { 0.12 } else { 0.05 }))
                    .child(
                        text(&cell.def.name)
                            .font(font(12.5, if on { 560.0 } else { 450.0 }))
                            .color(ink(if on { 0.95 } else { 0.66 }))
                            .truncate(),
                    )
                    .help(if on { "Play or pause (space)" } else { "" })
                    .on_press_tx(move |g: &mut Grid, tx: &mut Tx| {
                        tx.instant();
                        if g.selected == i { g.toggle_play() } else { g.select(i) }
                    }),
            );
        }
    }
    col().w(SIDEBAR_W).h_full().fill(chrome())
        .child(el().h(TITLE_BAR_H).w_full().window_drag())
        .child(col().key("sidebar").grow(1.0).min_h(0.0).w_full().scroll(ScrollSpec { stick_bottom: false }).child(list))
}

fn inspector(g: &Grid) -> El {
    let cell = &g.cells[g.selected];
    let w = INSPECTOR_W - 32.0;
    let mut body = col().w(w).pb(24.0)
        .child(text(&cell.def.name).font(font(15.0, 650.0)).color(ink(0.95)).w(w).wrap());
    if !cell.def.about.is_empty() {
        body = body.child(el().h(6.0)).child(text(&cell.def.about).font(font(12.0, 450.0)).color(ink(0.55)).w(w).wrap());
    }
    if cell.overflows {
        body = body.child(el().h(6.0)).child(
            text("Wider than its panel: the app cuts it off.").font(font(12.0, 550.0)).color(srgb(0.96, 0.42, 0.36, 1.0)).w(w).wrap(),
        );
    }
    let play = if g.play == Play::Playing { "❚❚ Pause" } else { "▶ Play" };
    let fast = g.scale > 0.5;
    body = body
        .child(section("Playback", None))
        .child(
            row().center_y().gap(4.0).ml(-8.0)
                .child(action("play", play, "space, or click the scene again", Grid::toggle_play))
                .child(action("reset", "Reset", "r · back to resting on its state", Grid::hold)),
        )
        .child(setting("Speed", vec![
            choice("speed-1".into(), "1×", fast, |g: &mut Grid| g.scale = 1.0),
            choice("speed-q".into(), "¼×", !fast, |g: &mut Grid| g.scale = 0.25),
        ]))
        .child(setting(
            "Backdrop",
            Backdrop::ALL.iter().map(|&b| choice(format!("bg-{}", b.name()), b.name(), g.backdrop == b, move |g: &mut Grid| g.backdrop = b)).collect(),
        ))
        .child(setting("Size", vec![
            choice("size-fit".into(), "Fit", g.fit, |g: &mut Grid| g.fit = true),
            choice("size-1".into(), "Actual", !g.fit, |g: &mut Grid| g.fit = false),
        ]));
    let frames = cell.def.frames();
    if frames > 1 {
        let mut picks = row().center_y().ml(-7.0);
        for f in 0..frames {
            let on = cell.shown && cell.frame == f;
            // Frames move on the design's springs, so these don't apply instantly.
            picks = picks.child(
                row().key(format!("frame-{f}")).center().size(26.0, 24.0).radius(6.0).hover(ink(0.06))
                    .child(text((f + 1).to_string()).font(font(12.5, if on { 600.0 } else { 450.0 })).color(ink(if on { 0.95 } else { 0.45 })))
                    .help("1–9")
                    .on_press(move |g: &mut Grid| g.show_frame(f)),
            );
        }
        body = body.child(section("Frames", None)).child(picks);
    }
    let now = style::style();
    body = body.child(section("Style", Some(("style", |g: &mut Grid| g.restyle(Style::DEFAULT)))));
    for (n, (name, field, range)) in STYLE_KNOBS.iter().enumerate() {
        let (mut current, mut default) = (now, Style::DEFAULT);
        let field = *field;
        body = body.child(slider(format!("style-{n}"), name, *field(&mut current), *field(&mut default), *range, move |g: &mut Grid, v| {
            let mut next = style::style();
            *field(&mut next) = v;
            if next != style::style() {
                g.restyle(next);
            }
        }));
    }
    if matches!(cell.def.look, Look::Real(_)) {
        body = body.child(section("Springs", Some(("springs", Grid::reset_tune))));
        for (key, name, range) in SPRING_KNOBS {
            let value = g.tune.get(key).unwrap_or_default();
            let default = Tune::DEFAULT.get(key).unwrap_or_default();
            body = body.child(slider(format!("spring-{key}"), name, value, default, *range, move |g: &mut Grid, v| g.set_tune(key, v)));
        }
    }
    col().w(INSPECTOR_W).h_full().fill(chrome())
        .child(el().h(TITLE_BAR_H - 10.0).w_full().window_drag())
        .child(
            col().key("inspector").grow(1.0).min_h(0.0).w_full().px(16.0).scroll(ScrollSpec { stick_bottom: false }).child(body),
        )
}

fn dbg_log(what: &str) {
    if std::env::var_os("SCRAWL_PILL_GRID_LOG").is_some() {
        eprintln!("pill lab: {what}");
    }
}

fn presence(surface: &Surface) -> Transition {
    Transition::blur(1.5).combined(Transition::scale(0.96)).springs(SpringPair::new(surface.presence(), surface::PRESENCE_EXIT))
}

/// The scene, `zoom` times its size, its top a little below the stage's (where the real capsule
/// hangs from the top of the screen).
fn stage(g: &Grid, cx: &mut Cx<Grid>) -> El {
    const MARGIN: f32 = 48.0;
    let i = g.selected;
    let cell = &g.cells[i];
    let panel_h = cell.def.panel_h;
    let [_, _, sw, sh] = cx.layout("stage").unwrap_or([0.0, 0.0, SNAP_SIZE.0 - SIDEBAR_W - INSPECTOR_W, SNAP_SIZE.1]);
    let zoom = if g.fit {
        ((sw - 2.0 * MARGIN) / PANEL_W).min((sh - 2.0 * MARGIN) / panel_h).clamp(0.5, MAX_ZOOM)
    } else {
        1.0
    };
    let panel = match cell.def.look {
        Look::Real(_) => capsule(&cell.surface, &cell.live, cx, move |g: &mut Grid, id| g.press(id)),
        Look::Proto(p) => proto::capsule(p, cell.frame, &cell.live.meter, cell.shown, presence(&cell.surface)),
    };
    // Scaled about its centre: place it so the scaled top sits at the margin.
    let x = (sw - PANEL_W) * 0.5;
    let y = MARGIN - panel_h * 0.5 * (1.0 - zoom);
    let scene = el().key(format!("panel-{i}-{}", cell.epoch)).absolute(x, y).size(PANEL_W, panel_h)
        // The capsule's window is the panel: what doesn't fit there is cut off.
        .clip_rounded(10.0)
        .child(panel)
        .scale(zoom)
        .time_scale(g.scene_rate());
    let backdrop = el().absolute(0.0, 0.0).w_full().h_full();
    let backdrop = match (g.backdrop, &g.screenshot) {
        (Backdrop::Busy, Some(pixels)) => backdrop.pixels_fit(pixels, SurfaceFit::Fill),
        (Backdrop::Light, _) => backdrop.fill(srgb(0.957, 0.957, 0.965, 1.0)),
        (Backdrop::Busy, None) => backdrop.fill(srgb(0.45, 0.47, 0.52, 1.0)),
        (Backdrop::Dark, _) => backdrop.fill(srgb(0.059, 0.067, 0.090, 1.0)),
    };
    let mut stage = el().key("stage").grow(1.0).h_full().child(backdrop).child(scene);
    if g.backdrop == Backdrop::Busy && g.screenshot.is_none() {
        stage = stage.child(
            text("No screenshot: set SCRAWL_LAB_SCREENSHOT to a .png (default: the latest capture's first frame).")
                .font(font(12.0, 500.0))
                .color(ink(0.8))
                .absolute(16.0, sh - 32.0),
        );
    }
    stage
}

fn view(g: &Grid, cx: &mut Cx<Grid>) -> El {
    let stage = stage(g, cx);
    // After the capsule, which sets the window's clock to its surface's 1×.
    cx.time_scale(g.window_scale());
    let line = || el().w(1.0).h_full().fill(srgb(1.0, 1.0, 1.0, 0.06));
    row().w_full().h_full().fill(chrome()).child(sidebar(g)).child(line()).child(stage).child(line()).child(inspector(g))
}

// ─── Running ───────────────────────────────────────────────────────────────────────────────

/// `SCRAWL_LAB_SCREENSHOT`, else the newest capture's first keyframe: real content for the
/// capsule's glass and shadow to read over.
fn screenshot() -> Option<(Pixels, PathBuf)> {
    let path = std::env::var_os("SCRAWL_LAB_SCREENSHOT").map(PathBuf::from).or_else(|| {
        let captures = crate::handoff::default_captures_dir();
        let mut bundles: Vec<_> = std::fs::read_dir(captures).ok()?.flatten().map(|e| e.path()).collect();
        bundles.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        bundles.iter().rev().map(|b| b.join("frames/frame-01.png")).find(|f| f.exists())
    })?;
    let (w, h, rgba) = decode_png(&path)?;
    let pixels = Pixels::new();
    pixels.set(w, h, rgba);
    Some((pixels, path))
}

fn decode_png(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&v| [v, v, v, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((info.width, info.height, rgba))
}

/// Writes every knob at its shipping value to `pill-lab.json` the first time, so the file lists
/// what can be tuned.
fn write_lab_file() {
    let Some(path) = lab_path() else { return };
    if path.exists() {
        return;
    }
    let json = serde_json::to_string_pretty(&style::Style::DEFAULT.to_json()).unwrap_or_default();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::write(&path, json + "\n").is_ok() {
        eprintln!("pill lab: wrote the knobs to {}", path.display());
    }
}

fn init() -> Grid {
    Grid::new(false)
}

fn spec(size: (f32, f32), fonts: &'static [&'static [u8]]) -> AppSpec<Grid> {
    AppSpec {
        title: "Scrawl pill lab",
        size,
        min_size: Some((SIDEBAR_W + INSPECTOR_W + PANEL_W * 0.75, 520.0)),
        init,
        view,
        scenes: &[],
        fonts,
        families: families(fonts),
        setup: None,
        poll: Some(poll),
        on_key: Some(on_key),
    }
}

/// Running without a window someone looks at: a film or a scripted check.
fn headless() -> bool {
    std::env::var_os("SCRAWL_PILL_GRID_FILM").is_some() || std::env::var_os("SCRAWL_PILL_GRID_RUNE_ARGS").is_some()
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// `--snap DIR`: one still per scene held, and one per frame of a design that has several.
fn snap_all(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let exe = std::env::current_exe()?;
    for (i, def) in defs().iter().enumerate() {
        let frames: Vec<Option<usize>> =
            if def.frames() > 1 { (0..def.frames()).map(Some).collect() } else { vec![None] };
        for frame in frames {
            let name = match frame {
                Some(f) => format!("scene-{i:02}-{}-{}.png", slug(&def.name), f + 1),
                None => format!("scene-{i:02}-{}.png", slug(&def.name)),
            };
            let out = dir.join(name);
            let mut cmd = std::process::Command::new(&exe);
            cmd.args(["lab", "pill", "grid", "--rune", "--snap"])
                .arg(&out)
                .env("SCRAWL_PILL_LAB_SCENE", i.to_string())
                .env_remove("SCRAWL_PILL_LAB_FRAME")
                .env_remove("SCRAWL_MOTION_TRACE");
            if let Some(f) = frame {
                cmd.env("SCRAWL_PILL_LAB_FRAME", (f + 1).to_string());
            }
            let status = cmd.status()?;
            anyhow::ensure!(status.success(), "snap {} failed ({status})", def.name);
            println!("{}", out.display());
        }
    }
    Ok(())
}

pub fn run(snap: Option<PathBuf>) -> anyhow::Result<()> {
    if let Some(dir) = &snap
        && dir.extension().is_none_or(|e| e != "png" && e != "mp4")
    {
        return snap_all(dir);
    }
    let fonts = fonts();
    // Headless with Rune's own flags (`--script`, `--snap`, `--tree`…), as the live window
    // starts: for reproducing what a click does.
    if let Ok(args) = std::env::var("SCRAWL_PILL_GRID_RUNE_ARGS") {
        let args: Vec<String> = args.split_whitespace().map(str::to_owned).collect();
        rune::run_windows_with_args(RunOptions { accessory: true }, args, move |_| {
            vec![WindowSpec::from_app("grid", App::new(spec(SNAP_SIZE, fonts))).state(Grid::new(false))]
        });
        return Ok(());
    }
    if let Some(out) = snap.as_ref().filter(|p| p.extension().is_some_and(|e| e == "mp4")) {
        // Headless, on Rune's virtual clock: the scene (`SCRAWL_PILL_LAB_SCENE`) playing at 1×.
        unsafe { std::env::set_var("SCRAWL_PILL_GRID_FILM", "1") };
        let seconds = std::env::var("SCRAWL_PILL_GRID_SECONDS").unwrap_or_else(|_| "8".into());
        let args = ["--video", &out.to_string_lossy(), "--duration", &seconds, "--fps", "30", "--scale", "1"]
            .map(str::to_owned);
        rune::run_windows_with_args(RunOptions { accessory: true }, args, move |_| {
            vec![WindowSpec::from_app("grid", App::new(spec(SNAP_SIZE, fonts))).state(Grid::new(false))]
        });
        return Ok(());
    }
    if let Some(out) = snap {
        let args = ["--snap".to_owned(), out.to_string_lossy().into_owned()];
        rune::run_windows_with_args(RunOptions { accessory: true }, args, move |_| {
            let grid = Grid::new(true);
            if let Some(from) = &grid.screenshot_from
                && grid.backdrop == Backdrop::Busy
            {
                eprintln!("pill lab: screenshot backdrop {}", from.display());
            }
            vec![WindowSpec::from_app("grid", App::new(spec(SNAP_SIZE, fonts))).state(grid)]
        });
        return Ok(());
    }
    rune::run_windows_with_args(RunOptions { accessory: false }, Vec::new(), move |displays| {
        let (w, h) = displays
            .iter()
            .find(|d| d.primary)
            .or(displays.first())
            .map_or((1440.0, 900.0), |d| (d.size.0 as f32, d.size.1 as f32));
        let size = ((w - 160.0).clamp(1100.0, 1880.0), (h - 160.0).clamp(620.0, 1080.0));
        vec![WindowSpec::from_app("grid", App::new(spec(size, fonts)))]
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_loop_ends_hidden_inside_its_period() {
        for def in defs() {
            let last = def.script.last().expect("steps");
            assert!(matches!(last.1, Act::Hide), "{}", def.name);
            assert!(last.0 + TAIL_S <= def.period, "{}", def.name);
            assert!(def.script.windows(2).all(|w| w[0].0 <= w[1].0), "{}", def.name);
        }
    }

    #[test]
    fn a_looping_state_is_gone_for_under_half_a_second() {
        for def in defs().into_iter().filter(|d| d.group != Group::Motion) {
            let shown: f64 = def.script.first().map_or(0.0, |(at, _)| *at);
            let hidden_mid: f64 = def
                .script
                .windows(2)
                .filter(|w| matches!(w[0].1, Act::Hide))
                .map(|w| w[1].0 - w[0].0)
                .sum();
            let last = def.script.last().map_or(0.0, |(at, _)| *at);
            let gap = shown + (def.period - last);
            assert!(gap <= 0.5 && hidden_mid <= 0.5, "{}: hidden {gap:.2} s", def.name);
        }
    }

    #[test]
    fn the_sidebar_lists_f_first_and_every_group_together() {
        let defs = defs();
        assert_eq!(defs[0].name, "f · Sure");
        let groups: Vec<Group> = defs.iter().map(|d| d.group).collect();
        let mut seen = groups.clone();
        seen.dedup();
        assert_eq!(seen, Group::ALL, "each group once, in order");
    }

    #[test]
    fn a_scene_starts_held_and_play_pause_and_reset_touch_only_it() {
        fonts();
        let mut g = Grid::new(false);
        assert!(g.cells.iter().all(|c| !c.looping && c.shown), "everything held, at rest");
        g.toggle_play();
        assert_eq!(g.play, Play::Playing);
        assert!(g.cells[0].looping && !g.cells[0].shown, "plays from hidden");
        g.select(3);
        assert!(g.cells[3].looping, "a scene picked while playing plays");
        g.toggle_play();
        assert_eq!(g.play, Play::Paused);
        assert!(g.window_scale() > 0.0, "paused, the chrome and scrolling still move");
        assert_eq!(g.scene_rate(), 0.0, "and the scene's springs stop where they are");
        g.select(4);
        assert_eq!(g.play, Play::Held);
        assert!(!g.cells[4].looping && g.cells[4].shown, "picked while paused, it rests");
        g.toggle_play();
        g.hold();
        assert!(!g.cells[4].looping && g.cells[4].shown);
        g.show_frame(2);
        assert!(g.cells[4].frame == 2 || g.cells[4].def.frames() <= 2);
        g.step(-10);
        assert_eq!(g.selected, 0);
        g.step(1_000);
        assert_eq!(g.selected, g.cells.len() - 1);
    }

    #[test]
    fn a_held_cell_rests_on_its_state_and_a_looping_one_plays_it() {
        fonts();
        let def = Def::state(Group::States, "Copied", done("Copied", &[]));
        let mut held = Cell::new(def.clone(), 0.0, false);
        assert!(matches!(held.surface.pres, Pres::Completed { .. }));
        assert!(held.surface.shell_on);
        held.looping = true;
        held.reset(0.0);
        assert!(matches!(held.surface.pres, Pres::Hidden));
        let mut t = 0.0;
        while t < LEAD_S + 0.5 {
            t += 1.0 / 60.0;
            held.advance(t, 1.0);
        }
        assert!(matches!(held.surface.pres, Pres::Completed { .. }));
        while t < def.period + LEAD_S * 0.5 {
            t += 1.0 / 60.0;
            held.advance(t, 1.0);
        }
        assert!(matches!(held.surface.pres, Pres::Hidden), "hidden again at the loop's start");
    }

    #[test]
    fn every_spring_knob_is_a_tune_key() {
        for (key, ..) in SPRING_KNOBS {
            assert!(Tune::DEFAULT.get(key).is_some(), "{key}");
        }
    }
}
