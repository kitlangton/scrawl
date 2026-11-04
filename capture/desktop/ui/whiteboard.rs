//! A separate native paper window. Recording keeps running while the browser remains clickable.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, Entity, FocusHandle, FontWeight, Image, ImageFormat, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels,
    Point, Render, Window, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
    canvas, div, img, point, prelude::*, px, rgb, size, svg,
};

use super::super::{ApparentCoordinator, platform};
use super::pill_theme::{CARBON, PAPER};

pub(in crate::capture::desktop) const PAPER_WIDTH: f32 = 1600.0;
pub(in crate::capture::desktop) const PAPER_HEIGHT: f32 = 1100.0;
const TOOLBAR_HEIGHT: f32 = 64.0;
const LILAC: u32 = 0xb7a5d8;

fn toolbar_button(id: &'static str, label: &'static str, close: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(36.0))
        .px(px(12.0))
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(rgb(if close { LILAC } else { 0x645a69 }))
        .when(close, |button| {
            button.bg(rgb(LILAC)).text_color(rgb(CARBON))
        })
        .rounded(px(2.0))
        .cursor_pointer()
        .hover(move |style| {
            style
                .bg(rgb(if close { 0xc4a9e6 } else { 0x302442 }))
                .border_color(rgb(LILAC))
        })
        .when_else(
            close,
            |button| {
                button.w(px(36.0)).px(px(0.0)).child(
                    svg()
                        .path(crate::capture::desktop::icons::X)
                        .size(px(16.0))
                        .text_color(rgb(CARBON)),
                )
            },
            |button| button.child(label),
        )
}

#[derive(Default, Clone)]
pub(in crate::capture::desktop) struct Board {
    pub strokes: Vec<Vec<[f32; 2]>>,
    pub active: Option<Vec<[f32; 2]>>,
}

impl Board {
    pub fn start(&mut self, point: [f32; 2]) {
        if on_paper(point) {
            self.active = Some(vec![point]);
        }
    }

    pub fn extend(&mut self, point: [f32; 2]) {
        if let Some(stroke) = self.active.as_mut() {
            stroke.push([
                point[0].clamp(0.0, PAPER_WIDTH),
                point[1].clamp(0.0, PAPER_HEIGHT),
            ]);
        }
    }

    pub fn finish(&mut self) {
        if let Some(stroke) = self.active.take() {
            self.strokes.push(stroke);
        }
    }

    pub fn undo(&mut self) {
        if self.active.take().is_none() {
            self.strokes.pop();
        }
    }

    pub fn clear(&mut self) {
        self.active = None;
        self.strokes.clear();
    }
}

pub(in crate::capture::desktop) fn reset_for_capture(board: &Rc<RefCell<Board>>) {
    board.borrow_mut().clear();
}

fn on_paper([x, y]: [f32; 2]) -> bool {
    (0.0..PAPER_WIDTH).contains(&x) && (0.0..PAPER_HEIGHT).contains(&y)
}

/// The canvas starts immediately beneath the toolbar. GPUI pointer positions are window-local;
/// paper coordinates do not depend on the current window width or height.
fn paper_point(position: Point<Pixels>) -> [f32; 2] {
    [
        f32::from(position.x),
        f32::from(position.y) - TOOLBAR_HEIGHT,
    ]
}

pub(in crate::capture::desktop) struct WhiteboardWindow {
    board: Rc<RefCell<Board>>,
    coordinator: Entity<ApparentCoordinator>,
    focus: FocusHandle,
    mark: Arc<Image>,
}

impl WhiteboardWindow {
    fn snapshot_soon(&self, cx: &mut Context<Self>) {
        let coordinator = self.coordinator.clone();
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            let _ = coordinator.update(cx, |coord, _| coord.request_immediate_frame());
        })
        .detach();
    }
}

