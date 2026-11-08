//! `apparent lab pill <scene>`: drives the capsule through exact states on a fixed timeline, with a
//! pretend recording (no capture, microphone, files, overlay, hotkeys or ingest server), so a
//! motion can be replayed, traced (`APPARENT_MOTION_TRACE`), filmed and tuned (`apparent tune`).

use std::time::Duration;

use gpui::{
    App, Application, Bounds, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
    div, point, prelude::*, px, rgb, size,
};

use super::super::{ApparentCoordinator, SelectionMode};
use super::motion;
use crate::capture::engine::runtime::SharedCaptureTelemetry;
use crate::shared::model::{CaptureMode, TargetFrame, TargetInfo};

/// How long a pretend recording "processes" before it's done.
pub(in crate::capture::desktop) const PROCESSING_S: f32 = 0.8;
/// Quiet time before a scene's first step, so motion-lint's launch settle passes and a screen
/// recording starts on a still frame.
const LEAD_IN_S: f32 = 0.85;
/// Quiet time after the last step (and between repeats).
const TAIL_S: f32 = 0.75;
/// The backdrop: x offset from the display's centre line, top, width and height in points.
/// The capsule window is 520 × 84 at y = 34, under the menu bar.
pub const BACKDROP: [f32; 4] = [0.0, 38.0, 640.0, 76.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Scene {
    /// Tap: recording + chips, then the Area chip turns it into the picker.
    TapThenArea,
    /// Tap: recording + chips, then the Window chip turns it into the picker.
    TapThenWindow,
    /// Tap: recording + chips, then the Screen chip folds them back.
    TapThenScreen,
    /// The menu's picker opens on Screen, a click on Area, then Screen, then Screen again records.
    Picker,
    /// Tap, chips fold, finish: processing, then done, then dismissed.
    Finish,
    /// Tap: recording + chips, then cancel straight from the recording.
    Cancel,
    /// Open the picker, then dismiss it.
    Dismiss,
}

#[derive(Clone, Copy, Debug)]
enum Step {
    Tap,
    Chip(SelectionMode),
    Open,
    Click(SelectionMode),
    Cancel,
    Finish,
    Dismiss,
}

impl Scene {
    pub fn name(self) -> &'static str {
        match self {
            Self::TapThenArea => "tap-then-area",
            Self::TapThenWindow => "tap-then-window",
            Self::TapThenScreen => "tap-then-screen",
            Self::Picker => "picker",
            Self::Finish => "finish",
            Self::Cancel => "cancel",
            Self::Dismiss => "dismiss",
        }
    }

    /// Steps at animation seconds from the scene's start.
    fn steps(self) -> Vec<(f32, Step)> {
        use SelectionMode::{Area, Screen, Window};
        match self {
            Self::TapThenArea => vec![
                (0.0, Step::Tap),
                (1.2, Step::Chip(Area)),
                (2.4, Step::Cancel),
            ],
            Self::TapThenWindow => {
                vec![
                    (0.0, Step::Tap),
                    (1.2, Step::Chip(Window)),
                    (2.4, Step::Cancel),
                ]
            }
            Self::TapThenScreen => {
                vec![
                    (0.0, Step::Tap),
                    (1.2, Step::Chip(Screen)),
                    (2.4, Step::Cancel),
                ]
            }
            Self::Picker => vec![
                (0.0, Step::Open),
                (0.9, Step::Click(Area)),
                (1.8, Step::Click(Screen)),
                (2.7, Step::Click(Screen)),
                (3.9, Step::Cancel),
            ],
            Self::Finish => vec![
                (0.0, Step::Tap),
                (0.9, Step::Chip(Screen)),
                (2.0, Step::Finish),
                (2.0 + PROCESSING_S + 1.4, Step::Dismiss),
            ],
            Self::Cancel => vec![(0.0, Step::Tap), (1.2, Step::Cancel)],
            Self::Dismiss => vec![(0.0, Step::Open), (1.2, Step::Dismiss)],
        }
    }
}

pub struct Options {
    pub scene: Scene,
    /// Times to play the scene; 0 repeats until killed (for tuning).
    pub repeat: u32,
    /// A plain backdrop behind the capsule, for clean recordings.
    pub backdrop: bool,
}

