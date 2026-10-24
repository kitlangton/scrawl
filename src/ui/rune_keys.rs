//! The shortcut control in Settings, drawn with Rune: only the keys of the shortcut, as rounded
//! keycaps with a little depth, in a recessed field. A port of `helper/ShortcutKeys.swift`
//! (same states, springs and timings); see that file's header for what each state looks like.
//!
//! Every value is a spring evaluated in closed form at a time, so a moment draws the same live
//! (from the window's frame clock) or headless (from Rune's virtual clock). Retargeting keeps
//! position and velocity.

use std::collections::HashMap;

use rune::{El, Spring, TextStyle, el, text};

use super::rune_settings::{Rgba, Theme};

// MARK: Springs

/// A scalar on a spring that can be retargeted mid-flight, keeping position and velocity.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Track {
    spring: Spring,
    settle: f64,
    from: f64,
    pub(crate) to: f64,
    v0: f64,
    t0: f64,
}

/// SwiftUI's `Spring.settlingDuration` threshold, close enough that a key is still when it
/// stops moving by a hundredth of a point.
const SETTLE_EPS: f32 = 0.001;

fn settling(spring: Spring) -> f64 {
    f64::from(spring.settling_duration(SETTLE_EPS))
}

impl Track {
    pub(crate) fn new(value: f64, spring: Spring) -> Self {
        Self {
            spring,
            settle: settling(spring),
            from: value,
            to: value,
            v0: 0.0,
            t0: -1e9,
        }
    }

    pub(crate) fn value(&self, t: f64) -> f64 {
        let dt = t - self.t0;
        if dt <= 0.0 {
            return self.from;
        }
        if dt >= self.settle {
            return self.to;
        }
        let (x, _) = self.spring.sample(
            self.from as f32,
            self.v0 as f32,
            self.to as f32,
            dt as f32,
        );
        f64::from(x)
    }

    fn velocity(&self, t: f64) -> f64 {
        let dt = t - self.t0;
        if dt <= 0.0 || dt >= self.settle {
            return 0.0;
        }
        let (_, v) = self.spring.sample(
            self.from as f32,
            self.v0 as f32,
            self.to as f32,
            dt as f32,
        );
        f64::from(v)
    }

    pub(crate) fn set(&mut self, target: f64, t: f64) {
        self.set_with(target, t, None);
    }

    pub(crate) fn set_with(&mut self, target: f64, t: f64, spring: Option<Spring>) {
        if target == self.to {
            return;
        }
        let (x, v) = (self.value(t), self.velocity(t));
        self.from = x;
        self.v0 = v;
        self.to = target;
        self.t0 = t;
        if let Some(spring) = spring
            && spring != self.spring
        {
            self.spring = spring;
            self.settle = settling(spring);
        }
    }

    pub(crate) fn jump(&mut self, v: f64, t: f64) {
        self.from = v;
        self.to = v;
        self.v0 = 0.0;
        self.t0 = t;
    }

    pub(crate) fn still(&self, t: f64) -> bool {
        t - self.t0 >= self.settle
    }
}

