//! Transcribing while recording (`"experiments": {"liveTranscribe": true}`, Apple Speech only).
//!
//! A long-lived `scrawl-helper transcribe-stream` gets the microphone's 16 kHz samples, quantized
//! exactly as `audio.wav` will store them, about every 160 ms, and keeps SpeechAnalyzer's
//! finalized results. At finish the samples it hasn't had are sent from the finished WAV and only
//! that tail is left to transcribe, so the words and times are the WAV's own. Anything wrong (the
//! helper fails, the model isn't installed, it lost samples or took too long) returns an error and
//! the caller transcribes the WAV as before.
//!
//! The same stream shows the words as they're said (`live_audio`, `"liveAudio": true`): given a
//! `Caption`, the helper also reports its guesses at the words in progress (`--volatile`), and
//! the reader keeps the caption up to date. With the experiment off, a `preview` stream does only
//! that, asks for quicker guesses (`--fast`, which may cost the finalized words some accuracy),
//! and is dropped when the recording stops.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::Deserialize;

use crate::live_audio::Caption;
use crate::model::{TranscriptData, TranscriptSegment, WordTiming};

const FEED_EVERY: Duration = Duration::from_millis(160);
/// How long finishing waits for the tail before giving up on the stream. The analyzer keeps up
/// with speech in real time, so the tail normally takes well under 100 ms; a model still loading
/// when a short recording ends takes longer, as it would after the recording too.
const TAIL_TIMEOUT: Duration = Duration::from_secs(6);

/// Whether recordings should start a live transcription now. It streams to `scrawl-helper`'s
/// SpeechAnalyzer, so only on macOS (Windows transcribes after the recording).
pub fn wanted() -> bool {
    cfg!(target_os = "macos")
        && crate::handoff::experiment(crate::handoff::LIVE_TRANSCRIBE)
        && crate::transcribe::Engine::preferred() == crate::transcribe::Engine::Apple
}

pub struct Live {
    stop: Arc<AtomicBool>,
    feeder: Option<JoinHandle<Result<Feeding>>>,
}

struct Feeding {
    child: Child,
    stdin: Option<ChildStdin>,
    fed: usize,
    lines: Receiver<Line>,
}

impl Drop for Feeding {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Event {
    Done {
        #[allow(dead_code)]
        done: bool,
        samples: usize,
    },
    Ready {
        #[allow(dead_code)]
        ready: bool,
        locale: String,
    },
    Result {
        segment: TranscriptSegment,
        #[serde(default)]
        words: Vec<WordTiming>,
    },
    /// A guess at the words still being said (`--volatile`), for showing only.
    Volatile { volatile: TranscriptSegment },
}

enum Line {
    Event(Event),
    /// stdout closed; the helper's stderr.
    Closed(String),
}

/// What finishing took, for the log.
#[derive(Debug)]
pub struct Finished {
    pub transcript: TranscriptData,
    pub tail_ms: u64,
    pub streamed_samples: usize,
}

impl Live {
    /// Starts streaming `samples` (the recorder's 16 kHz buffer, which only grows until the
    /// recording stops) to a new helper, keeping `caption` up to date with the words when given.
    /// Nothing here blocks: the helper starts on the feeder thread.
    pub fn start(
        samples: Arc<Mutex<Vec<f32>>>,
        caption: Option<Arc<Mutex<Caption>>>,
    ) -> Result<Self> {
        let mut command = Command::new(crate::helper::path()?);
        command.arg("transcribe-stream");
        command.args(crate::speech_model::locale_args());
        if caption.is_some() {
            command.arg("--volatile");
        }
        Self::start_with(command, samples, caption)
    }

    /// Streams `samples` only to show the words in `caption`, guessing at them about every second
    /// (`--fast`); never `finish`ed, just dropped when the recording stops.
    pub fn preview(samples: Arc<Mutex<Vec<f32>>>, caption: Arc<Mutex<Caption>>) -> Result<Self> {
        let mut command = Command::new(crate::helper::path()?);
        command.args(["transcribe-stream", "--volatile", "--fast"]);
        command.args(crate::speech_model::locale_args());
        Self::start_with(command, samples, Some(caption))
    }

