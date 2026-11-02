//! Pause-ahead transcription for the local Linux Parakeet engine.
//! Completed speech is processed while capture continues; the saved WAV supplies the tail.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};

use crate::capture::engine::runtime::audio::persistence::Samples;
use crate::shared::model::TranscriptData;

const RATE: usize = 16_000;
const QUIET_MS: usize = 700;
const PAD_MS: usize = 200;
const MIN_CHUNK_MS: usize = 3_000;
const MAX_PENDING_MS: usize = 60_000;

pub fn wanted() -> bool {
    cfg!(target_os = "linux")
        && crate::capture::engine::runtime::audio::transcription::transcribe::Engine::preferred() == crate::capture::engine::runtime::audio::transcription::transcribe::Engine::Apple
}

pub struct Pause {
    stop: Arc<AtomicBool>,
    tail: Sender<Vec<u8>>,
    worker: Option<JoinHandle<Result<TranscriptData>>>,
}

impl Pause {
    pub fn start(samples: Arc<Mutex<Samples>>) -> Result<Self> {
        crate::capture::engine::runtime::audio::transcription::warm_recognizer::clear_if_disabled();
        if crate::shared::handoff::warm_recognizer() {
            return Self::start_with(samples, crate::capture::engine::runtime::audio::transcription::warm_recognizer::transcriber()?);
        }
        let mut model: Option<crate::capture::engine::runtime::audio::transcription::speech::Incremental> = None;
        Self::start_with(samples, move |chunk, offset| {
            if model.is_none() {
                model = Some(crate::capture::engine::runtime::audio::transcription::speech::Incremental::new()?);
            }
            model.as_mut().unwrap().transcribe(chunk, offset)
        })
    }

    pub(super) fn start_with(
        samples: Arc<Mutex<Samples>>,
        mut transcribe: impl FnMut(&[f32], u64) -> Result<TranscriptData> + Send + 'static,
    ) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (tail, finished) = mpsc::channel::<Vec<u8>>();
        let worker = std::thread::Builder::new()
            .name("apparent-pause-transcribe".into())
            .spawn(move || {
                let mut consumed = 0;
                let mut read_at = 0;
                let mut pending = Vec::new();
                let mut parts = Vec::new();
                while !worker_stop.load(Ordering::Acquire) {
                    {
                        let captured = samples
                            .lock()
                            .map_err(|_| anyhow!("Microphone buffer poisoned"))?;
                        if captured.len() > read_at {
                            ensure!(
                                captured.len() - read_at
                                    <= MAX_PENDING_MS * RATE / 1_000 - pending.len(),
                                "Pause transcription fell behind the recording"
                            );
                            pending.extend_from_slice(
                                captured
                                    .get(read_at..)
                                    .context("Pause transcription fell behind the recording")?,
                            );
                            read_at = captured.len();
                        }
                    }
                    if let Some((start, end)) = pause_boundary(&pending) {
                        let part = transcribe(
                            &pending[start..end],
                            (consumed + start) as u64 * 1_000 / RATE as u64,
                        )?;
                        parts.push(part);
                        pending.drain(..end);
                        consumed += end;
                    } else {
                        std::thread::park_timeout(Duration::from_millis(160));
                    }
                }
                let wav = finished.recv().context("No finished recording")?;
                let all = crate::capture::engine::runtime::audio::audio::decode_wav_16k(&wav)?;
                ensure!(
                    consumed <= all.len(),
                    "Pause exceeded the finished recording"
                );
                if consumed < all.len() {
                    parts.push(transcribe(
                        &all[consumed..],
                        consumed as u64 * 1_000 / RATE as u64,
                    )?);
                }
                Ok(merge(parts))
            })
            .context("Could not start pause transcriber")?;
        Ok(Self {
            stop,
            tail,
            worker: Some(worker),
        })
    }

    pub fn finish(mut self, wav: &[u8]) -> Result<TranscriptData> {
        self.stop.store(true, Ordering::Release);
        self.tail
            .send(wav.to_vec())
            .context("Pause transcriber stopped")?;
        let worker = self
            .worker
            .take()
            .context("Pause transcriber already finished")?;
        worker.thread().unpark();
        worker
            .join()
            .map_err(|_| anyhow!("Pause transcriber panicked"))?
    }
}

impl Drop for Pause {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Dropping the channel releases the worker if it is waiting for a tail.
        // Do not block capture teardown on a model still running.
    }
}

