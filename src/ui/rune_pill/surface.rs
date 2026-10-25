//! The capsule's state, decided in Rust: what it shows (`Pres`, the same enum the Metal capsule
//! takes), where everything rests (`layout`, the SwiftUI capsule's layout), and how each change
//! moves (springs from `motion::Tune`, keyed on the kind of change). The view only draws what is
//! here; Rune animates between the targets.
//!
//! Timing that a spring can't express lives here too, as pending steps the app's poll hook runs:
//! narrowing, the walls start `SHELL_WAIT_S` after the content so leaving chips are already
//! fading when a wall reaches them (the SwiftUI capsule's delay), and an entering chip mounts
//! once the walls have room for it (predicted from the same analytic spring Rune animates the
//! walls on). The view masks the content by the capsule, and content entering over content that
//! leaves crossfades with it (its `dissolve` scope).

use rune::Spring;

use crate::native::pill::model::{
    CHIP_H, CX, ElemId, ISLAND_H, ISLAND_INSET, Metrics, Pres, TOP,
};
use crate::ui::SelectionMode;
use crate::ui::lab::PROCESSING_S;
use crate::ui::motion::Tune;

/// Taller than the capsule so its drop shadow fades out inside the window (the SwiftUI panel).
pub const PANEL_H: f32 = 112.0;
/// The red dot's ink spans 12…20 of the lead's 32 pt square; the timer starts 8 pt after it.
pub const LEAD_TEXT_X: f32 = 28.0;
pub const PROCESSING_TEXT_X: f32 = 31.5;
const PROCESSING_TRAIL: f32 = 8.5;
/// The check's ink spans 12…20, as the recording dot's does; its label starts where the timer does.
pub const DONE_TEXT_X: f32 = LEAD_TEXT_X;
/// The capsule unfolds from this fraction of its height, anchored on its top edge.
const TOP_FROM: f32 = 0.7;
const SHELL_WAIT_S: f32 = 0.06;
/// Content leaves on the SwiftUI capsule's exit, `pill.exit` (0.20 s) × 0.85: exits run
/// shorter than entrances.
pub const PRESENCE_EXIT: Spring = Spring::new(0.20 * 0.85, 0.0);
const ENTRY_CONTENT_DELAY_S: f32 = 0.016;
const STAGGER_S: f32 = 0.008;
/// Opening, sliding content takes this many times the walls' duration (`pill.follow`).
const FOLLOW: f32 = 1.15;
/// The selection highlight trails a mode change in place.
const HIGHLIGHT: Spring = Spring::new(0.25, 0.0);

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: ElemId,
    pub rect: [f32; 4],
    pub active: bool,
    pub enabled: bool,
}

/// Widths of the Completed island's changing labels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Labels {
    pub completed: f32,
    pub routes: [f32; 2],
}

impl Labels {
    pub fn of(pres: &Pres) -> Self {
        let mut labels = Labels::default();
        if let Pres::Completed { label, routes } = pres {
            labels.completed = super::measure(label, true);
            for (i, route) in routes.iter().take(2).enumerate() {
                labels.routes[i] = super::measure(route, false);
            }
        }
        labels
    }
}

const fn group_of(id: ElemId) -> u8 {
    match id {
        ElemId::Lead | ElemId::DoneLead | ElemId::Processing => 0,
        ElemId::ChipArea | ElemId::ChipWindow | ElemId::ChipScreen | ElemId::ChipIphone => 1,
        ElemId::Close => 3,
        _ => 2,
    }
}

