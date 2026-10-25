use std::io::Cursor;
use std::sync::{Arc, Mutex};

#[cfg(feature = "mic")]
use anyhow::Context;
use anyhow::{Result, anyhow, bail, ensure};
#[cfg(feature = "mic")]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const SAMPLE_RATE: u32 = 16_000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GapSpan {
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Mono audio on the capture's clock. `samples[0]` is `start_offset_ms` after the capture
/// started; spans in `gaps` (also from the capture's start) are silence where audio was
/// missing or dropped.
#[derive(Clone, Debug, Default)]
pub struct RecordedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub start_offset_ms: u64,
    pub gaps: Vec<GapSpan>,
    pub(crate) sample_count: usize,
    pub(crate) duration_ms: u64,
    pub(crate) wav_bytes: Vec<u8>,
    pub(crate) live: crate::live_transcribe::Slot,
}

impl RecordedAudio {
    pub fn new(
        samples: Vec<f32>,
        sample_rate: u32,
        start_offset_ms: u64,
        gaps: Vec<GapSpan>,
    ) -> Self {
        let rate = if sample_rate == 0 {
            SAMPLE_RATE
        } else {
            sample_rate
        };
        let sample_count = samples.len();
        let duration_ms = start_offset_ms + (sample_count as u64 * 1_000) / u64::from(rate);
        Self {
            samples,
            sample_rate: rate,
            start_offset_ms,
            gaps,
            sample_count,
            duration_ms,
            wav_bytes: Vec::new(),
            live: Default::default(),
        }
    }

    pub fn sample_count(&self) -> usize {
        if !self.samples.is_empty() {
            self.samples.len()
        } else {
            self.sample_count
        }
    }

    pub fn duration_ms(&self) -> u64 {
        if self.duration_ms > 0 {
            self.duration_ms
        } else if !self.samples.is_empty() {
            let rate = if self.sample_rate == 0 {
                SAMPLE_RATE
            } else {
                self.sample_rate
            };
            self.start_offset_ms + (self.samples.len() as u64 * 1_000) / u64::from(rate)
        } else if self.sample_count > 0 {
            self.start_offset_ms + (self.sample_count as u64 * 1_000) / u64::from(SAMPLE_RATE)
        } else {
            self.start_offset_ms
        }
    }

    pub fn total_duration_ms(&self) -> u64 {
        self.duration_ms()
    }

    pub fn samples_16k(&self) -> Result<std::borrow::Cow<'_, [f32]>> {
        if !self.samples.is_empty() {
            if self.sample_rate == 0 || self.sample_rate == SAMPLE_RATE {
                return Ok(std::borrow::Cow::Borrowed(&self.samples));
            }
            return Ok(std::borrow::Cow::Owned(resample_to_16k(
                &self.samples,
                self.sample_rate,
            )));
        }
        if !self.wav_bytes.is_empty() {
            return Ok(std::borrow::Cow::Owned(decode_wav_16k(&self.wav_bytes)?));
        }
        Ok(std::borrow::Cow::Borrowed(&[]))
    }

    /// Encodes 16 kHz mono PCM16 WAV bytes. When `start_offset_ms > 0`, leading silence is
    /// prepended so `audio.wav` on disk starts at `0 ms` of the capture clock.
    pub fn wav_bytes_16k(&self) -> Result<std::borrow::Cow<'_, [u8]>> {
        if !self.wav_bytes.is_empty() && self.start_offset_ms == 0 {
            return Ok(std::borrow::Cow::Borrowed(&self.wav_bytes));
        }
        let samples = self.samples_16k()?;
        let pad = (self.start_offset_ms * u64::from(SAMPLE_RATE) / 1_000) as usize;
        if pad == 0 {
            return Ok(std::borrow::Cow::Owned(encode_wav_16k(&samples)?));
        }
        let mut padded = vec![0.0_f32; pad + samples.len()];
        padded[pad..].copy_from_slice(&samples);
        Ok(std::borrow::Cow::Owned(encode_wav_16k(&padded)?))
    }

    pub fn set_live(&self, live: Option<crate::live_transcribe::Live>) {
        self.live.set(live);
    }

    pub fn take_live(&self) -> Option<crate::live_transcribe::Live> {
        self.live.take()
    }
}

