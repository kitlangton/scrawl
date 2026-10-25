//! `scrawl lab pill <scene>`: drives the capsule through exact states on a fixed timeline, with a
//! pretend recording (no capture, microphone, files, overlay, hotkeys or ingest server), so a
//! motion can be replayed, traced (`SCRAWL_MOTION_TRACE`), filmed and tuned (`scrawl tune`).

use std::time::Duration;

use gpui::{
    App, Application, Bounds, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
    div, point, prelude::*, px, rgb, size,
};

use super::{ScrawlCoordinator, SelectionMode, motion};
use crate::capture::SharedCaptureTelemetry;
use crate::model::{CaptureMode, TargetFrame, TargetInfo};

/// The backdrop: x offset from the display's centre line, top, width and height in points.
/// The capsule window is 520 × 84 at y = 34, under the menu bar.
pub const BACKDROP: [f32; 4] = [0.0, 38.0, 640.0, 76.0];

mod scene;

pub use scene::Scene;
pub(super) use scene::{PROCESSING_S, Step, TAIL_S, Timeline};

pub struct Options {
    pub scene: Scene,
    /// Times to play the scene; 0 repeats until killed (for tuning).
    pub repeat: u32,
    /// A plain backdrop behind the capsule, for clean recordings.
    pub backdrop: bool,
}

/// The display a pretend recording "records".
pub(super) fn target() -> TargetInfo {
    TargetInfo {
        mode: CaptureMode::Display,
        app: None,
        bundle_id: None,
        window_title: None,
        window_id: None,
        display_id: 0,
        frame: TargetFrame {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 900.0,
            scale: 2.0,
        },
    }
}

fn apply(step: Step, coord: &mut ScrawlCoordinator, cx: &mut gpui::Context<ScrawlCoordinator>) {
    motion::trace_event(format!("lab:{step:?}"));
    match step {
        Step::Tap => coord.record_screen_now(cx),
        Step::Chip(mode) => coord.switch_recording(mode, cx),
        Step::Open => coord.open_picker(Some(SelectionMode::Screen), cx),
        Step::Click(mode) => coord.click_selection_mode(mode, cx),
        Step::Cancel => coord.cancel_recording(cx),
        Step::Finish => coord.finish_recording(cx),
        Step::Dismiss => coord.dismiss_island(cx),
    }
}

pub fn run(options: Options) {
    let app = Application::new().with_assets(crate::icons::Assets);
    app.run(move |cx: &mut App| {
        motion::poll_tune();
        let (sender, _receiver) = flume::unbounded();
        let telemetry =
            SharedCaptureTelemetry::new(sender, crate::capture::ShortcutConfig::defaults());
        let coordinator = cx.new(|_| {
            let mut coord = ScrawlCoordinator::new(telemetry, None);
            coord.lab = true;
            coord
        });
        let display = cx.primary_display().map_or([1440.0, 900.0], |d| {
            let b = d.bounds();
            [f32::from(b.size.width), f32::from(b.size.height)]
        });
        motion::trace_open(&format!("pill.{}", options.scene.name()), display);
        if options.backdrop {
            open_backdrop(display, cx);
        }
        coordinator.update(cx, |coord, _| coord.ensure_native_pill());
        // For screen recordings (`perf/pill/record.sh`): the backdrop's rect in screen points.
        println!(
            "region {},{},{},{}",
            ((display[0] - BACKDROP[2]) * 0.5).round(),
            BACKDROP[1],
            BACKDROP[2],
            BACKDROP[3]
        );

        // The app's frame loop, minus everything that isn't the capsule, stepping the scene's
        // timeline on the animation clock (`--rune` steps the same `Timeline` the same way).
        let mut timeline = Timeline::new(options.scene, options.repeat);
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(8))
                    .await;
                let done = cx.update(|cx| {
                    motion::poll_tune();
                    let due = timeline.advance(motion::now());
                    coordinator.update(cx, |coord, cx| {
                        for step in due.steps {
                            apply(step, coord, cx);
                        }
                        coord.island_tick(cx);
                    });
                    due.done
                });
                match done {
                    Ok(false) => {}
                    Ok(true) => break,
                    Err(_) => return,
                }
            }
            let _ = cx.update(|cx| {
                let _ = coordinator.update(cx, |coord, _| {
                    coord.native_pill = None;
                });
                motion::trace_close();
                cx.quit();
                std::process::exit(0);
            });
        })
        .detach();
    });
}

struct Backdrop {
    theme: String,
}

