//! iPhone sources: a live cabled capture mirrored through `scrawl-helper`, and recordings made
//! in-app with ScrawlKit. Both become ordinary `.scrawl` bundles whose keyframes are decoded
//! from the phone's own video, never from a screenshot of the mirror window.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

use crate::capture::{LiveOverlayEvent, SharedCaptureTelemetry, scrawl_log};
use crate::delivery::{Delivery, Outcome};
use crate::helper::{self, FrameTimes, PhoneDevice};
use crate::model::{
    CaptureMode, ContextChange, DeviceInfo, InteractionEvent, RgbaFrame, ScrawlBundle, ScreenSpan,
    Stroke, TargetFrame, TargetInfo,
};

/// Keyframes are decoded from the native video and fit to this long edge (736×1600 for a
/// 1320×2868 phone): every UI point stays resolvable, and agents downsample anything larger.
pub const PHONE_FRAME_EDGE: u32 = 1600;
/// Candidate frames for keyframe selection only need enough detail for scene detection.
const CANDIDATE_EDGE: u32 = 320;
const MAX_CANDIDATES: u64 = 480;

#[derive(Default)]
struct MirrorState {
    device: Option<PhoneDevice>,
    rect: Option<[f32; 4]>,
    recording_epoch_ms: Option<u64>,
    recording_instant: Option<Instant>,
    markers_epoch_ms: Vec<u64>,
    finished: Option<(PathBuf, u64)>,
    ended: bool,
    error: Option<String>,
}

/// A running mirror: the helper's floating window shows the phone, records its native video
/// with the Mac's microphone into the bundle, and reports where the picture is on screen so
/// pointer and pen input normalize into phone-screen coordinates.
pub struct PhoneSession {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    reader: Option<JoinHandle<()>>,
    state: Arc<Mutex<MirrorState>>,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    started_at: Instant,
    session_epoch_ms: u64,
    /// Held while the mirror records (not while it only previews).
    busy: Option<crate::retention::Recording>,
}

pub struct PhoneRecording {
    pub movie: PathBuf,
    pub device: PhoneDevice,
    /// How long after Scrawl's session clock started the movie's first frame arrived.
    pub offset_ms: u64,
    /// M presses in the mirror, on the movie's clock.
    pub flags: Vec<u64>,
}

#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
enum MirrorEvent {
    Device(PhoneDevice),
    Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },
    Recording {
        #[serde(rename = "startedAt")]
        started_at: u64,
    },
    Marker {
        at: u64,
    },
    Finished {
        path: PathBuf,
        #[serde(rename = "durationMs")]
        duration_ms: u64,
    },
    Cancelled,
    Warning {
        message: String,
    },
    Error {
        message: String,
    },
}

impl PhoneSession {
    /// Opens the live iPhone mirror window in preview mode without recording to disk yet.
    pub fn start_preview(
        device_id: Option<&str>,
        hidden: bool,
        telemetry: Option<Arc<SharedCaptureTelemetry>>,
    ) -> Result<Self> {
        Self::spawn_mirror(None, hidden, device_id, telemetry)
    }

    /// `telemetry` (when capturing through the app) receives the mirror's on-screen rect as the
    /// capture target, and hears when the helper ends the capture on its own.
    pub fn start(
        movie: &Path,
        device_id: Option<&str>,
        telemetry: Option<Arc<SharedCaptureTelemetry>>,
    ) -> Result<Self> {
        Self::spawn_mirror(Some(movie), false, device_id, telemetry)
    }

