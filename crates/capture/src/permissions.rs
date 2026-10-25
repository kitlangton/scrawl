//! Permission grants read from macOS (`scrawl-helper permissions`) or Windows (`win::privacy`).

use std::collections::BTreeMap;

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
    pub fn usable(&self, permission: Permission) -> bool {
        self.granted(permission)
            && !(permission == Permission::ScreenRecording && self.screen_needs_relaunch)
    }
}

/// Reads what's granted now from a fresh helper, plus whether this process can use a Screen
/// Recording grant yet. Only once the helper says it's granted does this process try a real
/// ScreenCaptureKit capture (`capture::screen_capture_usable`), so no prompt can come of it.
pub fn read_grants() -> Option<Grants> {
    let statuses = crate::helper::permissions()
        .inspect_err(|error| crate::log::scrawl_log(format!("permissions: {error:#}")))
        .ok()?;
    let screen = statuses.get(&Permission::ScreenRecording) == Some(&Status::Granted);
    let grants = Grants {
        statuses,
        screen_needs_relaunch: screen && !crate::capture::screen_capture_usable(true),
    };
    crate::speech_model::note_grants(&grants);
    Some(grants)
}
