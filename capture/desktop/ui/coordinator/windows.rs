use super::*;

#[cfg(any(test, target_os = "macos"))]
pub(in crate::capture::desktop) fn restore_picker_exclusion(
    ids: &mut Vec<u32>,
    picker: Option<u32>,
) {
    if let Some(id) = picker.filter(|id| *id > 0)
        && !ids.contains(&id)
    {
        ids.push(id);
    }
}

pub(in crate::capture::desktop) fn open_picker_windows(
    coord_entity: &Entity<ApparentCoordinator>,
    cx: &mut App,
) {
    close_picker_window(cx);
    let screen = coord_entity.read(cx).display_rect;
    let display = platform::display_at(screen, cx);
    // Windows wants the display's own origin, or GPUI puts the picker at its default bounds.
    let picker_bounds = Bounds {
        origin: platform::window_origin(display.as_deref()),
        size: platform::layout_size(screen, display.as_deref()),
    };
    let display_id = display.as_ref().map(|d| d.id());
    let lab = coord_entity.read(cx).lab;

    let picker_coord = coord_entity.clone();
    let _ = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(picker_bounds)),
            display_id,
            titlebar: None,
            focus: !lab,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        },
        move |window, cx| {
            // The display the picker was opened for, or has since followed the pointer to.
            let frame = picker_coord.read(cx).display_rect.map(f64::from);
            if let Some(num) = platform::configure_picker_window(window, lab, frame, cx) {
                picker_coord
                    .read(cx)
                    .telemetry
                    .excluded_window_ids
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(num);
            }
            let focus = cx.focus_handle();
            if !lab {
                window.focus(&focus);
            }
            cx.new(|cx| {
                cx.observe(&picker_coord, |_, _, cx| cx.notify()).detach();
                TargetPickerWindow {
                    coordinator: picker_coord,
                    focus,
                }
            })
        },
    );
    // A menu bar (accessory) app isn't active when its menu or capsule is used, and keys only
    // reach the picker while it is.
    if !lab {
        platform::take_keys(cx);
    }

    ensure_hud_pill_open(coord_entity, cx);
}

/// Puts Apparent's floating windows back in order once they've all opened: the pill above the
/// picker above the ink overlay (which places itself a turn later).
pub(in crate::capture::desktop) fn restack_soon(cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(60))
            .await;
        let _ = cx.update(|cx| {
            for handle in cx.windows() {
                if handle.downcast::<TargetPickerWindow>().is_some() {
                    let _ = handle.update(cx, |_, window, _| platform::raise_window(window));
                }
            }
            for handle in cx.windows() {
                if handle.downcast::<HudPillWindow>().is_some() {
                    let _ = handle.update(cx, |_, window, _| platform::raise_window(window));
                }
            }
        });
    })
    .detach();
}

pub(in crate::capture::desktop) fn ensure_hud_pill_open(
    coord_entity: &Entity<ApparentCoordinator>,
    cx: &mut App,
) {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        coord_entity.update(cx, |coord, _| coord.ensure_electron_pill());
        if coord_entity.read(cx).electron_pill_routed {
            return;
        }
        // Linux reserves presentation for the launching renderer. The tick
        // restores GPUI only on a real launch/readiness failure, avoiding a
        // native pill flashing before the healthy Electron surface arrives.
        #[cfg(target_os = "linux")]
        if coord_entity
            .read(cx)
            .electron_pill
            .as_ref()
            .is_some_and(|session| session.healthy())
        {
            return;
        }
    }
    if crate::shared::handoff::native_pill() {
        let _ = coord_entity.update(cx, |coord, _| {
            coord.ensure_native_pill();
            coord.sync_native_pill();
        });
        return;
    }
    restack_soon(cx);
    for handle in cx.windows() {
        if let Some(pill) = handle.downcast::<HudPillWindow>() {
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            let telemetry = Arc::clone(&coord_entity.read(cx).telemetry);
            let _ = pill.update(cx, |view, _, cx| {
                view.chrome.raise();
                // Linux's exclusion is candidate-only; Mac includes the pill in this experiment.
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                if let Some(num) = view.chrome.window_number() {
                    #[cfg(target_os = "macos")]
                    let exclude = !crate::shared::handoff::experiment("movableRecordingControls");
                    #[cfg(target_os = "linux")]
                    let exclude = true;
                    let mut ids = telemetry
                        .excluded_window_ids
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if exclude && !ids.contains(&num) {
                        ids.push(num);
                    }
                }
                cx.notify();
            });
            return;
        }
    }
    // The display in use: the picker's or recording's, else the pointer's.
    let coord = coord_entity.read(cx);
    let in_use = match (&coord.active_recording, coord.presentation.island_phase) {
        (Some(rec), _) if rec.target.mode != CaptureMode::Device => {
            let f = rec.target.frame;
            Some([f.x, f.y, f.width, f.height].map(|v| v as f32))
        }
        (_, IslandPhase::Selecting { .. }) => Some(coord.display_rect),
        _ => None,
    };
    let near = in_use.or_else(|| {
        crate::capture::engine::runtime::screens::pointer()
            .map(|[x, y]| [x as f32, y as f32, 1.0, 1.0])
    });
    let display = near.map_or_else(|| cx.primary_display(), |r| platform::display_at(r, cx));
    let display_bounds = display.as_ref().map_or_else(
        || Bounds {
            origin: Point::default(),
            size: size(px(1440.0), px(900.0)),
        },
        |d| d.bounds(),
    );
    let display_id = display.as_ref().map(|d| d.id());

    // Transparent Dynamic Island host positioned cleanly below the 37pt macOS menu bar
    let pill_size: Size<Pixels> = size(px(PILL_WINDOW_W), px(PILL_WINDOW_H));
    let pill_bounds = Bounds {
        origin: platform::hud_pill_origin(display_bounds),
        size: pill_size,
    };
    let pill_coord = coord_entity.clone();
    let _ = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(pill_bounds)),
            display_id,
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: true,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        },
        move |window, cx| {
            let chrome = PillChrome::open(window, cx);
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            if let Some(num) = chrome.window_number() {
                #[cfg(target_os = "macos")]
                let exclude = !crate::shared::handoff::experiment("movableRecordingControls");
                #[cfg(target_os = "linux")]
                let exclude = true;
                if exclude {
                    pill_coord
                        .read(cx)
                        .telemetry
                        .excluded_window_ids
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(num);
                }
            }
            let focus = cx.focus_handle();
            window.focus(&focus);
            cx.new(|cx| {
                cx.observe(&pill_coord, |_, _, cx| cx.notify()).detach();
                HudPillWindow {
                    coordinator: pill_coord,
                    focus,
                    chrome,
                    frames: 0,
                    #[cfg(target_os = "linux")]
                    equalizer: InputEqualizer::default(),
                }
            })
        },
    );
}

