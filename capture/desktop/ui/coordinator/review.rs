use super::*;

pub(in crate::capture::desktop) fn review_neighbor<'a>(
    history: &'a [PathBuf],
    current: &std::path::Path,
    older: bool,
) -> Option<&'a PathBuf> {
    let index = history.iter().position(|path| path == current)?;
    if older {
        history.get(index + 1)
    } else {
        index.checked_sub(1).and_then(|index| history.get(index))
    }
}

pub(in crate::capture::desktop) fn review_capture_id(
    directory: &std::path::Path,
) -> anyhow::Result<&str> {
    directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("Capture directory has no UTF-8 name"))
}

pub(in crate::capture::desktop) fn capture_reference_response(
    directory: Option<&std::path::Path>,
    expected: Option<&str>,
    enabled: bool,
    expired: bool,
    copy: impl FnOnce(&str) -> anyhow::Result<()>,
) -> serde_json::Value {
    if expired {
        return serde_json::json!({"error":"admission_expired"});
    }
    if !enabled {
        return serde_json::json!({"error":"feature_disabled"});
    }
    let Some(directory) = directory else {
        return if expected.is_some() {
            serde_json::json!({"error":"no_selection"})
        } else {
            serde_json::json!({"reference":{"id":null,"copied":false}})
        };
    };
    let Ok(id) = review_capture_id(directory) else {
        return serde_json::json!({"error":"invalid_capture"});
    };
    if let Some(expected) = expected {
        if expected != id {
            return serde_json::json!({"error":"conflict"});
        }
        if copy(id).is_err() {
            return serde_json::json!({"error":"clipboard_failed"});
        }
    }
    serde_json::json!({"reference":{"id":id,"copied":expected.is_some()}})
}

pub(in crate::capture::desktop) fn review_notice(
    history_error: Option<&str>,
    status: Option<&str>,
) -> Option<String> {
    status
        .filter(|message| {
            message.starts_with("Could not open capture:")
                || *message == "Capture ID copied"
                || message.starts_with("Could not copy capture ID:")
        })
        .map(str::to_owned)
        .or_else(|| history_error.map(|error| format!("Could not read captures: {error}")))
}

/// Pair the frame with its words; stack them when the window cannot fit both.
pub(in crate::capture::desktop) fn review_workspace_widths(window_width: f32) -> (f32, f32) {
    if window_width < 800.0 {
        ((window_width - 40.0).max(1.0), window_width)
    } else {
        let words = (window_width * 0.4).clamp(320.0, 520.0);
        ((window_width - words - 40.0).max(1.0), words)
    }
}

pub(in crate::capture::desktop) fn review_frame_speech<'a>(
    frames: &[Option<&'a str>],
    selected: usize,
) -> &'a str {
    frames
        .get(selected)
        .and_then(|said| *said)
        .filter(|said| !said.trim().is_empty())
        .unwrap_or("No words at this moment.")
}

pub(in crate::capture::desktop) fn review_speech_status(
    no_words: bool,
    missing_reason: Option<&str>,
) -> Option<String> {
    if let Some(reason) = missing_reason {
        Some(format!("Transcription unavailable: {reason}"))
    } else if no_words {
        Some("No words transcribed. Your audio was saved—check it or try again closer to the microphone.".into())
    } else {
        None
    }
}

/// A point dragged past the picker's display (global points), pulled back onto its edge: an
/// area belongs to the display the drag started on, which the picker stays on while dragging.

pub struct BundleReviewWindow {
    pub(in crate::capture::desktop) coordinator: Entity<ApparentCoordinator>,
    pub(in crate::capture::desktop) focus: FocusHandle,
    pub(in crate::capture::desktop) cached_sheet_path: Option<PathBuf>,
    pub(in crate::capture::desktop) cached_sheet_image: Option<Arc<Image>>,
    pub(in crate::capture::desktop) cached_frames: Vec<Option<Arc<Image>>>,
    pub(in crate::capture::desktop) history: Vec<PathBuf>,
    pub(in crate::capture::desktop) history_error: Option<String>,
}