pub(crate) mod springs {
    use rune::Spring;
    pub const ENTER: Spring = Spring::new(0.3, 0.0);
    pub const EXIT: Spring = Spring::new(0.22, 0.0);
    pub const SLIDE: Spring = Spring::new(0.32, 0.0);
    pub const WIDTH: Spring = Spring::new(0.28, 0.0);
    pub const DOWN: Spring = Spring::new(0.07, 0.0);
    pub const UP: Spring = Spring::new(0.24, 0.0);
    pub const TINT: Spring = Spring::new(0.2, 0.0);
    pub const DRAW: Spring = Spring::new(0.5, 0.0);
    pub const FADE: Spring = Spring::new(0.35, 0.0);
    pub const REDUCED: Spring = Spring::new(0.15, 0.0);
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct KeyMetrics {
    /// A key's height, its depth included.
    pub height: f64,
    /// The narrowest key (one glyph).
    pub unit: f64,
    pub gap: f64,
    /// Inside the container, each side.
    pub pad: f64,
    /// Room before the keys while listening (the red dot).
    pub lead: f64,
    /// Room after the last key while listening (the caret).
    pub slot: f64,
    /// The content's width while listening with nothing held (the prompt).
    pub empty: f64,
    /// Key labels.
    pub font: f64,
}

impl KeyMetrics {
    pub(crate) fn width(&self, label: &str) -> f64 {
        let n = label.chars().count();
        if n == 1 {
            self.unit
        } else {
            self.unit.max(n as f64 * self.font * 0.6 + 14.0)
        }
    }
}

pub(crate) const FIELD: KeyMetrics = KeyMetrics {
    height: 26.0,
    unit: 26.0,
    gap: 4.0,
    pad: 4.0,
    lead: 18.0,
    slot: 8.0,
    empty: 100.0,
    font: 13.0,
};

// MARK: Motion

#[derive(Clone, Debug)]
pub(crate) struct Key {
    pub x: Track,
    pub presence: Track,
    pub press: Track,
    pub ink: Track,
}

/// Work scheduled for a moment: a tap's release, the flourish's steps, a refusal's end.
#[derive(Clone, Debug)]
enum Later {
    Release(String),
    FlourishDraw,
    FlourishFade,
    FlourishEnd,
    Restore,
}

/// The keys on screen, each with its own identity (its label): entering where it will rest,
/// leaving where it was, sliding when neighbours come and go, pressing when held. Positions are
/// key centres measured from the container's trailing edge.
pub(crate) struct KeyMotion {
    pub m: KeyMetrics,
    pub reduce: bool,
    pub keys: HashMap<String, Key>,
    pub order: Vec<String>,
    pub width: Track,
    pub listen: Track,
    /// 1 while there are no keys and nothing listens: the "Set shortcut" placeholder.
    pub blank: Track,
    pub caret: Track,
    /// 0…1…0 after a new shortcut lands.
    pub flourish: Track,
    pub flourish_fade: Track,
    pub shake_at: f64,
    tap_until: HashMap<String, f64>,
    pending: Vec<(f64, Later)>,
}

impl KeyMotion {
    fn new(m: KeyMetrics, reduce: bool) -> Self {
        Self {
            m,
            reduce,
            keys: HashMap::new(),
            order: Vec::new(),
            width: Track::new(0.0, springs::WIDTH),
            listen: Track::new(0.0, springs::ENTER),
            blank: Track::new(0.0, springs::ENTER),
            caret: Track::new(0.0, springs::SLIDE),
            flourish: Track::new(0.0, springs::DRAW),
            flourish_fade: Track::new(1.0, springs::FADE),
            shake_at: -10.0,
            tap_until: HashMap::new(),
            pending: Vec::new(),
        }
    }

    fn spring(&self, s: Spring) -> Spring {
        if self.reduce { springs::REDUCED } else { s }
    }

    fn mv(reduce: bool, track: &mut Track, target: f64, t: f64, instant: bool) {
        if instant || reduce {
            track.jump(target, t);
        } else {
            track.set(target, t);
        }
    }

