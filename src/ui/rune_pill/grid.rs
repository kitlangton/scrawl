//! `scrawl lab pill grid --rune`: every capsule state at once in one ordinary window, each a live
//! capsule drawn by the real view (`capsule`) at real size over a dark, light or busy backdrop,
//! with the post-record designs Kit is weighing (`proto`) beside today's.
//!
//! Every cell starts held at rest; clicking its title loops its entry, transition or flow. One
//! lab clock drives every cell's script, so the speed and pause apply to all of them; Rune's
//! springs follow the speed. Springs follow `scrawl tune` (tune.json) and spacing, type and colours
//! follow `pill-lab.json` (`style`), both re-read within a quarter second.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use rune::{
    App, AppSpec, Color, Cx, El, Key, KeyEvent, Pixels, RunOptions, ScrollSpec, SpringPair, SurfaceFit, TextStyle,
    Transition, Tx, WindowSpec, col, el, row, text,
};

use super::proto::{self, Proto};
use super::style::{self, lab_path};
use super::{Capsule, Meter, PANEL_H, PANEL_W, Surface, capsule, families, fonts, new_surface, srgb, surface};
use crate::native::pill::model::{ElemId, Pres};
use crate::ui::motion::{self, Tune};
use crate::ui::{SelectionMode, chip_title, island_title};

const PAD: f32 = 24.0;
const COL_GAP: f32 = 20.0;
const ROW_GAP: f32 = 12.0;
const TITLE_H: f32 = 22.0;
const TOOLBAR_H: f32 = 48.0;
const SECTION_GAP: f32 = 26.0;
const HEADER_H: f32 = 40.0;
/// Columns in a `--snap` still.
const SNAP_COLS: usize = 3;

/// A looping cell: hidden for `LEAD_S`, then its steps, then hidden again for `TAIL_S` (its
/// exit included), so it's gone for well under half a second a loop.
const LEAD_S: f64 = 0.15;
const HOLD_S: f64 = 2.2;
const TAIL_S: f64 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    PostRecord,
    States,
    Delivery,
    Motion,
}

impl Section {
    const ALL: [Section; 4] = [Self::PostRecord, Self::States, Self::Delivery, Self::Motion];

