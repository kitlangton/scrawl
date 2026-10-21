//! First-run onboarding: a welcome, one screen on how Scrawl works, one screen with every
//! permission it needs, and a first capture. The window is SwiftUI in `scrawl-helper onboarding`;
//! this module decides what it shows (`Flow`) and remembers progress in
//! `~/.config/scrawl/config.json` (`"onboarding"`, see `Progress`).
//!
//! Screen Recording, Microphone and Accessibility are required and asked for together on the
//! permissions screen, Screen Recording first. Camera (the cabled iPhone) is asked for the first
//! time it's used, in the same window with just that row (`Flow::fix`). On macOS 15, where
//! transcripts come from SFSpeechRecognizer, Speech Recognition is an optional fourth row, so
//! its prompt comes up here rather than after the first capture. Until the required ones
//! are on, ⌥⇧S starts nothing: it brings this window forward once the keys are up (`Gate`).
//!
//! A Screen Recording grant may only reach a running copy of Scrawl after it relaunches, and
//! macOS offers "Quit & Reopen" the moment it's granted. Whatever the user picks there, nobody
//! reopens Scrawl by hand: when a fresh helper says it's granted but this process still can't
//! capture, the window relaunches Scrawl by itself (once, see `Progress::relaunched_at`) and the
//! tour comes back on the same screen. Input Monitoring is never asked for: clicks come from an
//! AppKit mouse monitor that needs no permission, keys from an Accessibility event tap.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    ScreenRecording,
    Microphone,
    /// No longer asked for, shown or read (macOS wants a relaunch after granting it). Kept so
    /// a config written by 0.3.x, which lists it under `granted`, still loads.
    InputMonitoring,
    Accessibility,
    Camera,
    /// SFSpeechRecognizer's own permission, for transcripts on macOS 15. Optional, and only on
    /// the screen when the helper reports it (SpeechAnalyzer on macOS 26 needs none).
    SpeechRecognition,
}

impl Permission {
    /// What a capture needs, in the order the checklist shows them. Screen Recording comes first
    /// because it's the one that may relaunch Scrawl. Accessibility drives the key event tap
    /// (⌘Z, Esc) and ⌥⇧V's paste. Camera is asked for when first used.
    pub const REQUIRED: [Permission; 3] = [
        Permission::ScreenRecording,
        Permission::Microphone,
        Permission::Accessibility,
    ];

    /// The tour's checklist: the required ones, then Speech Recognition where it's asked for.
    pub const TOUR: [Permission; 4] = [
        Permission::ScreenRecording,
        Permission::Microphone,
        Permission::Accessibility,
        Permission::SpeechRecognition,
    ];

    pub fn required(self) -> bool {
        Self::REQUIRED.contains(&self)
    }

    /// Whether this Mac asks for it at all: Speech Recognition only when the helper reports it
    /// (macOS before 26).
    pub fn asked(self, grants: Option<&Grants>) -> bool {
        self != Permission::SpeechRecognition
            || grants.is_some_and(|g| g.statuses.contains_key(&self))
    }

    /// Privacy lists the app can be dragged into: Grant flies the drag panel to System
    /// Settings (`scrawl-helper permission-panel`) instead of raising Apple's prompt.
    pub fn draggable(self) -> bool {
        matches!(
            self,
            Permission::ScreenRecording | Permission::Accessibility
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Step {
    Welcome,
    /// A short silent demo on a pretend app: tap ⌥⇧S, talk, hold ⇧ and move to mark, click,
    /// tap ⌥⇧S to finish, paste into the agent. It can be skipped, and a mark can be tried on
    /// it (drawn in the window only, nothing is recorded). See `Tutorial`.
    #[serde(alias = "record", alias = "draw", alias = "finish")]
    HowItWorks,
    /// Every permission on one screen.
    #[serde(
        alias = "screen-recording",
        alias = "microphone",
        alias = "input-monitoring",
        alias = "accessibility",
        alias = "camera"
    )]
    Permissions,
    TryIt,
    /// After the first capture: it's on your clipboard.
    Done,
}

impl Step {
    const TOUR: [Step; 5] = [
        Step::Welcome,
        Step::HowItWorks,
        Step::Permissions,
        Step::TryIt,
        Step::Done,
    ];

    fn position(self) -> usize {
        Self::TOUR.iter().position(|s| *s == self).unwrap_or(0)
    }

    fn next(self) -> Step {
        Self::TOUR[(self.position() + 1).min(Self::TOUR.len() - 1)]
    }
}

/// What happened with the demo on the how-it-works screen. Later variants win: a demo skipped
/// and then watched after Back is `Watched`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tutorial {
    /// Skip was pressed before the demo had played through.
    Skipped,
    /// The demo played through once: the button reads Continue.
    Watched,
    /// The user drew a mark of their own on the demo.
    Practiced,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    #[default]
    NotDetermined,
    Denied,
    Granted,
}

/// What macOS has granted Scrawl, from a fresh `scrawl-helper permissions`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grants {
    pub statuses: BTreeMap<Permission, Status>,
    /// Screen recording is granted, but this running copy of Scrawl can't use it until it
    /// relaunches.
    pub screen_needs_relaunch: bool,
}

impl Grants {
    pub fn status(&self, permission: Permission) -> Status {
        self.statuses.get(&permission).copied().unwrap_or_default()
    }

    pub fn granted(&self, permission: Permission) -> bool {
        self.status(permission) == Status::Granted
    }

    /// Granted and usable now (screen recording may need a relaunch first).
    fn usable(&self, permission: Permission) -> bool {
        self.granted(permission)
            && !(permission == Permission::ScreenRecording && self.screen_needs_relaunch)
    }
}

/// Once everything a fix asks for is on, its window shows the checks this long, then closes.
pub const AUTO_ADVANCE_MS: u64 = 800;

/// Screen Recording's check shows this long before Scrawl relaunches to use it.
pub const RELAUNCH_AFTER_MS: u64 = 1_000;

/// After relaunching itself, Scrawl doesn't do it again by itself for this long; the row offers
/// Reopen instead. Stops a loop if a fresh copy still can't capture.
pub const RELAUNCH_GUARD_MS: u64 = 120_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The whole tour, first launch or "Welcome to Scrawl…".
    Tour,
    /// Just the permissions screen, for permissions that are missing now: required ones that
    /// went away or that ⌥⇧S found off, or Accessibility or Camera on first use. Closes once
    /// they're on.
    Fix,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// The primary button.
    Continue,
    Back,
    /// "Set up later" / "Not now".
    Later,
    /// A permission row's button.
    Grant(Permission),
    /// The window was closed.
    Closed,
    /// "Open Settings" on the last screen.
    Settings,
    /// "Skip" under the demo, before it has played through: goes on like Continue.
    Skip,
    /// The demo played through once.
    Watched,
    /// The user drew a mark on the demo.
    Practiced,
    Grants {
        grants: Grants,
        now_ms: u64,
    },
    /// ⌥⇧S was pressed while a required permission is off.
    Blocked,
    CaptureStarted,
    CaptureEnded {
        completed: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    /// Ask macOS (the system prompt), the first time a row's button is pressed.
    Request(Permission),
    /// Already asked or denied: open its System Settings pane.
    OpenSettings(Permission),
    /// Relaunch Scrawl so Screen Recording works. The tour comes back at `resume`; a fix (no
    /// `resume`) opens again.
    Relaunch {
        resume: Option<Step>,
    },
    Hide,
    Show,
    Close,
    /// Close the tour and open the Settings window.
    Settings,
    /// Screen Recording works: list the screen's content once now, so macOS's "bypass the
    /// system private window picker" question comes up here, explained, rather than over the
    /// first recording.
    ConfirmScreen,
}