/// The resting capsule `[x, y, w, h]` and its items for `pres`, in panel points.
///
/// Every state is centred on `CX`, except the recording capsule with its chips: it centres the
/// chip row (`Area … ✕`) where the picker puts the same row and hangs the lead off its left, so
/// switching to Area or Window moves no chip.
///
/// Gaps, chip padding and the lead's trail come from `style()`: the capsule's own values unless
/// the pill lab is tuning them.
pub fn layout(pres: &Pres, clock: &str, labels: Labels, m: &Metrics) -> ([f32; 4], Vec<Item>) {
    let style = super::style::style();
    let chip_width = |label: f32| label + 2.0 * style.chip_px;
    // After the timer, equal to the dot's 12 pt inset.
    let lead_trail = style.lead_trail;
    let mut raw: Vec<(ElemId, f32, bool, bool)> = Vec::with_capacity(8);
    let mut held_row = false;
    match pres {
        Pres::Hidden => return ([CX - 48.0, TOP, 96.0, ISLAND_H], Vec::new()),
        Pres::Selecting { mode, has_iphone } => {
            for &seg in SelectionMode::available(*has_iphone) {
                raw.push((ElemId::from_mode(seg), m.segment_width(seg), seg == *mode, true));
            }
            if mode.shows_record() {
                raw.push((ElemId::Record, chip_width(m.record), false, true));
            }
            raw.push((ElemId::Close, CHIP_H, false, true));
        }
        Pres::Recording { chips, has_iphone } => {
            raw.push((ElemId::Lead, LEAD_TEXT_X + m.clock_width(clock) + lead_trail, false, true));
            if *chips {
                held_row = true;
                for &seg in SelectionMode::available(*has_iphone) {
                    let active = seg == SelectionMode::Screen;
                    raw.push((ElemId::from_mode(seg), m.segment_width(seg), active, true));
                }
                raw.push((ElemId::Close, CHIP_H, false, true));
            } else {
                raw.push((ElemId::Cancel, chip_width(m.cancel), false, true));
                raw.push((ElemId::Finish, chip_width(m.finish), false, true));
            }
        }
        Pres::Processing => {
            let w = PROCESSING_TEXT_X + m.processing + PROCESSING_TRAIL;
            raw.push((ElemId::Processing, w, false, false));
        }
        Pres::Completed { routes, .. } => {
            raw.push((ElemId::DoneLead, DONE_TEXT_X + labels.completed + lead_trail, false, true));
            raw.push((ElemId::Paste, chip_width(m.paste), false, true));
            for (i, id) in ElemId::ROUTES.into_iter().enumerate().take(routes.len()) {
                raw.push((id, chip_width(labels.routes[i]), false, true));
            }
            raw.push((ElemId::Inspect, chip_width(m.inspect), false, true));
            raw.push((ElemId::Close, CHIP_H, false, true));
        }
    }
    let gap_after = |i: usize| {
        if group_of(raw[i].0) == group_of(raw[i + 1].0) {
            style.chip_gap
        } else {
            style.group_gap
        }
    };
    let span = |from: usize| -> f32 {
        (from..raw.len()).map(|i| raw[i].1).sum::<f32>()
            + (from..raw.len().saturating_sub(1)).map(gap_after).sum::<f32>()
    };
    let (l, r) = if held_row {
        let row = span(1);
        (CX - row * 0.5 - gap_after(0) - raw[0].1 - ISLAND_INSET, CX + row * 0.5 + ISLAND_INSET)
    } else {
        let w = span(0) + 2.0 * ISLAND_INSET;
        (CX - w * 0.5, CX + w * 0.5)
    };
    let mut x = l + ISLAND_INSET;
    let mut items = Vec::with_capacity(raw.len());
    for (i, &(id, w, active, enabled)) in raw.iter().enumerate() {
        items.push(Item { id, rect: [x, TOP + ISLAND_INSET, w, CHIP_H], active, enabled });
        if i + 1 < raw.len() {
            x += w + gap_after(i);
        }
    }
    ([l, TOP, r - l, ISLAND_H], items)
}

/// Where the capsule enters from and leaves to: its own width, `TOP_FROM` of its height.
fn pose(rest: [f32; 4]) -> [f32; 4] {
    [rest[0], rest[1], rest[2], rest[3] * TOP_FROM]
}

