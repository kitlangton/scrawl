//! The drag panel for Screen Recording and Accessibility drawn with Rune, in Scrawl's own process.
//! A port of `helper/PermissionPanel.swift` (same sizes, colours, springs, placement and timings),
//! used when the `runePermissionPanel` experiment is on at launch; otherwise
//! `rune_permissions::open_settings` launches the SwiftUI `scrawl-helper permission-panel`.
//!
//! The card flies from the Grant button to System Settings in one overlay (`.floating + 1`, no
//! input) and hands off to a card-sized overlay at `.floating` that follows the Settings window
//! and is dragged from. The docked overlay's state is the controller: its poll waits for the
//! Settings window, tracks it, and reads the grant from a fresh `scrawl-helper permission-panel
//! --check` (a running process may keep a stale answer). All coordinates are global top-left
//! points; both overlays are placed on the primary display, whose top-left is the global origin.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rune::{
    App, AppSpec, Color, Cursor, Cx, DisplayTarget, DragItem, DragOutcome, El, KeyPolicy, OverlayHandle,
    OverlayInput, OverlayOptions, OverlayPlacement, OverlaySpec, Pixels, Shadow, Span, Spring, SpringPair,
    SystemMaterial, Transition, Tx, el, rich, text,
};

use super::rune_settings::{Rgba, Theme, app_icon_px, families, fonts, icons, measure, reduce_motion, style};
use super::rune_symbols::{symbol, weight};
use crate::screens::{self, Rect};

// MARK: Tuning

/// The flight from the Grant button to the dock, and back on Back.
const FLIGHT: StepSpring = StepSpring { duration: 0.85, bounce: 0.0 };
const RETURN: StepSpring = StepSpring { duration: 0.7, bounce: 0.0 };
/// Width leads height (and the path) by running on a shorter spring.
const WIDTH_LEAD: f64 = 0.78;
/// Peak height of the arc above the higher end, and its cap as a share of the distance.
const ARC_HEIGHT: f64 = 160.0;
const ARC_SHARE: f64 = 0.35;
/// Reduce Motion and the in-place fades.
const FADE: f64 = 0.18;
/// The exits: alpha, with the content shrinking to 0.96 and blurring 3 pt.
const FADE_OUT: f64 = FADE * 0.8;
/// Room around the docked card for its shadows.
const MARGIN: f64 = 48.0;
/// Room around the flight path.
const FLIGHT_PAD: f64 = 64.0;

// System Settings (macOS 26/27): a fixed 232 pt sidebar; the Privacy list starts 20 pt past it
// and ends 20 pt before the window's right edge.
const LIST_LEADING: f64 = 252.0;
const LIST_TRAILING: f64 = 20.0;
const SETTINGS_BUNDLE: &str = "com.apple.systempreferences";

// MARK: Permissions

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    ScreenRecording,
    Accessibility,
}

impl Kind {
    fn parse(raw: &str) -> Option<Kind> {
        match raw {
            "screen-recording" => Some(Kind::ScreenRecording),
            "accessibility" => Some(Kind::Accessibility),
            _ => None,
        }
    }

    fn raw(self) -> &'static str {
        match self {
            Kind::ScreenRecording => "screen-recording",
            Kind::Accessibility => "accessibility",
        }
    }

    /// The name the pane's list is known by. macOS 27 renamed Accessibility.
    fn title(self) -> &'static str {
        match self {
            Kind::ScreenRecording => "Screen Recording",
            Kind::Accessibility => super::rune_onboarding::accessibility_name(),
        }
    }

    fn settings_url(self) -> String {
        let anchor = match self {
            Kind::ScreenRecording => "Privacy_ScreenCapture",
            Kind::Accessibility => "Privacy_Accessibility",
        };
        format!("x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?{anchor}")
    }
}

// MARK: Spring

/// The step response of a spring from 0 to 1 at rest: ω = 2π / duration, ζ = 1 − bounce.
#[derive(Clone, Copy, Debug, PartialEq)]
struct StepSpring {
    duration: f64,
    bounce: f64,
}

impl StepSpring {
    fn value(self, t: f64) -> f64 {
        if t <= 0.0 {
            return 0.0;
        }
        let omega = 2.0 * std::f64::consts::PI / self.duration;
        let zeta = 1.0 - self.bounce;
        if zeta >= 1.0 {
            return 1.0 - (1.0 + omega * t) * (-omega * t).exp();
        }
        let damped = omega * (1.0 - zeta * zeta).sqrt();
        1.0 - (-zeta * omega * t).exp() * ((damped * t).cos() + zeta * omega / damped * (damped * t).sin())
    }

    /// Long enough that what's left is under a hundredth of a point on a screen-wide move.
    fn settle(self) -> f64 {
        self.duration * 1.8
    }
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// MARK: Geometry

/// Card metrics. The app row matches the Privacy list's column in the Settings window.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Layout {
    row_width: f64,
}

impl Layout {
    const INSTRUCTION: f64 = 28.0;
    /// Less above the instruction than below the row: line height already pads the text.
    const TOP: f64 = 7.0;
    const PADDING: f64 = 12.0;
    const GUTTER: f64 = 52.0;
    const ROW: f64 = 44.0;
    const GAP: f64 = 4.0;
    const INSET: f64 = 8.0;
    const DOCK_GAP: f64 = 10.0;
    /// Inside the window: this far above its bottom edge.
    const INSIDE_BOTTOM: f64 = 20.0;
    const RADIUS: f64 = 18.0;
    const ROW_RADIUS: f64 = 9.0;
    const ICON: f64 = 26.0;
    const ICON_LEADING: f64 = 10.0;
    const TEXT_LEADING: f64 = Self::ICON_LEADING + Self::ICON + 10.0;
    const ROW_Y: f64 = Self::TOP + Self::INSTRUCTION + Self::GAP;

    fn new(settings_width: f64) -> Layout {
        let column = settings_width - LIST_LEADING - LIST_TRAILING;
        Layout { row_width: column.min(640.0).max(320.0) }
    }

