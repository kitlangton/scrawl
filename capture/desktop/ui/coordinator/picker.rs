use super::*;

pub struct TargetPickerWindow {
    pub(in crate::capture::desktop) coordinator: Entity<ApparentCoordinator>,
    pub(in crate::capture::desktop) focus: FocusHandle,
}

impl Render for TargetPickerWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let coord = self.coordinator.read(cx);
        let mode = match coord.presentation.island_phase {
            IslandPhase::Selecting { mode } => mode,
            _ => SelectionMode::Area,
        };
        let drag = coord.selection_drag;
        let hovered = coord.hovered_window.clone();
        let anim_rect = coord.animated_window_rect;
        let pointer = coord.selection_pointer;

        let c_down = self.coordinator.clone();
        let c_move = self.coordinator.clone();
        let c_up = self.coordinator.clone();
        let c_key = self.coordinator.clone();

        let win_bounds = window.bounds();
        // The picker covers one display; pointer, windows and areas are global points.
        let origin = [coord.display_rect[0], coord.display_rect[1]];
        // Global coordinates are points on macOS and pixels on Windows; this window lays out in points.
        let k = platform::points_per_pixel(window);
        let screen = [
            f32::from(win_bounds.size.width),
            f32::from(win_bounds.size.height),
        ];

        let iphone_rect = coord.iphone_preview.as_ref().and_then(PhoneSession::rect);
        let iphone_name = coord
            .iphone_preview
            .as_ref()
            .and_then(PhoneSession::device)
            .or_else(|| coord.iphone.clone())
            .map_or_else(|| "iPhone".to_string(), |d| d.name);

        // Selection rect in window-local coordinates, clipped to the screen, plus its badge.
        let highlight: Option<([f32; 4], String)> = match mode {
            SelectionMode::Area => drag.map(|(a, b)| {
                let [a, b] = [a, b].map(|p| clamp_point(p, coord.display_rect));
                let w = (a[0] - b[0]).abs().max(1.0);
                let h = (a[1] - b[1]).abs().max(1.0);
                (
                    [
                        (a[0].min(b[0]) - origin[0]) * k,
                        (a[1].min(b[1]) - origin[1]) * k,
                        w * k,
                        h * k,
                    ],
                    format!("{} × {}", w.round() as i32, h.round() as i32),
                )
            }),
            SelectionMode::Window => hovered.map(|t| {
                let title = match (&t.app, &t.window_title) {
                    (Some(app), Some(win)) if !win.is_empty() && win != app => {
                        format!("{app} — {win}")
                    }
                    (Some(app), _) => app.clone(),
                    _ => format!(
                        "{} × {}",
                        t.frame.width.round() as i32,
                        t.frame.height.round() as i32
                    ),
                };
                let [rx, ry, rw, rh] = anim_rect.unwrap_or([
                    t.frame.x as f32,
                    t.frame.y as f32,
                    t.frame.width as f32,
                    t.frame.height as f32,
                ]);
                (
                    [(rx - origin[0]) * k, (ry - origin[1]) * k, rw * k, rh * k],
                    title,
                )
            }),
            SelectionMode::Screen => Some((
                [0.0, 0.0, screen[0], screen[1]],
                "Full screen · Click to record".to_string(),
            )),
            SelectionMode::Iphone => iphone_rect.map(|[rx, ry, rw, rh]| {
                (
                    [(rx - origin[0]) * k, (ry - origin[1]) * k, rw * k, rh * k],
                    format!("{iphone_name} · Click to record"),
                )
            }),
        }
        .map(|(rect, label)| (clip_rect(rect, screen), label));

        let badge = highlight.as_ref().map(|(rect, label)| {
            let max_text = (screen[0] - 2.0 * BADGE_MARGIN - 2.0 * BADGE_PX - 2.0).min(480.0);
            let measured_label = if mode == SelectionMode::Area {
                clock_template(label)
            } else {
                label.clone()
            };
            let text_w =
                measure_text(window, &measured_label, BADGE_TEXT, FontWeight::MEDIUM).min(max_text);
            let size = [text_w + 2.0 * BADGE_PX + 2.0, BADGE_H];
            let anchor = match mode {
                SelectionMode::Area | SelectionMode::Iphone => BadgeAnchor::BelowTrailing,
                SelectionMode::Window | SelectionMode::Screen => BadgeAnchor::Center,
            };
            (
                place_badge(*rect, size, screen, anchor),
                text_w,
                label.clone(),
            )
        });

        // The 120Hz `LaserOverlay` draws the backdrop dim, selection cutout, window tint,
        // and inset border continuously across both `Selecting` and `Recording` so there is zero
        // window swap or border jump on recording start. `TargetPickerWindow` only provides the
        // hit-test surface, cursor style, camera cursor follower, and tabular dimension badge.
        let mut root = div()
            .id("apparent-target-picker")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .when(mode == SelectionMode::Area, |d| d.cursor_crosshair())
            .when(
                matches!(mode, SelectionMode::Iphone | SelectionMode::Screen),
                |d| d.cursor_pointer(),
            )
            .when(mode == SelectionMode::Window, |d| {
                d.cursor(gpui::CursorStyle::None)
            });

        if mode == SelectionMode::Window
            && let Some([px_screen, py_screen]) = pointer
        {
            let cx_local = (px_screen - origin[0]) * k;
            let cy_local = (py_screen - origin[1]) * k;
            root = root.child(camera_cursor_badge(cx_local, cy_local));
        }

        root.child(running_build_badge())
            .when_some(badge, |d, ([bx, by], text_w, label)| {
                d.child(
                    div()
                        .absolute()
                        .left(px(bx))
                        .top(px(by))
                        .h(px(BADGE_H))
                        .px(px(BADGE_PX))
                        .rounded(px(BADGE_H / 2.0))
                        .flex()
                        .items_center()
                        .bg(rgba(0x1c1c1ef2))
                        .border_1()
                        .border_color(rgba(0xffffff21))
                        .shadow(vec![BoxShadow {
                            color: gpui::hsla(0.0, 0.0, 0.0, 0.3),
                            offset: point(px(0.0), px(2.0)),
                            blur_radius: px(8.0),
                            spread_radius: px(0.0),
                        }])
                        .text_size(px(BADGE_TEXT))
                        .font(tabular_font(FontWeight::MEDIUM))
                        .text_color(rgb(TEXT_COLOR))
                        .child(div().w(px(text_w)).truncate().child(label)),
                )
            })
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                // Unhandled, gpui hands the key back to AppKit, which offers it again (as a key
                // equivalent, keyDown, or input method command) and this runs up to three times:
                // Space and the arrows then toggle or cycle back to where they started.
                if c_key.update(cx, |coord, cx| {
                    coord.handle_selection_key(&event.keystroke, event.is_held, cx)
                }) {
                    cx.stop_propagation();
                }
            })
            .on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, _window, cx| {
                    let [sx, sy] = current_mouse_screen()
                        .unwrap_or([f32::from(event.position.x), f32::from(event.position.y)]);
                    c_down.update(cx, |coord, cx| {
                        let cur_mode = match coord.presentation.island_phase {
                            IslandPhase::Selecting { mode } => mode,
                            _ => return,
                        };
                        match cur_mode {
                            SelectionMode::Area => {
                                coord.selection_drag = Some(([sx, sy], [sx, sy]));
                                coord.sync_picker_overlay();
                                cx.notify();
                            }
                            SelectionMode::Iphone => {
                                if let Some([rx, ry, rw, rh]) =
                                    coord.iphone_preview.as_ref().and_then(PhoneSession::rect)
                                    && sx >= rx
                                    && sx <= rx + rw
                                    && sy >= ry
                                    && sy <= ry + rh
                                {
                                    coord.start_recording_iphone(cx);
                                } else {
                                    coord.set_selection_mode(SelectionMode::Area, cx);
                                    coord.selection_drag = Some(([sx, sy], [sx, sy]));
                                    coord.sync_picker_overlay();
                                    cx.notify();
                                }
                            }
                            SelectionMode::Window => {
                                if coord.windows_loading {
                                    return;
                                }
                                if let Some(target) = selectable_window_at(
                                    &coord.selectable_windows,
                                    f64::from(sx),
                                    f64::from(sy),
                                )
                                .cloned()
                                {
                                    coord.start_recording_with_target(target, cx);
                                } else {
                                    coord.start_recording_full_screen(cx);
                                }
                            }
                            SelectionMode::Screen => coord.start_recording_full_screen(cx),
                        }
                    });
                },
            )
            .on_mouse_move(move |event: &MouseMoveEvent, _window, cx| {
                let [sx, sy] = current_mouse_screen()
                    .unwrap_or([f32::from(event.position.x), f32::from(event.position.y)]);
                c_move.update(cx, |coord, cx| {
                    coord.selection_pointer = Some([sx, sy]);
                    let cur_mode = match coord.presentation.island_phase {
                        IslandPhase::Selecting { mode } => mode,
                        _ => return,
                    };
                    match cur_mode {
                        SelectionMode::Area => {
                            if let Some((start, _)) = coord.selection_drag {
                                coord.selection_drag = Some((start, [sx, sy]));
                                coord.sync_picker_overlay();
                                cx.notify();
                            }
                        }
                        SelectionMode::Iphone => {
                            coord.sync_picker_overlay();
                        }
                        SelectionMode::Screen => {}
                        SelectionMode::Window => {
                            let found = coord
                                .selectable_windows
                                .iter()
                                .find(|w| {
                                    let f = w.frame;
                                    f64::from(sx) >= f.x
                                        && f64::from(sx) <= f.x + f.width
                                        && f64::from(sy) >= f.y
                                        && f64::from(sy) <= f.y + f.height
                                })
                                .cloned();
                            if coord.hovered_window.as_ref().and_then(|w| w.window_id)
                                != found.as_ref().and_then(|w| w.window_id)
                            {
                                coord.hovered_window = found;
                                coord.sync_picker_overlay();
                                cx.notify();
                            }
                        }
                    }
                });
            })
            .on_mouse_up(MouseButton::Left, {
                let c_up = c_up.clone();
                move |event: &MouseUpEvent, _window, cx| {
                    c_up.update(cx, |coord, cx| coord.picker_mouse_up(event, cx));
                }
            })
            // A drag released past the picker's display (on another one) ends there too, at the
            // display's edge: the window that saw the press gets the release, outside its bounds.
            .on_mouse_up_out(
                MouseButton::Left,
                move |event: &MouseUpEvent, _window, cx| {
                    c_up.update(cx, |coord, cx| coord.picker_mouse_up(event, cx));
                },
            )
    }
}

