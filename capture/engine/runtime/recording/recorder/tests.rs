use crate::capture::engine::runtime::{
    push_candidate, CandidateBuffer, ScreenCaptureSession, FRAME_SAMPLE_INTERVAL,
};
use crate::shared::model::{CaptureMode, RgbaFrame, TargetFrame, TargetInfo};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
impl ScreenCaptureSession {
    /// Controlled producer for coordinator lifecycle tests; no native capture or output.
    pub(crate) fn test_pending_start() -> (Self, Arc<AtomicBool>, Arc<AtomicBool>) {
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = Arc::clone(&stopping);
        let failed = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(AtomicBool::new(false));
        let worker = thread::spawn(move || {
            while !worker_stopping.load(Ordering::Acquire) {
                thread::park_timeout(Duration::from_millis(5));
            }
            None
        });
        (Self {
            started_at: Instant::now(), stopping,
            urgent_snapshot: Arc::new(AtomicBool::new(false)),
            frames: Arc::new(Mutex::new(CandidateBuffer::new(1 << 20))),
            real_frame_seen: Arc::clone(&seen), startup_failed: Arc::clone(&failed),
            video_offset_ms: Arc::new(AtomicU64::new(0)), sampler_thread: Some(worker),
            _recording: crate::capture::engine::bundle::retention::Recording::start(),
        }, failed, seen)
    }
}

#[cfg(any(target_os = "macos", test))]
use crate::capture::engine::runtime::sample_real_candidate;

#[test]
fn window_recording_is_refused_before_native_setup_or_output_creation() {
    // This boundary also rejects direct callers that bypass the CLI and picker.
    let target = TargetInfo {
        mode: CaptureMode::Window,
        app: None,
        bundle_id: None,
        window_title: None,
        window_id: None,
        display_id: 0,
        frame: TargetFrame {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            scale: 1.0,
        },
    };
    let output = std::env::temp_dir()
        .join(crate::shared::ids::make_bundle_id())
        .join("recording.mp4");
    for window_id in [None, Some(42)] {
        let mut target = target.clone();
        target.window_id = window_id;
        let error = ScreenCaptureSession::start(
            target,
            Some(output.clone()),
            Arc::new(Mutex::new(vec![])),
        )
        .err()
        .expect("Window recording must fail synchronously, even with a valid window ID");
        assert!(error.to_string().contains("Window recording is temporarily disabled"));
        assert!(!output.parent().unwrap().exists());
    }
}

#[test]
fn synthetic_candidate_does_not_make_agent_capture_ready() {
    let frames = Arc::new(Mutex::new(CandidateBuffer::new(1 << 20)));
    let seen = Arc::new(AtomicBool::new(false));
    push_candidate(
        &frames,
        &seen,
        crate::capture::engine::render::render_synthetic_demo_frame(1280, 800, 0, 0),
        false,
        false,
    );
    let session = ScreenCaptureSession {
        started_at: Instant::now(),
        stopping: Arc::new(AtomicBool::new(false)),
        urgent_snapshot: Arc::new(AtomicBool::new(false)),
        frames,
        real_frame_seen: seen,
        startup_failed: Arc::new(AtomicBool::new(false)),
        video_offset_ms: Arc::new(AtomicU64::new(0)),
        sampler_thread: None,
        _recording: crate::capture::engine::bundle::retention::Recording::start(),
    };
    assert!(
        !session.has_frames(),
        "synthetic pixels are not a real capture"
    );
    assert!(session.failed_to_start(), "no sampler and no real frames must fail start");
    push_candidate(
        &session.frames,
        &session.real_frame_seen,
        RgbaFrame::new(1, 1, 1, vec![1, 2, 3, 255]),
        false,
        true,
    );
    assert!(session.has_frames(), "real pixels unlock agent Stop");
    assert!(!session.failed_to_start(), "real frames distinguish started from startup failure");
}