fn resample_to_16k(samples: &[f32], rate: u32) -> Vec<f32> {
    if rate == 0 || rate == SAMPLE_RATE {
        return samples.to_vec();
    }
    let mut output = Vec::with_capacity(
        (samples.len() as u64 * u64::from(SAMPLE_RATE) / u64::from(rate)) as usize + 1,
    );
    let mut phase = 0_u32;
    let mut sum = 0.0_f64;
    for &sample in samples {
        let mono = if sample.is_finite() {
            f64::from(sample)
        } else {
            0.0
        };
        let mut remaining = SAMPLE_RATE;
        while remaining > 0 {
            let weight = remaining.min(rate - phase);
            sum += mono * f64::from(weight);
            phase += weight;
            remaining -= weight;
            if phase == rate {
                output.push((sum / f64::from(rate)) as f32);
                phase = 0;
                sum = 0.0;
            }
        }
    }
    output
}

pub struct AudioRecorder {
    worker: Option<std::thread::JoinHandle<Result<()>>>,
    stop_sender: Option<std::sync::mpsc::Sender<()>>,
    samples: Arc<Mutex<Vec<f32>>>,
    error: Arc<Mutex<Option<String>>>,
    live: Option<crate::live_transcribe::Live>,
}

impl AudioRecorder {
    #[cfg(not(feature = "mic"))]
    pub fn start() -> Result<Self> {
        bail!("Built-in microphone recording requires the `mic` feature")
    }

    #[cfg(feature = "mic")]
    pub fn start() -> Result<Self> {
        let started = std::time::Instant::now();
        let samples = Arc::new(Mutex::new(Vec::new()));
        let error = Arc::new(Mutex::new(None));
        let (stop_sender, stop_receiver) = std::sync::mpsc::channel();
        let worker_samples = Arc::clone(&samples);
        let worker_error = Arc::clone(&error);
        let worker = std::thread::Builder::new()
            .name("scrawl-audio".into())
            .spawn(move || {
                let stream =
                    Self::open_stream(worker_samples, worker_error).inspect_err(|error| {
                        crate::log::scrawl_log(format!("Microphone startup failed: {error:#}"));
                    })?;
                crate::log::start_metric("audio_ready_ms", started);
                // Stream never crosses threads. Disconnect also stops abandoned recorders.
                let _ = stop_receiver.recv();
                drop(stream);
                Ok(())
            })
            .context("Could not spawn microphone worker")?;
        let live = crate::live_transcribe::wanted()
            .then(|| crate::live_transcribe::Live::start(Arc::clone(&samples)))
            .and_then(|live| {
                live.inspect_err(|error| {
                    crate::log::scrawl_log(format!("Live transcription: {error:#}"));
                })
                .ok()
            });
        Ok(Self {
            worker: Some(worker),
            stop_sender: Some(stop_sender),
            samples,
            error,
            live,
        })
    }

    /// The chosen microphone (`mic`), else the macOS default. A chosen device that is missing or
    /// won't open never stops the recording.
    #[cfg(feature = "mic")]
    fn open_stream(
        samples: Arc<Mutex<Vec<f32>>>,
        error: Arc<Mutex<Option<String>>>,
    ) -> Result<cpal::Stream> {
        let choice = crate::mic::choice();
        if let Some(choice) = &choice {
            if let Some(device) = crate::mic::device(&choice.id) {
                let input = crate::mic::Input {
                    id: choice.id.clone(),
                    name: device_name(&device).unwrap_or_else(|| choice.name.clone()),
                };
                crate::mic::note_pick(&crate::mic::Pick::Chosen(&input));
                match Self::open_device(&device, samples.clone(), error.clone()) {
                    Ok(stream) => {
                        crate::log::scrawl_log(format!("Microphone: {}", input.name));
                        return Ok(stream);
                    }
                    Err(e) => crate::log::scrawl_log(format!(
                        "Microphone: could not open {} ({e:#}); using the default",
                        input.name
                    )),
                }
            } else {
                let default = cpal::default_host()
                    .default_input_device()
                    .and_then(|device| {
                        Some(crate::mic::Input {
                            id: device.id().ok()?.id().to_owned(),
                            name: device_name(&device)?,
                        })
                    });
                let inputs = crate::mic::Inputs {
                    default: default.as_ref().map(|d| d.id.clone()),
                    devices: default.into_iter().collect(),
                };
                crate::mic::note_pick(&crate::mic::Pick::new(Some(choice), &inputs));
            }
        }
        let device = cpal::default_host()
            .default_input_device()
            .context("No microphone is available")?;
        let stream = Self::open_device(&device, samples, error)?;
        crate::log::scrawl_log(format!(
            "Microphone: {} (default)",
            device_name(&device).unwrap_or_default()
        ));
        Ok(stream)
    }

