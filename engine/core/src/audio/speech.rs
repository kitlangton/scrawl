//! Existing Parakeet session execution, without model discovery/download or transcript policy.
//!
//! The caller supplies 16 kHz mono PCM and an already-authorized local model path. This
//! synchronous mechanism does not impose a process deadline or authenticate the model.
//! Capture/2 must provide a pinned verified model and supervised cancellation/reaping before
//! adopting it. A present file or successful construction is not public voice readiness.
use anyhow::Result;
use std::path::Path;
use transcribe_cpp::{Model, RunOptions, Session, TimestampKind, Transcript};

/// One reusable native session. Each run replaces native results and returns owned output.
pub struct Recognizer {
    session: Session,
}

impl Recognizer {
    pub fn load(path: &Path) -> Result<Self> {
        init_backends()?;
        Ok(Self {
            session: Model::load(path)?.session()?,
        })
    }

    pub fn run(&mut self, samples: &[f32]) -> Result<Transcript> {
        let options = english_word_options();
        Ok(self.session.run(samples, &options)?)
    }
}

fn english_word_options() -> RunOptions {
    RunOptions {
        timestamps: TimestampKind::Word,
        language: Some("en".into()),
        ..Default::default()
    }
}

/// The native registry cannot be mutated concurrently with model loading.
pub fn init_backends() -> Result<()> {
    static INIT: std::sync::OnceLock<std::result::Result<(), String>> = std::sync::OnceLock::new();
    INIT.get_or_init(|| {
        transcribe_cpp::disable_logging();
        transcribe_cpp::init_backends_default().map_err(|error| format!("{error:#}"))
    })
    .clone()
    .map_err(|error| anyhow::anyhow!("transcribe.cpp backends: {error}"))
}

/// Linux descriptor-bound native execution for the capture worker's supervised child.
/// The caller must enforce a hard child deadline/cancellation/reaping independently:
/// native load/run are synchronous. No code path here discovers or fetches a model.
#[cfg(target_os = "linux")]
pub struct PinnedRecognizer {
    recognizer: Recognizer,
    model: super::model::ModelFile,
}

#[cfg(target_os = "linux")]
impl PinnedRecognizer {
    pub fn load(model: super::model::ModelFile, deadline: std::time::Instant) -> Result<Self> {
        use std::os::fd::AsRawFd;
        model.revalidate(deadline)?;
        // Native fopen follows only this write-sealed snapshot, never mutable source bytes.
        let path = std::path::PathBuf::from(format!("/proc/self/fd/{}", model.as_raw_fd()));
        let recognizer = Recognizer::load(&path)?;
        model.revalidate(deadline)?;
        Ok(Self { recognizer, model })
    }

    pub fn run(&mut self, samples: &[f32], deadline: std::time::Instant) -> Result<Transcript> {
        anyhow::ensure!(std::time::Instant::now() < deadline, "speech deadline");
        anyhow::ensure!(
            samples.iter().all(|s| s.is_finite()),
            "nonfinite speech samples"
        );
        self.model.revalidate(deadline)?;
        let result = self.recognizer.run(samples)?;
        // No result may acquire original-fact authority after lost seals or late return.
        self.model.revalidate(deadline)?;
        Ok(result)
    }
}
