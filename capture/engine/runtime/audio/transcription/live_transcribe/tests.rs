use super::*;

#[cfg(unix)]
#[test]
fn finalized_results_keep_absolute_source_times() {
    let script = format!(
        r#"{READY}; cat >/dev/null; echo '{{"segment":{{"t0":63,"t1":150,"text":"First."}},"words":[{{"t0":63,"t1":150,"text":"First."}}]}}'; echo '{{"segment":{{"t0":913,"t1":1000,"text":"Tail."}},"words":[{{"t0":913,"t1":1000,"text":"Tail."}}]}}'; echo '{{"done":true,"samples":16000}}'"#
    );
    let mut live =
        Live::start_with(fake(&script), Arc::new(Mutex::new(Samples::default()))).unwrap();
    let done = live
        .finish_within(&wav(&[0.1; 16000]), Duration::from_secs(2))
        .unwrap();
    assert_eq!(done.transcript.text, "First. Tail.");
    assert_eq!(
        done.transcript
            .words
            .iter()
            .map(|w| (w.t0, w.t1))
            .collect::<Vec<_>>(),
        vec![(63, 150), (913, 1000)]
    );
}

#[cfg(unix)]
#[test]
fn failure_after_a_completed_prefix_returns_no_partial_transcript() {
    let script = format!(
        r#"{READY}; cat >/dev/null; echo '{{"segment":{{"t0":0,"t1":100,"text":"Private prefix."}},"words":[{{"t0":0,"t1":100,"text":"Private prefix."}}]}}'; echo 'apparent-helper: injected tail failure' >&2; exit 1"#
    );
    let mut live =
        Live::start_with(fake(&script), Arc::new(Mutex::new(Samples::default()))).unwrap();
    let result = live.finish_within(&wav(&[0.1; 16000]), Duration::from_secs(2));
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("injected tail failure")
    );
}

#[cfg(unix)]
#[test]
fn reports_the_selected_apple_module() {
    for engine in ["SpeechTranscriber", "DictationTranscriber"] {
        let script = format!(
            r#"echo '{{"ready":true,"locale":"en_US","engine":"{engine}"}}'; cat >/dev/null; echo '{{"segment":{{"t0":0,"t1":20,"text":"Tail."}},"words":[{{"t0":0,"t1":20,"text":"Tail."}}]}}'; echo '{{"done":true,"samples":320}}'"#
        );
        let mut live =
            Live::start_with(fake(&script), Arc::new(Mutex::new(Samples::default()))).unwrap();
        let done = live
            .finish_within(&wav(&[0.1; 320]), Duration::from_secs(2))
            .unwrap();
        assert_eq!(done.transcript.model, Some(format!("{engine} en_US")));
        assert_eq!(done.transcript.text, "Tail.");
        assert_eq!(done.transcript.words.len(), 1);
        assert_eq!(done.transcript.words[0].t1, 20);
    }
}

/// A stand-in helper: echoes how many samples it read, as the real one does, after the
/// given lines.
fn fake(script: &str) -> Command {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", script]);
    command
}

#[cfg(unix)]
fn wav(samples: &[f32]) -> Vec<u8> {
    crate::capture::engine::runtime::audio::audio::encode_wav_16k(samples).unwrap()
}

#[cfg(unix)]
const READY: &str = r#"echo '{"ready":true,"locale":"en_US"}'"#;

#[cfg(unix)]
#[test]
fn collects_finalized_results_and_counts_every_sample() {
    let script = format!(
        r#"{READY}; n=$(wc -c | tr -d ' '); echo '{{"segment":{{"t0":900,"t1":1500,"text":"Look here."}},"words":[{{"t0":900,"t1":1200,"text":"Look"}},{{"t0":1200,"t1":1500,"text":"here."}}]}}'; echo '{{"segment":{{"t0":60,"t1":600,"text":"Okay,"}},"words":[{{"t0":60,"t1":600,"text":"Okay,"}}]}}'; echo "{{\"done\":true,\"samples\":$((n / 2))}}""#
    );
    let buffer = Arc::new(Mutex::new(vec![0.25_f32; 4_000].into()));
    let mut live = Live::start_with(fake(&script), Arc::clone(&buffer)).unwrap();
    std::thread::sleep(Duration::from_millis(250));
    let mut all = vec![0.25_f32; 4_000];
    all.extend([-0.5_f32; 28_000]);
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
    let out = std::env::temp_dir().join(format!("apparent-live-pcm-{}", std::process::id()));
    let script = format!(
        r#"{READY}; cat > '{}'; echo '{{"done":true,"samples":3}}'"#,
        out.display()
    );
    let buffer = Arc::new(Mutex::new(vec![0.5_f32, -0.25].into()));
    let mut live = Live::start_with(fake(&script), Arc::clone(&buffer)).unwrap();
    std::thread::sleep(Duration::from_millis(250));
    buffer.lock().unwrap().clear();
    let recording = [0.5_f32, -0.25, 1.5];
    live.finish_within(&wav(&recording), Duration::from_secs(5))
        .unwrap();
    let written = std::fs::read(&out).unwrap();
    let _ = std::fs::remove_file(&out);
    let expected =
        crate::capture::engine::runtime::audio::audio::decode_wav_pcm16(&wav(&recording)).unwrap();
    assert_eq!(written, le_bytes(&expected));
}