/// The display a pretend recording "records".
pub(in crate::capture::desktop) fn target() -> TargetInfo {
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

fn apply(step: Step, coord: &mut ApparentCoordinator, cx: &mut gpui::Context<ApparentCoordinator>) {
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

/// Real seconds for `s` animation seconds at the current time scale.
fn real(s: f32) -> Duration {
    Duration::from_secs_f32((s / motion::time_scale()).max(0.0))
}

pub fn run(options: Options) {
    let app = Application::new().with_assets(crate::capture::desktop::icons::Assets);
    app.run(move |cx: &mut App| {
        motion::poll_tune();
        let (sender, _receiver) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(sender);
        let coordinator = cx.new(|_| {
            let mut coord = ApparentCoordinator::new(telemetry, None);
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
        // For screen recordings (`capture/desktop/scripts/perf/pill/record.sh`): the backdrop's rect in screen points.
        println!(
            "region {},{},{},{}",
            ((display[0] - BACKDROP[2]) * 0.5).round(),
            BACKDROP[1],
            BACKDROP[2],
            BACKDROP[3]
        );

        // The app's frame loop, minus everything that isn't the capsule.
        let loop_coord = coordinator.clone();
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(8))
                    .await;
                let alive = cx.update(|cx| {
                    motion::poll_tune();
                    loop_coord.update(cx, |coord, cx| coord.island_tick(cx));
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();

        let scene = options.scene;
        let repeat = options.repeat;
        cx.spawn(async move |cx| {
            let mut run = 0;
            loop {
                cx.background_executor().timer(real(LEAD_IN_S)).await;
                let mut at = 0.0;
                for (when, step) in scene.steps() {
                    cx.background_executor().timer(real(when - at)).await;
                    at = when;
                    let _ = coordinator.update(cx, |coord, cx| apply(step, coord, cx));
                }
                cx.background_executor().timer(real(TAIL_S)).await;
                run += 1;
                if repeat != 0 && run >= repeat {
                    break;
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
                        .child(div().w(px(88.0)).h(px(12.0)).rounded(px(4.0)).bg(rgb(0x1e293b)))
                        .child(div().w(px(64.0)).h(px(12.0)).rounded(px(4.0)).bg(rgb(0x2563eb)))
                        .child(div().w(px(108.0)).h(px(12.0)).rounded(px(4.0)).bg(rgb(0x94a3b8)))
                        .child(div().w(px(74.0)).h(px(12.0)).rounded(px(4.0)).bg(rgb(0x10b981)))
                        .child(div().w(px(84.0)).h(px(12.0)).rounded(px(4.0)).bg(rgb(0xf43f5e)))
                        .child(div().w(px(96.0)).h(px(12.0)).rounded(px(4.0)).bg(rgb(0x475569))),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(10.0))
                        .child(div().w(px(104.0)).h(px(11.0)).rounded(px(4.0)).bg(rgb(0x64748b)))
                        .child(div().w(px(78.0)).h(px(11.0)).rounded(px(4.0)).bg(rgb(0xf59e0b)))
                        .child(div().w(px(124.0)).h(px(11.0)).rounded(px(4.0)).bg(rgb(0x0f172a)))
                        .child(div().w(px(68.0)).h(px(11.0)).rounded(px(4.0)).bg(rgb(0x8b5cf6)))
                        .child(div().w(px(116.0)).h(px(11.0)).rounded(px(4.0)).bg(rgb(0x3b82f6))),
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
                        .child(div().w(px(84.0)).h(px(12.0)).rounded(px(4.0)).bg(rgb(0xf43f5e)))
                        .child(div().w(px(104.0)).h(px(12.0)).rounded(px(4.0)).bg(rgb(0x38bdf8)))
                        .child(div().w(px(72.0)).h(px(12.0)).rounded(px(4.0)).bg(rgb(0xfbbf24)))
                        .child(div().w(px(112.0)).h(px(12.0)).rounded(px(4.0)).bg(rgb(0xe2e8f0)))
                        .child(div().w(px(82.0)).h(px(12.0)).rounded(px(4.0)).bg(rgb(0xa855f7)))
                        .child(div().w(px(76.0)).h(px(12.0)).rounded(px(4.0)).bg(rgb(0x34d399))),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(10.0))
                        .child(div().w(px(96.0)).h(px(11.0)).rounded(px(4.0)).bg(rgb(0x34d399)))
                        .child(div().w(px(84.0)).h(px(11.0)).rounded(px(4.0)).bg(rgb(0xf97316)))
                        .child(div().w(px(128.0)).h(px(11.0)).rounded(px(4.0)).bg(rgb(0x60a5fa)))
                        .child(div().w(px(72.0)).h(px(11.0)).rounded(px(4.0)).bg(rgb(0xf472b6)))
                        .child(div().w(px(108.0)).h(px(11.0)).rounded(px(4.0)).bg(rgb(0xcbd5e1))),
                ),
            _ => div().size_full().bg(rgb(0xd9d9de)),
        }
    }
}

/// A plain light panel behind the capsule's window, so recordings don't show whatever is on the
/// desktop. It takes no input.
fn open_backdrop(display: [f32; 2], cx: &mut App) {
    let theme = std::env::var("APPARENT_LAB_BACKDROP")
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
    super::super::platform::configure_lab_backdrop(w, h);
}