pub(in crate::capture::desktop) fn close_picker_window(cx: &mut App) {
    for handle in cx.windows() {
        if handle.downcast::<TargetPickerWindow>().is_some() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}

pub(in crate::capture::desktop) fn close_capture_windows(cx: &mut App) {
    for handle in cx.windows() {
        if handle.downcast::<HudPillWindow>().is_some()
            || handle.downcast::<TargetPickerWindow>().is_some()
        {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}

pub(in crate::capture::desktop) fn close_review_window(cx: &mut App) {
    for handle in cx.windows() {
        if handle.downcast::<BundleReviewWindow>().is_some() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}

pub(in crate::capture::desktop) fn open_review_capture(
    coord: &Entity<ApparentCoordinator>,
    path: &std::path::Path,
    cx: &mut App,
) {
    match crate::shared::handoff::load_bundle(path) {
        Ok(bundle) => {
            coord.update(cx, |coord, cx| {
                coord.mark_handled(path);
                coord.set_review_bundle(bundle, None);
                cx.notify();
            });
        }
        Err(error) => {
            coord.update(cx, |coord, cx| {
                coord.review_status = Some(format!("Could not open capture: {error}"));
                cx.notify();
            });
        }
    }
}

/// Opens the review window in front with the keys. A menu bar (accessory) app isn't active when
/// its menu is used, and the cooperative `NSApp.activate()` may leave the window behind the app
/// in front, so it activates ignoring other apps once the window exists.
pub(in crate::capture::desktop) fn open_review_window(
    coord_entity: &Entity<ApparentCoordinator>,
    cx: &mut App,
) {
    for handle in cx.windows() {
        if let Some(review) = handle.downcast::<BundleReviewWindow>() {
            platform::take_keys(cx);
            let _ = review.update(cx, |_, window, cx| {
                window.activate_window();
                cx.notify();
            });
            return;
        }
    }
    let bounds = Bounds::centered(None, size(px(1120.0), px(740.0)), cx);
    let coord_clone = coord_entity.clone();
    let _ = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                // The taskbar and Alt+Tab show it on Windows and Linux; macOS draws no title here.
                title: cfg!(not(target_os = "macos")).then(|| "Apparent".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(16.0), px(16.0))),
            }),
            app_id: Some("dev.kitlangton.apparent".into()),
            focus: true,
            show: true,
            kind: WindowKind::Normal,
            is_resizable: true,
            window_background: WindowBackgroundAppearance::Opaque,
            ..Default::default()
        },
        move |window, cx| {
            platform::decorate_app_window(window);
            let focus = cx.focus_handle();
            window.focus(&focus);
            cx.new(|cx: &mut Context<BundleReviewWindow>| {
                cx.observe(&coord_clone, |_, _, cx| cx.notify()).detach();
                cx.spawn(async move |view, cx| {
                    loop {
                        let history = cx
                            .background_executor()
                            .spawn(async {
                                crate::shared::handoff::capture_history(
                                    &crate::shared::handoff::default_captures_dir(),
                                )
                            })
                            .await;
                        if view
                            .update(cx, |review, cx| review.update_history(history, cx))
                            .is_err()
                        {
                            break;
                        }
                        cx.background_executor().timer(Duration::from_secs(2)).await;
                    }
                })
                .detach();
                BundleReviewWindow::new(coord_clone, focus)
            })
        },
    );
    platform::take_keys(cx);
}