    fn spawn_mirror(
        movie: Option<&Path>,
        hidden: bool,
        device_id: Option<&str>,
        telemetry: Option<Arc<SharedCaptureTelemetry>>,
    ) -> Result<Self> {
        let session_epoch_ms = crate::now_epoch_ms();
        let recording = movie.map(|_| crate::retention::Recording::start());
        let mut command = Command::new(helper::path()?);
        command.arg("mirror");
        if let Some(movie) = movie {
            command.arg("--out").arg(movie);
        } else {
            command.arg("--preview");
            if hidden {
                command.arg("--hidden");
            }
        }
        if let Some(id) = device_id
            && !id.is_empty()
        {
            command.arg("--device").arg(id);
        }
        // Narration comes from the Mac's microphone: the chosen one, if any.
        if let Some(choice) = crate::mic::choice() {
            command.args(["--microphone", &choice.id, "--microphone-name", &choice.name]);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Could not start the iPhone mirror")?;
        if let Some(telemetry) = &telemetry {
            telemetry.set_capture_source_pid(child.id());
        }
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().context("No mirror output")?;
        if let Some(stderr) = child.stderr.take() {
            let _ = std::thread::Builder::new()
                .name("scrawl-phone-stderr".into())
                .spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            scrawl_log(trimmed.to_owned());
                        }
                    }
                });
        }
        let state = Arc::new(Mutex::new(MirrorState::default()));
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader_state = Arc::clone(&state);
        let reader_cancelled = Arc::clone(&cancelled);
        let reader = std::thread::Builder::new()
            .name("scrawl-phone".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    let Ok(event) = serde_json::from_str::<MirrorEvent>(&line) else {
                        continue;
                    };
                    let mut state = reader_state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    match event {
                        MirrorEvent::Device(device) => {
                            state.device = Some(device);
                        }
                        MirrorEvent::Rect {
                            x,
                            y,
                            width,
                            height,
                        } => {
                            let rect = [x as f32, y as f32, width as f32, height as f32];
                            state.rect = Some(rect);
                            if let Some(telemetry) = &telemetry {
                                *telemetry
                                    .target_frame
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                                    TargetFrame {
                                        x,
                                        y,
                                        width,
                                        height,
                                        scale: 2.0,
                                    };
                                if telemetry
                                    .selecting
                                    .load(std::sync::atomic::Ordering::Acquire)
                                {
                                    telemetry.set_picker_rect(Some((rect, 3)));
                                }
                            }
                        }
                        MirrorEvent::Recording { started_at } => {
                            state.recording_epoch_ms = Some(started_at);
                            state.recording_instant = Some(Instant::now());
                            if let Some(telemetry) = &telemetry {
                                telemetry
                                    .active
                                    .store(true, std::sync::atomic::Ordering::Release);
                            }
                        }
                        MirrorEvent::Marker { at } => state.markers_epoch_ms.push(at),
                        MirrorEvent::Finished { path, duration_ms } => {
                            state.finished = Some((path, duration_ms));
                            state.ended = true;
                        }
                        MirrorEvent::Cancelled => {
                            state.ended = true;
                        }
                        MirrorEvent::Warning { message } => {
                            scrawl_log(format!("iPhone mirror: {message}"));
                        }
                        MirrorEvent::Error { message } => {
                            scrawl_log(format!("iPhone mirror: {message}"));
                            state.error = Some(message);
                            state.ended = true;
                        }
                    }
                }
                reader_state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .ended = true;
                // The helper ended; if Scrawl didn't ask (phone unplugged, no phone, denied),
                // let the app finish or cancel the capture.
                if !reader_cancelled.load(std::sync::atomic::Ordering::Acquire)
                    && let Some(telemetry) = &telemetry
                {
                    let _ = telemetry.ui_sender.try_send(LiveOverlayEvent::PhoneEnded);
                }
            })
            .context("Could not watch the iPhone mirror")?;
        Ok(Self {
            child: Some(child),
            stdin,
            reader: Some(reader),
            state,
            cancelled,
            started_at: Instant::now(),
            session_epoch_ms,
            busy: recording,
        })
    }

    /// Transitions an open preview session into active movie recording.
    pub fn begin_recording(&mut self, movie: &Path) {
        self.started_at = Instant::now();
        self.session_epoch_ms = crate::now_epoch_ms();
        self.busy
            .get_or_insert_with(crate::retention::Recording::start);
        {
            let mut state = self.lock();
            state.recording_epoch_ms = None;
            state.recording_instant = None;
            state.markers_epoch_ms.clear();
        }
        self.send(&format!("start {}", movie.display()));
    }

    pub fn show_preview(&mut self) {
        self.send("show");
    }

    pub fn hide_preview(&mut self) {
        self.send("hide");
    }

    pub fn device(&self) -> Option<PhoneDevice> {
        self.lock().device.clone()
    }

    pub fn rect(&self) -> Option<[f32; 4]> {
        self.lock().rect
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.lock()
            .recording_instant
            .map_or(0, |t| t.elapsed().as_millis() as u64)
    }

    /// Whether the movie has started (or the capture already ended).
    pub fn has_recording(&self) -> bool {
        let state = self.lock();
        state.recording_epoch_ms.is_some() || state.finished.is_some()
    }

    /// Whether the helper process has finished, cancelled, or exited with an error.
    pub fn has_ended(&self) -> bool {
        let state = self.lock();
        state.ended || state.finished.is_some() || state.error.is_some()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MirrorState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn send(&mut self, command: &str) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = writeln!(stdin, "{command}");
            let _ = stdin.flush();
        }
    }

    fn wait(&mut self, timeout: Duration) {
        self.stdin.take();
        if let Some(ref mut child) = self.child {
            let deadline = Instant::now() + timeout;
            while Instant::now() < deadline {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }

    fn cancel_async(&mut self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.send("cancel");
        self.stdin.take();
        let child = self.child.take();
        let reader = self.reader.take();
        if child.is_some() || reader.is_some() {
            std::thread::spawn(move || {
                if let Some(mut child) = child {
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while Instant::now() < deadline {
                        if matches!(child.try_wait(), Ok(Some(_))) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                }
                if let Some(reader) = reader {
                    let _ = reader.join();
                }
            });
        }
    }

    /// Stops recording and waits for the helper to finalize the movie.
    pub fn stop(mut self) -> Result<PhoneRecording> {
        self.send("stop");
        self.wait(Duration::from_secs(15));
        let state = std::mem::take(&mut *self.lock());
        let Some((movie, _)) = state.finished else {
            bail!(
                "{}",
                state
                    .error
                    .unwrap_or_else(|| "The iPhone recording did not finish".into())
            );
        };
        let device = state.device.context("No iPhone was attached")?;
        let started = state.recording_epoch_ms.unwrap_or(self.session_epoch_ms);
        Ok(PhoneRecording {
            movie,
            device,
            offset_ms: started.saturating_sub(self.session_epoch_ms),
            flags: state
                .markers_epoch_ms
                .iter()
                .map(|at| at.saturating_sub(started))
                .collect(),
        })
    }

    pub fn cancel(mut self) {
        self.cancel_async();
    }
}

impl Drop for PhoneSession {
    fn drop(&mut self) {
        self.cancel_async();
    }
}

/// Everything a phone movie needs to become a bundle, on the movie's clock.
pub struct MovieCapture {
    pub movie: PathBuf,
    pub target: TargetInfo,
    pub device: DeviceInfo,
    pub events: Vec<InteractionEvent>,
    pub strokes: Vec<Stroke>,
    pub flags: Vec<u64>,
    pub note: Option<String>,
    /// What the app said about itself (ScrawlKit context and screens).
    pub context: BTreeMap<String, String>,
    pub context_changes: Vec<ContextChange>,
    pub screens: Vec<ScreenSpan>,
}

/// `app` is the display name and `bundle_id` the identifier; an old timeline without a bundle
/// ID falls back to the name for both.
pub fn phone_target(
    name: &str,
    app: Option<&str>,
    bundle_id: Option<&str>,
    width: u32,
    height: u32,
) -> TargetInfo {
    let bundle_id = bundle_id.filter(|id| !id.is_empty()).or(app);
    TargetInfo {
        mode: CaptureMode::Device,
        app: Some(app.or(bundle_id).unwrap_or(name).to_owned()),
        bundle_id: bundle_id.map(str::to_owned),
        window_title: None,
        window_id: None,
        display_id: 0,
        frame: TargetFrame {
            x: 0.0,
            y: 0.0,
            width: f64::from(width),
            height: f64::from(height),
            scale: 1.0,
        },
    }
}

/// Builds a bundle from a phone movie already inside `bundle_dir`: narration transcribed with
/// word timings, keyframes chosen on small candidate frames and then decoded again from the
/// native video.
pub fn build_movie_bundle(
    bundle_dir: &Path,
    mut capture: MovieCapture,
) -> Result<(PathBuf, ScrawlBundle, String)> {
    let started_at = Instant::now();
    let wav = bundle_dir.join("audio.wav");
    let mut mark_times: Vec<u64> = crate::sketch::marks_of(&capture.strokes)
        .into_iter()
        .map(|m| m.t1)
        .chain(capture.flags.iter().copied())
        .collect();
    mark_times.sort_unstable();
    mark_times.dedup();
    let ((transcript, audio_ms, transcribe_ms, audio_err), joined) =
        std::thread::scope(|scope| {
            let base_target = capture.target.clone();
            let strokes_ref = &capture.strokes;
            let events_ref = &capture.events;
            let flags_ref = &capture.flags;
            let movie_ref = &capture.movie;
            let frames = scope.spawn(move || -> Result<_> {
                let probe = helper::probe(movie_ref)?;
                ensure!(probe.duration_ms > 0, "The recording is empty");
                let mut target = base_target;
                if probe.width > 0 && probe.height > 0 {
                    target.frame.width = f64::from(probe.width);
                    target.frame.height = f64::from(probe.height);
                }
                let every = (probe.duration_ms / MAX_CANDIDATES).max(250);
                let t0 = Instant::now();
                let budget = crate::model::keyframe_budget(probe.duration_ms);
                // With a mark for every slot, only the frames under the marks are needed, unless
                // marks on one screen share a keyframe and leave slots for other moments.
                if mark_times.len() >= budget {
                    let direct = helper::frames(
                        movie_ref,
                        FrameTimes::At(&mark_times),
                        PHONE_FRAME_EDGE,
                    );
                    let decode_ms = t0.elapsed().as_millis() as u64;
                    if let Ok(raw_frames) = direct
                        && !raw_frames.is_empty()
                    {
                        let mut preview_marks = crate::sketch::marks_of(strokes_ref);
                        for (i, &t) in flags_ref.iter().enumerate() {
                            preview_marks.push(crate::model::Mark {
                                id: format!("f{}", i + 1),
                                strokes: Vec::new(),
                                t0: t,
                                t1: t,
                                box_norm: [0.0, 0.0, 1.0, 1.0],
                                shape: crate::model::Shape::Flag,
                                from: None,
                                to: None,
                                pin: None,
                                on: None,
                                description: "flagged this moment".into(),
                                frame: None,
                            });
                        }
                        let preview = crate::keyframes::select_keyframes(
                            &raw_frames,
                            strokes_ref,
                            &mut preview_marks,
                            events_ref,
                            &crate::model::TranscriptData::default(),
                            budget,
                        );
                        if preview.len() == budget
                            && preview
                                .iter()
                                .all(|(k, _)| k.reason == crate::model::KeyframeReason::Mark)
                        {
                            let prep = crate::prepare_visuals(
                                preview,
                                raw_frames.first(),
                                None,
                                &target,
                                probe.duration_ms,
                                strokes_ref,
                                &preview_marks,
                                events_ref,
                                &crate::windows::ActivityLog::default(),
                            );
                            return Ok((probe, raw_frames, decode_ms, true, prep));
                        }
                    }
                }
                let res = helper::frames(movie_ref, FrameTimes::Every(every), CANDIDATE_EDGE)?;
                Ok((probe, res, t0.elapsed().as_millis() as u64, false, None))
            });
            let audio_started = Instant::now();
            let exported = helper::export_audio(&capture.movie, &wav)
                .and_then(|()| crate::audio::recorded_audio_from_wav(std::fs::read(&wav)?));
            let audio_ms = audio_started.elapsed().as_millis() as u64;
            let (audio, audio_err) = match exported {
                Ok(a) => (Some(a), None),
                Err(e) => (None, Some(e)),
            };
            let t0 = Instant::now();
            let transcript = crate::transcribe::transcribe(
                audio.as_ref().map(|_| wav.as_path()),
                audio.as_ref(),
            );
            let transcribe_ms = t0.elapsed().as_millis() as u64;
            (
                (transcript, audio_ms, transcribe_ms, audio_err),
                frames.join(),
            )
        });
    let (probe, frames, candidates_ms, already_full, prepared) = joined
        .map_err(|_| anyhow::anyhow!("Frame decoding panicked"))??;
    if probe.audio_tracks > 0
        && let Some(error) = audio_err
    {
        scrawl_log(format!("Phone audio export failed: {error:#}"));
    }
    if probe.width > 0 && probe.height > 0 {
        capture.device.width = probe.width;
        capture.device.height = probe.height;
        capture.target.frame.width = f64::from(probe.width);
        capture.target.frame.height = f64::from(probe.height);
    }
    ensure!(!frames.is_empty(), "No frames could be decoded");
    let movie = capture.movie.clone();
    let extras = crate::SourceExtras {
        source: Some("iphone".into()),
        device: Some(capture.device),
        context: capture.context,
        context_changes: capture.context_changes,
        screens: capture.screens,
        flags: capture.flags,
        full_frames: (!already_full).then(|| -> crate::FullFrames {
            Box::new(move |times: &[u64]| -> Result<Vec<RgbaFrame>> {
                helper::frames(&movie, FrameTimes::At(times), PHONE_FRAME_EDGE)
            })
        }),
        prepared,
        timings: crate::StageTimings {
            started_at: Some(started_at),
            audio_ms: Some(audio_ms),
            candidates_ms: Some(candidates_ms),
            transcribe_ms: Some(transcribe_ms),
            ..Default::default()
        },
        ..Default::default()
    };
    crate::build_bundle_from_parts(
        bundle_dir,
        capture.target,
        probe.duration_ms,
        frames,
        capture.strokes,
        capture.events,
        transcript,
        Some(capture.movie),
        capture.note,
        extras,
    )
}

/// Finishes a live capture: re-bases Scrawl's pointer and pen input onto the movie's clock.
pub fn finalize_live_capture(
    bundle_dir: &Path,
    recording: PhoneRecording,
    mut events: Vec<InteractionEvent>,
    mut strokes: Vec<Stroke>,
    note: Option<String>,
) -> Result<(PathBuf, ScrawlBundle, String)> {
    events.retain_mut(|event| event.rebase(recording.offset_ms));
    strokes.retain_mut(|stroke| stroke.rebase(recording.offset_ms));
    let device = &recording.device;
    let target = phone_target(&device.name, None, None, device.width, device.height);
    let info = DeviceInfo {
        name: device.name.clone(),
        capture: "mirror".into(),
        id: Some(device.id.clone()),
        model: Some(device.model.clone()),
        width: device.width,
        height: device.height,
        ..Default::default()
    };
    let built = build_movie_bundle(
        bundle_dir,
        MovieCapture {
            movie: recording.movie,
            target,
            device: info,
            events,
            strokes,
            flags: recording.flags,
            note,
            context: BTreeMap::new(),
            context_changes: Vec::new(),
            screens: Vec::new(),
        },
    )?;
    // The native video is ~18 Mbps; re-encode it a minute from now (in the app).
    crate::retention::after_phone_capture(&built.0);
    Ok(built)
}

// MARK: ScrawlKit import

/// The `.json` ScrawlKit writes beside each in-app recording (seconds from the start, points in
/// screen coordinates).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub recorded_at: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub screen: Option<Screen>,
    #[serde(default)]
    pub markers: Vec<f64>,
    #[serde(default)]
    pub events: Vec<AppEvent>,
    #[serde(default)]
    pub touches: Vec<Touch>,
    #[serde(default)]
    pub strokes: Vec<TimelineStroke>,
    /// `CFBundleShortVersionString`; `version` is the build.
    #[serde(default)]
    pub short_version: Option<String>,
    /// App-defined values when the recording started.
    #[serde(default)]
    pub context: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub context_changes: Vec<TimelineContextChange>,
    /// Screen changes; the first is at 0 when the app named a screen before recording.
    #[serde(default)]
    pub screens: Vec<TimelineScreen>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TimelineContextChange {
    pub t: f64,
    pub key: String,
    #[serde(default)]
    pub value: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TimelineScreen {
    pub t: f64,
    /// `null` when the app stopped naming a screen.
    #[serde(default)]
    pub name: Option<String>,
}

/// A context value as text: strings as they are, numbers and booleans spelled out, `null` none.
fn context_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

impl Timeline {
    fn context(&self) -> BTreeMap<String, String> {
        self.context
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), context_text(v)?)))
            .collect()
    }

    fn context_changes(&self) -> Vec<ContextChange> {
        let mut changes: Vec<ContextChange> = self
            .context_changes
            .iter()
            .map(|c| ContextChange {
                t: ms(c.t),
                key: c.key.clone(),
                value: context_text(&c.value),
            })
            .collect();
        changes.sort_by_key(|c| c.t);
        changes
    }

    fn screen_spans(&self, duration_ms: u64) -> Vec<ScreenSpan> {
        let changes: Vec<(u64, Option<String>)> = self
            .screens
            .iter()
            .map(|s| (ms(s.t), s.name.clone()))
            .collect();
        ScreenSpan::from_changes(&changes, duration_ms)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct TimelineStroke {
    pub t0: f64,
    pub t1: f64,
    #[serde(default)]
    pub points: Vec<[f64; 3]>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Screen {
    pub width: f64,
    pub height: f64,
    #[serde(default = "one")]
    pub scale: f64,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug, Deserialize)]
pub struct AppEvent {
    pub t: f64,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Touch {
    pub t: f64,
    pub id: i64,
    pub x: f64,
    pub y: f64,
    pub phase: String,
    #[serde(default)]
    pub label: Option<String>,
}

fn ms(seconds: f64) -> u64 {
    if seconds.is_finite() && seconds > 0.0 {
        (seconds * 1_000.0).round() as u64
    } else {
        0
    }
}

/// Finger movement under this many points, held under this long, is a tap.
const TAP_SLOP_PT: f64 = 12.0;
const TAP_MS: u64 = 420;

/// Turns raw touch samples into taps and swipes, normalized to the screen.
pub fn touch_events(touches: &[Touch], screen: Screen) -> Vec<InteractionEvent> {
    let mut sorted = touches.to_vec();
    sorted.sort_by(|a, b| a.t.total_cmp(&b.t));
    let (w, h) = (screen.width.max(1.0), screen.height.max(1.0));
    let norm = |touch: &Touch| {
        [
            (touch.x / w).clamp(0.0, 1.0) as f32,
            (touch.y / h).clamp(0.0, 1.0) as f32,
        ]
    };
    let mut open: Vec<(i64, Vec<Touch>)> = Vec::new();
    let mut out = Vec::new();
    let close = |samples: Vec<Touch>, out: &mut Vec<InteractionEvent>| {
        let (Some(first), Some(last)) = (samples.first(), samples.last()) else {
            return;
        };
        let travel = samples
            .iter()
            .map(|s| (s.x - first.x).hypot(s.y - first.y))
            .fold(0.0, f64::max);
        let (t0, t1) = (ms(first.t), ms(last.t));
        let [x0, y0] = norm(first);
        let hit_label = samples
            .iter()
            .find_map(|s| s.label.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if travel < TAP_SLOP_PT && t1.saturating_sub(t0) < TAP_MS {
            out.push(InteractionEvent::Click {
                t: t0,
                x: x0,
                y: y0,
                button: Some("touch".into()),
                label: hit_label.map(str::to_owned),
                role: None,
            });
        } else {
            let [x1, y1] = norm(last);
            let drag_label = if travel < TAP_SLOP_PT {
                Some(match hit_label {
                    Some(l) => format!("long press: {l}"),
                    None => "long press".into(),
                })
            } else {
                hit_label.map(str::to_owned)
            };
            out.push(InteractionEvent::Drag {
                t: t0,
                t1,
                x0,
                y0,
                x1,
                y1,
                label: drag_label,
                button: Some("touch".into()),
                path: samples.iter().map(norm).collect(),
            });
        }
    };
    for touch in sorted {
        let index = open.iter().position(|(id, _)| *id == touch.id);
        match (touch.phase.as_str(), index) {
            ("down", Some(i)) => {
                let (_, samples) = open.remove(i);
                close(samples, &mut out);
                open.push((touch.id, vec![touch]));
            }
            ("down", None) => open.push((touch.id, vec![touch])),
            ("up" | "cancel", Some(i)) => {
                let (_, mut samples) = open.remove(i);
                samples.push(touch);
                close(samples, &mut out);
            }
            ("up" | "cancel", None) => close(vec![touch], &mut out),
            (_, Some(i)) => open[i].1.push(touch),
            (_, None) => open.push((touch.id, vec![touch])),
        }
    }
    for (_, samples) in open {
        close(samples, &mut out);
    }
    out.sort_by_key(InteractionEvent::timestamp_ms);
    out
}

fn is_movie(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "mp4" | "mov" | "m4v"))
}

fn sibling(path: &Path, want_movie: bool) -> Option<PathBuf> {
    let stem = path.file_stem()?;
    let dir = path.parent()?;
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p != path && p.file_stem() == Some(stem))
        .find(|p| {
            if want_movie {
                is_movie(p)
            } else {
                p.extension().is_some_and(|e| e == "json")
            }
        })
}

