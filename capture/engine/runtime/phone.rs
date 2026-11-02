//! Live cabled iPhone capture mirrored through `apparent-helper`. Keyframes are decoded
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

use crate::capture::desktop::helper::{self, FrameTimes, PhoneDevice};
use crate::capture::engine::runtime::{LiveOverlayEvent, SharedCaptureTelemetry, apparent_log};
use crate::shared::model::{
    ApparentBundle, CaptureMode, ContextChange, DeviceInfo, InteractionEvent, RgbaFrame,
    ScreenSpan, Stroke, TargetFrame, TargetInfo,
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
    busy: Option<crate::capture::engine::bundle::retention::Recording>,
}

pub struct PhoneRecording {
    pub movie: PathBuf,
    pub device: PhoneDevice,
    /// How long after Apparent's session clock started the movie's first frame arrived.
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
        let session_epoch_ms = crate::shared::clock::now_epoch_ms();
        let recording =
            movie.map(|_| crate::capture::engine::bundle::retention::Recording::start());
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
        if let Some(choice) = crate::capture::engine::runtime::audio::mic::choice() {
            command.args([
                "--microphone",
                &choice.id,
                "--microphone-name",
                &choice.name,
            ]);
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
                .name("apparent-phone-stderr".into())
                .spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            apparent_log(trimmed.to_owned());
                        }
                    }
                });
        }
        let state = Arc::new(Mutex::new(MirrorState::default()));
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader_state = Arc::clone(&state);
        let reader_cancelled = Arc::clone(&cancelled);
        let reader = std::thread::Builder::new()
            .name("apparent-phone".into())
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
                            apparent_log(format!("iPhone mirror: {message}"));
                        }
                        MirrorEvent::Error { message } => {
                            apparent_log(format!("iPhone mirror: {message}"));
                            state.error = Some(message);
                            state.ended = true;
                        }
                    }
                }
                reader_state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .ended = true;
                // The helper ended; if Apparent didn't ask (phone unplugged, no phone, denied),
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
        self.session_epoch_ms = crate::shared::clock::now_epoch_ms();
        self.busy
            .get_or_insert_with(crate::capture::engine::bundle::retention::Recording::start);
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
    /// Additional context carried by the capture.
    pub context: BTreeMap<String, String>,
    pub context_changes: Vec<ContextChange>,
    pub screens: Vec<ScreenSpan>,
}

