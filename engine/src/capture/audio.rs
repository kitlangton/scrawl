//! Original narration uses the capture's existing owner/journal and Store FD.
//! No detached inference, material rewrite, device settings writer or new lease.
use super::{Shared, hash_file, operation_dir, persist};
use apparent_capture_engine::audio::{
    persistence::{DescriptorWriter, Samples},
    timeline::Timeline,
};
use apparent_capture_engine::linux::microphone::{PcmTiming, Stream};
use serde_json::{Value, json};
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    timeline: Timeline,
    first: Option<PcmTiming>,
    failed: bool,
}

pub(super) struct Capture {
    input: Option<Stream>,
    writer: Option<DescriptorWriter>,
    state: Arc<Mutex<State>>,
    disk_error: Arc<Mutex<Option<String>>>,
}

pub(super) struct Completed {
    pub file: File,
    pub duration_ms: u64,
    pub initial_gap_ms: u64,
    pub clock: Value,
}

impl Completed {
    pub(super) fn seal(
        &self,
        directory: &apparent_capture_engine::SecureStore,
        capture_duration: u64,
    ) -> Result<Value, &'static str> {
        directory
            .validate(&self.file)
            .map_err(|_| "source_mismatch")?;
        let current = directory
            .file("audio.wav", false)
            .map_err(|_| "source_mismatch")?;
        let meta = self.file.metadata().map_err(|_| "invalid_materials")?;
        let bound = current.metadata().map_err(|_| "source_mismatch")?;
        if meta.dev() != bound.dev()
            || meta.ino() != bound.ino()
            || self.initial_gap_ms >= self.duration_ms
            || self.initial_gap_ms > capture_duration
        {
            return Err("invalid_materials");
        }
        self.file.sync_all().map_err(|_| "invalid_materials")?;
        Ok(
            json!({"relativePath":"audio.wav","identity":{"device":meta.dev().to_string(),"inode":meta.ino().to_string(),"size":meta.len(),"modifiedNs":(meta.mtime() as i128*1_000_000_000+meta.mtime_nsec() as i128).to_string()},"sha256":hash_file(self.file.as_raw_fd()).map_err(|_| "invalid_materials")?}),
        )
    }
}

