use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use super::persistence::{Samples, Writer};

const SAMPLE_RATE: u32 = 16_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputWarning {
    Unavailable,
    Stopped,
    VeryQuiet,
}

impl InputWarning {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "Mic unavailable",
            Self::Stopped => "Mic stopped",
            Self::VeryQuiet => "Mic very quiet",
        }
    }

    pub fn announcement(self) -> &'static str {
        match self {
            Self::Unavailable => "Microphone unavailable; screen recording continues",
            Self::Stopped => "Microphone input stopped; screen recording continues",
            Self::VeryQuiet => "Microphone input is very quiet; screen recording continues",
        }
    }
}

/// Actual captured samples drive this warning; the pill's animation is not a meter.
struct InputHealth {
    started: Instant,
    last_sample: Option<Instant>,
    last_audible: Instant,
    warning: Option<InputWarning>,
    recovering_since: Option<Instant>,
    open_failed: bool,
}

impl InputHealth {
    fn new(now: Instant) -> Self {
        Self {
            started: now,
            last_sample: None,
            last_audible: now,
            warning: None,
            recovering_since: None,
            open_failed: false,
        }
    }

    fn sample(&mut self, now: Instant, peak: f32) {
        self.last_sample = Some(now);
        if peak > 0.005 {
            self.last_audible = now;
        }
        match self.warning {
            Some(InputWarning::Stopped) => { self.recovering_since.get_or_insert(now); }
            Some(InputWarning::VeryQuiet) if peak > 0.005 => {
                self.recovering_since.get_or_insert(now);
            }
            Some(InputWarning::VeryQuiet) => self.recovering_since = None,
            _ => {}
        }
    }

    fn open_failed(&mut self) {
        self.open_failed = true;
        self.warning = Some(InputWarning::Unavailable);
    }

    fn stream_failed(&mut self) {
        self.last_sample = None;
        self.recovering_since = None;
        self.warning = Some(InputWarning::Stopped);
    }

    fn status(&mut self, now: Instant) -> Option<InputWarning> {
        if self.open_failed {
            return Some(InputWarning::Unavailable);
        }
        let stalled = self.last_sample.map_or(
            now.duration_since(self.started) >= Duration::from_secs(4),
            |last| now.duration_since(last) >= Duration::from_secs(3),
        );
        if stalled {
            self.warning = Some(InputWarning::Stopped);
            self.recovering_since = None;
        } else if self.warning.is_some()
            && self.recovering_since.is_some_and(|since| now.duration_since(since) >= Duration::from_secs(1))
        {
            self.warning = None;
            self.recovering_since = None;
        }
        if self.warning.is_none()
            && self.last_sample.is_some()
            && now.duration_since(self.last_audible) >= Duration::from_secs(20)
        {
            self.warning = Some(InputWarning::VeryQuiet);
        }
        self.warning
    }
}

/// Level of the newest 80 ms of captured mono PCM, not a speech or device-health verdict.
fn input_level(samples: &[f32]) -> f32 {
    let recent = &samples[samples.len().saturating_sub(1280)..];
    if recent.is_empty() {
        return 0.0;
    }
    let mean_square = recent.iter().map(|v| v * v).sum::<f32>() / recent.len() as f32;
    (mean_square.sqrt() * 4.0).clamp(0.0, 1.0)
}

#[derive(Clone, Debug)]
pub struct RecordedAudio {
    pub sample_count: usize,
    pub duration_ms: u64,
    pub wav_bytes: Vec<u8>,
    pub saved_lease: Option<Arc<super::persistence::Lease>>,
    /// The transcription that ran while recording, if one did (`live_transcribe`).
    pub live: crate::capture::engine::runtime::audio::transcription::live_transcribe::Slot,
    #[cfg(target_os = "linux")]
    pub pause: crate::capture::engine::runtime::audio::transcription::pause_transcribe::Slot,
}

