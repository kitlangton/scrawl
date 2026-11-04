use super::*;

impl ApparentCoordinator {
    /// The menu bar's picker, opening on the mode the last recording used.
    pub fn begin_target_selection(&mut self, cx: &mut Context<Self>) {
        if self.ready_to_record() {
            self.open_picker(None, cx);
        }
    }

    /// Opens the picker on `mode`, else on the mode the last recording used.
    pub(in crate::capture::desktop) fn open_picker(
        &mut self,
        mode: Option<SelectionMode>,
        cx: &mut Context<Self>,
    ) {
        if mode == Some(SelectionMode::Window) {
            self.report_window_recording_disabled(cx);
            return;
        }
        if self.is_recording() || self.is_processing() {
            return;
        }
        apparent_log(format!("open_picker invoked (mode: {mode:?})"));
        let started = Instant::now();
        self.recording_shortcut_key = false;
        self.selectable_windows.clear();
        self.hovered_window = None;
        self.animated_window_rect = None;
        self.selection_pointer = current_mouse_screen();
        // The picker opens on the display under the pointer and follows it there
        // (`follow_pointer_display`); Screen mode highlights all of it.
        let screens = crate::capture::engine::runtime::screens::current();
        if let Some(i) = crate::capture::engine::runtime::screens::pointer()
            .and_then(|p| crate::capture::engine::runtime::screens::index_at(&screens, p))
        {
            self.display_rect = screens[i].frame.map(|v| v as f32);
        }
        if let Some(ref mut overlay) = self.picker_metal_overlay {
            overlay.move_to(self.display_rect.map(f64::from));
        }
        self.selection_generation += 1;
        self.windows_loading = !self.lab;
        if !self.lab {
            self.refresh_iphone(cx);
        }
        let generation = self.selection_generation;
        cx.spawn(async move |entity, cx| {
            if entity.read_with(cx, |coord, _| coord.lab).unwrap_or(true) {
                return;
            }
            let windows = cx
                .background_executor()
                .spawn(async { list_selectable_windows() })
                .await;
            let _ = entity.update(cx, |coord, cx| {
                // A previous picker request must not overwrite a reopened picker or recording.
                if coord.selection_generation != generation || !coord.is_selecting() {
                    return;
                }
                coord.windows_loading = false;
                let ptr = current_mouse_screen().or(coord.selection_pointer);
                coord.selection_pointer = ptr;
                coord.hovered_window = ptr
                    .and_then(|[x, y]| selectable_window_at(&windows, f64::from(x), f64::from(y)))
                    .cloned();
                if matches!(
                    coord.presentation.island_phase,
                    IslandPhase::Selecting {
                        mode: SelectionMode::Window
                    }
                ) {
                    coord.animated_window_rect = coord.hovered_window.as_ref().map(|w| {
                        [
                            w.frame.x as f32,
                            w.frame.y as f32,
                            w.frame.width as f32,
                            w.frame.height as f32,
                        ]
                    });
                }
                coord.selectable_windows = windows;
                coord.sync_picker_overlay();
                start_metric("picker_ready_ms", started);
                cx.notify();
            });
        })
        .detach();
        self.selection_drag = None;
        if matches!(
            self.presentation.island_phase,
            IslandPhase::Hidden | IslandPhase::Dismissing
        ) {
            // Enter near full size (0.92 -> 1.0) at full intrinsic layout width so inner buttons
            // are never clipped.
            self.presentation.island_width = self.presentation.island_target_width.max(195.0);
            self.presentation.island_velocity = 0.0;
            self.presentation.island_scale = 0.92;
            self.presentation.island_scale_velocity = 0.0;
            self.presentation.island_y = -6.0;
            self.presentation.island_y_velocity = 0.0;
            self.presentation.island_opacity = 0.0;
            self.presentation.island_phase = IslandPhase::Hidden;
        }
        // Open on the mode the last recording used. The phone needs a live preview, started once
        // the picker is up; Window starts on Area's crosshair until the window list arrives.
        let opening = match mode.unwrap_or(self.last_mode) {
            SelectionMode::Iphone => SelectionMode::Area,
            mode => mode,
        };
        self.set_island_phase(IslandPhase::Selecting { mode: opening });
        self.open_iphone_when_found = false;
        if mode.is_none() && self.last_mode == SelectionMode::Iphone {
            if self.iphone.is_some() {
                self.preview_iphone(cx);
            } else {
                self.open_iphone_when_found = true;
            }
        }
        self.telemetry.begin_picker_session();
        self.sync_picker_overlay();
        if self.picker_metal_overlay.is_none() && !self.lab {
            let [x, y, w, h] = self.display_rect.map(f64::from);
            let overlay =
                platform::open_overlay(Arc::clone(&self.telemetry), Some([x, y, w, h]), cx)
                    .map_err(|error| apparent_log(format!("Metal overlay failed: {error}")))
                    .ok();
            if let Some(ref ov) = overlay {
                self.telemetry
                    .excluded_window_ids
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(ov.window_number());
                self.telemetry
                    .passthrough_window_ids
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(ov.window_number());
            }
            self.picker_metal_overlay = overlay;
        }

        let entity = cx.entity().clone();
        cx.defer(move |cx| {
            close_review_window(cx);
            open_picker_windows(&entity, cx);
            start_metric("picker_open_ms", started);
        });
        cx.notify();
        start_metric("picker_prep_ms", started);
    }

