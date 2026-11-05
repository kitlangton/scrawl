//! First-run onboarding: a welcome, one screen on how Apparent works, one screen with every
//! permission it needs, and a first capture. The window is SwiftUI in `apparent-helper onboarding`;
//! this module decides what it shows (`Flow`) and remembers progress in
//! `~/.config/apparent/config.json` (`"onboarding"`, see `Progress`).
//!
//! Screen Recording, Microphone and Accessibility are required and asked for together on the
//! permissions screen, Screen Recording first. Camera (the cabled iPhone) is asked for the first
//! time it's used, in the same window with just that row (`Flow::fix`). On macOS 15, where
//! transcripts come from SFSpeechRecognizer, Speech Recognition is an optional fourth row, so
//! its prompt comes up here rather than after the first capture. Until the required ones
//! are on, ⌥⇧S starts nothing: it brings this window forward once the keys are up (`Gate`).
//!
//! A Screen Recording grant may only reach a running copy of Apparent after it relaunches, and
//! macOS offers "Quit & Reopen" the moment it's granted. Whatever the user picks there, nobody
//! reopens Apparent by hand: when a fresh helper says it's granted but this process still can't
//! capture, the window relaunches Apparent by itself (once, see `Progress::relaunched_at`) and the
//! tour comes back on the same screen. Input Monitoring is never asked for: clicks come from an
//! AppKit mouse monitor that needs no permission, keys from an Accessibility event tap.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

#[cfg(target_os = "macos")]
pub(crate) mod permission_helper;

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
    /// because it's the one that may relaunch Apparent. Accessibility drives the key event tap
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
        Self::REQUIRED.contains(&self) && (self != Self::Microphone || crate::capture::engine::runtime::audio::mic::enabled())
    }

    /// Whether this Mac asks for it at all: Speech Recognition only when the helper reports it
    /// (macOS before 26).
    pub fn asked(self, grants: Option<&Grants>) -> bool {
        self != Permission::SpeechRecognition
            || grants.is_some_and(|g| g.statuses.contains_key(&self))
    }

    /// Privacy lists the app can be dragged into: Grant flies the drag panel to System
    /// Settings (`apparent-helper permission-panel`) instead of raising Apple's prompt.
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
    /// An optional chance to change Record before the first capture.
    Shortcut,
    TryIt,
    /// After the first capture: it's on your clipboard.
    Done,
}