    fn size(self) -> (f64, f64) {
        (Self::GUTTER + self.row_width + Self::PADDING, Self::ROW_Y + Self::ROW + Self::PADDING)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dock {
    Below,
    Inside,
    Right,
    Left,
    Clamped,
}

impl Dock {
    fn name(self) -> &'static str {
        match self {
            Dock::Below => "below",
            Dock::Inside => "inside",
            Dock::Right => "right",
            Dock::Left => "left",
            Dock::Clamped => "clamped",
        }
    }
}

/// Where the card docks for a Settings window on a screen, both in AppKit coordinates (y up;
/// `visible` is the screen's visible frame). In order:
/// - below the window, its app row under the Privacy list's column, if all of it fits;
/// - inside the window's empty lower area (the lowest 30% of these panes holds no rows or
///   toggles), on the list's column and 20 pt above its bottom edge;
/// - beside it, right then left;
/// - else the below position pushed onto the screen.
fn dock(settings: Rect, visible: Rect, layout: Layout) -> (Rect, Dock) {
    let (w, h) = layout.size();
    let (min_x, max_x) = (|r: Rect| r[0], |r: Rect| r[0] + r[2]);
    let (min_y, max_y) = (|r: Rect| r[1], |r: Rect| r[1] + r[3]);
    let fits = |r: Rect| {
        min_x(r) >= min_x(visible) && max_x(r) <= max_x(visible) && min_y(r) >= min_y(visible) && max_y(r) <= max_y(visible)
    };
    let clamp_x = |x: f64| x.max(min_x(visible) + 8.0).min(max_x(visible) - w - 8.0);
    let clamp_y = |y: f64| y.max(min_y(visible) + 8.0).min(max_y(visible) - h - 8.0);
    // The list's column; nudged left if the card would pass the window's right edge.
    let column = (settings[0] + LIST_LEADING - Layout::GUTTER).min(max_x(settings) - Layout::INSET - w);
    let gap = Layout::DOCK_GAP;

    let below = [clamp_x(column), settings[1] - gap - h, w, h];
    if fits(below) {
        return (below, Dock::Below);
    }
    let inside_y = (settings[1] + Layout::INSIDE_BOTTOM).max(min_y(visible) + 8.0);
    let inside = [column, inside_y, w, h];
    if fits(inside) && inside[0] >= settings[0] && max_y(inside) <= settings[1] + settings[3] * 0.3 {
        return (inside, Dock::Inside);
    }
    let right = [max_x(settings) + gap, clamp_y(settings[1]), w, h];
    if fits(right) {
        return (right, Dock::Right);
    }
    let left = [settings[0] - gap - w, clamp_y(settings[1]), w, h];
    if fits(left) {
        return (left, Dock::Left);
    }
    ([clamp_x(below[0]), clamp_y(below[1]), w, h], Dock::Clamped)
}

/// Global top-left ⇄ y-up (its own inverse). `dock` only compares, so any flip will do.
fn up(r: Rect) -> Rect {
    [r[0], -r[1] - r[3], r[2], r[3]]
}

/// `dock` for a Settings window in global coordinates, on the screen holding it.
fn dock_for(settings: Rect, layout: Layout, screens: &[screens::Screen]) -> (Rect, Dock) {
    let visible = screens::index_for_rect(screens, settings).map_or(settings, |i| screens[i].visible);
    let (card, how) = dock(up(settings), up(visible), layout);
    (up(card), how)
}

fn mid(r: Rect) -> [f64; 2] {
    [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0]
}

fn inset(r: Rect, d: f64) -> Rect {
    [r[0] + d, r[1] + d, r[2] - 2.0 * d, r[3] - 2.0 * d]
}

fn integral(r: Rect) -> [i64; 4] {
    let (x0, y0) = (r[0].floor(), r[1].floor());
    let (x1, y1) = ((r[0] + r[2]).ceil(), (r[1] + r[3]).ceil());
    [x0 as i64, y0 as i64, (x1 - x0) as i64, (y1 - y0) as i64]
}

// MARK: Flight

/// The card in the air, in one overlay over the whole path. The path is a quadratic Bézier
/// through a point above both ends; the size runs ahead of the path (width first), the corners
/// go from the button's capsule to the card's, and the content resolves (opacity, 0.96 → 1
/// scale, 3 pt → 0 blur) as the card reaches full size.
#[derive(Clone, Copy, Debug, PartialEq)]
struct FlightRun {
    source: Rect,
    target: Rect,
    control: [f64; 2],
    /// The flight window, global.
    window: Rect,
    spring: StepSpring,
    /// Runs from the dock back to the source.
    reverse: bool,
    /// Frame-clock time of the first frame.
    started: Option<f64>,
    ended: bool,
}

/// One frame of a flight: the card's global rect, radius, lift (0 docked, 1 at the top of the
/// arc), the surface's alpha and how far its content has resolved.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Step {
    rect: Rect,
    radius: f64,
    lift: f64,
    alpha: f64,
    resolve: f64,
}

impl FlightRun {
    /// `visible_top` is the top of the visible frame on the source's screen (global).
    fn new(source: Rect, target: Rect, visible_top: f64, spring: StepSpring, reverse: bool) -> FlightRun {
        let control = control(source, target, visible_top);
        let x0 = source[0].min(target[0]).min(control[0]) - FLIGHT_PAD;
        let y0 = source[1].min(target[1]).min(control[1]) - FLIGHT_PAD;
        let x1 = (source[0] + source[2]).max(target[0] + target[2]).max(control[0] + 1.0) + FLIGHT_PAD;
        let y1 = (source[1] + source[3]).max(target[1] + target[3]).max(control[1] + 1.0) + FLIGHT_PAD;
        FlightRun {
            source,
            target,
            control,
            window: [x0, y0, x1 - x0, y1 - y0],
            spring,
            reverse,
            started: None,
            ended: false,
        }
    }

    fn step(&self, t: f64) -> Step {
        // The last frame lands exactly, so the docked window takes over pixel for pixel.
        let done = t >= self.spring.settle();
        let path = if done { 1.0 } else { self.spring.value(t) };
        let width =
            if done { 1.0 } else { StepSpring { duration: self.spring.duration * WIDTH_LEAD, ..self.spring }.value(t) };
        let p = if self.reverse { 1.0 - path } else { path };
        let w = if self.reverse { 1.0 - width } else { width };
        let h = p;
        let (start, end) = (mid(self.source), mid(self.target));
        let u = 1.0 - p;
        let bezier = |a: usize| u * u * start[a] + 2.0 * u * p * self.control[a] + p * p * end[a];
        let size = [
            self.source[2] + (self.target[2] - self.source[2]) * w,
            self.source[3] + (self.target[3] - self.source[3]) * h,
        ];
        let start_radius = self.source[3] / 2.0;
        let k = w.min(h);
        let (from, to) = if self.reverse { (0.5, 0.9) } else { (0.3, 0.85) };
        Step {
            rect: [bezier(0) - size[0] / 2.0, bezier(1) - size[1] / 2.0, size[0], size[1]],
            radius: start_radius + (Layout::RADIUS - start_radius) * k,
            lift: 4.0 * p * (1.0 - p),
            // Lift off the button over the first stretch; land on it fading on the way back.
            alpha: smoothstep(0.0, 0.14, p),
            resolve: smoothstep(from, to, k),
        }
    }
}

/// The Bézier's control point: midway across, `ARC_HEIGHT` above the higher end (less on short
/// hops), and low enough that the card fits below the top of the source's screen.
fn control(source: Rect, target: Rect, visible_top: f64) -> [f64; 2] {
    let (s, t) = (mid(source), mid(target));
    let lift = ARC_HEIGHT.min((t[0] - s[0]).hypot(t[1] - s[1]) * ARC_SHARE);
    let top = s[1].min(t[1]);
    let ceiling = visible_top + target[3] / 2.0 + 8.0;
    [(s[0] + t[0]) / 2.0, top.min((top - lift).max(ceiling))]
}

// MARK: State

#[derive(Clone, Copy, Debug, PartialEq)]
enum Role {
    Dock,
    Flight,
    Preview,
}

/// What both overlays draw, so the hand-off is pixel for pixel.
#[derive(Clone)]
struct Look {
    kind: Kind,
    layout: Layout,
    granted: bool,
    hovering: bool,
    back_hover: bool,
    dragging: bool,
    reduce: bool,
    name: String,
    app: PathBuf,
    icon: Option<Pixels>,
}