impl BundleReviewWindow {
    pub(in crate::capture::desktop) fn new(
        coordinator: Entity<ApparentCoordinator>,
        focus: FocusHandle,
    ) -> Self {
        Self {
            coordinator,
            focus,
            cached_sheet_path: None,
            cached_sheet_image: None,
            cached_frames: Vec::new(),
            history_error: None,
            history: Vec::new(),
        }
    }

    pub(in crate::capture::desktop) fn update_history(
        &mut self,
        result: anyhow::Result<Vec<PathBuf>>,
        cx: &mut Context<Self>,
    ) {
        let error = result.as_ref().err().map(ToString::to_string);
        if let Ok(paths) = result {
            if self.history != paths || self.history_error.is_some() {
                self.history = paths;
                self.history_error = None;
                cx.notify();
            }
        } else if self.history_error != error {
            self.history_error = error;
            cx.notify();
        }
    }

    pub(in crate::capture::desktop) fn ensure_images(
        &mut self,
        dir: &std::path::Path,
        bundle: &ApparentBundle,
    ) {
        let sheet_path = dir.join(&bundle.contact_sheet);
        if self.cached_sheet_path.as_ref() != Some(&sheet_path) {
            self.cached_sheet_path = Some(sheet_path.clone());
            self.cached_sheet_image = std::fs::read(&sheet_path)
                .ok()
                .map(|bytes| Arc::new(Image::from_bytes(ImageFormat::Png, bytes)));
            self.cached_frames = bundle
                .keyframes
                .iter()
                .map(|kf| {
                    let bytes = std::fs::read(dir.join(&kf.file)).ok()?;
                    Some(Arc::new(Image::from_bytes(ImageFormat::Png, bytes)))
                })
                .collect();
        }
    }
}

