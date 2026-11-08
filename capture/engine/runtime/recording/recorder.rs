//! Candidate data and screen/media session lifecycle, independent of capture telemetry.
//! The native sampler and retention guard still belong to the root application.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::super::{MAX_CAPTURE_SCALE, platform};
use crate::shared::model::{RgbaFrame, TargetInfo};

pub const FRAME_SAMPLE_INTERVAL: Duration = Duration::from_millis(220);
/// Candidate frames only feed scene diffs and keyframe selection; the keyframes themselves are
/// decoded from `recording.mp4` at full size afterwards.
pub const CANDIDATE_EDGE: f64 = 480.0;
/// Without a recording to decode from, the candidates are the keyframes, so keep them sharper.
pub(crate) const CANDIDATE_EDGE_NO_VIDEO: f64 = 1280.0;
/// The candidates' memory ceiling, however long the capture runs.
pub const CANDIDATE_BUDGET_BYTES: usize = 384 << 20;

/// The candidate frames of a capture, bounded in memory. When full it drops every other frame
/// (keeping the first and the ones taken for a click, stroke or shortcut) and samples half as
/// often from then on, so a long capture keeps its whole timeline, evenly, at a coarser step.
pub struct CandidateBuffer {
    frames: Vec<(RgbaFrame, bool)>,
    cap: usize,
    budget: usize,
    interval: Duration,
}

impl CandidateBuffer {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            frames: Vec::new(),
            cap: 0,
            budget: budget_bytes,
            interval: FRAME_SAMPLE_INTERVAL,
        }
    }

    /// How often to sample now.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn push(&mut self, frame: RgbaFrame, urgent: bool) {
        if self.cap == 0 {
            self.cap = (self.budget / frame.rgba.len().max(1)).clamp(60, 1_200);
        }
        self.frames.push((frame, urgent));
        if self.frames.len() >= self.cap {
            self.thin();
        }
    }

    fn thin(&mut self) {
        let mut regular = 0_usize;
        let mut kept: Vec<(RgbaFrame, bool)> = std::mem::take(&mut self.frames)
            .into_iter()
            .enumerate()
            .filter(|(i, (_, urgent))| {
                let keep = *i == 0 || *urgent || regular.is_multiple_of(2);
                if !urgent {
                    regular += 1;
                }
                keep
            })
            .map(|(_, f)| f)
            .collect();
        // Mostly urgent frames (a storm of clicks): thin those too.
        if kept.len() > self.cap * 3 / 4 {
            kept = kept
                .into_iter()
                .enumerate()
                .filter(|(i, _)| i.is_multiple_of(2))
                .map(|(_, f)| f)
                .collect();
        }
        self.frames = kept;
        self.interval *= 2;
    }

    /// Forgets frames taken after `t_ms`, except the first.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn drop_since(&mut self, t_ms: u64) {
        let mut index = 0;
        self.frames.retain(|(frame, _)| {
            index += 1;
            index == 1 || frame.t_ms <= t_ms
        });
    }

    pub fn into_frames(self) -> Vec<RgbaFrame> {
        self.frames.into_iter().map(|(f, _)| f).collect()
    }
}

/// What a finished screen capture leaves behind.
pub struct ScreenRecording {
    /// Small candidate frames for keyframe selection (see `CANDIDATE_EDGE`).
    pub frames: Vec<RgbaFrame>,
    pub video: Option<PathBuf>,
    pub duration_ms: u64,
    /// The capture time (on the frames' clock) of the video's start.
    pub video_offset_ms: u64,
}

/// The capture size for `target` with its long edge at most `long_edge` px (and at most its
/// display's pixels: a 1x display isn't upscaled, and on Windows frames are pixels already,
/// `MAX_CAPTURE_SCALE`).
pub fn capture_size(target: &TargetInfo, long_edge: f64) -> (u32, u32) {
    let max_dim = target.frame.width.max(target.frame.height).max(1.0);
    let native = target.frame.scale.clamp(1.0, MAX_CAPTURE_SCALE);
    let scale = (long_edge / max_dim).min(native);
    (
        ((target.frame.width * scale).round() as u32).max(48) & !1,
        ((target.frame.height * scale).round() as u32).max(48) & !1,
    )
}

/// What the platform's sampler thread records into. It runs until `stopping`, pushes candidate
/// frames (at once when `urgent`), stores when the movie started into `video_offset_ms`, and
/// returns the movie it wrote.
pub(crate) struct Sampler {
    pub target: TargetInfo,
    pub video_output_path: Option<PathBuf>,
    /// Apparent's own windows to leave out of the recording.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub excluded_window_ids: Arc<Mutex<Vec<u32>>>,
    pub started_at: Instant,
    pub stopping: Arc<AtomicBool>,
    pub urgent: Arc<AtomicBool>,
    pub frames: Arc<Mutex<CandidateBuffer>>,
    pub real_frame_seen: Arc<AtomicBool>,
    /// A terminal producer error, independent of worker lifetime (cleanup may still run).
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub startup_failed: Arc<AtomicBool>,
    pub video_offset_ms: Arc<AtomicU64>,
}

