//! Synthetic, real file/process boundaries. No input device or screen permission is used.
use super::*;
use crate::capture::engine::bundle::recover;
use std::process::{Command, Stdio};

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("apparent-narration-{name}-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn tone(at: usize) -> f32 {
    ((at % 997) as f32 - 498.0) / 1000.0
}
fn wait_for(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "synthetic writer deadline");
        std::thread::sleep(Duration::from_millis(5));
    }
}
struct SyntheticChild {
    child: std::process::Child,
    dir: PathBuf,
    logs: Scratch,
}
impl SyntheticChild {
    fn wait_for(&mut self, stage: &str, mut predicate: impl FnMut() -> bool) -> Result<()> {
        let started = Instant::now();
        while !predicate() {
            let status = self.child.try_wait()?;
            if status.is_some() || started.elapsed() >= Duration::from_secs(5) {
                let log = |name: &str| {
                    std::fs::read_to_string(self.logs.0.join(name))
                        .unwrap_or_default()
                        .chars()
                        .take(4096)
                        .collect::<String>()
                };
                anyhow::bail!(
                    "synthetic {stage}: pid={}, elapsed={:?}, status={status:?}, produced={:?}\nstdout: {}\nstderr: {}",
                    self.child.id(),
                    started.elapsed(),
                    std::fs::read_to_string(self.dir.join("produced")).ok(),
                    log("child.stdout"),
                    log("child.stderr"),
                );
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
}
impl Drop for SyntheticChild {
    fn drop(&mut self) {
        // Reap before Scratch removes a directory still owned by the producer.
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}
fn spawn_unready(dir: &Path, mode: &str) -> SyntheticChild {
    // Capture admission requires an empty output directory. Diagnostics have
    // their own guard, registered before spawning and dropped after reaping.
    let log_dir = dir.with_extension("child-logs");
    std::fs::create_dir(&log_dir).unwrap();
    let logs = Scratch(log_dir);
    let test = format!(
        "{}::synthetic_child",
        module_path!().split_once("::").unwrap().1
    );
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &test, "--ignored", "--nocapture"])
        .env("APPARENT_SYNTHETIC_AUDIO_DIR", dir)
        .env("APPARENT_SYNTHETIC_AUDIO_MODE", mode)
        .stdout(Stdio::from(
            std::fs::File::create(logs.0.join("child.stdout")).unwrap(),
        ))
        .stderr(Stdio::from(
            std::fs::File::create(logs.0.join("child.stderr")).unwrap(),
        ))
        .spawn()
        .unwrap();
    SyntheticChild {
        child,
        dir: dir.to_owned(),
        logs,
    }
}
fn spawn(dir: &Path, mode: &str) -> SyntheticChild {
    let mut child = spawn_unready(dir, mode);
    // Process/dynamic-loader/writer initialization is not audio-production time.
    child
        .wait_for("readiness", || dir.join("ready").exists())
        .unwrap();
    child
}
fn publish_produced(dir: &Path, produced: usize) {
    std::fs::write(dir.join("produced.next"), produced.to_string()).unwrap();
    std::fs::rename(dir.join("produced.next"), dir.join("produced")).unwrap();
}

#[test]
#[ignore = "subprocess boundary; parent supplies a disposable synthetic directory"]
fn synthetic_child() {
    let dir = PathBuf::from(std::env::var_os("APPARENT_SYNTHETIC_AUDIO_DIR").unwrap());
    let mode = std::env::var("APPARENT_SYNTHETIC_AUDIO_MODE").unwrap();
    assert_ne!(
        mode, "fail-before-ready",
        "injected synthetic startup failure"
    );
    let _signals = (mode == "graceful")
        .then(|| crate::capture::desktop::workflow::shutdown::Signals::install().unwrap());
    recover::begin_screen_capture(&dir).unwrap();
    let samples = Arc::new(Mutex::new(Samples::default()));
    let error = Arc::new(Mutex::new(None));
    let mut writer = Writer::start(&dir, samples.clone(), error.clone()).unwrap();
    if mode == "full" {
        // Limit only this disposable child. Real kernel ENOSPC-equivalent EFBIG after a prefix.
        unsafe {
            libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
            let limit = libc::rlimit {
                rlim_cur: 3_244,
                rlim_max: 3_244,
            };
            assert_eq!(libc::setrlimit(libc::RLIMIT_FSIZE, &limit), 0);
        }
    }
    std::fs::write(dir.join("ready"), b"writer initialized\n").unwrap();
    let started = Instant::now();
    let mut produced = 0;
    for at in (0..32_000).step_by(160) {
        if mode == "graceful" && crate::capture::desktop::workflow::shutdown::requested() {
            break;
        }
        samples.lock().unwrap().extend((at..at + 160).map(tone));
        produced = at + 160;
        // Do not truncate/write a control file for every simulated callback.
        if produced == 4_000 || produced == 20_000 {
            publish_produced(&dir, produced);
        }
        if mode.starts_with("crash") && produced == 20_000 {
            // Freeze the published source count, not the real WAV worker. Only
            // the parent's real kill may end this recording; never run Stop.
            std::thread::sleep(Duration::from_secs(10));
            panic!("parent did not kill the synthetic crash producer");
        }
        // Follow the source clock instead of accumulating relative-sleep drift.
        let due = started + Duration::from_micros(produced as u64 * 1_000_000 / RATE as u64);
        let delay = due.saturating_duration_since(Instant::now());
        if !delay.is_zero() {
            let late = if mode == "crash-late-wake" {
                Duration::from_millis(40)
            } else {
                Duration::ZERO
            };
            std::thread::sleep(delay + late);
        }
        if mode == "full" && error.lock().unwrap().is_some() {
            assert!(writer.finish().is_err());
            return;
        }
    }
    let (wav, _lease) = writer.finish().unwrap();
    publish_produced(&dir, produced);
    assert_eq!(wav.len(), 44 + produced * 2);
}

#[test]
fn synthetic_startup_failure_reports_child_error_and_reaps_it() {
    let dir = Scratch::new("startup-diagnostic");
    let mut child = spawn_unready(&dir.0, "fail-before-ready");
    let failure = child
        .wait_for("readiness", || dir.0.join("ready").exists())
        .unwrap_err();
    assert!(
        failure
            .to_string()
            .contains("injected synthetic startup failure")
    );
    assert!(child.child.try_wait().unwrap().is_some());
}

#[test]
fn abrupt_process_death_repairs_only_narration_and_measures_tail() {
    for mode in ["crash", "crash-late-wake"] {
        let dir = Scratch::new("crash");
        let mut child = spawn(&dir.0, mode);
        child
            .wait_for("20,000 accepted samples", || {
                std::fs::read_to_string(dir.0.join("produced"))
                    .ok()
                    .and_then(|n| n.parse::<usize>().ok())
                    .is_some_and(|n| n >= 20_000)
            })
            .unwrap();
        assert!(persistence_live(&dir.0));
        assert!(!recover::owed(&dir.0));
        assert!(recover::rebuild(&dir.0, &recover::Media::SYSTEM).is_err());
        child.child.kill().unwrap();
        assert!(!child.child.wait().unwrap().success());
        let produced: usize = std::fs::read_to_string(dir.0.join("produced"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            produced, 20_000,
            "killed source must match its published counter"
        );
        let raw = std::fs::read(dir.0.join("audio.wav")).unwrap();
        let expected_samples = (raw.len() - 44) / 2;
        assert!(recover::owed(&dir.0));
        let summary = recover::rebuild(&dir.0, &recover::Media::SYSTEM).unwrap();
        assert!(summary.contains("Recovered narration only"));
        let wav = std::fs::read(dir.0.join("audio.wav")).unwrap();
        let decoded = super::super::audio::decode_wav_16k(&wav).unwrap();
        assert_eq!(decoded.len(), expected_samples);
        assert!(decoded.len() > 16_000, "must retain more than a header");
        for (at, sample) in decoded.iter().enumerate() {
            assert!((sample - tone(at)).abs() < 0.0001);
        }
        let tail = produced - decoded.len();
        eprintln!(
            "synthetic process crash: produced={produced}, saved={}, lost={tail} samples / {} ms",
            decoded.len(),
            tail as f64 / 16.0
        );
        assert!(
            tail <= MAX_BACKLOG,
            "measured tail exceeds intended backlog bound"
        );
        assert!(dir.0.join(recover::UNSAFE_VIDEO).exists());
        assert!(!dir.0.join("apparent.json").exists());
        assert!(!dir.0.join("frames").exists());
        assert!(!recover::owed(&dir.0));
    }
}
fn persistence_live(dir: &Path) -> bool {
    live(dir)
}

#[test]
fn graceful_sigterm_drains_every_accepted_sample() {
    let dir = Scratch::new("term");
    let mut child = spawn(&dir.0, "graceful");
    child
        .wait_for("4,000 accepted samples", || {
            std::fs::read_to_string(dir.0.join("produced"))
                .ok()
                .and_then(|n| n.parse::<usize>().ok())
                .is_some_and(|n| n >= 4_000)
        })
        .unwrap();
    assert_eq!(
        unsafe { libc::kill(child.child.id() as libc::pid_t, libc::SIGTERM) },
        0
    );
    assert!(child.child.wait().unwrap().success());
    let produced: usize = std::fs::read_to_string(dir.0.join("produced"))
        .unwrap()
        .parse()
        .unwrap();
    let wav = std::fs::read(dir.0.join("audio.wav")).unwrap();
    let decoded = super::super::audio::decode_wav_16k(&wav).unwrap();
    assert_eq!(decoded.len(), produced);
    for (at, sample) in decoded.iter().enumerate() {
        assert!((sample - tone(at)).abs() < 0.0001);
    }
}

#[test]
fn long_recording_saves_expired_history_without_growing_callback_memory() {
    let dir = Scratch::new("long");
    let samples = Arc::new(Mutex::new(Samples::default()));
    let allocation = samples.lock().unwrap().data.as_ptr();
    let mut writer = Writer::start(&dir.0, samples.clone(), Arc::default()).unwrap();
    let mut produced = 0;
    for _ in 0..65 {
        samples
            .lock()
            .unwrap()
            .extend((produced..produced + RATE).map(tone));
        produced += RATE;
        wait_for(|| {
            std::fs::metadata(dir.0.join("audio.wav")).unwrap().len() == (44 + produced * 2) as u64
        });
    }
    let (wav, lease) = writer.finish().unwrap();
    assert!(
        Lease::acquire(&dir.0).is_err(),
        "Stop must keep ownership through bundle finalization"
    );
    assert_eq!(samples.lock().unwrap().data.as_ptr(), allocation);
    assert_eq!(samples.lock().unwrap().recent().len(), HISTORY);
    let decoded = super::super::audio::decode_wav_16k(&wav).unwrap();
    assert_eq!(decoded.len(), produced);
    for (at, sample) in decoded.iter().enumerate() {
        assert!((sample - tone(at)).abs() < 0.0001);
    }
    drop((writer, lease));
    assert!(Lease::acquire(&dir.0).is_ok());
}

#[cfg(target_os = "linux")]
#[test]
fn kernel_enospc_is_a_save_failure_not_successful_silence() {
    let mut full = OpenOptions::new().write(true).open("/dev/full").unwrap();
    let samples = Mutex::new(Samples::from(vec![0.2; 160]));
    let (sender, receiver) = std::sync::mpsc::channel();
    drop(sender);
    let error = write_stream(&mut full, &samples, receiver).unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .raw_os_error(),
        Some(libc::ENOSPC)
    );
}

#[test]
fn normal_stop_keeps_every_sample_and_holds_lease_until_consumed() {
    let dir = Scratch::new("normal");
    let mut child = spawn(&dir.0, "normal");
    assert!(child.child.wait().unwrap().success());
    let before = std::fs::read(dir.0.join("audio.wav")).unwrap();
    let lease = Lease::acquire(&dir.0).unwrap();
    assert_eq!(repair(&dir.0, &lease).unwrap(), 32_000);
    assert_eq!(std::fs::read(dir.0.join("audio.wav")).unwrap(), before);
    for (at, sample) in super::super::audio::decode_wav_16k(&before)
        .unwrap()
        .iter()
        .enumerate()
    {
        assert!((sample - tone(at)).abs() < 0.0001);
    }
}

#[test]
fn real_file_size_failure_retains_prefix_and_reports_failure() {
    let dir = Scratch::new("full");
    let mut child = spawn(&dir.0, "full");
    assert!(child.child.wait().unwrap().success());
    let lease = Lease::acquire(&dir.0).unwrap();
    let saved = repair(&dir.0, &lease).unwrap();
    assert_eq!(saved, 1_600);
    let wav = std::fs::read(dir.0.join("audio.wav")).unwrap();
    let decoded = super::super::audio::decode_wav_16k(&wav).unwrap();
    assert_eq!(decoded.len(), saved);
    for (at, sample) in decoded.iter().enumerate() {
        assert!((sample - tone(at)).abs() < 0.0001);
    }
}

#[test]
fn truncated_sample_repair_refuses_unknown_files_and_preserves_healthy_bytes() {
    let dir = Scratch::new("repair");
    let lease = Lease::create(&dir.0).unwrap();
    let mut partial = header(0);
    partial.extend([0x34, 0x12, 0x78]);
    std::fs::write(dir.0.join("audio.wav"), &partial).unwrap();
    assert_eq!(repair(&dir.0, &lease).unwrap(), 1);
    let healthy = std::fs::read(dir.0.join("audio.wav")).unwrap();
    assert_eq!(&healthy[44..], &[0x34, 0x12]);
    let modified = std::fs::metadata(dir.0.join("audio.wav"))
        .unwrap()
        .modified()
        .unwrap();
    repair(&dir.0, &lease).unwrap();
    assert_eq!(
        std::fs::metadata(dir.0.join("audio.wav"))
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
    let other = b"not a narration file";
    std::fs::write(dir.0.join("audio.wav"), other).unwrap();
    assert!(repair(&dir.0, &lease).is_err());
    assert_eq!(std::fs::read(dir.0.join("audio.wav")).unwrap(), other);
}

#[test]
fn long_history_is_bounded_and_absolute_ranges_do_not_shift() {
    let mut samples = Samples::default();
    let allocation = samples.data.as_ptr();
    for at in 0..(3 * HISTORY + 137) {
        samples.push(at as f32);
    }
    assert_eq!(samples.data.as_ptr(), allocation);
    assert_eq!(samples.data.len(), 2 * HISTORY);
    assert_eq!(samples.len(), 3 * HISTORY + 137);
    assert!(samples.get(0..160).is_none());
    assert_eq!(
        samples.get(samples.len() - 160..).unwrap(),
        (samples.len() - 160..samples.len())
            .map(|at| at as f32)
            .collect::<Vec<_>>()
    );
    assert_eq!(samples.recent().len(), HISTORY);
}

#[test]
fn cancelled_writer_is_joined_before_removal_and_does_not_resurrect_audio() {
    let dir = Scratch::new("cancel");
    let samples = Arc::new(Mutex::new(Samples::default()));
    let writer = Writer::start(&dir.0, samples.clone(), Arc::default()).unwrap();
    samples.lock().unwrap().extend((0..160).map(tone));
    assert!(live(&dir.0));
    drop(writer);
    std::fs::remove_dir_all(&dir.0).unwrap();
    std::thread::sleep(POLL + POLL);
    assert!(!dir.0.exists());
}

#[test]
fn cancelled_narration_is_never_repaired_even_after_the_writer_dies() {
    let dir = Scratch::new("cancel-tombstone");
    let samples = Arc::new(Mutex::new(Samples::from(vec![0.25; 160])));
    let writer = Writer::start(&dir.0, samples, Arc::default()).unwrap();
    recover::cancel(&dir.0).unwrap();
    drop(writer);
    let before = std::fs::read(dir.0.join("audio.wav")).unwrap();
    assert!(!recover::owed(&dir.0));
    assert!(
        recover::rebuild(&dir.0, &recover::Media::SYSTEM)
            .unwrap_err()
            .to_string()
            .contains("cancelled")
    );
    assert_eq!(std::fs::read(dir.0.join("audio.wav")).unwrap(), before);
    assert!(!dir.0.join("narration-recovered.txt").exists());
}

#[test]
fn interrupted_recursive_cancel_cleanup_stays_outside_recovery_namespace() {
    let root = Scratch::new("cancel-interrupted");
    let dir = root.0.join("capture.apparent");
    recover::begin_screen_capture(&dir).unwrap();
    let mut writer = Writer::start(
        &dir,
        Arc::new(Mutex::new(Samples::from(vec![0.2; 160]))),
        Arc::default(),
    )
    .unwrap();
    let (_, lease) = writer.finish().unwrap();
    drop((writer, lease));
    recover::cancel(&dir).unwrap();
    let quarantined = recover::quarantine_cancelled(&dir).unwrap();
    // Model remove_dir_all deleting its tombstone first, then dying with audio still present.
    std::fs::remove_file(quarantined.join(recover::CANCELLED)).unwrap();
    let before = std::fs::read(quarantined.join("audio.wav")).unwrap();
    assert!(
        recover::pending(
            &root.0,
            std::time::SystemTime::now() + Duration::from_secs(3600)
        )
        .is_empty()
    );
    assert!(!recover::owed(&quarantined));
    assert!(recover::rebuild(&quarantined, &recover::Media::SYSTEM).is_err());
    assert_eq!(
        std::fs::read(quarantined.join("audio.wav")).unwrap(),
        before
    );
}

#[test]
fn startup_failure_is_typed_and_preserves_preexisting_audio_without_orphan_journal() {
    let dir = Scratch::new("startup-fail");
    std::fs::write(dir.0.join("audio.wav"), b"healthy unrelated bytes").unwrap();
    let error = Writer::start(&dir.0, Arc::default(), Arc::default())
        .err()
        .expect("occupied audio must refuse startup");
    assert!(error.is::<SaveFailure>());
    assert!(!dir.0.join(RECORDING).exists());
    assert!(!recover::owed(&dir.0));
    assert_eq!(
        std::fs::read(dir.0.join("audio.wav")).unwrap(),
        b"healthy unrelated bytes"
    );
}

#[test]
fn failed_screen_start_cancels_local_narration_before_it_can_become_an_orphan() {
    let root = Scratch::new("screen-start-failed");
    let dir = root.0.join("capture.apparent");
    recover::begin_screen_capture(&dir).unwrap();
    let writer = Writer::start(&dir, Arc::new(Mutex::new(Samples::from(vec![0.2; 160]))), Arc::default()).unwrap();
    wait_for(|| std::fs::metadata(dir.join("audio.wav")).unwrap().len() > 44);
    // Same error-path owner used when screen startup returns Err after mic start.
    recover::abandon_start(&dir, || {
        assert!(dir.join(recover::CANCELLED).is_file(), "cancellation must precede producer teardown");
        drop(writer);
    }).unwrap();
    assert!(!dir.exists());
    assert!(recover::pending(&root.0, std::time::SystemTime::now() + Duration::from_secs(3600)).is_empty());
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0);
}

#[test]
fn recovery_metadata_cannot_overwrite_symlink_targets() {
    use std::os::unix::fs::symlink;
    let root = Scratch::new("symlinks");
    let outside = root.0.join("outside-private-file");
    std::fs::write(&outside, b"must stay unchanged").unwrap();
    let dir = root.0.join("capture.apparent");
    recover::begin_screen_capture(&dir).unwrap();
    let mut writer = Writer::start(
        &dir,
        Arc::new(Mutex::new(Samples::from(vec![0.2; 160]))),
        Arc::default(),
    )
    .unwrap();
    let (_, lease) = writer.finish().unwrap();
    drop((writer, lease));
    symlink(&outside, dir.join(recover::RECOVERING)).unwrap();
    symlink(&outside, dir.join("narration-recovered.txt")).unwrap();
    recover::rebuild(&dir, &recover::Media::SYSTEM).unwrap();
    assert_eq!(std::fs::read(&outside).unwrap(), b"must stay unchanged");
    symlink(&outside, dir.join(recover::CANCELLED)).unwrap();
    recover::cancel(&dir).unwrap();
    assert_eq!(std::fs::read(&outside).unwrap(), b"must stay unchanged");
    assert!(
        !std::fs::symlink_metadata(dir.join(recover::CANCELLED))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}