/// The mirror supplies the phone name; app and bundle ID are optional metadata.
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
) -> Result<(PathBuf, ApparentBundle, String)> {
    let started_at = Instant::now();
    let wav = bundle_dir.join("audio.wav");
    let mut mark_times: Vec<u64> = crate::capture::engine::sketch::marks_of(&capture.strokes)
        .into_iter()
        .map(|m| m.t1)
        .chain(capture.flags.iter().copied())
        .collect();
    mark_times.sort_unstable();
    mark_times.dedup();
    let ((transcript, audio_ms, transcribe_ms, audio_err), joined) = std::thread::scope(|scope| {
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
            let budget = crate::shared::model::keyframe_budget(probe.duration_ms);
            // With a mark for every slot, only the frames under the marks are needed, unless
            // marks on one screen share a keyframe and leave slots for other moments.
            if mark_times.len() >= budget {
                let direct =
                    helper::frames(movie_ref, FrameTimes::At(&mark_times), PHONE_FRAME_EDGE);
                let decode_ms = t0.elapsed().as_millis() as u64;
                if let Ok(raw_frames) = direct
                    && !raw_frames.is_empty()
                {
                    let mut preview_marks = crate::capture::engine::sketch::marks_of(strokes_ref);
                    for (i, &t) in flags_ref.iter().enumerate() {
                        preview_marks.push(crate::shared::model::Mark {
                            id: format!("f{}", i + 1),
                            strokes: Vec::new(),
                            t0: t,
                            t1: t,
                            box_norm: [0.0, 0.0, 1.0, 1.0],
                            shape: crate::shared::model::Shape::Flag,
                            from: None,
                            to: None,
                            pin: None,
                            on: None,
                            description: "flagged this moment".into(),
                            frame: None,
                        });
                    }
                    let preview = crate::capture::engine::keyframes::select_keyframes(
                        &raw_frames,
                        strokes_ref,
                        &mut preview_marks,
                        events_ref,
                        &crate::shared::model::TranscriptData::default(),
                        budget,
                    );
                    if preview.len() == budget
                        && preview
                            .iter()
                            .all(|(k, _)| k.reason == crate::shared::model::KeyframeReason::Mark)
                    {
                        let prep =
                            crate::capture::engine::bundle::bundle_processing::prepare_visuals(
                                preview,
                                raw_frames.first(),
                                None,
                                &target,
                                probe.duration_ms,
                                strokes_ref,
                                &preview_marks,
                                events_ref,
                                &crate::shared::windows::ActivityLog::default(),
                            );
                        return Ok((probe, raw_frames, decode_ms, true, prep));
                    }
                }
            }
            let res = helper::frames(movie_ref, FrameTimes::Every(every), CANDIDATE_EDGE)?;
            Ok((probe, res, t0.elapsed().as_millis() as u64, false, None))
        });
        let audio_started = Instant::now();
        let exported = helper::export_audio(&capture.movie, &wav).and_then(|()| {
            crate::capture::engine::runtime::audio::audio::recorded_audio_from_wav(std::fs::read(
                &wav,
            )?)
        });
        let audio_ms = audio_started.elapsed().as_millis() as u64;
        let (audio, audio_err) = match exported {
            Ok(a) => (Some(a), None),
            Err(e) => (None, Some(e)),
        };
        let t0 = Instant::now();
        let transcript =
            crate::capture::engine::runtime::audio::transcription::transcribe::transcribe(
                audio.as_ref().map(|_| wav.as_path()),
                audio.as_ref(),
            );
        let transcribe_ms = t0.elapsed().as_millis() as u64;
        (
            (transcript, audio_ms, transcribe_ms, audio_err),
            frames.join(),
        )
    });
    let (probe, frames, candidates_ms, already_full, prepared) =
        joined.map_err(|_| anyhow::anyhow!("Frame decoding panicked"))??;
    if probe.audio_tracks > 0
        && let Some(error) = audio_err
    {
        apparent_log(format!("Phone audio export failed: {error:#}"));
    }
    if probe.width > 0 && probe.height > 0 {
        capture.device.width = probe.width;
        capture.device.height = probe.height;
        capture.target.frame.width = f64::from(probe.width);
        capture.target.frame.height = f64::from(probe.height);
    }
    ensure!(!frames.is_empty(), "No frames could be decoded");
    let movie = capture.movie.clone();
    let extras = crate::capture::engine::bundle::bundle_processing::SourceExtras {
        source: Some("iphone".into()),
        device: Some(capture.device),
        context: capture.context,
        context_changes: capture.context_changes,
        screens: capture.screens,
        flags: capture.flags,
        full_frames: (!already_full).then(
            || -> crate::capture::engine::bundle::bundle_processing::FullFrames {
                Box::new(move |times: &[u64]| -> Result<Vec<RgbaFrame>> {
                    helper::frames(&movie, FrameTimes::At(times), PHONE_FRAME_EDGE)
                })
            },
        ),
        prepared,
        timings: crate::capture::engine::bundle::bundle_processing::StageTimings {
            started_at: Some(started_at),
            audio_ms: Some(audio_ms),
            candidates_ms: Some(candidates_ms),
            transcribe_ms: Some(transcribe_ms),
            ..Default::default()
        },
        ..Default::default()
    };
    crate::capture::engine::bundle::bundle_processing::build_bundle_from_parts(
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

/// Finishes a live capture: re-bases Apparent's pointer and pen input onto the movie's clock.
pub fn finalize_live_capture(
    bundle_dir: &Path,
    recording: PhoneRecording,
    mut events: Vec<InteractionEvent>,
    mut strokes: Vec<Stroke>,
    note: Option<String>,
) -> Result<(PathBuf, ApparentBundle, String)> {
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
    Ok(built)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_target_has_device_identity_without_app_bundle_id() {
        let target = phone_target("iPhone", None, None, 1320, 2868);
        assert_eq!(target.mode, CaptureMode::Device);
        assert_eq!(target.app.as_deref(), Some("iPhone"));
        assert_eq!(target.bundle_id, None);
        assert_eq!(target.frame.width, 1320.0);
    }
}