fn spring(s: crate::ui::Spring) -> Spring {
    Spring::new(s.response.clamp(0.04, 3.0), s.bounce.clamp(0.0, 0.9))
}

/// One edge of the capsule, followed on the same closed-form spring Rune animates it on, so an
/// entering chip can wait for the wall to pass it.
#[derive(Clone, Copy, Debug)]
struct Wall {
    x0: f32,
    v0: f32,
    target: f32,
    t0: f64,
    spring: Spring,
}

impl Wall {
    fn at_rest(x: f32) -> Self {
        Self { x0: x, v0: 0.0, target: x, t0: 0.0, spring: HIGHLIGHT }
    }

    fn at(&self, t: f64) -> (f32, f32) {
        let dt = (t - self.t0).max(0.0) as f32;
        if (self.x0 - self.target).abs() < 1e-4 && self.v0.abs() < 1e-4 {
            return (self.target, 0.0);
        }
        self.spring.sample(self.x0, self.v0, self.target, dt)
    }

    fn to(&mut self, t: f64, target: f32, spring: Spring) {
        let (x, v) = self.at(t);
        *self = Self { x0: x, v0: v, target, t0: t, spring };
    }

    fn snap(&mut self, t: f64, x: f32) {
        *self = Self { x0: x, v0: 0.0, target: x, t0: t, spring: self.spring };
    }
}

/// Something mounted in the capsule. A key carries a generation, so a chip that leaves and comes
/// back is a new element entering in place rather than the old one flying back from where it
/// faded.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub id: ElemId,
    pub key: String,
    pub rect: [f32; 4],
    pub active: bool,
    pub enabled: bool,
    /// The spring for this element's next layout change.
    pub motion: Spring,
}

#[derive(Clone, Debug)]
enum Step {
    /// The capsule's walls narrowing, after the content has started (`SHELL_WAIT_S`).
    Shell([f32; 4], Spring),
    /// The second frame of an entry: from the origin pose to rest.
    Entry,
    Mount(Shown),
    /// The pretend recording finished processing.
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    ChipsOpen,
    ChipsClose,
    Switch,
    Phase,
}

pub struct Surface {
    pub pres: Pres,
    pub clock: String,
    pub reduced: bool,
    pub tune: Tune,
    pub metrics: Metrics,
    labels: Labels,
    /// The capsule's resting rect, whether it shows, and the springs for its next change.
    pub shell: [f32; 4],
    pub shell_on: bool,
    pub shell_motion: Spring,
    pub shell_fade: Spring,
    pub items: Vec<Shown>,
    pub highlight: Option<Shown>,
    /// The next commit lands with no animation (an entry's origin pose).
    pub instant: bool,
    /// Counts the width morphs (each one lights the edge glow) and the last one's settle time.
    pub morphs: u32,
    pub morph_settle: f32,
    /// App events for the motion trace (`phase:*`, `lab:*`).
    pub events: Vec<String>,
    /// Session routes the Completed island offers ("Send to …"; state only for now).
    pub routes: Vec<String>,
    pending: Vec<(f64, u64, Step)>,
    target: Option<([f32; 4], Vec<Item>, Change)>,
    walls: [Wall; 3],
    generation: u64,
    seq: u64,
    /// Unscaled clock → animation time (the spring clock), re-anchored when the scale changes.
    anchor: (f64, f64),
    pub time_scale: f32,
}

impl Surface {
    pub fn new(tune: Tune, metrics: Metrics) -> Self {
        let shell = [CX - 60.0, TOP, 120.0, ISLAND_H];
        Self {
            pres: Pres::Hidden,
            clock: "0:00".into(),
            reduced: false,
            tune,
            metrics,
            labels: Labels::default(),
            shell: pose(shell),
            shell_on: false,
            shell_motion: spring(tune.island),
            shell_fade: spring(tune.island_scale),
            items: Vec::new(),
            highlight: None,
            instant: false,
            morphs: 0,
            morph_settle: 0.0,
            events: Vec::new(),
            routes: Vec::new(),
            pending: Vec::new(),
            target: None,
            walls: [Wall::at_rest(shell[0]), Wall::at_rest(shell[0] + shell[2]), Wall::at_rest(TOP + shell[3] * TOP_FROM)],
            generation: 0,
            seq: 0,
            anchor: (0.0, 0.0),
            time_scale: tune.time_scale.clamp(0.05, 8.0),
        }
    }