#[cfg(unix)]
#[test]
fn a_failing_helper_or_lost_samples_is_an_error() {
    let buffer = || Arc::new(Mutex::new(vec![0.1_f32; 320].into()));
    let failed = Live::start_with(
        fake("echo 'apparent-helper: the speech model isnt installed' >&2; exit 1"),
        buffer(),
    )
    .unwrap()
    .finish_within(&wav(&[0.1; 320]), Duration::from_secs(5));
    assert!(failed.is_err());

    let short = format!(r#"{READY}; cat > /dev/null; echo '{{"done":true,"samples":7}}'"#);
    let lost = Live::start_with(fake(&short), buffer())
        .unwrap()
        .finish_within(&wav(&[0.1; 320]), Duration::from_secs(5));
    assert!(lost.unwrap_err().to_string().contains("heard 7 samples"));

    let hung = Live::start_with(
        fake(&format!("{READY}; cat > /dev/null; sleep 30")),
        buffer(),
    )
    .unwrap()
    .finish_within(&wav(&[0.1; 320]), Duration::from_millis(300));
    assert!(hung.unwrap_err().to_string().contains("to finish"));
}

#[test]
fn dropping_an_unfinished_stream_stops_the_helper() {
    let live =
        Live::start_with(fake("sleep 30"), Arc::new(Mutex::new(Samples::default()))).unwrap();
    let started = Instant::now();
    drop(live);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[cfg(unix)]
#[test]
fn finish_deadline_includes_a_blocked_feeder_and_tail_write() {
    for already_streaming in [false, true] {
        let samples = vec![0.25; 160_000];
        let buffer = Arc::new(Mutex::new(
            if already_streaming {
                samples.clone()
            } else {
                Vec::new()
            }
            .into(),
        ));
        // No stdin reads: both a recording-time write and the WAV tail can fill the pipe.
        // A finite stand-in bounds the regression itself on the old implementation.
        let mut live = Live::start_with(fake("exec sleep 2"), buffer).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let started = Instant::now();
        assert!(
            live.finish_within(&wav(&samples), Duration::from_millis(100))
                .is_err()
        );
        assert!(
            started.elapsed() < Duration::from_millis(700),
            "blocked Finish: {:?}",
            started.elapsed()
        );
    }
}

#[cfg(unix)]
#[test]
fn cancellation_interrupts_a_blocked_recording_write() {
    let buffer = Arc::new(Mutex::new(vec![0.25; 160_000].into()));
    let live = Live::start_with(fake("exec sleep 2"), buffer).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    drop(live);
    assert!(
        started.elapsed() < Duration::from_millis(700),
        "blocked Cancel: {:?}",
        started.elapsed()
    );
}

#[cfg(unix)]
#[test]
fn corrupt_streams_cannot_replace_whole_wav_transcription() {
    for events in [
        r#"echo '{"ready":false,"locale":"en_US"}'; echo '{"done":true,"samples":16000}'"#
            .to_owned(),
        format!(r#"{READY}; echo '{{"done":false,"samples":16000}}'"#),
        format!(r#"{READY}; echo 'not json'; echo '{{"done":true,"samples":16000}}'"#),
        format!(
            r#"{READY}; echo '{{"segment":{{"t0":0,"t1":1001,"text":"beyond end"}}}}'; echo '{{"done":true,"samples":16000}}'"#
        ),
        format!(
            r#"{READY}; echo '{{"segment":{{"t0":800,"t1":100,"text":"backwards"}}}}'; echo '{{"done":true,"samples":16000}}'"#
        ),
        format!(
            r#"{READY}; echo '{{"segment":{{"t0":0,"t1":100,"text":"word"}},"words":[{{"t0":0,"t1":1001,"text":"word"}}]}}'; echo '{{"done":true,"samples":16000}}'"#
        ),
        r#"echo '{"ready":true,"locale":"en_US","engine":"Unknown"}'; echo '{"done":true,"samples":16000}'"#.to_owned(),
    ] {
        let mut live = Live::start_with(
            fake(&format!("cat >/dev/null; {events}")),
            Arc::new(Mutex::new(Samples::default())),
        )
        .unwrap();
        assert!(
            live.finish_within(&wav(&vec![0.1; 16000]), Duration::from_secs(2))
                .is_err(),
            "accepted {events}"
        );
    }
}

#[cfg(unix)]
#[test]
fn stopped_mid_write_resends_only_the_unwritten_wav_tail() {
    let dir = std::env::temp_dir().join(format!("apparent-live-tail-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let out = dir.join("pcm");
    let script = format!(
        r#"{READY}; sleep 0.2; cat > '{}'; echo '{{"done":true,"samples":160000}}'"#,
        out.display()
    );
    let samples: Vec<f32> = (0..160_000).map(|i| (i % 1000) as f32 / 1000.0).collect();
    let mut live =
        Live::start_with(fake(&script), Arc::new(Mutex::new(samples.clone().into()))).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let recording = wav(&samples);
    live.finish_within(&recording, Duration::from_secs(3))
        .unwrap();
    let actual = std::fs::read(&out).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    assert_eq!(
        actual,
        le_bytes(
            &crate::capture::engine::runtime::audio::audio::decode_wav_pcm16(&recording).unwrap()
        )
    );
}

#[cfg(unix)]
#[test]
fn cancellation_does_not_wait_for_the_microphone_buffer_lock() {
    let buffer = Arc::new(Mutex::new(Samples::default()));
    let guard = buffer.lock().unwrap();
    let live = Live::start_with(fake("exec sleep 2"), Arc::clone(&buffer)).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    let started = Instant::now();
    drop(live);
    assert!(started.elapsed() < Duration::from_millis(700));
    drop(guard);
}

#[cfg(unix)]
#[test]
fn helper_stderr_cannot_block_audio_or_completion() {
    let script = format!(
        r#"{READY}; dd if=/dev/zero bs=1024 count=256 1>&2 2>/dev/null; n=$(wc -c | tr -d ' '); echo "{{\"done\":true,\"samples\":$((n / 2))}}""#
    );
    let mut live =
        Live::start_with(fake(&script), Arc::new(Mutex::new(Samples::default()))).unwrap();
    live.finish_within(&wav(&[0.1; 320]), Duration::from_secs(2))
        .unwrap();
}
#[test]
#[cfg(unix)]
fn overwritten_samples_fail_instead_of_skipping_to_the_recent_audio() {
    let mut samples = Samples::default();
    samples.extend(std::iter::repeat_n(0.1, 61 * 16_000));
    assert_eq!(samples.len(), 61 * 16_000);
    assert!(samples.get(0..).is_none());
    let mut live =
        Live::start_with(fake("cat > /dev/null"), Arc::new(Mutex::new(samples))).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !live.feeder.as_ref().unwrap().is_finished() {
        assert!(
            Instant::now() < deadline,
            "overwritten samples must fail before Stop"
        );
        std::thread::yield_now();
    }
    let error = live
        .finish_within(&wav(&[]), Duration::from_secs(1))
        .unwrap_err();
    assert!(error.to_string().contains("fell behind"));
}

#[cfg(unix)]
#[test]
fn terminal_protocol_requires_order_eof_and_successful_exit() {
    for script in [
        format!(
            r#"echo '{{"segment":{{"t0":0,"t1":1,"text":"early"}}}}'; {READY}; echo '{{"done":true,"samples":0}}'"#
        ),
        format!(
            r#"{READY}; echo '{{"done":true,"samples":0}}'; echo '{{"done":true,"samples":0}}'"#
        ),
        format!(r#"{READY}; echo '{{"done":true,"samples":0}}'; exit 1"#),
        format!(r#"{READY}; echo '{{"done":true,"samples":0}}'; exec sleep 2"#),
        // A descendant keeps stdout open even after the direct child exits.
        format!(r#"{READY}; echo '{{"done":true,"samples":0}}'; sleep 2 & exit 0"#),
    ] {
        let mut live =
            Live::start_with(fake(&script), Arc::new(Mutex::new(Samples::default()))).unwrap();
        assert!(
            live.finish_within(&wav(&[]), Duration::from_millis(150))
                .is_err(),
            "accepted {script}"
        );
    }
}

#[cfg(unix)]
#[test]
fn output_budgets_fail_instead_of_retaining_unbounded_results() {
    for script in [
        "head -c 70000 /dev/zero".to_owned(),
        format!(
            r#"{READY}; i=0; while [ "$i" -lt 17000 ]; do echo '{{"segment":{{"t0":0,"t1":0,"text":""}}}}'; i=$((i + 1)); done"#
        ),
    ] {
        let mut live =
            Live::start_with(fake(&script), Arc::new(Mutex::new(Samples::default()))).unwrap();
        assert!(
            live.finish_within(&wav(&[]), Duration::from_secs(3))
                .is_err()
        );
    }
}
#[test]
fn wrapped_history_uses_absolute_offsets_and_matches_saved_pcm() {
    let values: Vec<f32> = (0..61 * 16_000)
        .map(|i| (i % 1000) as f32 / 1000.0)
        .collect();
    let mut history = Samples::default();
    history.extend_from_slice(&values);
    let fed = 60 * 16_000 + 137;
    let actual = pending_pcm(&history, fed).unwrap();
    let recording = wav(&values[fed..]);
    let expected =
        crate::capture::engine::runtime::audio::audio::decode_wav_pcm16(&recording).unwrap();
    assert_eq!(actual, le_bytes(&expected));
    assert!(pending_pcm(&history, 0).is_err());
    history.clear();
    assert!(pending_pcm(&history, fed).unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn dropping_stream_kills_descendants_that_hold_the_pipes() {
    let dir = std::env::temp_dir().join(format!("apparent-live-descendant-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let marker = dir.join("survived");
    let script = format!("(sleep 0.5; touch '{}') & wait", marker.display());
    let live = Live::start_with(fake(&script), Arc::new(Mutex::new(Samples::default()))).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    drop(live);
    std::thread::sleep(Duration::from_millis(650));
    let survived = marker.exists();
    std::fs::remove_dir_all(dir).unwrap();
    assert!(
        !survived,
        "helper descendant survived process-group cleanup"
    );
}
#[cfg(unix)]
#[test]
fn successful_completion_cleans_descendants_before_reaping() {
    let dir = std::env::temp_dir().join(format!(
        "apparent-live-success-descendant-{}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).unwrap();
    let marker = dir.join("survived");
    let script = format!(
        r#"{READY}; cat >/dev/null; (sleep 0.5; touch '{}') >/dev/null 2>&1 & echo '{{"done":true,"samples":0}}'; exit 0"#,
        marker.display()
    );
    let mut live =
        Live::start_with(fake(&script), Arc::new(Mutex::new(Samples::default()))).unwrap();
    live.finish_within(&wav(&[]), Duration::from_secs(2))
        .unwrap();
    std::thread::sleep(Duration::from_millis(650));
    let survived = marker.exists();
    std::fs::remove_dir_all(dir).unwrap();
    assert!(!survived, "successful helper left a descendant alive");
}

#[cfg(unix)]
#[test]
fn aggregate_output_budget_applies_below_line_and_event_limits() {
    let line = format!(
        r#"{{"segment":{{"t0":0,"t1":0,"text":"{}"}}}}"#,
        "x".repeat(32_000)
    );
    let script = format!(
        r#"{READY}; i=0; while [ "$i" -lt 270 ]; do echo '{}'; i=$((i + 1)); done"#,
        line
    );
    let mut live =
        Live::start_with(fake(&script), Arc::new(Mutex::new(Samples::default()))).unwrap();
    let error = live
        .finish_within(&wav(&[]), Duration::from_secs(3))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("invalid live transcription protocol"),
        "expected the aggregate output guard, got: {error:#}"
    );
}
#[cfg(unix)]
#[test]
fn failed_live_prefix_invokes_complete_saved_recording_fallback() {
    use crate::capture::engine::runtime::audio::{audio::RecordedAudio, transcription::transcribe};
    let script = format!(
        r#"{READY}; cat >/dev/null; echo '{{"segment":{{"t0":0,"t1":100,"text":"Prefix."}}}}'; exit 1"#
    );
    let live = Live::start_with(fake(&script), Arc::new(Mutex::new(Samples::default()))).unwrap();
    let bytes = wav(&[0.1; 16_000]);
    let audio = RecordedAudio {
        sample_count: 16_000,
        duration_ms: 1000,
        wav_bytes: bytes.clone(),
        saved_lease: None,
        live: Slot::default(),
        #[cfg(target_os = "linux")]
        pause: Default::default(),
    };
    let called = std::cell::Cell::new(false);
    let result = transcribe::live_or_saved(Some(live), &audio, || {
        called.set(true);
        assert_eq!(audio.wav_bytes, bytes);
        assert_eq!(
            crate::capture::engine::runtime::audio::audio::decode_wav_pcm16(&audio.wav_bytes)
                .unwrap()
                .len(),
            16_000
        );
        TranscriptData {
            text: "Complete saved recording.".into(),
            ..Default::default()
        }
    });
    assert!(called.get());
    assert_eq!(result.text, "Complete saved recording.");
    assert_eq!(audio.wav_bytes, bytes);
}
#[cfg(target_os = "macos")]
#[test]
fn native_source_converter_and_completion_seams() {
    let status = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/capture/engine/native/tests/live-transcribe.py"
        ))
        .status()
        .unwrap();
    assert!(status.success());
}
