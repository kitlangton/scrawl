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