pub struct AudioRecorder {
    worker: Option<std::thread::JoinHandle<Result<()>>>,
    stop_sender: Option<std::sync::mpsc::Sender<()>>,
    samples: Arc<Mutex<Samples>>,
    writer: Option<Writer>,
    error: Arc<Mutex<Option<String>>>,
    health: Arc<Mutex<InputHealth>>,
    live: Option<crate::capture::engine::runtime::audio::transcription::live_transcribe::Live>,
    word_probe: Option<crate::capture::engine::runtime::audio::transcription::live_word_probe::Probe>,
    #[cfg(target_os = "linux")]
    pause: Option<crate::capture::engine::runtime::audio::transcription::pause_transcribe::Pause>,
}

// The variant's stream is kept alive until the capture worker exits.
#[allow(dead_code)]
enum InputStream {
    Cpal(cpal::Stream),
    #[cfg(target_os = "linux")]
    Pulse(crate::capture::desktop::linux::microphone::Stream),
}

impl AudioRecorder {
    /// A read-only view of recorded samples. A repeated count means no new input arrived.
    #[cfg(target_os = "linux")]
    pub fn input_activity(&self) -> Option<(usize, f32)> {
        let samples = self.samples.lock().ok()?;
        Some((samples.len(), input_level(&samples)))
    }
    pub fn start(bundle_dir: &std::path::Path) -> Result<Self> {
        let started = std::time::Instant::now();
        let samples = Arc::new(Mutex::new(Samples::default()));
        let error = Arc::new(Mutex::new(None));
        let writer = Writer::start(bundle_dir, Arc::clone(&samples), Arc::clone(&error))?;
        let health = Arc::new(Mutex::new(InputHealth::new(started)));
        let (stop_sender, stop_receiver) = std::sync::mpsc::channel();
        let worker_samples = Arc::clone(&samples);
        let worker_error = Arc::clone(&error);
        let worker_health = Arc::clone(&health);
        let worker = std::thread::Builder::new()
            .name("apparent-audio".into())
            .spawn(move || {
                let stream =
                    Self::open_stream(worker_samples, worker_error, Arc::clone(&worker_health)).inspect_err(|error| {
                        if let Ok(mut health) = worker_health.lock() { health.open_failed(); }
                        crate::capture::engine::runtime::apparent_log(format!("Microphone startup failed: {error:#}"));
                    })?;
                crate::capture::desktop::start_metric("audio_ready_ms", started);
                // Stream never crosses threads. Disconnect also stops abandoned recorders.
                let _ = stop_receiver.recv();
                drop(stream);
                Ok(())
            })
            .context("Could not spawn microphone worker")?;
        let live = crate::capture::engine::runtime::audio::transcription::live_transcribe::wanted()
            .then(|| crate::capture::engine::runtime::audio::transcription::live_transcribe::Live::start(Arc::clone(&samples)))
            .and_then(|live| {
                live.inspect_err(|error| {
                    crate::capture::engine::runtime::apparent_log(format!("Live transcription: {error:#}"));
                })
                .ok()
            });
        let word_probe = crate::shared::handoff::experiment("liveWordWarning")
            .then(|| crate::capture::engine::runtime::audio::transcription::live_word_probe::Probe::start(Arc::clone(&samples)));
        #[cfg(target_os = "linux")]
        crate::capture::engine::runtime::audio::transcription::warm_recognizer::clear_if_disabled();
        #[cfg(target_os = "linux")]
        let pause = crate::capture::engine::runtime::audio::transcription::pause_transcribe::wanted()
            .then(|| crate::capture::engine::runtime::audio::transcription::pause_transcribe::Pause::start(Arc::clone(&samples)))
            .and_then(|result| {
                result.inspect_err(|error| crate::capture::engine::runtime::apparent_log(format!("Pause transcription: {error:#}"))).ok()
            });
        Ok(Self {
            worker: Some(worker),
            stop_sender: Some(stop_sender),
            samples,
            writer: Some(writer),
            error,
            health,
            live,
            word_probe,
            #[cfg(target_os = "linux")]
            pause,
        })
    }

