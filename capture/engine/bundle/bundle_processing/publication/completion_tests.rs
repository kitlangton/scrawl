//! The real shared screen/phone completion wrapper, optional whiteboard publisher,
//! and Desktop dispatch. Native clipboard/presentation still needs OS proof.
use super::*;
use crate::capture::engine::bundle::bundle_processing::{self, SourceExtras, publication};
use crate::shared::model::{RgbaFrame, TargetInfo, TranscriptData};
use std::path::PathBuf;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(crate::shared::ids::make_bundle_id()))
    }
    fn build(&self) -> CaptureResult {
        let target: TargetInfo = serde_json::from_value(serde_json::json!({
            "mode":"display","displayId":0,"frame":{"x":0.0,"y":0.0,"width":32.0,"height":32.0,"scale":1.0}
        }))?;
        let built = bundle_processing::build_bundle_from_parts(
            &self.0,
            target,
            1000,
            vec![RgbaFrame::new(0, 32, 32, vec![128; 32 * 32 * 4])],
            vec![],
            vec![],
            TranscriptData::default(),
            None,
            None,
            SourceExtras {
                autocrop: Some(None),
                ..Default::default()
            },
        )?;
        let recipe: crate::capture::engine::bundle::recover::Unfinished =
            serde_json::from_value(serde_json::to_value(&built.1)?)?;
        crate::capture::engine::bundle::recover::write(&self.0, &recipe)?;
        Ok(built)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn board() -> whiteboard::Board {
    let mut board = whiteboard::Board::default();
    board.start([40.0, 40.0]);
    board.extend([420.0, 310.0]);
    board.finish();
    board
}

fn dispatch(result: CaptureResult, success: bool) {
    let mut notifications = Vec::new();
    let result = after_publication(result, |path| {
        assert!(!path.join(publication::IN_PROGRESS).exists());
        assert!(!path.join(crate::capture::engine::bundle::recover::UNFINISHED).exists());
        notifications.push(path.to_path_buf());
    });
    assert_eq!(
        notifications.len(),
        usize::from(success),
        "retention notification must consume only the successful outer result"
    );
    use crate::capture::desktop::{ApparentCoordinator, IslandPhase};
    use crate::capture::engine::runtime::SharedCaptureTelemetry;
    let (sender, _receiver) = flume::unbounded();
    let mut coordinator = ApparentCoordinator::new(SharedCaptureTelemetry::new(sender), None);
    coordinator.set_island_phase(IslandPhase::Processing);
    assert!(
        coordinator
            .agent_captures
            .admit("publication-test", false, false)
    );
    let mut admitted = Vec::new();
    let mut current = false;
    coordinator.receive_capture_result(
        coordinator.capture_generation,
        Some("publication-test"),
        result,
        |dir| admitted.push(dir.to_path_buf()),
        |_, result| {
            current = true;
            assert_eq!(result.is_ok(), success);
        },
    );
    assert!(current);
    assert_eq!(
        admitted.len(),
        usize::from(success),
        "failed result reached Connect admission"
    );
    let receipt = coordinator.agent_captures.inspect("publication-test");
    assert_eq!(receipt["status"], if success { "saved" } else { "failed" });
    if !success {
        assert!(receipt["bundle_path"].is_null());
    }
}

#[test]
fn completion_whiteboard_success_is_checked_after_append_and_then_admitted() {
    let fixture = Fixture::new();
    let result = complete_bundle(&fixture.0, &board(), || fixture.build());
    let (dir, bundle, summary) = result.as_ref().unwrap();
    assert_eq!(
        bundle.keyframes.last().unwrap().reason,
        crate::shared::model::KeyframeReason::Whiteboard
    );
    assert!(summary.contains("whiteboard drawing"));
    publication::checked(dir, bundle, summary).unwrap();
    assert!(!dir.join(crate::capture::engine::bundle::recover::UNFINISHED).exists());
    dispatch(result, true);
}

#[test]
fn completion_failed_append_only_admits_the_intact_revalidated_fallback() {
    let fixture = Fixture::new();
    let result = complete_bundle_with(
        &fixture.0,
        || fixture.build(),
        |dir, bundle, summary| {
            assert!(
                dir.join(crate::capture::engine::bundle::recover::UNFINISHED).exists(),
                "inner build cleared outer recovery recipe"
            );
            whiteboard::finish_handoff_with(dir, bundle, summary, &board(), |index| {
                if index == 2 {
                    anyhow::bail!("injected append failure")
                }
                Ok(())
            })
        },
    );
    let (dir, bundle, summary) = result.as_ref().unwrap();
    assert_ne!(
        bundle.keyframes.last().unwrap().reason,
        crate::shared::model::KeyframeReason::Whiteboard
    );
    publication::checked(dir, bundle, summary).unwrap();
    dispatch(result, true);
}

#[test]
fn completion_failed_rollback_or_corrupt_fallback_preserves_recipe_and_blocks_admission() {
    for rollback_failure in [true, false] {
        let fixture = Fixture::new();
        let result = complete_bundle_with(
            &fixture.0,
            || fixture.build(),
            |dir, bundle, summary| {
                let returned =
                    whiteboard::finish_handoff_with(dir, bundle, summary, &board(), |index| {
                        if index == 2 {
                            if rollback_failure {
                                // A nonempty directory cannot be overwritten by rollback's
                                // rename, even under root: exercise the real rollback error.
                                std::fs::remove_file(dir.join("contact-sheet.png"))?;
                                std::fs::create_dir(dir.join("contact-sheet.png"))?;
                                std::fs::write(
                                    dir.join("contact-sheet.png/occupied"),
                                    b"sentinel",
                                )?;
                            }
                            anyhow::bail!("injected append failure");
                        }
                        Ok(())
                    });
                if !rollback_failure {
                    std::fs::write(dir.join("contact-sheet.png"), b"corrupt surviving fallback")
                        .unwrap();
                }
                returned
            },
        );
        let error = format!("{:#}", result.as_ref().unwrap_err());
        assert!(
            error.contains(if rollback_failure {
                "unfinished whiteboard publication"
            } else {
                "invalid PNG artifact: contact-sheet.png"
            }),
            "unexpected rejection: {error}"
        );
        assert!(fixture.0.join(crate::capture::engine::bundle::recover::UNFINISHED).exists());
        assert!(fixture.0.join(publication::IN_PROGRESS).exists());
        dispatch(result, false);
    }
}

#[test]
fn completion_marker_cleanup_failure_cannot_admit_an_intermediate_inner_success() {
    let fixture = Fixture::new();
    let result = complete_bundle_with(
        &fixture.0,
        || fixture.build(),
        |dir, bundle, summary| {
            publication::Directory::open(dir, false)
                .unwrap()
                .child(crate::capture::engine::bundle::recover::RECOVERING, true)
                .unwrap();
            (bundle, summary)
        },
    );
    let error = format!("{:#}", result.as_ref().unwrap_err());
    assert!(
        error.contains("cannot clean recovery marker .recovering"),
        "unexpected rejection: {error}"
    );
    assert!(fixture.0.join(crate::capture::engine::bundle::recover::UNFINISHED).exists());
    dispatch(result, false);
}
