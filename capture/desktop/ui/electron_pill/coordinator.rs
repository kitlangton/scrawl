//! Presentation routing and recovery; capture policy stays with the coordinator.
use super::{Session, View};
use crate::capture::desktop::{ApparentCoordinator, HudPillWindow, IslandPhase, apparent_log};
use gpui::Context;

impl ApparentCoordinator {
    fn electron_pill_view(&self, revision: u64) -> View {
        let view = View::new(
            &self.pill_snapshot(),
            revision,
            crate::shared::handoff::experiment("movableRecordingControls"),
        )
        .with_handoff(self.completed_dir.is_some(), self.review_bundle.is_some())
        .with_completion_notice(self.completed_note.as_deref())
        .with_notes(self.note_ui.as_ref().map_or(0, |ui| ui.saved_count()));
        let kind = if self.share_copied.as_ref() == self.completed_dir.as_ref() && self.completed_dir.is_some() {
            crate::connect::hosted::UploadQueueKind::PublicCopy
        } else { crate::connect::hosted::UploadQueueKind::Private };
        match &self.completed_upload_size_failure {
            Some((bundle, generation, failure)) if *generation == self.capture_generation
                && self.completed_dir.as_ref() == Some(bundle) && failure.kind == kind => view.with_size_failure(failure.size, failure.limit),
            _ => view,
        }
    }
    fn electron_pill_selection(&self) -> super::session::Selection {
        (
            self.completed_dir.clone(),
            self.review_bundle.as_ref().map(|(path, _)| path.clone()),
        )
    }
    pub(crate) fn reconcile_electron_pill(&mut self) {
        #[cfg(target_os = "macos")]
        if !self.is_recording() && !self.is_processing() && !Self::electron_pill_compatible() {
            if let Some(session) = self.electron_pill.take() {
                if let Some(id) = session.window_number() {
                    self.electron_pill_exclusions.push(id);
                }
                // Closing is asynchronous. Preserve this exact window's exclusion in
                // the next stream's initial filter, before it can supply any frame.
                drop(session);
                self.electron_pill_failed = true;
            }
            self.electron_pill_routed = false;
        }
    }
    fn electron_pill_compatible() -> bool {
        #[cfg(target_os = "macos")]
        return crate::capture::desktop::native::pill::PillStyle::current()
            != crate::capture::desktop::native::pill::PillStyle::Glass
            && crate::shared::handoff::experiment("movableRecordingControls");
        #[cfg(target_os = "linux")]
        true
    }
    pub(crate) fn ensure_electron_pill(&mut self) {
        self.reconcile_electron_pill();
        if self.electron_pill.is_some()
            || self.electron_pill_failed
            || !Self::electron_pill_compatible()
        {
            return;
        }
        let view = self.electron_pill_view(1);
        match Session::open(
            view,
            self.capture_generation,
            self.electron_pill_selection(),
        ) {
            Ok(session) => self.electron_pill = Some(session),
            Err(error) => {
                apparent_log(format!(
                    "Recording controls retain native presentation: {error:#}"
                ));
                self.electron_pill_failed = true;
            }
        }
    }
    pub(crate) fn sync_electron_pill(&mut self) {
        if self.electron_pill.is_none() {
            return;
        }
        let mut view = self.electron_pill_view(0);
        let selection = self.electron_pill_selection();
        #[cfg(target_os = "linux")]
        if matches!(self.island_phase, IslandPhase::Recording) {
            let activity =
                self.active_recording
                    .as_ref()
                    .and_then(|recording| match &recording.source {
                        crate::capture::desktop::CaptureSource::Screen {
                            audio: Some(audio),
                            ..
                        } => audio.input_activity(),
                        _ => None,
                    });
            self.electron_equalizer.step(activity, self.reduced_motion);
            view.input_level = Some(self.electron_equalizer.level.clamp(0.0, 1.0));
        }
        if let Some(session) = &mut self.electron_pill {
            session.update(view, self.capture_generation, selection);
        }
    }
    pub(crate) fn tick_electron_pill(&mut self, cx: &mut Context<Self>) {
        self.sync_electron_pill();
        if self
            .electron_pill
            .as_ref()
            .is_some_and(|session| !session.healthy())
        {
            self.electron_pill.take();
            self.electron_pill_failed = true;
            self.electron_pill_routed = false;
            apparent_log("Electron recording controls unavailable; restoring native controls");
            if !matches!(self.island_phase, IslandPhase::Hidden | IslandPhase::Dismissing) {
                let coordinator = cx.entity().clone();
                cx.defer(move |cx| super::super::super::ensure_hud_pill_open(&coordinator, cx));
            }
            return;
        }
        if !self.electron_pill.as_ref().is_some_and(Session::ready) {
            return;
        }
        if !self.electron_pill_routed {
            #[cfg(target_os = "linux")]
            if let Some(id) = self.electron_pill.as_ref().and_then(Session::window_number) {
                let mut excluded = self
                    .telemetry
                    .excluded_window_ids
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !excluded.contains(&id) {
                    excluded.push(id);
                }
            }
            self.electron_pill_routed = true;
            self.island_await_frame = false;
            self.native_pill.take();
            // Only the previous pill closes; picker/ink and recordings retain their owner.
            let coordinator = cx.entity().clone();
            cx.defer(move |cx| {
                for handle in cx.windows() {
                    if handle.downcast::<HudPillWindow>().is_some() {
                        let _ = handle.update(cx, |_, window, _| window.remove_window());
                    }
                }
                coordinator.update(cx, |coord, _| {
                    if let Some(session) = &mut coord.electron_pill {
                        session.activate();
                    }
                });
            });
        }
        let actions = self
            .electron_pill
            .as_mut()
            .map(Session::actions)
            .unwrap_or_default();
        for action in actions {
            // Each action was admitted against the freshly projected state, never renderer policy.
            self.apply_pill_action(action, cx);
            self.sync_electron_pill();
        }
        if matches!(self.island_phase, IslandPhase::Hidden) {
            self.sync_electron_pill();
        }
    }
}