    pub fn live_word_state(&self) -> Option<crate::capture::engine::runtime::audio::transcription::live_word_warning::State> {
        self.word_probe.as_ref().map(|probe| probe.state())
    }

    /// The chosen microphone (`mic`), else the system default. A chosen device that is missing or
    /// won't open never stops the recording.
    fn open_stream(
        samples: Arc<Mutex<Samples>>,
        error: Arc<Mutex<Option<String>>>,
        health: Arc<Mutex<InputHealth>>,
    ) -> Result<InputStream> {
        let choice = crate::capture::engine::runtime::audio::mic::choice();
        let inputs = crate::capture::engine::runtime::audio::mic::inputs();
        let pick = crate::capture::engine::runtime::audio::mic::Pick::new(choice.as_ref(), &inputs);
        if choice.is_some() {
            crate::capture::engine::runtime::audio::mic::note_pick(&pick);
        }
        if let crate::capture::engine::runtime::audio::mic::Pick::Chosen(input) = pick {
            #[cfg(target_os = "linux")]
            if let Some(source) = crate::capture::desktop::linux::microphone::source(&input.id) {
                match Self::open_pulse(source, samples.clone(), health.clone()) {
                    Ok(stream) => {
                        crate::capture::engine::runtime::apparent_log(format!("Microphone: {}", input.name));
                        return Ok(stream);
                    }
                    Err(e) => crate::capture::engine::runtime::apparent_log(format!(
                        "Microphone: could not open {} ({e:#}); using the default",
                        input.name
                    )),
                }
            }
            if let Some(device) = crate::capture::engine::runtime::audio::mic::device(&input.id) {
                match Self::open_device(&device, samples.clone(), error.clone(), health.clone()) {
                    Ok(stream) => {
                        crate::capture::engine::runtime::apparent_log(format!("Microphone: {}", input.name));
                        return Ok(InputStream::Cpal(stream));
                    }
                    Err(e) => crate::capture::engine::runtime::apparent_log(format!(
                        "Microphone: could not open {} ({e:#}); using the default",
                        input.name
                    )),
                }
            }
        }
        #[cfg(target_os = "linux")]
        if let Some(default) = crate::capture::desktop::linux::microphone::inputs()
            .and_then(|inputs| inputs.default_input().cloned())
        {
            let source = crate::capture::desktop::linux::microphone::source(&default.id)
                .expect("Pulse enumeration always returns Pulse source ids");
            match Self::open_pulse(source, samples.clone(), health.clone()) {
                Ok(stream) => {
                    crate::capture::engine::runtime::apparent_log(format!("Microphone: {} (default)", default.name));
                    return Ok(stream);
                }
                Err(e) => crate::capture::engine::runtime::apparent_log(format!(
                    "Microphone: could not open Pulse default {} ({e:#}); trying the system input",
                    default.name
                )),
            }
        }
        let device = cpal::default_host()
            .default_input_device()
            .context("No microphone is available")?;
        let stream = Self::open_device(&device, samples, error, health)?;
        crate::capture::engine::runtime::apparent_log(format!(
            "Microphone: {} (default)",
            device_name(&device).unwrap_or_default()
        ));
        Ok(InputStream::Cpal(stream))
    }

    #[cfg(target_os = "linux")]
    fn open_pulse(
        source: &str,
        samples: Arc<Mutex<Samples>>,
        health: Arc<Mutex<InputHealth>>,
    ) -> Result<InputStream> {
        let stream = crate::capture::desktop::linux::microphone::Stream::start(source, move |data| {
            if let Ok(mut output) = samples.lock() {
                output.extend_from_slice(data);
            }
            let peak = data.iter().map(|value| value.abs()).fold(0.0_f32, f32::max);
            if let Ok(mut health) = health.lock() {
                health.sample(Instant::now(), peak);
            }
        })?;
        Ok(InputStream::Pulse(stream))
    }