impl Look {
    fn new(kind: Kind) -> Look {
        let app = app_bundle();
        let name = app.file_stem().map_or_else(|| "Scrawl".to_owned(), |s| s.to_string_lossy().into_owned());
        Look {
            kind,
            layout: Layout::new(740.0),
            granted: false,
            hovering: false,
            back_hover: false,
            dragging: false,
            reduce: reduce_motion(),
            name,
            app,
            icon: app_icon_px(128),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    /// Reading the grant before anything shows.
    Checking,
    /// Settings is opening: waiting for its window to stop moving.
    Opening { deadline: Instant, first_seen: Option<Instant>, last: Option<Rect>, next: Instant },
    Flying,
    Docked,
    /// Showing the grant before fading.
    Granted { until: Instant },
    /// Fading out, or flying back (`back`).
    Leaving { back: bool },
}

struct Session {
    id: u64,
    kind: Kind,
    /// The Grant button, global.
    from: Option<Rect>,
    stage: Stage,
    started: Instant,
    settings_id: Option<u32>,
    settings_frame: Option<Rect>,
    /// The docked card, global.
    card: Option<Rect>,
    missing_since: Option<Instant>,
    checking: bool,
    last_poll: Option<Instant>,
    fast_until: Option<Instant>,
    /// After a drop: until a prompt Settings showed goes, or the deadline. `(deadline, peak)`.
    race: Option<(Instant, usize)>,
    race_next: Instant,
    reshow_at: Option<Instant>,
    /// The docked window is shown (or fading in).
    shown: bool,
    /// The grant arrived mid-flight.
    granted_pending: bool,
    /// The frontmost app's pid and whether it's Settings or Scrawl.
    front: Option<(i32, bool)>,
}

impl Session {
    fn finished(&self) -> bool {
        matches!(self.stage, Stage::Granted { .. } | Stage::Leaving { .. })
    }
}

pub(crate) struct Panel {
    role: Role,
    look: Look,
    /// The frame clock, from the last poll.
    now: f64,
    /// Flight overlay: the run in the air.
    flight: Option<FlightRun>,
    /// Docked overlay: the controller.
    session: Option<Session>,
    /// Docked overlay: frame-clock start of the exit fade.
    leave: Option<f64>,
}

impl Panel {
    fn new(role: Role) -> Panel {
        Panel { role, look: Look::new(Kind::ScreenRecording), now: 0.0, flight: None, session: None, leave: None }
    }

    // MARK: Flow

    fn start(&mut self, kind: Kind, from: Option<Rect>, tx: &mut Tx) {
        self.end_now();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed) + 1;
        self.look = Look::new(kind);
        let now = Instant::now();
        self.session = Some(Session {
            id,
            kind,
            from: from.filter(|r| r[2] > 0.0 && r[3] > 0.0),
            stage: Stage::Checking,
            started: now,
            settings_id: None,
            settings_frame: None,
            card: None,
            missing_since: None,
            checking: true,
            last_poll: Some(now),
            fast_until: None,
            race: None,
            race_next: now,
            reshow_at: None,
            shown: false,
            granted_pending: false,
            front: None,
        });
        check_fresh(id, kind, false);
        tx.wake_after(Duration::from_millis(100));
    }

    /// Hides both windows and forgets the session.
    fn end_now(&mut self) {
        if self.session.take().is_some() {
            if let Some(dock) = DOCK.get() {
                dock.hide();
            }
            if let Some(flight) = FLIGHT_OVERLAY.get() {
                flight.update(|f, _| f.flight = None);
                flight.hide();
            }
        }
        self.leave = None;
        self.look.hovering = false;
        self.look.back_hover = false;
        self.look.dragging = false;
    }

    fn checked(&mut self, id: u64, granted: bool, reshow: bool, tx: &mut Tx) {
        let Some(s) = self.session.as_mut().filter(|s| s.id == id) else { return };
        s.checking = false;
        if s.stage == Stage::Checking {
            if granted {
                emit(s.kind, "granted", None);
                self.session = None;
                return;
            }
            super::rune_permissions::open_url(&s.kind.settings_url());
            let now = Instant::now();
            s.stage = Stage::Opening { deadline: now + Duration::from_secs(6), first_seen: None, last: None, next: now };
            tx.wake_after(Duration::ZERO);
            return;
        }
        if s.finished() {
            return;
        }
        if granted {
            self.finish(tx);
        } else if reshow && s.race.is_none() {
            s.shown = true;
            fade_in();
        }
    }

    /// One controller step. True when the look changed.
    fn tick(&mut self, now_clock: f64, tx: &mut Tx) -> bool {
        let Some(s) = self.session.as_mut() else { return false };
        let now = Instant::now();
        match s.stage {
            Stage::Checking | Stage::Flying | Stage::Leaving { back: true } => false,
            Stage::Opening { .. } => self.opening(now, tx),
            Stage::Docked => self.track(now, tx),
            Stage::Granted { until } => {
                if now >= until {
                    self.leave(tx);
                } else {
                    tx.wake_after(until - now);
                }
                false
            }
            Stage::Leaving { back: false } => {
                let start = *self.leave.get_or_insert(now_clock);
                tx.instant();
                if now_clock - start >= FADE_OUT {
                    // `fade_out` hides the window when its fade ends.
                    self.session = None;
                } else {
                    tx.wake_after(Duration::ZERO);
                }
                true
            }
        }
    }

    /// Waits for the Settings window to stop moving: two equal samples 100 ms apart, or 1.2 s
    /// after it first appeared (a cold launch, or the genie from the Dock).
    fn opening(&mut self, now: Instant, tx: &mut Tx) -> bool {
        let Some(s) = self.session.as_mut() else { return false };
        let Stage::Opening { deadline, first_seen, last, next } = &mut s.stage else { return false };
        if now < *next {
            tx.wake_after(*next - now);
            return false;
        }
        *next = now + Duration::from_millis(100);
        if let Some((id, frame)) = settings_main() {
            let settled = last.is_some_and(|l| integral(l) == integral(frame))
                || first_seen.is_some_and(|f| now - f >= Duration::from_millis(1200));
            if settled {
                return self.dock_at(id, frame, tx);
            }
            first_seen.get_or_insert(now);
            *last = Some(frame);
        } else {
            (*first_seen, *last) = (None, None);
            if now > *deadline {
                emit(s.kind, "cancelled", Some("no-settings-window"));
                self.session = None;
                return false;
            }
        }
        tx.wake_after(Duration::from_millis(100));
        false
    }

    fn dock_at(&mut self, id: u32, frame: Rect, tx: &mut Tx) -> bool {
        let Some(s) = self.session.as_mut() else { return false };
        let screens = screens::current();
        s.settings_id = Some(id);
        s.settings_frame = Some(frame);
        self.look.layout = Layout::new(frame[2]);
        let (card, how) = dock_for(frame, self.look.layout, &screens);
        s.card = Some(card);
        place_dock(card);
        crate::log::scrawl_log(format!(
            "permission-panel: docking {} {} card {:?} settings {:?}",
            s.kind.raw(),
            how.name(),
            card.map(f64::round),
            frame.map(f64::round),
        ));
        match s.from.filter(|_| !self.look.reduce) {
            Some(from) => {
                s.stage = Stage::Flying;
                let top = screens::index_at(&screens, mid(from)).map_or(f64::MIN, |i| screens[i].visible[1]);
                fly(FlightRun::new(from, card, top, FLIGHT, false), self.look.clone());
            }
            None => {
                s.shown = true;
                fade_in();
                self.docked(tx);
            }
        }
        true
    }

    /// The flight landed: the docked window took its place.
    fn landed(&mut self, tx: &mut Tx) {
        if let Some(s) = self.session.as_mut().filter(|s| s.stage == Stage::Flying) {
            s.shown = true;
            self.docked(tx);
        }
    }

    fn docked(&mut self, tx: &mut Tx) {
        let Some(s) = self.session.as_mut() else { return };
        s.stage = Stage::Docked;
        emit(s.kind, "docked", None);
        s.last_poll = None;
        if s.granted_pending {
            self.finish(tx);
        } else {
            tx.wake_after(Duration::ZERO);
        }
    }

    /// The panel follows Settings 1:1 (the user is moving it), steps aside while another app is
    /// in front, and reads the grant.
    fn track(&mut self, now: Instant, tx: &mut Tx) -> bool {
        let Some(s) = self.session.as_mut() else { return false };
        let mut frame = s.settings_id.and_then(window_frame);
        if frame.is_none()
            && let Some((id, f)) = settings_main()
        {
            s.settings_id = Some(id);
            frame = Some(f);
        }
        match frame {
            None => {
                let since = *s.missing_since.get_or_insert(now);
                if now - since > Duration::from_millis(600) {
                    return self.cancel("settings-closed", tx);
                }
            }
            Some(frame) => {
                s.missing_since = None;
                if s.settings_frame != Some(frame) {
                    s.settings_frame = Some(frame);
                    let card = dock_for(frame, self.look.layout, &screens::current()).0;
                    s.card = Some(card);
                    place_dock(card);
                }
                if s.race.is_none() {
                    let ours = self.look.dragging || frontmost_ours(&mut s.front, &self.look.app);
                    if ours != s.shown {
                        s.shown = ours;
                        if ours { fade_in() } else { fade_out() }
                    }
                }
            }
        }
        if now - s.started > Duration::from_secs(600) {
            return self.cancel("timeout", tx);
        }
        let fast = s.race.is_some() || s.fast_until.is_some_and(|u| now < u);
        let interval = Duration::from_millis(if fast { 250 } else { 750 });
        if s.last_poll.is_none_or(|l| now - l >= interval) {
            poll_grant(s, false);
        }
        // The race ends when a prompt Settings showed is dismissed, or after 10 s.
        if let Some((deadline, peak)) = s.race
            && now >= s.race_next
        {
            s.race_next = now + Duration::from_millis(250);
            let count = settings_count();
            s.race = Some((deadline, peak.max(count)));
            if count < peak || now > deadline {
                s.race = None;
                s.reshow_at = Some(now + Duration::from_millis(200));
            }
        }
        if let Some(at) = s.reshow_at
            && now >= at
            && !s.checking
        {
            s.reshow_at = None;
            poll_grant(s, true);
        }
        tx.wake_after(Duration::from_millis(16));
        false
    }

    /// A drop is a hint, never the answer: step out of the way of whatever Settings shows, read
    /// the grant fresh, and come back if it isn't there.
    fn drag_ended(&mut self, outcome: DragOutcome, tx: &mut Tx) {
        self.look.dragging = false;
        self.look.hovering = false;
        if outcome == DragOutcome::Cancelled {
            return;
        }
        let Some(s) = self.session.as_mut().filter(|s| s.stage == Stage::Docked) else { return };
        emit(s.kind, "dropped", None);
        let now = Instant::now();
        s.race = Some((now + Duration::from_secs(10), settings_count()));
        s.race_next = now + Duration::from_millis(250);
        s.fast_until = Some(now + Duration::from_secs(10));
        s.shown = false;
        fade_out();
        poll_grant(s, false);
        tx.wake_after(Duration::from_millis(16));
    }

    fn finish(&mut self, tx: &mut Tx) {
        let Some(s) = self.session.as_mut() else { return };
        match s.stage {
            Stage::Granted { .. } | Stage::Leaving { .. } => return,
            Stage::Flying => {
                s.granted_pending = true;
                return;
            }
            Stage::Checking | Stage::Opening { .. } => {
                emit(s.kind, "granted", None);
                self.session = None;
                return;
            }
            Stage::Docked => {}
        }
        emit(s.kind, "granted", None);
        self.look.granted = true;
        if !s.shown {
            s.shown = true;
            fade_in();
        }
        let hold = Duration::from_millis(900);
        s.stage = Stage::Granted { until: Instant::now() + hold };
        tx.wake_after(hold);
    }

    /// Fades out (the window's alpha, with the content shrinking and blurring) and ends.
    fn leave(&mut self, tx: &mut Tx) {
        let Some(s) = self.session.as_mut() else { return };
        s.stage = Stage::Leaving { back: false };
        if !s.shown {
            self.end_now();
            return;
        }
        self.leave = None;
        if let Some(dock) = DOCK.get() {
            dock.fade_out(Duration::from_secs_f64(FADE_OUT));
        }
        tx.wake_after(Duration::ZERO);
    }

    fn cancel(&mut self, reason: &str, tx: &mut Tx) -> bool {
        let Some(s) = self.session.as_mut() else { return false };
        match s.stage {
            Stage::Granted { .. } | Stage::Leaving { .. } => return false,
            Stage::Checking | Stage::Opening { .. } | Stage::Flying => {
                emit(s.kind, "cancelled", Some(reason));
                self.end_now();
                return false;
            }
            Stage::Docked => {}
        }
        emit(s.kind, "cancelled", Some(reason));
        match (reason, s.from, s.card) {
            ("back", Some(from), Some(card)) if !self.look.reduce => {
                s.stage = Stage::Leaving { back: true };
                let screens = screens::current();
                let top = screens::index_at(&screens, mid(from)).map_or(f64::MIN, |i| screens[i].visible[1]);
                fly(FlightRun::new(from, card, top, RETURN, true), self.look.clone());
            }
            _ => self.leave(tx),
        }
        false
    }

    /// The Back flight reached the button.
    fn flown_back(&mut self) {
        if self.session.as_ref().is_some_and(|s| s.stage == Stage::Leaving { back: true }) {
            self.session = None;
            self.leave = None;
        }
    }

    /// The docked content during the exit fade: with motion, a 0.96 scale and 3 pt blur.
    fn exit_fx(&self) -> Fx {
        let Some(start) = self.leave.filter(|_| !self.look.reduce) else { return Fx::REST };
        let k = StepSpring { duration: FADE_OUT, bounce: 0.0 }.value((self.now - start).max(0.0));
        Fx { opacity: 1.0, scale: 1.0 - 0.04 * k, blur: 3.0 * k }
    }
}

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
static DOCK: OnceLock<OverlayHandle<Panel>> = OnceLock::new();
static FLIGHT_OVERLAY: OnceLock<OverlayHandle<Panel>> = OnceLock::new();

fn rect_placement(r: Rect) -> Option<OverlayPlacement> {
    Some(OverlayPlacement::Rect { x: r[0] as f32, y: r[1] as f32, width: r[2] as f32, height: r[3] as f32 })
}

fn place_dock(card: Rect) {
    if let Some(dock) = DOCK.get() {
        dock.set_placement(rect_placement(inset(card, -MARGIN)));
    }
}

fn fade_in() {
    if let Some(dock) = DOCK.get() {
        dock.fade_in(Duration::from_secs_f64(FADE));
    }
}

fn fade_out() {
    if let Some(dock) = DOCK.get() {
        dock.fade_out(Duration::from_secs_f64(FADE));
    }
}

fn fly(run: FlightRun, look: Look) {
    let Some(flight) = FLIGHT_OVERLAY.get() else { return };
    flight.set_placement(rect_placement(run.window));
    flight.update(move |f, tx| {
        f.look = look;
        f.flight = Some(run);
        tx.wake_after(Duration::ZERO);
    });
    flight.show();
}

fn emit(kind: Kind, event: &str, reason: Option<&str>) {
    let reason = reason.map(|r| format!(" ({r})")).unwrap_or_default();
    crate::log::scrawl_log(format!("permission-panel: {event} {}{reason}", kind.raw()));
}

fn poll_grant(s: &mut Session, reshow: bool) {
    if s.checking {
        return;
    }
    s.checking = true;
    s.last_poll = Some(Instant::now());
    check_fresh(s.id, s.kind, reshow);
}

/// Reads the grant in a new `scrawl-helper` process, off the main thread
/// (`CGPreflightScreenCaptureAccess` keeps its first answer per process).
fn check_fresh(id: u64, kind: Kind, reshow: bool) {
    let _ = std::thread::Builder::new().name("permission-check".into()).spawn(move || {
        let granted = fresh_grant(kind);
        if let Some(dock) = DOCK.get() {
            dock.update(move |s, tx| s.checked(id, granted, reshow, tx));
        }
    });
}

fn fresh_grant(kind: Kind) -> bool {
    let Ok(helper) = crate::helper::path() else { return false };
    let Ok(out) = Command::new(helper)
        .args(["permission-panel", "--check", kind.raw()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    serde_json::from_slice::<serde_json::Value>(&out.stdout)
        .ok()
        .and_then(|v| v.get("granted")?.as_bool())
        .unwrap_or(false)
}

/// The .app that contains Scrawl, since that bundle is what TCC grants. Outside an app (a debug
/// build) the installed Scrawl.
fn app_bundle() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let exe = exe.canonicalize().unwrap_or(exe);
        if let Some(app) = exe.ancestors().find(|a| a.extension().is_some_and(|e| e.eq_ignore_ascii_case("app"))) {
            return app.to_path_buf();
        }
    }
    let installed = std::env::var_os("HOME").map(|h| Path::new(&h).join("Applications/Scrawl.app"));
    installed.filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from("/Applications/Scrawl.app"))
}

