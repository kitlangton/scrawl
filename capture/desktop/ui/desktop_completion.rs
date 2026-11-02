//! Desktop completion policy: Connect admission, clipboard and the current capture's feedback.
//! Receipts and Connect completion precede the stale-UI gate; presentation never does.

use std::time::Instant;

use gpui::{Context, WeakEntity};

use super::super::{
    ApparentCoordinator, CAPTURE_STATUS, IslandPhase, close_capture_windows, open_review_window,
};
use super::recording_completion::CaptureResult;
use crate::capture::engine::runtime::apparent_log;

pub(in crate::capture::desktop) fn connect_completed(dir: &std::path::Path) {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    crate::connect::hosted::completed(dir);
}

impl ApparentCoordinator {
    /// Dispatches a finalizer receipt separately from effects on the current capture UI.
    pub(in crate::capture::desktop) fn receive_capture_result(
        &mut self,
        generation: u64,
        #[cfg(any(target_os = "macos", target_os = "linux"))] finishing_id: Option<&str>,
        feedback_capture: bool,
        finalized: CaptureResult,
        on_completed: impl FnOnce(&std::path::Path),
        on_current: impl FnOnce(&mut Self, CaptureResult),
    ) {
        // The worker still owns its exact ID's receipt after dismissal. Only its
        // UI/clipboard/onboarding effects lose ownership when a new capture starts.
        if finalized.is_err() {
            crate::capture::desktop::workflow::error_reporting::report(
                crate::capture::desktop::workflow::error_reporting::Failure::FinalizeCapture,
            );
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if let Some(id) = finishing_id {
            self.agent_captures.finish(
                id,
                finalized
                    .as_ref()
                    .map(|(dir, _, _)| dir.as_path())
                    .map_err(|error| format!("{error:#}")),
            );
        }
        // A saved ordinary capture completes independently of its pill. Hosted
        // admission rechecks current consent; feedback and failed takes stay local.
        if !feedback_capture {
            if let Ok((dir, _, _)) = &finalized {
                on_completed(dir);
            }
        }
        if !self.accepts_capture_result(generation) {
            return;
        }
        on_current(self, finalized);
    }

    pub(in crate::capture::desktop) fn present_capture_result(
        &mut self,
        finalized: CaptureResult,
        duration_ms: u64,
        feedback_capture: bool,
        auto_active: bool,
        entity: &WeakEntity<Self>,
        cx: &mut Context<Self>,
    ) {
        self.completed_dir = None;
        let (dir, bundle, _) = match finalized {
            Ok(capture) => capture,
            Err(error) => {
                apparent_log(format!("Capture failed: {error:#}"));
                #[cfg(target_os = "linux")]
                crate::capture::desktop::linux::shell::notify("Capture failed", &error.to_string());
                if let Ok(mut status) = CAPTURE_STATUS.lock() {
                    *status = Some(
                        if error
                            .to_string()
                            .starts_with("No screen frames were captured")
                        {
                            "Capture failed — no screen frames".into()
                        } else {
                            "Capture failed".into()
                        },
                    );
                }
                self.set_island_phase(IslandPhase::Dismissing);
                cx.notify();
                return;
            }
        };
        let frame_count = bundle.keyframes.len();
        if let Ok(mut status) = CAPTURE_STATUS.lock() {
            *status = None;
        }
        #[cfg(target_os = "linux")]
        if let Ok(summary) = std::fs::read_to_string(dir.join("summary.md")) {
            if summary.contains("Image quality: ") {
                crate::capture::desktop::linux::shell::notify(
                    "Capture used smaller keyframes",
                    "Full-quality MP4 decoding failed. Open summary.md for details and Settings › Recording for decoder setup.",
                );
            }
        }
        // Feedback stays local until the user explicitly copies or attaches it.
        // For the blocked automatic trial, leave only text on Ctrl+V even if
        // the independent clipboardImage experiment is enabled.
        let clipboard_ready = !feedback_capture
            && (if auto_active {
                #[cfg(target_os = "linux")]
                {
                    crate::shared::handoff::copy_bundle_text_to_clipboard(&dir)
                }
                #[cfg(not(target_os = "linux"))]
                {
                    crate::shared::handoff::copy_bundle_to_clipboard(&dir)
                }
            } else {
                crate::shared::handoff::copy_bundle_to_clipboard(&dir)
            })
            .map_err(|error| apparent_log(format!("Copy handoff failed: {error:#}")))
            .is_ok();
        #[cfg(target_os = "linux")]
        if auto_active && clipboard_ready {
            // Do not dispatch a paste until the receiver can prove its
            // draft identity and acknowledge the image attachment.
            if let Err(error) = crate::capture::desktop::linux::auto_paste::auto_receiver_preflight() {
                apparent_log(format!("Automatic draft paste skipped: {error:#}"));
                crate::capture::desktop::linux::shell::notify(
                    "Draft paste skipped",
                    &format!(
                        "{error:#}. Press Ctrl+V for the text; attach contact-sheet.png from {} if needed",
                        dir.display()
                    ),
                );
            }
        } else if auto_active {
            crate::capture::desktop::linux::shell::notify(
                "Draft paste skipped",
                "Could not copy the handoff. Use the saved bundle from the paper tile.",
            );
        }
        self.completed_dir = Some(dir.clone());
        self.last_local_capture_ms = crate::shared::clock::now_epoch_ms();
        if feedback_capture {
            // No automatic clipboard write: the completed pill must not claim
            // that this private capture was copied.
            self.completed_note = Some("Saved · review".into());
        }
        #[cfg(target_os = "linux")]
        if auto_active {
            // No receiver receipt: keep the manual handoff visible.
            self.completed_note =
                Some(crate::capture::desktop::linux::auto_paste::completion_note(clipboard_ready).into());
        }
        self.set_review_bundle((dir, bundle), self.completed_note.clone());
        if feedback_capture {
            self.review_feedback = true;
            self.review_status =
                Some("Review before sharing publicly. Nothing is uploaded automatically.".into());
            let entity = entity.clone();
            cx.defer(move |cx| {
                close_capture_windows(cx);
                if let Some(entity) = entity.upgrade() {
                    open_review_window(&entity, cx);
                }
            });
        }
        self.set_island_phase(IslandPhase::Completed {
            finished_at: Instant::now(),
            frame_count,
            duration_ms,
            inbox_count: 0,
        });
        cx.notify();
    }
}
