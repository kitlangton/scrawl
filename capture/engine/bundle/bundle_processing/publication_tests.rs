//! Production-entry regressions: invalid persisted bytes must not escape as success.
use super::*;

fn target() -> TargetInfo {
    serde_json::from_value(serde_json::json!({
        "mode": "window", "displayId": 1,
        "frame": {"x":0.0,"y":0.0,"width":32.0,"height":32.0,"scale":1.0}
    }))
    .unwrap()
}

#[test]
fn publication_rejects_corrupt_prepared_png_before_success() {
    let root = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
    let frame = RgbaFrame::new(0, 32, 32, vec![128; 32 * 32 * 4]);
    let mut marks = vec![];
    let selected = select_keyframes(
        &[frame.clone()],
        &[],
        &mut marks,
        &[],
        &TranscriptData::default(),
        6,
    );
    let mut prepared = prepare_visuals(
        selected,
        Some(&frame),
        None,
        &target(),
        1000,
        &[],
        &[],
        &[],
        &windows::ActivityLog::default(),
    )
    .unwrap();
    prepared.contact_sheet_png = b"not a PNG".to_vec();
    let result = build_bundle_from_parts(
        &root,
        target(),
        1000,
        vec![frame],
        vec![],
        vec![],
        TranscriptData::default(),
        None,
        None,
        SourceExtras {
            prepared: Some(prepared),
            autocrop: Some(None),
            ..Default::default()
        },
    );
    let success_effect = result
        .as_ref()
        .ok()
        .map(|_| "Saved/clipboard/upload eligible");
    assert!(
        success_effect.is_none(),
        "invalid prepared bytes escaped the production finalizer"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn publication_checks_wav_persistence_before_transcription_or_manifest() {
    let root = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
    publication::Directory::open(&root, true)
        .unwrap()
        .child("audio.wav", true)
        .unwrap();
    let recorded =
        audio::recorded_audio_from_wav(audio::encode_wav_16k(&[0.0; 16]).unwrap()).unwrap();
    let result = finalize_capture_bundle(
        &root,
        target(),
        capture::ScreenRecording {
            frames: vec![RgbaFrame::new(0, 32, 32, vec![128; 32 * 32 * 4])],
            video: None,
            duration_ms: 1,
            video_offset_ms: 0,
        },
        vec![],
        vec![],
        Some(recorded),
        None,
        windows::ActivityLog::default(),
        None,
    );
    assert!(
        result.is_err(),
        "an occupied audio path must fail before transcription"
    );
    assert!(
        !root.join("apparent.json").exists(),
        "WAV write failure must stop before manifest publication"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn publication_screen_wrapper_checks_journal_and_keeps_required_audio() {
    for collide in [true, false] {
        let root = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
        let directory = publication::Directory::open(&root, true).unwrap();
        if collide {
            directory.child(recover::UNFINISHED, true).unwrap();
        }
        let stored = audio::encode_wav_16k(&[0.125; 160]).unwrap();
        if !collide {
            publication::write(&root, "audio.wav", &stored).unwrap();
        }
        publication::write(
            &root,
            crate::capture::engine::runtime::audio::persistence::RECORDING,
            b"saved audio recipe",
        )
        .unwrap();
        let mut target = target();
        target.frame.x = 0.12345678901234567;
        // Valid, empty captured WAV avoids any native/network transcriber call.
        let recorded = audio::recorded_audio_from_wav(audio::encode_wav_16k(&[]).unwrap()).unwrap();
        let result = finalize_capture_bundle(
            &root,
            target,
            capture::ScreenRecording {
                frames: vec![RgbaFrame::new(0, 32, 32, vec![128; 32 * 32 * 4])],
                video: None,
                duration_ms: 1,
                video_offset_ms: 0,
            },
            vec![],
            vec![],
            Some(recorded),
            None,
            windows::ActivityLog::default(),
            None,
        );
        if collide {
            assert!(
                format!("{:#}", result.unwrap_err()).contains("Could not persist recovery journal")
            );
            assert!(!root.join("apparent.json").exists());
        } else {
            let (_, bundle, summary) = result.unwrap();
            assert_eq!(bundle.audio.as_deref(), Some("audio.wav"));
            assert_eq!(std::fs::read(root.join("audio.wav")).unwrap(), stored);
            assert!(
                !root
                    .join(crate::capture::engine::runtime::audio::persistence::RECORDING)
                    .exists()
            );
            assert!(!root.join(recover::UNFINISHED).exists());
            assert!(!root.join(publication::IN_PROGRESS).exists());
            publication::checked(&root, &bundle, &summary).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn publication_rejects_external_and_symlinked_movies_before_decoder_callback() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let outside = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
    let outside_dir = publication::Directory::open(&outside, true).unwrap();
    let source = outside.join("recording.mp4");
    outside_dir
        .write(
            "recording.mp4",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/capture/engine/bundle/bundle_processing/publication/fixtures/tiny.mp4"
            )),
        )
        .unwrap();
    for linked in [false, true] {
        let root = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
        publication::Directory::open(&root, true).unwrap();
        let video = if linked {
            let path = root.join("recording.mp4");
            std::os::unix::fs::symlink(&source, &path).unwrap();
            path
        } else {
            source.clone()
        };
        let called = Arc::new(AtomicBool::new(false));
        let observed = called.clone();
        let result = build_bundle_from_parts(
            &root,
            target(),
            1000,
            vec![RgbaFrame::new(0, 32, 32, vec![128; 32 * 32 * 4])],
            vec![],
            vec![],
            TranscriptData::default(),
            Some(video),
            None,
            SourceExtras {
                full_frames: Some(Box::new(move |_| {
                    observed.store(true, Ordering::SeqCst);
                    anyhow::bail!("decoder must not be reached")
                })),
                autocrop: Some(None),
                ..Default::default()
            },
        );
        assert!(result.is_err());
        assert!(!called.load(Ordering::SeqCst));
        assert!(!root.join("apparent.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    std::fs::remove_dir_all(outside).unwrap();
}
