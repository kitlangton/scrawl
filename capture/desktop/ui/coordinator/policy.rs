use super::*;

static NATIVE_PEN_EFFECT_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static NATIVE_CLIPBOARD_EFFECT_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static NATIVE_HOST_REVISION_FOR_WORKER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct NativeHostObservation {
    configured: crate::capture::desktop::settings::Shortcuts,
    record: crate::capture::engine::runtime::ShortcutConfig,
    paste: crate::capture::engine::runtime::ShortcutConfig,
    registration: &'static str,
    can_save: bool,
    permission_actions: Vec<(&'static str, Vec<&'static str>)>,
    fingerprint: String,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn permission_id(permission: crate::capture::desktop::settings::onboarding::Permission) -> &'static str {
    use crate::capture::desktop::settings::onboarding::Permission;
    match permission {
        Permission::ScreenRecording => "screen-recording",
        Permission::Microphone => "microphone",
        Permission::Accessibility => "accessibility",
        Permission::Camera => "camera",
        Permission::SpeechRecognition => "speech-recognition",
        Permission::InputMonitoring => unreachable!("retired permission is not exposed"),
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn native_host_permission_actions(
    permission: crate::capture::desktop::settings::onboarding::Permission,
    onboarding_session_active: bool,
) -> Vec<&'static str> {
    #[cfg(target_os = "macos")]
    if !onboarding_session_active
        && !crate::capture::desktop::settings::onboarding::permission_helper::is_busy()
    {
        return vec!["request-permission", "open-system-settings"];
    }
    #[cfg(target_os = "linux")]
    if crate::capture::desktop::linux::privacy::settings_available(permission) {
        return vec!["open-system-settings"];
    }
    let _ = permission;
    let _ = onboarding_session_active;
    Vec::new()
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn restore_runtime_shortcut(
    telemetry: &SharedCaptureTelemetry,
    previous_record: &crate::capture::engine::runtime::ShortcutConfig,
    previous_paste: &crate::capture::engine::runtime::ShortcutConfig,
) -> bool {
    let (current_record, current_paste) = telemetry.shortcuts();
    let registered = crate::capture::engine::runtime::shortcut_registration_status(telemetry) == "registered";
    if current_record == *previous_record && current_paste == *previous_paste && registered {
        return true;
    }
    crate::capture::engine::runtime::set_shortcuts(telemetry, previous_record.clone(), previous_paste.clone()).is_ok()
        && crate::capture::engine::runtime::shortcut_registration_status(telemetry) == "registered"
}

impl ApparentCoordinator {
    pub(in crate::capture::desktop) fn new(
        telemetry: Arc<SharedCaptureTelemetry>,
        initial_bundle: Option<(PathBuf, ApparentBundle)>,
    ) -> Self {
        crate::capture::desktop::playback::warm_up();
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let native_host_epoch = crate::capture::desktop::workflow::agent_capture::new_id()
            .unwrap_or_else(|_| format!("process-{}", std::process::id()));
        NATIVE_HOST_REVISION_FOR_WORKER.store(0, std::sync::atomic::Ordering::Release);
        Self {
            telemetry,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            native_host_epoch,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            native_host_revision: 0,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            native_host_fingerprint: None,
            note_ui: None,
            admitted_microphone: None,
            whiteboard: Rc::new(RefCell::new(whiteboard::Board::default())),
            active_recording: None,
            capture_generation: 0,
            capture_local_practice: false,
            pending_finalizers: 0,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            agent_capture_id: None,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            agent_captures: crate::capture::desktop::workflow::agent_capture::Registry::default(),
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            public_share: crate::capture::desktop::workflow::public_sharing::Controller::default(),
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            public_popup: None,
            #[cfg(target_os = "linux")]
            agent_finishing: false,
            last_live_no_words: false,
            playback: None,
            picker_metal_overlay: None,
            native_pill: None,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            electron_pill: None,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            electron_pill_failed: false,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            electron_pill_routed: false,
            #[cfg(target_os = "macos")]
            electron_pill_exclusions: Vec::new(),
            #[cfg(target_os = "linux")]
            electron_equalizer: InputEqualizer::default(),
            iphone_preview: None,
            presentation: CapsulePresentation::new(),
            selectable_windows: Vec::new(),
            selection_generation: 0,
            windows_loading: false,
            selection_pointer: None,
            hovered_window: None,
            animated_window_rect: None,
            selection_drag: None,
            display_rect: [0.0, 0.0, 1440.0, 900.0],
            paste_drag_start: None,
            recording_shortcut_key: false,
            review_audio_issue: initial_bundle
                .as_ref()
                .and_then(|(dir, bundle)| bundle_audio_issue(dir, bundle)),
            review_bundle: initial_bundle,
            review_status: None,
            selected_tab: ReviewTab::Keyframes,
            selected_keyframe: 0,
            iphone: None,
            completed_dir: None,
            completed_note: None,
            share_online: false,
            share_probe_at: None,
            share_pending: None,
            share_waiting_account: false,
            share_login: None,
            share_copied: None,
            completed_upload_size_failure: None,
            last_local_capture_ms: 0,
            inbox: crate::capture::desktop::workflow::inbox::Inbox::load_default(),
            last_mode: SelectionMode::load_last_used(),
            open_iphone_when_found: false,
            onboarding: None,
            onboarding_editor: crate::capture::desktop::settings::Panel::default(),
            settings: None,
            grants: None,
            gate: crate::capture::desktop::settings::onboarding::Gate::default(),
            shortcut: crate::capture::desktop::settings::shortcut::Shortcut::default(),
            hold_mods: 0,
            lab: false,
        }
    }

    /// Remembers the mode a recording is starting in, so the next picker opens on it.
    pub(in crate::capture::desktop) fn remember_mode(&mut self, mode: SelectionMode) {
        if self.last_mode != mode {
            self.last_mode = mode;
            mode.save_last_used();
        }
    }

    /// Kit opened, copied, pasted or dismissed `dir`, so it leaves the inbox.
    pub(in crate::capture::desktop) fn mark_handled(&mut self, dir: &std::path::Path) {
        if self.inbox.remove(dir) {
            refresh_status_bar(self.inbox.len());
        }
    }

    /// What ⌥⇧V pastes: the bundle on the Completed island, else the newest unhandled iPhone
    /// capture unless this Mac captured something since, else the latest bundle.
    pub(in crate::capture::desktop) fn take_paste_target(&mut self) -> PathBuf {
        let showing = matches!(
            self.presentation.island_phase,
            IslandPhase::Completed { .. }
        )
        .then(|| self.completed_dir.clone())
        .flatten();
        let dir = showing
            .or_else(|| {
                self.inbox
                    .newest()
                    .filter(|entry| entry.received_at_ms > self.last_local_capture_ms)
                    .map(|entry| entry.dir.clone())
            })
            .unwrap_or_else(|| PathBuf::from("latest"));
        self.mark_handled(&dir);
        dir
    }

    pub fn paste_latest(&mut self, cx: &mut Context<Self>) {
        // Without Accessibility macOS drops the posted ⌘V (onboarding asks for it, but it can be
        // turned off later); bring the permissions window instead.
        if !crate::capture::engine::runtime::accessibility_granted() {
            apparent_log("⌥⇧V without Accessibility; asking for it");
            self.gate.defer(vec![
                crate::capture::desktop::settings::onboarding::Permission::Accessibility,
            ]);
            return;
        }
        let dir = self.take_paste_target();
        if matches!(
            self.presentation.island_phase,
            IslandPhase::Completed { .. }
        ) {
            self.dismiss_island(cx);
        }
        std::thread::spawn(move || {
            if let Err(err) = crate::shared::handoff::paste_bundle_image_and_text(&dir) {
                apparent_log(format!("paste_bundle_image_and_text error: {err}"));
                #[cfg(target_os = "linux")]
                crate::capture::desktop::linux::shell::notify(
                    "Paste stopped",
                    &format!(
                        "{err:#}. Manually attach the contact sheet and paste summary.md from {}",
                        dir.display()
                    ),
                );
            }
        });
    }

    /// Looks for a cabled iPhone in the background; the picker shows "iPhone" when one is there.
    pub(in crate::capture::desktop) fn refresh_iphone(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |entity, cx| {
            let found = cx
                .background_executor()
                .spawn(async {
                    crate::capture::desktop::helper::devices()
                        .inspect_err(|error| apparent_log(format!("iPhone discovery: {error:#}")))
                        .ok()
                        .and_then(|devices| devices.into_iter().next())
                })
                .await;
            let _ = entity.update(cx, |coord, cx| {
                if coord.iphone != found {
                    coord.iphone = found;
                    cx.notify();
                }
                if std::mem::take(&mut coord.open_iphone_when_found)
                    && coord.iphone.is_some()
                    && matches!(
                        coord.presentation.island_phase,
                        IslandPhase::Selecting {
                            mode: SelectionMode::Area | SelectionMode::Iphone
                        }
                    )
                {
                    coord.preview_iphone(cx);
                }
            });
        })
        .detach();
    }

    pub fn is_recording(&self) -> bool {
        self.active_recording.is_some()
    }

    pub fn is_processing(&self) -> bool {
        matches!(self.presentation.island_phase, IslandPhase::Processing)
    }

    pub(in crate::capture::desktop) fn refuse_capture_restart(&self) -> bool {
        if self.capture_busy() {
            let message = "Finish or cancel the capture before restarting Apparent.";
            apparent_log(message);
            if let Ok(mut status) = CAPTURE_STATUS.lock() {
                *status = Some(message.into());
            }
            true
        } else {
            false
        }
    }

    pub(in crate::capture::desktop) fn request_quit(&mut self, cx: &mut Context<Self>) {
        crate::capture::desktop::workflow::shutdown::request();
        self.lab = false;
        self.recording_shortcut_key = false;
        self.iphone_preview.take();
        if self.is_recording() {
            self.finish_recording(cx);
        } else if !self.is_processing() {
            self.cancel_recording(cx);
        }
        self.publish_capture_busy();
        if !self.capture_busy() {
            cx.quit();
        }
    }

    /// Native callbacks cannot borrow the GPUI entity. This is only a copy of the
    /// coordinator-owned admission decision, never separate lifecycle accounting.
    pub(in crate::capture::desktop) fn publish_capture_busy(&self) {
        crate::capture::desktop::workflow::shutdown::set_phase_busy(self.capture_busy());
    }

    pub fn is_selecting(&self) -> bool {
        matches!(
            self.presentation.island_phase,
            IslandPhase::Selecting { .. }
        )
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(in crate::capture::desktop) fn agent_state(
        &self,
    ) -> crate::capture::desktop::workflow::agent_capture::State {
        use crate::capture::desktop::workflow::agent_capture::{Phase, State};
        if matches!(
            self.active_recording
                .as_ref()
                .map(|recording| &recording.source),
            Some(CaptureSource::Lab)
        ) {
            return State {
                phase: Phase::Idle,
                capture_id: None,
            };
        }
        let phase = if let Some(recording) = &self.active_recording {
            match &recording.source {
                CaptureSource::Screen { session, .. } if !session.has_frames() => Phase::Starting,
                CaptureSource::Phone(session) if !session.has_recording() => Phase::Starting,
                _ => Phase::Recording,
            }
        } else {
            match self.presentation.island_phase {
                IslandPhase::Selecting { .. } => Phase::Selecting,
                IslandPhase::Processing => Phase::Processing,
                IslandPhase::Completed { .. } => Phase::Completed,
                _ => Phase::Idle,
            }
        };
        State {
            phase,
            capture_id: self.agent_capture_id.clone(),
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(in crate::capture::desktop) fn agent_request(
        &mut self,
        request: crate::capture::desktop::workflow::agent_capture::Request,
        cx: &mut Context<Self>,
    ) {
        use crate::capture::desktop::workflow::agent_capture::{Action, Outcome};
        if let Some(native_host) = request.native_host.as_ref() {
            if let Some(response) = self.native_host_request(native_host, request.expired(), request.reply.clone()) {
                let _ = request.reply.send(response.to_string());
            }
            return;
        }
        if let Some((
            action @ (Action::PublicShareStart
            | Action::PublicShareRead
            | Action::PublicShareDeliver
            | Action::PublicShareCancel),
            value,
        )) = &request.action
        {
            let selected = self.review_bundle.as_ref().and_then(|(path, _)| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| (path, name.to_owned()))
            });
            let response = if request.expired() {
                serde_json::json!({"error":"admission_expired"})
            } else {
                match action {
                    Action::PublicShareStart => match selected {
                        Some((path, name))
                            if name == *value
                                && !self.is_recording()
                                && !self.is_processing()
                                && !self.is_selecting() =>
                        {
                            match self.public_share.start(
                                name,
                                path.clone(),
                                self.capture_generation,
                            ) {
                                Ok(operation) => {
                                    serde_json::json!({"publicShare":{"operation":operation,"state":"preparing"}})
                                }
                                Err(code) => serde_json::json!({"error":code}),
                            }
                        }
                        _ => serde_json::json!({"error":"conflict"}),
                    },
                    Action::PublicShareRead => match selected {
                        Some((path, name)) => match self.public_share.read(
                            value,
                            &name,
                            path,
                            self.capture_generation,
                        ) {
                            Ok(state) => serde_json::json!({"publicShare":{"state":state}}),
                            Err(code) => serde_json::json!({"error":code}),
                        },
                        None => {
                            self.public_share.cancel(value);
                            serde_json::json!({"error":"no_selection"})
                        }
                    },
                    Action::PublicShareDeliver => match selected {
                        Some((path, name)) => match self.public_share.deliver(
                            value,
                            &name,
                            path,
                            self.capture_generation,
                            crate::shared::handoff::copy_string_to_clipboard,
                        ) {
                            Ok(state) => serde_json::json!({"publicShare":{"state":state}}),
                            Err(code) => serde_json::json!({"error":code}),
                        },
                        None => {
                            self.public_share.cancel(value);
                            serde_json::json!({"error":"no_selection"})
                        }
                    },
                    Action::PublicShareCancel => {
                        self.public_share.cancel(value);
                        serde_json::json!({"publicShare":{"state":"cancelled"}})
                    }
                    _ => unreachable!(),
                }
            };
            let _ = request.reply.send(response.to_string());
            return;
        }
        if let Some((action @ (Action::ReferenceRead | Action::ReferenceCopy), expected)) =
            &request.action
        {
            let enabled = crate::shared::handoff::experiment("copyCaptureId");
            let response = capture_reference_response(
                self.review_bundle
                    .as_ref()
                    .map(|(directory, _)| directory.as_path()),
                (*action == Action::ReferenceCopy).then_some(expected.as_str()),
                enabled,
                request.expired(),
                crate::shared::handoff::copy_string_to_clipboard,
            );
            let _ = request.reply.send(response.to_string());
            return;
        }
        if let Some((action @ (Action::DrawingRead | Action::DrawingUndo), expected)) =
            &request.action
        {
            use std::hash::{Hash, Hasher};
            let revision = self.active_recording.as_ref().filter(|r|
                matches!(&r.source, CaptureSource::Screen { session, .. } if session.has_frames())
            ).map(|r| {
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                r.started.hash(&mut hash);
                format!("{:016x}", hash.finish())
            });
            if let Err(error) = crate::capture::desktop::workflow::agent_capture::drawing_admission(
                revision.as_deref(),
                (*action == Action::DrawingUndo).then_some(expected.as_str()),
                request.expired(),
            ) {
                let _ = request
                    .reply
                    .send(serde_json::json!({"error":error}).to_string());
                return;
            }
            if *action == Action::DrawingUndo {
                self.undo_stroke(cx);
            }
            let count = {
                let strokes = self
                    .telemetry
                    .strokes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let desktop = self
                    .telemetry
                    .desktop_ink
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                desktop.len()
                    + strokes
                        .iter()
                        .filter(|s| !s.laser && s.pin.is_some())
                        .count()
            };
            let _ = request.reply.send(
                serde_json::json!({"drawing":{
                    "revision":revision, "strokes":if revision.is_some() { count } else { 0 }
                }})
                .to_string(),
            );
            return;
        }
        // Report an ended sampler, but never discard another session as a side
        // effect of State, Inspect or a competing Start. Cancellation stays explicit.
        if matches!(self.active_recording.as_ref().map(|r| &r.source),
            Some(CaptureSource::Screen { session, .. }) if session.failed_to_start())
        {
            if let Some(id) = &self.agent_capture_id {
                self.agent_captures
                    .set(&id, "failed", None, Some("start_failed"));
            }
        }
        let before = self.agent_state();
        if before.phase == crate::capture::desktop::workflow::agent_capture::Phase::Recording
            && let Some(id) = &before.capture_id
        {
            self.agent_captures.set(id, "recording", None, None);
        }
        if let Some((Action::Start, id)) = &request.action {
            let occupied = self.lab
                || self.active_recording.is_some()
                || self.is_selecting()
                || self.is_processing()
                || self.iphone_preview.is_some()
                || self.agent_capture_id.is_some()
                || self.recording_shortcut_key
                || crate::capture::desktop::workflow::shutdown::requested();
            if self.agent_captures.admit(id, occupied, request.expired()) {
                self.agent_capture_id = Some(id.clone());
                self.publish_capture_busy();
                self.record_screen_now(cx);
                if self.active_recording.is_none() {
                    self.agent_capture_id = None;
                    self.agent_captures
                        .set(id, "failed", None, Some("start_failed"));
                }
            }
            let _ = request
                .reply
                .send(self.agent_captures.inspect(id).to_string());
            return;
        }
        if let Some((Action::Inspect, id)) = &request.action {
            let _ = request
                .reply
                .send(self.agent_captures.inspect(id).to_string());
            return;
        }
        if request.action.is_some() && request.expired() {
            let _ = request.reply.send(
                serde_json::json!({"error": "admission_expired", "state": before}).to_string(),
            );
            return;
        }
        let outcome = request
            .action
            .as_ref()
            .map(|(action, id)| self.agent_captures.decide(&before, *action, id));
        match (&request.action, outcome) {
            (Some((Action::Stop, _)), Some(Outcome::Stopping)) => {
                #[cfg(target_os = "linux")]
                {
                    self.agent_finishing = true;
                }
                self.finish_recording(cx)
            }
            (Some((Action::Cancel, _)), Some(Outcome::Cancelled)) => self.cancel_recording(cx),
            _ => {}
        }
        let response = match outcome {
            Some(outcome) => {
                let mut response = self
                    .agent_captures
                    .inspect(&request.action.as_ref().unwrap().1);
                // Preserve non-wait Stop/Cancel's existing outcome semantics.
                response["outcome"] = serde_json::json!(outcome);
                response["state"] = serde_json::json!(self.agent_state());
                if outcome == Outcome::AlreadyEnded && response["status"] == "unknown" {
                    response["error"] = serde_json::Value::Null;
                }
                response
            }
            None => serde_json::json!({"state": before}),
        };
        let _ = request.reply.send(response.to_string());
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn native_host_request(
        &mut self,
        request: &crate::capture::desktop::workflow::native_host::Admitted,
        expired: bool,
        reply: std::sync::mpsc::SyncSender<String>,
    ) -> Option<serde_json::Value> {
        use crate::capture::desktop::settings::onboarding::Permission;
        use crate::capture::desktop::workflow::native_host::Operation;
        use crate::capture::engine::runtime::{CARBON_CMD_KEY, CARBON_CONTROL_KEY, CARBON_OPTION_KEY, CARBON_SHIFT_KEY, ShortcutKind};
        let refusal = |reason: &str| serde_json::json!({"status":"refused","reason":reason});
        if expired || crate::capture::desktop::workflow::agent_capture::storage_clock_ms() >= request.deadline {
            return Some(refusal("unavailable"));
        }
        if self.native_host_epoch.is_empty() { return Some(refusal("unavailable")); }
        if matches!(&request.operation, Operation::ReadStrokeColor) {
            if crate::capture::desktop::workflow::agent_capture::storage_clock_ms() >= request.deadline {
                return Some(refusal("unavailable"));
            }
            return Some(serde_json::json!({
                "api": 1,
                "processEpoch": self.native_host_epoch,
                "scope": "running-desktop-process",
                "rgb": self.telemetry.stroke_rgb(),
            }));
        }
        let observation = self.native_host_observation();
        self.refresh_native_host_revision(&observation.fingerprint);
        if !matches!(&request.operation, Operation::ReadSnapshot) {
            let Some(expected) = request.expected.as_ref() else { return Some(refusal("invalid")); };
            if expected.process_epoch != self.native_host_epoch || expected.revision != self.native_host_revision {
                return Some(refusal("stale"));
            }
        }
        if crate::capture::desktop::workflow::agent_capture::storage_clock_ms() >= request.deadline {
            return Some(refusal("unavailable"));
        }
        if let Operation::WriteClipboardText(text) = &request.operation {
            use std::sync::atomic::Ordering;
            use sha2::{Digest, Sha256};
            if let Some(reason) = crate::capture::desktop::workflow::native_host::clipboard_text_refusal(text) {
                return Some(refusal(reason));
            }
            let Some(expected) = request.expected.as_ref() else { return Some(refusal("unavailable")); };
            if expected.process_epoch != self.native_host_epoch || expected.revision != self.native_host_revision {
                return Some(refusal("stale"));
            }
            if NATIVE_CLIPBOARD_EFFECT_BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
                return Some(refusal("busy"));
            }
            let text = text.clone();
            let deadline = request.deadline;
            let expected_revision = expected.revision;
            let deadline_instant = std::time::Instant::now()
                + std::time::Duration::from_millis(deadline.saturating_sub(crate::capture::desktop::workflow::agent_capture::storage_clock_ms()));
            let reply = reply.clone();
            let worker = std::thread::Builder::new().name("apparent-native-clipboard-write".into()).spawn(move || {
                struct ReleaseClipboardLane;
                impl Drop for ReleaseClipboardLane {
                    fn drop(&mut self) {
                        NATIVE_CLIPBOARD_EFFECT_BUSY.store(false, Ordering::Release);
                    }
                }
                let _lane = ReleaseClipboardLane;
                let refused = |reason: &str| serde_json::json!({"status":"refused","reason":reason});
                let result = if crate::capture::desktop::workflow::agent_capture::storage_clock_ms() >= deadline {
                    refused("unavailable")
                } else if NATIVE_HOST_REVISION_FOR_WORKER.load(Ordering::Acquire) != expected_revision {
                    refused("stale")
                } else {
                    match crate::shared::handoff::copy_string_to_clipboard_verified_until(&text, deadline_instant) {
                        crate::shared::handoff::VerifiedClipboardTextWrite::Applied
                            if crate::capture::desktop::workflow::agent_capture::storage_clock_ms() < deadline => {
                                let digest = Sha256::digest(text.as_bytes());
                                serde_json::json!({"status":"applied","digest":format!("{digest:x}"),"byteLength":text.len()})
                            }
                        crate::shared::handoff::VerifiedClipboardTextWrite::Unavailable
                            if crate::capture::desktop::workflow::agent_capture::storage_clock_ms() < deadline => refused("unavailable"),
                        _ => serde_json::json!({"status":"outcome-unknown"}),
                    }
                };
                let _ = reply.send(result.to_string());
            });
            return match worker {
                Ok(_) => None,
                Err(_) => {
                    NATIVE_CLIPBOARD_EFFECT_BUSY.store(false, Ordering::Release);
                    Some(refusal("unavailable"))
                }
            };
        }
        Some(match &request.operation {
            Operation::ReadSnapshot => self.native_host_snapshot(&observation),
            Operation::RequestPermission(id) | Operation::OpenSystemSettings(id) => {
                let permission = match id.as_str() {
                    "screen-recording" => Permission::ScreenRecording,
                    "microphone" => Permission::Microphone,
                    "accessibility" => Permission::Accessibility,
                    "camera" => Permission::Camera,
                    "speech-recognition" => Permission::SpeechRecognition,
                    _ => return Some(refusal("invalid")),
                };
                let action = if matches!(&request.operation, Operation::RequestPermission(_)) { "request-permission" } else { "open-system-settings" };
                #[cfg(target_os = "macos")]
                if self.onboarding.is_some() { return Some(refusal("busy")); }
                if !observation.permission_actions.iter().any(|(key, actions)|
                    *key == permission_id(permission) && actions.contains(&action)) {
                    return Some(refusal("unavailable"));
                }
                #[cfg(target_os = "linux")]
                {
                    let deadline = request.deadline;
                    let reply = reply.clone();
                    return match std::thread::Builder::new().name("apparent-open-permission-settings".into()).spawn(move || {
                        let result = if crate::capture::desktop::workflow::agent_capture::storage_clock_ms() < deadline
                            && crate::capture::desktop::linux::privacy::open_settings(permission) {
                            serde_json::json!({"status":"accepted"})
                        } else {
                            serde_json::json!({"status":"refused","reason":"unavailable"})
                        };
                        let _ = reply.send(result.to_string());
                    }) {
                        Ok(_) => None,
                        Err(_) => Some(refusal("unavailable")),
                    };
                }
                #[cfg(target_os = "macos")]
                {
                    if self.onboarding.is_some() { return Some(refusal("busy")); }
                    let operation = if action == "request-permission" { "request" } else { "open-settings" };
                    match crate::capture::desktop::settings::onboarding::permission_helper::dispatch(
                        operation, permission, Some(request.deadline), Some(reply),
                    ) {
                        crate::capture::desktop::settings::onboarding::permission_helper::Dispatch::Queued => return None,
                        crate::capture::desktop::settings::onboarding::permission_helper::Dispatch::Busy => return Some(refusal("busy")),
                        crate::capture::desktop::settings::onboarding::permission_helper::Dispatch::Unavailable => return Some(refusal("unavailable")),
                    }
                }
            }
            Operation::ValidateRecordShortcut(key, modifiers) => {
                let supported_modifiers = CARBON_CMD_KEY | CARBON_CONTROL_KEY | CARBON_OPTION_KEY | CARBON_SHIFT_KEY;
                if modifiers & !supported_modifiers != 0 { return Some(refusal("invalid")); }
                let candidate = crate::capture::engine::runtime::shortcut_from_key_and_mods(
                    key, modifiers & CARBON_CMD_KEY != 0, modifiers & CARBON_CONTROL_KEY != 0,
                    modifiers & CARBON_OPTION_KEY != 0, modifiers & CARBON_SHIFT_KEY != 0,
                );
                let Some(candidate) = candidate else { return Some(refusal("invalid")); };
                let pressed = crate::capture::desktop::settings::Pressed {
                    key_code: candidate.key_code, modifiers: candidate.carbon_modifiers,
                    key: candidate.key_label().to_owned(),
                    reserved: crate::capture::desktop::settings::onboarding::host_shortcut_reserved(candidate.key_code, candidate.carbon_modifiers),
                };
                let paste = observation.paste.clone();
                match crate::capture::desktop::settings::validate(ShortcutKind::Record, &pressed, &paste) {
                    Ok(value) => serde_json::json!({"status":"accepted","shortcut":{"keyCode":value.key_code,"modifiers":value.carbon_modifiers,"label":value.label}}),
                    Err(crate::capture::desktop::settings::Rejection::Reserved(_)) => refusal("reserved"),
                    Err(_) => refusal("invalid"),
                }
            }
            Operation::ApplyStrokeColor(rgb, expected_pen_color) => {
                use std::sync::atomic::Ordering;
                if NATIVE_PEN_EFFECT_BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
                    return Some(refusal("busy"));
                }
                let telemetry = std::sync::Arc::clone(&self.telemetry);
                let config_path = crate::shared::handoff::default_config_path();
                let expected_pen_color = crate::shared::handoff::LegacyValue {
                    present: expected_pen_color.present,
                    value: expected_pen_color.value.clone(),
                };
                let rgb = *rgb;
                let deadline = request.deadline;
                let reply = reply.clone();
                let result = std::thread::Builder::new().name("apparent-native-pen-effect".into()).spawn(move || {
                    struct Release;
                    impl Drop for Release { fn drop(&mut self) { NATIVE_PEN_EFFECT_BUSY.store(false, Ordering::Release); } }
                    let _release = Release;
                    let allowed = || crate::capture::desktop::workflow::agent_capture::storage_clock_ms() < deadline;
                    let outcome = crate::shared::handoff::with_pen_color_current_allowed_in(
                        &config_path, expected_pen_color, &allowed, || {
                            anyhow::ensure!(allowed(), "pen effect deadline expired");
                            telemetry.set_stroke_rgb(rgb);
                            let actual = telemetry.stroke_rgb();
                            anyhow::ensure!(actual == rgb, "pen telemetry readback mismatch");
                            Ok(actual)
                        },
                    );
                    let response = match outcome {
                        Ok(crate::shared::handoff::PenColorEffectResult::Applied(actual)) => serde_json::json!({"status":"applied","rgb":actual}),
                        Ok(crate::shared::handoff::PenColorEffectResult::Conflict(current)) => serde_json::json!({"status":"conflict","current":{"present":current.present,"value":current.value}}),
                        Err(_) => serde_json::json!({"status":"outcome-unknown"}),
                    };
                    let _ = reply.send(response.to_string());
                });
                if result.is_err() {
                    NATIVE_PEN_EFFECT_BUSY.store(false, Ordering::Release);
                    return Some(refusal("unavailable"));
                }
                return None;
            }
            Operation::ReadStrokeColor => unreachable!("pen read was handled before config observation"),
            Operation::WriteClipboardText(_) => unreachable!("clipboard write was dispatched to its bounded worker"),
            Operation::SaveRecordShortcut(value) => self.native_host_save_shortcut(value, &observation, request.deadline),
        })
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn native_host_observation(&self) -> NativeHostObservation {
        use crate::capture::desktop::settings::onboarding::Permission;
        let (record, paste) = self.telemetry.shortcuts();
        let config_path = crate::shared::handoff::default_config_path();
        let (configured, _) = crate::capture::desktop::settings::Shortcuts::load_from(&config_path, std::path::Path::new("/dev/null"));
        let registration = crate::capture::engine::runtime::shortcut_registration_status(&self.telemetry);
        let can_save = registration == "registered" && !self.is_recording() && !self.is_processing() && !self.is_selecting();
        let permission_actions = [Permission::ScreenRecording, Permission::Microphone, Permission::Accessibility, Permission::Camera, Permission::SpeechRecognition]
            .into_iter().map(|permission| {
                let actions = native_host_permission_actions(permission, self.onboarding.is_some());
                (permission_id(permission), actions)
            }).collect::<Vec<_>>();
        let fingerprint = format!("{}|{}|{}|{}|{}|{}|{:?}|{:?}|{:?}",
            serde_json::to_string(&configured.record).unwrap_or_default(), serde_json::to_string(&configured.paste).unwrap_or_default(),
            serde_json::to_string(&record).unwrap_or_default(), serde_json::to_string(&paste).unwrap_or_default(),
            registration, can_save, self.grants, permission_actions, self.telemetry.stroke_rgb());
        NativeHostObservation { configured, record, paste, registration, can_save, permission_actions, fingerprint }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn native_host_snapshot(&self, observation: &NativeHostObservation) -> serde_json::Value {
        use crate::capture::desktop::settings::onboarding::{Permission, Status};
        let shortcut_fact = |cfg: &crate::capture::engine::runtime::ShortcutConfig, effective: &crate::capture::engine::runtime::ShortcutConfig| {
            let value = serde_json::json!({"keyCode":cfg.key_code,"modifiers":cfg.carbon_modifiers,"label":cfg.label});
            let effective_value = serde_json::json!({"keyCode":effective.key_code,"modifiers":effective.carbon_modifiers,"label":effective.label});
            serde_json::json!({"configured":value,"effective":if observation.registration == "registered" { effective_value } else { serde_json::Value::Null },
                "registration":observation.registration,"canValidate":true,"canSave":observation.can_save})
        };
        let permissions = [Permission::ScreenRecording, Permission::Microphone, Permission::Accessibility, Permission::Camera, Permission::SpeechRecognition]
            .into_iter().map(|permission| {
                let id = permission_id(permission);
                let status = match self.grants.as_ref().map(|g| g.status(permission)) {
                    None => "unavailable", Some(Status::Granted) => "granted", Some(Status::Denied) => "denied",
                    Some(Status::NotDetermined) => "not-determined",
                };
                let relaunch = permission == Permission::ScreenRecording && self.grants.as_ref().is_some_and(|g| g.screen_needs_relaunch);
                let actions = observation.permission_actions.iter().find(|(key, _)| *key == id)
                    .map(|(_, actions)| actions.clone()).unwrap_or_default();
                serde_json::json!({"id":id,"status":status,"usableNow":status == "granted" && !relaunch,"relaunchNeeded":relaunch,"actions":actions})
            }).collect::<Vec<_>>();
        serde_json::json!({"api":1,"processEpoch":self.native_host_epoch,"revision":self.native_host_revision,
            "scope":"running-desktop-process","freshness":"cached-gui-observation","permissions":permissions,
            "shortcuts":{"record":shortcut_fact(&observation.configured.record, &observation.record),"paste":shortcut_fact(&observation.configured.paste, &observation.paste)}})
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn refresh_native_host_revision(&mut self, fingerprint: &str) {
        if self.native_host_fingerprint.as_ref().is_some_and(|previous| previous != fingerprint) {
            self.native_host_revision = self.native_host_revision.saturating_add(1);
        }
        NATIVE_HOST_REVISION_FOR_WORKER.store(self.native_host_revision, std::sync::atomic::Ordering::Release);
        self.native_host_fingerprint = Some(fingerprint.to_owned());
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn native_host_save_shortcut(
        &mut self,
        shortcut: &crate::capture::desktop::workflow::native_host::Shortcut,
        observation: &NativeHostObservation,
        deadline: u64,
    ) -> serde_json::Value {
        use crate::capture::engine::runtime::ShortcutKind;
        use std::cell::Cell;
        let refusal = |reason: &str| serde_json::json!({"status":"refused","reason":reason});
        if self.is_recording() || self.is_processing() || self.is_selecting() { return refusal("busy"); }
        if observation.registration != "registered" { return refusal("unavailable"); }
        if crate::capture::desktop::workflow::agent_capture::storage_clock_ms() >= deadline {
            return refusal("unavailable");
        }
        let old_record = observation.record.clone();
        let paste = observation.paste.clone();
        let (current_record, current_paste) = self.telemetry.shortcuts();
        if current_record != old_record || current_paste != paste
            || crate::capture::engine::runtime::shortcut_registration_status(&self.telemetry) != observation.registration {
            return refusal("stale");
        }
        let config_path = crate::shared::handoff::default_config_path();
        let key = shortcut.label.trim_start_matches(|ch| matches!(ch, '⌘' | '⌃' | '⌥' | '⇧'));
        let pressed = crate::capture::desktop::settings::Pressed { key_code:shortcut.key_code, modifiers:shortcut.modifiers,
            key:key.to_owned(), reserved:crate::capture::desktop::settings::onboarding::host_shortcut_reserved(shortcut.key_code, shortcut.modifiers) };
        let validated = match crate::capture::desktop::settings::validate(ShortcutKind::Record, &pressed, &paste) {
            Ok(value) => value,
            Err(crate::capture::desktop::settings::Rejection::Reserved(_)) => return refusal("reserved"),
            Err(_) => return refusal("invalid"),
        };
        if validated.key_code != shortcut.key_code || validated.carbon_modifiers != shortcut.modifiers || validated.label != shortcut.label {
            return refusal("invalid");
        }
        let allowed = || crate::capture::desktop::workflow::agent_capture::storage_clock_ms() < deadline;
        let registration_refused = Cell::new(false);
        let write_result = crate::capture::desktop::settings::save_shortcut_if_current_allowed_in(
            &config_path,
            ShortcutKind::Record,
            &observation.configured.record,
            &validated,
            &allowed,
            || {
                if !allowed() { anyhow::bail!("native shortcut request expired before registration"); }
                if crate::capture::engine::runtime::set_shortcuts(&self.telemetry, validated.clone(), paste.clone()).is_err()
                    || crate::capture::engine::runtime::shortcut_registration_status(&self.telemetry) != "registered" {
                    registration_refused.set(true);
                    anyhow::bail!("shortcut registration refused");
                }
                Ok(())
            },
        );
        match write_result {
            Ok(false) => return refusal("stale"),
            Ok(true) => {}
            Err(error) => {
                let uncertain = error.is::<crate::shared::handoff::ConfigPublicationUncertain>();
                let runtime_restored = restore_runtime_shortcut(&self.telemetry, &old_record, &paste);
                let config_restored = if uncertain && runtime_restored {
                    crate::capture::desktop::settings::restore_shortcut_if_current_allowed_in(
                        &config_path, ShortcutKind::Record, &validated, &observation.configured.record, &allowed,
                    ).unwrap_or(false)
                } else { !uncertain };
                if runtime_restored && config_restored {
                    return if registration_refused.get() { refusal("registration-refused") }
                        else if !allowed() { refusal("busy") }
                        else { refusal("unavailable") };
                }
                return serde_json::json!({"status":"outcome-unknown"});
            }
        }
        let persisted = crate::capture::desktop::settings::Shortcuts::load_from(&config_path, std::path::Path::new("/dev/null")).0.record;
        if persisted != validated {
            let config_restored = crate::capture::desktop::settings::restore_shortcut_if_current_allowed_in(
                &config_path, ShortcutKind::Record, &validated, &observation.configured.record, &allowed,
            ).unwrap_or(false);
            let runtime_restored = restore_runtime_shortcut(&self.telemetry, &old_record, &paste);
            let persisted_restored = crate::capture::desktop::settings::Shortcuts::load_from(&config_path, std::path::Path::new("/dev/null")).0.record == observation.configured.record;
            if config_restored && runtime_restored && persisted_restored { return refusal("unavailable"); }
            return serde_json::json!({"status":"outcome-unknown"});
        }
        self.native_host_revision = self.native_host_revision.saturating_add(1);
        NATIVE_HOST_REVISION_FOR_WORKER.store(self.native_host_revision, std::sync::atomic::Ordering::Release);
        let (effective, _) = self.telemetry.shortcuts();
        if effective != validated || crate::capture::engine::runtime::shortcut_registration_status(&self.telemetry) != "registered" {
            return serde_json::json!({"status":"outcome-unknown"});
        }
        serde_json::json!({"status":"accepted"})
    }

    pub fn open_onboarding(&mut self, flow: crate::capture::desktop::settings::onboarding::Flow) {
        // Drop the old window before the new one opens.
        self.cancel_onboarding_editor();
        self.onboarding = None;
        let sender = self.telemetry.ui_sender.clone();
        let events = move |event| {
            let _ = sender.send(LiveOverlayEvent::Onboarding(event));
        };
        match crate::capture::desktop::settings::onboarding::Session::open(
            flow,
            &self.shortcut_label(),
            events,
        ) {
            Ok(session) => {
                self.onboarding = Some(session);
                self.sync_onboarding_editor();
            }
            Err(error) => apparent_log(format!("onboarding could not open: {error:#}")),
        }
    }

    /// Opens Settings, or brings it forward on `pane`.
    pub fn open_settings(&mut self, pane: Option<crate::capture::desktop::settings::Pane>) {
        if let Some(session) = self.settings.as_mut() {
            session.show(pane);
            return;
        }
        let sender = self.telemetry.ui_sender.clone();
        let events = move |event| {
            let _ = sender.send(LiveOverlayEvent::Settings(event));
        };
        match crate::capture::desktop::settings::Session::open(pane, self.grants.clone(), events) {
            Ok(mut session) => {
                session.sync(&crate::capture::desktop::settings::AppHost {
                    telemetry: &self.telemetry,
                });
                self.settings = Some(session);
            }
            Err(error) => apparent_log(format!("Settings could not open: {error:#}")),
        }
    }

    pub(in crate::capture::desktop) fn settings_event(
        &mut self,
        event: crate::capture::desktop::settings::Event,
        cx: &mut Context<Self>,
    ) {
        use crate::capture::desktop::settings::{Effect, Event};
        let Some(event) = self
            .settings
            .as_ref()
            .and_then(|session| session.current_event(event))
        else {
            return;
        };
        #[cfg(target_os = "linux")]
        if self.defer_update_restart(&event) {
            // Check before Session spawns its relaunch helper. Capture admission
            // and this decision run on the same UI thread.
            apparent_log("Update restart deferred until capture is idle");
            crate::capture::desktop::linux::shell::notify(
                "Update ready",
                "Finish or cancel the capture before restarting Apparent.",
            );
            return;
        }
        if let Event::Grants(grants) = &event {
            self.note_grants(grants);
        }
        let Some(session) = self.settings.as_mut() else {
            return;
        };
        let mut host = crate::capture::desktop::settings::AppHost {
            telemetry: &self.telemetry,
        };
        let effect = session.event(event, &mut host);
        let shortcut = self.shortcut_label();
        if let Some(onboarding) = self.onboarding.as_mut() {
            onboarding.shortcut_changed(&shortcut);
        }
        match effect {
            Effect::Close => {
                self.settings = None;
                self.cancel_share_login();
                if let Some((_, _, active)) = self.share_pending.take() {
                    active.store(false, std::sync::atomic::Ordering::SeqCst);
                }
                self.share_waiting_account = false;
            }
            Effect::ConnectHosted => {
                if self.share_login.is_none()
                    && crate::shared::handoff::experiment(crate::connect::hosted::HOSTED_SHARING)
                {
                    let active = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
                    self.share_login = Some(active.clone());
                    cx.spawn(async move |entity, cx| {
                        let worker_active = active.clone();
                        let result = cx
                            .background_executor()
                            .spawn(async move {
                                crate::connect::hosted::connect_sharing(&|| {
                                    worker_active.load(std::sync::atomic::Ordering::SeqCst)
                                })
                            })
                            .await;
                        let _ = entity.update(cx, |coord, _| {
                            if coord.finish_share_login(&active, result.is_ok()) {
                                crate::connect::hosted::start_worker();
                            }
                        });
                    })
                    .detach();
                }
            }
            Effect::ManageSharedLinks => {
                if let Some(url) = crate::connect::hosted::shared_links_url() {
                    let _ = crate::shared::handoff::open_path(std::path::Path::new(&url));
                }
            }
            Effect::Tour => {
                self.open_onboarding(crate::capture::desktop::settings::onboarding::Flow::tour(
                    crate::capture::desktop::settings::onboarding::Step::Welcome,
                    self.grants.clone(),
                ))
            }
            Effect::Relaunch => {
                if self.refuse_capture_restart() {
                    return;
                }
                self.settings = None;
                crate::capture::desktop::settings::onboarding::relaunch();
                cx.quit();
            }
            Effect::QuitForUpdate => {
                self.settings = None;
                cx.quit();
            }
            _ => {}
        }
    }

    #[cfg(target_os = "linux")]
    pub(in crate::capture::desktop) fn defer_update_restart(
        &self,
        event: &crate::capture::desktop::settings::Event,
    ) -> bool {
        matches!(
            event,
            crate::capture::desktop::settings::Event::Action(
                crate::capture::desktop::settings::Action::UpdateNow
            )
        ) && self.capture_busy()
    }

    /// UI-thread snapshot for restart/termination admission, not just visible UI.
    pub(crate) fn capture_busy(&self) -> bool {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if self.agent_capture_id.is_some() {
            return true;
        }
        self.pending_finalizers > 0
            || self.lab
            || self.active_recording.is_some()
            || self.is_selecting()
            || self.is_processing()
            || self.iphone_preview.is_some()
            || self.recording_shortcut_key
    }

    pub(in crate::capture::desktop) fn capture_finalizer_started(&mut self) {
        self.pending_finalizers += 1;
    }

    pub(in crate::capture::desktop) fn capture_finalizer_finished(&mut self) {
        self.pending_finalizers = self
            .pending_finalizers
            .checked_sub(1)
            .expect("finalizer completion must own a pending worker");
    }

    pub(in crate::capture::desktop) fn cancel_share_login(&mut self) {
        if let Some(active) = self.share_login.take() {
            active.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }

    pub(in crate::capture::desktop) fn finish_share_login(
        &mut self,
        active: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        succeeded: bool,
    ) -> bool {
        // A cancelled request can finish after Settings has started another login.
        // Only its owner may clear the cancellation handle or start the worker.
        if !self
            .share_login
            .as_ref()
            .is_some_and(|current| std::sync::Arc::ptr_eq(current, active))
        {
            return false;
        }
        self.share_login = None;
        succeeded && active.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Keeps a fresh read of the grants and remembers what was granted. (The key tap follows
    /// Accessibility by itself, in the HID poller.)
    pub(in crate::capture::desktop) fn note_grants(
        &mut self,
        grants: &crate::capture::desktop::settings::onboarding::Grants,
    ) {
        use crate::capture::desktop::settings::onboarding::Progress;
        let mut progress = Progress::load();
        if progress.note(grants) {
            progress.save();
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if self.grants.as_ref() != Some(grants) {
            self.native_host_revision = self.native_host_revision.saturating_add(1);
            NATIVE_HOST_REVISION_FOR_WORKER.store(self.native_host_revision, std::sync::atomic::Ordering::Release);
        }
        self.grants = Some(grants.clone());
    }

    /// Required permissions that keep a capture from starting now.
    pub(in crate::capture::desktop) fn missing_to_record(
        &mut self,
        microphone: bool,
    ) -> Vec<crate::capture::desktop::settings::onboarding::Permission> {
        use crate::capture::desktop::settings::onboarding::{Permission, missing_to_record};
        let screen = crate::capture::engine::runtime::screen_capture_usable(false);
        let accessibility = crate::capture::engine::runtime::accessibility_granted();
        let mut missing = missing_to_record(self.grants.as_ref(), screen, accessibility);
        if !microphone {
            missing.retain(|p| *p != Permission::Microphone);
        }
        // The last read may be old; read again before standing in the way.
        if missing.contains(&Permission::Microphone)
            && let Some(fresh) = crate::capture::desktop::settings::onboarding::read_grants()
        {
            self.note_grants(&fresh);
            return missing_to_record(self.grants.as_ref(), screen, accessibility);
        }
        missing
    }

    /// Whether this may go on: with a required permission off it starts nothing, and the
    /// permissions window comes forward once the modifiers are up.
    pub(in crate::capture::desktop) fn ready_to_record(&mut self) -> bool {
        if self.lab {
            return true;
        }
        let microphone = crate::capture::engine::runtime::audio::mic::enabled();
        let missing = self.missing_to_record(microphone);
        if missing.is_empty() {
            self.admitted_microphone = Some(microphone);
            return true;
        }
        apparent_log(format!(
            "Not recording: {missing:?} off; opening onboarding"
        ));
        self.gate.defer(missing);
        false
    }

    /// Whether `permission`, asked for on first use (Camera, Accessibility), is on. If not, its
    /// window comes forward once the modifiers are up.
    pub(in crate::capture::desktop) fn ready_for(
        &mut self,
        permission: crate::capture::desktop::settings::onboarding::Permission,
    ) -> bool {
        let granted =
            |g: &crate::capture::desktop::settings::onboarding::Grants| g.granted(permission);
        if self.lab || self.grants.as_ref().is_none_or(granted) {
            return true;
        }
        if let Some(fresh) = crate::capture::desktop::settings::onboarding::read_grants() {
            self.note_grants(&fresh);
            if granted(&fresh) {
                return true;
            }
        }
        apparent_log(format!("{permission:?} is off; asking for it"));
        self.gate.defer(vec![permission]);
        false
    }

    /// Polled while the gate holds a request: shows the permissions window once ⌥⇧ is up.
    pub fn tick_gate(&mut self) {
        use crate::capture::desktop::settings::onboarding::{Flow, Input, Mode, Permission};
        let Some(asks) = self
            .gate
            .tick(crate::capture::engine::runtime::modifier_flags() != 0)
        else {
            return;
        };
        let required = asks.iter().any(|p| p.required());
        match self.onboarding.as_mut() {
            Some(session) if session.mode() == Mode::Tour && required => {
                session.input(Input::Blocked);
            }
            _ => {
                let asks = if required {
                    Permission::REQUIRED.to_vec()
                } else {
                    asks
                };
                self.open_onboarding(Flow::fix(asks, self.grants.clone()));
            }
        }
    }

    pub(in crate::capture::desktop) fn onboarding_event(
        &mut self,
        event: crate::capture::desktop::settings::onboarding::Event,
        cx: &mut Context<Self>,
    ) {
        use crate::capture::desktop::settings::onboarding::{Effect, Event, Progress};
        let effect = match event {
            Event::Launch(grants) => {
                self.note_grants(&grants);
                let mut progress = Progress::load();
                let plan =
                    crate::capture::desktop::settings::onboarding::launch_plan(&progress, &grants);
                if std::mem::take(&mut progress.reopen_fix) {
                    progress.save();
                }
                if self.onboarding.is_none()
                    && let Some(flow) = plan
                {
                    self.open_onboarding(flow);
                }
                Effect::None
            }
            Event::Grants(grants) => {
                self.note_grants(&grants);
                // A single app-wide Mac poller feeds both windows and the capture gate.
                #[cfg(target_os = "macos")]
                if let Some(session) = self.settings.as_mut() {
                    session.event(
                        crate::capture::desktop::settings::Event::Grants(grants.clone()),
                        &mut crate::capture::desktop::settings::AppHost {
                            telemetry: &self.telemetry,
                        },
                    );
                }
                self.onboarding
                    .as_mut()
                    .map_or(Effect::None, |session| session.grants(grants))
            }
            Event::Action(action)
                if matches!(
                    action.as_str(),
                    "use-desktop-shortcut" | "use-current-shortcut"
                ) =>
            {
                let host = crate::capture::desktop::settings::AppHost {
                    telemetry: &self.telemetry,
                };
                let editor = self.onboarding_editor.record_shortcut(&host);
                let accepted = !editor.recording
                    && editor.error.is_none()
                    && (action == "use-current-shortcut" || editor.externally_managed);
                if accepted && let Some(session) = self.onboarding.as_mut() {
                    session.input(
                        crate::capture::desktop::settings::onboarding::Input::ShortcutChosen,
                    );
                }
                Effect::None
            }
            Event::Action(action) => self
                .onboarding
                .as_mut()
                .map_or(Effect::None, |session| session.action(&action)),
            Event::Shortcut(action) => {
                if self.onboarding.as_ref().is_some_and(|session| {
                    session.editing_shortcut()
                        || (session.choosing_shortcut()
                            && matches!(
                                action,
                                crate::capture::desktop::settings::Action::ResetShortcut { .. }
                            ))
                }) && matches!(
                    action,
                    crate::capture::desktop::settings::Action::Shortcut {
                        kind: crate::capture::engine::runtime::ShortcutKind::Record,
                        ..
                    } | crate::capture::desktop::settings::Action::ResetShortcut {
                        kind: crate::capture::engine::runtime::ShortcutKind::Record
                    } | crate::capture::desktop::settings::Action::CancelShortcut
                ) {
                    let mut host = crate::capture::desktop::settings::AppHost {
                        telemetry: &self.telemetry,
                    };
                    let choosing = matches!(
                        action,
                        crate::capture::desktop::settings::Action::Shortcut { .. }
                    ) && self.onboarding_editor.record_shortcut(&host).recording;
                    self.onboarding_editor.on(action, &mut host);
                    let editor = self.onboarding_editor.record_shortcut(&host);
                    if choosing
                        && !editor.recording
                        && editor.error.is_none()
                        && let Some(session) = self.onboarding.as_mut()
                    {
                        session.input(
                            crate::capture::desktop::settings::onboarding::Input::ShortcutChosen,
                        );
                    }
                    self.sync_onboarding_editor();
                }
                Effect::None
            }
            Event::Exited => {
                if self
                    .onboarding
                    .as_mut()
                    .is_some_and(|session| session.restore_native())
                {
                    self.sync_onboarding_editor();
                    return;
                }
                if let Some(session) = self.onboarding.as_ref() {
                    session.leave(true);
                }
                self.cancel_onboarding_editor();
                self.onboarding = None;
                Effect::None
            }
        };
        match effect {
            Effect::EditShortcut => {
                let mut host = crate::capture::desktop::settings::AppHost {
                    telemetry: &self.telemetry,
                };
                self.onboarding_editor.on(
                    crate::capture::desktop::settings::Action::RecordShortcut {
                        kind: crate::capture::engine::runtime::ShortcutKind::Record,
                    },
                    &mut host,
                );
                self.sync_onboarding_editor();
            }
            Effect::Close | Effect::Settings => {
                if let Some(session) = self.onboarding.as_ref() {
                    session.leave(false);
                }
                self.cancel_onboarding_editor();
                if effect == Effect::Settings {
                    self.open_settings(None);
                }
                self.onboarding = None;
            }
            Effect::Relaunch { resume } => {
                if self.refuse_capture_restart() {
                    return;
                }
                self.cancel_onboarding_editor();
                apparent_log(format!(
                    "Screen Recording granted but not usable here; relaunching (resume {resume:?})"
                ));
                let mut progress = Progress::load();
                progress.relaunching(
                    resume,
                    crate::capture::desktop::settings::onboarding::unix_ms(),
                );
                progress.save();
                self.onboarding = None;
                crate::capture::desktop::settings::onboarding::relaunch();
                cx.quit();
            }
            _ => {}
        }
        if self
            .onboarding
            .as_ref()
            .is_some_and(|session| !session.editing_shortcut())
        {
            self.cancel_onboarding_editor();
        }
    }

    pub(in crate::capture::desktop) fn sync_onboarding_editor(&mut self) {
        use crate::capture::desktop::settings::Host as _;
        let host = crate::capture::desktop::settings::AppHost {
            telemetry: &self.telemetry,
        };
        if let Some(session) = self.onboarding.as_mut() {
            session.shortcut_changed(&host.shortcuts().record.label);
            session.shortcut_editor(&self.onboarding_editor.record_shortcut(&host));
        }
    }

    pub(in crate::capture::desktop) fn cancel_onboarding_editor(&mut self) {
        let mut host = crate::capture::desktop::settings::AppHost {
            telemetry: &self.telemetry,
        };
        self.onboarding_editor.on(
            crate::capture::desktop::settings::Action::CancelShortcut,
            &mut host,
        );
    }

    pub(in crate::capture::desktop) fn activity(
        &self,
    ) -> crate::capture::desktop::settings::shortcut::Activity {
        use crate::capture::desktop::settings::shortcut::Activity;
        if let Some(rec) = &self.active_recording {
            Activity::Recording {
                elapsed_ms: rec.started.elapsed().as_millis() as u64,
            }
        } else if self.is_processing() {
            Activity::Processing
        } else if self.is_selecting() {
            Activity::Picker
        } else {
            Activity::Free
        }
    }

    pub(in crate::capture::desktop) fn begin_playback(&mut self) {
        if !self.lab && self.playback.is_none() {
            self.playback = crate::capture::desktop::playback::Guard::start();
        }
    }

    /// The capture shortcut went down (from Carbon, the event tap or the HID poller).
    pub fn press_shortcut(&mut self, cx: &mut Context<Self>) {
        let activity = self.activity();
        if self.gate.is_pending() {
            return;
        }
        if !self.shortcut.is_pending()
            && activity == crate::capture::desktop::settings::shortcut::Activity::Free
        {
            // No picker or overlay before the permissions are on: they sit above
            // macOS's permission prompts and take the clicks meant for them.
            let missing =
                self.missing_to_record(crate::capture::engine::runtime::audio::mic::enabled());
            if !self.gate.press(missing) {
                apparent_log("Shortcut held back: a required permission is off");
                return;
            }
            self.begin_playback();
            self.hold_mods = crate::capture::engine::runtime::modifier_flags();
            // An Esc from before this press doesn't cancel it.
            self.telemetry.take_escape();
        }
        let action = self.shortcut.press(activity);
        self.apply_shortcut(action, cx);
    }

    /// Polled from the UI loop while a press is pending: watches for ⌥⇧ coming up and Esc,
    /// whichever app or window has focus.
    pub fn tick_shortcut(&mut self, cx: &mut Context<Self>) {
        if !self.shortcut.is_pending() {
            return;
        }
        let action = if self.telemetry.take_escape() {
            self.shortcut.escape()
        } else {
            let mods = crate::capture::engine::runtime::modifier_flags();
            let held = self.hold_mods != 0 && mods & self.hold_mods == self.hold_mods;
            self.shortcut.tick(held)
        };
        self.apply_shortcut(action, cx);
    }

    pub(in crate::capture::desktop) fn apply_shortcut(
        &mut self,
        action: crate::capture::desktop::settings::shortcut::Action,
        cx: &mut Context<Self>,
    ) {
        use crate::capture::desktop::settings::shortcut::Action;
        if action != Action::None {
            apparent_log(format!("shortcut -> {action:?}"));
        }
        match action {
            Action::None => {}
            Action::Cancel => {
                if let Some(session) = self.onboarding.as_mut() {
                    session.input(
                        crate::capture::desktop::settings::onboarding::Input::CaptureEnded {
                            completed: false,
                        },
                    );
                }
                // Esc can cancel before the picker or a recording exists.
                self.playback.take();
            }
            Action::RecordScreen => {
                if let Some(session) = self.onboarding.as_mut() {
                    session.input(
                        crate::capture::desktop::settings::onboarding::Input::ShortcutTriggered,
                    );
                }
                self.record_screen_now(cx);
            }
            Action::RecordSelected => {
                if let Some(session) = self.onboarding.as_mut() {
                    session.input(
                        crate::capture::desktop::settings::onboarding::Input::ShortcutTriggered,
                    );
                }
                self.record_selected_mode(cx);
            }
            Action::Finish => self.finish_recording(cx),
        }
    }

    /// The menu's Record Screen item, the review window's Record button and `app --record`.
    pub fn record_or_finish(&mut self, cx: &mut Context<Self>) {
        if self.is_recording() {
            self.finish_recording(cx);
        } else if self.is_selecting() {
            self.record_selected_mode(cx);
        } else if !self.is_processing() {
            self.record_screen_now(cx);
        }
    }

    /// Records the display under the cursor straight away (no picker), auto-cropped.
    pub fn record_screen_now(&mut self, cx: &mut Context<Self>) {
        let target = if self.lab {
            lab::target()
        } else {
            discover_target(None)
        };
        self.start_recording_with_target(target, cx);
        if self.is_recording() {
            self.show_reveal(cx);
        }
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod native_pen_snapshot_tests {
    use super::*;

    #[test]
    fn read_stroke_color_returns_process_telemetry_without_changing_it() {
        use crate::capture::desktop::workflow::native_host::{Admitted, Operation};

        let (sender, _receiver) = flume::unbounded();
        let telemetry = std::sync::Arc::new(crate::capture::engine::runtime::SharedCaptureTelemetry::new(sender));
        let mut coordinator = ApparentCoordinator::new(std::sync::Arc::clone(&telemetry), None);
        let read = |coordinator: &mut ApparentCoordinator| {
            let (reply, _receive) = std::sync::mpsc::sync_channel(1);
            coordinator.native_host_request(
                &Admitted {
                    id: "read-1".into(),
                    deadline: crate::capture::desktop::workflow::agent_capture::storage_clock_ms() + 1000,
                    expected: None,
                    operation: Operation::ReadStrokeColor,
                },
                false,
                reply,
            ).expect("read returns synchronously")
        };

        telemetry.set_stroke_rgb(Some([59, 121, 189]));
        assert_eq!(read(&mut coordinator), serde_json::json!({
            "api":1,"processEpoch":coordinator.native_host_epoch,
            "scope":"running-desktop-process","rgb":[59,121,189]
        }));
        assert_eq!(telemetry.stroke_rgb(), Some([59, 121, 189]));

        telemetry.set_stroke_rgb(None);
        assert_eq!(read(&mut coordinator)["rgb"], serde_json::Value::Null);
        telemetry.set_stroke_rgb(Some([0, 0, 0]));
        assert_eq!(read(&mut coordinator)["rgb"], serde_json::json!([0, 0, 0]));
        assert_eq!(telemetry.stroke_rgb(), Some([0, 0, 0]));
    }
}