    fn open_device(
        device: &cpal::Device,
        samples: Arc<Mutex<Samples>>,
        error: Arc<Mutex<Option<String>>>,
        health: Arc<Mutex<InputHealth>>,
    ) -> Result<cpal::Stream> {
        let supported = device.default_input_config()?;
        let config = supported.config();
        crate::capture::engine::runtime::apparent_log(format!(
            "Microphone: {} Hz, {} ch, {:?}",
            config.sample_rate,
            config.channels,
            supported.sample_format()
        ));
        ensure!(
            config.channels > 0 && config.sample_rate > 0,
            "Invalid microphone format"
        );
        macro_rules! stream {
            ($ty:ty) => {
                input_stream::<$ty>(&device, config, samples.clone(), error.clone(), health.clone())?
            };
        }
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => stream!(f32),
            cpal::SampleFormat::F64 => stream!(f64),
            cpal::SampleFormat::I8 => stream!(i8),
            cpal::SampleFormat::I16 => stream!(i16),
            cpal::SampleFormat::I24 => stream!(cpal::I24),
            cpal::SampleFormat::I32 => stream!(i32),
            cpal::SampleFormat::I64 => stream!(i64),
            cpal::SampleFormat::U8 => stream!(u8),
            cpal::SampleFormat::U16 => stream!(u16),
            cpal::SampleFormat::U24 => stream!(cpal::U24),
            cpal::SampleFormat::U32 => stream!(u32),
            cpal::SampleFormat::U64 => stream!(u64),
            format => bail!("Unsupported microphone format: {format}"),
        };
        stream.play().context("Could not start microphone")?;
        Ok(stream)
    }

    pub fn warning(&self) -> Option<InputWarning> {
        self.health.lock().ok()?.status(Instant::now())
    }

    fn join_worker(&mut self) -> Result<()> {
        self.stop_sender.take();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| anyhow!("Microphone worker panicked"))??;
        }
        Ok(())
    }

    pub fn stop(mut self) -> Result<RecordedAudio> {
        // Joining drops the stream and stops its callback before taking the buffer,
        // including a stop requested while the device was still being initialized.
        let input_result = self.join_worker();
        let error = self
            .error
            .lock()
            .map_err(|_| anyhow!("Microphone error lock poisoned"))?
            .take();
        let (wav_bytes, saved_lease) = if let Some(mut writer) = self.writer.take() {
            let (bytes, lease) = writer.finish()?;
            (bytes, Some(lease))
        } else {
            (encode_wav_16k(self.samples.lock().map_err(|_| anyhow!("Microphone buffer lock poisoned"))?.recent())?, None)
        };
        let sample_count = wav_16k_reader(&wav_bytes)?.duration() as usize;
        // Check saving even if input failed. A save failure must not become successful silence.
        if let Err(error) = input_result {
            crate::capture::engine::runtime::apparent_log(format!("Microphone unavailable: {error:#}; retaining narration ownership through finalization"));
        }
        self.samples.lock().map_err(|_| anyhow!("Microphone buffer lock poisoned"))?.clear();
        // An overload, or a device unplugged partway, keeps what was heard before it.
        if let Some(error) = error {
            crate::capture::engine::runtime::apparent_log(format!("Microphone: {error}; keeping what it heard"));
        }
        let duration_ms = sample_count as u64 * 1_000 / SAMPLE_RATE as u64;
        Ok(RecordedAudio {
            sample_count,
            duration_ms,
            wav_bytes,
            saved_lease,
            live: crate::capture::engine::runtime::audio::transcription::live_transcribe::Slot::new(self.live.take()),
            #[cfg(target_os = "linux")]
            pause: crate::capture::engine::runtime::audio::transcription::pause_transcribe::Slot::new(self.pause.take()),
        })
    }
}

impl Drop for AudioRecorder {
    fn drop(&mut self) {
        let _ = self.join_worker();
    }
}

/// Mic permission/device failure does not invalidate a screen capture. Save failure does.
pub fn stop_optional(recorder: Option<AudioRecorder>) -> Result<Option<RecordedAudio>> {
    recorder.map(AudioRecorder::stop).transpose()
}

fn device_name(device: &cpal::Device) -> Option<String> {
    Some(device.description().ok()?.name().to_owned())
}