    fn show(&mut self, labels: &[String], t: f64, instant: bool) {
        let m = self.m;
        let reduce = self.reduce;
        let listening = self.listen.to > 0.5;
        let mut x = m.pad + if listening { m.lead } else { 0.0 };
        let mut centres: Vec<(String, f64)> = Vec::new();
        for (i, label) in labels.iter().enumerate() {
            let w = m.width(label);
            if i > 0 {
                x += m.gap;
            }
            centres.push((label.clone(), x + w / 2.0));
            x += w;
        }
        let mut caret_centre = x;
        let blank_now = !listening && labels.is_empty();
        if blank_now {
            x += m.empty;
        }
        Self::mv(reduce, &mut self.blank, if blank_now { 1.0 } else { 0.0 }, t, instant);
        if listening {
            if labels.is_empty() {
                caret_centre = x + m.empty / 2.0;
                x += m.empty;
            } else {
                caret_centre = x + m.gap + m.slot / 2.0;
                x += m.gap + m.slot;
            }
        }
        let total = x + m.pad;
        Self::mv(reduce, &mut self.width, total, t, instant);
        if listening {
            let opened = self.listen.from < 0.5 && self.listen.value(t) < 0.05;
            Self::mv(reduce, &mut self.caret, caret_centre - total, t, instant || opened);
        }
        let enter = self.spring(springs::ENTER);
        let exit = self.spring(springs::EXIT);
        let tint = self.spring(springs::TINT);
        let count = centres.len();
        for (label, centre) in &centres {
            let target = centre - total;
            match self.keys.get_mut(label) {
                Some(key) if key.presence.to > 0.5 || key.presence.value(t) > 0.02 => {
                    Self::mv(reduce, &mut key.x, target, t, instant);
                    key.presence.set_with(1.0, t, Some(enter));
                }
                _ => {
                    let mut key = Key {
                        x: Track::new(target, springs::SLIDE),
                        presence: Track::new(if instant { 1.0 } else { 0.0 }, enter),
                        press: Track::new(0.0, springs::UP),
                        ink: Track::new(0.0, tint),
                    };
                    // Its neighbours start making room first, so it lands in a gap rather than
                    // on them.
                    let crowded = !instant && count > 1;
                    key.presence.set(1.0, if crowded { t + 0.06 } else { t });
                    self.keys.insert(label.clone(), key);
                    self.order.retain(|l| l != label);
                    self.order.push(label.clone());
                }
            }
        }
        for (label, key) in &mut self.keys {
            if !centres.iter().any(|(l, _)| l == label) {
                key.presence.set_with(0.0, t, Some(exit));
            }
        }
    }

    fn press(&mut self, labels: &[String], t: f64) {
        for (label, key) in &mut self.keys {
            if t >= self.tap_until.get(label).copied().unwrap_or(-1.0) {
                let down = labels.contains(label);
                key.press.set_with(
                    if down { 1.0 } else { 0.0 },
                    t,
                    Some(if down { springs::DOWN } else { springs::UP }),
                );
            }
        }
    }

    fn tap(&mut self, label: &str, t: f64) {
        if let Some(key) = self.keys.get_mut(label) {
            key.press.set_with(1.0, t, Some(springs::DOWN));
        }
        self.tap_until.insert(label.to_owned(), t + 0.14);
        self.later(t + 0.14, Later::Release(label.to_owned()));
    }

    fn ink(&mut self, labels: &[String], t: f64) {
        for (label, key) in &mut self.keys {
            key.ink.set(if labels.contains(label) { 1.0 } else { 0.0 }, t);
        }
    }

    fn flourish(&mut self, t: f64) {
        self.flourish.jump(0.0, t);
        self.flourish_fade.jump(1.0, t);
        if self.reduce {
            self.flourish.jump(1.0, t);
            self.flourish_fade.jump(0.0, t);
            self.flourish_fade.set_with(1.0, t, Some(springs::REDUCED));
        } else {
            self.later(t + 0.05, Later::FlourishDraw);
        }
        self.later(t + 1.0, Later::FlourishFade);
        self.later(t + 1.4, Later::FlourishEnd);
    }

    fn later(&mut self, at: f64, what: Later) {
        self.pending.push((at, what));
    }

    /// The earliest pending step due by `t`.
    fn due(&mut self, t: f64) -> Option<(f64, Later)> {
        let i = self
            .pending
            .iter()
            .enumerate()
            .filter(|(_, (at, _))| *at <= t)
            .min_by(|a, b| a.1.0.total_cmp(&b.1.0))
            .map(|(i, _)| i)?;
        Some(self.pending.remove(i))
    }

    fn run(&mut self, at: f64, what: Later) {
        match what {
            Later::Release(label) => {
                if let Some(key) = self.keys.get_mut(&label) {
                    key.press.set_with(0.0, at, Some(springs::UP));
                }
            }
            Later::FlourishDraw => self.flourish.set(1.0, at),
            Later::FlourishFade => self.flourish_fade.set(0.0, at),
            Later::FlourishEnd => {
                self.flourish.jump(0.0, at);
                self.flourish_fade.jump(1.0, at);
            }
            Later::Restore => {}
        }
    }

    pub(crate) fn shake(&self, t: f64) -> f64 {
        let dt = t - self.shake_at;
        if self.reduce || !(0.0..0.55).contains(&dt) {
            return 0.0;
        }
        5.0 * (-dt * 8.0).exp() * (2.0 * std::f64::consts::PI * 7.0 * dt).sin()
    }