impl Render for BundleReviewWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (stage_w, words_w) = review_workspace_widths(f32::from(window.bounds().size.width));
        let narrow = f32::from(window.bounds().size.width) < 800.0;
        let compact = f32::from(window.bounds().size.width) < 600.0;
        let narrow_stage_height = (f32::from(window.bounds().size.height) * 0.46).max(220.0);
        let coord_read = self.coordinator.read(cx);
        let state = coord_read.review_bundle.clone();
        let audio_issue = coord_read.review_audio_issue;
        let status = coord_read.review_status.clone();
        let tab = coord_read.selected_tab;
        let selected_kf = coord_read.selected_keyframe;
        let shortcut_label = coord_read.shortcut_label();
        let recording_shortcut = coord_read.recording_shortcut_key;

        let Some((dir, bundle)) = state else {
            let c_start = self.coordinator.clone();
            let c_demo = self.coordinator.clone();
            let c_keydown = self.coordinator.clone();
            return div()
                .id("apparent-review-empty")
                .relative()
                .track_focus(&self.focus)
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    let handled = c_keydown
                        .update(cx, |coord, cx| coord.handle_app_key(&event.keystroke, cx));
                    let key = event.keystroke.key.as_str();
                    if !handled
                        && (key == "escape"
                            || (event.keystroke.modifiers.secondary()
                                && key.eq_ignore_ascii_case("w")))
                    {
                        window.remove_window();
                    } else if !handled {
                        return;
                    }
                    cx.stop_propagation();
                })
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .bg(rgb(CANVAS_BG))
                .text_color(rgb(TEXT_COLOR))
                .child(
                    div()
                        .text_size(px(16.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Apparent"),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            hint_button("empty-record", "Record", Some(shortcut_label), true)
                                .on_click(move |_, _, cx| {
                                    c_start.update(cx, |coord, cx| coord.record_or_finish(cx));
                                }),
                        )
                        .child(
                            action_button("empty-demo", "Load Demo Bundle", false).on_click(
                                move |_, _, cx| {
                                    if let Ok((dir, bundle, _)) =
                                        crate::capture::desktop::demo::generate_demo_bundle(None)
                                    {
                                        c_demo.update(cx, |coord, cx| {
                                            coord.set_review_bundle(
                                                (dir, bundle),
                                                Some("Generated demo bundle".into()),
                                            );
                                            cx.notify();
                                        });
                                    }
                                },
                            ),
                        ),
                )
                .child(running_build_badge())
                .into_any_element();
        };

        self.ensure_images(&dir, &bundle);
        let history_position = self.history.iter().position(|path| path == &dir);
        let newer = history_position
            .and_then(|_| review_neighbor(&self.history, &dir, false))
            .cloned();
        let older = history_position
            .and_then(|_| review_neighbor(&self.history, &dir, true))
            .cloned();
        let history_count = self.history.len();
        let frame_rows = bundle
            .keyframes
            .iter()
            .enumerate()
            .map(|(idx, frame)| {
                let c_select = self.coordinator.clone();
                div()
                    .id(("review-frame-row", idx))
                    .w_full()
                    .p_3()
                    .rounded(px(8.0))
                    .bg(rgb(if tab == ReviewTab::Keyframes && idx == selected_kf {
                        SURFACE_BG
                    } else {
                        SIDEBAR_BG
                    }))
                    .border_1()
                    .border_color(rgb(if tab == ReviewTab::Keyframes && idx == selected_kf {
                        ACCENT_COLOR
                    } else {
                        LINE_COLOR
                    }))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        c_select.update(cx, |coord, cx| {
                            coord.selected_tab = ReviewTab::Keyframes;
                            coord.selected_keyframe = idx;
                            cx.notify();
                        });
                    })
                    .child(format!("{}  ·  {}", frame.index, format_clock(frame.t)))
                    .child(
                        div()
                            .mt_1()
                            .text_size(px(12.0))
                            .text_color(rgb(MUTED_COLOR))
                            .child(frame.event.clone()),
                    )
            })
            .collect::<Vec<_>>();
        let frame_words = bundle
            .keyframes
            .iter()
            .map(|kf| kf.said.as_deref())
            .collect::<Vec<_>>();
        let review_notice = review_notice(self.history_error.as_deref(), status.as_deref());
        let no_words = audio_issue == Some(CompletedAudioIssue::NoWords);
        let no_audio = audio_issue == Some(CompletedAudioIssue::NoAudio);
        let missing_reason = if audio_issue == Some(CompletedAudioIssue::TranscriptionUnavailable) {
            bundle.transcript.missing.as_ref()
        } else {
            None
        };
        let speech_status = if no_audio {
            Some("No microphone audio was saved. The screen capture is still available—check the microphone and try again.".into())
        } else if audio_issue == Some(CompletedAudioIssue::TranscriptionUnavailable)
            && missing_reason.is_none()
        {
            Some("Transcription unavailable: recognition did not run".into())
        } else {
            review_speech_status(
                no_words,
                missing_reason.map(|missing| missing.reason.as_str()),
            )
        };

        let target_label = match (&bundle.target.app, &bundle.target.window_title) {
            (Some(app), Some(win)) if !win.is_empty() && win != app => {
                format!("{app} — “{win}”")
            }
            (Some(app), _) => app.clone(),
            _ => format!("Display {}", bundle.target.display_id),
        };
        let timeline = timeline(&bundle);

        let c_keydown = self.coordinator.clone();
        let c_shortcut = self.coordinator.clone();
        let external_shortcut = crate::capture::desktop::settings::external_record_owned(
            crate::capture::engine::runtime::ShortcutKind::Record,
        );
        let c_copy = self.coordinator.clone();
        let c_copy_id = self.coordinator.clone();
        let c_send = self.coordinator.clone();
        let c_tab_sheet = self.coordinator.clone();
        let c_tab_frames = self.coordinator.clone();
        let c_newer = self.coordinator.clone();
        let c_older = self.coordinator.clone();
        let copy_dir = dir.clone();
        let copy_id_dir = dir.clone();
        let send_dir = dir.clone();

        div()
            .id("apparent-review-root")
            .relative()
            .track_focus(&self.focus)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                let mods = event.keystroke.modifiers;
                let handled = c_keydown.update(cx, |coord, cx| {
                    if coord.recording_shortcut_key {
                        if key == "escape" {
                            coord.recording_shortcut_key = false;
                            cx.notify();
                            return true;
                        }
                        if let Some(cfg) = shortcut_from_key_and_mods(
                            key,
                            mods.platform,
                            mods.control,
                            mods.alt,
                            mods.shift,
                        ) {
                            update_global_shortcut(&coord.telemetry, cfg);
                            coord.recording_shortcut_key = false;
                            cx.notify();
                            return true;
                        }
                    }
                    if coord.handle_app_key(&event.keystroke, cx) {
                        return true;
                    }
                    if !mods.platform && !mods.control && !mods.alt && !mods.shift
                        && coord.selected_tab == ReviewTab::Keyframes
                    {
                        let count = coord.review_bundle.as_ref().map_or(0, |(_, b)| b.keyframes.len());
                        let next = match key {
                            "left" => coord.selected_keyframe.checked_sub(1),
                            "right" => coord.selected_keyframe.checked_add(1).filter(|&i| i < count),
                            _ => None,
                        };
                        if let Some(next) = next {
                            coord.selected_keyframe = next;
                            cx.notify();
                            return true;
                        }
                    }
                    if let Some(cfg) = shortcut_from_key_and_mods(
                        key,
                        mods.platform,
                        mods.control,
                        mods.alt,
                        mods.shift,
                    ) && coord.telemetry.matches_shortcut(
                        cfg.key_code as u16,
                        mods.platform,
                        mods.control,
                        mods.alt,
                        mods.shift,
                    ) {
                        coord.telemetry.emit_capture_shortcut();
                        return true;
                    }
                    false
                });
                if !handled
                    && (key == "escape" || (mods.secondary() && key.eq_ignore_ascii_case("w")))
                {
                    window.remove_window();
                } else if !handled {
                    return;
                }
                cx.stop_propagation();
            })
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(CANVAS_BG))
            .text_color(rgb(TEXT_COLOR))
            .child(
                div()
                    .h(px(48.0))
                    .pl(px(platform::HEADER_LEAD))
                    // Reserve the corner for the passive build label, including +modified.
                    .pr(px(platform::HEADER_TRAIL.max(144.0)))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .bg(rgb(SIDEBAR_BG))
                    .border_b_1()
                    .border_color(rgb(LINE_COLOR))
                    .map(platform::window_drag_region)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_3()
                            .when(!compact, |header| header.child(
                                div()
                                    .flex_none()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(14.0))
                                    .child("Capture review"),
                            ))
                            .when(!compact, |header| header.child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.0))
                                    .text_color(rgb(MUTED_COLOR))
                                    .child(format!(
                                        "{} · {} · {} frames",
                                        target_label,
                                        format_clock(bundle.duration_ms),
                                        bundle.keyframes.len(),
                                    )),
                            ))
                            .child(
                                div()
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .when_some(newer, |d, path| {
                                        d.child(
                                            action_button("review-newer", if compact { "←" } else { "← Newer" }, false)
                                                .on_click(move |_, _, cx| {
                                                    open_review_capture(&c_newer, &path, cx);
                                                }),
                                        )
                                    })
                                    .when_some(history_position.filter(|_| !compact), |d, index| {
                                        d.child(format!("{} / {}", index + 1, history_count))
                                    })
                                    .when_some(older, |d, path| {
                                        d.child(
                                            action_button("review-older", if compact { "→" } else { "Older →" }, false)
                                                .on_click(move |_, _, cx| {
                                                    open_review_capture(&c_older, &path, cx);
                                                }),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(crate::shared::handoff::experiment("copyCaptureId"), |actions| {
                                actions.child(
                                    action_button("btn-copy-capture-id", if compact { "Copy ID" } else { "Copy capture ID" }, false)
                                        .on_click(move |_, _, cx| {
                                            let result = review_capture_id(&copy_id_dir)
                                                .and_then(crate::shared::handoff::copy_string_to_clipboard);
                                            c_copy_id.update(cx, |coord, cx| {
                                                coord.review_status = Some(match result {
                                                    Ok(()) => "Capture ID copied".into(),
                                                    Err(error) => format!("Could not copy capture ID: {error}"),
                                                });
                                                cx.notify();
                                            });
                                        }),
                                )
                            })
                            .child(
                                action_button(
                                    "btn-copy",
                                    if status.as_deref() == Some("Copied ✓") {
                                        "Copied"
                                    } else {
                                        "Copy"
                                    },
                                    true,
                                )
                                .on_click(move |_, _, cx| {
                                    let res = crate::shared::handoff::copy_bundle_to_clipboard(&copy_dir);
                                    c_copy.update(cx, |coord, cx| {
                                        if res.is_ok() {
                                            coord.mark_handled(&copy_dir);
                                        }
                                        coord.review_status = Some(match res {
                                            Ok(()) => "Copied ✓".into(),
                                            Err(e) => format!("Copy failed: {e}"),
                                        });
                                        cx.notify();
                                    });
                                }),
                            ),
                    )
                    .children(platform::caption_buttons()),
            )
            .child(
                div()
                    .h(px(if compact { 76.0 } else { 44.0 }))
                    .flex_none()
                    .px_5()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .when(compact, |toolbar| toolbar.flex_col().items_start().justify_center().gap_1())
                    .border_b_1()
                    .border_color(rgb(LINE_COLOR))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                tab_chip(
                                    "tab-contact-sheet",
                                    "Contact Sheet",
                                    tab == ReviewTab::ContactSheet,
                                )
                                .on_click(move |_, _, cx| {
                                    c_tab_sheet.update(cx, |coord, cx| {
                                        coord.selected_tab = ReviewTab::ContactSheet;
                                        cx.notify();
                                    });
                                }),
                            )
                            .child(
                                tab_chip("tab-keyframes", "Keyframes", tab == ReviewTab::Keyframes)
                                    .on_click(move |_, _, cx| {
                                        c_tab_frames.update(cx, |coord, cx| {
                                            coord.selected_tab = ReviewTab::Keyframes;
                                            cx.notify();
                                        });
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                hint_button(
                                    "btn-shortcut",
                                    if external_shortcut { "Managed in Hyprland" } else if recording_shortcut { "Press shortcut…" } else { "Shortcut" },
                                    (!recording_shortcut && !compact).then_some(shortcut_label),
                                    false,
                                )
                                .when(!external_shortcut, |button| button.on_click(move |_, _, cx| {
                                    c_shortcut.update(cx, |coord, cx| {
                                        coord.recording_shortcut_key = !coord.recording_shortcut_key;
                                        cx.notify();
                                    });
                                })),
                            )
                            .child(
                                action_button(
                                    "btn-reveal",
                                    &if compact { "Show Files".to_owned() } else { format!("Show in {}", platform::FILE_MANAGER) },
                                    false,
                                )
                                .on_click(move |_, _, cx| {
                                    let res = crate::shared::handoff::open_path(&send_dir);
                                    let manager = platform::FILE_MANAGER;
                                    c_send.update(cx, |coord, cx| {
                                        coord.review_status = Some(match res {
                                            Ok(()) => format!("Opened in {manager}"),
                                            Err(e) => format!("Show in {manager} failed: {e}"),
                                        });
                                        cx.notify();
                                    });
                                }),
                            ),
                    ),
            )
            .when_some(review_notice, |root, message| {
                root.child(
                    div()
                        .px_5()
                        .py_2()
                        .text_size(px(12.0))
                        .text_color(rgb(TEXT_COLOR))
                        .child(message),
                )
            })
            .child(
                // Keep the selected frame and its words together, even in a narrow window.
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .when(narrow, |workspace| workspace.flex_col())
                    .child(
                        div()
                            .id("review-stage-scroll")
                            .min_w_0()
                            .when(narrow, |stage| stage.h(px(narrow_stage_height)).flex_none())
                            .when(!narrow, |stage| stage.flex_1().h_full())
                            .overflow_y_scroll()
                            .overflow_x_hidden()
                            .p_5()
                            .child(match tab {
                                ReviewTab::ContactSheet => self.render_contact_sheet_stage(stage_w),
                                ReviewTab::Keyframes => {
                                    self.render_keyframes_stage(&bundle, selected_kf, stage_w)
                                }
                            }),
                    )
                    .child(
                        div()
                            .id("review-timeline-sidebar")
                            .when(narrow, |sidebar| sidebar.w_full().flex_1().min_h_0())
                            .when(!narrow, |sidebar| sidebar.w(px(words_w)).h_full().flex_none())
                            .overflow_x_hidden()
                            .bg(rgb(SIDEBAR_BG))
                            .when(narrow, |sidebar| sidebar.border_t_1())
                            .when(!narrow, |sidebar| sidebar.border_l_1())
                            .border_color(rgb(LINE_COLOR))
                            .px_4()
                            .py(px(18.0))
                            .flex()
                            .flex_col()
                            .child(sidebar_heading(if tab == ReviewTab::Keyframes {
                                "WORDS AT THIS MOMENT"
                            } else {
                                "FULL TRANSCRIPT"
                            }))
                            .when(tab == ReviewTab::Keyframes, |sidebar| {
                                sidebar.child(
                                    div()
                                        .id("review-selected-speech")
                                        .p_4()
                                        .max_h(px(240.0))
                                        .overflow_y_scroll()
                                        .rounded(px(8.0))
                                        .bg(rgb(SURFACE_BG))
                                        .border_1()
                                        .border_color(rgb(LINE_COLOR))
                                        .text_size(px(15.0))
                                        .line_height(relative(1.5))
                                        .child(review_frame_speech(&frame_words, selected_kf).to_owned()),
                                )
                            })
                            .when(tab == ReviewTab::Keyframes, |sidebar| {
                                sidebar.when_some(speech_status, |sidebar, message| {
                                    sidebar.child(
                                        div()
                                            .mt_3()
                                            .p_3()
                                            .rounded(px(8.0))
                                            .bg(rgba(0xf5a62320))
                                            .text_color(rgb(0xf5b544))
                                            .text_size(px(13.0))
                                            .child(message),
                                    )
                                })
                            })
                            .when(tab == ReviewTab::ContactSheet && no_words, |sidebar| {
                                sidebar.child(
                                    div()
                                        .mt_3()
                                        .p_3()
                                        .rounded(px(8.0))
                                        .bg(rgba(0xf5a62320))
                                        .text_color(rgb(0xf5b544))
                                        .text_size(px(13.0))
                                        .child("No words transcribed. Your audio was saved—check it or try again closer to the microphone."),
                                )
                            })
                            .when(tab == ReviewTab::ContactSheet && no_audio, |sidebar| {
                                sidebar.child(
                                    div().mt_3().p_3().rounded(px(8.0))
                                        .bg(rgba(0xf5a62320)).text_color(rgb(0xf5b544))
                                        .text_size(px(13.0))
                                        .child("No microphone audio was saved. The screen capture is still available—check the microphone and try again."),
                                )
                            })
                            .when(tab == ReviewTab::ContactSheet && !no_words && (!no_audio || !bundle.transcript.text.trim().is_empty()), |sidebar| {
                                sidebar.child(
                                    div()
                                        .id("review-full-transcript")
                                        .p_3()
                                        .max_h(px(240.0))
                                        .overflow_y_scroll()
                                        .rounded(px(8.0))
                                        .bg(rgb(SURFACE_BG))
                                        .border_1()
                                        .border_color(rgb(LINE_COLOR))
                                        .text_size(px(13.0))
                                        .line_height(relative(1.45))
                                        .map(|d| {
                                            let text = bundle.transcript.text.trim();
                                            if text.is_empty() {
                                                d.text_color(rgb(MUTED_COLOR)).child(
                                                    missing_reason
                                                        .map(|missing| format!("Transcription unavailable: {}", missing.reason))
                                                        .unwrap_or_else(|| match audio_issue {
                                                            Some(CompletedAudioIssue::NoAudio) => "No audio",
                                                            Some(CompletedAudioIssue::TranscriptionUnavailable) => "Transcription unavailable: recognition did not run",
                                                            _ => "No speech",
                                                        }.into()),
                                                )
                                            } else {
                                                d.text_color(rgb(TEXT_COLOR)).child(format!("“{text}”"))
                                            }
                                        }),
                                )
                            })
                            .child(
                                div()
                                    .id("review-evidence-list")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .child(div().mt(px(20.0)).child(sidebar_heading("FRAMES")))
                                    .child(div().flex().flex_col().gap_2().children(frame_rows))
                                    .child(div().mt(px(20.0)).child(sidebar_heading("TIMELINE")))
                                    .child(
                                        div().mx(px(-8.0)).flex().flex_col().gap(px(1.0)).children(
                                            timeline
                                                .into_iter()
                                                .enumerate()
                                                .map(|(i, line)| timeline_row(i, &line)),
                                        ),
                                    ),
                            ),
                    ),
            )
            .child(running_build_badge())
            .into_any_element()
    }
}