// MARK: Settings window

/// System Settings' processes, by bundle id (its window owner name is localized).
fn settings_pids() -> Vec<i32> {
    use objc2_app_kit::NSRunningApplication;
    use objc2_foundation::NSString;
    let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(SETTINGS_BUNDLE));
    apps.iter().map(|a| a.processIdentifier()).collect()
}

fn settings_windows() -> Vec<crate::windows::RawWindow> {
    let pids = settings_pids();
    if pids.is_empty() {
        return Vec::new();
    }
    crate::windows::raw_windows().into_iter().filter(|w| pids.contains(&w.pid)).collect()
}

/// The largest ordinary Settings window: its number and global bounds.
fn settings_main() -> Option<(u32, Rect)> {
    settings_windows()
        .into_iter()
        .filter(|w| w.layer == 0 && w.bounds[2] >= 300.0 && w.bounds[3] >= 200.0)
        .max_by(|a, b| (a.bounds[2] * a.bounds[3]).total_cmp(&(b.bounds[2] * b.bounds[3])))
        .map(|w| (w.id, w.bounds))
}

/// Sheets and alerts (the Quit & Reopen prompt) add windows; their count dropping back means one
/// was dismissed.
fn settings_count() -> usize {
    settings_windows().len()
}

/// One window's current bounds while it's on screen, cheaper than listing every window.
fn window_frame(id: u32) -> Option<Rect> {
    use core_foundation::array::CFArray;
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use std::ffi::c_void;

    // CGWindowIDs travel in the array as pointer-sized values, not CF objects.
    let ids = CFArray::<*const c_void>::from_copyable(&[id as usize as *const c_void]);
    // SAFETY: a valid CFArray of window numbers; the result follows the create rule.
    let raw = unsafe { core_graphics::window::CGWindowListCreateDescriptionFromArray(ids.as_concrete_TypeRef()) };
    if raw.is_null() {
        return None;
    }
    // SAFETY: non-null and owned (create rule).
    let list: CFArray<*const c_void> = unsafe { CFArray::wrap_under_create_rule(raw) };
    let first = *list.get(0)?;
    if first.is_null() {
        return None;
    }
    // SAFETY: each element is a CFDictionary the array keeps alive.
    let info: CFDictionary<CFString, CFType> = unsafe { CFDictionary::wrap_under_get_rule(first as CFDictionaryRef) };
    let onscreen = info
        .find(CFString::from_static_string("kCGWindowIsOnscreen"))
        .and_then(|v| v.downcast::<CFBoolean>())
        .is_some_and(bool::from);
    if !onscreen {
        return None;
    }
    let bounds = info.find(CFString::from_static_string("kCGWindowBounds"))?.downcast::<CFDictionary>()?;
    // SAFETY: the bounds dictionary is kept alive by `info`.
    let bounds: CFDictionary<CFString, CFType> = unsafe { CFDictionary::wrap_under_get_rule(bounds.as_concrete_TypeRef()) };
    let n = |k: &'static str| bounds.find(CFString::from_static_string(k)).and_then(|v| v.downcast::<CFNumber>()?.to_f64());
    Some([n("X")?, n("Y")?, n("Width")?, n("Height")?])
}