    pub(crate) fn still(&self, t: f64) -> bool {
        self.pending.is_empty()
            && self.listen.to == 0.0
            && self.listen.still(t)
            && self.blank.still(t)
            && self.width.still(t)
            && self.caret.still(t)
            && self.flourish.still(t)
            && self.flourish_fade.still(t)
            && t - self.shake_at >= 0.55
            && self.keys.values().all(|k| {
                k.x.still(t) && k.presence.still(t) && k.press.still(t) && k.ink.still(t)
            })
    }

    fn prune(&mut self, t: f64) {
        let gone: Vec<String> = self
            .keys
            .iter()
            .filter(|(_, k)| k.presence.to == 0.0 && k.presence.still(t))
            .map(|(l, _)| l.clone())
            .collect();
        for label in gone {
            self.keys.remove(&label);
            self.order.retain(|l| *l != label);
        }
    }
}

pub(crate) const MODIFIERS: [&str; 4] = ["⌃", "⌥", "⇧", "⌘"];

/// The keys of a shortcut in macOS order: ⌃ ⌥ ⇧ ⌘, then the key.
pub(crate) fn key_labels(control: bool, option: bool, shift: bool, command: bool, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (on, label) in [(control, "⌃"), (option, "⌥"), (shift, "⇧"), (command, "⌘")] {
        if on {
            out.push(label.to_owned());
        }
    }
    if !key.is_empty() {
        out.push(key.to_owned());
    }
    out
}

/// The held modifiers as keycap labels.
pub(crate) fn held_labels(mods: rune::Mods) -> Vec<String> {
    let mut out = Vec::new();
    for (on, label) in [
        (mods.ctrl, "⌃"),
        (mods.alt, "⌥"),
        (mods.shift, "⇧"),
        (mods.cmd || mods.logo, "⌘"),
    ] {
        if on {
            out.push(label.to_owned());
        }
    }
    out
}

fn modifiers_in(held: &[String]) -> Vec<String> {
    MODIFIERS
        .iter()
        .filter(|m| held.iter().any(|h| h == *m))
        .map(|m| (*m).to_owned())
        .collect()
}

// MARK: Controller

/// Decides what one shortcut's keys show, from what Scrawl sent and what the hands are doing.
pub(crate) struct KeyController {
    pub motion: KeyMotion,
    shortcut: Vec<String>,
    recording: bool,
    held: Vec<String>,
    typed: Option<String>,
    refusing: bool,
    refused_at: f64,
    attempt: Vec<String>,
    restore: Vec<String>,
}

impl KeyController {
    pub(crate) fn new(m: KeyMetrics, shortcut: Vec<String>, reduce: bool, t: f64) -> Self {
        let mut motion = KeyMotion::new(m, reduce);
        motion.show(&shortcut, t, true);
        Self {
            motion,
            shortcut,
            recording: false,
            held: Vec::new(),
            typed: None,
            refusing: false,
            refused_at: -10.0,
            attempt: Vec::new(),
            restore: Vec::new(),
        }
    }

    pub(crate) fn update(&mut self, new: Vec<String>, on: bool, error: bool, t: f64) {
        let was = self.recording;
        self.recording = on;
        if was && !on && error && !self.attempt.is_empty() {
            self.refuse(new.clone(), t);
        }
        if !on {
            self.typed = None;
        }
        if on && !was {
            self.attempt.clear();
            self.refusing = false;
            self.motion.listen.set(1.0, t);
        } else if !on && was {
            self.motion.listen.set(0.0, t);
        }
        if !self.refusing {
            if new != self.shortcut && !on {
                self.motion.flourish(t);
            }
            self.shortcut = new;
        }
        self.refresh(t);
    }

    pub(crate) fn hold(&mut self, mods: Vec<String>, t: f64) {
        if mods == self.held {
            return;
        }
        self.held = mods;
        if self.recording {
            self.typed = None;
        }
        self.refresh(t);
    }