pub(in crate::capture::desktop) const BADGE_H: f32 = 22.0;
pub(in crate::capture::desktop) const BADGE_PX: f32 = 8.0;
pub(in crate::capture::desktop) const BADGE_TEXT: f32 = 12.0;
pub(in crate::capture::desktop) const BADGE_GAP: f32 = 8.0;
pub(in crate::capture::desktop) const BADGE_MARGIN: f32 = 8.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::capture::desktop) enum BadgeAnchor {
    /// Just outside the bottom-right corner (Area drags), flipping above or inside near edges.
    BelowTrailing,
    /// Centered over the target (Window hover).
    Center,
}

/// The smallest area the picker records, points a side (`screens::MIN_AREA` is the floor
/// after trimming).
pub(in crate::capture::desktop) const MIN_AREA_SIDE: f32 = 36.0;

pub(in crate::capture::desktop) fn clamp_point(
    [x, y]: [f32; 2],
    [dx, dy, dw, dh]: [f32; 4],
) -> [f32; 2] {
    [x.clamp(dx, dx + dw), y.clamp(dy, dy + dh)]
}

/// A dragged area in global points, kept on the picker's `display` (so the cutout, the size
/// badge and the recording agree), if it's big enough to record.
pub(in crate::capture::desktop) fn area_rect(
    a: [f32; 2],
    b: [f32; 2],
    display: [f32; 4],
) -> Option<[f64; 4]> {
    let [a, b] = [a, b].map(|p| clamp_point(p, display));
    let w = (a[0] - b[0]).abs();
    let h = (a[1] - b[1]).abs();
    (w >= MIN_AREA_SIDE && h >= MIN_AREA_SIDE)
        .then(|| [a[0].min(b[0]), a[1].min(b[1]), w, h].map(f64::from))
}