/// Whether Settings, Scrawl or one of Scrawl's helpers is in front. Cached per frontmost pid.
fn frontmost_ours(cache: &mut Option<(i32, bool)>, app: &Path) -> bool {
    use objc2_app_kit::NSWorkspace;
    let Some(front) = NSWorkspace::sharedWorkspace().frontmostApplication() else { return false };
    let pid = front.processIdentifier();
    if let Some((cached, ours)) = *cache
        && cached == pid
    {
        return ours;
    }
    let ours = pid == std::process::id() as i32
        || front.bundleIdentifier().is_some_and(|id| id.to_string() == SETTINGS_BUNDLE)
        || front
            .executableURL()
            .and_then(|u| u.path())
            .is_some_and(|p| Path::new(&p.to_string()).starts_with(app));
    *cache = Some((pid, ours));
    ours
}

// MARK: Polls

/// The docked overlay's poll is the controller.
fn dock_poll(s: &mut Panel, now: f64, tx: &mut Tx) -> bool {
    s.now = now;
    s.tick(now, tx)
}

/// Steps the flight from the frame clock, then hands off: forward, the docked window shows and
/// the flight window hides in one turn; back, the flight ends on the button.
fn flight_poll(s: &mut Panel, now: f64, tx: &mut Tx) -> bool {
    let Some(run) = s.flight.as_mut().filter(|r| !r.ended) else { return false };
    let started = *run.started.get_or_insert_with(|| {
        if run.reverse
            && let Some(dock) = DOCK.get()
        {
            dock.hide();
        }
        now
    });
    s.now = now;
    tx.instant();
    if now - started >= run.spring.settle() {
        run.ended = true;
        let (dock, flight) = (DOCK.get(), FLIGHT_OVERLAY.get());
        if run.reverse {
            if let Some(dock) = dock {
                dock.update(|d, _| d.flown_back());
            }
        } else if let Some(dock) = dock {
            dock.show();
            dock.update(|d, tx| d.landed(tx));
        }
        if let Some(flight) = flight {
            flight.hide();
        }
    } else {
        tx.wake_after(Duration::ZERO);
    }
    true
}