#[test]
fn transient_screenshot_failure_retries_to_readiness_and_finalization() {
    for error in ["screenshot acquisition failed", "screenshot has invalid pixel length"] {
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = Arc::clone(&stopping);
        let worker = thread::spawn(move || {
            while !worker_stopping.load(Ordering::Acquire) {
                thread::park_timeout(Duration::from_millis(5));
            }
            None
        });
        let session = ScreenCaptureSession {
            started_at: Instant::now(),
            stopping,
            urgent_snapshot: Arc::new(AtomicBool::new(false)),
            frames: Arc::new(Mutex::new(CandidateBuffer::new(1 << 20))),
            real_frame_seen: Arc::new(AtomicBool::new(false)),
            startup_failed: Arc::new(AtomicBool::new(false)),
            video_offset_ms: Arc::new(AtomicU64::new(0)),
            sampler_thread: Some(worker),
            _recording: crate::capture::engine::bundle::retention::Recording::start(),
        };
        sample_real_candidate(
            &session.frames, &session.real_frame_seen, &session.startup_failed,
            true, || Err(error.into()),
        );
        assert!(!session.has_frames(), "a failed sample cannot supply readiness");
        assert_eq!(session.frames.lock().unwrap().len(), 0, "no demo pixels");
        let mut retried = false;
        sample_real_candidate(
            &session.frames, &session.real_frame_seen, &session.startup_failed,
            false, || {
                retried = true;
                Ok(RgbaFrame::new(220, 64, 64, vec![255; 64 * 64 * 4]))
            },
        );
        assert!(retried, "{error}: an individual sample failure must not disable later attempts");
        assert!(session.has_frames(), "the next successful sample unlocks Stop");
        assert!(!session.failed_to_start());
        let recording = session.stop();
        assert_eq!(recording.frames.len(), 1);
        let dir = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
        let target = TargetInfo {
            mode: CaptureMode::Region,
            app: None, bundle_id: None, window_title: None, window_id: None,
            display_id: 0,
            frame: TargetFrame { x: 0.0, y: 0.0, width: 64.0, height: 64.0, scale: 1.0 },
        };
        let (saved, bundle, _) = crate::capture::engine::bundle::bundle_processing::finalize_capture_bundle(
            &dir, target, recording, vec![], vec![], None, None,
            crate::shared::windows::ActivityLog::default(), None,
        ).expect("a recovered real sample must allow ordinary finalization");
        assert_eq!(saved, dir);
        assert!(!bundle.keyframes.is_empty());
        assert!(dir.join("apparent.json").is_file());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn pending_worker_is_not_failed_and_terminal_signal_does_not_discard_it() {
    let stopping = Arc::new(AtomicBool::new(false));
    let worker_stopping = Arc::clone(&stopping);
    let worker = thread::spawn(move || {
        while !worker_stopping.load(Ordering::Acquire) {
            thread::park_timeout(Duration::from_millis(5));
        }
        None
    });
    let session = ScreenCaptureSession {
        started_at: Instant::now(),
        stopping,
        urgent_snapshot: Arc::new(AtomicBool::new(false)),
        frames: Arc::new(Mutex::new(CandidateBuffer::new(1 << 20))),
        real_frame_seen: Arc::new(AtomicBool::new(false)),
        startup_failed: Arc::new(AtomicBool::new(false)),
        video_offset_ms: Arc::new(AtomicU64::new(0)),
        sampler_thread: Some(worker),
        _recording: crate::capture::engine::bundle::retention::Recording::start(),
    };
    assert!(!session.failed_to_start(), "a pending producer is not an error");
    session.startup_failed.store(true, Ordering::Release);
    sample_real_candidate(
        &session.frames, &session.real_frame_seen, &session.startup_failed,
        true, || panic!("a classified terminal setup failure must not retry"),
    );
    assert!(!session.has_frames());
    assert!(session.failed_to_start());
    assert!(session.failed_to_start(), "inspection must be read-only");
    assert!(!session.stopping.load(Ordering::Acquire));
    assert!(!session.sampler_thread.as_ref().unwrap().is_finished());
    assert!(session.stop().frames.is_empty());
}

#[test]
fn screen_stop_joins_worker_before_collecting_its_final_media() {
    let stopping = Arc::new(AtomicBool::new(false));
    let frames = Arc::new(Mutex::new(CandidateBuffer::new(1 << 20)));
    let worker_stopping = Arc::clone(&stopping);
    let worker_frames = Arc::clone(&frames);
    let path = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
    let worker_path = path.clone();
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !worker_stopping.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(worker_stopping.load(Ordering::Acquire));
        worker_frames
            .lock()
            .unwrap()
            .push(RgbaFrame::new(42, 1, 1, vec![1, 2, 3, 255]), false);
        std::fs::write(&worker_path, b"finalized media").unwrap();
        Some(worker_path)
    });
    let session = ScreenCaptureSession {
        started_at: Instant::now(),
        stopping,
        urgent_snapshot: Arc::new(AtomicBool::new(false)),
        frames,
        real_frame_seen: Arc::new(AtomicBool::new(false)),
        video_offset_ms: Arc::new(AtomicU64::new(120)),
        startup_failed: Arc::new(AtomicBool::new(false)),
        sampler_thread: Some(worker),
        _recording: crate::capture::engine::bundle::retention::Recording::start(),
    };
    let recording = session.stop();
    let frames = recording.frames;
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].t_ms, 42);
    assert_eq!(frames[0].rgba, vec![1, 2, 3, 255]);
    assert_eq!(recording.video.as_ref(), Some(&path));
    assert_eq!(recording.video_offset_ms, 120);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn candidate_buffer_stays_bounded_and_keeps_the_whole_timeline() {
    // A 1 MB budget of 4 KB frames: a 256-frame cap.
    let mut buffer = CandidateBuffer::new(1 << 20);
    let frame = |t| RgbaFrame::new(t, 32, 32, vec![0; 32 * 32 * 4]);
    let mut t = 0;
    let mut peak = 0;
    // Three hours at the sampler's pace, with a click frame every 10 s.
    while t < 3 * 3_600_000 {
        buffer.push(frame(t), t % 10_000 == 0);
        peak = peak.max(buffer.len());
        t += buffer.interval().as_millis() as u64;
    }
    assert!(peak <= 256, "peak {peak}");
    assert!(buffer.interval() >= FRAME_SAMPLE_INTERVAL * 64);
    let frames = buffer.into_frames();
    assert_eq!(frames[0].t_ms, 0, "the start is kept");
    assert!(frames.windows(2).all(|w| w[0].t_ms < w[1].t_ms));
    // Spread over the whole capture, not bunched at the end.
    let first_half = frames.iter().filter(|f| f.t_ms < 3 * 1_800_000).count();
    assert!(
        first_half * 3 > frames.len(),
        "{first_half} of {}",
        frames.len()
    );
}