impl Render for Backdrop {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        match self.theme.as_str() {
            "light" | "white" => div()
                .size_full()
                .bg(rgb(0xfafafc))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(6.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .w(px(88.0))
                                .h(px(12.0))
                                .rounded(px(4.0))
                                .bg(rgb(0x1e293b)),
                        )
                        .child(
                            div()
                                .w(px(64.0))
                                .h(px(12.0))
                                .rounded(px(4.0))
                                .bg(rgb(0x2563eb)),
                        )
                        .child(
                            div()
                                .w(px(108.0))
                                .h(px(12.0))
                                .rounded(px(4.0))
                                .bg(rgb(0x94a3b8)),
                        )
                        .child(
                            div()
                                .w(px(74.0))
                                .h(px(12.0))
                                .rounded(px(4.0))
                                .bg(rgb(0x10b981)),
                        )
                        .child(
                            div()
                                .w(px(84.0))
                                .h(px(12.0))
                                .rounded(px(4.0))
                                .bg(rgb(0xf43f5e)),
                        )
                        .child(
                            div()
                                .w(px(96.0))
                                .h(px(12.0))
                                .rounded(px(4.0))
                                .bg(rgb(0x475569)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .w(px(104.0))
                                .h(px(11.0))
                                .rounded(px(4.0))
                                .bg(rgb(0x64748b)),
                        )
                        .child(
                            div()
                                .w(px(78.0))
                                .h(px(11.0))
                                .rounded(px(4.0))
                                .bg(rgb(0xf59e0b)),
                        )
                        .child(
                            div()
                                .w(px(124.0))
                                .h(px(11.0))
                                .rounded(px(4.0))
                                .bg(rgb(0x0f172a)),
                        )
                        .child(
                            div()
                                .w(px(68.0))
                                .h(px(11.0))
                                .rounded(px(4.0))
                                .bg(rgb(0x8b5cf6)),
                        )
                        .child(
                            div()
                                .w(px(116.0))
                                .h(px(11.0))
                                .rounded(px(4.0))
                                .bg(rgb(0x3b82f6)),
                        ),
                ),
            "dark" | "busy-dark" => div()
                .size_full()
                .bg(rgb(0x0f1117))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(6.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .w(px(84.0))
                                .h(px(12.0))
                                .rounded(px(4.0))
                                .bg(rgb(0xf43f5e)),
                        )
                        .child(
                            div()
                                .w(px(104.0))
                                .h(px(12.0))
                                .rounded(px(4.0))
                                .bg(rgb(0x38bdf8)),
                        )
                        .child(
                            div()
                                .w(px(72.0))
                                .h(px(12.0))
                                .rounded(px(4.0))
                                .bg(rgb(0xfbbf24)),
                        )
                        .child(
                            div()
                                .w(px(112.0))
                                .h(px(12.0))
                                .rounded(px(4.0))
                                .bg(rgb(0xe2e8f0)),
                        )
                        .child(
                            div()
                                .w(px(82.0))
                                .h(px(12.0))
                                .rounded(px(4.0))
                                .bg(rgb(0xa855f7)),
                        )
                        .child(
                            div()
                                .w(px(76.0))
                                .h(px(12.0))
                                .rounded(px(4.0))
                                .bg(rgb(0x34d399)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .w(px(96.0))
                                .h(px(11.0))
                                .rounded(px(4.0))
                                .bg(rgb(0x34d399)),
                        )
                        .child(
                            div()
                                .w(px(84.0))
                                .h(px(11.0))
                                .rounded(px(4.0))
                                .bg(rgb(0xf97316)),
                        )
                        .child(
                            div()
                                .w(px(128.0))
                                .h(px(11.0))
                                .rounded(px(4.0))
                                .bg(rgb(0x60a5fa)),
                        )
                        .child(
                            div()
                                .w(px(72.0))
                                .h(px(11.0))
                                .rounded(px(4.0))
                                .bg(rgb(0xf472b6)),
                        )
                        .child(
                            div()
                                .w(px(108.0))
                                .h(px(11.0))
                                .rounded(px(4.0))
                                .bg(rgb(0xcbd5e1)),
                        ),
                ),
            _ => div().size_full().bg(rgb(0xd9d9de)),
        }
    }
}

/// A plain light panel behind the capsule's window, so recordings don't show whatever is on the
/// desktop. It takes no input.
fn open_backdrop(display: [f32; 2], cx: &mut App) {
    let theme = std::env::var("SCRAWL_LAB_BACKDROP")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let w = BACKDROP[2];
    let h = BACKDROP[3];
    let bounds = Bounds {
        origin: point(px((display[0] - w) * 0.5), px(BACKDROP[1])),
        size: size(px(w), px(h)),
    };
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Opaque,
            ..Default::default()
        },
        move |_, cx| cx.new(|_| Backdrop { theme }),
    );
    if opened.is_err() {
        return;
    }
    super::platform::configure_lab_backdrop(w, h);
}