/// Resolves what `scrawl import` was given into a movie and its timeline: a movie (timeline
/// beside it), a timeline (movie beside it), or nothing (the newest pair in ~/Downloads, where
/// AirDrop puts them).
pub fn resolve_import(
    path: Option<&Path>,
    timeline: Option<&Path>,
) -> Result<(PathBuf, Option<PathBuf>)> {
    let movie = match path {
        Some(path) if is_movie(path) => path.to_path_buf(),
        Some(path) => sibling(path, true)
            .with_context(|| format!("No .mp4 or .mov beside {}", path.display()))?,
        None => {
            let downloads = dirs::download_dir().context("No Downloads folder")?;
            std::fs::read_dir(&downloads)?
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter(|p| is_movie(p) && sibling(p, false).is_some())
                .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
                .with_context(|| {
                    format!(
                        "No ScrawlKit recording (movie + .json) in {}",
                        downloads.display()
                    )
                })?
        }
    };
    ensure!(movie.is_file(), "{} does not exist", movie.display());
    let timeline = match (timeline, path) {
        (Some(t), _) => Some(t.to_path_buf()),
        (None, Some(p)) if !is_movie(p) => Some(p.to_path_buf()),
        _ => sibling(&movie, false),
    };
    Ok((movie, timeline))
}