    /// `pres` at rest, as a first frame shows it (headless goldens).
    pub fn settled(mut self, pres: Pres) -> Self {
        self.labels = Labels::of(&pres);
        let (rest, items) = layout(&pres, &self.clock, self.labels, &self.metrics);
        self.shell_on = !matches!(pres, Pres::Hidden);
        self.shell = if self.shell_on { rest } else { pose(rest) };
        self.items = items.iter().map(|item| self.shown(item, spring(self.tune.island))).collect();
        self.highlight = self.highlight_for(&items, spring(self.tune.island));
        self.pres = pres;
        self
    }

    /// Animation seconds at clock `now`.
    pub fn anim(&self, now: f64) -> f64 {
        self.anchor.1 + (now - self.anchor.0) * f64::from(self.time_scale)
    }

    /// Clock seconds for `s` animation seconds.
    fn real(&self, s: f32) -> f64 {
        f64::from(s / self.time_scale)
    }

    pub fn set_time_scale(&mut self, now: f64, scale: f32) {
        let scale = scale.clamp(0.05, 8.0);
        if (scale - self.time_scale).abs() > 1e-6 {
            self.anchor = (now, self.anim(now));
            self.time_scale = scale;
        }
    }

    fn shown(&mut self, item: &Item, motion: Spring) -> Shown {
        self.generation += 1;
        Shown {
            id: item.id,
            key: format!("{}#{}", item.id.trace_key(), self.generation),
            rect: item.rect,
            active: item.active,
            enabled: item.enabled,
            motion,
        }
    }

    /// The fade an element enters with (it leaves on `PRESENCE_EXIT`).
    pub fn presence(&self) -> Spring {
        Spring::new((self.tune.reveal_enter_s * 1.6).clamp(0.05, 2.0), 0.0)
    }

    fn highlight_for(&mut self, items: &[Item], motion: Spring) -> Option<Shown> {
        let active = items.iter().find(|i| i.active)?;
        self.generation += 1;
        Some(Shown {
            id: active.id,
            key: format!("highlight#{}", self.generation),
            rect: active.rect,
            active: true,
            enabled: false,
            motion,
        })
    }

    /// The timer reads `clock`. True when the capsule re-lays out for it (a wider timer); the
    /// view draws the digits themselves from a `LiveText`.
    pub fn set_clock(&mut self, clock: &str, now: f64) -> bool {
        if self.clock == clock {
            return false;
        }
        let resize = self.clock.len() != clock.len()
            || self.metrics.clock_width(&self.clock) != self.metrics.clock_width(clock);
        self.clock = clock.to_owned();
        if resize && matches!(self.pres, Pres::Recording { .. }) {
            let same = self.pres.clone();
            self.transition(same, now, false);
        }
        resize
    }

    /// Shows `next`. Retargets keep every element's position and velocity (Rune's springs).
    pub fn set(&mut self, next: Pres, now: f64) {
        if next == self.pres {
            return;
        }
        let phase = |p: &Pres| match p {
            Pres::Hidden => "hidden",
            Pres::Selecting { .. } => "selecting",
            Pres::Recording { .. } => "recording",
            Pres::Processing => "processing",
            Pres::Completed { .. } => "completed",
        };
        if std::mem::discriminant(&next) != std::mem::discriminant(&self.pres) {
            self.events.push(format!("phase:{}", phase(&next)));
        }
        let prev = std::mem::replace(&mut self.pres, next);
        self.transition(prev, now, true);
    }