fn paint_stroke(window: &mut Window, points: &[[f32; 2]], origin: Point<Pixels>) {
    if points.is_empty() {
        return;
    }
    let mut path = PathBuilder::stroke(px(3.0));
    let at = |[x, y]: [f32; 2]| point(origin.x + px(x), origin.y + px(y));
    path.move_to(at(points[0]));
    if points.len() == 1 {
        path.line_to(at([points[0][0] + 0.01, points[0][1]]));
    } else {
        for &p in &points[1..] {
            path.line_to(at(p));
        }
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, gpui::hsla(0.0, 0.0, 0.10, 1.0));
    }
}

impl Render for WhiteboardWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let board = self.board.borrow().clone();
        div()
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(PAPER))
            .font_family("Host Grotesk")
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                if key.eq_ignore_ascii_case("escape") {
                    window.remove_window();
                    cx.stop_propagation();
                    return;
                }
                let undo = key.eq_ignore_ascii_case("z") && event.keystroke.modifiers.platform;
                if undo {
                    view.board.borrow_mut().undo();
                    view.snapshot_soon(cx);
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .h(px(TOOLBAR_HEIGHT))
                    .flex_none()
                    .px(px(16.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .bg(rgb(CARBON))
                    .border_b_1()
                    .border_color(rgb(LILAC))
                    .text_size(px(14.0))
                    .text_color(rgb(PAPER))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(img(self.mark.clone()).size(px(28.0)).flex_none())
                            .child(
                                div()
                                    .text_size(px(16.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Whiteboard"),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .font_family("monospace")
                                    .text_color(rgb(PAPER))
                                    .child(crate::capture::desktop::running_build_label()),
                            ),
                    )
                    .child(
                        toolbar_button("whiteboard-undo", "Undo", false).on_click(cx.listener(
                            |view, _, _, cx| {
                                view.board.borrow_mut().undo();
                                view.snapshot_soon(cx);
                                cx.notify();
                            },
                        )),
                    )
                    .child(toolbar_button("whiteboard-clear", "Clear", false).on_click(
                        cx.listener(|view, _, _, cx| {
                            view.board.borrow_mut().clear();
                            view.snapshot_soon(cx);
                            cx.notify();
                        }),
                    ))
                    .child(
                        toolbar_button("whiteboard-close", "Close", true)
                            .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                    ),
            )
            .child(
                div()
                    .id("whiteboard-paper")
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .cursor_crosshair()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, event: &MouseDownEvent, _, cx| {
                            view.board.borrow_mut().start(paper_point(event.position));
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _, cx| {
                        if view.board.borrow().active.is_some() {
                            view.board.borrow_mut().extend(paper_point(event.position));
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|view, _: &MouseUpEvent, _, cx| {
                            view.board.borrow_mut().finish();
                            view.snapshot_soon(cx);
                            cx.notify();
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|view, _: &MouseUpEvent, _, cx| {
                            view.board.borrow_mut().finish();
                            view.snapshot_soon(cx);
                            cx.notify();
                        }),
                    )
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
                                for stroke in &board.strokes {
                                    paint_stroke(window, stroke, bounds.origin);
                                }
                                if let Some(active) = &board.active {
                                    paint_stroke(window, active, bounds.origin);
                                }
                            },
                        )
                        .size_full(),
                    ),
            )
    }
}

pub(in crate::capture::desktop) fn open(coord: &Entity<ApparentCoordinator>, cx: &mut App) {
    if !coord.read(cx).is_recording() {
        return;
    }
    for handle in cx.windows() {
        if let Some(board) = handle.downcast::<WhiteboardWindow>() {
            let _ = board.update(cx, |_, window, _| window.activate_window());
            return;
        }
    }
    let shared = coord.read(cx).whiteboard.clone();
    let coordinator = coord.clone();
    let bounds = Bounds::centered(None, size(px(720.0), px(540.0)), cx);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Apparent Whiteboard".into()),
                ..Default::default()
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
            cx.new(|_| WhiteboardWindow {
                board: shared,
                coordinator,
                focus,
                mark: Arc::new(Image::from_bytes(
                    ImageFormat::Png,
                    include_bytes!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/shared/assets/brand/apparent-a.png"
                    ))
                    .to_vec(),
                )),
            })
        },
    );
    if opened.is_ok() {
        platform::float_whiteboard(cx);
        platform::take_keys(cx);
    }
}