// MARK: View

#[derive(Clone, Copy, Debug, PartialEq)]
struct Fx {
    opacity: f64,
    scale: f64,
    blur: f64,
}

impl Fx {
    const REST: Fx = Fx { opacity: 1.0, scale: 1.0, blur: 0.0 };

    fn resolve(k: f64) -> Fx {
        Fx { opacity: k, scale: 0.96 + 0.04 * k, blur: 3.0 * (1.0 - k) }
    }
}

fn view(s: &Panel, cx: &mut Cx<Panel>) -> El {
    let dark = cx.system_dark();
    let root = el().key("root").w_full().h_full();
    let (w, h) = s.look.layout.size();
    match s.role {
        Role::Dock => {
            let card = card(&s.look, dark, [MARGIN, MARGIN, w, h], Layout::RADIUS, 0.0, 1.0, s.exit_fx(), true);
            root.child(card)
        }
        Role::Flight => {
            let Some(run) = s.flight else { return root };
            let step = run.step(run.started.map_or(0.0, |t| s.now - t));
            let r = step.rect;
            let local = [r[0] - run.window[0], r[1] - run.window[1], r[2], r[3]];
            let card = card(&s.look, dark, local, step.radius, step.lift, step.alpha, Fx::resolve(step.resolve), false);
            root.child(card)
        }
        Role::Preview => {
            let theme = Theme { dark };
            root.fill(theme.sidebar().color())
                .child(card(&s.look, dark, [MARGIN, MARGIN, w, h], Layout::RADIUS, 0.0, 1.0, Fx::REST, false))
        }
    }
}

fn ink(dark: bool, a: f32) -> Rgba {
    if dark { Rgba::WHITE.alpha(a) } else { Rgba::BLACK.alpha(a) }
}

fn primary(dark: bool) -> Rgba {
    ink(dark, 0.85)
}

fn secondary(dark: bool) -> Rgba {
    ink(dark, if dark { 0.55 } else { 0.5 })
}

fn green(dark: bool) -> Rgba {
    if dark { Rgba::hex(0x30D158) } else { Rgba::hex(0x34C759) }
}

fn springs(duration: f32) -> SpringPair {
    SpringPair::new(Spring::new(duration, 0.0), Spring::new(duration, 0.0))
}

/// One card surface: two Core Animation shadows that deepen with `lift`, Liquid Glass frosted
/// with the window background, and the content (laid out at the card's full size, centred, as
/// the flight resizes it) clipped to continuous corners.
#[allow(clippy::too_many_arguments)]
fn card(look: &Look, dark: bool, rect: Rect, radius: f64, lift: f64, alpha: f64, fx: Fx, interactive: bool) -> El {
    let [x, y, w, h] = rect.map(|v| v as f32);
    let (cw, ch) = look.layout.size();
    let (cw, ch) = (cw as f32, ch as f32);
    let (radius, lift) = (radius as f32, lift as f32);
    let frost = Theme { dark }.window().alpha(0.62).color();
    let content = content(look, dark, interactive)
        .key("content")
        .absolute((w - cw) / 2.0, (h - ch) / 2.0)
        .size(cw, ch)
        .opacity(fx.opacity as f32)
        .scale(fx.scale as f32)
        .blur(fx.blur as f32);
    let frosted = el()
        .key("frost")
        .w_full()
        .h_full()
        .radius(radius)
        .continuous_corners()
        .clip_rounded(radius)
        .fill(frost)
        .child(content);
    let black = Rgba::BLACK;
    let card = el()
        .key("card")
        .absolute(x, y)
        .size(w, h)
        .radius(radius)
        .continuous_corners()
        .box_shadow(Shadow::core_animation(0.0, 0.5, 1.5, black.alpha(0.3 - 0.1 * lift).color()))
        .box_shadow(Shadow::core_animation(0.0, 8.0 + 12.0 * lift, 18.0 + 14.0 * lift, black.alpha(0.32 + 0.14 * lift).color()))
        .system_material(SystemMaterial::Glass)
        .opacity(alpha as f32)
        .child(frosted);
    if interactive { card.hit() } else { card }
}

fn content(look: &Look, dark: bool, interactive: bool) -> El {
    let row_w = look.layout.row_width as f32;
    let (gutter, row_y) = (Layout::GUTTER as f32, Layout::ROW_Y as f32);
    el().child(instruction(look, dark, row_w)).child(app_row(look, dark, row_w, interactive)).child(
        back_button(look, dark, interactive)
            .absolute((gutter - 28.0) / 2.0 + 1.0, row_y + (Layout::ROW as f32 - 28.0) / 2.0),
    )
}

/// "Drag Scrawl to the list above to allow Screen Recording", swapping to "Scrawl is allowed".
fn instruction(look: &Look, dark: bool, row_w: f32) -> El {
    let (x, y) = (Layout::GUTTER as f32, Layout::TOP as f32);
    let height = Layout::INSTRUCTION as f32;
    let icon_swap = Transition::opacity().combined(Transition::scale(0.6)).springs(springs(0.3));
    let text_swap = Transition::opacity().combined(Transition::blur(3.0)).springs(springs(0.3));
    let icon = if look.granted {
        let check = symbol(&icons::CHECK_CIRCLE_FILL, 17.0, weight::SEMIBOLD, green(dark).color());
        // The palette's white check: a disc under the cut-out glyph.
        el().key("check")
            .child(el().absolute(7.5, 8.5).size(11.0, 11.0).radius(5.5).fill(Color::WHITE))
            .child(el().absolute(0.0, 0.0).size(Layout::ICON as f32, height).center().child(check))
    } else {
        let accent = Theme { dark }.accent().color();
        el().key("arrow").center().child(symbol(&icons::ARROW_UP, 18.0, weight::BOLD, accent))
    };
    let icon = icon
        .absolute(x + Layout::ICON_LEADING as f32, y)
        .size(Layout::ICON as f32, height)
        .transition(icon_swap);

    let (strong, soft) = (primary(dark).color(), secondary(dark).color());
    let mut line = String::new();
    let mut spans = Vec::new();
    let mut push = |line: &mut String, part: &str, bold: bool| {
        let start = line.len();
        line.push_str(part);
        if bold {
            let mut span = Span::new(start, line.len());
            span.weight = Some(500.0);
            span.color = Some(strong);
            spans.push(span);
        }
    };
    let key = if look.granted {
        push(&mut line, &look.name, true);
        push(&mut line, " is allowed", false);
        "allowed"
    } else {
        push(&mut line, "Drag ", false);
        push(&mut line, &look.name, true);
        push(&mut line, " to the list above to allow ", false);
        push(&mut line, look.kind.title(), true);
        "drag"
    };
    let room = row_w - Layout::TEXT_LEADING as f32;
    // SwiftUI's `.minimumScaleFactor(0.85)`; medium runs are a little wider than regular.
    let wide = measure(&line, style(13.0, 400.0)) * 1.02;
    let size = if wide > room { 13.0 * (room / wide).max(0.85) } else { 13.0 };
    let words = rich(line, spans).font(style(size, 400.0)).color(soft).single_line();
    let words = el()
        .key(key)
        .absolute(x + Layout::TEXT_LEADING as f32, y)
        .size(room, height)
        .items_center()
        .child(words)
        .transition(text_swap);
    el().key("instruction").absolute(0.0, 0.0).size(0.0, 0.0).child(icon).child(words)
}

