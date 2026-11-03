//! Typed failure breadcrumbs shared with consumers; never format arbitrary errors for telemetry.
use anyhow::Result;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Step {
    PhoneStop,
    AudioStop,
    NotesFinish,
    NotesRead,
    NoScreenFrames,
    AudioPersist,
    SourcePreflight,
    RecoveryJournal,
    BundleBuild,
    AudioMissing,
    ArtifactWrite,
    TranscriptPending,
    Publication,
    PublicationValidate,
    PngValidate,
    WavValidate,
    MovieValidate,
}

#[derive(Debug, Serialize)]
pub(crate) struct Breadcrumb {
    pub step: Step,
    pub file: &'static str,
    pub line: u32,
    #[serde(skip)]
    source: anyhow::Error,
}
impl std::fmt::Display for Breadcrumb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.source, f)
    }
}
impl std::error::Error for Breadcrumb {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

pub(crate) trait Diagnose<T> {
    #[track_caller]
    fn diagnostic(self, step: Step) -> Result<T>;
}
impl<T> Diagnose<T> for Result<T> {
    #[track_caller]
    fn diagnostic(self, step: Step) -> Result<T> {
        let location = std::panic::Location::caller();
        self.map_err(|source| {
            anyhow::Error::new(Breadcrumb {
                step,
                file: location.file(),
                line: location.line(),
                source,
            })
        })
    }
}

/// Reproduction inputs only. No target title, coordinates, device or bundle identity.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct CaptureFacts {
    pub mode: crate::shared::model::CaptureMode,
    pub duration_ms: Option<u64>,
    pub candidate_frames: Option<usize>,
    pub has_video: Option<bool>,
    pub microphone_recorded: Option<bool>,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
    pub notes_pending: bool,
    pub strokes: usize,
    pub interactions: usize,
}

#[derive(Debug)]
pub(crate) struct CaptureFailure {
    pub facts: CaptureFacts,
    source: anyhow::Error,
}
impl CaptureFacts {
    pub fn attach(self, source: anyhow::Error) -> anyhow::Error {
        anyhow::Error::new(CaptureFailure {
            facts: self,
            source,
        })
    }
}
impl std::fmt::Display for CaptureFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.source, f)
    }
}
impl std::error::Error for CaptureFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}