    /// Retargets from `prev` to `self.pres`.
    fn transition(&mut self, prev: Pres, now: f64, cancel_complete: bool) {
        let next = self.pres.clone();
        self.seq += 1;
        let t = self.anim(now);
        self.pending.retain(|(_, _, step)| !cancel_complete && matches!(step, Step::Complete));
        self.target = None;
        self.instant = false;

        if matches!(next, Pres::Hidden) {
            // Exit in place: the capsule settles back to its origin pose as it fades, and every
            // item leaves with it.
            let exit = Spring::new((self.tune.island_scale.response * 0.9).clamp(0.04, 3.0), 0.0);
            self.shell = pose(self.shell);
            self.shell_on = false;
            self.shell_motion = exit;
            self.shell_fade = exit;
            self.walls[2].to(t, self.shell[1] + self.shell[3], exit);
            self.items.clear();
            self.highlight = None;
            return;
        }

        self.labels = Labels::of(&next);
        let (rest, items) = layout(&next, &self.clock, self.labels, &self.metrics);
        if !self.shell_on || matches!(prev, Pres::Hidden) {
            // Entry: this commit puts the capsule at its origin pose, invisibly and instantly;
            // the next frame springs it to rest (`Step::Entry`).
            self.shell = pose(rest);
            self.shell_on = false;
            self.instant = true;
            self.items.clear();
            self.highlight = None;
            for (i, wall) in [rest[0], rest[0] + rest[2], self.shell[1] + self.shell[3]].into_iter().enumerate() {
                self.walls[i].snap(t, wall);
            }
            self.target = Some((rest, items, Change::Phase));
            // Due after this commit, never in the same poll.
            self.pending.push((now + 1e-4, self.seq, Step::Entry));
            return;
        }

        let change = match (&prev, &next) {
            (Pres::Recording { chips: false, .. }, Pres::Recording { chips: true, .. }) => Change::ChipsOpen,
            (Pres::Recording { chips: true, .. }, Pres::Recording { chips: false, .. }) => Change::ChipsClose,
            (Pres::Recording { .. }, Pres::Selecting { .. }) => Change::Switch,
            _ => Change::Phase,
        };
        let leaving = self.items.iter().any(|s| !items.iter().any(|i| i.id == s.id));
        self.items.retain(|s| items.iter().any(|i| i.id == s.id));
        if self.highlight.as_ref().is_some_and(|h| !items.iter().any(|i| i.active && i.id == h.id))
            && !items.iter().any(|i| i.active)
        {
            self.highlight = None;
        }
        let narrowing = rest[2] < self.shell[2] - 1.0;
        self.target = Some((rest, items, change));
        let wait = if narrowing && leaving && !self.reduced { SHELL_WAIT_S } else { 0.0 };
        self.apply_geometry(now, wait);
    }