/// The app row the list will take: icon and name; dragged, it carries the app bundle.
fn app_row(look: &Look, dark: bool, row_w: f32, interactive: bool) -> El {
    let lifted = look.dragging;
    let icon = Layout::ICON as f32;
    let row_h = Layout::ROW as f32;
    let image = match &look.icon {
        Some(pixels) => el().size(icon, icon).pixels(pixels),
        None => el().size(icon, icon),
    };
    let body = el()
        .key("row-body")
        .w_full()
        .h_full()
        .opacity(if lifted { 0.28 } else { 1.0 })
        .opacity_motion(Spring::new(0.25, 0.0))
        .child(image.absolute(Layout::ICON_LEADING as f32, (row_h - icon) / 2.0))
        .child(
            el().absolute(Layout::TEXT_LEADING as f32, 0.0).h(row_h).items_center().child(
                text(look.name.clone()).font(style(13.0, 400.0)).color(primary(dark).color()).single_line(),
            ),
        );
    let radius = Layout::ROW_RADIUS as f32;
    let row = el()
        .key("row")
        .absolute(Layout::GUTTER as f32, Layout::ROW_Y as f32)
        .size(row_w, row_h)
        .radius(radius)
        .continuous_corners()
        .fill(ink(dark, if look.hovering && !lifted { 0.1 } else { 0.065 }).color())
        .border(1.0, ink(dark, 0.08).color())
        .motion(Spring::new(0.25, 0.0))
        .opacity(if look.granted { 0.5 } else { 1.0 })
        .opacity_motion(Spring::new(0.3, 0.0))
        .child(body);
    if !interactive {
        return row;
    }
    // The picture under the pointer, opaque so it reads over any window it crosses.
    let backing = ink(dark, 0.06).over(Theme { dark }.window());
    let app = look.app.clone();
    row.hit()
        .cursor(Cursor::Grab)
        .drag_threshold(3.0)
        .drag_out(move |_: &Panel| Some(DragItem::new().file(app.clone()).opaque(backing)))
        .on_drag_start(|s: &mut Panel, _tx: &mut Tx| s.look.dragging = true)
        .on_drag_end(|s: &mut Panel, outcome, tx: &mut Tx| s.drag_ended(outcome, tx))
        .on_hover_change(|s: &mut Panel, inside: bool, _tx: &mut Tx| s.look.hovering = inside)
}

fn back_button(look: &Look, dark: bool, interactive: bool) -> El {
    let hover = look.back_hover;
    let tint = if hover { primary(dark) } else { secondary(dark) };
    let button = el()
        .key("back")
        .size(28.0, 28.0)
        .radius(14.0)
        .fill(ink(dark, if hover { 0.12 } else { 0.07 }).color())
        .motion(Spring::new(0.25, 0.0))
        .center()
        .child(symbol(&icons::CHEVRON_LEFT, 12.0, weight::SEMIBOLD, tint.color()))
        .opacity(if look.granted { 0.0 } else { 1.0 })
        .opacity_motion(Spring::new(0.3, 0.0))
        .help("Back");
    if !interactive {
        return button;
    }
    button
        .hit()
        .on_press_tx(|s: &mut Panel, tx: &mut Tx| {
            s.cancel("back", tx);
        })
        .on_hover_change(|s: &mut Panel, inside: bool, _tx: &mut Tx| s.look.back_hover = inside)
}

// MARK: Hosting

/// Adds the two hidden overlays to Scrawl's Rune app when the `runePermissionPanel` experiment is
/// on; `open` uses them.
pub(crate) fn install<S: 'static>(app: &mut App<S>) {
    if !crate::handoff::experiment("runePermissionPanel") {
        return;
    }
    let options = |input| OverlayOptions {
        display: DisplayTarget::Primary,
        placement: rect_placement([0.0, 0.0, 200.0, 100.0]),
        input,
        keys: KeyPolicy::None,
        prewarm: true,
        ..Default::default()
    };
    let flight = app.overlay(
        OverlaySpec::new("Scrawl permission flight", (200.0, 100.0), view)
            .options(options(OverlayInput::None))
            .macos_level(4)
            .srgb_blend(true)
            .poll(flight_poll),
        Panel::new(Role::Flight),
    );
    let (w, h) = Layout::new(740.0).size();
    let size = ((w + 2.0 * MARGIN) as f32, (h + 2.0 * MARGIN) as f32);
    let dock = app.overlay(
        OverlaySpec::new("Scrawl permission panel", size, view)
            .options(options(OverlayInput::Regions))
            .macos_level(3)
            .srgb_blend(true)
            .poll(dock_poll),
        Panel::new(Role::Dock),
    );
    let _ = FLIGHT_OVERLAY.set(flight);
    let _ = DOCK.set(dock);
}

/// Opens System Settings at `permission`'s list with the Rune drag panel, flying from `from`
/// (the button, global top-left points). False when the panel isn't installed or the
/// permission isn't dragged into a list; the caller then launches the helper's panel.
pub(crate) fn open(permission: &str, from: Option<[f32; 4]>) -> bool {
    let (Some(kind), Some(dock)) = (Kind::parse(permission), DOCK.get()) else { return false };
    let from = from.map(|r| r.map(f64::from));
    dock.update(move |s, tx| s.start(kind, from, tx));
    true
}

/// The row turned on: the panel says so and goes.
pub(crate) fn granted(permission: &str) {
    let (Some(kind), Some(dock)) = (Kind::parse(permission), DOCK.get()) else { return };
    dock.update(move |s, tx| {
        if s.session.as_ref().is_some_and(|session| session.kind == kind) {
            s.finish(tx);
        }
    });
}

// MARK: Renders

fn preview(kind: Kind) -> Panel {
    let mut p = Panel::new(Role::Preview);
    p.look = Look::new(kind);
    p.look.reduce = false;
    p
}

fn preview_init() -> Panel {
    preview(Kind::ScreenRecording)
}

macro_rules! scenes {
    ($($name:literal => $apply:expr),* $(,)?) => {
        const SCENE_NAMES: &[&str] = &[$($name),*];
        static SCENES: &[(&str, fn(&mut Panel))] = &[$(($name, $apply)),*];
    };
}

scenes!(
    "screen-recording" => |_| {},
    "accessibility" => |p| *p = preview(Kind::Accessibility),
    "hover" => |p| p.look.hovering = true,
    "back-hover" => |p| p.look.back_hover = true,
    "dragging" => |p| p.look.dragging = true,
    "granted" => |p| p.look.granted = true,
);

/// `--snap` one sample (`permission-panel-<name>`, `@light` for the light appearance) headless,
/// over a stand-in for the Settings window (the glass draws a neutral tint headless).
pub(crate) fn run_headless(scene: &str, mut args: Vec<String>) {
    let name = scene.trim_start_matches("permission-panel-");
    let (name, light) = match name.strip_suffix("@light") {
        Some(n) => (n, true),
        None => (name.strip_suffix("@dark").unwrap_or(name), false),
    };
    if let Some(i) = args.iter().position(|a| a == "--scene") {
        args[i + 1] = name.to_owned();
    }
    args.extend(["--theme".to_owned(), if light { "light" } else { "dark" }.to_owned()]);
    let (w, h) = Layout::new(740.0).size();
    let fonts = fonts();
    let spec = AppSpec {
        scenes: SCENES,
        fonts,
        families: families(fonts),
        ..AppSpec::new("Scrawl permission panel", ((w + 2.0 * MARGIN) as f32, (h + 2.0 * MARGIN) as f32), preview_init, view)
    };
    App::new(spec).srgb_blend(true).args(args).run();
}