#[test]
fn candidate_buffer_keeps_urgent_frames_when_thinning() {
    let mut buffer = CandidateBuffer::new(40 * 4096);
    let frame = |t| RgbaFrame::new(t, 32, 32, vec![0; 32 * 32 * 4]);
    for t in 0..60 {
        buffer.push(frame(t * 220), t == 33);
    }
    assert!(buffer.len() < 60);
    assert_eq!(buffer.interval(), FRAME_SAMPLE_INTERVAL * 2);
    assert!(buffer.into_frames().iter().any(|f| f.t_ms == 33 * 220));
}

#[test]
fn frames_that_may_show_a_system_prompt_are_dropped_but_never_the_first() {
    let mut buffer = CandidateBuffer::new(1 << 20);
    for t in [0, 220, 440, 660, 880] {
        buffer.push(RgbaFrame::new(t, 4, 4, vec![0; 64]), false);
    }
    buffer.drop_since(440);
    let times: Vec<u64> = buffer.into_frames().iter().map(|f| f.t_ms).collect();
    assert_eq!(times, [0, 220, 440]);
    let mut buffer = CandidateBuffer::new(1 << 20);
    buffer.push(RgbaFrame::new(500, 4, 4, vec![0; 64]), false);
    buffer.drop_since(0);
    assert_eq!(buffer.len(), 1);
}

#[test]
fn stopped_capture_without_real_frames_stays_empty() {
    let session = ScreenCaptureSession {
        started_at: Instant::now(),
        stopping: Arc::new(AtomicBool::new(false)),
        urgent_snapshot: Arc::new(AtomicBool::new(false)),
        frames: Arc::new(Mutex::new(CandidateBuffer::new(1 << 20))),
        real_frame_seen: Arc::new(AtomicBool::new(false)),
        video_offset_ms: Arc::new(AtomicU64::new(0)),
        startup_failed: Arc::new(AtomicBool::new(false)),
        sampler_thread: None,
        _recording: crate::capture::engine::bundle::retention::Recording::start(),
    };
    assert!(session.stop().frames.is_empty());
}