    fn title(self) -> &'static str {
        match self {
            Self::PostRecord => "After recording: today vs prototypes",
            Self::States => "States",
            Self::Delivery => "Delivery (the Completed labels the app shows)",
            Self::Motion => "Motion",
        }
    }

    fn caption(self) -> &'static str {
        match self {
            Self::PostRecord => "f–h and b–e are lab-only drawings (proto.rs): no routing behind them.",
            Self::States => "",
            Self::Delivery => {
                "No “failed” state: a send that fails is queued and retried, which shows “couldn't reach”."
            }
            Self::Motion => "",
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

/// One cell: what it shows held, and its loop.
#[derive(Clone, Debug)]
struct Def {
    section: Section,
    title: String,
    look: Look,
    clock: &'static str,
    /// `Surface::routes`, for a finish that completes with route chips.
    routes: Vec<String>,
    script: Vec<(f64, Act)>,
    period: f64,
    loops: bool,
    panel_h: f32,
}

impl Def {
    /// `pres` entering from hidden, resting, and leaving.
    fn state(section: Section, title: impl Into<String>, pres: Pres) -> Self {
        Self {
            section,
            title: title.into(),
            look: Look::Real(pres.clone()),
            clock: "0:07",
            routes: Vec::new(),
            script: vec![(LEAD_S, Act::Show(pres)), (LEAD_S + HOLD_S, Act::Hide)],
            period: LEAD_S + HOLD_S + TAIL_S,
            loops: false,
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
            section: Section::PostRecord,
            title: proto.title().into(),
            look: Look::Proto(proto),
            clock: "0:07",
            routes: Vec::new(),
            script,
            period: end + TAIL_S,
            loops: false,
            panel_h: proto.panel_h(),
        }
    }

    /// Steps at seconds after the lead-in; held, it shows `hold`. It leaves after its last step.
    fn motion(title: impl Into<String>, hold: Pres, steps: Vec<(f64, Act)>) -> Self {
        let last = steps.last().map_or(0.0, |(at, _)| *at);
        let mut script: Vec<(f64, Act)> = steps.into_iter().map(|(at, act)| (LEAD_S + at, act)).collect();
        let ends_hidden = matches!(script.last(), Some((_, Act::Hide)));
        let mut end = LEAD_S + last;
        if !ends_hidden {
            end += 1.4;
            script.push((end, Act::Hide));
        }
        Self {
            section: Section::Motion,
            title: title.into(),
            look: Look::Real(hold),
            clock: "0:07",
            routes: Vec::new(),
            script,
            period: end + TAIL_S,
            loops: false,
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
}

fn done(label: &str, routes: &[&str]) -> Pres {
    Pres::Completed { label: label.into(), routes: routes.iter().map(|r| (*r).to_owned()).collect() }
}

fn defs() -> Vec<Def> {
    use SelectionMode::{Area, Iphone, Screen, Window};
    use Section::{Delivery, PostRecord, States};
    let picker = |mode, has_iphone| Pres::Selecting { mode, has_iphone };
    let recording = |chips, has_iphone| Pres::Recording { chips, has_iphone };
    let long_title = "OpenCode Desktop · Problems board rewrite";
    let long_chip = "Scrawl · Rune pill lab follow-ups";

    let mut defs: Vec<Def> = Proto::NEW.map(Def::proto).into();
    defs.push(Def::state(PostRecord, "a · Today: Copied + Send to chips", done("Copied", &["Send to CONCLAVE", "HEX"])));
    defs.extend(Proto::OLD.map(Def::proto));
    defs.extend([
        Def::state(States, "Picker · Screen", picker(Screen, true)),
        Def::state(States, "Picker · Area (Record)", picker(Area, true)),
        Def::state(States, "Picker · Window", picker(Window, true)),
        Def::state(States, "Picker · iPhone", picker(Iphone, true)),
        Def::state(States, "Picker · no iPhone cabled", picker(Screen, false)),
        Def::state(States, "Recording + switch chips", recording(true, true)),
        Def::state(States, "Recording + chips · no iPhone", recording(true, false)),
        Def::state(States, "Recording", recording(false, true)),
        Def::state(States, "Recording · 12:34", recording(false, true)).clock("12:34"),
        Def::state(States, "Processing", Pres::Processing),
        Def::state(States, "Copied", done("Copied", &[])),
        Def::state(States, "From iPhone", done("From iPhone", &[])),
        Def::state(States, "3 from iPhone", done("3 from iPhone", &[])),
        Def::state(Delivery, "Offer · one route", done("Copied", &["Send to CONCLAVE"])),
        Def::state(Delivery, "Offer · two routes", done("Copied", &["Send to CONCLAVE", "HEX"])),
        Def::state(Delivery, "Sending (3 s to undo)", done("Sending to CONCLAVE…", &["Send to HEX"])),
        Def::state(Delivery, "Sent", done("Sent to CONCLAVE", &[])),
        Def::state(Delivery, "Queued (couldn't reach)", done("Copied · couldn't reach CONCLAVE", &[])),
        Def::state(
            Delivery,
            "Long names, truncated as the app does",
            Pres::Completed {
                label: format!("Sending to {}…", island_title(long_title)),
                routes: vec![format!("Send to {}", chip_title(long_chip))],
            },
        ),
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
            "Finish → Processing → Copied + routes",
            done("Copied", &["Send to CONCLAVE", "HEX"]),
            vec![(0.0, Act::Show(recording(false, true))), (0.9, Act::Finish), (3.4, Act::Hide)],
        )
        .routes(&["Send to CONCLAVE", "HEX"]),
        Def::motion(
            "Sending… → Sent",
            done("Sent to CONCLAVE", &[]),
            vec![
                (0.0, Act::Show(done("Sending to CONCLAVE…", &["Send to HEX"]))),
                (1.6, Act::Show(done("Sent to CONCLAVE", &[]))),
            ],
        ),
        Def::motion(
            "Cancel",
            recording(true, true),
            vec![(0.0, Act::Show(recording(true, true))), (1.2, Act::Hide)],
        ),
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
    /// Bumped to remount the cell's capsule: a held state lands at rest, and an entry lands its
    /// origin pose without animating from the last run (the app's `Tx::instant`, per cell).
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
        let looping = def.loops;
        let mut cell =
            Self { def, looping, surface, live, shown: false, frame: 0, epoch: 0, start: t, next: 0, overflows: false };
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
    fn from_env() -> Self {
        match std::env::var("SCRAWL_LAB_BACKDROP").unwrap_or_default().trim().to_ascii_lowercase().as_str() {
            "light" | "white" => Self::Light,
            "busy" | "screenshot" => Self::Busy,
            _ => Self::Dark,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Busy => "busy",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Busy,
            Self::Busy => Self::Dark,
        }
    }

    fn light(self) -> bool {
        self == Self::Light
    }
}

pub struct Grid {
    cells: Vec<Cell>,
    /// A headless still: everything rests, nothing plays.
    still: bool,
    /// Lab seconds: real seconds × `scale`, stopped while paused. Every cell's script runs on
    /// it; Rune's springs run at `scale`.
    t: f64,
    last: Option<f64>,
    /// Rune's clock for the whole window, toolbar and scrolling included, so it never goes to
    /// 0: a stopped scene clock freezes every exit and colour change, and removed backdrops and
    /// button states stay on screen. Pause stops the scripts instead.
    scale: f32,
    paused: bool,
    /// What the lab clock runs at this frame (0 paused).
    rate: f32,
    backdrop: Backdrop,
    screenshot: Option<Pixels>,
    screenshot_from: Option<PathBuf>,
    tune: Tune,
}

impl Grid {
    fn new(still: bool) -> Self {
        if !still && !headless_film() {
            write_lab_file();
        }
        motion::poll_tune();
        style::poll_lab_file();
        let tune = motion::tune();
        let (screenshot, screenshot_from) = match screenshot() {
            Some((pixels, path)) => (Some(pixels), Some(path)),
            None => (None, None),
        };
        let cells = defs()
            .into_iter()
            .map(|mut def| {
                if still {
                    def.loops = false;
                } else if headless_film() {
                    def.loops = true;
                }
                Cell::new(def, 0.0, !still)
            })
            .collect();
        Self {
            cells,
            still,
            t: 0.0,
            last: None,
            scale: tune.time_scale.clamp(0.05, 4.0),
            paused: false,
            rate: if still { 0.0 } else { tune.time_scale },
            backdrop: Backdrop::from_env(),
            screenshot,
            screenshot_from,
            tune,
        }
    }

    /// Every looping cell back to resting on its state; held ones stay as they are.
    fn hold_all(&mut self) {
        dbg_log("hold all");
        let t = self.t;
        for cell in self.cells.iter_mut().filter(|c| c.looping) {
            cell.looping = false;
            cell.reset(t);
        }
    }

    fn toggle(&mut self, i: usize) {
        dbg_log(&format!("toggle {i}"));
        let t = self.t;
        if let Some(cell) = self.cells.get_mut(i) {
            cell.looping = !cell.looping;
            cell.reset(t);
        }
    }

    fn restart(&mut self) {
        let t = self.t;
        for cell in &mut self.cells {
            cell.reset(t);
        }
    }

    fn speed(&mut self) {
        self.scale = if self.scale > 0.5 { 0.25 } else { 1.0 };
    }

    /// Rune's clock for the window: never 0 while it's live (see `scale`).
    fn scene_scale(&self) -> f32 {
        if self.still { 0.0 } else { self.scale }
    }

    /// Stops every loop where it is (springs already moving settle), or starts them again.
    fn pause(&mut self) {
        dbg_log("pause");
        self.paused = !self.paused;
    }

    fn press(&mut self, i: usize, id: ElemId) {
        let t = self.t;
        if let Some(cell) = self.cells.get_mut(i) {
            cell.press(id, t);
        }
    }
}

fn poll(g: &mut Grid, now: f64, tx: &mut Tx) -> bool {
    if g.still {
        return false;
    }
    let mut changed = false;
    motion::poll_tune();
    let tune = motion::tune();
    if tune != g.tune {
        if (tune.time_scale - g.tune.time_scale).abs() > 1e-6 {
            g.scale = tune.time_scale.clamp(0.05, 4.0);
        }
        g.tune = tune;
        for cell in &mut g.cells {
            cell.surface.tune = tune;
        }
        changed = true;
    }
    if style::poll_lab_file() {
        // New widths and type: every cell lays out again from scratch.
        g.restart();
        changed = true;
    }
    let dt = g.last.map_or(0.0, |last| (now - last).max(0.0));
    g.last = Some(now);
    let rate = if g.paused { 0.0 } else { g.scale };
    if rate != g.rate {
        g.rate = rate;
        changed = true;
    }
    g.t += dt * f64::from(rate);
    let t = g.t;
    for cell in &mut g.cells {
        changed |= cell.advance(t, rate);
    }
    if rate > 0.0
        && let Some(due) = g.cells.iter().filter_map(Cell::next_due).min_by(f64::total_cmp)
    {
        tx.wake_after(Duration::from_secs_f64(((due - t) / f64::from(rate)).max(0.0)));
    }
    // The tune and lab files are polled at most four times a second.
    tx.wake_after(Duration::from_millis(250));
    changed
}

fn on_key(g: &mut Grid, key: &KeyEvent, _tx: &mut Tx) -> bool {
    let Key::Text(text) = &key.key else { return false };
    match text.to_ascii_lowercase().as_str() {
        " " => g.pause(),
        "s" => g.speed(),
        "h" => g.hold_all(),
        "b" => g.backdrop = g.backdrop.next(),
        "r" => g.restart(),
        _ => return false,
    }
    true
}

// ─── View ────────────────────────────────────────────────────────────────────────────────────

fn ink(light: bool, level: f32) -> Color {
    if light { srgb(0.11, 0.11, 0.13, level) } else { srgb(0.96, 0.96, 0.97, level) }
}

fn font(size: f32, weight: f32) -> TextStyle {
    TextStyle::new(size, weight)
}

fn accent() -> Color {
    srgb(0.231, 0.361, 0.965, 1.0)
}

/// A toolbar button: one action per press. `on` fills it with the accent.
fn button(label: &str, on: bool, light: bool, press: impl Fn(&mut Grid) + 'static) -> El {
    let (fill, hover, fg) = if on {
        (accent(), accent(), Color::WHITE)
    } else {
        (ink(light, 0.07), ink(light, 0.13), ink(light, 0.8))
    };
    row().key(format!("tb-{label}")).center().h(26.0).px(10.0).radius(7.0).fill(fill).hover(hover)
        .child(text(label).font(font(12.0, if on { 600.0 } else { 500.0 })).color(fg))
        .on_press(press)
}

/// Buttons side by side, one of them on.
fn segmented(buttons: impl IntoIterator<Item = El>, light: bool) -> El {
    let mut seg = row().center_y().gap(2.0).padding(2.0).radius(9.0).fill(ink(light, 0.05));
    for b in buttons {
        seg = seg.child(b);
    }
    seg
}

fn toolbar(g: &Grid) -> El {
    let light = g.backdrop.light();
    let looping = g.cells.iter().filter(|c| c.looping).count();
    let fast = g.scale > 0.5;
    let play = if g.paused { "▶ Play" } else { "❚❚ Pause" };
    let hold = if looping == 0 { "Hold all".to_owned() } else { format!("Hold all ({looping} looping)") };
    let mut bar = row().center_y().gap(8.0).h(TOOLBAR_H).px(PAD).w_full()
        .fill(if light { srgb(0.95, 0.95, 0.96, 0.92) } else { srgb(0.07, 0.07, 0.08, 0.92) })
        .child(text("Pill lab").font(font(14.0, 650.0)).color(ink(light, 0.95)))
        .child(el().w(8.0))
        .child(button(play, g.paused, light, Grid::pause))
        .child(segmented(
            [
                button("1×", fast, light, |g: &mut Grid| g.scale = 1.0),
                button("0.25×", !fast, light, |g: &mut Grid| g.scale = 0.25),
            ],
            light,
        ))
        .child(el().w(8.0));
    if looping > 0 {
        bar = bar.child(button(&hold, false, light, Grid::hold_all));
    }
    bar = bar.child(button("Restart", false, light, Grid::restart)).child(el().w(8.0)).child(segmented(
        [Backdrop::Dark, Backdrop::Light, Backdrop::Busy].map(|b| {
            button(b.name(), g.backdrop == b, light, move |g: &mut Grid| {
                dbg_log(&format!("backdrop {b:?}"));
                g.backdrop = b;
            })
        }),
        light,
    ));
    let file = lab_path().map(|p| p.display().to_string().replace(&home(), "~")).unwrap_or_default();
    bar.child(el().grow(1.0)).child(
        text(format!("click a title to loop it · space pause · s speed · h hold all · b backdrop · knobs: {file} · springs: scrawl tune"))
            .font(font(11.0, 450.0))
            .color(ink(light, 0.5)),
    )
}

fn dbg_log(what: &str) {
    if std::env::var_os("SCRAWL_PILL_GRID_LOG").is_some() {
        eprintln!("pill lab: {what}");
    }
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

fn presence(surface: &Surface) -> Transition {
    Transition::blur(1.5).combined(Transition::scale(0.96)).springs(SpringPair::new(surface.presence(), surface::PRESENCE_EXIT))
}

fn cell_view(g: &Grid, i: usize, cell: &Cell, cx: &mut Cx<Grid>) -> El {
    let light = g.backdrop.light();
    let busy = g.backdrop == Backdrop::Busy;
    let badge = if cell.looping { "LOOP" } else { "HOLD" };
    let title = row().center_y().gap(7.0).h(TITLE_H).px(if busy { 7.0 } else { 0.0 }).radius(6.0)
        .fill(if busy { srgb(0.05, 0.05, 0.06, 0.72) } else { Color::TRANSPARENT })
        .child(
            row().center().h(15.0).px(5.0).radius(4.0)
                .fill(if cell.looping { srgb(0.231, 0.361, 0.965, 0.85) } else { ink(light && !busy, 0.12) })
                .child(text(badge).font(font(9.0, 700.0)).color(if cell.looping {
                    Color::WHITE
                } else {
                    ink(light && !busy, 0.7)
                })),
        )
        .child(text(&cell.def.title).font(font(12.5, 560.0)).color(ink(light && !busy, 0.9)))
        .children(cell.overflows.then(|| {
            text("wider than the panel: clipped in the app").font(font(11.5, 600.0)).color(srgb(0.96, 0.42, 0.36, 1.0))
        }))
        .hover(ink(light && !busy, 0.06))
        .key(format!("title-{i}"))
        .on_press(move |g: &mut Grid| g.toggle(i));
    let panel = match cell.def.look {
        Look::Real(_) => capsule(&cell.surface, &cell.live, cx, move |g: &mut Grid, id| g.press(i, id)),
        Look::Proto(p) => proto::capsule(p, cell.frame, &cell.live.meter, cell.shown, presence(&cell.surface)),
    };
    col().key(format!("cell-{i}")).gap(4.0)
        .child(row().child(title))
        .child(
            el().key(format!("panel-{}", cell.epoch)).size(PANEL_W, cell.def.panel_h).radius(10.0)
                .border(1.0, ink(light, if busy { 0.0 } else { 0.07 }))
                // The capsule's window is the panel: what doesn't fit there is cut off.
                .clip_rounded(10.0)
                .child(panel),
        )
}

fn view(g: &Grid, cx: &mut Cx<Grid>) -> El {
    let light = g.backdrop.light();
    let busy = g.backdrop == Backdrop::Busy;
    let mut sections = col().gap(SECTION_GAP).padding(PAD);
    for section in Section::ALL {
        let mut cells = row().flex_wrap().gaps(ROW_GAP, COL_GAP);
        if g.still {
            cells = cells.w(grid_width(SNAP_COLS) - 2.0 * PAD);
        }
        for (i, cell) in g.cells.iter().enumerate().filter(|(_, c)| c.def.section == section) {
            cells = cells.child(cell_view(g, i, cell, cx));
        }
        let caption = section.caption();
        let mut header = row().center_y().gap(12.0).h(HEADER_H - 12.0).self_start().px(if busy { 10.0 } else { 0.0 }).radius(8.0)
            .fill(if busy { srgb(0.05, 0.05, 0.06, 0.72) } else { Color::TRANSPARENT })
            .child(text(section.title()).font(font(16.0, 650.0)).color(ink(light, 0.95)));
        if !caption.is_empty() {
            header = header.child(text(caption).font(font(12.0, 450.0)).color(ink(light, 0.55)));
        }
        sections = sections.child(col().gap(12.0).child(header).child(cells));
    }
    if busy && g.screenshot.is_none() {
        sections = sections.child(text("No screenshot: set SCRAWL_LAB_SCREENSHOT to a .png (default: the latest capture's first frame).").font(font(12.0, 500.0)).color(ink(false, 0.7)));
    }
    let body = if g.still {
        sections
    } else {
        col().key("scroll").grow(1.0).min_h(0.0).w_full().scroll(ScrollSpec { stick_bottom: false }).child(sections)
    };
    let backdrop = match (g.backdrop, &g.screenshot) {
        (Backdrop::Busy, Some(pixels)) => el().absolute(0.0, 0.0).w_full().h_full().pixels_fit(pixels, SurfaceFit::Fill),
        (Backdrop::Light, _) => el().absolute(0.0, 0.0).w_full().h_full().fill(srgb(0.957, 0.957, 0.965, 1.0)),
        (Backdrop::Busy, None) => el().absolute(0.0, 0.0).w_full().h_full().fill(srgb(0.45, 0.47, 0.52, 1.0)),
        (Backdrop::Dark, _) => el().absolute(0.0, 0.0).w_full().h_full().fill(srgb(0.059, 0.067, 0.090, 1.0)),
    };
    // The lab's speed drives Rune's springs too (the capsules set it to their surfaces' 1×).
    cx.time_scale(g.scene_scale());
    el().w_full().h_full().child(backdrop).child(col().w_full().h_full().child(toolbar(g)).child(body))
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

fn grid_width(cols: usize) -> f32 {
    2.0 * PAD + cols as f32 * PANEL_W + (cols.saturating_sub(1)) as f32 * COL_GAP
}

/// The whole grid's height at `cols` columns, for a still that shows every cell.
fn grid_height(cols: usize) -> f32 {
    let defs = defs();
    let sections: f32 = Section::ALL
        .iter()
        .map(|s| {
            let heights: Vec<f32> = defs.iter().filter(|d| d.section == *s).map(|d| TITLE_H + 4.0 + d.panel_h).collect();
            let rows: Vec<f32> = heights.chunks(cols.max(1)).map(|row| row.iter().copied().fold(0.0, f32::max)).collect();
            HEADER_H + rows.iter().sum::<f32>() + rows.len().saturating_sub(1) as f32 * ROW_GAP
        })
        .sum();
    TOOLBAR_H + 2.0 * PAD + sections + SECTION_GAP * (Section::ALL.len() - 1) as f32
}

fn init() -> Grid {
    Grid::new(false)
}

fn spec(size: (f32, f32), fonts: &'static [&'static [u8]]) -> AppSpec<Grid> {
    AppSpec {
        title: "Scrawl pill lab",
        size,
        min_size: Some((PANEL_W + 2.0 * PAD, 400.0)),
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

/// Filming headless (`--snap grid.mp4`): every cell loops.
fn headless_film() -> bool {
    std::env::var_os("SCRAWL_PILL_GRID_FILM").is_some()
}

/// `--snap DIR`: one still per backdrop, `grid-<backdrop>.png`.
fn snap_all(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let exe = std::env::current_exe()?;
    for backdrop in [Backdrop::Dark, Backdrop::Light, Backdrop::Busy] {
        let out = dir.join(format!("grid-{}.png", backdrop.name()));
        let status = std::process::Command::new(&exe)
            .args(["lab", "pill", "grid", "--rune", "--snap"])
            .arg(&out)
            .env("SCRAWL_LAB_BACKDROP", backdrop.name())
            .env_remove("SCRAWL_MOTION_TRACE")
            .status()?;
        anyhow::ensure!(status.success(), "snap {} failed ({status})", backdrop.name());
        println!("{}", out.display());
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
        let size = (grid_width(SNAP_COLS), 900.0);
        rune::run_windows_with_args(RunOptions { accessory: true }, args, move |_| {
            vec![WindowSpec::from_app("grid", App::new(spec(size, fonts))).state(Grid::new(false))]
        });
        return Ok(());
    }
    if let Some(out) = snap.as_ref().filter(|p| p.extension().is_some_and(|e| e == "mp4"))
    {
        // Headless, on Rune's virtual clock: every cell looping, at 1× scale.
        unsafe { std::env::set_var("SCRAWL_PILL_GRID_FILM", "1") };
        let size = (grid_width(SNAP_COLS), grid_height(SNAP_COLS));
        let seconds = std::env::var("SCRAWL_PILL_GRID_SECONDS").unwrap_or_else(|_| "8".into());
        let args = ["--video", &out.to_string_lossy(), "--duration", &seconds, "--fps", "30", "--scale", "1"]
            .map(str::to_owned);
        rune::run_windows_with_args(RunOptions { accessory: true }, args, move |_| {
            vec![WindowSpec::from_app("grid", App::new(spec(size, fonts))).state(Grid::new(false))]
        });
        return Ok(());
    }
    if let Some(out) = snap {
        let size = (grid_width(SNAP_COLS), grid_height(SNAP_COLS));
        let args = ["--snap".to_owned(), out.to_string_lossy().into_owned()];
        rune::run_windows_with_args(RunOptions { accessory: true }, args, move |_| {
            let grid = Grid::new(true);
            if let Some(from) = &grid.screenshot_from {
                eprintln!("pill lab: screenshot backdrop {}", from.display());
            }
            vec![WindowSpec::from_app("grid", App::new(spec(size, fonts))).state(grid)]
        });
        return Ok(());
    }
    rune::run_windows_with_args(RunOptions { accessory: false }, Vec::new(), move |displays| {
        // As many columns as the main display holds (up to three), and nearly its height.
        let (w, h) = displays
            .iter()
            .find(|d| d.primary)
            .or(displays.first())
            .map_or((1440.0, 900.0), |d| (d.size.0 as f32, d.size.1 as f32));
        let cols = (((w - 2.0 * PAD + COL_GAP) / (PANEL_W + COL_GAP)).floor() as usize).clamp(1, 3);
        let size = (grid_width(cols), (h - 120.0).clamp(500.0, grid_height(cols)));
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
            assert!(matches!(last.1, Act::Hide), "{}", def.title);
            assert!(last.0 + TAIL_S <= def.period, "{}", def.title);
            assert!(def.script.windows(2).all(|w| w[0].0 <= w[1].0), "{}", def.title);
        }
    }

    #[test]
    fn a_looping_state_is_gone_for_under_half_a_second() {
        for def in defs().into_iter().filter(|d| d.section != Section::Motion) {
            let shown: f64 = def.script.first().map_or(0.0, |(at, _)| *at);
            let hidden_mid: f64 = def
                .script
                .windows(2)
                .filter(|w| matches!(w[0].1, Act::Hide))
                .map(|w| w[1].0 - w[0].0)
                .sum();
            let last = def.script.last().map_or(0.0, |(at, _)| *at);
            let gap = shown + (def.period - last);
            assert!(gap <= 0.5 && hidden_mid <= 0.5, "{}: hidden {gap:.2} s", def.title);
        }
    }

    #[test]
    fn it_starts_held_and_play_pause_and_titles_touch_only_their_own_cells() {
        fonts();
        let mut g = Grid::new(false);
        assert!(g.cells.iter().all(|c| !c.looping && c.shown), "everything held, at rest");
        let epochs = |g: &Grid| g.cells.iter().map(|c| c.epoch).collect::<Vec<_>>();
        let before = epochs(&g);
        g.pause();
        assert!(g.scene_scale() > 0.0, "paused, the toolbar, backdrop and scrolling still move");
        g.pause();
        assert_eq!(epochs(&g), before, "pause and play leave every cell where it is");
        g.toggle(3);
        assert_eq!(g.cells.iter().filter(|c| c.looping).count(), 1);
        assert!(g.cells[3].looping);
        g.toggle(3);
        assert!(g.cells.iter().all(|c| !c.looping), "a second click holds it again");
        g.toggle(1);
        g.toggle(5);
        g.hold_all();
        assert!(g.cells.iter().all(|c| !c.looping));
    }

    #[test]
    fn a_held_cell_rests_on_its_state_and_a_looping_one_plays_it() {
        fonts();
        let def = Def::state(Section::States, "Copied", done("Copied", &[]));
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
}