#[cfg(test)]
mod upload_projection_tests {
    use super::*;
    use crate::connect::hosted::{UploadQueueKind, UploadSizeFailure};
    #[test]
    fn electron_pill_upload_cache_requires_current_capture_generation_phase_and_intent() {
        let (sender, _receiver) = flume::unbounded();
        let mut coord = ApparentCoordinator::new(crate::capture::desktop::SharedCaptureTelemetry::new(sender), None);
        let bundle = std::path::PathBuf::from("/synthetic/selected.apparent");
        coord.completed_dir = Some(bundle.clone());
        coord.capture_generation = 7;
        coord.island_phase = IslandPhase::Completed { finished_at: std::time::Instant::now(), frame_count: 1, duration_ms: 1000, inbox_count: 1 };
        let failure = UploadSizeFailure { kind: UploadQueueKind::Private, size: 90_000_001, limit: 90_000_000 };
        coord.completed_upload_size_failure = Some((bundle.clone(), 7, failure));
        assert_eq!(coord.electron_pill_view(1).pocket.unwrap().title, "Too large to upload");
        coord.capture_generation = 8;
        assert!(coord.electron_pill_view(2).pocket.is_none());
        coord.capture_generation = 7;
        coord.completed_dir = Some("/synthetic/other.apparent".into());
        assert!(coord.electron_pill_view(3).pocket.is_none());
        coord.completed_dir = Some(bundle.clone());
        coord.island_phase = IslandPhase::Hidden;
        assert!(coord.electron_pill_view(4).pocket.is_none());
        coord.island_phase = IslandPhase::Completed { finished_at: std::time::Instant::now(), frame_count: 1, duration_ms: 1000, inbox_count: 1 };
        coord.share_copied = Some(bundle);
        assert!(coord.electron_pill_view(5).pocket.is_none(), "private failure cannot become public-copy admission feedback");
        coord.completed_upload_size_failure.as_mut().unwrap().2.kind = UploadQueueKind::PublicCopy;
        assert_eq!(coord.electron_pill_view(6).pocket.unwrap().title, "Too large to upload");
    }
}