pub(in crate::capture::desktop) fn recording_pointer_destination(
    current: [f64; 4],
    pointer: [f64; 2],
    screens: &[crate::capture::engine::runtime::screens::Screen],
) -> Option<[f64; 4]> {
    if crate::capture::engine::runtime::screens::contains(current, pointer) {
        return None;
    }
    let frame = screens
        .get(crate::capture::engine::runtime::screens::index_at(
            screens, pointer,
        )?)?
        .frame;
    ((0..4).any(|i| (frame[i] - current[i]).abs() >= 1.0)).then_some(frame)
}

pub(in crate::capture::desktop) fn clip_rect(
    [x, y, w, h]: [f32; 4],
    [sw, sh]: [f32; 2],
) -> [f32; 4] {
    let x0 = x.clamp(0.0, sw);
    let y0 = y.clamp(0.0, sh);
    let x1 = (x + w).clamp(0.0, sw);
    let y1 = (y + h).clamp(0.0, sh);
    [x0, y0, x1 - x0, y1 - y0]
}

/// Positions a badge of `size` relative to `rect`, keeping it fully on screen.
pub(in crate::capture::desktop) fn place_badge(
    [x, y, w, h]: [f32; 4],
    [bw, bh]: [f32; 2],
    [sw, sh]: [f32; 2],
    anchor: BadgeAnchor,
) -> [f32; 2] {
    let (bx, by) = match anchor {
        BadgeAnchor::BelowTrailing => {
            let below = y + h + BADGE_GAP;
            let above = y - BADGE_GAP - bh;
            let by = if below + bh <= sh - BADGE_MARGIN {
                below
            } else if above >= BADGE_MARGIN {
                above
            } else {
                y + h - BADGE_GAP - bh
            };
            (x + w - bw, by)
        }
        BadgeAnchor::Center => (x + (w - bw) / 2.0, y + (h - bh) / 2.0),
    };
    let max_x = (sw - BADGE_MARGIN - bw).max(BADGE_MARGIN);
    let max_y = (sh - BADGE_MARGIN - bh).max(BADGE_MARGIN);
    [
        bx.clamp(BADGE_MARGIN, max_x).round(),
        by.clamp(BADGE_MARGIN, max_y).round(),
    ]
}