    fn start_with(
        mut command: Command,
        samples: Arc<Mutex<Vec<f32>>>,
        caption: Option<Arc<Mutex<Caption>>>,
    ) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let feeder_stop = Arc::clone(&stop);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let feeder = std::thread::Builder::new()
            .name("scrawl-live-transcribe".into())
            .spawn(move || feed(command, &samples, &feeder_stop, caption))
            .context("Could not spawn the live transcription thread")?;
        Ok(Self {
            stop,
            feeder: Some(feeder),
        })
    }

    /// Sends what the helper hasn't had of `wav_bytes` (the recording's `audio.wav`), ends its
    /// input and collects the transcript.
    pub fn finish(mut self, wav_bytes: &[u8]) -> Result<Finished> {
        self.finish_within(wav_bytes, TAIL_TIMEOUT)
    }

    fn finish_within(&mut self, wav_bytes: &[u8], timeout: Duration) -> Result<Finished> {
        let finishing = Instant::now();
        let mut feeding = self.join_feeder()?;
        let samples = crate::audio::decode_wav_pcm16(wav_bytes)?;
        ensure!(
            feeding.fed <= samples.len(),
            "streamed {} samples but the recording has {}",
            feeding.fed,
            samples.len()
        );
        let streamed = feeding.fed;
        {
            let mut stdin = feeding.stdin.take().context("No helper stdin")?;
            stdin
                .write_all(&le_bytes(&samples[feeding.fed..]))
                .context("The live transcriber stopped")?;
        }
        let deadline = finishing + timeout;
        let mut locale = None;
        let mut segments = Vec::new();
        let mut words = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match feeding.lines.recv_timeout(left) {
                Ok(Line::Event(Event::Ready { locale: l, .. })) => locale = Some(l),
                Ok(Line::Event(Event::Volatile { .. })) => {}
                Ok(Line::Event(Event::Result {
                    segment,
                    words: more,
                })) => {
                    segments.push(segment);
                    words.extend(more);
                }
                Ok(Line::Event(Event::Done { samples: heard, .. })) => {
                    ensure!(
                        heard == samples.len(),
                        "the live transcriber heard {heard} samples of {}",
                        samples.len()
                    );
                    break;
                }
                Ok(Line::Closed(stderr)) => bail!(
                    "the live transcriber stopped: {}",
                    stderr.trim().trim_start_matches("scrawl-helper: ")
                ),
                Err(RecvTimeoutError::Timeout) => {
                    bail!(
                        "the live transcriber took over {} s to finish",
                        timeout.as_secs()
                    )
                }
                Err(RecvTimeoutError::Disconnected) => bail!("the live transcriber vanished"),
            }
        }
        let locale = locale.context("the live transcriber never said it was ready")?;
        Ok(Finished {
            transcript: transcript(&locale, segments, words),
            tail_ms: finishing.elapsed().as_millis() as u64,
            streamed_samples: streamed,
        })
    }

    fn join_feeder(&mut self) -> Result<Feeding> {
        self.stop.store(true, Ordering::Release);
        let feeder = self.feeder.take().context("Already finished")?;
        feeder.thread().unpark();
        feeder
            .join()
            .map_err(|_| anyhow!("The live transcription thread panicked"))?
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        if self.feeder.is_some() {
            let _ = self.join_feeder();
        }
    }
}

/// As `helper::transcribe` builds it from the batch helper's output.
fn transcript(
    locale: &str,
    mut segments: Vec<TranscriptSegment>,
    mut words: Vec<WordTiming>,
) -> TranscriptData {
    segments.sort_by_key(|s| s.t0);
    words.sort_by_key(|w| w.t0);
    TranscriptData {
        source: "speech-analyzer".into(),
        model: Some(format!("SpeechTranscriber {locale}")),
        text: segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        segments,
        words,
        estimated: false,
        missing: None,
    }
}