    /// The walls (after `wait` animation seconds) and staying items move to the target; entering
    /// items are scheduled for when the walls have room for them.
    fn apply_geometry(&mut self, now: f64, wait: f32) {
        let Some((rest, items, change)) = self.target.take() else { return };
        let t = self.anim(now);
        let tune = self.tune;
        let opening = rest[2] >= self.shell[2] - 1.0;
        let (shell_spring, item_spring) = match change {
            Change::ChipsOpen => {
                let s = spring(tune.reveal_open);
                (s, Spring::new(s.duration * FOLLOW, 0.0))
            }
            Change::ChipsClose => {
                let s = spring(tune.reveal_close);
                (s, Spring::new(s.duration, 0.0))
            }
            Change::Switch => (spring(tune.switch_width), Spring::new(spring(tune.switch_slide).duration, 0.0)),
            Change::Phase => {
                let s = spring(tune.island);
                (s, Spring::new(s.duration * if opening { FOLLOW } else { 1.0 }, 0.0))
            }
        };
        let entry = !self.shell_on;
        if entry {
            let enter = spring(tune.island_scale);
            self.shell = rest;
            self.shell_motion = enter;
            self.shell_fade = Spring::new((enter.duration * 0.9).max(0.04), 0.0);
            self.walls[2].to(t, rest[1] + rest[3], enter);
        } else {
            if (rest[2] - self.shell[2]).abs() > 1.0 && !self.reduced {
                self.morphs = self.morphs.wrapping_add(1);
                self.morph_settle = shell_spring.duration;
            }
            let at = t + f64::from(wait);
            self.walls[0].to(at, rest[0], shell_spring);
            self.walls[1].to(at, rest[0] + rest[2], shell_spring);
            self.walls[2].to(at, rest[1] + rest[3], shell_spring);
            if wait <= 0.0 {
                self.move_shell(rest, shell_spring);
            } else {
                self.pending.push((now + self.real(wait), self.seq, Step::Shell(rest, shell_spring)));
            }
        }
        self.shell_on = true;

        for item in &items {
            if let Some(shown) = self.items.iter_mut().find(|s| s.id == item.id) {
                shown.motion = item_spring;
                shown.rect = item.rect;
                shown.active = item.active;
                shown.enabled = item.enabled;
            }
        }
        // The highlight rides its chip when the chip travels and trails a change of mode.
        let active = items.iter().find(|i| i.active).cloned();
        match (&mut self.highlight, &active) {
            (Some(h), Some(a)) => {
                h.motion = if h.id == a.id { item_spring } else { HIGHLIGHT };
                h.id = a.id;
                h.rect = a.rect;
            }
            (h, None) => *h = None,
            (None, Some(_)) => {}
        }

        let presence = self.presence();
        let mut rank = 0;
        for item in &items {
            if self.items.iter().any(|s| s.id == item.id) {
                continue;
            }
            let room = if entry {
                ENTRY_CONTENT_DELAY_S + self.room_delay(item.rect, t, true)
            } else {
                self.room_delay(item.rect, t, false)
            };
            let delay = room + STAGGER_S * rank as f32;
            rank += 1;
            let shown = self.shown(item, presence);
            if item.active && self.highlight.is_none() {
                let mut h = self.highlight_for(std::slice::from_ref(item), presence).expect("active");
                h.rect = item.rect;
                self.pending.push((now + self.real(delay), self.seq, Step::Mount(h)));
            }
            if delay <= 1e-4 {
                self.items.push(shown);
            } else {
                self.pending.push((now + self.real(delay), self.seq, Step::Mount(shown)));
            }
        }
        self.sort_items();
    }

    fn move_shell(&mut self, rest: [f32; 4], spring: Spring) {
        self.shell = rest;
        self.shell_motion = spring;
        self.shell_fade = spring;
    }

    /// Animation seconds until the walls clear `rect` by most of its resting inset (and, on
    /// entry, until the bottom nearly reaches it).
    fn room_delay(&self, rect: [f32; 4], t: f64, entry: bool) -> f32 {
        let [l, r, _] = self.walls.map(|w| w.target);
        let need_l = rect[0] - (rect[0] - l).max(0.0) * 0.6;
        let need_r = rect[0] + rect[2] + (r - rect[0] - rect[2]).max(0.0) * 0.6;
        let need_b = if entry { rect[1] + rect[3] - 2.0 } else { f32::NEG_INFINITY };
        let mut tau = 0.0_f32;
        while tau < 1.5 {
            let at = t + f64::from(tau);
            let ok = self.walls[0].at(at).0 <= need_l + 0.01
                && self.walls[1].at(at).0 >= need_r - 0.01
                && self.walls[2].at(at).0 >= need_b - 0.01;
            if ok {
                return tau;
            }
            tau += 1.0 / 240.0;
        }
        tau
    }

    fn sort_items(&mut self) {
        let order = |id: ElemId| ElemId::ALL.iter().position(|x| *x == id).unwrap_or(0);
        self.items.sort_by_key(|s| order(s.id));
    }