pub(crate) fn render_names() -> Vec<String> {
    let mut out: Vec<String> = SCENE_NAMES.iter().map(|n| format!("permission-panel-{n}")).collect();
    out.extend(["permission-panel-screen-recording@light".to_owned(), "permission-panel-granted@light".to_owned()]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(settings: Rect, visible: Rect) -> (Rect, Dock) {
        dock(settings, visible, Layout::new(settings[2]))
    }

    /// `scrawl-helper permission-panel --plan --settings … --visible …` (AppKit coordinates).
    #[test]
    fn docks_where_the_swift_panel_does() {
        let screen = [0.0, 25.0, 1512.0, 950.0];
        let cases: &[(Rect, Rect, Dock, Rect)] = &[
            ([300.0, 500.0, 740.0, 600.0], screen, Dock::Below, [500.0, 395.0, 532.0, 95.0]),
            ([300.0, 100.0, 740.0, 800.0], screen, Dock::Inside, [500.0, 120.0, 532.0, 95.0]),
            ([300.0, 60.0, 740.0, 860.0], screen, Dock::Inside, [500.0, 80.0, 532.0, 95.0]),
            ([700.0, 60.0, 740.0, 860.0], screen, Dock::Inside, [900.0, 80.0, 532.0, 95.0]),
            ([0.0, 60.0, 1500.0, 860.0], screen, Dock::Inside, [200.0, 80.0, 704.0, 95.0]),
            ([100.0, 400.0, 1000.0, 500.0], screen, Dock::Below, [300.0, 295.0, 704.0, 95.0]),
            ([300.0, 30.0, 740.0, 300.0], screen, Dock::Clamped, [500.0, 33.0, 532.0, 95.0]),
            ([900.0, 30.0, 500.0, 300.0], screen, Dock::Left, [506.0, 33.0, 384.0, 95.0]),
            ([100.0, 30.0, 500.0, 300.0], screen, Dock::Right, [610.0, 33.0, 384.0, 95.0]),
            ([-1600.0, 200.0, 740.0, 600.0], [-1920.0, 0.0, 1920.0, 1055.0], Dock::Below, [-1400.0, 95.0, 532.0, 95.0]),
            ([1000.0, 700.0, 700.0, 280.0], screen, Dock::Below, [1012.0, 595.0, 492.0, 95.0]),
        ];
        for &(settings, visible, how, frame) in cases {
            assert_eq!(plan(settings, visible), (frame, how), "settings {settings:?}");
        }
    }

    #[test]
    fn docks_the_same_in_global_coordinates() {
        // The first case above, on a 982 pt primary display, in top-left coordinates.
        let flip = |r: Rect| screens::flip(r, 982.0);
        let screens = [screens::Screen {
            id: 1,
            frame: [0.0, 0.0, 1512.0, 982.0],
            visible: flip([0.0, 25.0, 1512.0, 950.0]),
            scale: 2.0,
            safe_top: 0.0,
        }];
        let settings = flip([300.0, 500.0, 740.0, 600.0]);
        let (card, how) = dock_for(settings, Layout::new(740.0), &screens);
        assert_eq!((card, how), (flip([500.0, 395.0, 532.0, 95.0]), Dock::Below));
    }

    #[test]
    fn card_matches_the_list_column() {
        assert_eq!(Layout::new(740.0).size(), (532.0, 95.0));
        assert_eq!(Layout::new(400.0).row_width, 320.0);
        assert_eq!(Layout::new(2000.0).row_width, 640.0);
        assert_eq!(Layout::ROW_Y, 39.0);
        assert_eq!(Layout::TEXT_LEADING, 46.0);
    }

    #[test]
    fn spring_rises_without_overshoot_and_settles() {
        let s = FLIGHT;
        assert_eq!(s.value(0.0), 0.0);
        let mut last = 0.0;
        for i in 1..=100 {
            let v = s.value(s.settle() * f64::from(i) / 100.0);
            assert!(v >= last && v <= 1.0);
            last = v;
        }
        assert!(1.0 - last < 2e-4);
        let bouncy = StepSpring { duration: 0.5, bounce: 0.3 };
        assert!((0..200).any(|i| bouncy.value(f64::from(i) * 0.01) > 1.0));
    }

    #[test]
    fn flight_arcs_from_the_button_to_the_dock() {
        let source = [100.0, 600.0, 120.0, 28.0];
        let target = [500.0, 300.0, 532.0, 95.0];
        let run = FlightRun::new(source, target, 25.0, FLIGHT, false);
        // Midway across, 160 pt above the higher end.
        assert_eq!(run.control, [(160.0 + 766.0) / 2.0, 347.5 - 160.0]);
        let start = run.step(0.0);
        assert_eq!((start.rect, start.radius, start.alpha, start.resolve), (source, 14.0, 0.0, 0.0));
        let end = run.step(FLIGHT.settle());
        assert!(end.rect.iter().zip(target).all(|(a, b)| (a - b).abs() < 0.01));
        assert!((end.radius - 18.0).abs() < 0.01 && end.lift.abs() < 0.01);
        assert!((end.alpha - 1.0).abs() < 1e-9 && (end.resolve - 1.0).abs() < 1e-9);
        // The window holds the whole path, padded.
        let w = run.window;
        assert!(w[0] <= source[0] - FLIGHT_PAD && w[1] <= run.control[1] - FLIGHT_PAD);
        assert!(w[0] + w[2] >= target[0] + target[2] + FLIGHT_PAD && w[1] + w[3] >= source[1] + source[3] + FLIGHT_PAD);

        let back = FlightRun::new(source, target, 25.0, RETURN, true);
        assert_eq!(back.step(0.0).rect, target);
        assert!(back.step(RETURN.settle()).alpha < 1e-6);
    }

    #[test]
    fn arc_stays_below_the_top_of_the_screen() {
        let source = [100.0, 200.0, 120.0, 28.0];
        let target = [700.0, 120.0, 532.0, 95.0];
        // Top of the visible frame at 25: the card's centre stays 47.5 + 8 below it.
        assert_eq!(control(source, target, 25.0)[1], 25.0 + 47.5 + 8.0);
        // Already higher than that: the higher end, no dip.
        assert_eq!(control(source, [700.0, 0.0, 532.0, 95.0], 25.0)[1], 47.5);
    }

    #[test]
    fn integral_matches_cgrect_integral() {
        assert_eq!(integral([10.4, 20.6, 100.2, 50.0]), [10, 20, 101, 51]);
        assert_eq!(integral([10.0, 20.0, 100.0, 50.0]), integral([10.0, 20.0, 100.0, 50.0]));
    }

    #[test]
    fn kinds_round_trip() {
        for kind in [Kind::ScreenRecording, Kind::Accessibility] {
            assert_eq!(Kind::parse(kind.raw()), Some(kind));
            assert!(kind.settings_url().starts_with("x-apple.systempreferences:"));
        }
        assert_eq!(Kind::parse("microphone"), None);
    }
}