fn input_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    samples: Arc<Mutex<Samples>>,
    error: Arc<Mutex<Option<String>>>,
    health: Arc<Mutex<InputHealth>>,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let channels = usize::from(config.channels);
    let rate = config.sample_rate;
    let mut phase = 0_u32;
    let mut sum = 0.0_f64;
    let callback_health = Arc::clone(&health);
    Ok(device.build_input_stream::<T, _, _>(
        config,
        move |data, _| {
            let Ok(mut output) = samples.lock() else {
                return;
            };
            let before = output.len();
            let mut peak = 0.0_f32;
            for frame in data.chunks_exact(channels) {
                let mono = frame
                    .iter()
                    .map(|sample| sample.to_sample::<f32>() as f64)
                    .sum::<f64>()
                    / channels as f64;
                let mono = if mono.is_finite() { mono } else { 0.0 };
                // Area averaging gives a continuous-phase downmix/decimator,
                // including non-integer ratios such as 44.1 kHz -> 16 kHz.
                let mut remaining = SAMPLE_RATE;
                while remaining > 0 {
                    let weight = remaining.min(rate - phase);
                    sum += mono * weight as f64;
                    phase += weight;
                    remaining -= weight;
                    if phase == rate {
                        let value = (sum / rate as f64) as f32;
                        peak = peak.max(value.abs());
                        output.push(value);
                        phase = 0;
                        sum = 0.0;
                    }
                }
            }
            if output.len() > before {
                if let Ok(mut health) = callback_health.lock() {
                    health.sample(Instant::now(), peak);
                }
            }
        },
        move |err| {
            // A buffer under- or overrun drops a few samples; the recording goes on.
            if err.kind() == cpal::ErrorKind::Xrun {
                return;
            }
            if let Ok(mut error) = error.lock() {
                *error = Some(err.to_string());
            }
            if let Ok(mut health) = health.lock() {
                health.stream_failed();
            }
        },
        None,
    )?)
}

/// Reads a 16 kHz mono WAV (as written by `apparent-helper audio`) back into a recording.
pub fn recorded_audio_from_wav(wav_bytes: Vec<u8>) -> Result<RecordedAudio> {
    let sample_count = wav_16k_reader(&wav_bytes)?.duration() as usize;
    Ok(RecordedAudio {
        sample_count,
        duration_ms: sample_count as u64 * 1_000 / u64::from(SAMPLE_RATE),
        wav_bytes,
        saved_lease: None,
        live: Default::default(),
        #[cfg(target_os = "linux")]
        pause: Default::default(),
    })
}

/// Decodes a 16 kHz mono 16-bit WAV (this module's own format) into samples.
pub fn decode_wav_16k(wav_bytes: &[u8]) -> Result<Vec<f32>> {
    wav_16k_reader(wav_bytes)?
        .into_samples::<i16>()
        .map(|sample| Ok(f32::from(sample?) / 32_768.0))
        .collect()
}

/// Decodes a 16 kHz mono 16-bit WAV into its raw samples.
pub fn decode_wav_pcm16(wav_bytes: &[u8]) -> Result<Vec<i16>> {
    Ok(wav_16k_reader(wav_bytes)?
        .into_samples::<i16>()
        .collect::<Result<_, _>>()?)
}

fn wav_16k_reader(wav_bytes: &[u8]) -> Result<hound::WavReader<Cursor<&[u8]>>> {
    let reader = hound::WavReader::new(Cursor::new(wav_bytes))?;
    let spec = reader.spec();
    ensure!(
        spec.sample_rate == SAMPLE_RATE
            && spec.channels == 1
            && spec.bits_per_sample == 16
            && spec.sample_format == hound::SampleFormat::Int,
        "Expected 16 kHz mono 16-bit audio"
    );
    Ok(reader)
}

const ISLAND_WINDOW_MS: u64 = 50;
const ISLAND_MERGE_SILENCE_MS: u64 = 350;