    pub(crate) fn type_key(&mut self, key: &str, t: f64) {
        if !self.recording {
            return;
        }
        self.typed = Some(key.to_owned());
        let mut attempt = modifiers_in(&self.held);
        attempt.push(key.to_owned());
        self.attempt = attempt;
        self.refresh(t);
        self.motion.tap(key, t);
    }

    fn refuse(&mut self, old: Vec<String>, t: f64) {
        self.refusing = true;
        self.refused_at = t;
        self.motion.shake_at = t + 0.1;
        self.restore = old;
        self.motion.later(t + 0.7, Later::Restore);
    }

    /// Runs what's due by `t`, in order.
    pub(crate) fn advance(&mut self, t: f64) {
        while let Some((at, what)) = self.motion.due(t) {
            if let Later::Restore = what {
                self.refusing = false;
                self.attempt.clear();
                self.shortcut = std::mem::take(&mut self.restore);
                self.refresh(at);
            } else {
                self.motion.run(at, what);
            }
        }
    }

    fn refresh(&mut self, t: f64) {
        self.motion.prune(t);
        if self.refusing {
            let attempt = self.attempt.clone();
            self.motion.show(&attempt, t, false);
            // Red once the refused key has landed.
            self.motion.ink(&attempt, t.max(self.refused_at + 0.08));
            self.motion.press(&[], t);
        } else if self.recording {
            let mut labels = modifiers_in(&self.held);
            labels.extend(self.typed.clone());
            self.motion.show(&labels, t, false);
            self.motion.ink(&[], t);
            let held = self.held.clone();
            self.motion.press(&held, t);
        } else {
            let shortcut = self.shortcut.clone();
            self.motion.show(&shortcut, t, false);
            let mods: Vec<String> = shortcut
                .iter()
                .filter(|l| MODIFIERS.contains(&l.as_str()))
                .cloned()
                .collect();
            let pressed: Vec<String> = mods
                .iter()
                .filter(|m| self.held.contains(m))
                .cloned()
                .collect();
            self.motion.press(&pressed, t);
            let ready = !mods.is_empty() && mods.iter().all(|m| self.held.contains(m));
            let ink: Vec<String> = if ready {
                shortcut
                    .iter()
                    .filter(|l| !MODIFIERS.contains(&l.as_str()))
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            };
            self.motion.ink(&ink, t);
        }
    }
}

// MARK: Views

pub(crate) const KEY_INK: Rgba = Rgba::rgb(1.0, 0x3B as f32 / 255.0, 0x30 as f32 / 255.0);

/// 1…0…1 every `period` seconds: the listening dot and caret.
fn breath(t: f64) -> f64 {
    0.5 + 0.5 * (2.0 * std::f64::consts::PI * t / 1.3).cos()
}

/// A rounded-rect key standing on its own base: the base shows below the cap as depth, and the
/// cap sinks onto it when pressed. `scale` shrinks it about its bottom centre.
#[allow(clippy::too_many_arguments)]
fn keycap(
    label: &str,
    width: f32,
    height: f32,
    font: f32,
    press: f32,
    ink: f32,
    dark: bool,
    under: Rgba,
    scale: f32,
) -> El {
    let depth = 2.0 * scale;
    let radius = 6.0 * scale;
    let (width, height) = (width * scale, height * scale);
    let down = press * (depth - 0.5 * scale);
    let face = height - depth;
    let top = if dark { Rgba::rgb(0.205, 0.205, 0.215) } else { Rgba::WHITE };
    let bottom = if dark { Rgba::rgb(0.165, 0.165, 0.175) } else { Rgba::rgb(0.965, 0.965, 0.97) };
    let base = if dark { Rgba::rgb(0.06, 0.06, 0.065) } else { Rgba::rgb(0.78, 0.78, 0.8) };
    let edge = if dark { Rgba::WHITE.alpha(0.07) } else { Rgba::BLACK.alpha(0.11) };
    let glyph = if dark {
        Rgba::WHITE.alpha(0.86 + 0.14 * press)
    } else {
        Rgba::BLACK.alpha(0.8 + 0.15 * press)
    };
    let cap_mid = top.mix(bottom, 0.5);
    let size = if label.chars().count() > 1 { font - 2.0 } else { font } * scale;
    let shadow = Rgba::BLACK.alpha(if dark { 0.3 } else { 0.07 });
    el().size(width, height)
        .child(
            el().absolute(0.0, depth)
                .size(width, face)
                .radius(radius)
                .fill(base.color())
                .shadow(1.0, 1.0, shadow.over(under)),
        )
        .child(
            el().absolute(0.0, down)
                .size(width, face)
                .radius(radius)
                .gradient(top.color(), bottom.color())
                .border(1.0, edge.mix(KEY_INK.alpha(0.75), ink).over(cap_mid))
                .center()
                .child(
                    // A catch of light along the top edge, as on a real cap.
                    el().absolute(0.5, 0.5)
                        .size(width - 1.0, face - 1.0)
                        .radius(radius - 0.5)
                        .border_gradient(
                            1.0,
                            Rgba::WHITE.alpha(if dark { 0.1 } else { 0.9 }).over(top),
                            Rgba::WHITE.alpha(0.0).over(cap_mid),
                        ),
                )
                .child(
                    text(label)
                        .font(super::rune_settings::style(size, 500.0))
                        .color(glyph.mix(KEY_INK, ink).over(cap_mid)),
                ),
        )
}

/// One shortcut's field at time `t`: the keys in a recessed field that reads as something to
/// click. Listening tints it red, a dot breathes, and keys land as they go down.
pub(crate) fn field(k: &KeyMotion, t: f64, theme: Theme, hovering: bool) -> El {
    let listen = k.listen.value(t) as f32;
    let w = k.width.value(t) as f32;
    let h = (k.m.height + k.m.pad * 2.0 - 1.0) as f32;
    let br = if k.reduce { 1.0 } else { breath(t) as f32 };
    let keyed = k
        .keys
        .values()
        .map(|key| key.presence.value(t))
        .fold(0.0_f64, f64::max) as f32;
    let under = theme.group_bg();
    let dark = theme.dark;
    let hover = if dark { Rgba::WHITE.alpha(0.16) } else { Rgba::BLACK.alpha(0.17) };
    let fill = (if dark { Rgba::BLACK.alpha(0.22) } else { Rgba::BLACK.alpha(0.04) })
        .mix(KEY_INK.alpha(if dark { 0.12 } else { 0.045 }), listen);
    let fill_bg = fill.over_rgba(under);
    let line = (if hovering { hover } else { theme.group_line() }).mix(KEY_INK.alpha(0.5), listen);
    let prompt = listen * (1.0 - 2.0 * keyed).max(0.0);
    let blank = k.blank.value(t) as f32;
    let shake = k.shake(t) as f32;
    let secondary = theme.secondary().over(fill_bg);

    let mut inner = el()
        .absolute(-2.0 + shake, -2.0)
        .size(w + 4.0, h + 4.0)
        .clip_soft(1.5)
        .child(
            el().absolute(2.0, 2.0)
                .size(w, h)
                .radius(10.0)
                .fill(fill_bg.color())
                .border(1.0, line.over(fill_bg)),
        )
        .child(
            el().absolute(2.0 + k.m.pad as f32 + 6.0 + 3.5 * (1.0 - (0.8 + 0.2 * br)), 2.0 + h / 2.0 - 3.5 * (0.8 + 0.2 * br))
                .size(7.0 * (0.8 + 0.2 * br), 7.0 * (0.8 + 0.2 * br))
                .radius(3.5)
                .fill(KEY_INK.color())
                .opacity(listen * (0.55 + 0.45 * br)),
        )
        .child(
            text("Type a shortcut")
                .font(super::rune_settings::style(12.0, 400.0))
                .color(secondary)
                .single_line()
                .absolute(2.0 + (k.m.pad + k.m.lead) as f32, 2.0 + h / 2.0 - 8.0)
                .opacity(prompt)
                .blur(if k.reduce { 0.0 } else { 1.0 - prompt }),
        )
        .child(
            el().absolute(2.0, 2.0)
                .size(w, h)
                .center()
                .child(
                    text("Set shortcut")
                        .font(super::rune_settings::style(12.0, 400.0))
                        .color(secondary)
                        .single_line(),
                )
                .opacity(blank)
                .blur(if k.reduce { 0.0 } else { 1.0 - blank }),
        )
        .child(
            el().absolute(2.0 + w + k.caret.value(t) as f32 - 0.75, 2.0 + h / 2.0 - 7.0)
                .size(1.5, 14.0)
                .radius(0.75)
                .fill(KEY_INK.color())
                .opacity(listen * keyed * (0.35 + 0.65 * br)),
        );

    // The keys, each from its own tracks, trailing edge at the field's trailing edge.
    let f = k.flourish.value(t);
    let live: Vec<&String> = k
        .order
        .iter()
        .filter(|l| k.keys.get(*l).is_some_and(|key| key.presence.to == 1.0))
        .collect();
    let top = 2.0 + k.m.pad as f32 - 0.5;
    for label in &k.order {
        let Some(key) = k.keys.get(label) else { continue };
        let p = key.presence.value(t) as f32;
        if p <= 0.003 {
            continue;
        }
        let kw = k.m.width(label) as f32;
        let i = live.iter().position(|l| *l == label).unwrap_or(0) as f64;
        let bob = ((std::f64::consts::PI * ((f - i * 0.14) / 0.42).clamp(0.0, 1.0)).sin()).max(0.0) * 0.85;
        let press = if k.reduce { 0.0 } else { key.press.value(t).max(bob) as f32 };
        let scale = if k.reduce { 1.0 } else { 0.92 + 0.08 * p };
        let lift = if k.reduce {
            0.0
        } else {
            (1.0 - p) * if key.presence.to > 0.5 { -4.0 } else { 2.0 }
        };
        let kh = k.m.height as f32;
        let cx = w + key.x.value(t) as f32;
        let left = 2.0 + cx - kw * scale / 2.0;
        let y = top + lift + kh * (1.0 - scale);
        inner = inner.child(
            keycap(
                label,
                kw,
                kh,
                k.m.font as f32,
                press,
                key.ink.value(t) as f32,
                dark,
                fill_bg,
                scale,
            )
            .absolute(left, y)
            .blur(if k.reduce { 0.0 } else { (1.0 - p) * 1.5 })
            .opacity(p),
        );
    }
    el().size(w, h).child(inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_track_retargets_from_where_it_is() {
        let mut track = Track::new(0.0, springs::SLIDE);
        track.set(10.0, 0.0);
        let mid = track.value(0.1);
        assert!(mid > 0.0 && mid < 10.0);
        track.set(0.0, 0.1);
        assert!((track.value(0.1) - mid).abs() < 1e-3);
        assert!(track.still(5.0));
        assert_eq!(track.value(5.0), 0.0);
    }

    #[test]
    fn listening_clears_the_keys_and_held_modifiers_land_in_macos_order() {
        let mut keys = KeyController::new(FIELD, labels(&["⌥", "⇧", "S"]), false, 0.0);
        keys.update(labels(&["⌥", "⇧", "S"]), true, false, 1.0);
        keys.hold(labels(&["⌘", "⌃"]), 1.2);
        keys.advance(3.0);
        let present: Vec<&String> = keys
            .motion
            .order
            .iter()
            .filter(|l| keys.motion.keys[*l].presence.to == 1.0)
            .collect();
        assert_eq!(present, vec!["⌃", "⌘"]);
        assert!(keys.motion.keys["⌃"].x.to < keys.motion.keys["⌘"].x.to);
    }

    #[test]
    fn a_refused_combination_shakes_then_the_old_keys_come_back() {
        let old = labels(&["⌥", "⇧", "S"]);
        let mut keys = KeyController::new(FIELD, old.clone(), false, 0.0);
        keys.update(old.clone(), true, false, 1.0);
        keys.hold(labels(&["⌘"]), 1.3);
        keys.type_key("V", 1.6);
        keys.update(old.clone(), false, true, 1.62);
        assert!(keys.motion.shake(1.8).abs() > 0.0);
        keys.advance(1.7);
        assert_eq!(keys.motion.keys["V"].presence.to, 1.0);
        keys.advance(2.4);
        assert_eq!(keys.motion.keys["S"].presence.to, 1.0);
        assert_eq!(keys.motion.keys["V"].presence.to, 0.0);
        keys.advance(6.0);
        assert!(keys.motion.still(6.0));
    }
}