fn feed(
    mut command: Command,
    samples: &Mutex<Vec<f32>>,
    stop: &AtomicBool,
    caption: Option<Arc<Mutex<Caption>>>,
) -> Result<Feeding> {
    let show = |change: &dyn Fn(&mut Caption)| {
        if let Some(caption) = &caption {
            change(
                &mut caption
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
        }
    };
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            show(&Caption::failed);
            return Err(error).context("Could not start scrawl-helper transcribe-stream");
        }
    };
    let stdin = child.stdin.take().context("No helper stdin")?;
    let stdout = child.stdout.take().context("No helper stdout")?;
    let mut stderr = child.stderr.take().context("No helper stderr")?;
    let (sender, lines) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("scrawl-live-transcribe-out".into())
        .spawn(move || {
            let show = |change: &dyn Fn(&mut Caption)| {
                if let Some(caption) = &caption {
                    change(
                        &mut caption
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner),
                    );
                }
            };
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                match serde_json::from_str::<Event>(&line) {
                    Ok(event) => {
                        match &event {
                            Event::Ready { .. } => show(&Caption::listening),
                            Event::Result { segment, .. } => {
                                show(&|c: &mut Caption| c.finalized(&segment.text));
                            }
                            Event::Volatile { volatile } => {
                                show(&|c: &mut Caption| c.volatile(&volatile.text));
                            }
                            Event::Done { .. } => {}
                        }
                        let _ = sender.send(Line::Event(event));
                    }
                    Err(error) => crate::capture::scrawl_log(format!(
                        "Live transcription: unreadable line ({error})"
                    )),
                }
            }
            let mut message = String::new();
            let _ = stderr.read_to_string(&mut message);
            show(&Caption::failed);
            let _ = sender.send(Line::Closed(message));
        })
        .context("Could not spawn the live transcription reader")?;
    let mut feeding = Feeding {
        child,
        stdin: Some(stdin),
        fed: 0,
        lines,
    };
    let mut bytes = Vec::new();
    while !stop.load(Ordering::Acquire) {
        bytes.clear();
        {
            let buffer = samples
                .lock()
                .map_err(|_| anyhow!("Microphone buffer lock poisoned"))?;
            // The recorder empties the buffer when it stops; the rest comes from the WAV.
            if let Some(fresh) = buffer.get(feeding.fed..) {
                bytes.extend(
                    fresh
                        .iter()
                        .flat_map(|&s| crate::audio::pcm16(s).to_le_bytes()),
                );
            }
        }
        if !bytes.is_empty() {
            let stdin = feeding.stdin.as_mut().context("No helper stdin")?;
            stdin
                .write_all(&bytes)
                .context("The live transcriber stopped")?;
            feeding.fed += bytes.len() / 2;
        }
        std::thread::park_timeout(FEED_EVERY);
    }
    Ok(feeding)
}