/// `scrawl import`: a ScrawlKit recording becomes a bundle. Touches become taps and swipes
/// (ripples and trails on keyframes), markers become flags, app events land in the summary.
pub fn import(
    movie: &Path,
    timeline_path: Option<&Path>,
    bundle_dir: &Path,
    note: Option<String>,
) -> Result<(PathBuf, ScrawlBundle, String)> {
    let timeline: Timeline = match timeline_path {
        Some(path) => serde_json::from_slice(&std::fs::read(path)?)
            .with_context(|| format!("Invalid ScrawlKit timeline {}", path.display()))?,
        None => Timeline::default(),
    };
    std::fs::create_dir_all(bundle_dir)?;
    let ext = movie
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4")
        .to_ascii_lowercase();
    let copied = bundle_dir.join(format!("recording.{ext}"));
    std::fs::copy(movie, &copied).context("Could not copy the recording")?;
    if let Some(path) = timeline_path {
        std::fs::copy(path, bundle_dir.join("timeline.json"))?;
    }

    let probe = helper::probe(&copied)?;
    let screen = timeline.screen.unwrap_or(Screen {
        width: f64::from(probe.width.max(1)),
        height: f64::from(probe.height.max(1)),
        scale: 1.0,
    });
    let mut events = touch_events(&timeline.touches, screen);
    events.extend(timeline.events.iter().map(|e| InteractionEvent::AppEvent {
        t: ms(e.t),
        label: e.label.clone(),
    }));
    events.sort_by_key(InteractionEvent::timestamp_ms);
    let (sw, sh) = (screen.width.max(1.0), screen.height.max(1.0));
    let strokes: Vec<Stroke> = timeline
        .strokes
        .iter()
        .filter_map(|s| {
            let t0 = ms(s.t0);
            let t1 = ms(s.t1).max(t0);
            let mut pts = Vec::with_capacity(s.points.len() * 3);
            for &[x, y, pt_t] in &s.points {
                let nx = ((x / sw).clamp(0.0, 1.0) * f64::from(crate::model::SCALE)).round() as i32;
                let ny = ((y / sh).clamp(0.0, 1.0) * f64::from(crate::model::SCALE)).round() as i32;
                let dt = ms(pt_t).saturating_sub(t0) as i32;
                pts.extend_from_slice(&[nx, ny, dt]);
            }
            (!pts.is_empty()).then_some(Stroke {
                t0,
                t1,
                fw: probe.width,
                fh: probe.height,
                on: None,
                pin: None,
                laser: false,
                pts,
            })
        })
        .collect();
    let name = timeline.device.clone().unwrap_or_else(|| "iPhone".into());
    let app = timeline.app.clone().filter(|a| !a.is_empty());
    let nonempty = |s: &Option<String>| s.clone().filter(|s| !s.is_empty());
    let device = DeviceInfo {
        name: name.clone(),
        capture: "scrawlkit".into(),
        system: timeline.system.clone(),
        width: probe.width,
        height: probe.height,
        screen: timeline.screen.map(|s| [s.width, s.height, s.scale]),
        app: app.clone(),
        app_version: nonempty(&timeline.version),
        app_short_version: nonempty(&timeline.short_version),
        ..Default::default()
    };
    let target = phone_target(
        &name,
        app.as_deref(),
        timeline.bundle_id.as_deref(),
        probe.width,
        probe.height,
    );
    build_movie_bundle(
        bundle_dir,
        MovieCapture {
            movie: copied,
            target,
            device,
            events,
            strokes,
            flags: timeline.markers.iter().map(|&t| ms(t)).collect(),
            note,
            context: timeline.context(),
            context_changes: timeline.context_changes(),
            screens: timeline.screen_spans(probe.duration_ms),
        },
    )
}

pub const DEFAULT_INGEST_PORT: u16 = 47777;

pub type IngestCallback = Arc<dyn Fn(PathBuf, ScrawlBundle, Option<String>) + Send + Sync>;

#[derive(Clone, Default)]
pub struct IngestConfig {
    pub copy_clipboard: bool,
    pub route_opencode: bool,
    pub on_bundle: Option<IngestCallback>,
    /// Where bundles are built and pending deliveries retried; the default captures folder if
    /// unset.
    captures: Option<PathBuf>,
    hooks: IngestHooks,
}

impl IngestConfig {
    fn captures_dir(&self) -> PathBuf {
        self.captures
            .clone()
            .unwrap_or_else(crate::handoff::default_captures_dir)
    }
}

/// The steps of an ingest that reach outside the process, swappable in tests so they never
/// build real movies or post to a real OpenCode session.
#[derive(Clone, Copy)]
struct IngestHooks {
    resolve: fn(Option<&str>, Option<&str>) -> Result<(String, String)>,
    /// Movie, timeline, bundle directory → the built bundle.
    build: fn(&Path, &Path, &Path) -> Result<(PathBuf, Option<ScrawlBundle>)>,
    forward: crate::delivery::Forward,
}

impl Default for IngestHooks {
    fn default() -> Self {
        Self {
            resolve: crate::handoff::resolve_opencode_session,
            build: |movie, timeline, bundle_dir| {
                let (dir, bundle, _) = import(movie, Some(timeline), bundle_dir, None)?;
                Ok((dir, Some(bundle)))
            },
            forward: crate::handoff::send_bundle_to_session,
        }
    }
}

/// How long an upload waits for its bundle to be built and sent before the Mac answers
/// "processing". The phone can ask for less with `X-Scrawl-Wait` (seconds); without it the Mac
/// stays under the 30 s idle timeout of ScrawlKit builds from before the header.
const REPLY_WAIT_MAX: Duration = Duration::from_secs(45);
const REPLY_WAIT_DEFAULT: Duration = Duration::from_secs(25);
/// Over USB the phone gives up 30 s after it stopped, and the copy takes some of that.
const USB_REPLY_WAIT: Duration = Duration::from_secs(15);

/// Constant-time comparison for bearer tokens so timing does not leak token bytes.
pub fn constant_time_eq(provided: &str, expected: &str) -> bool {
    let a = provided.as_bytes();
    let b = expected.as_bytes();
    if b.is_empty() {
        return false;
    }
    let mut diff = (a.len() ^ b.len()) as u8;
    for (i, &expected_byte) in b.iter().enumerate() {
        let provided_byte = a.get(i).copied().unwrap_or(0);
        diff |= provided_byte ^ expected_byte;
    }
    diff == 0 && a.len() == b.len()
}

fn bearer_token(header: &str) -> Option<&str> {
    header.lines().skip(1).find_map(|line| {
        let (k, v) = line.split_once(':')?;
        if !k.trim().eq_ignore_ascii_case("authorization") {
            return None;
        }
        let v = v.trim();
        let rest = v
            .strip_prefix("Bearer ")
            .or_else(|| v.strip_prefix("bearer "))?;
        let tok = rest.trim();
        (!tok.is_empty()).then_some(tok)
    })
}

/// Discovers this Mac's candidate Scrawl ingest base URLs on `port`: Tailscale IPv4 first,
/// followed by `en0` / `en1` LAN IPs.
pub fn discover_default_ingest_urls(port: u16) -> Vec<String> {
    let mut urls = Vec::new();
    let push_ip = |urls: &mut Vec<String>, raw: &str| {
        for line in raw.lines() {
            let ip = line.trim();
            if !ip.is_empty()
                && ip
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
            {
                let candidate = format!("http://{ip}:{port}");
                if !urls.contains(&candidate) {
                    urls.push(candidate);
                }
            }
        }
    };

    for bin in [
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
        "tailscale",
    ] {
        if let Ok(out) = Command::new(bin).args(["ip", "-4"]).output()
            && out.status.success()
        {
            push_ip(&mut urls, &String::from_utf8_lossy(&out.stdout));
            if !urls.is_empty() {
                break;
            }
        }
    }

    for iface in ["en0", "en1"] {
        if let Ok(out) = Command::new("/usr/sbin/ipconfig")
            .args(["getifaddr", iface])
            .output()
            && out.status.success()
        {
            push_ip(&mut urls, &String::from_utf8_lossy(&out.stdout));
        }
    }
    urls
}

fn normalize_http_base_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
        .with_context(|| format!("Base URL must start with http:// or https://: {raw}"))?;
    ensure!(
        !rest.is_empty() && !rest.starts_with('/'),
        "Base URL is missing a host: {raw}"
    );
    Ok(trimmed.to_owned())
}

/// Formats `<scheme>://scrawl?url=<urlencoded base>[&url=<second>]&token=<token>[&session=ses_…]`.
pub fn format_setup_link(
    scheme: &str,
    urls: &[String],
    token: &str,
    session: Option<&str>,
) -> Result<String> {
    let scheme = scheme
        .trim()
        .trim_end_matches("://")
        .trim_end_matches(':');
    ensure!(
        !scheme.is_empty()
            && scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b)),
        "Invalid URL scheme: {scheme}"
    );
    let token = token.trim();
    ensure!(!token.is_empty(), "Missing ingest token");

    let mut clean_urls = Vec::new();
    for raw in urls {
        let normalized = normalize_http_base_url(raw)?;
        if !clean_urls.contains(&normalized) {
            clean_urls.push(normalized);
        }
    }
    ensure!(
        !clean_urls.is_empty(),
        "At least one http:// or https:// base URL is required"
    );

    let mut query = Vec::with_capacity(clean_urls.len() + 2);
    for url in &clean_urls {
        query.push(format!("url={}", crate::handoff::query_escape(url)));
    }
    query.push(format!("token={}", crate::handoff::query_escape(token)));
    if let Some(session) = session.map(str::trim).filter(|s| !s.is_empty()) {
        query.push(format!("session={}", crate::handoff::query_escape(session)));
    }
    Ok(format!("{scheme}://scrawl?{}", query.join("&")))
}