    /// From the picker: the full screen it highlights.
    pub fn start_recording_full_screen(&mut self, cx: &mut Context<Self>) {
        let [x, y, w, h] = self.display_rect;
        let centre = [f64::from(x + w / 2.0), f64::from(y + h / 2.0)];
        let target = if self.lab {
            lab::target()
        } else {
            crate::capture::engine::runtime::discover_target_at(None, Some(centre))
        };
        self.start_recording_with_target(target, cx);
    }

    pub fn start_recording_with_target(&mut self, target: TargetInfo, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        self.reconcile_electron_pill();
        if crate::capture::desktop::workflow::shutdown::requested() {
            return;
        }
        if crate::capture::engine::runtime::ensure_recording_mode(target.mode).is_err() {
            self.report_window_recording_disabled(cx);
            return;
        }
        if self.is_recording() || self.is_processing() {
            return;
        }
        if self.lab {
            self.start_lab_recording(target, cx);
            return;
        }
        // No surprise prompt mid-capture: the permissions are granted in onboarding.
        if !self.ready_to_record() {
            self.cancel_recording(cx);
            return;
        }
        #[cfg(target_os = "linux")]
        if let Some(warning) = crate::capture::desktop::linux::media::encoder_warning() {
            apparent_log(&warning);
            crate::capture::desktop::linux::shell::notify(
                "Apparent setup: H.264 encoder missing",
                &warning,
            );
            self.cancel_recording(cx);
            return;
        }
        whiteboard::reset_for_capture(&self.whiteboard);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let Ok(id) = self
                .agent_capture_id
                .clone()
                .map_or_else(crate::capture::desktop::workflow::agent_capture::new_id, Ok)
            else {
                apparent_log("Could not create capture identity; refusing an untracked recording");
                self.cancel_recording(cx);
                return;
            };
            self.agent_captures.begin(&id);
            self.agent_capture_id = Some(id);
        }
        self.begin_playback();
        if let Ok(mut status) = CAPTURE_STATUS.lock() {
            *status = None;
        }
        let started = Instant::now();
        apparent_log(format!(
            "start_recording_with_target invoked: mode={:?} frame={:?}",
            target.mode, target.frame
        ));
        // Without the picker the start needs its own cue: the overlay flashes an edge glow.
        let quick = !self.is_selecting();
        self.telemetry.start_glow.store(
            quick && target.mode == CaptureMode::Display,
            std::sync::atomic::Ordering::Release,
        );
        if let IslandPhase::Selecting { mode } = self.presentation.island_phase {
            self.remember_mode(mode);
        }
        self.recording_shortcut_key = false;
        self.iphone_preview = None;
        self.selection_drag = None;
        self.hovered_window = None;
        self.animated_window_rect = None;
        let id = crate::shared::ids::make_bundle_id();
        let root = std::env::var_os("APPARENT_BENCH_START")
            .map(PathBuf::from)
            .unwrap_or_else(crate::shared::handoff::default_captures_dir);
        let bundle_dir = root.join(format!("{id}.apparent"));
        if let Err(error) =
            crate::capture::engine::bundle::recover::begin_screen_capture(&bundle_dir)
        {
            apparent_log(format!("Capture could not protect raw video: {error:#}"));
            workflow::error_reporting::report_error(
                workflow::error_reporting::Failure::ProtectRecording,
                &error,
            );
            self.cancel_recording(cx);
            if let Ok(mut status) = CAPTURE_STATUS.lock() {
                *status = Some("Capture failed — could not protect raw video".into());
            }
            return;
        }
        let video_path = bundle_dir.join("recording.mp4");