/// One row on the permissions screen.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub permission: Permission,
    pub status: Status,
    /// The button was pressed once already: it now opens System Settings.
    pub requested: bool,
    pub required: bool,
    /// Granted, and Scrawl is about to relaunch by itself to use it.
    pub reopening: bool,
    /// Granted, but this copy can't use it and won't relaunch by itself again: the row's
    /// button reopens Scrawl.
    pub relaunch: bool,
}

/// What the window shows, sent to the helper as JSON.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub step: Step,
    /// Position among the steps still to be shown, for the page dots.
    pub index: usize,
    pub count: usize,
    pub rows: Vec<Row>,
    /// Everything the permissions screen needs works: Continue is enabled.
    pub ready: bool,
    pub fix: bool,
    /// The demo's outcome so far; until it's `Watched` the button under it reads Skip.
    pub tutorial: Option<Tutorial>,
    /// macOS may be asking about its private window picker: say to choose Allow.
    pub picker_note: bool,
}

#[derive(Clone, Debug)]
pub struct Flow {
    mode: Mode,
    step: Step,
    grants: Option<Grants>,
    /// The rows on the permissions screen.
    asks: Vec<Permission>,
    requested: BTreeSet<Permission>,
    ready_at: Option<u64>,
    hidden: bool,
    /// May relaunch by itself once Screen Recording is granted (off right after a relaunch).
    auto_relaunch: bool,
    /// When a Screen Recording grant this copy can't use was first seen.
    stale_since: Option<u64>,
    /// A relaunch was asked for; never twice from one window.
    relaunching: bool,
    tutorial: Option<Tutorial>,
    /// The step `Effect::ConfirmScreen` was given on (once per window).
    screen_confirmed: Option<Step>,
}

impl Flow {
    /// The tour from `start`. Walking forward passes the permissions screen when everything on
    /// it works, and a later step never shows before it's done. A tour resumed on the
    /// permissions screen (after a relaunch) stays there, so the window comes back where it was.
    pub fn tour(start: Step, grants: Option<Grants>) -> Self {
        let mut flow = Self::new(Mode::Tour, start, Permission::TOUR.to_vec(), grants);
        flow.step = if start.position() > Step::Permissions.position()
            && flow.grants.is_some()
            && !flow.done()
        {
            Step::Permissions
        } else if start == Step::Permissions {
            start
        } else {
            flow.forward_from(start)
        };
        flow
    }

    /// The permissions screen with just `asks`.
    pub fn fix(asks: Vec<Permission>, grants: Option<Grants>) -> Self {
        Self::new(Mode::Fix, Step::Permissions, asks, grants)
    }

    fn new(mode: Mode, step: Step, asks: Vec<Permission>, grants: Option<Grants>) -> Self {
        Self {
            mode,
            step,
            grants,
            asks,
            requested: BTreeSet::new(),
            ready_at: None,
            hidden: false,
            auto_relaunch: true,
            stale_since: None,
            relaunching: false,
            tutorial: None,
            screen_confirmed: None,
        }
    }

    /// Whether this window may relaunch Scrawl by itself (`relaunch_allowed`).
    pub fn with_auto_relaunch(mut self, allowed: bool) -> Self {
        self.auto_relaunch = allowed;
        self
    }

    /// The demo's outcome from an earlier launch (`Progress::tutorial`).
    pub fn with_tutorial(mut self, tutorial: Option<Tutorial>) -> Self {
        self.tutorial = tutorial;
        self
    }

    pub fn tutorial(&self) -> Option<Tutorial> {
        self.tutorial
    }

    /// Records `outcome` unless something later already happened.
    fn note_tutorial(&mut self, outcome: Tutorial) {
        self.tutorial = self.tutorial.max(Some(outcome));
    }

    pub fn step(&self) -> Step {
        self.step
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Where a tour picks up if Scrawl quits now (macOS's own "Quit & Reopen" included).
    pub fn resume_point(&self) -> Option<Step> {
        (self.mode == Mode::Tour && self.step != Step::Done).then_some(self.step)
    }

    pub fn on(&mut self, input: Input) -> Effect {
        match input {
            Input::Continue => self.primary(),
            Input::Back => {
                self.step = self.back_from(self.step);
                Effect::None
            }
            Input::Later | Input::Closed => Effect::Close,
            Input::Settings if self.step == Step::Done => Effect::Settings,
            Input::Settings => Effect::None,
            Input::Skip | Input::Watched | Input::Practiced
                if self.step != Step::HowItWorks || self.mode == Mode::Fix =>
            {
                Effect::None
            }
            Input::Skip => {
                self.note_tutorial(Tutorial::Skipped);
                self.advance()
            }
            Input::Watched => {
                self.note_tutorial(Tutorial::Watched);
                Effect::None
            }
            Input::Practiced => {
                self.note_tutorial(Tutorial::Practiced);
                Effect::None
            }
            Input::Grant(permission) => self.grant(permission),
            Input::Grants { grants, now_ms } => {
                self.grants = Some(grants);
                self.tick(now_ms)
            }
            Input::Blocked => {
                if self.mode == Mode::Tour && self.step.position() < Step::Permissions.position() {
                    self.step = Step::Permissions;
                }
                Effect::Show
            }
            Input::CaptureStarted if !self.hidden => {
                self.hidden = true;
                Effect::Hide
            }
            Input::CaptureStarted => Effect::None,
            Input::CaptureEnded { completed } => {
                if completed && self.step == Step::TryIt {
                    self.step = Step::Done;
                }
                if std::mem::take(&mut self.hidden) {
                    Effect::Show
                } else {
                    Effect::None
                }
            }
        }
    }

    pub fn view(&self) -> View {
        let shown: Vec<Step> = Step::TOUR
            .into_iter()
            .filter(|s| *s != Step::Done)
            .filter(|s| *s == self.step || !self.passes(*s))
            .collect();
        View {
            step: self.step,
            index: shown.iter().position(|s| *s == self.step).unwrap_or(0),
            count: shown.len(),
            rows: self
                .shown()
                .map(|permission| {
                    let stale = permission == Permission::ScreenRecording && self.screen_relaunch();
                    Row {
                        permission,
                        status: self.status(permission),
                        requested: self.requested.contains(&permission),
                        required: permission.required(),
                        reopening: stale && self.auto_relaunch,
                        relaunch: stale && !self.auto_relaunch,
                    }
                })
                .collect(),
            ready: self.done(),
            fix: self.mode == Mode::Fix,
            tutorial: self.tutorial,
            picker_note: self.screen_confirmed == Some(self.step),
        }
    }

    fn status(&self, permission: Permission) -> Status {
        self.grants
            .as_ref()
            .map_or(Status::NotDetermined, |g| g.status(permission))
    }

    fn granted(&self, permission: Permission) -> bool {
        self.status(permission) == Status::Granted
    }

    fn usable(&self, permission: Permission) -> bool {
        self.grants.as_ref().is_some_and(|g| g.usable(permission))
    }

    /// The rows this Mac asks for.
    fn shown(&self) -> impl Iterator<Item = Permission> + '_ {
        self.asks
            .iter()
            .copied()
            .filter(|p| p.asked(self.grants.as_ref()))
    }

    /// What must be granted before Continue: a tour's required rows, every row of a fix.
    fn needed(&self) -> impl Iterator<Item = Permission> + '_ {
        self.shown()
            .filter(|p| self.mode == Mode::Fix || p.required())
    }

    /// Screen Recording is on the screen and granted, but this copy of Scrawl can't capture yet.
    fn screen_relaunch(&self) -> bool {
        self.asks.contains(&Permission::ScreenRecording)
            && self.granted(Permission::ScreenRecording)
            && self
                .grants
                .as_ref()
                .is_some_and(|g| g.screen_needs_relaunch)
    }

    /// Everything needed is granted and usable in this process.
    fn done(&self) -> bool {
        self.needed().all(|p| self.usable(p))
    }