impl Capture {
    pub(super) fn start(shared: &Shared) -> Result<Self, &'static str> {
        let directory = operation_dir(shared)?;
        let file = directory
            .file("audio.wav", true)
            .map_err(|_| "source_mismatch")?;
        let m = file.metadata().map_err(|_| "internal")?;
        {
            let mut j = shared.journal.lock().map_err(|_| "internal")?;
            j.partial_audio =
                Some(json!({"device":m.dev().to_string(),"inode":m.ino().to_string()}));
        }
        // Identity is durable before opening input or writing any WAV bytes.
        persist(shared).map_err(|_| "internal")?;
        let state = Arc::new(Mutex::new(State::default()));
        let samples = Arc::new(Mutex::new(Samples::default()));
        let disk_error = Arc::new(Mutex::new(None));
        let callback_state = state.clone();
        let callback_samples = samples.clone();
        let input = Stream::start_default_with_clock(shared.epoch, move |pcm, timing| {
            let Ok(mut state) = callback_state.lock() else {
                return;
            };
            if state.failed {
                return;
            }
            if state
                .timeline
                .accept(timing.running_pts_ns, timing.capture_elapsed_ns, pcm)
                .is_err()
            {
                state.failed = true;
                return;
            }
            state.first.get_or_insert(timing);
            match callback_samples.lock() {
                Ok(mut samples) => samples.extend_from_slice(pcm),
                Err(_) => state.failed = true,
            }
        })
        .map_err(|_| "unavailable")?;
        let gap = {
            let state = state.lock().map_err(|_| "internal")?;
            if state.failed {
                return Err("invalid_materials");
            }
            state
                .timeline
                .finish()
                .map_err(|_| "invalid_materials")?
                .0
                .initial_gap_ms
        };
        let writer = DescriptorWriter::start(file, samples, disk_error.clone(), gap)
            .map_err(|_| "invalid_materials")?;
        Ok(Self {
            input: Some(input),
            writer: Some(writer),
            state,
            disk_error,
        })
    }

    pub(super) fn check(&self) -> Result<(), &'static str> {
        if self
            .input
            .as_ref()
            .is_some_and(|input| input.error().is_some())
            || self.state.lock().map_err(|_| "internal")?.failed
            || self.disk_error.lock().map_err(|_| "internal")?.is_some()
        {
            return Err("invalid_materials");
        }
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<Completed, &'static str> {
        self.check()?;
        self.input
            .as_ref()
            .ok_or("internal")?
            .stop()
            .map_err(|_| "invalid_materials")?;
        self.input.take();
        // Null/drain precedes both final clock validation and the disk join.
        let state = self.state.lock().map_err(|_| "internal")?;
        if state.failed {
            return Err("invalid_materials");
        }
        let (origin, source_samples) = state.timeline.finish().map_err(|_| "invalid_materials")?;
        let first = state.first.ok_or("invalid_materials")?;
        drop(state);
        let file = self
            .writer
            .take()
            .ok_or("internal")?
            .finish()
            .map_err(|_| "invalid_materials")?;
        let total = origin.initial_gap_ms * 16 + source_samples;
        if file.metadata().map_err(|_| "invalid_materials")?.len() != 44 + total * 2 {
            return Err("invalid_materials");
        }
        Ok(Completed {
            file,
            duration_ms: total.div_ceil(16),
            initial_gap_ms: origin.initial_gap_ms,
            clock: json!({"sampleRate":16000,"channels":1,"sourceRunningPtsNs":first.running_pts_ns.to_string(),"sourceCaptureNs":origin.source_capture_ns.to_string(),"initialGapMs":origin.initial_gap_ms,"sourceSamples":source_samples,"originRounding":"ceil-integer-millisecond-once"}),
        })
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(input) = self.input.take() {
            let _ = input.stop();
        }
        // The donor writer joins after input teardown, even on a refused result.
        self.writer.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn original_audio_seal_matches_padded_wav_and_refuses_a_replaced_leaf() {
        let root = std::env::temp_dir().join(format!(
            "apparent-audio-seal-{}",
            apparent_capture_engine::new_id().unwrap()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = apparent_capture_engine::SecureStore::open(&root).unwrap();
        let file = store.file("audio.wav", true).unwrap();
        let mut timeline = Timeline::default();
        let pcm = [0.25; 160];
        let origin = timeline.accept(5_000_000, 2_001_000, &pcm).unwrap();
        let samples = Arc::new(Mutex::new(Samples::from(pcm.to_vec())));
        let mut file =
            DescriptorWriter::start(file, samples, Arc::default(), origin.initial_gap_ms)
                .unwrap()
                .finish()
                .unwrap();
        let (origin, count) = timeline.finish().unwrap();
        let mut bytes = Vec::new();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.read_to_end(&mut bytes).unwrap();
        let decoded = apparent_capture_engine::audio::decode_wav_pcm16(&bytes).unwrap();
        assert_eq!(decoded.len(), 3 * 16 + count as usize);
        assert!(decoded[..48].iter().all(|v| *v == 0));
        assert!(decoded[48..].iter().all(|v| *v == 8192));
        let audio = Completed {
            file,
            duration_ms: 13,
            initial_gap_ms: origin.initial_gap_ms,
            clock: json!({}),
        };
        let sealed = audio.seal(&store, 13).unwrap();
        assert_eq!(sealed["sha256"], super::super::sha(&bytes));
        assert_eq!(sealed["identity"]["size"], bytes.len());
        assert!(
            audio.seal(&store, 2).is_err(),
            "warning cannot lie beyond capture clock"
        );
        std::fs::rename(root.join("audio.wav"), root.join("held.wav")).unwrap();
        let replacement = store.file("audio.wav", true).unwrap();
        assert!(audio.seal(&store, 13).is_err());
        drop((replacement, audio, store));
        std::fs::remove_dir_all(root).unwrap();
    }
}