    /// Runs the steps that are due. True when something changed (the view re-runs).
    pub fn tick(&mut self, now: f64) -> bool {
        let mut changed = false;
        loop {
            let Some(i) = self.pending.iter().position(|(due, seq, step)| {
                *due <= now + 1e-9 && (*seq == self.seq || matches!(step, Step::Complete))
            }) else {
                break;
            };
            let (_, _, step) = self.pending.remove(i);
            changed = true;
            match step {
                Step::Entry => self.apply_geometry(now, 0.0),
                Step::Shell(rest, spring) => self.move_shell(rest, spring),
                Step::Mount(shown) if shown.key.starts_with("highlight") => self.highlight = Some(shown),
                Step::Mount(shown) => {
                    self.items.push(shown);
                    self.sort_items();
                }
                Step::Complete => {
                    let routes = self.routes.clone();
                    self.set(Pres::Completed { label: "Copied".into(), routes }, now);
                }
            }
        }
        self.pending.retain(|(_, seq, step)| *seq == self.seq || matches!(step, Step::Complete));
        changed
    }

    /// The next due step, in clock seconds.
    pub fn next_due(&self) -> Option<f64> {
        self.pending.iter().map(|(due, ..)| *due).min_by(f64::total_cmp)
    }

    /// What a click on `id` does, the way the capsule's actions do it in the app.
    pub fn click(&mut self, id: ElemId, now: f64) {
        let has_iphone = match self.pres {
            Pres::Recording { has_iphone, .. } | Pres::Selecting { has_iphone, .. } => has_iphone,
            _ => false,
        };
        let next = match (&self.pres, id.mode()) {
            (Pres::Selecting { mode, .. }, Some(m)) if *mode == m && m.shows_record() => {
                Pres::Recording { chips: false, has_iphone }
            }
            (Pres::Selecting { .. }, Some(mode)) => Pres::Selecting { mode, has_iphone },
            (Pres::Recording { .. }, Some(SelectionMode::Screen)) => Pres::Recording { chips: false, has_iphone },
            (Pres::Recording { .. }, Some(SelectionMode::Iphone)) => Pres::Recording { chips: false, has_iphone },
            (Pres::Recording { .. }, Some(mode)) => Pres::Selecting { mode, has_iphone },
            _ => match id {
                ElemId::Record => Pres::Recording { chips: false, has_iphone },
                ElemId::Finish => return self.finish(now),
                _ => Pres::Hidden,
            },
        };
        self.set(next, now);
    }