/// Energy-based speech spans in milliseconds, quantized to 50 ms. These are not recognizer
/// timestamps. The speech threshold follows the recording's own noise floor (10th percentile of
/// window energy) so a quiet microphone and a noisy room both find their speech.
pub fn speech_islands(samples_16k: &[f32]) -> Vec<(u64, u64)> {
    let window = (u64::from(SAMPLE_RATE) * ISLAND_WINDOW_MS / 1_000) as usize;
    let rms: Vec<f64> = samples_16k
        .chunks(window)
        .map(|chunk| {
            (chunk
                .iter()
                .map(|&x| {
                    if x.is_finite() {
                        f64::from(x).powi(2)
                    } else {
                        0.0
                    }
                })
                .sum::<f64>()
                / chunk.len() as f64)
                .sqrt()
        })
        .collect();
    if rms.is_empty() {
        return Vec::new();
    }
    let mut sorted = rms.clone();
    sorted.sort_by(f64::total_cmp);
    let floor = sorted[sorted.len() / 10];
    let threshold = (floor * 3.0).clamp(0.005, 0.02);
    let mut islands: Vec<(u64, u64)> = Vec::new();
    for (index, &level) in rms.iter().enumerate() {
        if level <= threshold {
            continue;
        }
        let start = index as u64 * ISLAND_WINDOW_MS;
        let end = start + ISLAND_WINDOW_MS;
        match islands.last_mut() {
            Some(last) if start.saturating_sub(last.1) < ISLAND_MERGE_SILENCE_MS => last.1 = end,
            _ => islands.push((start, end)),
        }
    }
    let duration = samples_16k.len() as u64 * 1_000 / u64::from(SAMPLE_RATE);
    for island in &mut islands {
        island.1 = island.1.min(duration);
    }
    islands.retain(|&(a, b)| b > a);
    islands
}