    /// A step the tour passes: the permissions screen once everything on it works.
    fn passes(&self, step: Step) -> bool {
        step == Step::Permissions && self.done()
    }

    fn forward_from(&self, mut step: Step) -> Step {
        while self.passes(step) {
            step = step.next();
        }
        step
    }

    fn back_from(&self, step: Step) -> Step {
        if self.mode == Mode::Fix {
            return step;
        }
        let mut i = step.position();
        while i > 0 {
            i -= 1;
            if !self.passes(Step::TOUR[i]) {
                return Step::TOUR[i];
            }
        }
        step
    }

    fn advance(&mut self) -> Effect {
        self.ready_at = None;
        if self.mode == Mode::Fix {
            return Effect::Close;
        }
        self.step = self.forward_from(self.step.next());
        Effect::None
    }

    fn primary(&mut self) -> Effect {
        match self.step {
            Step::TryIt | Step::Done => Effect::Close,
            Step::Permissions if !self.done() => Effect::None,
            _ => self.advance(),
        }
    }

    fn grant(&mut self, permission: Permission) -> Effect {
        if self.step != Step::Permissions || !self.asks.contains(&permission) {
            return Effect::None;
        }
        // The fallback after a relaunch that didn't take: the row's button is Reopen.
        if permission == Permission::ScreenRecording && self.screen_relaunch() {
            return if self.auto_relaunch {
                Effect::None
            } else {
                self.relaunch()
            };
        }
        if self.granted(permission) {
            return Effect::None;
        }
        if self.requested.insert(permission)
            && self.status(permission) != Status::Denied
            && !permission.draggable()
        {
            Effect::Request(permission)
        } else {
            Effect::OpenSettings(permission)
        }
    }

    /// Relaunch so Screen Recording works, coming back to this same screen. Once per window.
    fn relaunch(&mut self) -> Effect {
        if self.relaunching || self.step != Step::Permissions || !self.screen_relaunch() {
            return Effect::None;
        }
        self.relaunching = true;
        let resume = match self.mode {
            Mode::Tour => Some(Step::Permissions),
            Mode::Fix => None,
        };
        Effect::Relaunch { resume }
    }

    /// A Screen Recording grant this copy can't use relaunches Scrawl a moment after its check
    /// shows, whatever was picked in macOS's "Quit & Reopen" sheet. Otherwise a fix closes by
    /// itself once everything it asked for works; the tour waits for Continue.
    fn tick(&mut self, now_ms: u64) -> Effect {
        if self.step == Step::Permissions && self.screen_relaunch() {
            self.ready_at = None;
            if !self.auto_relaunch {
                return Effect::None;
            }
            let since = *self.stale_since.get_or_insert(now_ms);
            return if now_ms.saturating_sub(since) >= RELAUNCH_AFTER_MS {
                self.relaunch()
            } else {
                Effect::None
            };
        }
        self.stale_since = None;
        if matches!(self.step, Step::Permissions | Step::TryIt)
            && self.screen_confirmed.is_none()
            && self.asks.contains(&Permission::ScreenRecording)
            && self.usable(Permission::ScreenRecording)
        {
            self.screen_confirmed = Some(self.step);
            return Effect::ConfirmScreen;
        }
        if self.mode != Mode::Fix || !self.done() {
            self.ready_at = None;
            return Effect::None;
        }
        let since = *self.ready_at.get_or_insert(now_ms);
        if now_ms.saturating_sub(since) >= AUTO_ADVANCE_MS {
            self.advance()
        } else {
            Effect::None
        }
    }
}

/// Required permissions that keep ⌥⇧S from recording now. Screen Recording and Accessibility
/// are read in this process (a Screen Recording grant it can't use yet still blocks;
/// Accessibility is live); Microphone comes from the last fresh read.
pub fn missing_to_record(
    grants: Option<&Grants>,
    screen_usable: bool,
    accessibility: bool,
) -> Vec<Permission> {
    Permission::REQUIRED
        .into_iter()
        .filter(|p| match p {
            Permission::ScreenRecording => !screen_usable,
            Permission::Accessibility => !accessibility,
            p => grants.is_some_and(|g| !g.granted(*p)),
        })
        .collect()
}

/// Holds a request to show the permissions window until the shortcut's modifiers are up, so
/// nothing appears while ⌥⇧ is still down and every button is clickable when it does.
#[derive(Debug, Default)]
pub struct Gate {
    pending: Option<Vec<Permission>>,
}

impl Gate {
    /// ⌥⇧S went down with nothing recording. Returns whether it may go on; with `missing`
    /// permissions it starts nothing and the window waits for the keys to come up.
    pub fn press(&mut self, missing: Vec<Permission>) -> bool {
        if self.pending.is_some() {
            return false;
        }
        if missing.is_empty() {
            return true;
        }
        self.pending = Some(missing);
        false
    }

    /// Show the window for `asks` once the modifiers are up.
    pub fn defer(&mut self, asks: Vec<Permission>) {
        self.pending = Some(asks);
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Polled while pending: the permissions to show, once no modifier is held.
    pub fn tick(&mut self, modifiers_down: bool) -> Option<Vec<Permission>> {
        if modifiers_down {
            None
        } else {
            self.pending.take()
        }
    }
}

/// Onboarding progress, under `"onboarding"` in `~/.config/scrawl/config.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// The tour was finished or closed; it doesn't open by itself again.
    #[serde(default)]
    pub seen: bool,
    /// Where the tour picks up at the next launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<Step>,
    /// Every permission seen granted, so a later loss reopens the permissions screen.
    #[serde(default)]
    pub granted: BTreeSet<Permission>,
    /// When Scrawl last relaunched itself for Screen Recording (Unix ms), so it never does it
    /// twice in a row (`relaunch_allowed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relaunched_at: Option<u64>,
    /// A permissions fix was open when Scrawl relaunched itself; it opens again at launch.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reopen_fix: bool,
    /// The how-it-works demo was skipped, watched or drawn on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tutorial: Option<Tutorial>,
}

impl Progress {
    pub fn load() -> Self {
        Self::load_from(&crate::handoff::default_config_path())
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|config| config.get("onboarding").cloned())
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let _ = self.save_to(&crate::handoff::default_config_path());
    }

    /// Writes `"onboarding"`, keeping the rest of the config (and its `0600` mode) as it is.
    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        crate::handoff::set_config_value_in(path, "onboarding", serde_json::to_value(self)?)
    }

    /// Remembers what's granted now; returns whether anything new was.
    pub fn note(&mut self, grants: &Grants) -> bool {
        let before = self.granted.len();
        self.granted.extend(
            grants
                .statuses
                .iter()
                .filter(|(_, status)| **status == Status::Granted)
                .map(|(permission, _)| *permission),
        );
        self.granted.len() != before
    }

    /// Notes a relaunch Scrawl is about to do by itself, so the next launch comes back to the
    /// same window: the tour at `resume`, or the fix.
    pub fn relaunching(&mut self, resume: Option<Step>, now_ms: u64) {
        self.relaunched_at = Some(now_ms);
        match resume {
            Some(step) => {
                self.seen = false;
                self.resume = Some(step);
            }
            None => self.reopen_fix = true,
        }
    }
}

/// Whether Scrawl may relaunch itself now: not within `RELAUNCH_GUARD_MS` of the last time.
pub fn relaunch_allowed(progress: &Progress, now_ms: u64) -> bool {
    progress
        .relaunched_at
        .is_none_or(|at| now_ms.saturating_sub(at) >= RELAUNCH_GUARD_MS)
}