    /// Finish: processing for `PROCESSING_S`, then done.
    pub fn finish(&mut self, now: f64) {
        self.set(Pres::Processing, now);
        let seq = self.seq;
        self.pending.push((now + self.real(PROCESSING_S), seq, Step::Complete));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface() -> Surface {
        Surface::new(Tune::DEFAULT, Metrics::default())
    }

    fn recording(chips: bool) -> Pres {
        Pres::Recording { chips, has_iphone: false }
    }

    /// Steps the surface like frames at 120 Hz until `until`, returning the clock.
    fn run(s: &mut Surface, from: f64, until: f64) -> f64 {
        let mut now = from;
        while now < until {
            now += 1.0 / 120.0;
            s.tick(now);
        }
        now
    }

    #[test]
    fn switching_to_area_keeps_every_chip_still() {
        let m = Metrics::default();
        let (_, rec) = layout(&recording(true), "0:00", Labels::default(), &m);
        let area = Pres::Selecting { mode: SelectionMode::Area, has_iphone: false };
        let (_, pick) = layout(&area, "0:00", Labels::default(), &m);
        for item in pick.iter().filter(|i| i.id != ElemId::Lead) {
            let before = rec.iter().find(|r| r.id == item.id).expect("same chips");
            assert_eq!(before.rect, item.rect, "{:?} moved", item.id);
        }
    }

    #[test]
    fn layouts_are_inset_and_gapped() {
        let m = Metrics::default();
        let routes = Pres::Completed { label: "Copied".into(), routes: vec!["Send to A".into(), "Send to B".into()] };
        for pres in [recording(false), recording(true), Pres::Processing, routes] {
            let (shell, items) = layout(&pres, "0:00", Labels { completed: 46.0, routes: [60.0, 60.0] }, &m);
            let first = items.first().unwrap().rect;
            let last = items.last().unwrap().rect;
            assert!((first[0] - shell[0] - ISLAND_INSET).abs() < 1e-3);
            assert!((shell[0] + shell[2] - last[0] - last[2] - ISLAND_INSET).abs() < 1e-3);
            for pair in items.windows(2) {
                let gap = pair[1].rect[0] - pair[0].rect[0] - pair[0].rect[2];
                assert!(gap == crate::native::pill::model::CHIP_GAP || gap == 6.0, "{gap}");
            }
        }
    }

    #[test]
    fn entry_lands_the_pose_instantly_then_springs_to_rest_with_content_after() {
        let mut s = surface();
        s.set(recording(true), 0.0);
        assert!(s.instant && !s.shell_on && s.items.is_empty());
        assert!(s.shell[3] < ISLAND_H, "origin pose is shorter");
        s.instant = false;
        assert!(s.tick(0.001));
        assert!(s.shell_on && s.shell[3] == ISLAND_H);
        assert!(s.items.is_empty(), "content waits for the shell");
        run(&mut s, 0.001, 0.6);
        assert_eq!(s.items.len(), 5, "lead, three chips and close");
        assert!(s.highlight.as_ref().is_some_and(|h| h.id == ElemId::ChipScreen));
    }

    #[test]
    fn narrowing_walls_follow_the_content_and_opening_waits_for_room() {
        let mut s = surface();
        s.set(recording(true), 0.0);
        let now = run(&mut s, 0.0, 1.0);
        let wide = s.shell;
        s.set(recording(false), now);
        assert_eq!(s.shell, wide, "walls start after the chips begin to fade");
        assert!(!s.items.iter().any(|i| i.id == ElemId::ChipArea), "chips leave at once");
        let now = run(&mut s, now, now + 0.03);
        assert_eq!(s.shell, wide);
        assert!(
            s.items.iter().any(|i| i.id == ElemId::Finish),
            "Finish crossfades over the chips leaving under it, before the walls move"
        );
        let now = run(&mut s, now, now + 0.05);
        assert!(s.shell[2] < wide[2]);

        let now = run(&mut s, now, now + 1.0);
        s.finish(now);
        let now = run(&mut s, now, now + PROCESSING_S as f64 + 0.01);
        assert!(matches!(s.pres, Pres::Completed { .. }));
        assert!(!s.items.iter().any(|i| i.id == ElemId::Close), "close waits for the wall");
        run(&mut s, now, now + 1.0);
        assert!(s.items.iter().any(|i| i.id == ElemId::Close));
    }

    #[test]
    fn a_mode_change_in_place_slides_the_highlight() {
        let mut s = surface().settled(Pres::Selecting { mode: SelectionMode::Screen, has_iphone: false });
        let before = s.highlight.clone().unwrap();
        s.set(Pres::Selecting { mode: SelectionMode::Area, has_iphone: false }, 1.0);
        // Record leaves, so the walls (and the highlight with them) wait for its fade.
        run(&mut s, 1.0, 1.1);
        let after = s.highlight.clone().unwrap();
        assert_eq!(after.key, before.key, "one highlight travels");
        assert_eq!(after.id, ElemId::ChipArea);
        assert_eq!(after.motion, HIGHLIGHT);
    }

    #[test]
    fn an_interrupted_transition_drops_its_pending_steps() {
        let mut s = surface();
        s.set(recording(true), 0.0);
        s.tick(0.001);
        s.set(Pres::Hidden, 0.002);
        run(&mut s, 0.002, 1.0);
        assert!(s.items.is_empty() && !s.shell_on);
    }
}