/// Starts Scrawl's background ScrawlKit ingest inside the GUI app: an HTTP server on
/// `0.0.0.0:47777` the phone uploads to, and a cabled-iPhone `devicectl` watcher for when the
/// network can't reach the Mac. Each recording becomes a `.scrawl` bundle, lands on the clipboard
/// and is queued into the OpenCode session matching the app.
pub fn start_ingest_server(ui_sender: flume::Sender<LiveOverlayEvent>) {
    if let Err(err) = crate::handoff::ensure_ingest_token() {
        scrawl_log(format!("ingest token setup failed: {err:#}"));
    }
    let route_opencode = std::env::var_os("SCRAWL_NO_OPENCODE").is_none();
    let config = IngestConfig {
        copy_clipboard: true,
        route_opencode,
        on_bundle: Some(Arc::new(move |dir, bundle, status| {
            let _ = ui_sender.send(LiveOverlayEvent::IngestedBundle {
                dir,
                bundle: Box::new(bundle),
                status,
            });
        })),
        ..IngestConfig::default()
    };
    let http_cfg = config.clone();
    let _ = std::thread::Builder::new()
        .name("scrawl-ingest-http".into())
        .spawn(move || {
            if let Err(err) = serve_http(DEFAULT_INGEST_PORT, http_cfg) {
                scrawl_log(format!("ingest server: {err:#}"));
            }
        });
    spawn_usb_poller(config);
}

/// Runs the headless ScrawlKit ingest server (`scrawl serve`): same HTTP auth, dedupe, bundle
/// building, and OpenCode routing as the GUI app, without touching the clipboard or HUD.
pub fn run_ingest_server(port: u16, usb: bool, no_opencode: bool) -> Result<()> {
    let _ = crate::handoff::ensure_ingest_token()?;
    let route_opencode = !no_opencode && std::env::var_os("SCRAWL_NO_OPENCODE").is_none();
    let config = IngestConfig {
        copy_clipboard: false,
        route_opencode,
        on_bundle: None,
        ..IngestConfig::default()
    };
    if usb {
        spawn_usb_poller(config.clone());
    }
    serve_http(port, config)
}

fn serve_http(port: u16, config: IngestConfig) -> Result<()> {
    let addr = format!("0.0.0.0:{port}");
    let listener = std::net::TcpListener::bind(&addr)
        .with_context(|| format!("port {port} already in use"))?;
    scrawl_log(format!("ingest server listening on {addr}"));
    crate::web::set_serving();
    // Only the process that owns the port retries deliveries, so two never race over a bundle.
    if config.route_opencode {
        crate::delivery::spawn_retries(config.captures_dir(), config.hooks.forward, mark_sent);
    }
    let connections = crate::ingest_http::Connections::default();
    for stream in listener.incoming().flatten() {
        let Some(permit) = connections.acquire() else {
            drop(stream);
            continue;
        };
        let cfg = config.clone();
        let _ = std::thread::Builder::new()
            .name("scrawl-ingest-client".into())
            .spawn(move || {
                let _permit = permit;
                if let Err(err) = handle_ingest_connection(stream, &cfg) {
                    scrawl_log(format!("ingest http: {err:#}"));
                }
            });
    }
    Ok(())
}

fn spawn_usb_poller(config: IngestConfig) {
    let _ = std::thread::Builder::new()
        .name("scrawl-ingest-usb".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(1500));
                let apps = crate::handoff::usb_apps();
                if apps.is_empty() {
                    continue;
                }
                // The USB registry check is instant, so `xcrun` only runs with an iPhone cabled.
                if helper::devices().map_or(true, |d| d.is_empty()) {
                    continue;
                }
                if let Err(err) = poll_usb_app_containers(&apps, &config) {
                    scrawl_log(format!("ingest usb: {err:#}"));
                }
            }
        });
}

/// Where a recording is in its ingest.
#[derive(Clone, Debug)]
enum IngestState {
    Processing,
    Done(Delivery),
}

/// Recordings taken in, by `recordedAt`, so one that arrives over both HTTP and USB (or twice
/// over HTTP, to the LAN and Tailscale addresses) is built once and every copy hears how it
/// went. A recording whose bundle failed to build is forgotten so it can be sent again.
static INGESTED: Mutex<BTreeMap<String, IngestState>> = Mutex::new(BTreeMap::new());
static INGEST_CHANGED: std::sync::Condvar = std::sync::Condvar::new();

fn ingested() -> std::sync::MutexGuard<'static, BTreeMap<String, IngestState>> {
    INGESTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn ingest_key(timeline: &Timeline) -> String {
    timeline
        .recorded_at
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(crate::make_bundle_id)
}

/// A retried delivery went through: later copies of that recording hear it was sent.
fn mark_sent(_: &Path, pending: &crate::delivery::Pending) {
    if let Some(key) = &pending.key {
        ingested().insert(
            key.clone(),
            IngestState::Done(Delivery::Sent {
                title: pending.title.clone(),
            }),
        );
        INGEST_CHANGED.notify_all();
    }
}

/// Waits up to `wait` for the recording `key` to finish.
fn wait_for_ingest(key: &str, wait: Duration) -> Outcome {
    let deadline = Instant::now() + wait;
    let mut map = ingested();
    loop {
        match map.get(key) {
            Some(IngestState::Done(delivery)) => return Outcome::Done(delivery.clone()),
            None => return Outcome::Failed("The Mac couldn't build this recording".into()),
            Some(IngestState::Processing) => {}
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Outcome::Processing;
        }
        map = INGEST_CHANGED
            .wait_timeout(map, left)
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .0;
    }
}

/// Takes in a received recording whose files are in `staging` (removed when done): builds its
/// bundle and sends it to its session on a thread, and waits up to `wait` to say how that went.
/// A copy of a recording already being taken in waits for that one instead.
fn ingest_recording(
    staging: PathBuf,
    movie: PathBuf,
    timeline_path: PathBuf,
    timeline: &Timeline,
    config: &IngestConfig,
    wait: Duration,
) -> (Outcome, String) {
    let key = ingest_key(timeline);
    let (session, title) = ingest_target(timeline, config);
    let fresh = {
        let mut map = ingested();
        if map.contains_key(&key) {
            false
        } else {
            map.insert(key.clone(), IngestState::Processing);
            true
        }
    };
    if !fresh {
        let _ = std::fs::remove_dir_all(&staging);
        scrawl_log(format!(
            "ingest: {key} arrived again; waiting for the first copy"
        ));
        return (wait_for_ingest(&key, wait), title);
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let spawned = {
        let (key, title, config) = (key.clone(), title.clone(), config.clone());
        std::thread::Builder::new()
            .name("scrawl-ingest-build".into())
            .spawn(move || {
                let built = build_ingested(
                    &movie,
                    &timeline_path,
                    session.as_deref(),
                    &title,
                    &key,
                    &config,
                );
                let _ = std::fs::remove_dir_all(&staging);
                match &built {
                    Ok(delivery) => {
                        ingested().insert(key, IngestState::Done(delivery.clone()));
                    }
                    Err(err) => {
                        scrawl_log(format!("ingest: {key} failed to build ({err:#})"));
                        ingested().remove(&key);
                    }
                }
                INGEST_CHANGED.notify_all();
                let _ = sender.send(built.map_err(|err| format!("{err:#}")));
            })
    };
    if let Err(err) = spawned {
        ingested().remove(&key);
        INGEST_CHANGED.notify_all();
        return (
            Outcome::Failed(format!("Could not start the build: {err}")),
            title,
        );
    }
    let outcome = match receiver.recv_timeout(wait) {
        Ok(Ok(delivery)) => Outcome::Done(delivery),
        Ok(Err(err)) => Outcome::Failed(err),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            scrawl_log(format!(
                "ingest: {key} still building after {} s; answering \"processing\"",
                wait.as_secs()
            ));
            Outcome::Processing
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Outcome::Failed("The build stopped".into())
        }
    };
    (outcome, title)
}

/// The OpenCode session a recording goes to (if one resolves) and the name the phone shows.
fn ingest_target(timeline: &Timeline, config: &IngestConfig) -> (Option<String>, String) {
    if !config.route_opencode {
        return (None, "Mac".to_owned());
    }
    let app = timeline.app.as_deref().or(timeline.bundle_id.as_deref());
    match (config.hooks.resolve)(timeline.session.as_deref(), app) {
        Ok((id, title)) => (Some(id), title),
        Err(err) => {
            scrawl_log(format!("ingest: no OpenCode session ({err:#})"));
            (None, "Mac".to_owned())
        }
    }
}

/// Builds the bundle for a received recording, optionally copies it to the clipboard, queues it
/// into `session` (leaving a pending delivery to retry if that fails), and notifies the GUI HUD
/// if running inside the app.
fn build_ingested(
    movie: &Path,
    timeline_path: &Path,
    session: Option<&str>,
    title: &str,
    key: &str,
    config: &IngestConfig,
) -> Result<Delivery> {
    let bundle_dir = config
        .captures_dir()
        .join(format!("{}.scrawl", crate::make_bundle_id()));
    let (dir, bundle) = (config.hooks.build)(movie, timeline_path, &bundle_dir)?;
    if config.copy_clipboard {
        let _ = crate::handoff::copy_bundle_to_clipboard(&dir);
    }
    let delivery = match session {
        Some(session) if config.route_opencode => {
            crate::delivery::deliver(&dir, session, title, Some(key), config.hooks.forward)
        }
        _ => Delivery::Saved,
    };
    let status = delivery.status(if config.copy_clipboard {
        "Copied to clipboard"
    } else {
        "Saved bundle"
    });
    scrawl_log(format!("ingest: {} ({status})", dir.display()));
    // The user's own command, if any; it logs how it went and never holds up the phone's reply.
    crate::on_capture::spawn(&dir, |_| {});
    if let (Some(on_bundle), Some(bundle)) = (&config.on_bundle, bundle) {
        on_bundle(dir, bundle, Some(status));
    }
    Ok(delivery)
}