pub fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// What opens at launch: the tour until it's been seen (resuming where it was left), else a fix
/// that was open when Scrawl relaunched itself, else the permissions screen when a required
/// permission that was granted once is gone, else nothing.
pub fn launch_plan(progress: &Progress, grants: &Grants) -> Option<Flow> {
    if !progress.seen {
        let start = progress.resume.unwrap_or(Step::Welcome);
        return Some(Flow::tour(start, Some(grants.clone())));
    }
    let lost = Permission::REQUIRED
        .into_iter()
        .any(|p| progress.granted.contains(&p) && !grants.granted(p));
    (progress.reopen_fix || lost)
        .then(|| Flow::fix(Permission::REQUIRED.to_vec(), Some(grants.clone())))
}

/// From the welcome window's helper process and the permission poller.
#[derive(Clone, Debug)]
pub enum Event {
    /// Grants read at launch, to decide whether onboarding opens (`launch_plan`).
    Launch(Grants),
    Grants(Grants),
    /// A button: `continue`, `back`, `later`, `close`, `settings` or `grant:<permission>`.
    Action(String),
    Exited,
}

/// Reads what's granted now from a fresh helper, plus whether this process can use a Screen
/// Recording grant yet. Only once the helper says it's granted does this process try a real
/// ScreenCaptureKit capture (`capture::screen_capture_usable`), so no prompt can come of it.
pub fn read_grants() -> Option<Grants> {
    let statuses = crate::helper::permissions()
        .inspect_err(|error| crate::capture::scrawl_log(format!("permissions: {error:#}")))
        .ok()?;
    let screen = statuses.get(&Permission::ScreenRecording) == Some(&Status::Granted);
    let grants = Grants {
        statuses,
        screen_needs_relaunch: screen && !crate::capture::screen_capture_usable(true),
    };
    crate::speech_model::note_grants(&grants);
    Some(grants)
}

/// What draws the welcome window: the SwiftUI `scrawl-helper onboarding` process on macOS, an
/// in-process GPUI window (`crate::onboarding_window`) on Windows. Both take the same JSON
/// messages and report the same button actions.
enum Transport {
    Helper {
        child: std::process::Child,
        stdin: Option<std::process::ChildStdin>,
    },
    InProcess(Option<flume::Sender<serde_json::Value>>),
}

/// The open welcome window: the `Flow`, the window drawing it, and a poller re-reading the
/// grants every 0.5 s so each row's check appears by itself.
pub struct Session {
    flow: Flow,
    transport: Transport,
    polling: std::sync::Arc<std::sync::atomic::AtomicBool>,
    last_view: String,
    saved_resume: Option<Step>,
    saved_tutorial: Option<Tutorial>,
    clock: std::time::Instant,
}

impl Session {
    pub fn open(
        flow: Flow,
        shortcut: &str,
        events: impl Fn(Event) + Send + Sync + Clone + 'static,
    ) -> anyhow::Result<Self> {
        use std::sync::atomic::{AtomicBool, Ordering};

        let transport = if cfg!(target_os = "windows") {
            let (sender, receiver) = flume::unbounded();
            let window_events = events.clone();
            crate::onboarding_window::request_open(
                receiver,
                shortcut.to_owned(),
                std::sync::Arc::new(move |event| window_events(event)),
            );
            Transport::InProcess(Some(sender))
        } else {
            Self::spawn_helper(shortcut, events.clone())?
        };

        let polling = std::sync::Arc::new(AtomicBool::new(true));
        let alive = std::sync::Arc::clone(&polling);
        std::thread::spawn(move || {
            while alive.load(Ordering::Acquire) {
                if let Some(grants) = read_grants()
                    && alive.load(Ordering::Acquire)
                {
                    events(Event::Grants(grants));
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        });

        let progress = Progress::load();
        let allowed = relaunch_allowed(&progress, unix_ms());
        let mut session = Self {
            flow: flow
                .with_auto_relaunch(allowed)
                .with_tutorial(progress.tutorial),
            transport,
            polling,
            last_view: String::new(),
            saved_resume: None,
            saved_tutorial: progress.tutorial,
            clock: std::time::Instant::now(),
        };
        session.sync();
        Ok(session)
    }

    fn spawn_helper(
        shortcut: &str,
        events: impl Fn(Event) + Send + Sync + Clone + 'static,
    ) -> anyhow::Result<Transport> {
        use std::io::BufRead;
        use std::process::{Command, Stdio};

        let mut command = Command::new(crate::helper::path()?);
        command
            .args(["onboarding", "--shortcut", shortcut, "--paste"])
            .arg(crate::settings::paste_label())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let dev_icon = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/app-icon.png");
        if Path::new(dev_icon).is_file() {
            command.args(["--icon", dev_icon]);
        }
        let mut child = command.spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();

        std::thread::spawn(move || {
            if let Some(stdout) = stdout {
                for line in std::io::BufReader::new(stdout)
                    .lines()
                    .map_while(Result::ok)
                {
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };
                    let Some(action) = value.get("action").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let action = match value.get("permission").and_then(|v| v.as_str()) {
                        Some(permission) => format!("{action}:{permission}"),
                        None => action.to_owned(),
                    };
                    events(Event::Action(action));
                }
            }
            events(Event::Exited);
        });
        Ok(Transport::Helper { child, stdin })
    }

    pub fn mode(&self) -> Mode {
        self.flow.mode()
    }

    /// Feeds the flow and carries out what the window can do itself. Returns what needs the
    /// app: `Close` or `Relaunch`.
    pub fn input(&mut self, input: Input) -> Effect {
        let effect = self.flow.on(input);
        crate::capture::scrawl_log(format!("onboarding: {:?} -> {effect:?}", self.flow.step()));
        let result = match effect {
            // Attributed to whichever process asks, so Scrawl itself asks.
            Effect::Request(Permission::ScreenRecording) => {
                crate::capture::request_screen_capture();
                Effect::None
            }
            Effect::Request(permission) => {
                self.send(&serde_json::json!({"type": "request", "permission": permission}));
                Effect::None
            }
            Effect::OpenSettings(permission) => {
                self.send(&serde_json::json!({"type": "open-settings", "permission": permission}));
                Effect::None
            }
            Effect::Hide => {
                self.send(&serde_json::json!({"type": "hide"}));
                Effect::None
            }
            Effect::Show => {
                self.show();
                Effect::None
            }
            Effect::ConfirmScreen => {
                std::thread::spawn(|| {
                    // After macOS's own questions (System Settings' "Quit & Reopen") are
                    // answered and this window is back in front with its note, so the one
                    // question comes up by itself, over the explanation.
                    #[cfg(target_os = "macos")]
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
                    #[cfg(target_os = "macos")]
                    while std::time::Instant::now() < deadline
                        && crate::windows::macos_is_asking(&objc2::rc::autoreleasepool(|_| {
                            crate::windows::raw_windows()
                        }))
                    {
                        std::thread::sleep(std::time::Duration::from_millis(300));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1_200));
                    let result = crate::capture::screen_capture_probe();
                    crate::capture::scrawl_log(format!(
                        "onboarding: listed the screen once for macOS's window picker question: {result:?}"
                    ));
                });
                Effect::None
            }
            other => other,
        };
        self.sync();
        result
    }

    /// An action string from the helper as an `Input`.
    pub fn action(&mut self, action: &str) -> Effect {
        let input = match action {
            "continue" => Input::Continue,
            "back" => Input::Back,
            "later" => Input::Later,
            "close" => Input::Closed,
            "settings" => Input::Settings,
            "skip" => Input::Skip,
            "watched" => Input::Watched,
            "practiced" => Input::Practiced,
            other => match other
                .strip_prefix("grant:")
                .and_then(|p| serde_json::from_value(serde_json::Value::String(p.into())).ok())
            {
                Some(permission) => Input::Grant(permission),
                None => return Effect::None,
            },
        };
        self.input(input)
    }

    pub fn grants(&mut self, grants: Grants) -> Effect {
        let now_ms = self.clock.elapsed().as_millis() as u64;
        self.input(Input::Grants { grants, now_ms })
    }

