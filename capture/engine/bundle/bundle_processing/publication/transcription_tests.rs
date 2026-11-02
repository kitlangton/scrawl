use super::*;
use crate::capture::engine::bundle::bundle_processing::{self, publication};
use crate::shared::model::{RgbaFrame, TargetInfo, TranscriptData};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("{}.apparent", crate::shared::ids::make_bundle_id())))
    }
    fn build(&self) -> Result<(PathBuf, crate::shared::model::ApparentBundle, String)> {
        publication::write(
            &self.0,
            "audio.wav",
            &crate::capture::engine::runtime::audio::audio::encode_wav_16k(&[0.0; 16])?,
        )?;
        let target: TargetInfo = serde_json::from_value(serde_json::json!({
            "mode":"device","displayId":0,"frame":{"x":0.0,"y":0.0,"width":16.0,"height":16.0,"scale":1.0}
        }))?;
        bundle_processing::build_bundle_from_parts(
            &self.0,
            target,
            1,
            vec![RgbaFrame::new(0, 16, 16, vec![128; 16 * 16 * 4])],
            vec![],
            vec![],
            TranscriptData {
                missing: Some(missing("downloading", "en", None)),
                ..Default::default()
            },
            None,
            Some("audio.wav".into()),
            bundle_processing::SourceExtras {
                autocrop: Some(None),
                ..Default::default()
            },
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn pending_publication_waits_for_outer_final_mutation_before_transcribing() {
    let fixture = Fixture::new();
    let (entered, enter_rx) = mpsc::channel();
    let (transcribed, transcript_rx) = mpsc::channel();
    let mut worker = None;
    publication::publish_after(&fixture.0, || {
        let built = fixture.build()?;
        let path = fixture.0.clone();
        worker = Some(std::thread::spawn(move || {
            entered.send(()).unwrap();
            transcribe_again_with(&path, |_| {
                transcribed.send(()).unwrap();
                Ok(TranscriptData::default())
            })
        }));
        enter_rx.recv().unwrap();
        assert!(
            transcript_rx
                .recv_timeout(Duration::from_millis(40))
                .is_err()
        );
        assert!(fixture.0.join(PENDING).is_file());
        Ok(built)
    })
    .unwrap();
    transcript_rx.recv_timeout(Duration::from_secs(4)).unwrap();
    assert!(worker.unwrap().join().unwrap().unwrap());
    assert!(!fixture.0.join(PENDING).exists());
}

#[test]
fn pending_error_retry_and_success_preserve_real_bundle_until_committed_update() {
    let fixture = Fixture::new();
    fixture.build().unwrap();
    let before = std::fs::read(fixture.0.join("apparent.json")).unwrap();
    assert!(
        transcribe_again_with(&fixture.0, |_| anyhow::bail!("transcription unavailable")).is_err()
    );
    assert!(fixture.0.join(PENDING).is_file());
    assert_eq!(
        std::fs::read(fixture.0.join("apparent.json")).unwrap(),
        before
    );
    assert!(
        !transcribe_again_with(&fixture.0, |_| Ok(TranscriptData {
            missing: Some(missing("downloading", "en", None)),
            ..Default::default()
        }))
        .unwrap()
    );
    assert!(fixture.0.join(PENDING).is_file());
    assert!(transcribe_again_with(&fixture.0, |_| Ok(TranscriptData::default())).unwrap());
    assert!(!fixture.0.join(PENDING).exists());
    assert_ne!(
        std::fs::read(fixture.0.join("apparent.json")).unwrap(),
        before
    );
    let bundle =
        serde_json::from_str(&publication::read_text(&fixture.0, "apparent.json").unwrap())
            .unwrap();
    publication::checked(
        &fixture.0,
        &bundle,
        &publication::read_text(&fixture.0, "summary.md").unwrap(),
    )
    .unwrap();
}

#[test]
fn failed_initial_barrier_never_starts_pending_transcription() {
    let fixture = Fixture::new();
    assert!(
        publication::publish_after(&fixture.0, || {
            let built = fixture.build()?;
            std::fs::write(fixture.0.join("contact-sheet.png"), b"corrupt")?;
            Ok(built)
        })
        .is_err()
    );
    let called = AtomicBool::new(false);
    assert!(
        !transcribe_again_with(&fixture.0, |_| {
            called.store(true, Ordering::SeqCst);
            Ok(TranscriptData::default())
        })
        .unwrap()
    );
    assert!(!called.load(Ordering::SeqCst));
    assert!(fixture.0.join(PENDING).is_file());
}

#[test]
fn concurrent_pending_retry_revalidates_after_real_artifact_replacement() {
    let fixture = Arc::new(Fixture::new());
    fixture.build().unwrap();
    std::fs::remove_file(fixture.0.join("audio.wav")).unwrap();
    std::os::unix::fs::symlink("/dev/zero", fixture.0.join("audio.wav")).unwrap();
    let called = AtomicBool::new(false);
    assert!(
        transcribe_again_with(&fixture.0, |_| {
            called.store(true, Ordering::SeqCst);
            Ok(TranscriptData::default())
        })
        .is_err()
    );
    assert!(!called.load(Ordering::SeqCst));
    assert!(fixture.0.join(PENDING).is_file());
}

#[test]
fn pending_publication_restores_both_text_artifacts_after_late_failure() {
    for fail_at in [0, 1] {
        let fixture = Fixture::new();
        fixture.build().unwrap();
        let manifest = std::fs::read(fixture.0.join("apparent.json")).unwrap();
        let summary = std::fs::read(fixture.0.join("summary.md")).unwrap();
        let error = transcribe_again_with_publication(
            &fixture.0,
            |_| Ok(TranscriptData::default()),
            |step| {
                assert_ne!(std::fs::read(fixture.0.join("apparent.json")).unwrap(), manifest);
                if step == fail_at { anyhow::bail!("injected publication step {step}") }
                Ok(())
            },
        ).unwrap_err();
        assert!(format!("{error:#}").contains("injected publication step"));
        assert_eq!(std::fs::read(fixture.0.join("apparent.json")).unwrap(), manifest);
        assert_eq!(std::fs::read(fixture.0.join("summary.md")).unwrap(), summary);
        assert!(fixture.0.join(PENDING).is_file());
        assert!(transcribe_again_with(&fixture.0, |_| Ok(TranscriptData::default())).unwrap());
        assert!(!fixture.0.join(PENDING).exists());
    }
}