    #[cfg(feature = "mic")]
    fn open_device(
        device: &cpal::Device,
        samples: Arc<Mutex<Vec<f32>>>,
        error: Arc<Mutex<Option<String>>>,
    ) -> Result<cpal::Stream> {
        let supported = device.default_input_config()?;
        let config = supported.config();
        crate::log::scrawl_log(format!(
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
                input_stream::<$ty>(&device, config, samples.clone(), error.clone())?
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
        self.join_worker()?;
        let error = self
            .error
            .lock()
            .map_err(|_| anyhow!("Microphone error lock poisoned"))?
            .take();
        let samples_16k = std::mem::take(
            &mut *self
                .samples
                .lock()
                .map_err(|_| anyhow!("Microphone buffer lock poisoned"))?,
        );
        // An overload, or a device unplugged partway, keeps what was heard before it.
        if let Some(error) = error {
            if samples_16k.is_empty() {
                bail!("Microphone capture failed: {error}");
            }
            crate::log::scrawl_log(format!("Microphone: {error}; keeping what it heard"));
        }
        let sample_count = samples_16k.len();
        let duration_ms = sample_count as u64 * 1_000 / SAMPLE_RATE as u64;
        let wav_bytes = encode_wav_16k(&samples_16k)?;
        Ok(RecordedAudio {
            samples: Vec::new(),
            sample_rate: SAMPLE_RATE,
            start_offset_ms: 0,
            gaps: Vec::new(),
            sample_count,
            duration_ms,
            wav_bytes,
            live: crate::live_transcribe::Slot::new(self.live.take()),
        })
    }
}

impl Drop for AudioRecorder {
    fn drop(&mut self) {
        let _ = self.join_worker();
    }
}

#[cfg(feature = "mic")]
fn device_name(device: &cpal::Device) -> Option<String> {
    Some(device.description().ok()?.name().to_owned())
}

#[cfg(feature = "mic")]
fn input_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    samples: Arc<Mutex<Vec<f32>>>,
    error: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let channels = usize::from(config.channels);
    let rate = config.sample_rate;
    let mut phase = 0_u32;
    let mut sum = 0.0_f64;
    Ok(device.build_input_stream::<T, _, _>(
        config,
        move |data, _| {
            let Ok(mut output) = samples.lock() else {
                return;
            };
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
                        output.push((sum / rate as f64) as f32);
                        phase = 0;
                        sum = 0.0;
                    }
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
        },
        None,
    )?)
}

/// Reads a 16 kHz mono WAV (as written by `scrawl-helper audio`) back into a recording.
pub fn recorded_audio_from_wav(wav_bytes: Vec<u8>) -> Result<RecordedAudio> {
    let sample_count = wav_16k_reader(&wav_bytes)?.duration() as usize;
    Ok(RecordedAudio {
        samples: Vec::new(),
        sample_rate: SAMPLE_RATE,
        start_offset_ms: 0,
        gaps: Vec::new(),
        sample_count,
        duration_ms: sample_count as u64 * 1_000 / u64::from(SAMPLE_RATE),
        wav_bytes,
        live: Default::default(),
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
    use std::time::Duration;

    #[test]
    fn stop_joins_worker_before_reading_its_last_samples() {
        let samples = Arc::new(Mutex::new(Vec::new()));
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
            error: Arc::new(Mutex::new(None)),
            live: None,
        }
        .stop()
        .unwrap();
        assert_eq!(audio.sample_count(), 3);
        let wav = audio.wav_bytes_16k().unwrap();
        assert_eq!(
            hound::WavReader::new(Cursor::new(&*wav))
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
            samples: Arc::new(Mutex::new(samples)),
            error: Arc::new(Mutex::new(Some("Device disconnected".into()))),
            live: None,
        };
        assert_eq!(recorder(vec![0.1; 160]).stop().unwrap().sample_count(), 160);
        assert!(recorder(Vec::new()).stop().is_err());
    }

    #[test]
    fn stop_propagates_background_initialization_failure() {
        let recorder = AudioRecorder {
            worker: Some(std::thread::spawn(|| bail!("device unavailable"))),
            stop_sender: None,
            samples: Arc::new(Mutex::new(Vec::new())),
            error: Arc::new(Mutex::new(None)),
            live: None,
        };
        assert!(
            recorder
                .stop()
                .unwrap_err()
                .to_string()
                .contains("device unavailable")
        );
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

    #[test]
    fn wav_bytes_16k_pads_leading_silence_for_start_offset() {
        let audio = RecordedAudio::new(vec![0.5; 160], 16_000, 100, Vec::new());
        let wav = audio.wav_bytes_16k().unwrap();
        let decoded = decode_wav_16k(&wav).unwrap();
        // 100 ms of leading silence (1_600 samples) + 160 samples = 1_760 samples.
        assert_eq!(decoded.len(), 1_760);
        assert!(decoded[..1_600].iter().all(|&s| s == 0.0));
        assert!(decoded[1_600..].iter().all(|&s| (s - 0.5).abs() < 0.01));
    }
}