fn pause_boundary(samples: &[f32]) -> Option<(usize, usize)> {
    let islands = crate::capture::engine::runtime::audio::audio::speech_islands(samples);
    let clips = crate::shared::hex::plan_clips(&islands, samples.len() as u64 * 1_000 / RATE as u64);
    // Match the normal Parakeet path's lead-in; draining still advances through all captured audio.
    let start = clips.first()?.0.saturating_sub(PAD_MS as u64) as usize * RATE / 1_000;
    // Eligibility uses elapsed capture time, not the trimmed input length: a short
    // phrase after initial silence should still run before Stop.
    clips
        .into_iter()
        .map(|(_, end_ms)| end_ms as usize * RATE / 1_000)
        .map(|end| (end, end.saturating_add(PAD_MS * RATE / 1_000)))
        .filter(|&(end, boundary)| {
            boundary >= MIN_CHUNK_MS * RATE / 1_000
                && end + QUIET_MS * RATE / 1_000 <= samples.len()
        })
        .last()
        .map(|(_, boundary)| (start, boundary))
}

/// Feed a saved WAV at microphone speed for a repeatable CLI comparison.
pub fn simulate(wav: &[u8], speed: f64) -> Result<Pause> {
    let samples = crate::capture::engine::runtime::audio::audio::decode_wav_16k(wav)?;
    let buffer = Arc::new(Mutex::new(Samples::default()));
    let pause = Pause::start(Arc::clone(&buffer))?;
    let started = std::time::Instant::now();
    for (i, chunk) in samples.chunks(160).enumerate() {
        buffer
            .lock()
            .map_err(|_| anyhow!("Microphone buffer poisoned"))?
            .extend_from_slice(chunk);
        let due = Duration::from_secs_f64((i + 1) as f64 * 0.01 / speed.max(0.01));
        if let Some(wait) = due.checked_sub(started.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    buffer
        .lock()
        .map_err(|_| anyhow!("Microphone buffer poisoned"))?
        .clear();
    Ok(pause)
}

fn merge(parts: Vec<TranscriptData>) -> TranscriptData {
    let mut result = TranscriptData::default();
    for mut part in parts {
        if result.source.is_empty() {
            result.source = part.source.clone();
            result.model = part.model.clone();
        }
        result.estimated |= part.estimated;
        if !part.text.is_empty() {
            if !result.text.is_empty() {
                result.text.push(' ');
            }
            result.text.push_str(&part.text);
        }
        result.segments.append(&mut part.segments);
        result.words.append(&mut part.words);
    }
    result
}

#[derive(Clone, Default)]
pub struct Slot(Arc<Mutex<Option<Pause>>>);

impl Slot {
    pub fn new(pause: Option<Pause>) -> Self {
        Self(Arc::new(Mutex::new(pause)))
    }
    pub fn take(&self) -> Option<Pause> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Slot")
            .field(&self.0.lock().map(|v| v.is_some()).unwrap_or(false))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn local_recordings_use_pause_transcription_without_an_experiment() {
        // Engine overrides are process-wide. Keep each public policy check isolated
        // from the rest of the suite, with no experiment enabled in the child.
        const CASE: &str = "APPARENT_PAUSE_POLICY_TEST_ENGINE";
        if let Ok(name) = std::env::var(CASE) {
            let engine = crate::capture::engine::runtime::audio::transcription::transcribe::Engine::parse(&name).unwrap();
            crate::capture::engine::runtime::audio::transcription::transcribe::set_override(engine);
            assert!(!crate::shared::handoff::experiment("pauseTranscribe"));
            assert_eq!(wanted(), engine == crate::capture::engine::runtime::audio::transcription::transcribe::Engine::Apple);
            return;
        }
        for name in ["apple", "hex", "command"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "capture::engine::runtime::audio::transcription::pause_transcribe::tests::local_recordings_use_pause_transcription_without_an_experiment",
                    "--exact",
                    "--nocapture",
                ])
                .env(CASE, name)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(output.status.success(), "pause policy for {name}: {stdout}");
            assert!(stdout.lines().any(|line| line == "running 1 test"), "exact pause policy selector must run one test for {name}: {stdout}");
            assert!(stdout.contains("capture::engine::runtime::audio::transcription::pause_transcribe::tests::local_recordings_use_pause_transcription_without_an_experiment ... ok"), "the policy case must pass for {name}: {stdout}");
        }
    }

    #[test]
    fn finished_pause_is_processed_before_stop_and_tail_is_not_lost() {
        let samples = Arc::new(Mutex::new(Samples::default()));
        let (sent, received) = mpsc::channel();
        let pause = Pause::start_with(Arc::clone(&samples), move |chunk, offset| {
            sent.send((chunk.len(), offset)).unwrap();
            Ok(TranscriptData {
                source: "test".into(),
                text: if offset == 0 { "first" } else { "last" }.into(),
                ..Default::default()
            })
        })
        .unwrap();
        samples.lock().unwrap().extend(vec![0.2; 4 * 16_000]);
        samples.lock().unwrap().extend(vec![0.0; 16_000]);
        let first = received
            .recv_timeout(Duration::from_secs(3))
            .expect("pause must be processed during recording");
        assert_eq!(first.1, 0);
        assert!(first.0 >= 4 * 16_000);
        samples.lock().unwrap().extend(vec![0.2; 16_000]);
        let wav = crate::capture::engine::runtime::audio::audio::encode_wav_16k(&samples.lock().unwrap()).unwrap();
        let result = pause.finish(&wav).unwrap();
        assert_eq!(result.text, "first last");
        let second = received.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(second.1 as usize, first.0 * 1_000 / RATE);
        assert_eq!(first.0 + second.0, 6 * 16_000);
    }

    #[test]
    fn early_chunk_trims_leading_silence_without_shifting_word_times() {
        let samples = Arc::new(Mutex::new(vec![0.0; 2 * RATE].into()));
        let (sent, received) = mpsc::channel();
        let pause = Pause::start_with(Arc::clone(&samples), move |chunk, offset| {
            sent.send((chunk.len(), offset)).unwrap();
            Ok(TranscriptData {
                source: "test".into(),
                ..Default::default()
            })
        })
        .unwrap();
        samples.lock().unwrap().extend(vec![0.2; 4 * RATE]);
        samples.lock().unwrap().extend(vec![0.0; RATE]);
        let (length, offset) = received
            .recv_timeout(Duration::from_secs(3))
            .expect("first phrase should be transcribed during recording");
        assert_eq!(offset, 1_800);
        assert_eq!(length, 4 * RATE + 2 * PAD_MS * RATE / 1_000);
        let wav = crate::capture::engine::runtime::audio::audio::encode_wav_16k(&samples.lock().unwrap()).unwrap();
        pause.finish(&wav).unwrap();
    }

    #[test]
    fn short_phrase_after_silence_still_runs_before_stop() {
        let samples = Arc::new(Mutex::new(vec![0.0; 2 * RATE].into()));
        let (sent, received) = mpsc::channel();
        let pause = Pause::start_with(Arc::clone(&samples), move |chunk, offset| {
            sent.send((chunk.len(), offset)).unwrap();
            Ok(TranscriptData::default())
        })
        .unwrap();
        samples.lock().unwrap().extend(vec![0.2; RATE]);
        samples.lock().unwrap().extend(vec![0.0; RATE]);
        let (length, offset) = received
            .recv_timeout(Duration::from_secs(3))
            .expect("a short phrase after silence must be processed during recording");
        assert_eq!(offset, 1_800);
        assert_eq!(length, RATE + 2 * PAD_MS * RATE / 1_000);
        let wav = crate::capture::engine::runtime::audio::audio::encode_wav_16k(&samples.lock().unwrap()).unwrap();
        pause.finish(&wav).unwrap();
    }

    #[test]
    fn tail_audio_is_not_discarded_by_the_speech_detector() {
        let samples = Arc::new(Mutex::new(Samples::from(vec![0.2; 4 * RATE])));
        samples.lock().unwrap().extend(vec![0.0; RATE]);
        let (sent, received) = mpsc::channel();
        let pause = Pause::start_with(Arc::clone(&samples), move |chunk, _| {
            sent.send(chunk.len()).unwrap();
            Ok(TranscriptData {
                source: "test".into(),
                ..Default::default()
            })
        })
        .unwrap();
        let first = received.recv_timeout(Duration::from_secs(3)).unwrap();
        let wav = crate::capture::engine::runtime::audio::audio::encode_wav_16k(&samples.lock().unwrap()).unwrap();
        pause.finish(&wav).unwrap();
        let tail = received
            .recv_timeout(Duration::from_secs(1))
            .expect("retain tail even if it falls below the detector threshold");
        assert_eq!(first + tail, 5 * RATE);
    }

    #[test]
    fn short_islands_and_nearby_speech_wait_for_one_substantial_chunk() {
        let mut samples = vec![0.2; RATE];
        samples.extend(vec![0.0; RATE * 4 / 10]);
        samples.extend(vec![0.2; RATE]);
        samples.extend(vec![0.0; RATE]);
        assert_eq!(pause_boundary(&samples), None);
        samples.extend(vec![0.2; RATE * 2]);
        samples.extend(vec![0.0; RATE]);
        assert_eq!(
            pause_boundary(&samples),
            Some((0, 5 * RATE + RATE * 4 / 10 + RATE / 5))
        );
    }

    #[test]
    fn a_chunk_requires_700_ms_of_quiet_and_keeps_200_ms_margins() {
        let mut samples = vec![0.0; RATE];
        samples.extend(vec![0.2; 3 * RATE]);
        samples.extend(vec![0.0; 699 * RATE / 1_000]);
        assert_eq!(pause_boundary(&samples), None);
        samples.extend(vec![0.0; RATE / 1_000]);
        assert_eq!(
            pause_boundary(&samples),
            Some((800 * RATE / 1_000, 4 * RATE + RATE / 5))
        );
    }

    #[test]
    fn minimum_chunk_is_three_seconds_including_the_trailing_margin() {
        let mut too_short = vec![0.2; 2_750 * RATE / 1_000];
        too_short.extend(vec![0.0; RATE]);
        assert_eq!(pause_boundary(&too_short), None);
        let mut enough = vec![0.2; 2_800 * RATE / 1_000];
        enough.extend(vec![0.0; RATE]);
        assert_eq!(pause_boundary(&enough), Some((0, 3 * RATE)));
    }

    #[test]
    fn a_short_noise_blip_after_silence_is_not_a_transcription_chunk() {
        let mut samples = vec![0.0; 4 * RATE];
        samples.extend(vec![0.2; RATE / 20]);
        samples.extend(vec![0.0; RATE]);
        assert_eq!(pause_boundary(&samples), None);
    }

    #[test]
    fn an_overfull_worker_fails_without_allocating_an_unbounded_backlog() {
        let samples = Arc::new(Mutex::new(vec![
            0.0;
            (MAX_PENDING_MS + 1_000) * RATE / 1_000
        ].into()));
        let mut pause = Pause::start_with(samples, |_, _| panic!("no chunk is safe")).unwrap();
        let worker = pause.worker.take().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !worker.is_finished() {
            assert!(std::time::Instant::now() < deadline, "overfull worker must fail before Stop");
            std::thread::yield_now();
        }
        let error = worker.join().unwrap().unwrap_err();
        assert!(error.to_string().contains("fell behind"));
    }

    #[test]
    fn rolling_capture_keeps_absolute_chunk_and_tail_offsets() {
        let samples = Arc::new(Mutex::new(Samples::default()));
        let (sent, received) = mpsc::channel();
        let pause = Pause::start_with(Arc::clone(&samples), move |chunk, offset| {
            sent.send((chunk.len(), offset)).unwrap();
            Ok(TranscriptData::default())
        }).unwrap();
        let mut all = Vec::new();
        for i in 0..14 {
            let mut chunk = vec![0.2; 4 * RATE];
            chunk.extend(vec![0.0; RATE]);
            all.extend_from_slice(&chunk);
            samples.lock().unwrap().extend_from_slice(&chunk);
            let (_, offset) = received.recv_timeout(Duration::from_secs(3)).unwrap();
            assert_eq!(offset, if i == 0 { 0 } else { i * 5_000 - 200 });
        }
        {
            let captured = samples.lock().unwrap();
            assert_eq!(captured.len(), 70 * RATE);
            assert_eq!(captured.recent().len(), 60 * RATE);
            assert!(captured.get(0..).is_none());
        }
        let wav = crate::capture::engine::runtime::audio::audio::encode_wav_16k(&all).unwrap();
        samples.lock().unwrap().clear();
        pause.finish(&wav).unwrap();
        let (length, offset) = received.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(offset, 69_200);
        assert_eq!(length, 800 * RATE / 1_000);
    }

    /// Local-only benchmark: set APPARENT_PAUSE_TEST_WAV and APPARENT_MODELS_DIR.
    #[test]
    #[ignore]
    fn compare_real_audio_with_batch() {
        let path = std::env::var("APPARENT_PAUSE_TEST_WAV").unwrap();
        let wav = std::fs::read(&path).unwrap();
        let start = std::time::Instant::now();
        let baseline = crate::capture::engine::runtime::audio::transcription::speech::transcribe(std::path::Path::new(&path)).unwrap();
        let batch_ms = start.elapsed().as_millis();
        let pause = simulate(&wav, 1.0).unwrap();
        let stop = std::time::Instant::now();
        let candidate = pause.finish(&wav).unwrap();
        eprintln!(
            "batch_ms={batch_ms} pause_finish_ms={} baseline_words={} pause_words={}",
            stop.elapsed().as_millis(),
            baseline.words.len(),
            candidate.words.len()
        );
        assert!(!baseline.text.is_empty() && !candidate.text.is_empty());
        assert!(candidate.words.windows(2).all(|w| w[0].t0 <= w[1].t0));
    }
}