impl BundleReviewWindow {
    pub(in crate::capture::desktop) fn render_contact_sheet_stage(
        &self,
        stage_w: f32,
    ) -> AnyElement {
        if let Some(ref image) = self.cached_sheet_image {
            div()
                .w_full()
                .min_w_0()
                .flex()
                .justify_center()
                .child(
                    img(Arc::clone(image))
                        .w(px(stage_w))
                        .rounded(px(10.0))
                        .border_1()
                        .border_color(rgb(LINE_COLOR)),
                )
                .into_any_element()
        } else {
            div()
                .p_6()
                .text_color(rgb(MUTED_COLOR))
                .child("Contact sheet image unavailable.")
                .into_any_element()
        }
    }

    pub(in crate::capture::desktop) fn render_keyframes_stage(
        &self,
        bundle: &ApparentBundle,
        selected_kf: usize,
        stage_w: f32,
    ) -> AnyElement {
        let Some(frame) = bundle.keyframes.get(selected_kf) else {
            return div()
                .p_6()
                .child("No frames in this capture.")
                .into_any_element();
        };
        let image = self.cached_frames.get(selected_kf).and_then(Clone::clone);
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.0))
                    .text_color(rgb(MUTED_COLOR))
                    .child(format!(
                        "Frame {} / {}",
                        selected_kf + 1,
                        bundle.keyframes.len()
                    ))
                    .child(format_clock(frame.t))
                    .child(frame.event.clone()),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .rounded(px(10.0))
                    .bg(rgb(SURFACE_BG))
                    .border_1()
                    .border_color(rgb(LINE_COLOR))
                    .when_some(image, |d, image| {
                        d.child(img(image).w(px(stage_w)).rounded(px(9.0)))
                    }),
            )
            .into_any_element()
    }
}