/// Synthetic fallback pixels can be kept for diagnostics, but never unlock agent Stop.
pub(crate) fn push_candidate(
    frames: &Mutex<CandidateBuffer>,
    real_frame_seen: &AtomicBool,
    frame: RgbaFrame,
    urgent: bool,
    real: bool,
) {
    frames
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(frame, urgent);
    if real {
        real_frame_seen.store(true, Ordering::Release);
    }
}

/// Attempt one native screenshot without bypassing a terminal setup failure.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn sample_real_candidate(
    frames: &Mutex<CandidateBuffer>,
    real_frame_seen: &AtomicBool,
    startup_failed: &AtomicBool,
    urgent: bool,
    sample: impl FnOnce() -> std::result::Result<RgbaFrame, String>,
) {
    if startup_failed.load(Ordering::Acquire) {
        return;
    }
    // Individual acquisition/decode errors are not classified terminal source errors.
    // Skip this sample and let the sampler try again, including before its first frame.
    if let Ok(frame) = sample() {
        push_candidate(frames, real_frame_seen, frame, urgent, true);
    }
}

/// Active screen + optional MP4 recorder (ScreenCaptureKit on macOS, Windows.Graphics.Capture
/// and Media Foundation on Windows).
pub struct ScreenCaptureSession {
    started_at: Instant,
    stopping: Arc<AtomicBool>,
    urgent_snapshot: Arc<AtomicBool>,
    frames: Arc<Mutex<CandidateBuffer>>,
    real_frame_seen: Arc<AtomicBool>,
    startup_failed: Arc<AtomicBool>,
    video_offset_ms: Arc<AtomicU64>,
    sampler_thread: Option<JoinHandle<Option<PathBuf>>>,
    _recording: crate::capture::engine::bundle::retention::Recording,
}

impl ScreenCaptureSession {
    pub fn start(
        target: TargetInfo,
        video_output_path: Option<PathBuf>,
        excluded_window_ids: Arc<Mutex<Vec<u32>>>,
    ) -> anyhow::Result<Self> {
        super::super::ensure_recording_mode(target.mode)?;
        let started_at = Instant::now();
        let stopping = Arc::new(AtomicBool::new(false));
        let urgent_snapshot = Arc::new(AtomicBool::new(true));
        let frames = Arc::new(Mutex::new(CandidateBuffer::new(CANDIDATE_BUDGET_BYTES)));
        let real_frame_seen = Arc::new(AtomicBool::new(false));
        let startup_failed = Arc::new(AtomicBool::new(false));
        let video_offset_ms = Arc::new(AtomicU64::new(0));

        let thread_stopping = Arc::clone(&stopping);
        let thread_urgent = Arc::clone(&urgent_snapshot);
        let thread_frames = Arc::clone(&frames);
        let thread_real_frame_seen = Arc::clone(&real_frame_seen);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let thread_startup_failed = Arc::clone(&startup_failed);
        let thread_offset = Arc::clone(&video_offset_ms);
        let sampler_target = target;
        let sampler_thread = thread::Builder::new()
            .name("apparent-frame-sampler".into())
            .spawn(move || {
                platform::run_sampler(Sampler {
                    target: sampler_target,
                    video_output_path,
                    excluded_window_ids,
                    started_at,
                    stopping: thread_stopping,
                    urgent: thread_urgent,
                    frames: thread_frames,
                    real_frame_seen: thread_real_frame_seen,
                    #[cfg(any(target_os = "macos", target_os = "linux"))]
                    startup_failed: thread_startup_failed,
                    video_offset_ms: thread_offset,
                })
            })
            .ok();

        Ok(Self {
            started_at,
            stopping,
            urgent_snapshot,
            frames,
            real_frame_seen,
            startup_failed,
            video_offset_ms,
            sampler_thread,
            _recording: crate::capture::engine::bundle::retention::Recording::start(),
        })
    }

    pub fn request_immediate_frame(&self) {
        self.urgent_snapshot.store(true, Ordering::Release);
        if let Some(ref handle) = self.sampler_thread {
            handle.thread().unpark();
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    /// A portal or ScreenCaptureKit session is still pending until a real frame arrives.
    pub fn has_frames(&self) -> bool {
        self.real_frame_seen.load(Ordering::Acquire)
    }

    /// A known terminal error is failure even while the worker is alive or cleaning up.
    /// A pending portal reply alone never sets this producer flag.
    pub fn failed_to_start(&self) -> bool {
        !self.has_frames()
            && (self.startup_failed.load(Ordering::Acquire)
                || self.sampler_thread.as_ref().is_none_or(JoinHandle::is_finished))
    }

    pub fn stop(mut self) -> ScreenRecording {
        let duration_ms = self.elapsed_ms();
        self.stopping.store(true, Ordering::Release);
        let video_path = self
            .sampler_thread
            .take()
            .and_then(|handle| {
                handle.thread().unpark();
                handle.join().ok()
            })
            .flatten();
        let buffer = std::mem::replace(
            &mut *self
                .frames
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            CandidateBuffer::new(0),
        );
        let collected = buffer.into_frames();
        let valid_video =
            video_path.filter(|path| path.exists() && path.metadata().is_ok_and(|m| m.len() > 0));
        ScreenRecording {
            frames: collected,
            video: valid_video,
            duration_ms: duration_ms.max(100),
            video_offset_ms: self.video_offset_ms.load(Ordering::Acquire),
        }
    }
}

impl Drop for ScreenCaptureSession {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(handle) = self.sampler_thread.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests;