/// Snapshot the paper at Stop before the UI is dismissed; the bundle worker uses this copy.
pub(in crate::capture::desktop) fn finish_snapshot(
    board: &Rc<RefCell<Board>>,
    close: impl FnOnce(),
) -> Board {
    let snapshot = board.borrow().clone();
    close();
    snapshot
}

pub(in crate::capture::desktop) fn close(cx: &mut App) {
    for handle in cx.windows() {
        if handle.downcast::<WhiteboardWindow>().is_some() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}

/// A paper export is optional. Never turn a finished screen recording into a failed capture.
pub(in crate::capture::desktop) fn finish_handoff(
    dir: &std::path::Path,
    bundle: crate::shared::model::ApparentBundle,
    summary: String,
    board: &Board,
) -> (crate::shared::model::ApparentBundle, String) {
    finish_handoff_with(dir, bundle, summary, board, |_| Ok(()))
}

pub(in crate::capture::desktop) fn finish_handoff_with(
    dir: &std::path::Path,
    mut bundle: crate::shared::model::ApparentBundle,
    summary: String,
    board: &Board,
    before_publish: impl FnMut(usize) -> anyhow::Result<()>,
) -> (crate::shared::model::ApparentBundle, String) {
    if let Err(error) = append_to_bundle_with(dir, &mut bundle, board, before_publish) {
        crate::capture::engine::runtime::apparent_log(format!(
            "Whiteboard handoff skipped: {error:#}"
        ));
        return (bundle, summary);
    }
    let summary = crate::capture::engine::sketch::summary_markdown(&bundle, dir);
    (bundle, summary)
}

/// Keep the whole finished sketch legible even if the window was closed or resized at stop.
fn append_to_bundle_with(
    dir: &std::path::Path,
    bundle: &mut crate::shared::model::ApparentBundle,
    board: &Board,
    before_publish: impl FnMut(usize) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    use crate::shared::model::{Keyframe, KeyframeCrop, KeyframeReason};
    use std::io::BufReader;

    if board.strokes.is_empty() {
        return Ok(());
    }
    let root = crate::capture::engine::bundle::bundle_processing::publication::Directory::open(
        dir, false,
    )?;
    let read = |path: &str| -> anyhow::Result<crate::shared::model::RgbaFrame> {
        crate::capture::engine::render::decode_png(BufReader::new(root.file(path)?))
    };
    let mut frames: Vec<_> = bundle
        .keyframes
        .iter()
        .map(|key| Ok((key.clone(), read(&key.file)?)))
        .collect::<anyhow::Result<_>>()?;
    let index = frames.len() + 1;
    let count = board.strokes.len();
    let event = format!(
        "whiteboard drawing ({count} stroke{}; final state)",
        if count == 1 { "" } else { "s" }
    );
    let key = Keyframe {
        index,
        t: bundle.duration_ms,
        reason: KeyframeReason::Whiteboard,
        file: format!("frames/frame-{index:02}.png"),
        event,
        said: None,
        mark_id: None,
        cursor: None,
    };
    let paper = render_paper(&board.strokes, bundle.duration_ms);
    // Contact sheets are rendered before the small number chip is stamped on the frame.
    // Existing PNGs already have their chips; reusing them does not hide any sketch content.
    frames.push((key.clone(), paper.clone()));
    let sheet = crate::capture::engine::render::render_contact_sheet_for_duration(
        bundle.duration_ms,
        &frames,
    );
    let mut stamped = vec![(key.clone(), paper)];
    crate::capture::engine::render::stamp_keyframe_badges(&mut stamped);
    let png = crate::capture::engine::render::encode_png(&stamped[0].1)?;
    let full_sheet = if let Some(crop) = &bundle.autocrop {
        let mut full: Vec<_> = bundle
            .keyframes
            .iter()
            .map(|key| {
                let name = std::path::Path::new(&key.file).file_name().unwrap();
                let path = format!("{}/{}", crop.full_frames, name.to_string_lossy());
                Ok((key.clone(), read(&path)?))
            })
            .collect::<anyhow::Result<_>>()?;
        full.push((key.clone(), stamped[0].1.clone()));
        Some(
            crate::capture::engine::render::render_contact_sheet_for_duration(
                bundle.duration_ms,
                &full,
            ),
        )
    } else {
        None
    };

    let mut next = bundle.clone();
    let mut writes = vec![
        (key.file.clone(), png.clone()),
        (
            bundle.contact_sheet.clone(),
            crate::capture::engine::render::encode_png(&sheet)?,
        ),
    ];
    if let (Some(crop), Some(full)) = (&mut next.autocrop, full_sheet) {
        let name = std::path::Path::new(&key.file).file_name().unwrap();
        writes.push((
            format!("{}/{}", crop.full_frames, name.to_string_lossy()),
            png,
        ));
        writes.push((
            crop.full_contact_sheet.clone(),
            crate::capture::engine::render::encode_png(&full)?,
        ));
        crop.crops.push(KeyframeCrop {
            index,
            x: 0.0,
            y: 0.0,
            width: bundle.target.frame.width,
            height: bundle.target.frame.height,
            kind: crate::capture::engine::autocrop::autocrop::CropKind::Full,
            reason: "whiteboard paper (not a screen crop)".into(),
            apps: Vec::new(),
        });
        crop.changes.push(Vec::new());
    }
    next.keyframes.push(key);
    writes.push(("apparent.json".into(), serde_json::to_vec_pretty(&next)?));
    writes.push((
        "summary.md".into(),
        crate::capture::engine::sketch::summary_markdown(&next, dir).into_bytes(),
    ));
    publish_artifacts(dir, &writes, before_publish)?;
    *bundle = next;
    Ok(())
}

/// Stage replacements and independent backups before replacing anything. Rollback
/// renames allocate no payload bytes; a staging failure leaves originals intact.
fn publish_artifacts(
    dir: &std::path::Path,
    writes: &[(String, Vec<u8>)],
    mut before_publish: impl FnMut(usize) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    use anyhow::Context;

    let root = crate::capture::engine::bundle::bundle_processing::publication::Directory::open(
        dir, false,
    )?;
    let stage_name = format!(".whiteboard-{}", crate::shared::ids::make_bundle_id());
    let stage = root.child(&stage_name, true)?;
    let mut rollback_failed = false;
    let mut originals = vec![false; writes.len()];
    let result = (|| -> anyhow::Result<()> {
        for (index, (path, bytes)) in writes.iter().enumerate() {
            stage.write(&format!("new-{index}"), bytes)?;
            match root.file(path) {
                Ok(_) => {
                    root.copy_to(path, &stage, &format!("old-{index}"))
                        .with_context(|| format!("backing up {path}"))?;
                    originals[index] = true;
                }
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        let mut published = 0;
        for (index, (path, _)) in writes.iter().enumerate() {
            if let Err(error) = before_publish(index)
                .and_then(|()| stage.rename_to(&format!("new-{index}"), &root, path))
            {
                for (prior, (path, _)) in writes[..published].iter().enumerate().rev() {
                    let backup = format!("old-{prior}");
                    let restored = if originals[prior] {
                        stage.rename_to(&backup, &root, path)
                    } else {
                        root.remove_path(path)
                    };
                    if let Err(restore_error) = restored {
                        rollback_failed = true;
                        crate::capture::engine::runtime::apparent_log(format!(
                            "Whiteboard rollback failed for {path}: {restore_error}"
                        ));
                    }
                }
                return Err(error).context("publishing whiteboard handoff");
            }
            published += 1;
        }
        Ok(())
    })();
    if rollback_failed {
        crate::capture::engine::runtime::apparent_log(format!(
            "Whiteboard backups left at {} for recovery",
            dir.join(&stage_name).display()
        ));
    } else {
        before_publish(writes.len()).context("Whiteboard stage cleanup failed")?;
        for leaf in stage.names()? {
            stage
                .remove(&leaf)
                .context("Whiteboard stage cleanup failed")?;
        }
        root.remove_directory(&stage_name)
            .context("Whiteboard stage cleanup failed")?;
    }
    result
}

fn render_paper(strokes: &[Vec<[f32; 2]>], t_ms: u64) -> crate::shared::model::RgbaFrame {
    use crate::shared::model::RgbaFrame;

    let (mut x0, mut y0, mut x1, mut y1) = (PAPER_WIDTH, PAPER_HEIGHT, 0.0f32, 0.0f32);
    for point in strokes.iter().flatten() {
        x0 = x0.min(point[0]);
        y0 = y0.min(point[1]);
        x1 = x1.max(point[0]);
        y1 = y1.max(point[1]);
    }
    let width = (x1 - x0 + 100.0).max(400.0).min(PAPER_WIDTH);
    let height = (y1 - y0 + 100.0).max(300.0).min(PAPER_HEIGHT);
    let left = ((x0 + x1 - width) / 2.0).clamp(0.0, PAPER_WIDTH - width);
    let top = ((y0 + y1 - height) / 2.0).clamp(0.0, PAPER_HEIGHT - height);
    let (w, h) = (width.ceil() as u32, height.ceil() as u32);
    let mut image = RgbaFrame::new(t_ms, w, h, vec![255; w as usize * h as usize * 4]);
    let dot = |image: &mut RgbaFrame, x: f32, y: f32| {
        let (x, y) = (x.round() as i32, y.round() as i32);
        for dy in -2..=2 {
            for dx in -2..=2 {
                if dx * dx + dy * dy > 5 {
                    continue;
                }
                let (px, py) = (x + dx, y + dy);
                if px >= 0 && py >= 0 && px < image.width as i32 && py < image.height as i32 {
                    let at = (py as usize * image.width as usize + px as usize) * 4;
                    image.rgba[at..at + 3].fill(26);
                }
            }
        }
    };
    for stroke in strokes {
        for point in stroke {
            dot(&mut image, point[0] - left, point[1] - top);
        }
        for pair in stroke.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let steps = ((b[0] - a[0]).hypot(b[1] - a[1]) * 2.0).ceil() as usize;
            for i in 0..=steps.max(1) {
                let f = i as f32 / steps.max(1) as f32;
                dot(
                    &mut image,
                    a[0] + (b[0] - a[0]) * f - left,
                    a[1] + (b[1] - a[1]) * f - top,
                );
            }
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branded_buttons_construct_with_one_hover_style_each() {
        for (id, label, close) in [
            ("whiteboard-undo", "Undo", false),
            ("whiteboard-clear", "Clear", false),
            ("whiteboard-close", "Close", true),
        ] {
            let _ = toolbar_button(id, label, close);
        }
    }

    #[test]
    fn branded_toolbar_keeps_pointer_coordinates_on_the_paper() {
        let mut board = Board::default();
        board.start(paper_point(point(px(20.0), px(TOOLBAR_HEIGHT - 1.0))));
        assert!(
            board.active.is_none(),
            "toolbar clicks must never become ink"
        );
        board.start(paper_point(point(px(20.0), px(TOOLBAR_HEIGHT))));
        board.extend(paper_point(point(px(80.0), px(TOOLBAR_HEIGHT + 60.0))));
        board.finish();
        assert_eq!(board.strokes, vec![vec![[20.0, 0.0], [80.0, 60.0]]]);
    }

    #[test]
    fn stopping_closes_the_board_after_preserving_its_last_drawing() {
        let board = Rc::new(RefCell::new(Board::default()));
        board.borrow_mut().start([12.0, 18.0]);
        board.borrow_mut().extend([32.0, 48.0]);
        board.borrow_mut().finish();
        let closed = std::cell::Cell::new(false);
        let snapshot = finish_snapshot(&board, || closed.set(true));
        assert!(closed.get(), "Stop must dismiss the visible whiteboard");
        assert_eq!(snapshot.strokes, board.borrow().strokes);
        board.borrow_mut().clear();
        assert_eq!(
            snapshot.strokes.len(),
            1,
            "export must survive the next session"
        );
    }

    #[test]
    fn the_next_capture_starts_with_empty_paper() {
        let board = Rc::new(RefCell::new(Board::default()));
        board.borrow_mut().start([12.0, 18.0]);
        board.borrow_mut().finish();
        let first = finish_snapshot(&board, || {});
        reset_for_capture(&board);
        let next = finish_snapshot(&board, || {});
        assert_eq!(first.strokes.len(), 1);
        assert!(
            next.strokes.is_empty(),
            "the previous recording must not leak into the next"
        );
    }

    use crate::shared::model::KeyframeReason;

    #[test]
    fn strokes_survive_viewport_changes_and_undo_clear_are_explicit() {
        let mut board = Board::default();
        board.start([20.0, 40.0]);
        board.extend([40.0, 60.0]);
        board.finish();
        board.start([100.0, 120.0]);
        board.finish();
        assert_eq!(
            board.strokes,
            vec![vec![[20.0, 40.0], [40.0, 60.0]], vec![[100.0, 120.0]]]
        );
        board.undo();
        assert_eq!(board.strokes.len(), 1);
        board.clear();
        assert!(board.strokes.is_empty());
        assert_eq!((PAPER_WIDTH, PAPER_HEIGHT), (1600.0, 1100.0));
    }

    #[test]
    fn finished_whiteboard_is_a_legible_handoff_frame_and_text() {
        let root = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
        let (dir, mut bundle, _) =
            crate::capture::desktop::demo::generate_demo_bundle(Some(root.join("demo.apparent")))
                .unwrap();
        let sheet_before = std::fs::read(dir.join("contact-sheet.png")).unwrap();
        let full_dir = dir.join("frames-full");
        std::fs::create_dir(&full_dir).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&full_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        for frame in &bundle.keyframes {
            std::fs::copy(
                &dir.join(&frame.file),
                full_dir.join(std::path::Path::new(&frame.file).file_name().unwrap()),
            )
            .unwrap();
        }
        std::fs::copy(
            dir.join("contact-sheet.png"),
            dir.join("contact-sheet-full.png"),
        )
        .unwrap();
        bundle.autocrop = Some(crate::shared::model::AutocropInfo {
            label: "the activity".into(),
            full_frames: "frames-full".into(),
            full_contact_sheet: "contact-sheet-full.png".into(),
            crops: bundle
                .keyframes
                .iter()
                .map(|frame| crate::shared::model::KeyframeCrop {
                    index: frame.index,
                    x: 0.0,
                    y: 0.0,
                    width: bundle.target.frame.width,
                    height: bundle.target.frame.height,
                    kind: crate::capture::engine::autocrop::autocrop::CropKind::Full,
                    reason: "test".into(),
                    apps: vec![],
                })
                .collect(),
            changes: vec![vec![]; bundle.keyframes.len()],
        });
        let mut board = Board::default();
        board.start([40.0, 40.0]);
        board.extend([420.0, 310.0]);
        board.finish();
        append_to_bundle_with(&dir, &mut bundle, &board, |_| Ok(())).unwrap();
        let frame = bundle.keyframes.last().unwrap();
        assert_eq!(frame.reason, KeyframeReason::Whiteboard);
        assert!(
            crate::capture::engine::sketch::summary_markdown_with(&bundle, &dir, false)
                .contains("whiteboard drawing (1 stroke; final state)")
        );
        assert!(
            crate::capture::engine::sketch::summary_markdown_with(&bundle, &dir, true)
                .contains("whiteboard drawing (1 stroke; final state)")
        );
        assert!(dir.join(&frame.file).is_file());
        let image = crate::capture::engine::render::decode_png(std::io::BufReader::new(
            std::fs::File::open(dir.join(&frame.file)).unwrap(),
        ))
        .unwrap();
        assert!(image.rgba.chunks_exact(4).any(|p| p[0] < 100 && p[1] < 100));
        assert_ne!(
            std::fs::read(dir.join("contact-sheet.png")).unwrap(),
            sheet_before
        );
        assert_eq!(
            bundle.autocrop.as_ref().unwrap().crops.len(),
            bundle.keyframes.len()
        );
        assert!(
            full_dir
                .join(std::path::Path::new(&frame.file).file_name().unwrap())
                .exists()
        );
        assert_ne!(
            std::fs::read(dir.join("contact-sheet-full.png")).unwrap(),
            sheet_before
        );
        let saved: crate::shared::model::ApparentBundle =
            serde_json::from_slice(&std::fs::read(dir.join("apparent.json")).unwrap()).unwrap();
        assert_eq!(saved.keyframes, bundle.keyframes);
        assert_eq!(
            std::fs::read_to_string(dir.join("summary.md")).unwrap(),
            crate::capture::engine::sketch::summary_markdown(&bundle, &dir)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_or_cleared_paper_does_not_add_a_handoff_tile() {
        let root = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
        let (dir, mut bundle, _) =
            crate::capture::desktop::demo::generate_demo_bundle(Some(root.join("demo.apparent")))
                .unwrap();
        let before = std::fs::read(dir.join("contact-sheet.png")).unwrap();
        let keys = bundle.keyframes.len();
        let mut board = Board::default();
        board.start([30.0, 40.0]);
        board.finish();
        board.clear();
        append_to_bundle_with(&dir, &mut bundle, &board, |_| Ok(())).unwrap();
        assert_eq!(bundle.keyframes.len(), keys);
        assert_eq!(
            std::fs::read(dir.join("contact-sheet.png")).unwrap(),
            before
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_paper_publish_keeps_the_original_capture_and_clipboard_text() {
        let root = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
        let (dir, bundle, summary) =
            crate::capture::desktop::demo::generate_demo_bundle(Some(root.join("demo.apparent")))
                .unwrap();
        let manifest = std::fs::read(dir.join("apparent.json")).unwrap();
        let sheet = std::fs::read(dir.join("contact-sheet.png")).unwrap();
        let text = std::fs::read(dir.join("summary.md")).unwrap();
        let next_frame = dir.join(format!(
            "frames/frame-{:02}.png",
            bundle.keyframes.len() + 1
        ));
        let mut board = Board::default();
        board.start([40.0, 40.0]);
        board.extend([420.0, 310.0]);
        board.finish();

        // Fail after the sheet, then again after the manifest, and restore all originals.
        for fail_at in [2, 3] {
            let (returned, returned_summary) =
                finish_handoff_with(&dir, bundle.clone(), summary.clone(), &board, |index| {
                    if index == fail_at {
                        anyhow::bail!("injected paper publish failure")
                    }
                    Ok(())
                });
            assert_eq!(returned.keyframes, bundle.keyframes);
            assert_eq!(returned_summary, summary);
            assert_eq!(std::fs::read(dir.join("apparent.json")).unwrap(), manifest);
            assert_eq!(std::fs::read(dir.join("contact-sheet.png")).unwrap(), sheet);
            assert_eq!(std::fs::read(dir.join("summary.md")).unwrap(), text);
            assert!(!next_frame.exists());
            assert!(!std::fs::read_dir(&dir).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".whiteboard-")
            }));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cleanup_failure_retains_stage_and_blocks_fallback_publication() {
        use crate::capture::engine::bundle::bundle_processing::publication;
        let root = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
        let (dir, bundle, summary) =
            crate::capture::desktop::demo::generate_demo_bundle(Some(root.join("demo.apparent")))
                .unwrap();
        let mut board = Board::default();
        board.start([40.0, 40.0]);
        board.extend([420.0, 310.0]);
        board.finish();
        let result = publication::publish_after(&dir, || {
            let (fallback, text) =
                finish_handoff_with(&dir, bundle.clone(), summary.clone(), &board, |index| {
                    if index == 4 {
                        anyhow::bail!("injected stage cleanup failure")
                    }
                    Ok(())
                });
            assert_eq!(fallback.keyframes, bundle.keyframes);
            assert_eq!(text, summary);
            Ok((dir.clone(), fallback, text))
        });
        assert!(result.is_err());
        assert!(dir.join(publication::IN_PROGRESS).exists());
        assert!(std::fs::read_dir(&dir).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".whiteboard-")
        }));
        assert!(publication::checked(&dir, &bundle, &summary).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