/// One sample as `audio.wav` stores it.
pub fn pcm16(sample: f32) -> i16 {
    let sample = if sample.is_finite() {
        sample.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    (sample * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16
}

pub fn encode_wav_16k(samples_16k: &[f32]) -> Result<Vec<u8>> {
    let mut bytes = Cursor::new(Vec::with_capacity(44 + samples_16k.len() * 2));
    {
        let mut writer = hound::WavWriter::new(
            &mut bytes,
            hound::WavSpec {
                channels: 1,
                sample_rate: SAMPLE_RATE,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )?;
        let mut i16_writer = writer.get_i16_writer(samples_16k.len() as u32);
        for &sample in samples_16k {
            i16_writer.write_sample(pcm16(sample));
        }
        i16_writer.flush()?;
        writer.finalize()?;
    }
    Ok(bytes.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn cpal_choice_becomes_missing_when_pulse_inventory_takes_over() {
        use crate::capture::engine::runtime::audio::mic::{Choice, Input, Inputs, Pick};

        let saved = Choice {
            id: "cpal-mic".into(),
            name: "Original microphone".into(),
        };
        let cpal = Inputs {
            default: Some("cpal-mic".into()),
            devices: vec![Input {
                id: "cpal-mic".into(),
                name: saved.name.clone(),
            }],
        };
        assert!(matches!(Pick::new(Some(&saved), &cpal), Pick::Chosen(_)));

        let pulse = Inputs {
            default: Some("pulse:system".into()),
            devices: vec![Input {
                id: "pulse:system".into(),
                name: "System input".into(),
            }],
        };
        let pick = Pick::new(Some(&saved), &pulse);
        assert!(matches!(pick, Pick::Missing { .. }));
        assert_eq!(pick.device().unwrap().id, "pulse:system");
        assert_eq!(pick.note().unwrap(), "Original microphone isn't connected. Using System input.");
    }

    #[test]
    fn input_level_responds_to_real_samples_not_silence_or_an_old_peak() {
        assert_eq!(input_level(&[]), 0.0);
        assert_eq!(input_level(&vec![0.0; 1280]), 0.0);
        let mut samples = vec![0.9; 1280];
        samples.extend(vec![0.0; 1280]);
        assert_eq!(input_level(&samples), 0.0, "old sound must leave the window");
        assert!(input_level(&vec![0.2; 1280]) > 0.0);
    }
    use std::time::Duration;

    #[test]
    fn input_health_distinguishes_open_failure_stall_and_sustained_silence() {
        let start = std::time::Instant::now();
        let mut h = InputHealth::new(start);
        assert_eq!(h.status(start + Duration::from_secs(2)), None);
        assert_eq!(h.status(start + Duration::from_secs(5)), Some(InputWarning::Stopped));
        h.open_failed();
        assert_eq!(h.status(start + Duration::from_secs(6)), Some(InputWarning::Unavailable));

        let mut h = InputHealth::new(start);
        h.sample(start + Duration::from_secs(1), 0.0);
        assert_eq!(h.status(start + Duration::from_secs(3)), None);
        h.sample(start + Duration::from_secs(4), 0.0);
        assert_eq!(h.status(start + Duration::from_secs(6)), None);
        h.sample(start + Duration::from_secs(21), 0.0);
        assert_eq!(h.status(start + Duration::from_secs(21)), Some(InputWarning::VeryQuiet));
    }

    #[test]
    fn input_health_recovers_without_flicker_or_false_pause_alarm() {
        let start = std::time::Instant::now();
        let mut h = InputHealth::new(start);
        for second in 1..=10 {
            h.sample(start + Duration::from_secs(second), 0.2);
            assert_eq!(h.status(start + Duration::from_secs(second)), None);
        }
        assert_eq!(h.status(start + Duration::from_secs(12)), None);
        assert_eq!(h.status(start + Duration::from_secs(14)), Some(InputWarning::Stopped));
        h.sample(start + Duration::from_secs(15), 0.1);
        assert_eq!(h.status(start + Duration::from_secs(15)), Some(InputWarning::Stopped));
        h.sample(start + Duration::from_secs(16), 0.1);
        assert_eq!(h.status(start + Duration::from_secs(16)), None);
        h.sample(start + Duration::from_secs(17), 0.0);
        assert_eq!(h.status(start + Duration::from_secs(19)), None);
        h.stream_failed();
        assert_eq!(h.status(start + Duration::from_secs(19)), Some(InputWarning::Stopped));

        let mut h = InputHealth::new(start);
        h.sample(start + Duration::from_secs(21), 0.0);
        assert_eq!(h.status(start + Duration::from_secs(21)), Some(InputWarning::VeryQuiet));
        h.sample(start + Duration::from_secs(22), 0.1);
        h.sample(start + Duration::from_millis(22_500), 0.0);
        h.sample(start + Duration::from_secs(23), 0.1);
        assert_eq!(h.status(start + Duration::from_secs(23)), Some(InputWarning::VeryQuiet));
        h.sample(start + Duration::from_secs(24), 0.1);
        assert_eq!(h.status(start + Duration::from_secs(24)), None);
    }

    #[test]
    fn stop_joins_worker_before_reading_its_last_samples() {
        let samples = Arc::new(Mutex::new(Samples::default()));
        let output = Arc::clone(&samples);
        let (stop_sender, stop_receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            // Model a final callback flushed while the worker drops its stream.
            assert!(matches!(
                stop_receiver.recv_timeout(Duration::from_secs(2)),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
            ));
            output.lock().unwrap().extend([0.1, 0.2, 0.3]);
            Ok(())
        });
        let audio = AudioRecorder {
            worker: Some(worker),
            stop_sender: Some(stop_sender),
            samples,
            writer: None,
            error: Arc::new(Mutex::new(None)),
            health: Arc::new(Mutex::new(InputHealth::new(Instant::now()))),
            live: None,
            word_probe: None,
            #[cfg(target_os = "linux")]
            pause: None,
        }
        .stop()
        .unwrap();
        assert_eq!(audio.sample_count, 3);
        assert_eq!(
            hound::WavReader::new(Cursor::new(audio.wav_bytes))
                .unwrap()
                .duration(),
            3
        );
    }

    #[test]
    fn a_device_lost_partway_keeps_what_it_heard() {
        let recorder = |samples: Vec<f32>| AudioRecorder {
            worker: None,
            stop_sender: None,
            samples: Arc::new(Mutex::new(samples.into())),
            writer: None,
            error: Arc::new(Mutex::new(Some("Device disconnected".into()))),
            health: Arc::new(Mutex::new(InputHealth::new(Instant::now()))),
            live: None,
            word_probe: None,
            #[cfg(target_os = "linux")]
            pause: None,
        };
        assert_eq!(recorder(vec![0.1; 160]).stop().unwrap().sample_count, 160);
        assert_eq!(recorder(Vec::new()).stop().unwrap().sample_count, 0);
    }

    #[test]
    fn unavailable_input_keeps_its_lease_through_screen_finalization() {
        let dir = std::env::temp_dir().join(format!("apparent-no-mic-{}", crate::shared::ids::make_bundle_id()));
        crate::capture::engine::bundle::recover::begin_screen_capture(&dir).unwrap();
        let samples = Arc::new(Mutex::new(Samples::default()));
        let error = Arc::new(Mutex::new(None));
        let writer = Writer::start(&dir, samples.clone(), error.clone()).unwrap();
        let recorder = AudioRecorder {
            worker: Some(std::thread::spawn(|| bail!("device unavailable"))),
            stop_sender: None,
            samples,
            writer: Some(writer),
            error,
            health: Arc::new(Mutex::new(InputHealth::new(Instant::now()))),
            live: None,
            word_probe: None,
            #[cfg(target_os = "linux")]
            pause: None,
        };
        let audio = recorder.stop().unwrap();
        assert_eq!(audio.sample_count, 0, "unavailable input must not claim recorded narration");
        assert!(super::super::persistence::Lease::acquire(&dir).is_err());
        assert!(!crate::capture::engine::bundle::recover::owed(&dir));
        drop(audio);
        assert!(super::super::persistence::Lease::acquire(&dir).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn speech_islands_merge_only_short_silences() {
        let mut samples = vec![0.0; 32_000];
        samples[1_600..4_800].fill(0.2); // 100..300
        samples[8_800..12_800].fill(0.2); // 550..800: merge 250 ms gap
        samples[18_400..20_800].fill(0.2); // 1150..1300: keep 350 ms gap
        assert_eq!(speech_islands(&samples), vec![(100, 800), (1_150, 1_300)]);
        assert!(speech_islands(&[0.001; 800]).is_empty());
        assert!(speech_islands(&[]).is_empty());
    }

    #[test]
    fn speech_islands_adapt_to_a_noisy_floor() {
        let mut samples: Vec<f32> = (0..32_000)
            .map(|i| if i % 2 == 0 { 0.006 } else { -0.006 })
            .collect();
        samples[8_000..16_000]
            .iter_mut()
            .for_each(|s| *s = if *s > 0.0 { 0.15 } else { -0.15 });
        assert_eq!(speech_islands(&samples), vec![(500, 1_000)]);
    }

    #[test]
    fn decoded_wav_round_trips() {
        let wav = encode_wav_16k(&[0.0, 0.5, -0.5]).unwrap();
        assert_eq!(decode_wav_16k(&wav).unwrap(), vec![0.0, 0.5, -0.5]);
        assert!(decode_wav_16k(b"nope").is_err());
    }

    #[test]
    fn wav_is_mono_16k_pcm_and_clips_safely() {
        let bytes = encode_wav_16k(&[-2.0, -0.5, 0.0, 0.5, 2.0, f32::NAN]).unwrap();
        let mut reader = hound::WavReader::new(Cursor::new(bytes)).unwrap();
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(reader.spec().bits_per_sample, 16);
        assert_eq!(reader.spec().sample_format, hound::SampleFormat::Int);
        assert_eq!(
            reader
                .samples::<i16>()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap(),
            vec![-32_768, -16_384, 0, 16_384, 32_767, 0]
        );
    }
}
