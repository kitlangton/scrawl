//! Desktop capability adapter for Record's headless completion policy.
//! Receipts and Connect completion precede the stale-UI gate; presentation never does.

mod worker;

use std::time::Instant;

use gpui::Context;

use super::super::{ApparentCoordinator, CAPTURE_STATUS, IslandPhase};
use super::recording_completion::CaptureResult;
use crate::capture::engine::runtime::apparent_log;
pub(in crate::capture::desktop) use worker::{Capture, Facts};
use worker::{Completion, CopyMode, Effect, Outcome, State, Worker};

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
        // A saved capture completes independently of its pill. Hosted
        // admission rechecks current consent; failed takes stay local.
        if let Ok((dir, _, _)) = &finalized {
            on_completed(dir);
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
        facts: Facts,
        cx: &mut Context<Self>,
    ) {
        let generation = self.capture_generation;
        if self.completion_pending == Some(generation) { return; }
        self.completion_pending = Some(generation);
        let capture = facts.capture.clone();
        let outcome = match &finalized {
            Ok((dir, _, _)) => Outcome::Saved { bundle_path: dir.clone() },
            Err(error) => Outcome::Failed { message: error.to_string() },
        };
        // A renderer reload or exit cannot interrupt this worker or the finalizer.
        // The original result stays here until both bounded policy exchanges end.
        cx.spawn(async move |entity, cx| {
            let started = cx.background_executor().spawn(async move {
                let mut worker = Worker::open(facts, outcome)?;
                let decision = worker.read()?;
                anyhow::Ok((worker, decision))
            }).await;
            let completion = match started {
                Ok((worker, decision)) => {
                    let copy = decision.effects.first().map(|effect| match effect {
                        Effect::Copy { mode, .. } => *mode,
                    });
                    let receipt = copy.map(|mode| entity.update(cx, |coord, _| {
                        // Native capability admission is atomic with the clipboard call
                        // on this thread. Never trust a child/renderer generation alone.
                        if !coord.accepts_capture_result(generation) { return "revoked"; }
                        let Ok((dir, _, _)) = &finalized else { return "revoked"; };
                        let result = match mode {
                            #[cfg(target_os = "linux")]
                            CopyMode::Text => crate::shared::handoff::copy_bundle_text_to_clipboard(dir),
                            #[cfg(not(target_os = "linux"))]
                            CopyMode::Text => Err(anyhow::anyhow!("Unsupported text-only completion capability")),
                            CopyMode::Bundle => crate::shared::handoff::copy_bundle_to_clipboard(dir),
                        };
                        match result {
                            Ok(()) => "copied",
                            Err(error) => { apparent_log(format!("Copy handoff failed: {error:#}")); "failed" }
                        }
                    }).unwrap_or("revoked"));
                    cx.background_executor().spawn(async move {
                        worker.complete(decision, receipt)
                    }).await
                }
                Err(error) => Completion::unavailable(error),
            };
            let _ = entity.update(cx, |coord, cx| {
                if !coord.accepts_capture_result(generation) { return; }
                let state = match completion.state {
                    Ok(state) if state.current => Some(state),
                    Ok(_) => return,
                    Err(error) => {
                        apparent_log(format!("Record handoff unavailable; retaining finalizer result: {error:#}"));
                        None
                    }
                };
                coord.apply_completion_presentation(finalized, duration_ms, state, completion.clipboard_receipt, &capture, cx);
            });
        }).detach();
    }

    fn apply_completion_presentation(
        &mut self, finalized: CaptureResult, duration_ms: u64, state: Option<State>,
        clipboard_receipt: Option<bool>, capture: &Capture, cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.onboarding.as_mut() {
            session.practice_completion(
                self.capture_generation, &capture.capture_id, capture.request_id.as_deref(),
                finalized.as_ref().ok().map(|(dir, _, _)| dir.as_path()), clipboard_receipt,
            );
        }
        self.completed_dir = None;
        let (dir, bundle, _) = match finalized {
            Ok(capture) => capture,
            Err(error) => {
                apparent_log(format!("Capture failed: {error:#}"));
                #[cfg(target_os = "linux")]
                crate::capture::desktop::linux::shell::notify("Capture failed", &error.to_string());
                if let Ok(mut status) = CAPTURE_STATUS.lock() {
                    *status = Some(state.and_then(|s| s.status).unwrap_or_else(|| "Capture failed".into()));
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
        #[cfg(target_os = "linux")]
        if let Some(notification) = state.as_ref().and_then(|s| s.notification.as_ref()) {
            crate::capture::desktop::linux::shell::notify(&notification.title, &notification.body);
        }
        self.completed_dir = Some(dir.clone());
        self.last_local_capture_ms = crate::shared::clock::now_epoch_ms();
        // Exceptional recovery is factual only: preserve access to the exact saved
        // artifact, never emulate copy policy or infer that a write succeeded.
        self.completed_note = Some(state.and_then(|s| s.status)
            .unwrap_or_else(|| "Saved · handoff unavailable".into()));
        self.set_review_bundle((dir, bundle), self.completed_note.clone());
        self.set_island_phase(IslandPhase::Completed {
            finished_at: Instant::now(),
            frame_count,
            duration_ms,
            inbox_count: 0,
        });
        cx.notify();
    }
}