impl Step {
    const TOUR: [Step; 6] = [
        Step::Welcome,
        Step::HowItWorks,
        Step::Permissions,
        Step::Shortcut,
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

/// Permission statuses shown to the user. macOS Accessibility is checked in the recording
/// process; other statuses come from a fresh `apparent-helper permissions`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grants {
    pub statuses: BTreeMap<Permission, Status>,
    /// Screen recording is granted, but this running copy of Apparent can't use it until it
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

/// Screen Recording's check shows this long before Apparent relaunches to use it.
pub const RELAUNCH_AFTER_MS: u64 = 1_000;

/// After relaunching itself, Apparent doesn't do it again by itself for this long; the row offers
/// Reopen instead. Stops a loop if a fresh copy still can't capture.
pub const RELAUNCH_GUARD_MS: u64 = 120_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The whole tour, first launch or "Welcome to Apparent…".
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
    ChangeShortcut,
    /// A shortcut was explicitly chosen and successfully saved, or accepted from the desktop.
    ShortcutChosen,
    /// The global shortcut requested a capture (not the menu or a timed CLI recording).
    ShortcutTriggered,
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
    /// Relaunch Apparent so Screen Recording works. The tour comes back at `resume`; a fix (no
    /// `resume`) opens again.
    Relaunch {
        resume: Option<Step>,
    },
    Hide,
    Show,
    Close,
    /// Close the tour and open the Settings window.
    Settings,
    /// Open the existing recorder without dismissing the tour.
    EditShortcut,
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
    /// Granted, and Apparent is about to relaunch by itself to use it.
    pub reopening: bool,
    /// Granted, but this copy can't use it and won't relaunch by itself again: the row's
    /// button reopens Apparent.
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
    /// Show the inline Record recorder, not a second Settings window.
    pub shortcut_editing: bool,
    pub first_capture_required: bool,
    pub shortcut_chosen: bool,
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
    shortcut_editing: bool,
    first_capture_required: bool,
    shortcut_chosen: bool,
    first_capture_completed: bool,
    shortcut_pending: bool,
    practice_recording: bool,
}

impl Flow {
    /// Opt into the first-capture onboarding experiment, restoring durable milestones.
    pub fn with_first_capture(mut self, required: bool, chosen: bool, completed: bool) -> Self {
        self.first_capture_required = required && self.mode == Mode::Tour;
        self.shortcut_chosen = chosen;
        self.first_capture_completed = chosen && completed;
        if self.first_capture_required && self.step.position() >= Step::Shortcut.position() {
            self.step = if !chosen {
                Step::Shortcut
            } else if self.first_capture_completed {
                Step::Done
            } else if self.step == Step::Done {
                Step::TryIt
            } else {
                self.step
            };
        }
        self
    }
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
            shortcut_editing: false,
            first_capture_required: false,
            shortcut_chosen: false,
            first_capture_completed: false,
            shortcut_pending: false,
            practice_recording: false,
        }
    }

    /// Whether this window may relaunch Apparent by itself (`relaunch_allowed`).
    pub fn with_auto_relaunch(mut self, allowed: bool) -> Self {
        self.auto_relaunch = allowed;
        self
    }

    /// The demo's outcome from an earlier launch (`Progress::tutorial`).
    pub fn with_tutorial(mut self, tutorial: Option<Tutorial>) -> Self {
        self.tutorial = tutorial;
        self
    }

    pub(crate) fn local_practice(&self) -> bool {
        self.practice_recording || self.shortcut_pending
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

    /// Where a tour picks up if Apparent quits now (macOS's own "Quit & Reopen" included).
    pub fn resume_point(&self) -> Option<Step> {
        (self.mode == Mode::Tour && self.step != Step::Done).then_some(self.step)
    }

    pub fn on(&mut self, input: Input) -> Effect {
        match input {
            Input::ShortcutChosen => {
                if self.mode == Mode::Tour && self.step == Step::Shortcut {
                    self.shortcut_chosen = true;
                }
                Effect::None
            }
            Input::ShortcutTriggered => {
                self.shortcut_pending = self.step == Step::TryIt
                    && (!self.first_capture_required || self.shortcut_chosen);
                Effect::None
            }
            Input::Continue => self.primary(),
            Input::Back => {
                self.step = self.back_from(self.step);
                self.shortcut_editing = false;
                Effect::None
            }
            Input::Later | Input::Closed => Effect::Close,
            Input::Settings if self.step == Step::Done => Effect::Settings,
            Input::Settings => Effect::None,
            Input::ChangeShortcut if self.mode == Mode::Tour && self.step == Step::Shortcut => {
                self.shortcut_editing = true;
                Effect::EditShortcut
            }
            Input::ChangeShortcut => Effect::None,
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
                self.practice_recording = std::mem::take(&mut self.shortcut_pending);
                self.hidden = true;
                Effect::Hide
            }
            Input::CaptureStarted => Effect::None,
            Input::CaptureEnded { completed } => {
                if completed && self.step == Step::TryIt
                    && (!self.first_capture_required || self.practice_recording)
                {
                    self.first_capture_completed = true;
                    self.step = Step::Done;
                }
                self.shortcut_pending = false;
                self.practice_recording = false;
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
            shortcut_editing: self.step == Step::Shortcut && self.shortcut_editing,
            first_capture_required: self.first_capture_required,
            shortcut_chosen: self.shortcut_chosen,
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

    /// Screen Recording is on the screen and granted, but this copy of Apparent can't capture yet.
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
        self.shortcut_editing = false;
        if self.mode == Mode::Fix {
            return Effect::Close;
        }
        self.step = self.forward_from(self.step.next());
        Effect::None
    }

    fn primary(&mut self) -> Effect {
        match self.step {
            Step::Shortcut if self.first_capture_required && !self.shortcut_chosen => Effect::None,
            Step::TryIt if self.first_capture_required => Effect::None,
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

    /// A Screen Recording grant this copy can't use relaunches Apparent a moment after its check
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
/// Accessibility is live); Microphone comes from the last fresh read. On macOS a fresh
/// Screen Recording denial also blocks: this process can retain a successful preflight/probe
/// after revocation until it quits.
pub fn missing_to_record(
    grants: Option<&Grants>,
    screen_usable: bool,
    accessibility: bool,
) -> Vec<Permission> {
    Permission::REQUIRED
        .into_iter()
        .filter(|p| match p {
            Permission::ScreenRecording => {
                !screen_usable
                    || (cfg!(target_os = "macos")
                        && grants.is_some_and(|g| !g.usable(Permission::ScreenRecording)))
            }
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

/// Onboarding progress, under `"onboarding"` in `~/.config/apparent/config.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// The tour finished (or was dismissed in the legacy flow); it won't open by itself again.
    #[serde(default)]
    pub seen: bool,
    /// Where the tour picks up at the next launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<Step>,
    /// Every permission seen granted, so a later loss reopens the permissions screen.
    #[serde(default)]
    pub granted: BTreeSet<Permission>,
    /// When Apparent last relaunched itself for Screen Recording (Unix ms), so it never does it
    /// twice in a row (`relaunch_allowed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relaunched_at: Option<u64>,
    /// A permissions fix was open when Apparent relaunched itself; it opens again at launch.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reopen_fix: bool,
    /// The how-it-works demo was skipped, watched or drawn on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tutorial: Option<Tutorial>,
    /// Explicitly saved Record shortcut (or accepted desktop-owned shortcut) in the tour.
    #[serde(default)]
    pub shortcut_chosen: bool,
    /// A shortcut-started practice recording produced a completed local capture.
    #[serde(default)]
    pub first_capture_completed: bool,
}

impl Progress {
    /// Closing is allowed, but an experimental tour resumes until its capture succeeds.
    pub fn leave(&mut self, flow: &Flow) {
        if flow.mode == Mode::Fix {
            return;
        }
        self.seen = !flow.first_capture_required || flow.first_capture_completed;
        self.resume = if self.seen { None } else { flow.resume_point() };
    }
    pub fn load() -> Self {
        Self::load_from(&crate::shared::handoff::default_config_path())
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
        let _ = self.save_to(&crate::shared::handoff::default_config_path());
    }

    /// Writes `"onboarding"`, keeping the rest of the config (and its `0600` mode) as it is.
    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        crate::shared::handoff::set_config_value_in(path, "onboarding", serde_json::to_value(self)?)
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

    /// Notes a relaunch Apparent is about to do by itself, so the next launch comes back to the
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

/// Whether Apparent may relaunch itself now: not within `RELAUNCH_GUARD_MS` of the last time.
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
/// that was open when Apparent relaunched itself, else the permissions screen when a required
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
    /// The inline recorder uses the same actions and validation as Settings.
    Shortcut(crate::capture::desktop::settings::Action),
    Exited,
}

/// Reads what's granted now from a fresh helper, except Accessibility: the separately
/// signed helper may not share the recording process's trust. Only once the helper says
/// Screen Recording is granted does this process try a real ScreenCaptureKit capture.
pub fn read_grants() -> Option<Grants> {
    let statuses = crate::capture::desktop::helper::permissions()
        .inspect_err(|error| crate::capture::engine::runtime::apparent_log(format!("permissions: {error:#}")))
        .ok()?;
    let statuses = if cfg!(target_os = "macos") {
        reconcile_accessibility(statuses, crate::capture::engine::runtime::accessibility_granted())
    } else {
        statuses
    };
    let screen = statuses.get(&Permission::ScreenRecording) == Some(&Status::Granted);
    let grants = Grants {
        statuses,
        screen_needs_relaunch: screen && !crate::capture::engine::runtime::screen_capture_usable(true),
    };
    crate::capture::engine::runtime::audio::transcription::speech_model::note_grants(&grants);
    Some(grants)
}

fn reconcile_accessibility(
    mut statuses: BTreeMap<Permission, Status>,
    accessibility: bool,
) -> BTreeMap<Permission, Status> {
    statuses.insert(
        Permission::Accessibility,
        if accessibility {
            Status::Granted
        } else {
            Status::NotDetermined
        },
    );
    statuses
}

/// What draws the welcome window: the SwiftUI `apparent-helper onboarding` process on macOS, an
/// in-process GPUI window (`crate::onboarding_window`) on Windows. Both take the same JSON
/// messages and report the same button actions.
enum Transport {
    Helper {
        child: crate::capture::ui_process::UiChild,
        stdin: Option<std::process::ChildStdin>,
    },
    InProcess(Option<flume::Sender<serde_json::Value>>),
}

/// Re-read permissions for the app's lifetime, not a permission window's lifetime. The
/// caller owns delivery; a closed receiver stops the worker. Injection keeps the refresh
/// path testable without asking the OS for permissions.
#[cfg(any(target_os = "macos", test))]
pub fn poll_grants(
    mut read: impl FnMut() -> Option<Grants>,
    mut deliver: impl FnMut(Grants) -> bool,
    mut pause: impl FnMut(),
) {
    loop {
        if let Some(grants) = read()
            && !deliver(grants)
        {
            break;
        }
        pause();
    }
}

/// The open welcome window: the `Flow` and the window drawing it. On macOS the app
/// supplies permission updates even while this window is closed; other platforms poll here.
pub struct Session {
    flow: Flow,
    fallback: Option<Box<dyn Fn() -> anyhow::Result<Transport>>>,
    fallback_pending: bool,
    transport: Transport,
    polling: std::sync::Arc<std::sync::atomic::AtomicBool>,
    last_view: String,
    shortcut_label: String,
    shortcut_state: Option<serde_json::Value>,
    saved_resume: Option<Step>,
    saved_tutorial: Option<Tutorial>,
    saved_shortcut_chosen: bool,
    saved_first_capture_completed: bool,
    clock: std::time::Instant,
}

impl Session {
    /// State-only transport: tests exercise coordinator lifecycle without a window or user config.
    #[cfg(test)]
    pub(crate) fn for_test(flow: Flow) -> Self {
        Self {
            flow,
            fallback: None,
            fallback_pending: false,
            transport: Transport::InProcess(None),
            polling: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_view: String::new(),
            shortcut_label: String::new(),
            shortcut_state: None,
            saved_resume: None,
            saved_tutorial: None,
            saved_shortcut_chosen: false,
            saved_first_capture_completed: false,
            clock: std::time::Instant::now(),
        }
    }

    pub fn open(
        flow: Flow,
        shortcut: &str,
        events: impl Fn(Event) + Send + Sync + Clone + 'static,
    ) -> anyhow::Result<Self> {
        use std::sync::atomic::AtomicBool;
        #[cfg(not(target_os = "macos"))]
        use std::sync::atomic::Ordering;

        let polling = std::sync::Arc::new(AtomicBool::new(true));
        let event_alive = polling.clone();
        let events = move |event| {
            if event_alive.load(std::sync::atomic::Ordering::Acquire) { events(event); }
        };
        let electron = super::presentation::assets().filter(|(app, _)| app.join("dist/onboarding.html").is_file() && app.join("electron/onboarding.cjs").is_file());
        let native_events = events.clone();
        let native_shortcut = shortcut.to_owned();
        let native = move || {
            if cfg!(target_os = "macos") {
                Self::spawn_helper(&native_shortcut, native_events.clone())
            } else {
                let (sender, receiver) = flume::unbounded();
                let window_events = native_events.clone();
                crate::capture::desktop::settings::onboarding_window::request_open(
                    receiver, native_shortcut.clone(),
                    std::sync::Arc::new(move |event| window_events(event)),
                );
                Ok(Transport::InProcess(Some(sender)))
            }
        };
        let attempted = electron.and_then(|(app, runtime)| {
            let mut command = std::process::Command::new(runtime);
            crate::capture::ui_process::configure_build_identity(&mut command);
            crate::capture::desktop::workflow::error_reporting::configure_child(&mut command);
            Self::spawn_transport(command.arg(app).arg("--onboarding"), events.clone()).ok()
        });
        let (transport, fallback) = match attempted {
            Some(transport) => (transport, Some(Box::new(native) as Box<dyn Fn() -> anyhow::Result<Transport>>)),
            None => (native()?, None),
        };

        #[cfg(not(target_os = "macos"))]
        let alive = std::sync::Arc::clone(&polling);
        #[cfg(not(target_os = "macos"))]
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
                .with_tutorial(progress.tutorial)
                .with_first_capture(
                    crate::shared::handoff::experiment("onboardingFirstCapture"),
                    progress.shortcut_chosen,
                    progress.first_capture_completed,
                ),
            transport,
            fallback,
            fallback_pending: false,
            polling,
            last_view: String::new(),
            shortcut_label: String::new(),
            shortcut_state: None,
            saved_resume: None,
            saved_tutorial: progress.tutorial,
            saved_shortcut_chosen: progress.shortcut_chosen,
            saved_first_capture_completed: progress.first_capture_completed,
            clock: std::time::Instant::now(),
        };
        session.shortcut_changed(shortcut);
        session.sync();
        Ok(session)
    }

    fn spawn_helper(
        shortcut: &str,
        events: impl Fn(Event) + Send + Sync + Clone + 'static,
    ) -> anyhow::Result<Transport> {
        use std::process::{Command, Stdio};

        let mut command = Command::new(crate::capture::desktop::helper::path()?);
        crate::capture::ui_process::configure_build_identity(&mut command);
        command
            .args(["onboarding", "--shortcut", shortcut, "--paste"])
            .arg(crate::capture::desktop::settings::paste_label())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let dev_icon = concat!(env!("CARGO_MANIFEST_DIR"), "/capture/desktop/assets/app-icon.png");
        if Path::new(dev_icon).is_file() {
            command.args(["--icon", dev_icon]);
        }
        Self::spawn_transport(&mut command, events)
    }

    fn spawn_transport(command: &mut std::process::Command, events: impl Fn(Event) + Send + Sync + Clone + 'static) -> anyhow::Result<Transport> {
        use std::io::BufRead;
        command.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
        let mut child = command.spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();

        std::thread::spawn(move || {
            let mut closed = false;
            if let Some(stdout) = stdout {
                let mut reader = std::io::BufReader::new(stdout);
                loop {
                    let mut line = Vec::new();
                    let read = read_protocol_line(&mut reader, &mut line);
                    if !matches!(read, Ok(n) if n > 0 && n <= 65_536) { break; }
                    let Ok(line) = std::str::from_utf8(&line) else { continue; };
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };
                    let Some(action) = value.get("action").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let action = action.to_owned();
                    if matches!(
                        action.as_str(),
                        "shortcut" | "cancel-shortcut" | "reset-shortcut"
                    ) {
                        let mut value = value;
                        if action == "reset-shortcut" && value.get("kind").is_none() {
                            value["kind"] = "record".into();
                        }
                        if let Ok(mut action) = serde_json::from_value(value) {
                            #[cfg(target_os = "macos")]
                            if let super::settings::Action::Shortcut { pressed, .. } = &mut action {
                                pressed.reserved = native_shortcut_reserved(pressed.key_code, pressed.modifiers);
                            }
                            events(Event::Shortcut(action));
                        }
                        continue;
                    }
                    let action = match value.get("permission").and_then(|v| v.as_str()) {
                        Some(permission) => format!("{action}:{permission}"),
                        None => action.to_owned(),
                    };
                    closed |= action == "close";
                    events(Event::Action(action));
                }
            }
            if !closed { events(Event::Exited); }
        });
        Ok(Transport::Helper { child: crate::capture::ui_process::UiChild::new(child), stdin })
    }

    pub fn restore_native(&mut self) -> bool {
        if self.fallback.is_some() && self.flow.hidden {
            // A replacement native window may map before a queued hide reaches it.
            // Defer creating any replacement until capture completion reveals the flow.
            self.fallback_pending = true;
            let old = std::mem::replace(&mut self.transport, Transport::InProcess(None));
            retire_transport(old);
            return true;
        }
        self.fallback_pending = false;
        let Some(fallback) = self.fallback.take() else { return false; };
        let old = std::mem::replace(&mut self.transport, Transport::InProcess(None));
        retire_transport(old);
        match fallback() {
            Ok(transport) => {
                self.transport = transport;
                self.last_view.clear();
                let label = self.shortcut_label.clone();
                self.shortcut_changed(&label);
                if let Some(editor) = self.shortcut_state.clone() { self.send(&editor); }
                self.sync();
                if self.flow.hidden { self.send(&serde_json::json!({"type": "hide"})); }
                true
            }
            Err(_) => false,
        }
    }

    pub fn mode(&self) -> Mode {
        self.flow.mode()
    }

    pub fn leave(&self, exited: bool) {
        // A helper crash was not a completed tour before this experiment either.
        if exited && !self.flow.first_capture_required {
            return;
        }
        let mut progress = Progress::load();
        progress.leave(&self.flow);
        progress.save();
    }

    /// Feeds the flow and carries out what the window can do itself. Returns what needs the
    /// app: `Close` or `Relaunch`.
    pub fn input(&mut self, input: Input) -> Effect {
        let effect = self.flow.on(input);
        // The in-process window feeds the flow on every refresh: log only what changed.
        static LAST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
        let line = format!("onboarding: {:?} -> {effect:?}", self.flow.step());
        let mut last = LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if *last != line {
            crate::capture::engine::runtime::apparent_log(&line);
            *last = line;
        }
        drop(last);
        let result = match effect {
            // Attributed to whichever process asks, so Apparent itself asks.
            Effect::Request(Permission::ScreenRecording) => {
                crate::capture::engine::runtime::request_screen_capture();
                Effect::None
            }
            Effect::Request(permission) => {
                self.permission_action("request", permission);
                Effect::None
            }
            Effect::OpenSettings(permission) => {
                self.permission_action("open-settings", permission);
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
                        && crate::shared::windows::macos_is_asking(&objc2::rc::autoreleasepool(|_| {
                            crate::shared::windows::raw_windows()
                        }))
                    {
                        std::thread::sleep(std::time::Duration::from_millis(300));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1_200));
                    let result = crate::capture::engine::runtime::screen_capture_probe();
                    crate::capture::engine::runtime::apparent_log(format!(
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
            "change-shortcut" => Input::ChangeShortcut,
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
        if self.fallback_pending && !self.flow.hidden { self.restore_native(); }
        self.send(&serde_json::json!({"type": "show"}));
    }

    /// Keep every step's displayed Record keys in sync with the active shortcut.
    pub fn shortcut_changed(&mut self, label: &str) {
        self.shortcut_label = label.to_owned();
        self.send(&serde_json::json!({"type": "shortcut", "label": label}));
    }

    pub fn shortcut_editor(&mut self, editor: &crate::capture::desktop::settings::ShortcutView) {
        let message = serde_json::json!({"type": "shortcut-editor", "shortcut": editor});
        self.shortcut_state = Some(message.clone());
        self.send(&message);
    }

    pub(crate) fn local_practice(&self) -> bool {
        self.flow.local_practice()
    }

    pub(crate) fn choosing_shortcut(&self) -> bool {
        self.flow.step() == Step::Shortcut
    }

    pub fn editing_shortcut(&self) -> bool {
        self.flow.step() == Step::Shortcut && self.flow.shortcut_editing
    }

    fn sync(&mut self) {
        #[cfg(test)]
        if matches!(self.transport, Transport::InProcess(None)) {
            return;
        }
        #[cfg(target_os = "macos")]
        {
            for row in self.flow.view().rows.iter().filter(|row| row.status == Status::Granted) {
                permission_helper::close_guide(row.permission);
            }
        }
        let mut view = serde_json::to_value(self.flow.view()).unwrap_or_default();
        view["type"] = "state".into();
        view["speech"] = serde_json::to_value(crate::capture::engine::runtime::audio::transcription::speech_model::current()).unwrap_or_default();
        let text = view.to_string();
        if text != self.last_view {
            self.send(&view);
            self.last_view = text;
        }
        // Remember the step, so whatever quits Apparent, the tour comes back here.
        let resume = self.flow.resume_point();
        let tutorial = self.flow.tutorial();
        let new_resume = resume.is_some() && resume != self.saved_resume;
        let milestones_changed = self.flow.mode == Mode::Tour
            && (self.saved_shortcut_chosen != self.flow.shortcut_chosen
                || self.saved_first_capture_completed != self.flow.first_capture_completed);
        if new_resume || tutorial != self.saved_tutorial || milestones_changed {
            let mut progress = Progress::load();
            if new_resume {
                progress.resume = resume;
            }
            progress.tutorial = tutorial;
            if self.flow.mode == Mode::Tour {
                progress.shortcut_chosen = self.flow.shortcut_chosen;
                progress.first_capture_completed = self.flow.first_capture_completed;
            }
            progress.save();
            self.saved_resume = resume.or(self.saved_resume);
            self.saved_tutorial = tutorial;
            self.saved_shortcut_chosen = self.flow.shortcut_chosen;
            self.saved_first_capture_completed = self.flow.first_capture_completed;
        }
    }

    // Electron owns presentation; the installed helper retains consent attribution and
    // the lifetime of asynchronous native prompts. Never send it a presentation state.
    fn permission_action(&mut self, action: &str, permission: Permission) {
        if self.fallback_pending { return; }
        if self.fallback.is_none() {
            self.send(&serde_json::json!({"type": action, "permission": permission}));
            return;
        }
        #[cfg(target_os = "linux")]
        crate::capture::desktop::linux::privacy::open_settings(permission);
        #[cfg(target_os = "macos")]
        match permission_helper::dispatch(action, permission, None, None) {
            permission_helper::Dispatch::Queued => {}
            permission_helper::Dispatch::Busy => crate::capture::engine::runtime::apparent_log("permission helper busy"),
            permission_helper::Dispatch::Unavailable => crate::capture::engine::runtime::apparent_log("permission helper unavailable"),
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

fn read_protocol_line(reader: &mut impl std::io::BufRead, line: &mut Vec<u8>) -> std::io::Result<usize> {
    use std::io::BufRead;
    std::io::Read::take(reader, 65_537).read_until(b'\n', line)
}

fn retire_transport(transport: Transport) {
    if let Transport::Helper { mut child, stdin } = transport {
        drop(stdin);
        std::thread::spawn(move || {
            for _ in 0..20 {
                if !matches!(child.try_wait(), Ok(None)) { return; }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        });
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.polling
            .store(false, std::sync::atomic::Ordering::Release);
        self.send(&serde_json::json!({"type": "close"}));
        let transport = std::mem::replace(&mut self.transport, Transport::InProcess(None));
        retire_transport(transport);
    }
}

/// Starts Apparent again once this copy has quit: the new copy waits for this one
/// (`win::shell::wait_for_relauncher`) before it starts the app.
#[cfg(windows)]
pub fn relaunch() {
    use std::os::windows::process::CommandExt;
    /// No console window flashes up for the new copy (`apparent.exe` is a console program).
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env(crate::capture::engine::runtime::platform::legacy_windows::platform::shell::RELAUNCHED_BY, std::process::id().to_string())
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

/// Starts Apparent again once this copy has quit (after a Screen Recording grant). The app is
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
            .args(["apparent-relaunch", &pid])
            .arg(app),
        None => command
            .arg("-c")
            .arg(format!("{AFTER_EXIT}exec \"$@\""))
            .args(["apparent-relaunch", &pid])
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
mod tests;

#[cfg(test)]
mod local_practice_tests {
    use super::*;

    #[test]
    fn optional_practice_stays_local_with_or_without_explicit_shortcut_choice() {
        for chosen in [false, true] {
            let mut flow = Flow::tour(Step::TryIt, None).with_first_capture(false, chosen, false);
            assert!(!flow.local_practice());
            flow.on(Input::ShortcutTriggered);
            assert!(flow.local_practice(), "opting out must not enable practice uploads");
            flow.on(Input::CaptureStarted);
            flow.on(Input::Closed);
            assert!(flow.local_practice(), "closing the tour must not change practice policy");
            flow.on(Input::CaptureEnded { completed: true });
            assert!(!flow.local_practice());
        }
    }

    #[test]
    fn practice_policy_follows_shortcut_attempt_and_survives_processing() {
        let mut flow = Flow::tour(Step::TryIt, None).with_first_capture(true, true, false);
        assert!(!flow.local_practice());
        flow.on(Input::ShortcutTriggered);
        assert!(flow.local_practice());
        flow.on(Input::CaptureStarted);
        assert!(flow.local_practice());
        flow.on(Input::CaptureEnded { completed: false });
        assert!(!flow.local_practice());
        flow.on(Input::CaptureStarted);
        assert!(!flow.local_practice(), "menu captures cannot inherit practice status");
    }
}

#[cfg(test)]
mod adapter_regressions {
    use super::*;
    #[test]
    fn bounded_reader_limits_each_line_without_limiting_the_stream() {
        let mut reader = std::io::Cursor::new(b"{}\n{}\n");
        for _ in 0..2 {
            let mut line = Vec::new();
            assert_eq!(read_protocol_line(&mut reader, &mut line).unwrap(), 3);
        }
        let mut reader = std::io::Cursor::new(vec![b'x'; 100_000]);
        let mut line = Vec::new();
        assert_eq!(read_protocol_line(&mut reader, &mut line).unwrap(), 65_537);
        assert_eq!(line.len(), 65_537);
    }
    #[test]
    fn hidden_fallback_waits_until_capture_ends_before_creating_native_window() {
        let mut flow = Flow::tour(Step::TryIt, None);
        flow.hidden = true;
        let mut session = Session::for_test(flow);
        session.shortcut_label = "Test shortcut".into();
        let (tx, rx) = flume::unbounded();
        session.fallback = Some(Box::new(move || Ok(Transport::InProcess(Some(tx.clone())))));
        assert!(session.restore_native());
        assert!(rx.try_recv().is_err(), "no replacement transport is created while hidden");
        assert!(session.fallback_pending);
        assert!(session.fallback.is_some());
        session.input(Input::CaptureEnded { completed: false });
        let messages: Vec<_> = rx.try_iter().collect();
        assert_eq!(messages.last().unwrap()["type"], "show");
        assert!(messages.iter().any(|v| v["type"] == "show"));
        assert!(messages.iter().any(|v| v["type"] == "state"));
        assert!(messages.iter().any(|v| v["label"] == "Test shortcut"));
        assert!(!session.restore_native());
    }
}

/// Runs on the transport reader, never on the GUI thread. Missing native evidence rejects.
#[cfg(target_os = "macos")]
fn native_shortcut_reserved(key: u32, modifiers: u32) -> bool {
    use std::{process::Command, time::Duration};
    if key > 127 || modifiers & !0x1b00 != 0 { return true; }
    let Ok(path) = crate::capture::desktop::helper::path() else { return true; };
    let mut command = Command::new(path);
    command.args(["onboarding", "--validate-shortcut", &key.to_string(), &modifiers.to_string()]);
    query_shortcut_reservation(&mut command, Duration::from_secs(2))
}

#[cfg(target_os = "macos")]
pub(crate) fn host_shortcut_reserved(key: u32, modifiers: u32) -> bool {
    native_shortcut_reserved(key, modifiers)
}

#[cfg(target_os = "linux")]
pub(crate) fn host_shortcut_reserved(_key: u32, _modifiers: u32) -> bool { false }

#[cfg(any(target_os = "macos", test))]
fn query_shortcut_reservation(command: &mut std::process::Command, timeout: std::time::Duration) -> bool {
    use std::{io::Read, process::Stdio, time::{Duration, Instant}};
    let Ok(mut child) = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else { return true; };
    let stdout = child.stdout.take();
    let mut child = crate::capture::ui_process::UiChild::new(child);
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => { let _ = child.kill(); let _ = child.wait(); return true; }
        }
    }
    let mut bytes = Vec::new();
    let Some(stdout) = stdout else { return true; };
    if stdout.take(33).read_to_end(&mut bytes).is_err() { return true; }
    shortcut_reservation_projection(&bytes)
}

#[cfg(any(target_os = "macos", test))]
fn shortcut_reservation_projection(bytes: &[u8]) -> bool {
    if bytes.len() > 32 { return true; }
    serde_json::from_slice::<bool>(bytes).unwrap_or(true)
}

#[test]
fn shortcut_native_projection_fails_closed() {
    assert!(!shortcut_reservation_projection(b"false\n"));
    for bytes in [b"true".as_slice(), b"", b"null", b"{}", b"false true", b"{\"reserved\":false}"] {
        assert!(shortcut_reservation_projection(bytes));
    }
}

#[cfg(all(test, unix))]
#[test]
fn shortcut_native_process_failures_reject_without_accepting_renderer_claims() {
    use std::{process::Command, time::Duration};
    let query = |script: &str| {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        query_shortcut_reservation(&mut command, Duration::from_millis(100))
    };
    assert!(!query("printf false"));
    assert!(query("printf true"));
    assert!(query("printf false; exit 1"));
    assert!(query("printf invalid"));
    assert!(query("exec sleep 2"));
    assert!(query_shortcut_reservation(&mut Command::new("/nonexistent/apparent-test-helper"), Duration::from_millis(100)));
}