        self.telemetry.set_capture_mode(target.mode);
        self.telemetry.reset_for_session(target.frame);
        #[cfg(target_os = "linux")]
        if let Some(id) = self
            .electron_pill
            .as_ref()
            .and_then(ui::electron_pill::Session::window_number)
        {
            self.telemetry
                .excluded_window_ids
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(id);
        }
        #[cfg(target_os = "macos")]
        self.telemetry
            .excluded_window_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .append(&mut self.electron_pill_exclusions);
        // The picker stays open until present_recording_island defers its close.
        // Keep it out of the first ScreenCaptureKit frames after the session reset.
        #[cfg(target_os = "macos")]
        restore_picker_exclusion(
            &mut self
                .telemetry
                .excluded_window_ids
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            platform::picker_window_number(),
        );
        let stage = Instant::now();
        // Reuse the continuous 120Hz Metal overlay opened in `begin_target_selection` so the
        // cutout and border never swap windows or jump a single pixel when recording starts.
        // It moves to the target's display if that's another one (a window picked there).
        let f = target.frame;
        let target_rect = [f.x, f.y, f.width, f.height];
        let screens = crate::capture::engine::runtime::screens::current();
        self.display_rect = crate::capture::engine::runtime::screens::index_at(
            &screens,
            [f.x + f.width / 2.0, f.y + f.height / 2.0],
        )
        .map_or(target_rect, |i| screens[i].frame)
        .map(|v| v as f32);
        if let Some(ref pill) = self.native_pill {
            pill.place_on([f.x + f.width / 2.0, f.y + f.height / 2.0]);
        }
        #[cfg(target_os = "macos")]
        {
            platform::cancel_pill_drag();
            if self.native_pill.is_none() {
                platform::place_hud_pill([f.x + f.width / 2.0, f.y + f.height / 2.0]);
            }
        }
        #[cfg(target_os = "linux")]
        {
            platform::cancel_pill_drag();
            platform::place_hud_pill([f.x + f.width / 2.0, f.y + f.height / 2.0]);
        }
        let metal_overlay = self.picker_metal_overlay.take().map(|mut overlay| {
            overlay.move_to(target_rect);
            // reset_for_session cleared the picker's exclusion. Reusing its window
            // must not turn the ink layer into captured screen content on macOS.
            self.telemetry
                .excluded_window_ids
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(overlay.window_number());
            overlay
        });
        let mut metal_overlay = metal_overlay.or_else(|| {
            let ov = platform::open_overlay(Arc::clone(&self.telemetry), Some(target_rect), cx)
                .map_err(|error| apparent_log(format!("Metal overlay failed: {error}")))
                .ok();
            if let Some(ref o) = ov {
                self.telemetry
                    .excluded_window_ids
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(o.window_number());
                self.telemetry
                    .passthrough_window_ids
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(o.window_number());
            }
            ov
        });
        #[cfg(target_os = "macos")]
        if target.mode != CaptureMode::Device {
            if let Some(overlay) = metal_overlay.as_mut() {
                match overlay.cover_desktop() {
                    Ok(ids) => {
                        self.telemetry
                            .excluded_window_ids
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .extend(ids.iter().copied());
                        self.telemetry
                            .passthrough_window_ids
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .extend(ids);
                    }
                    Err(error) => apparent_log(format!("Desktop ink overlays failed: {error}")),
                }
            }
        }
        #[cfg(target_os = "linux")]
        if target.mode != CaptureMode::Device {
            if let Some(overlay) = metal_overlay.as_mut() {
                overlay.move_to(target_rect);
                match overlay.cover_desktop(cx) {
                    Ok(ids) => {
                        self.telemetry
                            .excluded_window_ids
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .extend(ids.iter().copied());
                        self.telemetry
                            .passthrough_window_ids
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .extend(ids);
                    }
                    Err(error) => apparent_log(format!("Desktop ink overlays failed: {error}")),
                }
            }
        }
        start_metric("metal_open_ms", stage);
        let stage = Instant::now();
        self.publish_capture_busy();
        let microphone_enabled = self
            .admitted_microphone
            .take()
            .unwrap_or_else(crate::capture::engine::runtime::audio::mic::enabled);
        if !microphone_enabled {
            // Durable admission intent survives orphan recovery; it is not inferred from an empty WAV.
            if let Err(error) = std::fs::write(bundle_dir.join("microphone-off"), b"") {
                apparent_log(format!("Could not preserve silent capture intent: {error}"));
                self.cancel_recording(cx);
                return;
            }
        }
        let audio_recorder = if !microphone_enabled {
            None
        } else {
            match AudioRecorder::start(&bundle_dir) {
                Ok(recorder) => Some(recorder),
                Err(error) => {
                    let message =
                        format!("Capture refused: narration could not be saved: {error:#}");
                    workflow::error_reporting::report_error(
                        workflow::error_reporting::Failure::AudioStart,
                        &error,
                    );
                    apparent_log(&message);
                    self.cancel_recording(cx);
                    if let Ok(mut status) = CAPTURE_STATUS.lock() {
                        *status = Some(message);
                    }
                    return;
                }
            }
        };
        start_metric("audio_start_ms", stage);
        let stage = Instant::now();
        let screen_session = match ScreenCaptureSession::start(
            target.clone(),
            Some(video_path),
            Arc::clone(&self.telemetry.excluded_window_ids),
        ) {
            Ok(session) => session,
            Err(error) => {
                apparent_log(format!("Capture refused: {error:#}"));
                workflow::error_reporting::report_error(
                    workflow::error_reporting::Failure::ScreenStart,
                    &error,
                );
                if let Err(cleanup) =
                    crate::capture::engine::bundle::recover::abandon_start(&bundle_dir, || {
                        drop(audio_recorder)
                    })
                {
                    apparent_log(format!(
                        "Failed capture startup cleanup at {}: {cleanup:#}",
                        bundle_dir.display()
                    ));
                }
                self.cancel_recording(cx);
                return;
            }
        };
        start_metric("sck_start_ms", stage);
        let window_log = (target.mode == CaptureMode::Display).then(|| {
            let started_at = *self
                .telemetry
                .started_at
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            crate::shared::windows::WindowLogger::start(
                target.frame,
                started_at,
                Arc::clone(&self.telemetry.window_log_request),
            )
        });
        self.active_recording = Some(ActiveRecording {
            started: Instant::now(),
            target,
            bundle_dir,
            source: CaptureSource::Screen {
                session: screen_session,
                audio: audio_recorder,
                windows: window_log,
            },
            _metal_overlay: metal_overlay,
            microphone_enabled,
        });
        match workflow::notes::Session::start(
            &self.active_recording.as_ref().unwrap().bundle_dir,
            Arc::clone(&self.telemetry),
            self.active_recording.as_ref().unwrap().target.frame,
        ) {
            Ok(session) => self.note_ui = Some(session),
            Err(error) => {
                apparent_log(format!("Notes UI unavailable: {error:#}"));
                if let Ok(mut status) = CAPTURE_STATUS.lock() {
                    *status = Some("Typed notes unavailable · recording continues".into());
                }
            }
        }
        self.present_recording_island(cx);
        start_metric("recording_start_ms", started);
    }

    /// The island turns into the recording pill: from the picker it morphs, from nothing it opens
    /// from a compact capsule so the shell springs out to its width, Dynamic Island style.
    pub(in crate::capture::desktop) fn present_recording_island(&mut self, cx: &mut Context<Self>) {
        if matches!(self.presentation.island_phase, IslandPhase::Hidden) {
            self.presentation.island_width = ISLAND_OPEN_FROM_W;
            self.presentation.island_velocity = 0.0;
            self.presentation.island_scale = 0.92;
            self.presentation.island_scale_velocity = 0.0;
            self.presentation.island_y = -6.0;
            self.presentation.island_y_velocity = 0.0;
        }
        self.set_island_phase(IslandPhase::Recording);
        let entity = cx.entity().clone();
        cx.defer(move |cx| {
            close_review_window(cx);
            close_picker_window(cx);
            ensure_hud_pill_open(&entity, cx);
        });
        cx.notify();
    }

    /// `apparent lab pill`: a pretend recording with no capture, microphone, files or overlay, so
    /// the capsule goes through exactly the states a real one does.
    pub(in crate::capture::desktop) fn start_lab_recording(
        &mut self,
        target: TargetInfo,
        cx: &mut Context<Self>,
    ) {
        whiteboard::reset_for_capture(&self.whiteboard);
        if let IslandPhase::Selecting { mode } = self.presentation.island_phase {
            motion::trace_event(format!("record:{}", mode.label()));
        }
        self.iphone_preview = None;
        self.selection_drag = None;
        self.hovered_window = None;
        self.animated_window_rect = None;
        self.active_recording = Some(ActiveRecording {
            started: Instant::now(),
            target,
            bundle_dir: PathBuf::new(),
            microphone_enabled: false,
            source: CaptureSource::Lab,
            _metal_overlay: None,
        });
        self.present_recording_island(cx);
    }

    /// Mirrors the cabled iPhone in the helper's window and records its native video. The
    /// window's on-screen rect is the capture target, so the laser, ink, pins and clicks over it
    /// normalize straight into phone-screen coordinates (kept current as the window moves).
    pub fn start_recording_iphone(&mut self, cx: &mut Context<Self>) {
        if self.is_recording() || self.is_processing() {
            return;
        }
        if let Ok(mut status) = CAPTURE_STATUS.lock() {
            *status = None;
        }
        apparent_log("start_recording_iphone invoked");
        if self.lab {
            self.start_lab_recording(
                crate::capture::engine::runtime::phone::phone_target("iPhone", None, None, 0, 0),
                cx,
            );
            return;
        }
        if !self.ready_for(crate::capture::desktop::settings::onboarding::Permission::Camera) {
            self.cancel_recording(cx);
            return;
        }
        whiteboard::reset_for_capture(&self.whiteboard);
        self.begin_playback();
        self.remember_mode(SelectionMode::Iphone);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let Ok(id) = crate::capture::desktop::workflow::agent_capture::new_id() else {
                apparent_log("Could not create capture identity; refusing an untracked recording");
                self.cancel_recording(cx);
                return;
            };
            self.agent_captures.begin(&id);
            self.agent_capture_id = Some(id);
        }
        self.recording_shortcut_key = false;
        self.selection_drag = None;
        self.hovered_window = None;
        self.animated_window_rect = None;
        let bundle_dir = crate::shared::handoff::default_captures_dir()
            .join(format!("{}.apparent", crate::shared::ids::make_bundle_id()));
        let _ = std::fs::create_dir_all(&bundle_dir);
        let movie_path = bundle_dir.join("recording.mov");
        let existing_preview = self.iphone_preview.take();
        if let Some(dev) = existing_preview.as_ref().and_then(PhoneSession::device) {
            self.iphone = Some(dev);
        }
        let device = self.iphone.clone();
        let name = device
            .as_ref()
            .map_or("iPhone", |d| d.name.as_str())
            .to_owned();
        let target = crate::capture::engine::runtime::phone::phone_target(&name, None, None, 0, 0);
        let initial_frame = existing_preview
            .as_ref()
            .and_then(PhoneSession::rect)
            .map_or(
                TargetFrame {
                    x: -100_000.0,
                    y: -100_000.0,
                    width: 1.0,
                    height: 1.0,
                    scale: 2.0,
                },
                |[rx, ry, rw, rh]| TargetFrame {
                    x: f64::from(rx),
                    y: f64::from(ry),
                    width: f64::from(rw),
                    height: f64::from(rh),
                    scale: 2.0,
                },
            );
        self.telemetry.set_capture_mode(CaptureMode::Device);
        self.telemetry.reset_for_session(initial_frame);
        // Hold interaction telemetry inactive until the helper's `recording` event arrives.
        self.telemetry
            .active
            .store(false, std::sync::atomic::Ordering::Release);
        let session = if let Some(mut preview) = existing_preview {
            preview.begin_recording(&movie_path);
            preview
        } else {
            let dev_id = device.as_ref().map(|d| d.id.as_str());
            match PhoneSession::start(&movie_path, dev_id, Some(Arc::clone(&self.telemetry))) {
                Ok(session) => session,
                Err(error) => {
                    apparent_log(format!("iPhone capture could not start: {error:#}"));
                    workflow::error_reporting::report_error(
                        workflow::error_reporting::Failure::PhoneStart,
                        &error,
                    );
                    #[cfg(any(target_os = "macos", target_os = "linux"))]
                    {
                        if let Some(id) = self.agent_capture_id.take() {
                            self.agent_captures
                                .set(&id, "failed", None, Some("start_failed"));
                        }
                    }
                    self.telemetry.finish_session();
                    let _ = std::fs::remove_dir_all(&bundle_dir);
                    self.dismiss_island(cx);
                    return;
                }
            }
        };
        // Over the mirror window's display (the pointer's until the window reports its rect).
        let mirror = (initial_frame.x > -10_000.0).then_some([
            initial_frame.x,
            initial_frame.y,
            initial_frame.width,
            initial_frame.height,
        ]);
        let metal_overlay = self.picker_metal_overlay.take().or_else(|| {
            platform::open_overlay(Arc::clone(&self.telemetry), mirror, cx)
                .map_err(|error| apparent_log(format!("Metal overlay failed: {error}")))
                .ok()
        });
        self.active_recording = Some(ActiveRecording {
            started: Instant::now(),
            target,
            bundle_dir,
            source: CaptureSource::Phone(session),
            _metal_overlay: metal_overlay,
            microphone_enabled: true,
        });
        self.present_recording_island(cx);
    }

    /// The mirror ended without being asked: keep what was recorded, or drop an empty capture.
    pub(in crate::capture::desktop) fn phone_ended(&mut self, cx: &mut Context<Self>) {
        if self.iphone_preview.is_some() {
            self.iphone_preview = None;
            self.set_selection_mode(SelectionMode::Area, cx);
            return;
        }
        let recorded = match &self.active_recording {
            Some(ActiveRecording {
                source: CaptureSource::Phone(session),
                ..
            }) => session.has_recording(),
            _ => return,
        };
        if recorded {
            self.finish_recording(cx);
        } else {
            self.cancel_recording(cx);
        }
    }

    pub fn cancel_recording(&mut self, cx: &mut Context<Self>) {
        self.cancel_capture_attempt();
        if self.is_recording() {
            return;
        }
        cx.defer(whiteboard::close);
        self.dismiss_island(cx);
    }

    /// Cancellation must also work before a visible island or recording exists.
    pub(in crate::capture::desktop) fn cancel_capture_attempt(&mut self) {
        // dismiss_island is a no-op while Hidden: startup failure must revoke the
        // shortcut attempt here, not wait for a visible phase transition.
        if let Some(session) = self.onboarding.as_mut() {
            session.input(
                crate::capture::desktop::settings::onboarding::Input::CaptureEnded {
                    completed: false,
                },
            );
        }
        self.playback.take();
        apparent_log("cancel_recording invoked");
        self.discard_recording();
    }

    pub fn finish_recording(&mut self, cx: &mut Context<Self>) {
        self.playback.take();
        apparent_log("finish_recording invoked");
        let Some(rec) = self.active_recording.take() else {
            return;
        };
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let finishing_id = self.agent_capture_id.take();
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if let Some(id) = &finishing_id {
            self.agent_captures.set(id, "processing", None, None);
        }
        #[cfg(target_os = "linux")]
        let auto_active = !std::mem::take(&mut self.agent_finishing)
            && crate::shared::handoff::experiment(
                crate::capture::desktop::linux::auto_paste::EXPERIMENT,
            )
            && crate::capture::desktop::linux::hyprland::running()
            && crate::capture::desktop::linux::wayland_session()
            && !self.lab
            && matches!(&rec.source, CaptureSource::Screen { .. });
        #[cfg(not(target_os = "linux"))]
        let auto_active = false;
        let notes_finished = self.note_ui.take().map(workflow::notes::Session::finish);
        let (events, strokes) = self.telemetry.finish_session();
        let bundle_dir = rec.bundle_dir;
        let target = rec.target;
        let source = rec.source;
        // Drop the Metal overlay immediately on the main thread so the screen frame clears,
        // and morph the Dynamic Island into the compact spinning "Processing…" state while
        // media teardown, transcription, and PNG encoding run on a background worker.
        drop(rec._metal_overlay);
        self.set_island_phase(IslandPhase::Processing);
        let generation = self.capture_generation;
        // Freeze practice policy with the worker, independent of window/UI lifetime.
        let local_practice = self.capture_local_practice;
        self.completed_note = None;
        cx.notify();
        if matches!(source, CaptureSource::Lab) {
            cx.defer(whiteboard::close);
            let duration_ms = rec.started.elapsed().as_millis() as u64;
            let wait = Duration::from_secs_f32(lab::PROCESSING_S / motion::time_scale());
            cx.spawn(async move |entity, cx| {
                cx.background_executor().timer(wait).await;
                let _ = entity.update(cx, |coord, cx| {
                    if !coord.accepts_capture_result(generation) {
                        return;
                    }
                    coord.set_island_phase(IslandPhase::Completed {
                        finished_at: Instant::now(),
                        frame_count: 6,
                        duration_ms,
                        inbox_count: 0,
                    });
                    cx.notify();
                });
            })
            .detach();
            return;
        }

        let final_whiteboard =
            whiteboard::finish_snapshot(&self.whiteboard, || whiteboard::close(cx));

        self.capture_finalizer_started();
        self.publish_capture_busy();
        cx.spawn(async move |entity, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    recording_completion::finalize(
                        source,
                        target,
                        bundle_dir,
                        events,
                        strokes,
                        final_whiteboard,
                        notes_finished,
                    )
                })
                .await;

            let _ = entity.update(cx, |coord, cx| {
                let (finalized, duration_ms) = result;
                if finalized.is_err() {
                    crate::capture::desktop::workflow::shutdown::clear_request();
                }
                // Release the worker even when its capsule was dismissed or
                // a newer capture owns the UI. A completion never auto-quits.
                coord.capture_finalizer_finished();
                coord.publish_capture_busy();
                coord.receive_capture_result(
                    generation,
                    #[cfg(any(target_os = "macos", target_os = "linux"))]
                    finishing_id.as_deref(),
                    finalized,
                    |dir| {
                        if !local_practice {
                            desktop_completion::connect_completed(dir);
                        }
                    },
                    |coord, finalized| {
                        coord.present_capture_result(finalized, duration_ms, auto_active, cx);
                    },
                );
            });
        })
        .detach();
    }

    pub(in crate::capture::desktop) fn accepts_capture_result(&self, generation: u64) -> bool {
        generation == self.capture_generation && self.is_processing()
    }
}