fn le_bytes(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

/// Plays `wav_bytes` into a live transcription as a microphone would, `speed` times real time in
/// 10 ms callbacks, and returns it ready to finish (`scrawl reprocess --bench N --live-speed S`).
pub fn simulate(wav_bytes: &[u8], speed: f64) -> Result<Live> {
    let samples = crate::audio::decode_wav_16k(wav_bytes)?;
    let buffer = Arc::new(Mutex::new(Vec::with_capacity(samples.len())));
    let live = Live::start(Arc::clone(&buffer), None)?;
    let started = Instant::now();
    for (i, chunk) in samples.chunks(160).enumerate() {
        buffer
            .lock()
            .map_err(|_| anyhow!("poisoned"))?
            .extend_from_slice(chunk);
        let due = Duration::from_secs_f64((i + 1) as f64 * 0.01 / speed.max(0.01));
        if let Some(wait) = due.checked_sub(started.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    // The recorder empties its buffer when it stops.
    buffer.lock().map_err(|_| anyhow!("poisoned"))?.clear();
    Ok(live)
}

/// Shares a live transcription between copies of a recording; the first `take` gets it.
#[derive(Clone, Default)]
pub struct Slot(Arc<Mutex<Option<Live>>>);

impl Slot {
    pub fn new(live: Option<Live>) -> Self {
        Self(Arc::new(Mutex::new(live)))
    }

    pub fn take(&self) -> Option<Live> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let live = self.0.lock().map(|live| live.is_some()).unwrap_or(false);
        f.debug_tuple("Slot").field(&live).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in helper: echoes how many samples it read, as the real one does, after the
    /// given lines.
    fn fake(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }

    #[cfg(unix)]
    fn wav(samples: &[f32]) -> Vec<u8> {
        crate::audio::encode_wav_16k(samples).unwrap()
    }

    #[cfg(unix)]
    const READY: &str = r#"echo '{"ready":true,"locale":"en_US"}'"#;

    #[cfg(unix)]
    #[test]
    fn collects_finalized_results_and_counts_every_sample() {
        let script = format!(
            r#"{READY}; n=$(wc -c | tr -d ' '); echo '{{"segment":{{"t0":900,"t1":1500,"text":"Look here."}},"words":[{{"t0":900,"t1":1200,"text":"Look"}},{{"t0":1200,"t1":1500,"text":"here."}}]}}'; echo '{{"segment":{{"t0":60,"t1":600,"text":"Okay,"}},"words":[{{"t0":60,"t1":600,"text":"Okay,"}}]}}'; echo "{{\"done\":true,\"samples\":$((n / 2))}}""#
        );
        let buffer = Arc::new(Mutex::new(vec![0.25_f32; 4_000]));
        let mut live = Live::start_with(fake(&script), Arc::clone(&buffer), None).unwrap();
        std::thread::sleep(Duration::from_millis(250));
        let mut all = vec![0.25_f32; 4_000];
        all.extend([-0.5_f32; 1_000]);
        let done = live
            .finish_within(&wav(&all), Duration::from_secs(5))
            .unwrap();
        assert_eq!(done.streamed_samples, 4_000);
        assert_eq!(done.transcript.text, "Okay, Look here.");
        assert_eq!(done.transcript.source, "speech-analyzer");
        assert_eq!(
            done.transcript.model.as_deref(),
            Some("SpeechTranscriber en_US")
        );
        assert_eq!(
            done.transcript
                .words
                .iter()
                .map(|w| (w.t0, w.text.as_str()))
                .collect::<Vec<_>>(),
            vec![(60, "Okay,"), (900, "Look"), (1200, "here.")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn streams_exactly_the_samples_the_wav_holds() {
        let out = std::env::temp_dir().join(format!("scrawl-live-pcm-{}", std::process::id()));
        let script = format!(
            r#"{READY}; cat > '{}'; echo '{{"done":true,"samples":3}}'"#,
            out.display()
        );
        let buffer = Arc::new(Mutex::new(vec![0.5_f32, -0.25]));
        let mut live = Live::start_with(fake(&script), Arc::clone(&buffer), None).unwrap();
        std::thread::sleep(Duration::from_millis(250));
        buffer.lock().unwrap().clear();
        let recording = [0.5_f32, -0.25, 1.5];
        live.finish_within(&wav(&recording), Duration::from_secs(5))
            .unwrap();
        let written = std::fs::read(&out).unwrap();
        let _ = std::fs::remove_file(&out);
        let expected = crate::audio::decode_wav_pcm16(&wav(&recording)).unwrap();
        assert_eq!(written, le_bytes(&expected));
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_helper_or_lost_samples_is_an_error() {
        let buffer = || Arc::new(Mutex::new(vec![0.1_f32; 320]));
        let failed = Live::start_with(
            fake("echo 'scrawl-helper: the speech model isnt installed' >&2; exit 1"),
            buffer(),
            None,
        )
        .unwrap()
        .finish_within(&wav(&[0.1; 320]), Duration::from_secs(5));
        assert!(failed.is_err());

        let short = format!(r#"{READY}; cat > /dev/null; echo '{{"done":true,"samples":7}}'"#);
        let lost = Live::start_with(fake(&short), buffer(), None)
            .unwrap()
            .finish_within(&wav(&[0.1; 320]), Duration::from_secs(5));
        assert!(lost.unwrap_err().to_string().contains("heard 7 samples"));

        let hung = Live::start_with(
            fake(&format!("{READY}; cat > /dev/null; sleep 30")),
            buffer(),
            None,
        )
        .unwrap()
        .finish_within(&wav(&[0.1; 320]), Duration::from_millis(300));
        assert!(hung.unwrap_err().to_string().contains("to finish"));
    }

    #[cfg(unix)]
    #[test]
    fn the_caption_shows_guesses_then_finalized_words_and_the_transcript_ignores_guesses() {
        let script = format!(
            r#"{READY}; echo '{{"volatile":{{"t0":60,"t1":400,"text":"Okay"}}}}'; echo '{{"volatile":{{"t0":60,"t1":700,"text":"Okay so"}}}}'; echo '{{"segment":{{"t0":60,"t1":900,"text":"Okay, so."}},"words":[{{"t0":60,"t1":900,"text":"Okay, so."}}]}}'; echo '{{"volatile":{{"t0":1000,"t1":1200,"text":"this"}}}}'; n=$(wc -c | tr -d ' '); echo "{{\"done\":true,\"samples\":$((n / 2))}}""#
        );
        let caption = Arc::new(Mutex::new(Caption::default()));
        let buffer = Arc::new(Mutex::new(vec![0.25_f32; 1_600]));
        let mut live = Live::start_with(
            fake(&script),
            Arc::clone(&buffer),
            Some(Arc::clone(&caption)),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while caption.lock().unwrap().text() != "Okay, so. this" && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(caption.lock().unwrap().text(), "Okay, so. this");
        assert_eq!(
            caption.lock().unwrap().state,
            crate::live_audio::CaptionState::Listening
        );
        let done = live
            .finish_within(&wav(&[0.25; 1_600]), Duration::from_secs(5))
            .unwrap();
        assert_eq!(done.transcript.text, "Okay, so.");
        // The helper exited after it was done.
        let deadline = Instant::now() + Duration::from_secs(5);
        while caption.lock().unwrap().state != crate::live_audio::CaptionState::Failed
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            caption.lock().unwrap().state,
            crate::live_audio::CaptionState::Failed
        );
    }

    #[test]
    fn a_helper_that_cant_start_fails_the_caption() {
        let caption = Arc::new(Mutex::new(Caption::default()));
        let live = Live::start_with(
            Command::new("/nonexistent/scrawl-helper"),
            Arc::new(Mutex::new(Vec::new())),
            Some(Arc::clone(&caption)),
        )
        .unwrap();
        drop(live);
        assert_eq!(
            caption.lock().unwrap().state,
            crate::live_audio::CaptionState::Failed
        );
    }

    /// The words of a real recording showing as it plays, through the real helper (needs macOS
    /// 26 with the speech model installed): `SCRAWL_LIVE_WAV=audio.wav cargo test
    /// live_words_from_the_real_helper -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_words_from_the_real_helper() {
        let wav =
            std::fs::read(std::env::var_os("SCRAWL_LIVE_WAV").expect("SCRAWL_LIVE_WAV")).unwrap();
        let mut samples = crate::audio::decode_wav_16k(&wav).unwrap();
        // A microphone goes on sending silence after the words, which lets the last ones settle.
        samples.extend([0.0; 16_000 * 3]);
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let caption = Arc::new(Mutex::new(Caption::default()));
        let live = Live::preview(Arc::clone(&buffer), Arc::clone(&caption)).unwrap();
        let started = Instant::now();
        let mut shown = String::new();
        for (i, chunk) in samples.chunks(160).enumerate() {
            buffer.lock().unwrap().extend_from_slice(chunk);
            let due = Duration::from_millis(10 * (i as u64 + 1));
            if let Some(wait) = due.checked_sub(started.elapsed()) {
                std::thread::sleep(wait);
            }
            let text = caption.lock().unwrap().text();
            if text != shown {
                println!("{:5.2}s  {text}", started.elapsed().as_secs_f32());
                shown = text;
            }
        }
        let caption = caption.lock().unwrap().clone();
        println!("{:?}: {}", caption.state, caption.text());
        assert_eq!(caption.state, crate::live_audio::CaptionState::Listening);
        assert!(!caption.text().is_empty());
        drop(live);
    }

    #[test]
    fn dropping_an_unfinished_stream_stops_the_helper() {
        let live =
            Live::start_with(fake("sleep 30"), Arc::new(Mutex::new(Vec::new())), None).unwrap();
        let started = Instant::now();
        drop(live);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