pub(in crate::capture::desktop) fn sidebar_heading(label: &'static str) -> Div {
    div()
        .mb(px(8.0))
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(MUTED_COLOR))
        .child(label)
}

/// Splits a `sketch::timeline` line ("0:03 · Clicked…", "0:04 ✎ Circle…", "0:05 “said”") into
/// a time column and a body, flagging annotation marks.
pub(in crate::capture::desktop) fn split_timeline_line(line: &str) -> (&str, &str, bool) {
    let (time, rest) = line.split_once(' ').unwrap_or(("", line));
    if let Some(body) = rest.strip_prefix("✎ ") {
        (time, body, true)
    } else {
        (time, rest.strip_prefix("· ").unwrap_or(rest), false)
    }
}

pub(in crate::capture::desktop) fn timeline_row(index: usize, line: &str) -> gpui::Stateful<Div> {
    let (time, body, is_mark) = split_timeline_line(line);
    div()
        .id(("timeline-row", index))
        .px(px(8.0))
        .py(px(6.0))
        .rounded(px(6.0))
        .flex()
        .items_start()
        .gap(px(10.0))
        .text_size(px(12.5))
        .line_height(relative(1.4))
        .hover(|s| s.bg(rgb(SURFACE_BG)))
        .child(
            div()
                .w(px(32.0))
                .flex_none()
                .text_color(rgb(MUTED_COLOR))
                .child(time.to_string()),
        )
        .when(is_mark, |d| {
            d.child(div().flex_none().text_color(rgb(INK_RED)).child("✎"))
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(if is_mark {
                    rgb(TEXT_COLOR)
                } else {
                    rgb(TEXT_SOFT)
                })
                .child(body.to_string()),
        )
}

pub(in crate::capture::desktop) fn tab_chip(
    id: &'static str,
    label: &'static str,
    selected: bool,
) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .h(px(24.0))
        .px(px(10.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .when(selected, |d| {
            d.bg(rgb(SURFACE_SELECTED)).shadow(vec![BoxShadow {
                color: gpui::hsla(0.0, 0.0, 0.0, 0.25),
                offset: point(px(0.0), px(1.0)),
                blur_radius: px(2.0),
                spread_radius: px(0.0),
            }])
        })
        .text_color(if selected {
            rgb(TEXT_COLOR)
        } else {
            rgb(MUTED_COLOR)
        })
        .when(!selected, |d| {
            d.cursor_pointer().hover(|s| s.text_color(rgb(TEXT_SOFT)))
        })
        .child(label)
}

pub(in crate::capture::desktop) fn button_base(
    id: &'static str,
    primary: bool,
) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .map(platform::drag_region_control)
        .h(px(28.0))
        .px(px(12.0))
        .rounded(px(7.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .whitespace_nowrap()
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .border_1()
        .border_color(if primary {
            rgb(ACCENT_COLOR)
        } else {
            rgb(LINE_COLOR)
        })
        .bg(if primary {
            rgb(ACCENT_COLOR)
        } else {
            rgb(SURFACE_BG)
        })
        .text_color(rgb(TEXT_COLOR))
        .cursor_pointer()
        .hover(move |s| {
            if primary {
                s.bg(rgb(ACCENT_HOVER)).border_color(rgb(ACCENT_HOVER))
            } else {
                s.bg(rgb(SURFACE_HOVER))
            }
        })
}

pub(in crate::capture::desktop) fn action_button(
    id: &'static str,
    label: &str,
    primary: bool,
) -> gpui::Stateful<Div> {
    button_base(id, primary).child(SharedString::from(label.to_string()))
}

/// Button with a trailing muted hint, e.g. a keyboard shortcut.
pub(in crate::capture::desktop) fn hint_button(
    id: &'static str,
    label: &str,
    hint: Option<String>,
    primary: bool,
) -> gpui::Stateful<Div> {
    button_base(id, primary)
        .child(SharedString::from(label.to_string()))
        .when_some(hint, |d, hint| {
            d.child(
                div()
                    .text_color(if primary {
                        rgba(0xffffffb3)
                    } else {
                        rgb(MUTED_COLOR)
                    })
                    .child(hint),
            )
        })
}