    pub fn show(&mut self) {
        self.send(&serde_json::json!({"type": "show"}));
    }

    fn sync(&mut self) {
        let mut view = serde_json::to_value(self.flow.view()).unwrap_or_default();
        view["type"] = "state".into();
        view["speech"] = serde_json::to_value(crate::speech_model::current()).unwrap_or_default();
        let text = view.to_string();
        if text != self.last_view {
            self.send(&view);
            self.last_view = text;
        }
        // Remember the step, so whatever quits Scrawl, the tour comes back here.
        let resume = self.flow.resume_point();
        let tutorial = self.flow.tutorial();
        let new_resume = resume.is_some() && resume != self.saved_resume;
        if new_resume || tutorial != self.saved_tutorial {
            let mut progress = Progress::load();
            if new_resume {
                progress.resume = resume;
            }
            progress.tutorial = tutorial;
            progress.save();
            self.saved_resume = resume.or(self.saved_resume);
            self.saved_tutorial = tutorial;
        }
    }

    fn send(&mut self, message: &serde_json::Value) {
        use std::io::Write;
        match &mut self.transport {
            Transport::Helper { stdin, .. } => {
                if let Some(pipe) = stdin.as_mut()
                    && writeln!(pipe, "{message}")
                        .and_then(|()| pipe.flush())
                        .is_err()
                {
                    *stdin = None;
                }
            }
            Transport::InProcess(sender) => {
                if sender
                    .as_ref()
                    .is_some_and(|s| s.send(message.clone()).is_err())
                {
                    *sender = None;
                }
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.polling
            .store(false, std::sync::atomic::Ordering::Release);
        self.send(&serde_json::json!({"type": "close"}));
        match &mut self.transport {
            Transport::Helper { child, stdin } => {
                *stdin = None;
                let _ = child.wait();
            }
            Transport::InProcess(sender) => *sender = None,
        }
    }
}

/// Starts Scrawl again once this copy has quit: the new copy waits for this one
/// (`win::shell::wait_for_relauncher`) before it starts the app.
#[cfg(windows)]
pub fn relaunch() {
    use std::os::windows::process::CommandExt;
    /// No console window flashes up for the new copy (`scrawl.exe` is a console program).
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env(crate::win::shell::RELAUNCHED_BY, std::process::id().to_string())
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(not(windows))]
/// Waits (at most 10 s) for process `$1` to exit, then runs the rest.
const AFTER_EXIT: &str = "pid=$1; shift; i=0; \
    while kill -0 \"$pid\" 2>/dev/null && [ $i -lt 100 ]; do sleep 0.1; i=$((i+1)); done; ";

/// Starts Scrawl again once this copy has quit (after a Screen Recording grant). The app is
/// opened without `-n`: if macOS's own "Quit & Reopen" already started a new copy, `open` only
/// brings that one forward, so there are never two.
#[cfg(not(windows))]
pub fn relaunch() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let bundle = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|ext| ext == "app"));
    let pid = std::process::id().to_string();
    let mut command = std::process::Command::new("/bin/sh");
    match bundle {
        Some(app) => command
            .arg("-c")
            .arg(format!("{AFTER_EXIT}exec /usr/bin/open \"$1\""))
            .args(["scrawl-relaunch", &pid])
            .arg(app),
        None => command
            .arg("-c")
            .arg(format!("{AFTER_EXIT}exec \"$@\""))
            .args(["scrawl-relaunch", &pid])
            .arg(&exe)
            .args(std::env::args_os().skip(1)),
    };
    let _ = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    use Permission::{Accessibility, Camera, InputMonitoring, Microphone, ScreenRecording};

    const ALL: [Permission; 3] = [ScreenRecording, Microphone, Accessibility];

    fn grants(granted: &[Permission]) -> Grants {
        Grants {
            statuses: granted.iter().map(|p| (*p, Status::Granted)).collect(),
            screen_needs_relaunch: false,
        }
    }

    /// The helper says Screen Recording is granted, but this copy can't capture yet.
    fn stale(granted: &[Permission]) -> Grants {
        Grants {
            screen_needs_relaunch: true,
            ..grants(granted)
        }
    }

    fn tick(flow: &mut Flow, g: &Grants, now_ms: u64) -> Effect {
        flow.on(Input::Grants {
            grants: g.clone(),
            now_ms,
        })
    }

    fn rows(flow: &Flow) -> Vec<Permission> {
        flow.view().rows.iter().map(|r| r.permission).collect()
    }

    #[test]
    fn the_tour_is_four_screens_with_every_permission_on_one() {
        let mut flow = Flow::tour(Step::Welcome, Some(grants(&[])));
        assert_eq!(flow.view().count, 4);
        assert_eq!(flow.on(Input::Continue), Effect::None);
        assert_eq!(flow.step(), Step::HowItWorks);
        assert_eq!(flow.on(Input::Continue), Effect::None);
        assert_eq!(flow.step(), Step::Permissions);
        assert_eq!(rows(&flow), Permission::REQUIRED);
        assert!(flow.view().rows.iter().all(|r| r.required));
        // Camera is asked for when used, never in the tour.
        assert!(!rows(&flow).contains(&Camera));
        // Continue does nothing until the required ones are on.
        assert!(!flow.view().ready);
        assert_eq!(flow.on(Input::Continue), Effect::None);
        assert_eq!(flow.step(), Step::Permissions);
        assert_eq!(flow.on(Input::Back), Effect::None);
        assert_eq!(flow.step(), Step::HowItWorks);
    }

    #[test]
    fn speech_recognition_is_an_optional_row_only_where_the_helper_reports_it() {
        use Permission::SpeechRecognition;
        // macOS 26: the helper doesn't report it, so it isn't on the screen or in Settings.
        assert!(!SpeechRecognition.asked(Some(&grants(&[]))));
        assert!(!SpeechRecognition.asked(None));
        let flow = Flow::tour(Step::Permissions, Some(grants(&[])));
        assert!(!rows(&flow).contains(&SpeechRecognition));

        // macOS 15: a fourth row after the required ones, asked for with Apple's prompt.
        let mut asking = grants(&[]);
        asking
            .statuses
            .insert(SpeechRecognition, Status::NotDetermined);
        let mut flow = Flow::tour(Step::Permissions, Some(asking));
        assert_eq!(
            rows(&flow),
            [
                ScreenRecording,
                Microphone,
                Accessibility,
                SpeechRecognition
            ]
        );
        let row = flow.view().rows[3].clone();
        assert!(!row.required);
        assert_eq!(
            flow.on(Input::Grant(SpeechRecognition)),
            Effect::Request(SpeechRecognition)
        );
        assert_eq!(
            flow.on(Input::Grant(SpeechRecognition)),
            Effect::OpenSettings(SpeechRecognition)
        );

        // It never holds up Continue.
        let mut required = grants(&ALL);
        required.statuses.insert(SpeechRecognition, Status::Denied);
        tick(&mut flow, &required, 0);
        assert!(flow.view().ready);
        assert_eq!(flow.on(Input::Continue), Effect::None);
        assert_eq!(flow.step(), Step::TryIt);
    }

    #[test]
    fn the_demo_can_be_skipped_and_adds_no_screen() {
        let mut flow = Flow::tour(Step::HowItWorks, Some(grants(&[])));
        assert_eq!(flow.view().count, 4);
        assert_eq!(flow.view().tutorial, None);
        // Skip goes straight on, like Continue, and is remembered.
        assert_eq!(flow.on(Input::Skip), Effect::None);
        assert_eq!(flow.step(), Step::Permissions);
        assert_eq!(flow.tutorial(), Some(Tutorial::Skipped));
        // Skip, Watched and Practiced mean nothing on another screen.
        assert_eq!(flow.on(Input::Skip), Effect::None);
        assert_eq!(flow.on(Input::Practiced), Effect::None);
        assert_eq!(flow.step(), Step::Permissions);
        assert_eq!(flow.tutorial(), Some(Tutorial::Skipped));
        // Back to it and watched through: the button reads Continue from then on.
        flow.on(Input::Back);
        assert_eq!(flow.step(), Step::HowItWorks);
        flow.on(Input::Watched);
        assert_eq!(flow.view().tutorial, Some(Tutorial::Watched));
        assert_eq!(flow.step(), Step::HowItWorks, "watching doesn't move on");
        // A mark drawn on the demo wins over watching, and watching again doesn't undo it.
        flow.on(Input::Practiced);
        flow.on(Input::Watched);
        assert_eq!(flow.tutorial(), Some(Tutorial::Practiced));
        // Skip after watching moves on without forgetting the mark.
        flow.on(Input::Skip);
        assert_eq!(flow.step(), Step::Permissions);
        assert_eq!(flow.tutorial(), Some(Tutorial::Practiced));
        // With every permission on, Skip passes the permissions screen too.
        let mut flow = Flow::tour(Step::HowItWorks, Some(grants(&ALL)));
        flow.on(Input::Skip);
        assert_eq!(flow.step(), Step::TryIt);
        // A fix never shows the demo.
        let mut fix = Flow::fix(Permission::REQUIRED.to_vec(), Some(grants(&[])));
        assert_eq!(fix.on(Input::Skip), Effect::None);
        assert_eq!(fix.tutorial(), None);
    }

    #[test]
    fn the_demo_outcome_comes_back_after_a_quit() {
        let progress = Progress {
            resume: Some(Step::HowItWorks),
            tutorial: Some(Tutorial::Watched),
            ..Progress::default()
        };
        let flow = launch_plan(&progress, &grants(&[]))
            .expect("the tour")
            .with_tutorial(progress.tutorial);
        assert_eq!(flow.step(), Step::HowItWorks);
        assert_eq!(flow.view().tutorial, Some(Tutorial::Watched));
        // The view carries it to the helper as kebab-case, or null before anything happened.
        let json = serde_json::to_value(flow.view()).unwrap();
        assert_eq!(json["tutorial"], "watched");
        let fresh = serde_json::to_value(Flow::tour(Step::Welcome, None).view()).unwrap();
        assert!(fresh["tutorial"].is_null());
        // An older config without the key loads as nothing yet.
        let old: Progress = serde_json::from_str(r#"{"seen":false,"resume":"how-it-works"}"#).unwrap();
        assert_eq!(old.tutorial, None);
    }

    #[test]
    fn screen_recording_is_the_first_row_and_accessibility_the_third() {
        assert_eq!(Permission::REQUIRED, ALL);
        let flow = Flow::tour(Step::Permissions, Some(grants(&[])));
        assert_eq!(rows(&flow), ALL);
        let fix = Flow::fix(Permission::REQUIRED.to_vec(), Some(grants(&[])));
        assert_eq!(rows(&fix)[0], ScreenRecording);
        assert_eq!(rows(&fix)[2], Accessibility);
        assert!(Accessibility.required() && Accessibility.draggable());
    }

    #[test]
    fn input_monitoring_is_never_requested() {
        assert!(!InputMonitoring.required());
        assert!(!InputMonitoring.draggable());
        let mut flow = Flow::tour(Step::Permissions, Some(grants(&[])));
        assert!(!rows(&flow).contains(&InputMonitoring));
        assert_eq!(flow.on(Input::Grant(InputMonitoring)), Effect::None);
        assert!(missing_to_record(Some(&grants(&ALL)), true, true).is_empty());
        // Nothing in Scrawl or its helper asks macOS for it.
        let sources = [
            include_str!("capture.rs"),
            include_str!("capture/macos.rs"),
            include_str!("ui/macos.rs"),
            include_str!("onboarding.rs"),
            include_str!("settings.rs"),
            include_str!("ui.rs"),
            include_str!("../helper/Onboarding.swift"),
            include_str!("../helper/Settings.swift"),
            include_str!("../helper/PermissionPanel.swift"),
        ];
        let calls = [
            concat!("CGRequest", "ListenEventAccess"),
            concat!("CGPreflight", "ListenEventAccess"),
            concat!("IOHIDRequest", "Access"),
            concat!("IOHIDCheck", "Access"),
            concat!("kCGEventTapOption", "ListenOnly"),
        ];
        for source in sources {
            for call in calls {
                assert!(!source.contains(call), "{call} is still called");
            }
        }
        // A 0.3.x config that remembers it granted still loads.
        let old: Progress = serde_json::from_str(
            r#"{"seen":true,"granted":["screen-recording","microphone","input-monitoring"]}"#,
        )
        .unwrap();
        assert!(old.seen);
    }

    #[test]
    fn permissions_are_granted_in_any_order_then_continue_moves_on() {
        let mut flow = Flow::tour(Step::Permissions, Some(grants(&[])));
        // A list the app can be dragged into goes straight to System Settings and the panel.
        assert_eq!(
            flow.on(Input::Grant(Accessibility)),
            Effect::OpenSettings(Accessibility)
        );
        assert_eq!(
            flow.on(Input::Grant(Microphone)),
            Effect::Request(Microphone)
        );
        // A second press opens System Settings.
        assert_eq!(
            flow.on(Input::Grant(Microphone)),
            Effect::OpenSettings(Microphone)
        );
        let some = grants(&[Microphone, Accessibility]);
        tick(&mut flow, &some, 0);
        let view = flow.view();
        assert_eq!(view.rows[1].status, Status::Granted);
        assert!(!view.rows[0].requested && !view.ready);
        // No auto-advance in the tour, even long after.
        let all = grants(&ALL);
        tick(&mut flow, &all, 1);
        tick(&mut flow, &all, 10_000);
        assert_eq!(flow.step(), Step::Permissions);
        assert!(flow.view().ready);
        // A granted row's button does nothing.
        assert_eq!(flow.on(Input::Grant(Microphone)), Effect::None);
        assert_eq!(flow.on(Input::Continue), Effect::None);
        assert_eq!(flow.step(), Step::TryIt);
        // Back from Try it passes the finished permissions screen.
        flow.on(Input::Back);
        assert_eq!(flow.step(), Step::HowItWorks);
    }

    #[test]
    fn a_denied_permission_goes_straight_to_settings() {
        let mut denied = grants(&[]);
        denied.statuses.insert(Microphone, Status::Denied);
        let mut flow = Flow::tour(Step::Permissions, Some(denied));
        assert_eq!(
            flow.on(Input::Grant(Microphone)),
            Effect::OpenSettings(Microphone)
        );
    }

    #[test]
    fn granted_permissions_pass_the_screen_and_leave_the_dots() {
        let all = grants(&ALL);
        let mut flow = Flow::tour(Step::HowItWorks, Some(all.clone()));
        assert_eq!(flow.view().count, 3);
        flow.on(Input::Continue);
        assert_eq!(flow.step(), Step::TryIt);
        assert_eq!(flow.view().index, 2);
        // A tour resumed past the screen goes back to it while something is missing.
        let some = grants(&[Microphone]);
        assert_eq!(
            Flow::tour(Step::TryIt, Some(some)).step(),
            Step::Permissions
        );
        // Resumed on the screen itself, it stays there with every check showing.
        let resumed = Flow::tour(Step::Permissions, Some(all));
        assert_eq!(resumed.step(), Step::Permissions);
        assert!(resumed.view().ready);
    }

    #[test]
    fn a_screen_recording_grant_this_copy_cant_use_relaunches_by_itself_once() {
        let mut flow = Flow::tour(Step::Permissions, Some(grants(&[])));
        // The user drags Scrawl into Screen Recording; macOS offers Quit & Reopen and the user
        // picks Later (or nothing): the helper says granted, this process still can't capture.
        let granted = stale(&[ScreenRecording]);
        assert_eq!(tick(&mut flow, &granted, 1_000), Effect::None);
        let row = flow.view().rows[0].clone();
        assert_eq!(row.status, Status::Granted);
        assert!(row.reopening && !row.relaunch);
        // Not ready: Continue can't skip past a grant that doesn't work yet.
        assert_eq!(flow.on(Input::Continue), Effect::None);
        // The check shows a moment, then Scrawl relaunches, back on this screen, even though
        // the other two rows are still off.
        assert_eq!(tick(&mut flow, &granted, 1_500), Effect::None);
        let relaunch = Effect::Relaunch {
            resume: Some(Step::Permissions),
        };
        assert_eq!(tick(&mut flow, &granted, 1_000 + RELAUNCH_AFTER_MS), relaunch);
        // Never twice from one window.
        assert_eq!(tick(&mut flow, &granted, 9_000), Effect::None);
        assert_eq!(flow.on(Input::Grant(ScreenRecording)), Effect::None);
        // A grant this copy can use (ScreenCaptureKit works without a relaunch, or macOS's
        // Quit & Reopen already restarted it) relaunches nothing.
        let mut flow = Flow::tour(Step::Permissions, Some(grants(&[])));
        assert_eq!(
            tick(&mut flow, &grants(&[ScreenRecording]), 0),
            Effect::ConfirmScreen
        );
        for t in [5_000, 60_000] {
            assert_eq!(tick(&mut flow, &grants(&[ScreenRecording]), t), Effect::None);
        }
        assert!(!flow.view().rows[0].reopening);
    }

    #[test]
    fn once_screen_recording_works_the_window_picker_question_comes_up_explained() {
        let mut flow = Flow::tour(Step::Permissions, Some(grants(&[])));
        assert_eq!(tick(&mut flow, &grants(&[Microphone]), 0), Effect::None);
        assert!(!flow.view().picker_note);
        // Not while the grant needs a relaunch to work: listing the screen would fail.
        assert_eq!(
            tick(&mut flow, &stale(&[ScreenRecording]), 100),
            Effect::None
        );
        assert_eq!(
            tick(&mut flow, &grants(&[ScreenRecording]), 200),
            Effect::ConfirmScreen
        );
        assert!(flow.view().picker_note);
        assert_eq!(
            tick(&mut flow, &grants(&[ScreenRecording]), 300),
            Effect::None
        );
        // Answered by the time Try it shows: no note there.
        flow.on(Input::Grants {
            grants: grants(&Permission::REQUIRED),
            now_ms: 400,
        });
        flow.on(Input::Continue);
        assert_eq!(flow.step(), Step::TryIt);
        assert!(!flow.view().picker_note);
        // Everything already on: it comes up on Try it instead, before the first capture.
        let mut flow = Flow::tour(Step::TryIt, Some(grants(&Permission::REQUIRED)));
        assert_eq!(flow.step(), Step::TryIt);
        assert_eq!(
            tick(&mut flow, &grants(&Permission::REQUIRED), 0),
            Effect::ConfirmScreen
        );
        assert!(flow.view().picker_note);
        // Gone from the last screen.
        flow.on(Input::CaptureEnded { completed: true });
        assert_eq!(flow.step(), Step::Done);
        assert!(!flow.view().picker_note);
    }

    #[test]
    fn a_fix_relaunches_too_and_reopens_after() {
        let mut flow = Flow::fix(Permission::REQUIRED.to_vec(), Some(grants(&[Microphone])));
        let granted = stale(&[ScreenRecording, Microphone]);
        tick(&mut flow, &granted, 0);
        assert_eq!(
            tick(&mut flow, &granted, RELAUNCH_AFTER_MS),
            Effect::Relaunch { resume: None }
        );
        let mut progress = Progress {
            seen: true,
            ..Progress::default()
        };
        progress.relaunching(None, 42);
        assert!(progress.reopen_fix && progress.seen);
        let flow = launch_plan(&progress, &grants(&[ScreenRecording, Microphone])).expect("fix");
        assert_eq!((flow.mode(), flow.step()), (Mode::Fix, Step::Permissions));
        assert_eq!(rows(&flow), Permission::REQUIRED);
    }

    #[test]
    fn right_after_a_relaunch_it_never_relaunches_by_itself_again() {
        let now = 1_000_000;
        let mut progress = Progress::default();
        assert!(relaunch_allowed(&progress, now));
        progress.relaunching(Some(Step::Permissions), now);
        assert!(!relaunch_allowed(&progress, now + 5_000));
        assert!(!relaunch_allowed(&progress, now + RELAUNCH_GUARD_MS - 1));
        assert!(relaunch_allowed(&progress, now + RELAUNCH_GUARD_MS));
        // The fresh copy still can't capture (something else is wrong): no loop. The row offers
        // Reopen, and only pressing it relaunches.
        let mut flow = Flow::tour(Step::Permissions, Some(grants(&[]))).with_auto_relaunch(false);
        let granted = stale(&[ScreenRecording]);
        for t in [0, RELAUNCH_AFTER_MS, 60_000] {
            assert_eq!(tick(&mut flow, &granted, t), Effect::None);
        }
        let row = flow.view().rows[0].clone();
        assert!(row.relaunch && !row.reopening);
        assert_eq!(
            flow.on(Input::Grant(ScreenRecording)),
            Effect::Relaunch {
                resume: Some(Step::Permissions)
            }
        );
        assert_eq!(flow.on(Input::Grant(ScreenRecording)), Effect::None);
    }

    #[test]
    fn after_either_kind_of_relaunch_the_tour_resumes_on_the_permissions_screen() {
        // Scrawl's own relaunch (the user picked Later, or nothing).
        let mut progress = Progress::default();
        progress.relaunching(Some(Step::Permissions), 7);
        assert_eq!(progress.resume, Some(Step::Permissions));
        assert!(!progress.seen);
        let fresh = grants(&[ScreenRecording]);
        let flow = launch_plan(&progress, &fresh).expect("the tour");
        assert_eq!((flow.mode(), flow.step()), (Mode::Tour, Step::Permissions));
        let view = flow.view();
        assert_eq!(view.rows[0].status, Status::Granted);
        assert!(!view.rows[0].reopening && !view.rows[0].relaunch);
        // The next row to do is Microphone.
        assert_eq!(
            view.rows.iter().find(|r| r.status != Status::Granted).map(|r| r.permission),
            Some(Microphone)
        );
        // macOS's Quit & Reopen: Scrawl never noted a relaunch, but the step it saved while
        // the window was open brings it back to the same place.
        let progress = Progress {
            resume: Flow::tour(Step::Permissions, Some(grants(&[]))).resume_point(),
            ..Progress::default()
        };
        assert_eq!(progress.relaunched_at, None);
        let flow = launch_plan(&progress, &fresh).expect("the tour");
        assert_eq!((flow.mode(), flow.step()), (Mode::Tour, Step::Permissions));
        // Screen Recording was the last one: the screen shows every check and Continue goes on.
        let mut flow = launch_plan(&progress, &grants(&ALL)).expect("the tour");
        assert_eq!(flow.step(), Step::Permissions);
        assert!(flow.view().ready);
        flow.on(Input::Continue);
        assert_eq!(flow.step(), Step::TryIt);
    }

    #[test]
    fn a_tour_quit_midway_resumes_where_it_was() {
        let mut flow = Flow::tour(Step::Welcome, Some(grants(&[])));
        assert_eq!(flow.resume_point(), Some(Step::Welcome));
        flow.on(Input::Continue);
        flow.on(Input::Continue);
        assert_eq!(flow.resume_point(), Some(Step::Permissions));
        let progress = Progress {
            resume: flow.resume_point(),
            ..Progress::default()
        };
        let flow = launch_plan(&progress, &grants(&[ScreenRecording])).expect("the tour");
        assert_eq!(flow.step(), Step::Permissions);
        assert_eq!(
            Flow::fix(vec![Camera], None).resume_point(),
            None,
            "a fix has nothing to resume"
        );
    }

    #[test]
    fn set_up_later_and_closing_end_the_tour() {
        let mut flow = Flow::tour(Step::Permissions, Some(grants(&[])));
        assert_eq!(flow.on(Input::Later), Effect::Close);
        assert_eq!(flow.on(Input::Closed), Effect::Close);
    }

    #[test]
    fn the_shortcut_with_a_permission_off_starts_nothing_and_opens_the_window_on_release() {
        // Mic and Accessibility on, Screen Recording not usable in this process.
        let some = grants(&[Microphone, Accessibility]);
        let missing = missing_to_record(Some(&some), false, true);
        assert_eq!(missing, [ScreenRecording]);
        // Accessibility is read live in this process, not from the last helper read.
        assert_eq!(
            missing_to_record(Some(&grants(&ALL)), true, false),
            [Accessibility]
        );
        let mut gate = Gate::default();
        assert!(!gate.press(missing.clone()), "nothing starts");
        // Pressing again while it waits starts nothing either.
        assert!(!gate.press(Vec::new()));
        // Nothing shows while ⌥⇧ is held.
        assert_eq!(gate.tick(true), None);
        let asks = gate.tick(false).expect("the window, once the keys are up");
        assert_eq!(gate.tick(false), None);
        assert!(!gate.is_pending());
        // The window lists every required permission so the user sees what's left.
        let mut flow = Flow::fix(Permission::REQUIRED.to_vec(), Some(some.clone()));
        assert_eq!(asks, missing);
        assert_eq!(flow.step(), Step::Permissions);
        assert_eq!(
            flow.on(Input::Grant(ScreenRecording)),
            Effect::OpenSettings(ScreenRecording)
        );
        // With everything on, the shortcut goes through.
        assert!(missing_to_record(Some(&grants(&ALL)), true, true).is_empty());
        assert!(gate.press(Vec::new()));
        // A tour that's still explaining jumps to the permissions screen.
        let mut tour = Flow::tour(Step::Welcome, Some(some));
        assert_eq!(tour.on(Input::Blocked), Effect::Show);
        assert_eq!(tour.step(), Step::Permissions);
        // Unknown grants (the launch read failed): only what's read here blocks.
        assert!(missing_to_record(None, true, true).is_empty());
    }

    #[test]
    fn camera_is_asked_for_on_its_own_when_first_used() {
        let mut flow = Flow::fix(vec![Camera], Some(grants(&[ScreenRecording])));
        assert_eq!(rows(&flow), [Camera]);
        let view = flow.view();
        assert!(view.fix && !view.ready && !view.rows[0].required);
        assert_eq!(flow.on(Input::Grant(Camera)), Effect::Request(Camera));
        // A permission that isn't on this screen can't be asked for from it.
        assert_eq!(flow.on(Input::Grant(Microphone)), Effect::None);
        let camera = grants(&[Camera]);
        assert_eq!(tick(&mut flow, &camera, 100), Effect::None);
        assert!(flow.view().ready);
        assert_eq!(
            tick(&mut flow, &camera, 100 + AUTO_ADVANCE_MS),
            Effect::Close
        );
        // ⌥⇧V after Accessibility was turned off asks for just that row.
        let mut flow = Flow::fix(vec![Accessibility], Some(grants(&[])));
        assert_eq!(flow.on(Input::Later), Effect::Close);
        // A fix never steps back into the tour.
        assert_eq!(flow.on(Input::Back), Effect::None);
        assert_eq!(flow.step(), Step::Permissions);
    }

    #[test]
    fn a_capture_hides_the_window_and_finishing_from_try_it_shows_done() {
        let mut flow = Flow::tour(Step::TryIt, None);
        assert_eq!(flow.on(Input::CaptureStarted), Effect::Hide);
        assert_eq!(flow.on(Input::CaptureStarted), Effect::None);
        assert_eq!(
            flow.on(Input::CaptureEnded { completed: true }),
            Effect::Show
        );
        assert_eq!(flow.step(), Step::Done);
        assert_eq!(flow.on(Input::Settings), Effect::Settings);
        assert_eq!(flow.on(Input::Continue), Effect::Close);
        // A cancelled capture comes back to the same step.
        let mut flow = Flow::tour(Step::TryIt, None);
        flow.on(Input::CaptureStarted);
        assert_eq!(
            flow.on(Input::CaptureEnded { completed: false }),
            Effect::Show
        );
        assert_eq!(flow.step(), Step::TryIt);
    }

    #[test]
    fn launch_shows_the_tour_once_then_only_a_lost_permission() {
        let all = grants(&ALL);
        let mut progress = Progress::default();
        assert_eq!(
            launch_plan(&progress, &all).map(|f| (f.mode(), f.step())),
            Some((Mode::Tour, Step::Welcome))
        );
        progress.seen = true;
        assert!(progress.note(&all));
        assert!(!progress.note(&all));
        assert!(launch_plan(&progress, &all).is_none());
        // Accessibility went away: the permissions screen, as a fix.
        let lost = grants(&[ScreenRecording, Microphone]);
        let flow = launch_plan(&progress, &lost).expect("a fix");
        assert_eq!((flow.mode(), flow.step()), (Mode::Fix, Step::Permissions));
        assert_eq!(rows(&flow), Permission::REQUIRED);
        // Never granted (put off in the tour): no nagging at launch.
        progress.granted.remove(&Accessibility);
        assert!(launch_plan(&progress, &lost).is_none());
    }

    #[test]
    fn progress_is_kept_beside_the_rest_of_the_config() {
        let dir = std::env::temp_dir().join(format!("scrawl-onboarding-{}", std::process::id()));
        let path = dir.join("config.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{"agentHost":"studio","autocrop":false}"#).unwrap();
        assert_eq!(Progress::load_from(&path), Progress::default());
        let progress = Progress {
            seen: true,
            resume: Some(Step::Permissions),
            granted: [Microphone].into_iter().collect(),
            relaunched_at: Some(1_792_000_000_000),
            reopen_fix: true,
            tutorial: Some(Tutorial::Practiced),
        };
        progress.save_to(&path).unwrap();
        assert_eq!(Progress::load_from(&path), progress);
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(config["agentHost"], "studio");
        assert_eq!(config["autocrop"], false);
        assert_eq!(config["onboarding"]["resume"], "permissions");
        assert_eq!(config["onboarding"]["relaunchedAt"], 1_792_000_000_000_u64);
        assert_eq!(config["onboarding"]["reopenFix"], true);
        assert_eq!(config["onboarding"]["tutorial"], "practiced");
        // A resume point saved by 0.3.2's one-permission-per-step tour still loads.
        std::fs::write(
            &path,
            r#"{"onboarding":{"seen":false,"resume":"microphone","granted":[]}}"#,
        )
        .unwrap();
        assert_eq!(Progress::load_from(&path).resume, Some(Step::Permissions));
        std::fs::write(&path, r#"{"onboarding":{"resume":"draw"}}"#).unwrap();
        assert_eq!(Progress::load_from(&path).resume, Some(Step::HowItWorks));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn helper_statuses_parse_into_grants() {
        let statuses: BTreeMap<Permission, Status> = serde_json::from_str(
            r#"{"screen-recording":"granted","microphone":"denied","camera":"not-determined"}"#,
        )
        .unwrap();
        let g = Grants {
            statuses,
            screen_needs_relaunch: false,
        };
        assert!(g.granted(ScreenRecording));
        assert_eq!(g.status(Microphone), Status::Denied);
        assert_eq!(g.status(Accessibility), Status::NotDetermined);
    }
}