fn http_json(stream: &mut std::net::TcpStream, status: &str, body: &serde_json::Value) {
    let body = body.to_string();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn http_unauthorized(stream: &mut std::net::TcpStream) {
    use std::io::Read as _;
    let response = b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    let _ = stream.write_all(response);
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let mut discard = [0_u8; 4096];
    for _ in 0..16 {
        match stream.read(&mut discard) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

fn handle_ingest_connection(
    stream: std::net::TcpStream,
    config: &IngestConfig,
) -> Result<()> {
    let expected_token = crate::handoff::ensure_ingest_token()?;
    handle_ingest_connection_with_token(stream, &expected_token, config)
}

fn handle_ingest_connection_with_token(
    mut stream: std::net::TcpStream,
    expected_token: &str,
    config: &IngestConfig,
) -> Result<()> {
    use std::io::Read;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

    let Some(header) =
        crate::ingest_http::read_header(&mut stream, crate::ingest_http::HEADER_TIMEOUT)?
    else {
        return Ok(());
    };
    // The dev-page SDK's routes check their own access (loopback pages, or the token).
    if crate::web::handle(&mut stream, &header, expected_token)? {
        return Ok(());
    }
    let authorized = bearer_token(&header)
        .is_some_and(|provided| constant_time_eq(provided, expected_token));
    if !authorized {
        http_unauthorized(&mut stream);
        return Ok(());
    }

    let first_line = header.lines().next().unwrap_or("");
    if first_line.starts_with("GET ") {
        http_json(&mut stream, "200 OK", &serde_json::json!({ "ok": true }));
        return Ok(());
    }
    ensure!(
        first_line.starts_with("POST /ingest"),
        "Unsupported route: {first_line}"
    );
    let content_len: usize = header
        .lines()
        .find_map(|line| {
            let (k, v) = line.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse().ok())
                .flatten()
        })
        .context("Missing Content-Length")?;
    ensure!(
        content_len > 4 && content_len <= 250 * 1024 * 1024,
        "Invalid payload length"
    );
    let mut payload = vec![0_u8; content_len];
    stream.read_exact(&mut payload)?;

    let json_len = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    ensure!(4 + json_len < payload.len(), "Invalid framed payload");
    let json_bytes = &payload[4..4 + json_len];
    let timeline: Timeline = serde_json::from_slice(json_bytes).unwrap_or_default();
    let staging = std::env::temp_dir().join(format!("scrawl-ingest-{}", crate::make_bundle_id()));
    std::fs::create_dir_all(&staging)?;
    let timeline_path = staging.join("timeline.json");
    let movie_path = staging.join("recording.mp4");
    std::fs::write(&timeline_path, json_bytes)?;
    std::fs::write(&movie_path, &payload[4 + json_len..])?;
    drop(payload);

    // Answer once the bundle is built and sent (or the wait runs out), so the phone says what
    // actually happened.
    let (outcome, title) = ingest_recording(
        staging,
        movie_path,
        timeline_path,
        &timeline,
        config,
        reply_wait(&header),
    );
    let body = crate::delivery::reply_json(&outcome, &title);
    scrawl_log(format!("ingest http: replied {body}"));
    let status = if matches!(outcome, Outcome::Failed(_)) {
        "500 Internal Server Error"
    } else {
        "200 OK"
    };
    http_json(&mut stream, status, &body);
    let _ = stream.shutdown(std::net::Shutdown::Both);
    Ok(())
}

/// How long the phone will wait for the answer: `X-Scrawl-Wait` seconds, at most
/// `REPLY_WAIT_MAX`.
fn reply_wait(header: &str) -> Duration {
    header
        .lines()
        .skip(1)
        .find_map(|line| {
            let (k, v) = line.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("x-scrawl-wait")
                .then(|| v.trim().parse::<u64>().ok())
                .flatten()
        })
        .map_or(REPLY_WAIT_DEFAULT, |secs| {
            Duration::from_secs(secs).min(REPLY_WAIT_MAX)
        })
}

fn devicectl_copy(
    udid: &str,
    bundle_id: &str,
    direction: &str,
    source: &str,
    destination: &str,
) -> bool {
    Command::new("xcrun")
        .args([
            "devicectl",
            "device",
            "copy",
            direction,
            "--device",
            udid,
            "--domain-type",
            "appDataContainer",
            "--domain-identifier",
            bundle_id,
            "--source",
            source,
            "--destination",
            destination,
        ])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Cabled recordings whose bundle failed to build: how many times, and when to try again.
static USB_FAILURES: Mutex<BTreeMap<String, (u32, Instant)>> = Mutex::new(BTreeMap::new());

fn usb_backing_off(key: &str) -> bool {
    USB_FAILURES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(key)
        .is_some_and(|&(_, next)| Instant::now() < next)
}

/// Notes a failed build and returns how long until the next try (1 min doubling to 30 min).
fn usb_failed(key: &str) -> Duration {
    let mut failures = USB_FAILURES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let count = failures.get(key).map_or(1, |&(n, _)| n + 1);
    let wait = Duration::from_secs((60_u64 << (count - 1).min(5)).min(30 * 60));
    failures.insert(key.to_owned(), (count, Instant::now() + wait));
    wait
}

/// Takes in recordings the phone marked `.ready` but has no `.sent` for (in `Documents/Scrawl` or
/// the legacy `Documents/Tether` directory). Once the bundle is built and sent (or 15 s pass),
/// writes back `.delivery` (the same JSON an HTTP upload gets) and then `.sent` (the session's
/// name, all that ScrawlKit builds before `.delivery` read).
fn poll_usb_app_containers(apps: &[String], config: &IngestConfig) -> Result<()> {
    if apps.is_empty() {
        return Ok(());
    }
    let list = Command::new("xcrun")
        .args(["devicectl", "list", "devices"])
        .output()?;
    let list = String::from_utf8_lossy(&list.stdout);
    let Some(udid) = list.lines().find_map(|line| {
        (line.contains("physical")
            && line.contains("available (paired)")
            && !line.contains("unavailable")
            && line.contains("iPhone"))
        .then(|| {
            line.split_whitespace()
                .find(|tok| {
                    tok.len() >= 20 && tok.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
                })
                .map(str::to_owned)
        })
        .flatten()
    }) else {
        return Ok(());
    };

    for bundle_id in apps {
        for subdir in ["Documents/Scrawl", "Documents/Tether"] {
            let files = Command::new("xcrun")
                .args([
                    "devicectl",
                    "device",
                    "info",
                    "files",
                    "--device",
                    &udid,
                    "--domain-type",
                    "appDataContainer",
                    "--domain-identifier",
                    bundle_id,
                    "--subdirectory",
                    subdir,
                ])
                .output()?;
            if !files.status.success() {
                continue;
            }
            let files = String::from_utf8_lossy(&files.stdout);
            let names: Vec<&str> = files
                .lines()
                .filter_map(|l| l.split_whitespace().next())
                .collect();
            for stem in names.iter().filter_map(|n| n.strip_suffix(".ready")) {
                if names.contains(&format!("{stem}.sent").as_str()) {
                    continue;
                }
                let retry_key = format!("{bundle_id}/{subdir}/{stem}");
                if usb_backing_off(&retry_key) {
                    continue;
                }
                let staging =
                    std::env::temp_dir().join(format!("scrawl-usb-{}", crate::make_bundle_id()));
                std::fs::create_dir_all(&staging)?;
                let local_json = staging.join(format!("{stem}.json"));
                let local_mp4 = staging.join(format!("{stem}.mp4"));
                let remote = |ext: &str| format!("{subdir}/{stem}.{ext}");
                if !devicectl_copy(
                    &udid,
                    bundle_id,
                    "from",
                    &remote("json"),
                    &local_json.to_string_lossy(),
                ) {
                    let _ = std::fs::remove_dir_all(&staging);
                    continue;
                }
                let timeline: Timeline = std::fs::read(&local_json)
                    .ok()
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or_default();
                if !devicectl_copy(
                    &udid,
                    bundle_id,
                    "from",
                    &remote("mp4"),
                    &local_mp4.to_string_lossy(),
                ) {
                    let _ = std::fs::remove_dir_all(&staging);
                    continue;
                }
                let (outcome, title) = ingest_recording(
                    staging,
                    local_mp4,
                    local_json,
                    &timeline,
                    config,
                    USB_REPLY_WAIT,
                );
                if let Outcome::Failed(err) = &outcome {
                    // No `.sent`: the phone keeps `.ready`, and this is tried again later.
                    let wait = usb_failed(&retry_key);
                    scrawl_log(format!(
                        "ingest usb {stem}: {err}; trying again in {} s",
                        wait.as_secs()
                    ));
                    continue;
                }
                USB_FAILURES
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&retry_key);
                // `.delivery` (the HTTP reply) goes first: ScrawlKit reads it once `.sent`
                // appears. Builds from before it only read `.sent`, the session's name.
                let replies =
                    std::env::temp_dir().join(format!("scrawl-usb-{}", crate::make_bundle_id()));
                std::fs::create_dir_all(&replies)?;
                let delivery = replies.join(format!("{stem}.delivery"));
                let sent = replies.join(format!("{stem}.sent"));
                std::fs::write(
                    &delivery,
                    crate::delivery::reply_json(&outcome, &title).to_string(),
                )?;
                std::fs::write(&sent, title.as_bytes())?;
                devicectl_copy(
                    &udid,
                    bundle_id,
                    "to",
                    &delivery.to_string_lossy(),
                    &remote("delivery"),
                );
                devicectl_copy(
                    &udid,
                    bundle_id,
                    "to",
                    &sent.to_string_lossy(),
                    &remote("sent"),
                );
                scrawl_log(format!("ingest usb {stem}: replied {outcome:?}"));
                let _ = std::fs::remove_dir_all(&replies);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(t: f64, id: i64, x: f64, y: f64, phase: &str) -> Touch {
        Touch {
            t,
            id,
            x,
            y,
            phase: phase.into(),
            label: None,
        }
    }

    #[test]
    fn touches_become_taps_swipes_and_long_presses() {
        let screen = Screen {
            width: 400.0,
            height: 800.0,
            scale: 3.0,
        };
        let events = touch_events(
            &[
                touch(1.0, 1, 200.0, 400.0, "down"),
                touch(1.08, 1, 202.0, 401.0, "up"),
                touch(2.0, 2, 200.0, 700.0, "down"),
                touch(2.1, 2, 200.0, 500.0, "move"),
                touch(2.2, 2, 200.0, 200.0, "up"),
                touch(3.0, 3, 100.0, 100.0, "down"),
                touch(3.9, 3, 101.0, 100.0, "up"),
                // A touch the system took over still ends.
                touch(5.0, 4, 300.0, 400.0, "down"),
                touch(5.1, 4, 380.0, 400.0, "cancel"),
            ],
            screen,
        );
        assert_eq!(events.len(), 4);
        assert!(matches!(
            &events[0],
            InteractionEvent::Click { t: 1000, x, y, button: Some(b), .. }
                if *x == 0.5 && *y == 0.5 && b == "touch"
        ));
        match &events[1] {
            InteractionEvent::Drag {
                t,
                t1,
                y0,
                y1,
                path,
                ..
            } => {
                assert_eq!((*t, *t1), (2000, 2200));
                assert!(*y0 > 0.85 && *y1 < 0.3);
                assert_eq!(path.len(), 3);
            }
            other => panic!("expected a swipe, got {other:?}"),
        }
        assert!(matches!(
            &events[2],
            InteractionEvent::Drag { label: Some(l), .. } if l == "long press"
        ));
        assert!(matches!(
            &events[3],
            InteractionEvent::Drag { t1: 5100, .. }
        ));
    }

    #[test]
    fn timeline_parses_scrawlkit_output() {
        let json = r#"{"app":"dev.kitlangton.UpDown","version":"42","device":"iPhone","system":"27.0",
            "screen":{"width":440,"height":956,"scale":3},"recordedAt":"2026-10-13T16:40:00Z",
            "duration":12.5,"markers":[3.2],"events":[{"t":4.1,"label":"Done tapped"}],
            "touches":[{"t":4.0,"id":7,"x":220,"y":900,"phase":"down"},{"t":4.05,"id":7,"x":220,"y":900,"phase":"up"}]}"#;
        let timeline: Timeline = serde_json::from_str(json).unwrap();
        assert_eq!(timeline.markers, vec![3.2]);
        assert_eq!(timeline.events[0].label, "Done tapped");
        let events = touch_events(&timeline.touches, timeline.screen.unwrap());
        assert!(matches!(events[0], InteractionEvent::Click { t: 4000, .. }));
        // Timelines from before app context still parse, with none.
        assert!(timeline.short_version.is_none());
        assert!(timeline.context().is_empty() && timeline.context_changes().is_empty());
        assert!(timeline.screen_spans(12_500).is_empty());
    }

    #[test]
    fn timeline_parses_app_context_and_screens() {
        let json = r#"{"app":"OpenCode","bundleId":"ai.opencode.mobilecode","version":"354",
            "shortVersion":"1.4","duration":20,
            "context":{"server":"studio","session":"ses_1","build":7,"gone":null},
            "contextChanges":[{"t":9.5,"key":"session","value":null},{"t":4.25,"key":"session","value":"ses_2"}],
            "screens":[{"t":0,"name":"Sessions"},{"t":4.2,"name":"Settings › Experimental"},
                {"t":12,"name":null},{"t":15,"name":"Sessions"}]}"#;
        let timeline: Timeline = serde_json::from_str(json).unwrap();
        assert_eq!(timeline.short_version.as_deref(), Some("1.4"));
        assert_eq!(
            timeline.context(),
            BTreeMap::from([
                ("build".to_owned(), "7".to_owned()),
                ("server".to_owned(), "studio".to_owned()),
                ("session".to_owned(), "ses_1".to_owned()),
            ])
        );
        assert_eq!(
            timeline.context_changes(),
            vec![
                ContextChange {
                    t: 4_250,
                    key: "session".into(),
                    value: Some("ses_2".into()),
                },
                ContextChange {
                    t: 9_500,
                    key: "session".into(),
                    value: None,
                },
            ]
        );
        let span = |t0, t1, name: &str| ScreenSpan {
            t0,
            t1,
            name: name.into(),
        };
        assert_eq!(
            timeline.screen_spans(20_000),
            vec![
                span(0, 4_200, "Sessions"),
                span(4_200, 12_000, "Settings › Experimental"),
                span(15_000, 20_000, "Sessions"),
            ]
        );
    }

    #[test]
    fn phone_target_keeps_the_bundle_id_apart_from_the_display_name() {
        let target = phone_target(
            "iPhone",
            Some("OpenCode"),
            Some("ai.opencode.mobilecode"),
            10,
            20,
        );
        assert_eq!(target.app.as_deref(), Some("OpenCode"));
        assert_eq!(target.bundle_id.as_deref(), Some("ai.opencode.mobilecode"));
        // Older timelines only had the name.
        let old = phone_target("iPhone", Some("UpDown"), Some(""), 10, 20);
        assert_eq!(old.app.as_deref(), Some("UpDown"));
        assert_eq!(old.bundle_id.as_deref(), Some("UpDown"));
        let mirror = phone_target("Kit's iPhone", None, None, 10, 20);
        assert_eq!(mirror.app.as_deref(), Some("Kit's iPhone"));
        assert_eq!(mirror.bundle_id, None);
    }

    #[test]
    fn import_pairs_movie_and_timeline_by_name() {
        let dir = std::env::temp_dir().join(crate::make_bundle_id());
        std::fs::create_dir_all(&dir).unwrap();
        let movie = dir.join("20261013T164000.mp4");
        let json = dir.join("20261013T164000.json");
        std::fs::write(&movie, b"x").unwrap();
        std::fs::write(&json, b"{}").unwrap();
        assert_eq!(
            resolve_import(Some(&movie), None).unwrap(),
            (movie.clone(), Some(json.clone()))
        );
        assert_eq!(
            resolve_import(Some(&json), None).unwrap(),
            (movie.clone(), Some(json.clone()))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn constant_time_compare_rejects_mismatches_and_empty_secrets() {
        let token = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
        assert!(constant_time_eq(token, token));
        assert!(!constant_time_eq("", token));
        assert!(!constant_time_eq(token, ""));
        assert!(!constant_time_eq("", ""));
        assert!(!constant_time_eq("short", token));
        assert!(!constant_time_eq(
            "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f91",
            token
        ));
        assert!(!constant_time_eq(&format!("{token}extra"), token));
    }

    #[test]
    fn setup_link_formats_and_escapes_urls_token_and_session() {
        let urls = vec![
            "http://100.64.0.12:47777/".to_owned(),
            "http://192.168.1.20:47777".to_owned(),
            "http://100.64.0.12:47777".to_owned(),
        ];
        let link = format_setup_link(
            "updown://",
            &urls,
            "deadbeef0123",
            Some("ses_abc123"),
        )
        .unwrap();
        assert_eq!(
            link,
            "updown://scrawl?url=http%3A%2F%2F100.64.0.12%3A47777&url=http%3A%2F%2F192.168.1.20%3A47777&token=deadbeef0123&session=ses_abc123"
        );

        assert!(format_setup_link("updown", &[], "tok", None).is_err());
        assert!(
            format_setup_link("updown", &["ftp://1.2.3.4:47777".into()], "tok", None)
                .is_err()
        );
        assert!(
            format_setup_link("updown", &["http://1.2.3.4:47777".into()], "", None)
                .is_err()
        );
        assert!(
            format_setup_link("", &["http://1.2.3.4:47777".into()], "tok", None).is_err()
        );
    }

    fn send_raw_http_request(request: &[u8], expected_token: &str) -> String {
        use std::io::Read as _;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let tok = expected_token.to_owned();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let cfg = IngestConfig::default();
            let _ = handle_ingest_connection_with_token(stream, &tok, &cfg);
        });
        let mut client = std::net::TcpStream::connect(addr).unwrap();
        client.write_all(request).unwrap();
        client.flush().unwrap();
        let mut resp = String::new();
        client.read_to_string(&mut resp).unwrap();
        server.join().unwrap();
        resp
    }

    #[test]
    fn ingest_rejects_missing_or_wrong_tokens_with_401_and_accepts_valid_token() {
        let secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        // 1. Health without token -> 401 with empty body
        let no_auth = send_raw_http_request(
            b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n",
            secret,
        );
        assert!(no_auth.starts_with("HTTP/1.1 401 Unauthorized\r\n"));
        assert!(no_auth.ends_with("\r\n\r\n"));

        // 2. Health with wrong token -> 401 with empty body
        let wrong_auth = send_raw_http_request(
            b"GET /health HTTP/1.1\r\nAuthorization: Bearer wrongtoken\r\n\r\n",
            secret,
        );
        assert!(wrong_auth.starts_with("HTTP/1.1 401 Unauthorized\r\n"));
        assert!(wrong_auth.ends_with("\r\n\r\n"));

        // 3. POST /ingest with wrong token -> 401 with empty body (no sessionTitle leaked)
        let wrong_post = send_raw_http_request(
            b"POST /ingest HTTP/1.1\r\nAuthorization: Bearer wrongtoken\r\nContent-Length: 10\r\n\r\n0123456789",
            secret,
        );
        assert!(wrong_post.starts_with("HTTP/1.1 401 Unauthorized\r\n"));
        assert!(!wrong_post.contains("sessionTitle"));
        assert!(wrong_post.ends_with("\r\n\r\n"));

        // 4. Health with valid token -> 200 OK
        let valid_health = send_raw_http_request(
            format!("GET /health HTTP/1.1\r\nAuthorization: Bearer {secret}\r\n\r\n").as_bytes(),
            secret,
        );
        assert!(valid_health.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(valid_health.contains(r#"{"ok":true}"#));
    }

    const SECRET: &str = "feedface0123456789abcdef0123456789abcdef0123456789abcdef01234567";

    /// Builds nothing real: the movie's bytes say how the build goes.
    fn fake_build(
        movie: &Path,
        _timeline: &Path,
        bundle_dir: &Path,
    ) -> Result<(PathBuf, Option<ScrawlBundle>)> {
        let bytes = std::fs::read(movie)?;
        if bytes == b"slow" {
            std::thread::sleep(Duration::from_millis(800));
        }
        ensure!(bytes != b"bad", "no video track");
        std::fs::create_dir_all(bundle_dir)?;
        Ok((bundle_dir.to_owned(), None))
    }

    /// Never reaches OpenCode: `ses_down` fails like an unreachable server.
    fn fake_forward(_: &Path, session: &str) -> Result<()> {
        ensure!(
            session != "ses_down",
            "OpenCode API failed: connection refused"
        );
        Ok(())
    }

    fn fake_resolve(session: Option<&str>, _app: Option<&str>) -> Result<(String, String)> {
        Ok((session.unwrap_or("ses_up").to_owned(), "UP DOWN".to_owned()))
    }

    fn test_config(captures: &Path, route_opencode: bool) -> IngestConfig {
        IngestConfig {
            route_opencode,
            captures: Some(captures.to_owned()),
            hooks: IngestHooks {
                resolve: fake_resolve,
                build: fake_build,
                forward: fake_forward,
            },
            ..IngestConfig::default()
        }
    }

    /// Uploads one framed recording; returns the status line and the JSON body.
    fn post_ingest(
        config: &IngestConfig,
        timeline: serde_json::Value,
        movie: &[u8],
        wait: Option<u64>,
    ) -> (String, serde_json::Value) {
        use std::io::Read as _;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let cfg = config.clone();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_ingest_connection_with_token(stream, SECRET, &cfg).unwrap();
        });
        let json = timeline.to_string();
        let mut payload = (json.len() as u32).to_be_bytes().to_vec();
        payload.extend_from_slice(json.as_bytes());
        payload.extend_from_slice(movie);
        let wait = wait.map_or(String::new(), |s| format!("X-Scrawl-Wait: {s}\r\n"));
        let mut request = format!(
            "POST /ingest HTTP/1.1\r\nAuthorization: Bearer {SECRET}\r\n{wait}Content-Length: {}\r\n\r\n",
            payload.len()
        )
        .into_bytes();
        request.extend_from_slice(&payload);
        let mut client = std::net::TcpStream::connect(addr).unwrap();
        client.write_all(&request).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        server.join().unwrap();
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        (
            head.lines().next().unwrap().to_owned(),
            serde_json::from_str(body).unwrap(),
        )
    }

    fn temp_captures() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("scrawl-ingest-test-{}", crate::make_bundle_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn bundles(captures: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(captures)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect()
    }

    #[test]
    fn ingest_answers_saved_when_routing_is_off() {
        let captures = temp_captures();
        let (status, body) = post_ingest(
            &test_config(&captures, false),
            serde_json::json!({"recordedAt": "2026-10-18T10:00:00Z-saved"}),
            b"movie",
            None,
        );
        assert_eq!(status, "HTTP/1.1 200 OK");
        assert_eq!(
            body,
            serde_json::json!({"ok": true, "delivered": false, "state": "saved", "sessionTitle": "Mac"})
        );
        assert_eq!(bundles(&captures).len(), 1);
        std::fs::remove_dir_all(captures).unwrap();
    }

    #[test]
    fn ingest_answers_sent_only_after_opencode_took_it() {
        let captures = temp_captures();
        let (status, body) = post_ingest(
            &test_config(&captures, true),
            serde_json::json!({"recordedAt": "2026-10-18T10:00:00Z-sent", "session": "ses_up"}),
            b"movie",
            None,
        );
        assert_eq!(status, "HTTP/1.1 200 OK");
        assert_eq!(
            body,
            serde_json::json!({"ok": true, "delivered": true, "state": "sent", "sessionTitle": "UP DOWN"})
        );
        std::fs::remove_dir_all(captures).unwrap();
    }

    #[test]
    fn a_failed_forward_is_queued_on_the_mac_and_retried() {
        let captures = temp_captures();
        let config = test_config(&captures, true);
        let timeline =
            serde_json::json!({"recordedAt": "2026-10-18T10:00:00Z-queued", "session": "ses_down"});
        let (status, body) = post_ingest(&config, timeline.clone(), b"movie", None);
        assert_eq!(status, "HTTP/1.1 200 OK");
        assert_eq!(body["delivered"], false);
        assert_eq!(body["state"], "queued");
        assert_eq!(body["sessionTitle"], "UP DOWN");
        assert_eq!(body["error"], "OpenCode API failed: connection refused");
        let bundle = bundles(&captures).pop().unwrap();
        let pending: crate::delivery::Pending =
            serde_json::from_slice(&std::fs::read(bundle.join(crate::delivery::PENDING)).unwrap())
                .unwrap();
        assert_eq!(pending.session, "ses_down");
        assert_eq!(pending.attempts, 1);

        // OpenCode is back (the session answers now): the Mac's retry goes through.
        let report = crate::delivery::retry_due(&captures, u64::MAX / 2, |dir, _| {
            fake_forward(dir, "ses_up")
        });
        assert_eq!(report.sent.len(), 1);
        assert!(!bundle.join(crate::delivery::PENDING).exists());
        for (dir, pending) in &report.sent {
            mark_sent(dir, pending);
        }
        // The phone sending it again (a lost reply) hears it was sent, and nothing is rebuilt.
        let (_, again) = post_ingest(&config, timeline, b"movie", None);
        assert_eq!(again["state"], "sent");
        assert_eq!(bundles(&captures).len(), 1);
        std::fs::remove_dir_all(captures).unwrap();
    }

    #[test]
    fn a_failed_build_answers_500_and_the_upload_can_be_sent_again() {
        let captures = temp_captures();
        let config = test_config(&captures, true);
        let timeline = serde_json::json!({"recordedAt": "2026-10-18T10:00:00Z-bad"});
        let (status, body) = post_ingest(&config, timeline.clone(), b"bad", None);
        assert_eq!(status, "HTTP/1.1 500 Internal Server Error");
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"], "no video track");
        let (status, body) = post_ingest(&config, timeline, b"movie", None);
        assert_eq!(status, "HTTP/1.1 200 OK");
        assert_eq!(body["state"], "sent");
        std::fs::remove_dir_all(captures).unwrap();
    }

    #[test]
    fn a_slow_build_answers_processing_and_a_copy_waits_for_the_real_outcome() {
        let captures = temp_captures();
        let config = test_config(&captures, true);
        let timeline = serde_json::json!({"recordedAt": "2026-10-18T10:00:00Z-slow"});
        let (status, body) = post_ingest(&config, timeline.clone(), b"slow", Some(0));
        assert_eq!(status, "HTTP/1.1 200 OK");
        assert_eq!(
            body,
            serde_json::json!({"ok": true, "delivered": false, "state": "processing", "sessionTitle": "UP DOWN"})
        );
        // The same recording over the other address waits for the first build.
        let (_, body) = post_ingest(&config, timeline, b"slow", Some(10));
        assert_eq!(body["state"], "sent");
        assert_eq!(bundles(&captures).len(), 1);
        std::fs::remove_dir_all(captures).unwrap();
    }

    #[test]
    fn reply_wait_reads_the_phones_header_and_caps_it() {
        assert_eq!(reply_wait("POST /ingest HTTP/1.1\r\n"), REPLY_WAIT_DEFAULT);
        assert_eq!(
            reply_wait("POST /ingest HTTP/1.1\r\nx-scrawl-wait: 10\r\n"),
            Duration::from_secs(10)
        );
        assert_eq!(
            reply_wait("POST /ingest HTTP/1.1\r\nX-Scrawl-Wait: 600\r\n"),
            REPLY_WAIT_MAX
        );
    }
}
