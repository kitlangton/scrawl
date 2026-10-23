//! The Rune desktop app (`"ui": "rune"` in `~/.config/scrawl/config.json` or `SCRAWL_UI=rune`):
//! Rune owns the main thread and event loop (`App::accessory`), drawing the capsule (`rune_pill`)
//! in a top-level overlay, one full-display target-picker overlay per display (`OverlayInput::All`
//! while picking, `OverlayInput::None` while recording), and the bundle review window.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rune::color::srgb_decode;
use rune::{
    App, AppHandle, AppSpec, Chrome, Color, Cursor, Cx, DisplayTarget, Drag, El, Image, ImageCache,
    Key, KeyEvent, KeyPolicy, LineCap, NativeDisplayId, OverlayHandle, OverlayInput, OverlayLevel,
    OverlayOptions, OverlayPlacement, OverlaySpec, Path, Phase, PointerPress, RunOptions,
    ScrollSpec, Shape, TextStyle, Tx, Waker, WindowOptions, WindowSpec, canvas, col, el, row, text,
};

use super::rune_pill::{Capsule, PANEL_H, PANEL_W, PANEL_Y, Surface};
use super::{
    ACCENT_COLOR, ACCENT_HOVER, ActiveRecording, BADGE_H, BADGE_MARGIN, BADGE_PX, BADGE_TEXT,
    BadgeAnchor, CANVAS_BG, CaptureSource, INK_RED, IslandPhase, LINE_COLOR, LaserOverlay,
    MUTED_COLOR, MenuAction, ReviewTab, RouteChip, RouteOutcome, SIDEBAR_BG, SMART_SEND_UNDO,
     SURFACE_BG, SURFACE_HOVER, SURFACE_SELECTED, SelectionMode, TEXT_COLOR, TEXT_SOFT, area_rect,
    clamp_point, clip_rect, clock_template, completed_delivery_note, completed_label, island_title,
    motion, place_badge, reveal_clock_ms, split_timeline_line, start_metric, start_video_export,
};
use crate::audio::AudioRecorder;
use crate::capture::{
    LiveOverlayEvent, ScreenCaptureSession, SharedCaptureTelemetry, current_mouse_screen,
    discover_target, install_global_hotkeys, list_selectable_windows,
    record_external_frontmost_app, region_target, scrawl_log, selectable_window_at,
    shortcut_from_key_and_mods, start_global_input_tap, update_global_shortcut,
};
use crate::helper::PhoneDevice;
use crate::model::{CaptureMode, ScrawlBundle, TargetFrame, TargetInfo, format_clock};
use crate::native::pill::model::{ElemId, Pres};
use crate::native::pill::{PillAction, PillSnapshot, PillStyle};
use crate::phone::PhoneSession;
use crate::sketch::timeline;

#[derive(Clone, Debug, Default)]
pub struct RuneAppOptions {
    pub snap: Option<PathBuf>,
    pub scene: Option<String>,
    pub script: Option<PathBuf>,
}

const ALL_SCENES: &[&str] = &[
    "picker-area",
    "picker-window",
    "picker-screen",
    "review-empty",
    "review-contact-sheet",
    "review-keyframes",
];

fn rgb_color(hex: u32) -> Color {
    let r = ((hex >> 16) & 0xff) as f32 / 255.0;
    let g = ((hex >> 8) & 0xff) as f32 / 255.0;
    let b = (hex & 0xff) as f32 / 255.0;
    Color {
        r: srgb_decode(r),
        g: srgb_decode(g),
        b: srgb_decode(b),
        a: 1.0,
    }
}

fn rgba_color(hex: u32) -> Color {
    let r = ((hex >> 24) & 0xff) as f32 / 255.0;
    let g = ((hex >> 16) & 0xff) as f32 / 255.0;
    let b = ((hex >> 8) & 0xff) as f32 / 255.0;
    let a = (hex & 0xff) as f32 / 255.0;
    Color {
        r: srgb_decode(r),
        g: srgb_decode(g),
        b: srgb_decode(b),
        a,
    }
}

enum InternalEvent {
    PillAction(PillAction),
    PickerDown {
        x: f32,
        y: f32,
    },
    PickerMove {
        x: f32,
        y: f32,
    },
    PickerUp {
        x: f32,
        y: f32,
    },
    PickerKey(KeyEvent),
    WindowsLoaded {
        generation: u64,
        windows: Vec<TargetInfo>,
    },
    IphoneFound(Option<PhoneDevice>),
    CaptureFinalized {
        finalized: anyhow::Result<(PathBuf, ScrawlBundle, String)>,
        duration_ms: u64,
        finish_started: Instant,
        route: Option<std::thread::JoinHandle<Option<crate::model::Destination>>>,
        look: Option<std::thread::JoinHandle<Option<crate::routing::look::Live>>>,
    },
    RouteResolved {
        dir: PathBuf,
        route: RouteOutcome,
        copied_ms: u128,
        finish_started: Instant,
    },
    UndoSendExpired {
        dir: PathBuf,
        dest: crate::model::Destination,
    },
    DeliveryDone {
        dir: PathBuf,
        title: String,
        delivery: crate::delivery::Delivery,
    },
}

// ─── Capsule Overlay (`PillState`) ───────────────────────────────────────────────────────────

const HIDE_AFTER_S: f64 = super::lab::TAIL_S as f64 - 0.1;

struct PillState {
    surface: Surface,
    live: Capsule,
    hidden_since: Option<f64>,
    shown: bool,
    pending: Arc<Mutex<Option<PillSnapshot>>>,
    pill_handle: Arc< std::sync::OnceLock<OverlayHandle<PillState>> >,
    events_tx: Option<flume::Sender<InternalEvent>>,
    app_handle: Option<AppHandle<ReviewState>>,
}

impl PillState {
    fn new(
        pending: Arc<Mutex<Option<PillSnapshot>>>,
        pill_handle: Arc<std::sync::OnceLock<OverlayHandle<PillState>>>,
        events_tx: Option<flume::Sender<InternalEvent>>,
        app_handle: Option<AppHandle<ReviewState>>,
    ) -> Self {
        let surface = super::rune_pill::new_surface();
        let live = Capsule::new(&surface);
        Self {
            surface,
            live,
            hidden_since: None,
            shown: false,
            pending,
            pill_handle,
            events_tx,
            app_handle,
        }
    }

    fn press(&mut self, id: ElemId) {
        if let Some(action) = super::rune_pill::action(&self.surface.pres, id) {
            if let Some(ref tx) = self.events_tx {
                let _ = tx.send(InternalEvent::PillAction(action));
            }
            if let Some(ref app) = self.app_handle {
                app.wake();
            }
        }
    }
}

fn pill_view(s: &PillState, cx: &mut Cx<PillState>) -> El {
    super::rune_pill::capsule(&s.surface, &s.live, cx, PillState::press)
}

fn pill_poll(s: &mut PillState, now: f64, tx: &mut Tx) -> bool {
    motion::poll_tune();
    let tune = motion::tune();
    s.surface.tune = tune;
    s.surface.set_time_scale(now, tune.time_scale);
    let scale = f64::from(s.surface.time_scale);
    let anim = s.surface.anim(now);

    let mut changed = s.surface.tick(now);
    let next_snap = s
        .pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(snap) = next_snap {
        s.surface.reduced = snap.reduced_motion;
        changed |= s.live.set_clock(&mut s.surface, &snap.clock, now);
        if snap.pres != s.surface.pres {
            s.surface.set(snap.pres, now);
            changed = true;
        }
    }
    s.live.sync(&s.surface);

    let mut wake = s.surface.next_due().map(|due| due - now);
    let hidden = matches!(s.surface.pres, Pres::Hidden);
    let since = match (hidden, s.hidden_since) {
        (true, None) => *s.hidden_since.insert(anim),
        (true, Some(since)) => since,
        (false, _) => {
            s.hidden_since = None;
            anim
        }
    };
    let leaving = (since + HIDE_AFTER_S - anim) / scale;
    if hidden && leaving > 0.0 {
        wake = Some(wake.map_or(leaving, |w| w.min(leaving)));
    }
    let shown = !hidden || leaving > 0.0;
    if s.shown != shown {
        s.shown = shown;
        if let Some(pill) = s.pill_handle.get() {
            if shown {
                pill.show();
            } else {
                pill.hide();
            }
        }
    }
    if let Some(wake) = wake {
        tx.wake_after(Duration::from_secs_f64(wake.max(0.0)));
    }
    for event in s.surface.events.drain(..) {
        motion::trace_event(event);
    }
    if std::mem::take(&mut s.surface.instant) {
        tx.instant();
    }
    changed
}

// ─── Full-Display Target Picker Overlay (`PickerState`) ──────────────────────────────────────

#[derive(Clone, Debug)]
struct PickerSnapshot {
    active: bool,
    mode: SelectionMode,
    pointer: Option<[f32; 2]>,
    drag: Option<([f32; 2], [f32; 2])>,
    hovered_window: Option<TargetInfo>,
    animated_window_rect: Option<[f32; 4]>,
    app_bundle_path: Option<PathBuf>,
    iphone_rect: Option<[f32; 4]>,
    iphone_name: String,
    has_iphone: bool,
    selectable_windows: Vec<TargetInfo>,
}

impl Default for PickerSnapshot {
    fn default() -> Self {
        Self {
            active: false,
            mode: SelectionMode::Area,
            pointer: None,
            drag: None,
            hovered_window: None,
            animated_window_rect: None,
            app_bundle_path: None,
            iphone_rect: None,
            iphone_name: "iPhone".to_owned(),
            has_iphone: false,
            selectable_windows: Vec::new(),
        }
    }
}

struct PickerState {
    display_frame: [f32; 4],
    snapshot: PickerSnapshot,
    images: Option<ImageCache>,
    events_tx: Option<flume::Sender<InternalEvent>>,
    app_handle: Option<AppHandle<ReviewState>>,
    /// True in headless `--snap` / `--script` runs so the picker draws its own backdrop scrim and
    /// selection cutout border without `MetalLaserOverlay`.
    still: bool,
}

impl PickerState {
    fn new(
        display_frame: [f32; 4],
        events_tx: Option<flume::Sender<InternalEvent>>,
        app_handle: Option<AppHandle<ReviewState>>,
        still: bool,
    ) -> Self {
        let waker_app = app_handle.clone();
        let images = Some(ImageCache::new(Arc::new(move || {
            if let Some(ref app) = waker_app {
                app.wake();
            }
        })));
        Self {
            display_frame,
            snapshot: PickerSnapshot::default(),
            images,
            events_tx,
            app_handle,
            still,
        }
    }

    fn emit(&self, ev: InternalEvent) {
        if let Some(ref tx) = self.events_tx {
            let _ = tx.send(ev);
        }
        if let Some(ref app) = self.app_handle {
            app.wake();
        }
    }
}

fn picker_setup(s: &mut PickerState, waker: Waker) {
    s.images = Some(ImageCache::new(waker));
}

fn camera_cursor_badge(x: f32, y: f32) -> El {
    el().key("picker-camera-cursor")
        .absolute(x - 20.0, y - 20.0)
        .size(40.0, 40.0)
        .radius(20.0)
        .fill(rgba_color(0x161619eb))
        .border(1.0, rgba_color(0xffffff38))
        .shadow(3.0, 10.0, Color::BLACK.alpha(0.45))
        .child(
            el().absolute(15.5, 9.5)
                .size(9.0, 2.5)
                .corners([1.5, 1.5, 0.0, 0.0])
                .fill(rgb_color(0xf5f5f7)),
        )
        .child(
            el().absolute(9.5, 12.0)
                .size(21.0, 14.5)
                .radius(3.5)
                .fill(rgb_color(0xf5f5f7))
                .child(
                    el().absolute(6.0, 2.75)
                        .size(9.0, 9.0)
                        .radius(4.5)
                        .fill(rgb_color(0x161619))
                        .child(
                            el().absolute(2.25, 2.25)
                                .size(4.5, 4.5)
                                .radius(2.25)
                                .fill(rgb_color(0xf5f5f7)),
                        ),
                ),
        )
}

fn crosshair_cursor(x: f32, y: f32) -> El {
    let shape = Shape::new()
        .view_box(24.0, 24.0)
        .stroke_path(
            &Path::new().move_to([12.0, 2.0]).line_to([12.0, 22.0]),
            1.5,
            LineCap::Round,
            Color::WHITE.alpha(0.92),
        )
        .stroke_path(
            &Path::new().move_to([2.0, 12.0]).line_to([22.0, 12.0]),
            1.5,
            LineCap::Round,
            Color::WHITE.alpha(0.92),
        );
    canvas(shape)
        .key("picker-crosshair")
        .absolute(x - 12.0, y - 12.0)
        .size(24.0, 24.0)
}

fn picker_view(s: &PickerState, _cx: &mut Cx<PickerState>) -> El {
    let screen = [s.display_frame[2], s.display_frame[3]];
    if !s.snapshot.active && !s.still {
        return el().key("scrawl-target-picker-idle").size(screen[0], screen[1]);
    }
    let origin = [s.display_frame[0], s.display_frame[1]];
    let mode = s.snapshot.mode;

    let highlight: Option<([f32; 4], String)> = match mode {
        SelectionMode::Area => s.snapshot.drag.map(|(a, b)| {
            let [a, b] = [a, b].map(|p| clamp_point(p, s.display_frame));
            let w = (a[0] - b[0]).abs().max(1.0);
            let h = (a[1] - b[1]).abs().max(1.0);
            (
                [
                    a[0].min(b[0]) - origin[0],
                    a[1].min(b[1]) - origin[1],
                    w,
                    h,
                ],
                format!("{} × {}", w.round() as i32, h.round() as i32),
            )
        }),
        SelectionMode::Window => s.snapshot.hovered_window.as_ref().map(|t| {
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
            let [rx, ry, rw, rh] = s.snapshot.animated_window_rect.unwrap_or([
                t.frame.x as f32,
                t.frame.y as f32,
                t.frame.width as f32,
                t.frame.height as f32,
            ]);
            ([rx - origin[0], ry - origin[1], rw, rh], title)
        }),
        SelectionMode::Screen => Some((
            [0.0, 0.0, screen[0], screen[1]],
            "Full screen · Click to record".to_owned(),
        )),
        SelectionMode::Iphone => s.snapshot.iphone_rect.map(|[rx, ry, rw, rh]| {
            (
                [rx - origin[0], ry - origin[1], rw, rh],
                format!("{} · Click to record", s.snapshot.iphone_name),
            )
        }),
    }
    .map(|(rect, label)| (clip_rect(rect, screen), label));

    let icon_img: Option<Image> = if mode == SelectionMode::Window {
        s.snapshot
            .app_bundle_path
            .as_ref()
            .and_then(|path| {
                s.images.as_ref().map(|cache| {
                    let img = cache.app_icon(path, 16.0);
                    if s.still {
                        let _ = img.wait();
                    }
                    img
                })
            })
    } else {
        None
    };
    let icon_extra = if icon_img.is_some() { 20.0 } else { 0.0 };

    let badge = highlight.as_ref().map(|(rect, label)| {
        let max_text = (screen[0] - 2.0 * BADGE_MARGIN - 2.0 * BADGE_PX - 2.0 - icon_extra).min(480.0);
        let measured_label = if mode == SelectionMode::Area {
            clock_template(label)
        } else {
            label.clone()
        };
        let text_w = (super::rune_pill::measure(&measured_label, false) * (BADGE_TEXT / 13.0))
            .ceil()
            .min(max_text);
        let size = [text_w + icon_extra + 2.0 * BADGE_PX + 2.0, BADGE_H];
        let anchor = match mode {
            SelectionMode::Area | SelectionMode::Iphone => BadgeAnchor::BelowTrailing,
            SelectionMode::Window | SelectionMode::Screen => BadgeAnchor::Center,
        };
        (
            place_badge(*rect, size, screen, anchor),
            size[0],
            text_w,
            label.clone(),
        )
    });

    let cursor = match mode {
        SelectionMode::Screen | SelectionMode::Iphone => Cursor::Pointer,
        SelectionMode::Area | SelectionMode::Window => Cursor::Default,
    };

    let mut root = el()
        .key("scrawl-target-picker")
        .size(screen[0], screen[1])
        .cursor(cursor)
        .hit()
        .on_pointer_move(|state: &mut PickerState, p: PointerPress| {
            let global = if state.still {
                [state.display_frame[0] + p.x, state.display_frame[1] + p.y]
            } else {
                current_mouse_screen()
                    .unwrap_or([state.display_frame[0] + p.x, state.display_frame[1] + p.y])
            };
            state.snapshot.pointer = Some(global);
            if state.snapshot.mode == SelectionMode::Window {
                let found = selectable_window_at(
                    &state.snapshot.selectable_windows,
                    f64::from(global[0]),
                    f64::from(global[1]),
                )
                .cloned();
                state.snapshot.animated_window_rect = found.as_ref().map(|w| {
                    [
                        w.frame.x as f32,
                        w.frame.y as f32,
                        w.frame.width as f32,
                        w.frame.height as f32,
                    ]
                });
                state.snapshot.app_bundle_path = found
                    .as_ref()
                    .and_then(|w| w.bundle_id.as_deref())
                    .and_then(super::platform::app_bundle_path);
                state.snapshot.hovered_window = found;
            }
            state.emit(InternalEvent::PickerMove {
                x: global[0],
                y: global[1],
            });
        })
        .on_drag(|state: &mut PickerState, drag: &mut Drag, _tx: &mut Tx| {
            let global = if state.still {
                [
                    state.display_frame[0] + drag.x,
                    state.display_frame[1] + drag.y,
                ]
            } else {
                current_mouse_screen().unwrap_or([
                    state.display_frame[0] + drag.x,
                    state.display_frame[1] + drag.y,
                ])
            };
            state.snapshot.pointer = Some(global);
            match drag.phase {
                Phase::Down => {
                    if state.snapshot.mode == SelectionMode::Area {
                        state.snapshot.drag = Some((global, global));
                    }
                    state.emit(InternalEvent::PickerDown {
                        x: global[0],
                        y: global[1],
                    });
                }
                Phase::Move => {
                    if state.snapshot.mode == SelectionMode::Area
                        && let Some((start, _)) = state.snapshot.drag
                    {
                        state.snapshot.drag = Some((start, global));
                    }
                    state.emit(InternalEvent::PickerMove {
                        x: global[0],
                        y: global[1],
                    });
                }
                Phase::Up => {
                    state.emit(InternalEvent::PickerUp {
                        x: global[0],
                        y: global[1],
                    });
                }
            }
        });

    if s.still {
        root = root.fill(rgb_color(0x1b1d24)).child(
            el().key("picker-scrim")
                .absolute(0.0, 0.0)
                .size(screen[0], screen[1])
                .fill(Color::BLACK.alpha(0.42)),
        );
        if let Some(([rx, ry, rw, rh], _)) = highlight {
            root = root.child(
                el().key("picker-cutout")
                    .absolute(rx, ry)
                    .size(rw, rh)
                    .radius(if mode == SelectionMode::Window { 10.0 } else { 4.0 })
                    .fill(rgb_color(0x232631))
                    .border(1.5, Color::WHITE.alpha(0.88))
                    .shadow(0.0, 18.0, rgb_color(ACCENT_COLOR).alpha(0.35)),
            );
        }
    }

    if let Some([px_screen, py_screen]) = s.snapshot.pointer {
        let cx_local = px_screen - origin[0];
        let cy_local = py_screen - origin[1];
        if mode == SelectionMode::Window {
            root = root.child(camera_cursor_badge(cx_local, cy_local));
        } else if mode == SelectionMode::Area {
            root = root.child(crosshair_cursor(cx_local, cy_local));
        }
    }

    if let Some(([bx, by], bw, text_w, label)) = badge {
        let badge_style = TextStyle::new(BADGE_TEXT, 500.0).features("'tnum' 1");
        let mut badge_row = row()
            .key("picker-badge")
            .absolute(bx, by)
            .size(bw, BADGE_H)
            .px(BADGE_PX)
            .radius(BADGE_H * 0.5)
            .center_y()
            .gap(4.0)
            .fill(rgba_color(0x1c1c1ef2))
            .border(1.0, rgba_color(0xffffff21))
            .shadow(2.0, 8.0, Color::BLACK.alpha(0.30));
        if let Some(ref img) = icon_img {
            badge_row = badge_row.child(el().size(15.0, 15.0).radius(3.5).pixels(img.pixels()));
        }
        badge_row = badge_row.child(
            text(&label)
                .font(badge_style)
                .color(rgb_color(TEXT_COLOR))
                .w(text_w)
                .truncate(),
        );
        root = root.child(badge_row);
    }

    root
}

fn picker_on_key(s: &mut PickerState, event: &KeyEvent, _tx: &mut Tx) -> bool {
    let has_iphone = s.snapshot.has_iphone;
    match &event.key {
        Key::Escape => {
            s.snapshot.active = false;
            s.snapshot.drag = None;
        }
        Key::Text(ch) if ch == " " => {
            s.snapshot.mode = match s.snapshot.mode {
                SelectionMode::Area => SelectionMode::Window,
                _ => SelectionMode::Area,
            };
        }
        Key::Tab => {
            s.snapshot.mode = s.snapshot.mode.cycled(!event.mods.shift, has_iphone);
        }
        Key::Right => {
            s.snapshot.mode = s.snapshot.mode.cycled(true, has_iphone);
        }
        Key::Left => {
            s.snapshot.mode = s.snapshot.mode.cycled(false, has_iphone);
        }
        Key::Text(ch) => {
            if let Some(next) = SelectionMode::from_digit(ch, has_iphone) {
                s.snapshot.mode = next;
            }
        }
        _ => {}
    }
    s.emit(InternalEvent::PickerKey(event.clone()));
    true
}

// ─── Main-Thread Coordinator (`RuneCoordinator`) ────────────────────────────────────────────

struct DisplaySlot {
    id: u32,
    frame: [f64; 4],
    pill_local: [f32; 2],
    pill_global: [f64; 4],
    picker: OverlayHandle<PickerState>,
}

struct RuneCoordinator {
    telemetry: Arc<SharedCaptureTelemetry>,
    ui_receiver: Option<flume::Receiver<LiveOverlayEvent>>,
    events_tx: flume::Sender<InternalEvent>,
    events_rx: flume::Receiver<InternalEvent>,
    menu_rx: Option<flume::Receiver<MenuAction>>,
    shown_export_status: Option<String>,
    status_bar_installed: bool,
    pending_start_record: bool,
    pending_start_iphone: bool,
    app_handle: Option<AppHandle<ReviewState>>,
    pill_overlay: Option<OverlayHandle<PillState>>,
    pill_pending: Arc<Mutex<Option<PillSnapshot>>>,
    last_pill_snapshot: Option<PillSnapshot>,
    pill_global_rect: [f64; 4],
    displays: Vec<DisplaySlot>,
    persistent_excluded_window_ids: Arc<Mutex<Vec<u32>>>,
    review_open: bool,

    active_recording: Option<ActiveRecording>,
    playback: Option<crate::playback::Guard>,
    picker_metal_overlay: Option<LaserOverlay>,
    iphone_preview: Option<PhoneSession>,
    island_phase: IslandPhase,
    reveal: crate::shortcut::Reveal,
    reduced_motion: bool,
    island_stepped: Instant,
    last_elapsed_ms: u64,
    selectable_windows: Vec<TargetInfo>,
    selection_generation: u64,
    windows_loading: bool,
    selection_pointer: Option<[f32; 2]>,
    hovered_window: Option<TargetInfo>,
    animated_window_rect: Option<[f32; 4]>,
    selection_drag: Option<([f32; 2], [f32; 2])>,
    display_rect: [f32; 4],
    recording_shortcut_key: bool,
    review_bundle: Option<(PathBuf, ScrawlBundle)>,
    review_status: Option<String>,
    selected_tab: ReviewTab,
    selected_keyframe: usize,
    iphone: Option<PhoneDevice>,
    completed_dir: Option<PathBuf>,
    completed_note: Option<String>,
    pending_send: Option<PathBuf>,
    routing: Option<PathBuf>,
    route_chips: Vec<RouteChip>,
    last_local_capture_ms: u64,
    inbox: crate::inbox::Inbox,
    last_mode: SelectionMode,
    open_iphone_when_found: bool,
    onboarding: Option<crate::onboarding::Session>,
    settings: Option<crate::settings::Session>,
    grants: Option<crate::onboarding::Grants>,
    gate: crate::onboarding::Gate,
    shortcut: crate::shortcut::Shortcut,
    hold_mods: u64,
    should_quit: bool,
}

impl RuneCoordinator {
    #[allow(clippy::too_many_arguments)]
    fn new(
        telemetry: Arc<SharedCaptureTelemetry>,
        ui_receiver: Option<flume::Receiver<LiveOverlayEvent>>,
        events_tx: flume::Sender<InternalEvent>,
        events_rx: flume::Receiver<InternalEvent>,
        initial_bundle: Option<(PathBuf, ScrawlBundle)>,
        pending_start_record: bool,
        pending_start_iphone: bool,
        app_handle: Option<AppHandle<ReviewState>>,
        pill_overlay: Option<OverlayHandle<PillState>>,
        pill_pending: Arc<Mutex<Option<PillSnapshot>>>,
        displays: Vec<DisplaySlot>,
        persistent_excluded_window_ids: Arc<Mutex<Vec<u32>>>,
    ) -> Self {
        crate::playback::warm_up();
        let review_open = initial_bundle.is_some();
        let pill_global_rect = displays
            .first()
            .map_or([0.0, 0.0, f64::from(PANEL_W), f64::from(PANEL_H)], |d| {
                d.pill_global
            });
        let display_rect = displays
            .first()
            .map_or([0.0, 0.0, 1440.0, 900.0], |d| d.frame.map(|v| v as f32));
        Self {
            telemetry,
            ui_receiver,
            events_tx,
            events_rx,
            menu_rx: None,
            shown_export_status: None,
            status_bar_installed: false,
            pending_start_record,
            pending_start_iphone,
            app_handle,
            pill_overlay,
            pill_pending,
            last_pill_snapshot: None,
            pill_global_rect,
            displays,
            persistent_excluded_window_ids,
            review_open,
            active_recording: None,
            playback: None,
            picker_metal_overlay: None,
            iphone_preview: None,
            island_phase: IslandPhase::Hidden,
            reveal: crate::shortcut::Reveal::default(),
            reduced_motion: false,
            island_stepped: Instant::now(),
            last_elapsed_ms: 0,
            selectable_windows: Vec::new(),
            selection_generation: 0,
            windows_loading: false,
            selection_pointer: None,
            hovered_window: None,
            animated_window_rect: None,
            selection_drag: None,
            display_rect,
            recording_shortcut_key: false,
            review_bundle: initial_bundle,
            review_status: None,
            selected_tab: ReviewTab::ContactSheet,
            selected_keyframe: 0,
            iphone: None,
            completed_dir: None,
            completed_note: None,
            pending_send: None,
            routing: None,
            route_chips: Vec::new(),
            last_local_capture_ms: 0,
            inbox: crate::inbox::Inbox::load_default(),
            last_mode: SelectionMode::load_last_used(),
            open_iphone_when_found: false,
            onboarding: None,
            settings: None,
            grants: None,
            gate: crate::onboarding::Gate::default(),
            shortcut: crate::shortcut::Shortcut::default(),
            hold_mods: 0,
            should_quit: false,
        }
    }

    fn restore_excluded_windows(&self) {
        let persistent = self
            .persistent_excluded_window_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut excluded = self
            .telemetry
            .excluded_window_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for id in persistent {
            if !excluded.contains(&id) {
                excluded.push(id);
            }
        }
    }

    fn place_pill_on(&mut self, point: [f64; 2]) {
        let Some(slot_idx) = self
            .displays
            .iter()
            .position(|d| crate::screens::contains(d.frame, point))
            .or_else(|| (!self.displays.is_empty()).then_some(0))
        else {
            return;
        };
        let slot = &self.displays[slot_idx];
        self.pill_global_rect = slot.pill_global;
        if let Some(ref pill) = self.pill_overlay {
            pill.set_display(DisplayTarget::Native(NativeDisplayId::MacOS(slot.id)));
            pill.set_placement(Some(OverlayPlacement::Rect {
                x: slot.pill_local[0],
                y: slot.pill_local[1],
                width: PANEL_W,
                height: PANEL_H,
            }));
        }
    }

    fn place_capsule(&mut self) {
        let centre = |[x, y, w, h]: [f64; 4]| [x + w / 2.0, y + h / 2.0];
        let anchor = match self.island_phase {
            IslandPhase::Recording => self
                .active_recording
                .as_ref()
                .filter(|rec| rec.target.mode != CaptureMode::Device)
                .map(|rec| {
                    let f = rec.target.frame;
                    centre([f.x, f.y, f.width, f.height])
                }),
            IslandPhase::Selecting { .. } => Some(centre(self.display_rect.map(f64::from))),
            _ => None,
        };
        if let Some(point) = anchor.or_else(crate::screens::pointer) {
            self.place_pill_on(point);
        }
    }

    fn pointer_on_pill(&self) -> bool {
        let Some([px, py]) = crate::screens::pointer() else {
            return false;
        };
        let [x, y, w, h] = self.pill_global_rect;
        px >= x - 24.0 && px <= x + w + 24.0 && py >= y - 16.0 && py <= y + h + 24.0
    }

    fn pill_snapshot(&self) -> PillSnapshot {
        let recording_ms = self
            .active_recording
            .as_ref()
            .map(ActiveRecording::elapsed_ms);
        let elapsed_ms = recording_ms.unwrap_or(self.last_elapsed_ms);
        let clock = format_clock(elapsed_ms);
        let has_iphone = self.iphone.is_some();

        let pres = match self.island_phase {
            IslandPhase::Hidden | IslandPhase::Dismissing => Pres::Hidden,
            IslandPhase::Selecting { mode } => Pres::Selecting { mode, has_iphone },
            IslandPhase::Recording => Pres::Recording {
                chips: self.reveal.is_open(),
                has_iphone,
            },
            IslandPhase::Processing => Pres::Processing,
            IslandPhase::Completed { inbox_count, .. } => Pres::Completed {
                label: completed_label(inbox_count, self.completed_note.as_deref()),
                routes: self.route_chip_labels(),
            },
        };
        PillSnapshot {
            pres,
            clock,
            reduced_motion: self.reduced_motion,
            style: PillStyle::current(),
        }
    }

    fn sync_rune_pill(&mut self) {
        let snap = self.pill_snapshot();
        if self.last_pill_snapshot.as_ref() != Some(&snap) {
            self.last_pill_snapshot = Some(snap.clone());
            *self
                .pill_pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(snap);
            if let Some(ref pill) = self.pill_overlay {
                pill.update(|_, _| {});
            }
        }
    }

    fn sync_picker_overlays(&self) {
        let selecting_mode = match self.island_phase {
            IslandPhase::Selecting { mode } => Some(mode),
            _ => None,
        };
        let iphone_rect = self.iphone_preview.as_ref().and_then(PhoneSession::rect);
        let iphone_name = self
            .iphone_preview
            .as_ref()
            .and_then(PhoneSession::device)
            .or_else(|| self.iphone.clone())
            .map_or_else(|| "iPhone".to_owned(), |d| d.name);
        let app_bundle_path = self
            .hovered_window
            .as_ref()
            .and_then(|w| w.bundle_id.as_deref())
            .and_then(super::platform::app_bundle_path);
        let has_iphone = self.iphone.is_some();

        for slot in &self.displays {
            let on_this_display = selecting_mode.is_some()
                && (0..4).all(|k| (slot.frame[k] as f32 - self.display_rect[k]).abs() < 2.0);
            let snap = PickerSnapshot {
                active: on_this_display,
                mode: selecting_mode.unwrap_or(SelectionMode::Area),
                pointer: self.selection_pointer,
                drag: self.selection_drag,
                hovered_window: self.hovered_window.clone(),
                animated_window_rect: self.animated_window_rect,
                app_bundle_path: app_bundle_path.clone(),
                iphone_rect,
                iphone_name: iphone_name.clone(),
                has_iphone,
                selectable_windows: self.selectable_windows.clone(),
            };
            slot.picker.update(move |state, _| {
                state.snapshot = snap;
            });
            if on_this_display {
                slot.picker.set_input(OverlayInput::All);
                slot.picker.show();
                slot.picker.request_keys();
            } else {
                slot.picker.set_input(OverlayInput::None);
                slot.picker.hide();
            }
        }
    }

    fn open_review_window(&mut self) {
        self.review_open = true;
        if let Some(ref app) = self.app_handle {
            app.open();
            app.focus();
        }
        super::platform::take_keys_now();
    }

    fn close_review_window(&mut self) {
        if self.review_open {
            self.review_open = false;
            if let Some(ref app) = self.app_handle {
                app.hide();
            }
        }
    }

    fn hand_back_keys_if_idle(&self) {
        if !self.is_selecting() && !self.review_open {
            super::platform::hand_back_keys_now();
        }
    }

    fn remember_mode(&mut self, mode: SelectionMode) {
        if self.last_mode != mode {
            self.last_mode = mode;
            mode.save_last_used();
        }
    }

    fn add_to_inbox(&mut self, dir: &std::path::Path, bundle: &ScrawlBundle) -> usize {
        let source = bundle.source.as_deref().unwrap_or("iphone");
        self.inbox.add(crate::inbox::InboxEntry::new(
            dir,
            bundle,
            source,
            crate::now_epoch_ms(),
        ));
        super::platform::set_status_inbox_count(self.inbox.len());
        self.inbox.len()
    }

    fn mark_handled(&mut self, dir: &std::path::Path) {
        if self.inbox.remove(dir) {
            super::platform::set_status_inbox_count(self.inbox.len());
        }
    }

    fn take_paste_target(&mut self) -> PathBuf {
        let showing = matches!(self.island_phase, IslandPhase::Completed { .. })
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

    fn paste_latest(&mut self) {
        if !crate::capture::accessibility_granted() {
            scrawl_log("⌥⇧V without Accessibility; asking for it");
            self.gate
                .defer(vec![crate::onboarding::Permission::Accessibility]);
            return;
        }
        let dir = self.take_paste_target();
        self.keep_pending_send(&dir);
        if matches!(self.island_phase, IslandPhase::Completed { .. }) {
            self.dismiss_island();
        }
        std::thread::spawn(move || {
            if let Err(err) = crate::handoff::paste_bundle_image_and_text(&dir) {
                scrawl_log(format!("paste_bundle_image_and_text error: {err}"));
            }
        });
    }

    fn refresh_iphone(&mut self) {
        let tx = self.events_tx.clone();
        let app = self.app_handle.clone();
        std::thread::spawn(move || {
            let found = crate::helper::devices()
                .inspect_err(|error| scrawl_log(format!("iPhone discovery: {error:#}")))
                .ok()
                .and_then(|devices| devices.into_iter().next());
            let _ = tx.send(InternalEvent::IphoneFound(found));
            if let Some(app) = app {
                app.wake();
            }
        });
    }

    fn is_recording(&self) -> bool {
        self.active_recording.is_some()
    }

    fn is_processing(&self) -> bool {
        matches!(self.island_phase, IslandPhase::Processing)
    }

    fn is_selecting(&self) -> bool {
        matches!(self.island_phase, IslandPhase::Selecting { .. })
    }

    fn is_island_visible(&self) -> bool {
        !matches!(self.island_phase, IslandPhase::Hidden)
    }

    fn shortcut_label(&self) -> String {
        self.telemetry
            .shortcut
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .label
            .clone()
    }

    fn set_review_bundle(&mut self, bundle: (PathBuf, ScrawlBundle), status: Option<String>) {
        self.review_bundle = Some(bundle);
        self.review_status = status;
        self.selected_tab = ReviewTab::ContactSheet;
        self.selected_keyframe = 0;
    }

    fn open_onboarding(&mut self, flow: crate::onboarding::Flow) {
        self.onboarding = None;
        let sender = self.telemetry.ui_sender.clone();
        let events = move |event| {
            let _ = sender.send(LiveOverlayEvent::Onboarding(event));
        };
        match crate::onboarding::Session::open(flow, &self.shortcut_label(), events) {
            Ok(session) => self.onboarding = Some(session),
            Err(error) => scrawl_log(format!("onboarding could not open: {error:#}")),
        }
    }

    fn open_settings(&mut self, pane: Option<crate::settings::Pane>) {
        if let Some(session) = self.settings.as_mut() {
            session.show(pane);
            return;
        }
        let sender = self.telemetry.ui_sender.clone();
        let events = move |event| {
            let _ = sender.send(LiveOverlayEvent::Settings(event));
        };
        match crate::settings::Session::open(pane, self.grants.clone(), events) {
            Ok(mut session) => {
                session.sync(&crate::settings::AppHost {
                    telemetry: &self.telemetry,
                });
                self.settings = Some(session);
            }
            Err(error) => scrawl_log(format!("Settings could not open: {error:#}")),
        }
    }

    fn settings_event(&mut self, event: crate::settings::Event) {
        use crate::settings::{Effect, Event};
        if let Event::Grants(grants) = &event {
            self.note_grants(grants);
        }
        let Some(session) = self.settings.as_mut() else {
            return;
        };
        let mut host = crate::settings::AppHost {
            telemetry: &self.telemetry,
        };
        match session.event(event, &mut host) {
            Effect::Close => self.settings = None,
            Effect::Tour => self.open_onboarding(crate::onboarding::Flow::tour(
                crate::onboarding::Step::Welcome,
                self.grants.clone(),
            )),
            Effect::Relaunch => {
                self.settings = None;
                crate::onboarding::relaunch();
                self.should_quit = true;
            }
            _ => {}
        }
    }

    fn note_grants(&mut self, grants: &crate::onboarding::Grants) {
        use crate::onboarding::Progress;
        let mut progress = Progress::load();
        if progress.note(grants) {
            progress.save();
        }
        self.grants = Some(grants.clone());
    }

    fn missing_to_record(&mut self) -> Vec<crate::onboarding::Permission> {
        use crate::onboarding::{Permission, missing_to_record};
        let screen = crate::capture::screen_capture_usable(false);
        let accessibility = crate::capture::accessibility_granted();
        let missing = missing_to_record(self.grants.as_ref(), screen, accessibility);
        if missing.contains(&Permission::Microphone)
            && let Some(fresh) = crate::onboarding::read_grants()
        {
            self.note_grants(&fresh);
            return missing_to_record(self.grants.as_ref(), screen, accessibility);
        }
        missing
    }

    fn ready_to_record(&mut self) -> bool {
        let missing = self.missing_to_record();
        if missing.is_empty() {
            return true;
        }
        scrawl_log(format!(
            "Not recording: {missing:?} off; opening onboarding"
        ));
        self.gate.defer(missing);
        false
    }

    fn ready_for(&mut self, permission: crate::onboarding::Permission) -> bool {
        let granted = |g: &crate::onboarding::Grants| g.granted(permission);
        if self.grants.as_ref().is_none_or(granted) {
            return true;
        }
        if let Some(fresh) = crate::onboarding::read_grants() {
            self.note_grants(&fresh);
            if granted(&fresh) {
                return true;
            }
        }
        scrawl_log(format!("{permission:?} is off; asking for it"));
        self.gate.defer(vec![permission]);
        false
    }

    fn tick_gate(&mut self) {
        use crate::onboarding::{Flow, Input, Mode, Permission};
        let Some(asks) = self.gate.tick(crate::capture::modifier_flags() != 0) else {
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

    fn onboarding_event(&mut self, event: crate::onboarding::Event) {
        use crate::onboarding::{Effect, Event, Progress};
        let effect = match event {
            Event::Launch(grants) => {
                self.note_grants(&grants);
                let mut progress = Progress::load();
                let plan = crate::onboarding::launch_plan(&progress, &grants);
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
                self.onboarding
                    .as_mut()
                    .map_or(Effect::None, |session| session.grants(grants))
            }
            Event::Action(action) => self
                .onboarding
                .as_mut()
                .map_or(Effect::None, |session| session.action(&action)),
            Event::Exited => {
                self.onboarding = None;
                Effect::None
            }
        };
        match effect {
            Effect::Close | Effect::Settings => {
                if effect == Effect::Settings {
                    self.open_settings(None);
                }
                let mut progress = Progress::load();
                progress.seen = true;
                progress.resume = None;
                progress.save();
                self.onboarding = None;
            }
            Effect::Relaunch { resume } => {
                scrawl_log(format!(
                    "Screen Recording granted but not usable here; relaunching (resume {resume:?})"
                ));
                let mut progress = Progress::load();
                progress.relaunching(resume, crate::onboarding::unix_ms());
                progress.save();
                self.onboarding = None;
                crate::onboarding::relaunch();
                self.should_quit = true;
            }
            _ => {}
        }
    }

    fn activity(&self) -> crate::shortcut::Activity {
        use crate::shortcut::Activity;
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

    fn begin_playback(&mut self) {
        if self.playback.is_none() {
            self.playback = crate::playback::Guard::start();
        }
    }

    fn press_shortcut(&mut self) {
        let activity = self.activity();
        if self.gate.is_pending() {
            return;
        }
        if !self.shortcut.is_pending() && activity == crate::shortcut::Activity::Free {
            let missing = self.missing_to_record();
            if !self.gate.press(missing) {
                scrawl_log("Shortcut held back: a required permission is off");
                return;
            }
            self.begin_playback();
            self.hold_mods = crate::capture::modifier_flags();
            self.telemetry.take_escape();
        }
        let action = self.shortcut.press(activity);
        self.apply_shortcut(action);
    }

    fn tick_shortcut(&mut self) {
        if !self.shortcut.is_pending() {
            return;
        }
        let action = if self.telemetry.take_escape() {
            self.shortcut.escape()
        } else {
            let mods = crate::capture::modifier_flags();
            let held = self.hold_mods != 0 && mods & self.hold_mods == self.hold_mods;
            self.shortcut.tick(held)
        };
        self.apply_shortcut(action);
    }

    fn apply_shortcut(&mut self, action: crate::shortcut::Action) {
        use crate::shortcut::Action;
        if action != Action::None {
            scrawl_log(format!("shortcut -> {action:?}"));
        }
        match action {
            Action::None => {}
            Action::Cancel => {
                self.playback.take();
            }
            Action::RecordScreen => self.record_screen_now(),
            Action::RecordSelected => self.record_selected_mode(),
            Action::Finish => self.finish_recording(),
        }
    }

    fn record_or_finish(&mut self) {
        if self.is_recording() {
            self.finish_recording();
        } else if self.is_selecting() {
            self.record_selected_mode();
        } else if !self.is_processing() {
            self.record_screen_now();
        }
    }

    fn record_screen_now(&mut self) {
        let target = discover_target(None);
        self.start_recording_with_target(target);
        if self.is_recording() {
            self.show_reveal();
        }
    }

    fn reveal_activity(&self) -> usize {
        use std::sync::PoisonError;
        let t = &self.telemetry;
        t.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
            + t.strokes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .len()
            + usize::from(
                t.live_ink
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .is_some(),
            )
    }

    fn show_reveal(&mut self) {
        self.refresh_iphone();
        self.reduced_motion = super::platform::reduce_motion();
        let activity = self.reveal_activity();
        self.reveal.show(reveal_clock_ms(), activity);
        self.sync_rune_pill();
    }

    fn fold_reveal(&mut self) {
        if self.reveal.is_open() {
            self.reveal.close();
            self.sync_rune_pill();
        }
    }

    fn switch_recording(&mut self, mode: SelectionMode) {
        if !self.is_recording() {
            return;
        }
        if mode == SelectionMode::Screen {
            self.fold_reveal();
            return;
        }
        scrawl_log(format!("switch_recording -> {mode:?}"));
        let Some((_, overlay)) = self.discard_recording() else {
            return;
        };
        if mode == SelectionMode::Iphone {
            self.fold_reveal();
            drop(overlay);
            self.start_recording_iphone();
            return;
        }
        self.picker_metal_overlay = overlay;
        self.open_picker(Some(mode));
    }

    fn discard_recording(&mut self) -> Option<(TargetInfo, Option<LaserOverlay>)> {
        let rec = self.active_recording.take()?;
        let _ = self.telemetry.finish_session();
        match rec.source {
            CaptureSource::Screen { session, audio, .. } => {
                let _ = session.stop();
                if let Some(audio) = audio {
                    let _ = audio.stop();
                }
            }
            CaptureSource::Phone(session) => session.cancel(),
            CaptureSource::Lab => return Some((rec.target, rec._metal_overlay)),
        }
        let _ = std::fs::remove_dir_all(rec.bundle_dir);
        Some((rec.target, rec._metal_overlay))
    }

    fn sync_picker_overlay(&self) {
        let rect = match self.island_phase {
            IslandPhase::Selecting {
                mode: SelectionMode::Area,
            } => self.selection_drag.and_then(|(a, b)| {
                let [a, b] = [a, b].map(|p| clamp_point(p, self.display_rect));
                let w = (a[0] - b[0]).abs();
                let h = (a[1] - b[1]).abs();
                (w >= 1.0 && h >= 1.0).then_some(([a[0].min(b[0]), a[1].min(b[1]), w, h], 1))
            }),
            IslandPhase::Selecting {
                mode: SelectionMode::Window,
            } => self.hovered_window.as_ref().map(|t| {
                (
                    [
                        t.frame.x as f32,
                        t.frame.y as f32,
                        t.frame.width as f32,
                        t.frame.height as f32,
                    ],
                    2,
                )
            }),
            IslandPhase::Selecting {
                mode: SelectionMode::Screen,
            } => Some((self.display_rect, 2)),
            IslandPhase::Selecting {
                mode: SelectionMode::Iphone,
            } => self
                .iphone_preview
                .as_ref()
                .and_then(PhoneSession::rect)
                .map(|r| (r, 3)),
            _ => None,
        };
        self.telemetry.set_picker_rect(rect);
    }

    fn set_selection_mode(&mut self, mode: SelectionMode) {
        self.open_iphone_when_found = false;
        if matches!(self.island_phase, IslandPhase::Selecting { .. }) {
            if mode != SelectionMode::Iphone
                && let Some(ref mut preview) = self.iphone_preview
            {
                preview.hide_preview();
            }
            self.island_phase = IslandPhase::Selecting { mode };
            self.selection_drag = None;
            if let Some(pos) = current_mouse_screen().or(self.selection_pointer) {
                self.selection_pointer = Some(pos);
                if mode == SelectionMode::Window {
                    self.hovered_window = selectable_window_at(
                        &self.selectable_windows,
                        f64::from(pos[0]),
                        f64::from(pos[1]),
                    )
                    .cloned();
                    self.animated_window_rect = self.hovered_window.as_ref().map(|w| {
                        [
                            w.frame.x as f32,
                            w.frame.y as f32,
                            w.frame.width as f32,
                            w.frame.height as f32,
                        ]
                    });
                } else {
                    self.animated_window_rect = None;
                }
            }
            self.sync_picker_overlay();
            self.sync_picker_overlays();
            self.sync_rune_pill();
        }
    }

    fn preview_iphone(&mut self) {
        if self.is_recording() || self.is_processing() {
            return;
        }
        scrawl_log("preview_iphone invoked");
        if !self.ready_for(crate::onboarding::Permission::Camera) {
            self.cancel_recording();
            return;
        }
        self.set_selection_mode(SelectionMode::Iphone);
        if let Some(ref mut preview) = self.iphone_preview {
            preview.show_preview();
        } else {
            let dev_id = self.iphone.as_ref().map(|d| d.id.as_str());
            match PhoneSession::start_preview(dev_id, false, Some(Arc::clone(&self.telemetry))) {
                Ok(session) => {
                    self.iphone_preview = Some(session);
                }
                Err(error) => {
                    scrawl_log(format!("iPhone preview could not start: {error:#}"));
                    self.set_selection_mode(SelectionMode::Area);
                    return;
                }
            }
        }
        self.sync_picker_overlay();
        self.sync_picker_overlays();
        self.sync_rune_pill();
    }

    fn toggle_selection_mode(&mut self) {
        if let IslandPhase::Selecting { mode } = self.island_phase {
            let next = match mode {
                SelectionMode::Area => SelectionMode::Window,
                SelectionMode::Window | SelectionMode::Screen | SelectionMode::Iphone => {
                    SelectionMode::Area
                }
            };
            self.set_selection_mode(next);
        }
    }

    fn selection_mode(&self) -> Option<SelectionMode> {
        match self.island_phase {
            IslandPhase::Selecting { mode } => Some(mode),
            _ => None,
        }
    }

    fn choose_selection_mode(&mut self, mode: SelectionMode) {
        if self.selection_mode().is_none() {
            return;
        }
        if mode == SelectionMode::Iphone {
            self.preview_iphone();
        } else {
            self.set_selection_mode(mode);
        }
    }

    fn click_selection_mode(&mut self, mode: SelectionMode) {
        if self.selection_mode() == Some(mode) && mode.shows_record() {
            self.record_selected_mode();
        } else {
            self.choose_selection_mode(mode);
        }
    }

    fn record_selected_mode(&mut self) {
        match self.selection_mode() {
            Some(SelectionMode::Iphone) => self.start_recording_iphone(),
            Some(_) => self.start_recording_full_screen(),
            None => {}
        }
    }

    fn handle_app_key_event(&mut self, event: &KeyEvent) -> bool {
        let m = event.mods;
        if !m.cmd || m.ctrl || m.alt || m.shift {
            return false;
        }
        match &event.key {
            Key::Text(ch) if ch.eq_ignore_ascii_case("q") => {
                self.should_quit = true;
                true
            }
            Key::Text(ch) if ch == "," => {
                if self.is_selecting() {
                    self.cancel_recording();
                }
                self.open_settings(None);
                true
            }
            _ => false,
        }
    }

    fn handle_selection_key_event(&mut self, event: &KeyEvent) -> bool {
        let Some(mode) = self.selection_mode() else {
            return false;
        };
        if event.key == Key::Escape {
            self.cancel_recording();
            return true;
        }
        if self.handle_app_key_event(event) {
            return true;
        }
        let m = event.mods;
        if let Key::Text(ref ch) = event.key
            && let Some(cfg) = shortcut_from_key_and_mods(ch, m.cmd, m.ctrl, m.alt, m.shift)
            && self.telemetry.matches_shortcut(
                cfg.key_code as u16,
                m.cmd,
                m.ctrl,
                m.alt,
                m.shift,
            )
        {
            self.telemetry.emit_capture_shortcut();
            return true;
        }
        if m.cmd || m.ctrl || m.alt {
            return false;
        }
        let has_iphone = self.iphone.is_some();
        let label = self.shortcut_label();
        let record_key = crate::capture::shortcut_parts(&label).1;
        match &event.key {
            Key::Text(ch) if ch == " " => self.toggle_selection_mode(),
            Key::Enter => self.record_selected_mode(),
            Key::Text(ch) if ch.eq_ignore_ascii_case(record_key) => self.record_selected_mode(),
            Key::Tab => self.choose_selection_mode(mode.cycled(!m.shift, has_iphone)),
            Key::Right => self.choose_selection_mode(mode.cycled(true, has_iphone)),
            Key::Left => self.choose_selection_mode(mode.cycled(false, has_iphone)),
            Key::Text(ch) => match SelectionMode::from_digit(ch, has_iphone) {
                Some(next) => self.choose_selection_mode(next),
                None => return false,
            },
            _ => return false,
        }
        true
    }

    fn undo_stroke(&mut self) {
        let mut strokes = self
            .telemetry
            .strokes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(idx) = strokes.iter().rposition(|s| !s.laser) {
            strokes.remove(idx);
        }
    }

    fn request_immediate_frame(&self) {
        if let Some(ActiveRecording {
            source: CaptureSource::Screen { session, .. },
            ..
        }) = &self.active_recording
        {
            session.request_immediate_frame();
        }
    }

    fn set_island_phase(&mut self, phase: IslandPhase) {
        use crate::onboarding::Input;
        let capture = match (self.island_phase, phase) {
            (IslandPhase::Recording, IslandPhase::Recording) => None,
            (_, IslandPhase::Recording) => Some(Input::CaptureStarted),
            (IslandPhase::Recording | IslandPhase::Processing, IslandPhase::Completed { .. }) => {
                Some(Input::CaptureEnded { completed: true })
            }
            (
                IslandPhase::Recording | IslandPhase::Processing,
                IslandPhase::Hidden | IslandPhase::Dismissing,
            ) => Some(Input::CaptureEnded { completed: false }),
            _ => None,
        };
        if let Some(input) = capture
            && let Some(session) = self.onboarding.as_mut()
        {
            session.input(input);
        }
        let changed = std::mem::discriminant(&self.island_phase) != std::mem::discriminant(&phase);
        if changed {
            if let Some(rec) = &self.active_recording {
                self.last_elapsed_ms = rec.elapsed_ms();
            }
            self.reveal.close();
            if matches!(self.island_phase, IslandPhase::Hidden) {
                self.island_stepped = Instant::now();
            }
            motion::trace_event(format!("phase:{}", phase.trace_name()));
        }
        let appearing = matches!(
            self.island_phase,
            IslandPhase::Hidden | IslandPhase::Dismissing
        );
        self.island_phase = phase;
        let place = changed
            && (appearing
                || matches!(
                    phase,
                    IslandPhase::Selecting { .. } | IslandPhase::Recording
                ));
        if place {
            self.place_capsule();
        }
        self.sync_picker_overlays();
        self.sync_rune_pill();
    }

    fn follow_phone_mirror(&mut self) {
        let Some(ActiveRecording {
            source: CaptureSource::Phone(_),
            _metal_overlay: Some(overlay),
            ..
        }) = self.active_recording.as_mut()
        else {
            return;
        };
        let f = *self
            .telemetry
            .target_frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if f.x <= -10_000.0 {
            return;
        }
        let centre = [f.x + f.width / 2.0, f.y + f.height / 2.0];
        if crate::screens::contains(overlay.screen(), centre)
            || !overlay.move_to([f.x, f.y, f.width, f.height])
        {
            return;
        }
        self.display_rect = overlay.screen().map(|v| v as f32);
        self.place_pill_on(centre);
    }

    fn follow_pointer_display(&mut self, pointer: [f32; 2]) {
        if self.selection_drag.is_some() || self.selection_mode() == Some(SelectionMode::Iphone) {
            return;
        }
        let p = pointer.map(f64::from);
        let current = self.display_rect.map(f64::from);
        if crate::screens::contains(current, p) {
            return;
        }
        let screens = crate::screens::current();
        let Some(i) = crate::screens::index_at(&screens, p) else {
            return;
        };
        let frame = screens[i].frame;
        if (0..4).all(|k| (frame[k] - current[k]).abs() < 1.0) {
            return;
        }
        scrawl_log(format!("picker follows the pointer to display {frame:?}"));
        self.display_rect = frame.map(|v| v as f32);
        if let Some(ref mut overlay) = self.picker_metal_overlay {
            overlay.move_to(frame);
        }
        self.place_capsule();
        self.sync_picker_overlay();
        self.sync_picker_overlays();
    }

    fn handle_picker_down(&mut self, sx: f32, sy: f32) {
        let cur_mode = match self.island_phase {
            IslandPhase::Selecting { mode } => mode,
            _ => return,
        };
        match cur_mode {
            SelectionMode::Area => {
                self.selection_drag = Some(([sx, sy], [sx, sy]));
                self.sync_picker_overlay();
                self.sync_picker_overlays();
            }
            SelectionMode::Iphone => {
                if let Some([rx, ry, rw, rh]) =
                    self.iphone_preview.as_ref().and_then(PhoneSession::rect)
                    && sx >= rx
                    && sx <= rx + rw
                    && sy >= ry
                    && sy <= ry + rh
                {
                    self.start_recording_iphone();
                } else {
                    self.set_selection_mode(SelectionMode::Area);
                    self.selection_drag = Some(([sx, sy], [sx, sy]));
                    self.sync_picker_overlay();
                    self.sync_picker_overlays();
                }
            }
            SelectionMode::Window => {
                if self.windows_loading {
                    return;
                }
                if let Some(target) =
                    selectable_window_at(&self.selectable_windows, f64::from(sx), f64::from(sy))
                        .cloned()
                {
                    self.start_recording_with_target(target);
                } else {
                    self.start_recording_full_screen();
                }
            }
            SelectionMode::Screen => self.start_recording_full_screen(),
        }
    }

    fn handle_picker_move(&mut self, sx: f32, sy: f32) {
        self.selection_pointer = Some([sx, sy]);
        let cur_mode = match self.island_phase {
            IslandPhase::Selecting { mode } => mode,
            _ => return,
        };
        match cur_mode {
            SelectionMode::Area => {
                if let Some((start, _)) = self.selection_drag {
                    self.selection_drag = Some((start, [sx, sy]));
                    self.sync_picker_overlay();
                    self.sync_picker_overlays();
                }
            }
            SelectionMode::Iphone => {
                self.sync_picker_overlay();
                self.sync_picker_overlays();
            }
            SelectionMode::Screen => {}
            SelectionMode::Window => {
                let found = selectable_window_at(
                    &self.selectable_windows,
                    f64::from(sx),
                    f64::from(sy),
                )
                .cloned();
                if self.hovered_window.as_ref().and_then(|w| w.window_id)
                    != found.as_ref().and_then(|w| w.window_id)
                {
                    self.animated_window_rect = found.as_ref().map(|w| {
                        [
                            w.frame.x as f32,
                            w.frame.y as f32,
                            w.frame.width as f32,
                            w.frame.height as f32,
                        ]
                    });
                    self.hovered_window = found;
                    self.sync_picker_overlay();
                    self.sync_picker_overlays();
                }
            }
        }
    }

    fn handle_picker_up(&mut self, sx: f32, sy: f32) {
        if let Some((start, _)) = self.selection_drag.take() {
            if let Some([x, y, w, h]) = area_rect(start, [sx, sy], self.display_rect) {
                let target = region_target(x, y, w, h, &self.selectable_windows);
                self.start_recording_with_target(target);
            } else {
                self.sync_picker_overlay();
                self.sync_picker_overlays();
            }
        }
    }

    fn apply_pill_action(&mut self, action: PillAction) {
        match action {
            PillAction::SelectMode(mode) => self.click_selection_mode(mode),
            PillAction::SwitchRecording(mode) => self.switch_recording(mode),
            PillAction::RecordSelected => self.record_selected_mode(),
            PillAction::Cancel => self.cancel_recording(),
            PillAction::Finish => self.finish_recording(),
            PillAction::Paste(target) => self.paste_completed_bundle(target),
            PillAction::Inspect => self.inspect_completed_bundle(),
            PillAction::DismissCompleted => self.dismiss_completed_bundle(),
            PillAction::Route(index) => self.choose_route(index),
        }
    }

    fn dismiss_island(&mut self) {
        if !self.is_recording() {
            self.playback.take();
        }
        if matches!(
            self.island_phase,
            IslandPhase::Hidden | IslandPhase::Dismissing
        ) {
            return;
        }
        self.telemetry.selecting.store(false, Ordering::Release);
        self.telemetry.set_picker_rect(None);
        self.picker_metal_overlay = None;
        self.iphone_preview = None;
        self.selection_drag = None;
        self.hovered_window = None;
        self.animated_window_rect = None;
        self.set_island_phase(IslandPhase::Hidden);
        self.hand_back_keys_if_idle();
    }

    fn dismiss_completed_bundle(&mut self) {
        if let Some(dir) = self.completed_dir.clone() {
            self.keep_pending_send(&dir);
            self.mark_handled(&dir);
        }
        self.dismiss_island();
    }

    fn inspect_completed_bundle(&mut self) {
        if let Some(dir) = self.completed_dir.clone() {
            self.mark_handled(&dir);
        }
        self.set_island_phase(IslandPhase::Hidden);
        self.open_review_window();
    }

    fn paste_completed_bundle(&mut self, click_target: Option<[f32; 2]>) {
        let dir = self
            .completed_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("latest"));
        self.keep_pending_send(&dir);
        self.dismiss_island();
        if !crate::capture::accessibility_granted() {
            scrawl_log("Paste chip without Accessibility; asking for it");
            self.gate
                .defer(vec![crate::onboarding::Permission::Accessibility]);
            return;
        }
        self.mark_handled(&dir);
        std::thread::spawn(move || {
            if let Err(err) = crate::handoff::paste_bundle_at_target(&dir, click_target) {
                scrawl_log(format!("paste_bundle_at_target error: {err}"));
            }
        });
    }

    fn act_on_route(
        &mut self,
        dir: PathBuf,
        judgement: crate::routing::smart::Judgement,
        url: Option<String>,
    ) {
        use crate::routing::smart::Decision;
        let showing = self.completed_dir.as_deref() == Some(dir.as_path());
        match &judgement.decision {
            Decision::Send { to, instead } => {
                if showing {
                    self.route_chips = RouteChip::switches(instead, url.as_deref());
                }
                self.send_after_undo(
                    dir,
                    crate::routing::smart::destination_of(to, url.as_deref()),
                );
            }
            Decision::Offer { choices } => {
                if showing {
                    self.route_chips = RouteChip::asks(choices, url.as_deref());
                }
                let names: Vec<String> = choices
                    .iter()
                    .map(|c| island_title(c.candidate.label()))
                    .collect();
                if self
                    .review_bundle
                    .as_ref()
                    .is_some_and(|(shown, _)| *shown == dir)
                {
                    self.review_status = Some(format!("Copied · for {}?", names.join(" or ")));
                }
            }
            Decision::Keep { .. } => {}
        }
        self.sync_rune_pill();
    }

    fn route_chip_labels(&self) -> Vec<String> {
        self.route_chips
            .iter()
            .map(|chip| chip.label.clone())
            .collect()
    }

    fn choose_route(&mut self, index: usize) {
        let Some(dir) = self.completed_dir.clone() else {
            return;
        };
        let Some(chip) = self.route_chips.get(index).cloned() else {
            return;
        };
        self.route_chips.clear();
        self.routing = None;
        self.pending_send = None;
        self.mark_handled(&dir);
        scrawl_log(format!(
            "routing (smart): chose {} ({}) for {}",
            chip.dest.title,
            chip.dest.session,
            dir.display()
        ));
        self.send_completed(dir, chip.dest);
        self.sync_rune_pill();
    }

    fn send_after_undo(&mut self, dir: PathBuf, dest: crate::model::Destination) {
        self.pending_send = Some(dir.clone());
        if self.completed_dir.as_deref() == Some(dir.as_path()) {
            self.completed_note = Some(format!("Sending to {}…", island_title(&dest.title)));
            self.sync_rune_pill();
        }
        let tx = self.events_tx.clone();
        let app = self.app_handle.clone();
        std::thread::spawn(move || {
            std::thread::sleep(SMART_SEND_UNDO);
            let _ = tx.send(InternalEvent::UndoSendExpired { dir, dest });
            if let Some(app) = app {
                app.wake();
            }
        });
    }

    fn keep_pending_send(&mut self, dir: &std::path::Path) {
        if self.completed_dir.as_deref() == Some(dir) {
            self.route_chips.clear();
        }
        if self.routing.as_deref() == Some(dir) {
            self.routing = None;
            scrawl_log(format!(
                "routing: kept {} on the clipboard while its route was worked out",
                dir.display()
            ));
        }
        if self.pending_send.as_deref() == Some(dir) {
            self.pending_send = None;
            self.completed_note = None;
            scrawl_log(format!(
                "routing (smart): kept {} on the clipboard",
                dir.display()
            ));
        }
        self.sync_rune_pill();
    }

    fn send_completed(&mut self, dir: PathBuf, dest: crate::model::Destination) {
        let title = island_title(&dest.title);
        if self.completed_dir.as_deref() == Some(dir.as_path()) {
            self.completed_note = Some(format!("Sending to {title}…"));
        }
        if self
            .review_bundle
            .as_ref()
            .is_some_and(|(shown, _)| *shown == dir)
        {
            self.review_status = Some(format!("Sending to {title}…"));
        }
        self.sync_rune_pill();
        let tx = self.events_tx.clone();
        let app = self.app_handle.clone();
        let send_dir = dir.clone();
        let spawned = std::thread::Builder::new()
            .name("scrawl-send".into())
            .spawn(move || {
                if let Err(err) =
                    crate::handoff::load_bundle(&send_dir).and_then(|(_, mut bundle)| {
                        crate::routing::record_destination(&send_dir, &mut bundle, dest.clone())
                    })
                {
                    scrawl_log(format!("routing: couldn't record the destination: {err:#}"));
                }
                let delivery = crate::delivery::deliver(
                    &send_dir,
                    &dest.session,
                    &dest.title,
                    None,
                    crate::handoff::send_bundle_to_session,
                );
                let _ = tx.send(InternalEvent::DeliveryDone {
                    dir: send_dir,
                    title,
                    delivery,
                });
                if let Some(app) = app {
                    app.wake();
                }
            });
        if spawned.is_err() {
            self.completed_note = None;
            self.sync_rune_pill();
        }
    }

    fn step_island(&mut self) {
        let real = self.island_stepped.elapsed().as_secs_f32();
        if real < 0.0002 {
            return;
        }
        self.island_stepped = Instant::now();
        if let IslandPhase::Completed { finished_at, .. } = self.island_phase
            && finished_at.elapsed() >= Duration::from_millis(25_000)
        {
            self.set_island_phase(IslandPhase::Hidden);
            return;
        }
        if let IslandPhase::Selecting { mode } = self.island_phase {
            if let Some(pos) = current_mouse_screen() {
                self.follow_pointer_display(pos);
                self.selection_pointer = Some(pos);
                if mode == SelectionMode::Area
                    && let Some((start, _)) = self.selection_drag
                {
                    self.selection_drag = Some((start, pos));
                }
                if mode == SelectionMode::Window && !self.windows_loading {
                    self.hovered_window = selectable_window_at(
                        &self.selectable_windows,
                        f64::from(pos[0]),
                        f64::from(pos[1]),
                    )
                    .cloned();
                }
            }
            if mode == SelectionMode::Window
                && let Some(ref win) = self.hovered_window
            {
                self.animated_window_rect = Some([
                    win.frame.x as f32,
                    win.frame.y as f32,
                    win.frame.width as f32,
                    win.frame.height as f32,
                ]);
            } else {
                self.animated_window_rect = None;
            }
            self.sync_picker_overlay();
            self.sync_picker_overlays();
        }
        self.follow_phone_mirror();

        if self.reveal.is_open() {
            let activity = self.reveal_activity();
            let on_pill = self.pointer_on_pill();
            if self.reveal.tick(reveal_clock_ms(), on_pill, activity) {
                self.reveal.close();
            }
        }
        self.sync_rune_pill();
    }

    fn begin_target_selection(&mut self) {
        if self.ready_to_record() {
            self.open_picker(None);
        }
    }

    fn open_picker(&mut self, mode: Option<SelectionMode>) {
        if self.is_recording() || self.is_processing() {
            return;
        }
        scrawl_log(format!("open_picker invoked (mode: {mode:?})"));
        let started = Instant::now();
        self.recording_shortcut_key = false;
        self.selectable_windows.clear();
        self.hovered_window = None;
        self.animated_window_rect = None;
        self.selection_pointer = current_mouse_screen();
        let screens = crate::screens::current();
        if let Some(i) =
            crate::screens::pointer().and_then(|p| crate::screens::index_at(&screens, p))
        {
            self.display_rect = screens[i].frame.map(|v| v as f32);
        }
        if let Some(ref mut overlay) = self.picker_metal_overlay {
            overlay.move_to(self.display_rect.map(f64::from));
        }
        self.selection_generation += 1;
        self.windows_loading = true;
        self.refresh_iphone();
        let generation = self.selection_generation;
        let tx = self.events_tx.clone();
        let app = self.app_handle.clone();
        std::thread::spawn(move || {
            let windows = list_selectable_windows();
            let _ = tx.send(InternalEvent::WindowsLoaded {
                generation,
                windows,
            });
            if let Some(app) = app {
                app.wake();
            }
        });
        self.selection_drag = None;
        let opening = match mode.unwrap_or(self.last_mode) {
            SelectionMode::Iphone => SelectionMode::Area,
            mode => mode,
        };
        self.set_island_phase(IslandPhase::Selecting { mode: opening });
        self.open_iphone_when_found = false;
        if mode.is_none() && self.last_mode == SelectionMode::Iphone {
            if self.iphone.is_some() {
                self.preview_iphone();
            } else {
                self.open_iphone_when_found = true;
            }
        }
        self.telemetry.begin_picker_session();
        self.sync_picker_overlay();
        if self.picker_metal_overlay.is_none() {
            let [x, y, w, h] = self.display_rect.map(f64::from);
            let overlay = LaserOverlay::open(Arc::clone(&self.telemetry), Some([x, y, w, h]))
                .map_err(|error| scrawl_log(format!("Metal overlay failed: {error}")))
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
        self.close_review_window();
        super::platform::take_keys_now();
        self.sync_picker_overlays();
        self.sync_rune_pill();
        start_metric("picker_open_ms", started);
    }

    fn start_recording_full_screen(&mut self) {
        let [x, y, w, h] = self.display_rect;
        let centre = [f64::from(x + w / 2.0), f64::from(y + h / 2.0)];
        let target = crate::capture::discover_target_at(None, Some(centre));
        self.start_recording_with_target(target);
    }

    fn start_recording_with_target(&mut self, target: TargetInfo) {
        if self.is_recording() || self.is_processing() {
            return;
        }
        if !self.ready_to_record() {
            self.cancel_recording();
            return;
        }
        self.begin_playback();
        let started = Instant::now();
        scrawl_log(format!(
            "start_recording_with_target invoked: mode={:?} frame={:?}",
            target.mode, target.frame
        ));
        let quick = !self.is_selecting();
        self.telemetry.start_glow.store(
            quick && target.mode == CaptureMode::Display,
            Ordering::Release,
        );
        if let IslandPhase::Selecting { mode } = self.island_phase {
            self.remember_mode(mode);
        }
        self.recording_shortcut_key = false;
        self.iphone_preview = None;
        self.selection_drag = None;
        self.hovered_window = None;
        self.animated_window_rect = None;
        let id = crate::make_bundle_id();
        let root = std::env::var_os("SCRAWL_BENCH_START")
            .map(PathBuf::from)
            .unwrap_or_else(crate::handoff::default_captures_dir);
        let bundle_dir = root.join(format!("{id}.scrawl"));
        let _ = std::fs::create_dir_all(&bundle_dir);
        let video_path = bundle_dir.join("recording.mp4");

        self.telemetry.set_capture_mode(target.mode);
        self.telemetry.reset_for_session(target.frame);
        self.restore_excluded_windows();
        let stage = Instant::now();
        let f = target.frame;
        let target_rect = [f.x, f.y, f.width, f.height];
        self.place_pill_on([f.x + f.width / 2.0, f.y + f.height / 2.0]);
        let metal_overlay = self.picker_metal_overlay.take().map(|mut overlay| {
            overlay.move_to(target_rect);
            overlay
        });
        let metal_overlay = metal_overlay.or_else(|| {
            let ov = LaserOverlay::open(Arc::clone(&self.telemetry), Some(target_rect))
                .map_err(|error| scrawl_log(format!("Metal overlay failed: {error}")))
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
        start_metric("metal_open_ms", stage);
        let stage = Instant::now();
        let audio_recorder = AudioRecorder::start().ok();
        start_metric("audio_start_ms", stage);
        let stage = Instant::now();
        let screen_session = ScreenCaptureSession::start(
            target.clone(),
            Some(video_path),
            Arc::clone(&self.telemetry.excluded_window_ids),
        );
        start_metric("sck_start_ms", stage);
        let window_log = (target.mode == CaptureMode::Display).then(|| {
            let started_at = *self
                .telemetry
                .started_at
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            crate::windows::WindowLogger::start(
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
        });
        self.present_recording_island();
        start_metric("recording_start_ms", started);
    }

    fn present_recording_island(&mut self) {
        self.set_island_phase(IslandPhase::Recording);
        self.close_review_window();
        self.hand_back_keys_if_idle();
    }

    fn start_recording_iphone(&mut self) {
        if self.is_recording() || self.is_processing() {
            return;
        }
        scrawl_log("start_recording_iphone invoked");
        if !self.ready_for(crate::onboarding::Permission::Camera) {
            self.cancel_recording();
            return;
        }
        self.begin_playback();
        self.remember_mode(SelectionMode::Iphone);
        self.recording_shortcut_key = false;
        self.selection_drag = None;
        self.hovered_window = None;
        self.animated_window_rect = None;
        let bundle_dir = crate::handoff::default_captures_dir()
            .join(format!("{}.scrawl", crate::make_bundle_id()));
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
        let target = crate::phone::phone_target(&name, None, None, 0, 0);
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
        self.restore_excluded_windows();
        self.telemetry.active.store(false, Ordering::Release);
        let session = if let Some(mut preview) = existing_preview {
            preview.begin_recording(&movie_path);
            preview
        } else {
            let dev_id = device.as_ref().map(|d| d.id.as_str());
            match PhoneSession::start(&movie_path, dev_id, Some(Arc::clone(&self.telemetry))) {
                Ok(session) => session,
                Err(error) => {
                    scrawl_log(format!("iPhone capture could not start: {error:#}"));
                    self.telemetry.finish_session();
                    let _ = std::fs::remove_dir_all(&bundle_dir);
                    self.dismiss_island();
                    return;
                }
            }
        };
        let mirror = (initial_frame.x > -10_000.0).then_some([
            initial_frame.x,
            initial_frame.y,
            initial_frame.width,
            initial_frame.height,
        ]);
        let metal_overlay = self.picker_metal_overlay.take().or_else(|| {
            LaserOverlay::open(Arc::clone(&self.telemetry), mirror)
                .map_err(|error| scrawl_log(format!("Metal overlay failed: {error}")))
                .ok()
        });
        self.active_recording = Some(ActiveRecording {
            started: Instant::now(),
            target,
            bundle_dir,
            source: CaptureSource::Phone(session),
            _metal_overlay: metal_overlay,
        });
        self.present_recording_island();
    }

    fn phone_ended(&mut self) {
        if self.iphone_preview.is_some() {
            self.iphone_preview = None;
            self.set_selection_mode(SelectionMode::Area);
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
            self.finish_recording();
        } else {
            self.cancel_recording();
        }
    }

    fn cancel_recording(&mut self) {
        self.playback.take();
        scrawl_log("cancel_recording invoked");
        self.discard_recording();
        self.dismiss_island();
    }

    fn finish_recording(&mut self) {
        self.playback.take();
        scrawl_log("finish_recording invoked");
        let Some(rec) = self.active_recording.take() else {
            return;
        };
        let (events, strokes) = self.telemetry.finish_session();
        let bundle_dir = rec.bundle_dir;
        let target = rec.target;
        let source = rec.source;
        drop(rec._metal_overlay);
        self.set_island_phase(IslandPhase::Processing);
        self.completed_note = None;
        self.route_chips.clear();
        self.sync_rune_pill();

        let smart = crate::handoff::experiment(crate::routing::SMART_ROUTING);
        let screen = matches!(source, CaptureSource::Screen { .. });
        let front = screen
            .then(|| {
                (target.mode != CaptureMode::Window)
                    .then(crate::capture::frontmost_app_info)
                    .flatten()
                    .and_then(|(_, _, id)| id)
            })
            .flatten();
        let captured = target.bundle_id.clone();
        let finish_started = Instant::now();
        let route = (screen && !smart).then(|| {
            let (front, captured) = (front.clone(), captured.clone());
            std::thread::spawn(move || {
                crate::routing::route_capture(front.as_deref(), captured.as_deref())
            })
        });
        let look = (screen && smart).then(|| {
            std::thread::spawn(move || {
                crate::routing::look_capture(front.as_deref(), captured.as_deref())
            })
        });

        let tx = self.events_tx.clone();
        let app = self.app_handle.clone();
        std::thread::spawn(move || {
            let (finalized, duration_ms) = match source {
                CaptureSource::Screen {
                    session: screen_session,
                    audio: audio_recorder,
                    windows: window_log,
                } => {
                    let stop_started = Instant::now();
                    let (recording, recorded_audio, mut activity) = std::thread::scope(|s| {
                        let screen_handle = s.spawn(|| screen_session.stop());
                        let win_handle = s.spawn(|| {
                            window_log
                                .map(crate::windows::WindowLogger::stop)
                                .unwrap_or_default()
                        });
                        let audio = audio_recorder.and_then(|r| r.stop().ok());
                        (
                            screen_handle.join().unwrap(),
                            audio,
                            win_handle.join().unwrap_or_default(),
                        )
                    });
                    activity.stop_started = Some(stop_started);
                    activity.stop_ms = Some(stop_started.elapsed().as_millis() as u64);
                    let duration_ms = recording.duration_ms;
                    let finalized = crate::finalize_capture_bundle(
                        &bundle_dir,
                        target,
                        recording,
                        strokes,
                        events,
                        recorded_audio,
                        None,
                        activity,
                    );
                    (finalized, duration_ms)
                }
                CaptureSource::Phone(session) => {
                    let finalized = session.stop().and_then(|recording| {
                        crate::phone::finalize_live_capture(
                            &bundle_dir,
                            recording,
                            events,
                            strokes,
                            None,
                        )
                    });
                    if let Err(error) = &finalized {
                        scrawl_log(format!("iPhone capture failed: {error:#}"));
                    }
                    let duration_ms = finalized.as_ref().map_or(0, |(_, b, _)| b.duration_ms);
                    (finalized, duration_ms)
                }
                CaptureSource::Lab => (Err(anyhow::anyhow!("a lab recording has no bundle")), 0),
            };
            let _ = tx.send(InternalEvent::CaptureFinalized {
                finalized,
                duration_ms,
                finish_started,
                route,
                look,
            });
            if let Some(app) = app {
                app.wake();
            }
        });
    }

    fn route_arrived(&mut self, dir: PathBuf, route: RouteOutcome) {
        if self.routing.as_deref() != Some(dir.as_path()) {
            if !matches!(route, RouteOutcome::None) {
                scrawl_log(format!(
                    "routing: {} was kept on the clipboard before its route arrived",
                    dir.display()
                ));
            }
            return;
        }
        self.routing = None;
        match route {
            RouteOutcome::None => {}
            RouteOutcome::Send(dest) => self.send_completed(dir, dest),
            RouteOutcome::Judged(judgement, url) => self.act_on_route(dir, judgement, url),
        }
    }

    fn poll_tick(&mut self) -> bool {
        let mut review_dirty = false;
        if !self.status_bar_installed {
            self.status_bar_installed = true;
            self.menu_rx = super::platform::install_status_bar();
            super::platform::set_status_inbox_count(self.inbox.len());
            if self.review_open {
                super::platform::take_keys_now();
            }
            if std::mem::take(&mut self.pending_start_record) {
                self.record_or_finish();
            }
            if std::mem::take(&mut self.pending_start_iphone) {
                self.start_recording_iphone();
            }
        }

        let status = super::EXPORT_STATUS.lock().ok().and_then(|s| s.clone());
        if status != self.shown_export_status {
            match &status {
                Some(text) => super::platform::set_status_text(text),
                None => super::platform::set_status_inbox_count(self.inbox.len()),
            }
            self.shown_export_status = status;
        }
        super::platform::poll_status_bar();

        let menu_actions: Vec<MenuAction> = self
            .menu_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for action in menu_actions {
            match action {
                MenuAction::ToggleCapture => self.record_or_finish(),
                MenuAction::ChooseTarget => {
                    if !self.is_recording() && !self.is_processing() && !self.is_selecting() {
                        self.begin_target_selection();
                    }
                }
                MenuAction::OpenLatest => {
                    if let Ok((dir, bundle)) =
                        crate::handoff::load_bundle(std::path::Path::new("latest"))
                    {
                        self.mark_handled(&dir);
                        self.set_review_bundle((dir, bundle), None);
                        self.open_review_window();
                        review_dirty = true;
                    }
                }
                MenuAction::OpenInbox(path) => {
                    let loaded = crate::handoff::load_bundle(&path).ok();
                    let opened = loaded.is_some();
                    self.mark_handled(&path);
                    if let Some(bundle) = loaded {
                        self.set_review_bundle(bundle, None);
                        review_dirty = true;
                    }
                    if opened {
                        self.open_review_window();
                    }
                }
                MenuAction::ExportVideo => start_video_export(),
                MenuAction::Settings => self.open_settings(None),
                MenuAction::CheckForUpdates => super::platform::check_for_updates(),
                MenuAction::Quit => self.should_quit = true,
            }
        }

        let ui_events: Vec<LiveOverlayEvent> = self
            .ui_receiver
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for ev in ui_events {
            match ev {
                LiveOverlayEvent::CaptureShortcut => self.press_shortcut(),
                LiveOverlayEvent::PasteLatestShortcut => {
                    if !self.is_recording() && !self.is_selecting() {
                        self.paste_latest();
                    }
                }
                LiveOverlayEvent::UndoStroke => self.undo_stroke(),
                LiveOverlayEvent::RequestSnapshot => self.request_immediate_frame(),
                LiveOverlayEvent::PhoneEnded => self.phone_ended(),
                LiveOverlayEvent::Onboarding(event) => self.onboarding_event(event),
                LiveOverlayEvent::Settings(event) => self.settings_event(event),
                LiveOverlayEvent::IngestedBundle {
                    dir,
                    bundle,
                    status,
                } => {
                    let frame_count = bundle.keyframes.len();
                    let duration_ms = bundle.duration_ms;
                    let inbox_count = self.add_to_inbox(&dir, &bundle);
                    self.completed_dir = Some(dir.clone());
                    self.route_chips.clear();
                    self.set_review_bundle((dir, *bundle), status);
                    self.set_island_phase(IslandPhase::Completed {
                        finished_at: Instant::now(),
                        frame_count,
                        duration_ms,
                        inbox_count,
                    });
                    review_dirty = true;
                }
            }
        }

        let internal: Vec<InternalEvent> = self.events_rx.try_iter().collect();
        for ev in internal {
            match ev {
                InternalEvent::PillAction(action) => self.apply_pill_action(action),
                InternalEvent::PickerDown { x, y, .. } => self.handle_picker_down(x, y),
                InternalEvent::PickerMove { x, y, .. } => self.handle_picker_move(x, y),
                InternalEvent::PickerUp { x, y, .. } => self.handle_picker_up(x, y),
                InternalEvent::PickerKey(key) => {
                    let _ = self.handle_selection_key_event(&key);
                }
                InternalEvent::WindowsLoaded {
                    generation,
                    windows,
                } => {
                    if self.selection_generation == generation && self.is_selecting() {
                        self.windows_loading = false;
                        let ptr = current_mouse_screen().or(self.selection_pointer);
                        self.selection_pointer = ptr;
                        self.hovered_window = ptr
                            .and_then(|[x, y]| {
                                selectable_window_at(&windows, f64::from(x), f64::from(y))
                            })
                            .cloned();
                        if matches!(
                            self.island_phase,
                            IslandPhase::Selecting {
                                mode: SelectionMode::Window
                            }
                        ) {
                            self.animated_window_rect = self.hovered_window.as_ref().map(|w| {
                                [
                                    w.frame.x as f32,
                                    w.frame.y as f32,
                                    w.frame.width as f32,
                                    w.frame.height as f32,
                                ]
                            });
                        }
                        self.selectable_windows = windows;
                        self.sync_picker_overlay();
                        self.sync_picker_overlays();
                    }
                }
                InternalEvent::IphoneFound(found) => {
                    if self.iphone != found {
                        self.iphone = found;
                        self.sync_picker_overlays();
                        self.sync_rune_pill();
                    }
                    if std::mem::take(&mut self.open_iphone_when_found)
                        && self.iphone.is_some()
                        && matches!(
                            self.island_phase,
                            IslandPhase::Selecting {
                                mode: SelectionMode::Area | SelectionMode::Iphone
                            }
                        )
                    {
                        self.preview_iphone();
                    }
                }
                InternalEvent::CaptureFinalized {
                    finalized,
                    duration_ms,
                    finish_started,
                    route,
                    look,
                } => {
                    let mut frame_count = 1;
                    let mut routed = None;
                    self.completed_dir = None;
                    if let Ok((dir, bundle, _)) = finalized {
                        frame_count = bundle.keyframes.len();
                        let _ = crate::handoff::copy_bundle_to_clipboard(&dir);
                        self.completed_dir = Some(dir.clone());
                        self.last_local_capture_ms = crate::now_epoch_ms();
                        if route.is_some() || look.is_some() {
                            self.routing = Some(dir.clone());
                            routed = Some((dir.clone(), look.is_some().then(|| bundle.clone())));
                        }
                        self.set_review_bundle((dir, bundle), self.completed_note.clone());
                        review_dirty = true;
                    }
                    self.set_island_phase(IslandPhase::Completed {
                        finished_at: Instant::now(),
                        frame_count,
                        duration_ms,
                        inbox_count: 0,
                    });
                    let copied_ms = finish_started.elapsed().as_millis();
                    if let Some((dir, bundle)) = routed {
                        let tx = self.events_tx.clone();
                        let app = self.app_handle.clone();
                        let route_dir = dir.clone();
                        std::thread::spawn(move || {
                            let outcome = if let Some(dest) =
                                route.and_then(|r| r.join().ok().flatten())
                            {
                                RouteOutcome::Send(dest)
                            } else if let (Some(bundle), Some(live)) =
                                (bundle, look.and_then(|l| l.join().ok().flatten()))
                            {
                                let (judgement, url) =
                                    crate::routing::judge_capture(&route_dir, &bundle, live);
                                RouteOutcome::Judged(judgement, url)
                            } else {
                                RouteOutcome::None
                            };
                            let _ = tx.send(InternalEvent::RouteResolved {
                                dir,
                                route: outcome,
                                copied_ms,
                                finish_started,
                            });
                            if let Some(app) = app {
                                app.wake();
                            }
                        });
                    }
                }
                InternalEvent::RouteResolved {
                    dir,
                    route,
                    copied_ms,
                    finish_started,
                } => {
                    scrawl_log(format!(
                        "finish: copied at {copied_ms}ms · routed at {}ms",
                        finish_started.elapsed().as_millis()
                    ));
                    self.route_arrived(dir, route);
                    review_dirty = true;
                }
                InternalEvent::UndoSendExpired { dir, dest } => {
                    if self.pending_send.as_deref() == Some(dir.as_path()) {
                        self.pending_send = None;
                        self.route_chips.clear();
                        self.send_completed(dir, dest);
                        review_dirty = true;
                    }
                }
                InternalEvent::DeliveryDone {
                    dir,
                    title,
                    delivery,
                } => {
                    if self.completed_dir.as_deref() == Some(dir.as_path()) {
                        self.completed_note = Some(completed_delivery_note(&delivery, &title));
                    }
                    if self
                        .review_bundle
                        .as_ref()
                        .is_some_and(|(shown, _)| *shown == dir)
                    {
                        self.review_status = Some(delivery.status("Copied to clipboard"));
                        review_dirty = true;
                    }
                    self.sync_rune_pill();
                }
            }
        }

        motion::poll_tune();
        if self.shortcut.is_pending() {
            self.tick_shortcut();
        } else if self.telemetry.take_escape() && self.is_selecting() {
            self.cancel_recording();
        }
        if self.gate.is_pending() {
            self.tick_gate();
        }
        if self.is_island_visible() {
            record_external_frontmost_app();
            self.step_island();
        }
        review_dirty
    }
}

// ─── Review Window (`ReviewState`) ───────────────────────────────────────────────────────────

struct ReviewState {
    coord: RuneCoordinator,
    images: Option<ImageCache>,
    cached_sheet_path: Option<PathBuf>,
    cached_sheet_image: Option<Image>,
    cached_frames: Vec<Option<Image>>,
    headless: bool,
}

impl ReviewState {
    fn new(coord: RuneCoordinator, headless: bool) -> Self {
        Self {
            coord,
            images: None,
            cached_sheet_path: None,
            cached_sheet_image: None,
            cached_frames: Vec::new(),
            headless,
        }
    }

    fn ensure_images(&mut self) {
        let Some(ref cache) = self.images else {
            return;
        };
        let Some((ref dir, ref bundle)) = self.coord.review_bundle else {
            self.cached_sheet_path = None;
            self.cached_sheet_image = None;
            self.cached_frames.clear();
            return;
        };
        let sheet_path = dir.join(&bundle.contact_sheet);
        if self.cached_sheet_path.as_ref() != Some(&sheet_path) {
            self.cached_sheet_path = Some(sheet_path.clone());
            self.cached_sheet_image = std::fs::read(&sheet_path).ok().map(|bytes| {
                let img = cache.load(bytes);
                if self.headless {
                    let _ = img.wait();
                }
                img
            });
            self.cached_frames = bundle
                .keyframes
                .iter()
                .map(|kf| {
                    let bytes = std::fs::read(dir.join(&kf.file)).ok()?;
                    let img = cache.load(bytes);
                    if self.headless {
                        let _ = img.wait();
                    }
                    Some(img)
                })
                .collect();
        }
    }
}

fn review_setup(s: &mut ReviewState, waker: Waker) {
    s.images = Some(ImageCache::new(waker));
    s.ensure_images();
}

fn review_poll(s: &mut ReviewState, _now: f64, tx: &mut Tx) -> bool {
    if s.headless {
        s.ensure_images();
        return false;
    }
    let dirty = s.coord.poll_tick();
    s.ensure_images();
    if s.coord.should_quit {
        tx.quit();
        return true;
    }
    dirty
}

fn on_review_close(s: &mut ReviewState, _tx: &mut Tx) {
    s.coord.review_open = false;
    s.coord.hand_back_keys_if_idle();
}

fn review_on_key(s: &mut ReviewState, event: &KeyEvent, tx: &mut Tx) -> bool {
    if s.coord.recording_shortcut_key {
        if event.key == Key::Escape {
            s.coord.recording_shortcut_key = false;
            return true;
        }
        let m = event.mods;
        if let Key::Text(ref ch) = event.key
            && let Some(cfg) = shortcut_from_key_and_mods(ch, m.cmd, m.ctrl, m.alt, m.shift)
        {
            update_global_shortcut(&s.coord.telemetry, cfg);
            s.coord.recording_shortcut_key = false;
            return true;
        }
    }
    if s.coord.handle_app_key_event(event) {
        if s.coord.should_quit {
            tx.quit();
        }
        return true;
    }
    let m = event.mods;
    if let Key::Text(ref ch) = event.key
        && let Some(cfg) = shortcut_from_key_and_mods(ch, m.cmd, m.ctrl, m.alt, m.shift)
        && s.coord
            .telemetry
            .matches_shortcut(cfg.key_code as u16, m.cmd, m.ctrl, m.alt, m.shift)
    {
        s.coord.telemetry.emit_capture_shortcut();
        return true;
    }
    let is_close = event.key == Key::Escape
        || (m.cmd
            && !m.ctrl
            && !m.alt
            && !m.shift
            && matches!(&event.key, Key::Text(ch) if ch.eq_ignore_ascii_case("w")));
    if is_close {
        tx.close_window();
        return true;
    }
    false
}

fn tab_chip(
    key: &'static str,
    label: &'static str,
    selected: bool,
    on_press: impl Fn(&mut ReviewState) + 'static,
) -> El {
    let mut chip = row()
        .key(key)
        .h(24.0)
        .px(10.0)
        .radius(6.0)
        .center_y()
        .cursor(if selected {
            Cursor::Default
        } else {
            Cursor::Pointer
        })
        .on_press(on_press);
    if selected {
        chip = chip
            .fill(rgb_color(SURFACE_SELECTED))
            .shadow(1.0, 2.0, Color::BLACK.alpha(0.25));
    } else {
        chip = chip.hover(Color::WHITE.alpha(0.05));
    }
    chip.child(
        text(label)
            .font(TextStyle::new(12.0, 500.0))
            .color(rgb_color(if selected { TEXT_COLOR } else { MUTED_COLOR })),
    )
}

fn action_button(
    key: &'static str,
    label: &str,
    hint: Option<&str>,
    primary: bool,
    on_press: impl Fn(&mut ReviewState) + 'static,
) -> El {
    let mut btn = row()
        .key(key)
        .h(28.0)
        .px(12.0)
        .radius(7.0)
        .center_y()
        .gap(6.0)
        .cursor(Cursor::Pointer)
        .border(
            1.0,
            rgb_color(if primary { ACCENT_COLOR } else { LINE_COLOR }),
        )
        .fill(rgb_color(if primary { ACCENT_COLOR } else { SURFACE_BG }))
        .hover(rgb_color(if primary {
            ACCENT_HOVER
        } else {
            SURFACE_HOVER
        }))
        .on_press(on_press)
        .child(
            text(label)
                .font(TextStyle::new(12.5, 500.0))
                .color(rgb_color(TEXT_COLOR)),
        );
    if let Some(hint) = hint {
        btn = btn.child(text(hint).font(TextStyle::new(12.5, 500.0)).color(if primary {
            rgba_color(0xffffffb3)
        } else {
            rgb_color(MUTED_COLOR)
        }));
    }
    btn
}

fn sidebar_heading(label: &'static str) -> El {
    el().mb(8.0).child(
        text(label)
            .font(TextStyle::new(11.0, 600.0))
            .color(rgb_color(MUTED_COLOR)),
    )
}

fn timeline_row_el(index: usize, line: &str) -> El {
    let (time, body, is_mark) = split_timeline_line(line);
    let mut r = row()
        .key(format!("timeline-row-{index}"))
        .w_full()
        .min_w(0.0)
        .px(8.0)
        .py(6.0)
        .radius(6.0)
        .items_start()
        .gap(10.0)
        .hover(rgb_color(SURFACE_BG))
        .child(
            el().w(32.0).child(
                text(time)
                    .font(TextStyle::new(12.5, 400.0))
                    .color(rgb_color(MUTED_COLOR)),
            ),
        );
    if is_mark {
        r = r.child(
            text("✎")
                .font(TextStyle::new(12.5, 500.0))
                .color(rgb_color(INK_RED)),
        );
    }
    r.child(
        el().grow(1.0).min_w(0.0).child(
            text(body)
                .font(TextStyle::new(12.5, 400.0))
                .color(rgb_color(if is_mark { TEXT_COLOR } else { TEXT_SOFT }))
                .w_full()
                .wrap(),
        ),
    )
}

fn review_view(s: &ReviewState, cx: &mut Cx<ReviewState>) -> El {
    let stage_w = cx
        .layout("review-stage-scroll")
        .map_or(1120.0 - 320.0 - 44.0, |r| (r[2] - 44.0).max(320.0));
    let shortcut_label = s.coord.shortcut_label();

    let Some((ref dir, ref bundle)) = s.coord.review_bundle else {
        return col()
            .key("scrawl-review-empty")
            .w_full()
            .h_full()
            .center()
            .gap(12.0)
            .fill(rgb_color(CANVAS_BG))
            .child(
                text("Scrawl")
                    .font(TextStyle::new(16.0, 600.0))
                    .color(rgb_color(TEXT_COLOR)),
            )
            .child(
                row()
                    .gap(8.0)
                    .child(action_button(
                        "empty-record",
                        "Record",
                        Some(&shortcut_label),
                        true,
                        |state| state.coord.record_or_finish(),
                    ))
                    .child(action_button(
                        "empty-demo",
                        "Load Demo Bundle",
                        None,
                        false,
                        |state| {
                            if let Ok((dir, bundle, _)) = crate::generate_demo_bundle(None) {
                                state.coord.set_review_bundle(
                                    (dir, bundle),
                                    Some("Generated demo bundle".into()),
                                );
                                state.ensure_images();
                            }
                        },
                    )),
            );
    };

    let target_label = match (&bundle.target.app, &bundle.target.window_title) {
        (Some(app), Some(win)) if !win.is_empty() && win != app => {
            format!("{app} — “{win}”")
        }
        (Some(app), _) => app.clone(),
        _ => format!("Display {}", bundle.target.display_id),
    };
    let timeline_lines = timeline(bundle);
    let tab = s.coord.selected_tab;
    let selected_kf = s.coord.selected_keyframe;
    let recording_shortcut = s.coord.recording_shortcut_key;
    let status = s.coord.review_status.as_deref();
    let copy_dir = dir.clone();
    let reveal_dir = dir.clone();

    let stage_content = match tab {
        ReviewTab::ContactSheet => {
            if let Some(ref img) = s.cached_sheet_image {
                let [iw, ih] = img.size().unwrap_or([1280, 800]);
                let aspect = (ih as f32 / iw.max(1) as f32).clamp(0.25, 3.0);
                let img_h = (stage_w * aspect).round();
                row().w_full().justify_center().child(
                    el().key("review-contact-sheet-img")
                        .size(stage_w, img_h)
                        .radius(10.0)
                        .border(1.0, rgb_color(LINE_COLOR))
                        .pixels(img.pixels()),
                )
            } else {
                el().padding(24.0).child(
                    text("Contact sheet image unavailable.")
                        .font(TextStyle::new(13.0, 400.0))
                        .color(rgb_color(MUTED_COLOR)),
                )
            }
        }
        ReviewTab::Keyframes => {
            let card_img_w = (stage_w - 18.0).max(240.0);
            let cards = bundle.keyframes.iter().enumerate().map(|(idx, kf)| {
                let is_selected = idx == selected_kf;
                let frame_img = s.cached_frames.get(idx).and_then(Clone::clone);
                let mut card = col()
                    .key(format!("kf-card-{idx}"))
                    .padding(8.0)
                    .radius(12.0)
                    .fill(rgb_color(SURFACE_BG))
                    .border(
                        1.0,
                        rgb_color(if is_selected {
                            ACCENT_COLOR
                        } else {
                            LINE_COLOR
                        }),
                    )
                    .gap(8.0)
                    .cursor(Cursor::Pointer)
                    .on_press(move |state: &mut ReviewState| {
                        state.coord.selected_keyframe = idx;
                    });
                if let Some(img) = frame_img {
                    let [iw, ih] = img.size().unwrap_or([1280, 800]);
                    let aspect = (ih as f32 / iw.max(1) as f32).clamp(0.25, 3.0);
                    let img_h = (card_img_w * aspect).round();
                    card = card.child(
                        el().size(card_img_w, img_h)
                            .radius(3.0)
                            .pixels(img.pixels()),
                    );
                }
                card = card.child(
                    row()
                        .px(4.0)
                        .pb(2.0)
                        .center_y()
                        .gap(8.0)
                        .child(
                            row()
                                .h(20.0)
                                .px(7.0)
                                .center_y()
                                .radius(10.0)
                                .fill(rgb_color(CANVAS_BG))
                                .child(
                                    text(&format!("#{} · {}", kf.index, format_clock(kf.t)))
                                        .font(TextStyle::new(11.5, 600.0))
                                        .color(rgb_color(TEXT_COLOR)),
                                ),
                        )
                        .child(
                            text(&kf.event)
                                .font(TextStyle::new(12.5, 400.0))
                                .color(rgb_color(TEXT_SOFT))
                                .truncate(),
                        ),
                );
                if let Some(ref said) = kf.said {
                    card = card.child(
                        el().w_full().px(4.0).pb(2.0).child(
                            text(&format!("“{said}”"))
                                .font(TextStyle::new(12.0, 400.0))
                                .color(rgb_color(MUTED_COLOR))
                                .w_full()
                                .wrap(),
                        ),
                    );
                }
                card
            });
            col().w_full().gap(16.0).children(cards)
        }
    };

    let transcript_str = bundle.transcript.text.trim();
    let transcript_box = el()
        .w_full()
        .min_w(0.0)
        .padding(12.0)
        .radius(8.0)
        .fill(rgb_color(SURFACE_BG))
        .border(1.0, rgb_color(LINE_COLOR))
        .child(if transcript_str.is_empty() {
            text("No speech")
                .font(TextStyle::new(13.0, 400.0))
                .color(rgb_color(MUTED_COLOR))
        } else {
            text(&format!("“{transcript_str}”"))
                .font(TextStyle::new(13.0, 400.0))
                .color(rgb_color(TEXT_COLOR))
                .w_full()
                .wrap()
        });

    col()
        .key("scrawl-review-root")
        .w_full()
        .h_full()
        .fill(rgb_color(CANVAS_BG))
        .child(
            row()
                .key("review-header")
                .w_full()
                .h(48.0)
                .pl(super::platform::HEADER_LEAD)
                .pr(super::platform::HEADER_TRAIL)
                .center_y()
                .justify_between()
                .gap(16.0)
                .fill(rgb_color(SIDEBAR_BG))
                .window_drag()
                .child(
                    row()
                        .grow(1.0)
                        .min_w(0.0)
                        .center_y()
                        .gap(12.0)
                        .child(
                            row()
                                .h(28.0)
                                .padding(1.0)
                                .center_y()
                                .gap(2.0)
                                .radius(8.0)
                                .fill(rgb_color(CANVAS_BG))
                                .border(1.0, rgb_color(LINE_COLOR))
                                .child(tab_chip(
                                    "tab-contact-sheet",
                                    "Contact Sheet",
                                    tab == ReviewTab::ContactSheet,
                                    |state| state.coord.selected_tab = ReviewTab::ContactSheet,
                                ))
                                .child(tab_chip(
                                    "tab-keyframes",
                                    "Keyframes",
                                    tab == ReviewTab::Keyframes,
                                    |state| state.coord.selected_tab = ReviewTab::Keyframes,
                                )),
                        )
                        .child(
                            text(&format!(
                                "{} · {} · {} {}",
                                target_label,
                                format_clock(bundle.duration_ms),
                                bundle.keyframes.len(),
                                if bundle.keyframes.len() == 1 {
                                    "keyframe"
                                } else {
                                    "keyframes"
                                }
                            ))
                            .font(TextStyle::new(12.0, 400.0))
                            .color(rgb_color(MUTED_COLOR))
                            .truncate(),
                        ),
                )
                .child(
                    row()
                        .center_y()
                        .gap(8.0)
                        .child(action_button(
                            "btn-shortcut",
                            if recording_shortcut {
                                "Press shortcut…"
                            } else {
                                "Shortcut"
                            },
                            (!recording_shortcut).then_some(shortcut_label.as_str()),
                            false,
                            |state| {
                                state.coord.recording_shortcut_key =
                                    !state.coord.recording_shortcut_key;
                            },
                        ))
                        .child(action_button(
                            "btn-reveal",
                            &format!("Show in {}", super::platform::FILE_MANAGER),
                            None,
                            false,
                            move |state| {
                                let res = crate::handoff::open_path(&reveal_dir);
                                let manager = super::platform::FILE_MANAGER;
                                state.coord.review_status = Some(match res {
                                    Ok(()) => format!("Opened in {manager}"),
                                    Err(e) => format!("Show in {manager} failed: {e}"),
                                });
                            },
                        ))
                        .child(action_button(
                            "btn-copy",
                            if status == Some("Copied ✓") {
                                "Copied"
                            } else {
                                "Copy"
                            },
                            None,
                            true,
                            move |state| {
                                let res = crate::handoff::copy_bundle_to_clipboard(&copy_dir);
                                if res.is_ok() {
                                    state.coord.mark_handled(&copy_dir);
                                }
                                state.coord.review_status = Some(match res {
                                    Ok(()) => "Copied ✓".into(),
                                    Err(e) => format!("Copy failed: {e}"),
                                });
                            },
                        )),
                ),
        )
        .child(el().w_full().h(1.0).fill(rgb_color(LINE_COLOR)))
        .child(
            row()
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .child(
                    col()
                        .key("review-stage-scroll")
                        .grow(1.0)
                        .min_w(0.0)
                        .min_h(0.0)
                        .h_full()
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(col().w_full().padding(20.0).child(stage_content)),
                )
                .child(el().w(1.0).h_full().fill(rgb_color(LINE_COLOR)))
                .child(
                    col()
                        .key("review-timeline-sidebar")
                        .w(320.0)
                        .min_h(0.0)
                        .h_full()
                        .fill(rgb_color(SIDEBAR_BG))
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(
                            col()
                                .w_full()
                                .px(16.0)
                                .py(18.0)
                                .child(sidebar_heading("Transcript"))
                                .child(transcript_box)
                                .child(el().mt(20.0).child(sidebar_heading("Timeline")))
                                .child(
                                    col().w_full().min_w(0.0).gap(1.0).children(
                                        timeline_lines
                                            .into_iter()
                                            .enumerate()
                                            .map(|(i, line)| timeline_row_el(i, &line)),
                                    ),
                                ),
                        ),
                ),
        )
}

// ─── Headless Scenes & Runner ────────────────────────────────────────────────────────────────

fn sample_windows() -> Vec<TargetInfo> {
    vec![
        TargetInfo {
            mode: CaptureMode::Window,
            app: Some("OpenCode".into()),
            bundle_id: Some("com.apple.Finder".into()),
            window_title: Some("scrawl — rune/app".into()),
            window_id: Some(4210),
            display_id: 1,
            frame: TargetFrame {
                x: 180.0,
                y: 110.0,
                width: 960.0,
                height: 620.0,
                scale: 2.0,
            },
        },
        TargetInfo {
            mode: CaptureMode::Window,
            app: Some("Terminal".into()),
            bundle_id: Some("com.apple.Terminal".into()),
            window_title: Some("zsh — 120×40".into()),
            window_id: Some(4211),
            display_id: 1,
            frame: TargetFrame {
                x: 420.0,
                y: 260.0,
                width: 760.0,
                height: 480.0,
                scale: 2.0,
            },
        },
    ]
}

fn picker_scene_area(s: &mut PickerState) {
    s.snapshot.active = true;
    s.snapshot.mode = SelectionMode::Area;
    s.snapshot.pointer = Some([860.0, 520.0]);
    s.snapshot.drag = Some(([260.0, 180.0], [860.0, 520.0]));
    s.snapshot.selectable_windows = sample_windows();
}

fn picker_scene_window(s: &mut PickerState) {
    let windows = sample_windows();
    let hovered = windows.first().cloned();
    s.snapshot.active = true;
    s.snapshot.mode = SelectionMode::Window;
    s.snapshot.pointer = Some([540.0, 360.0]);
    s.snapshot.animated_window_rect = hovered.as_ref().map(|w| {
        [
            w.frame.x as f32,
            w.frame.y as f32,
            w.frame.width as f32,
            w.frame.height as f32,
        ]
    });
    s.snapshot.app_bundle_path = hovered
        .as_ref()
        .and_then(|w| w.bundle_id.as_deref())
        .and_then(super::platform::app_bundle_path);
    s.snapshot.hovered_window = hovered;
    s.snapshot.selectable_windows = windows;
}

fn picker_scene_screen(s: &mut PickerState) {
    s.snapshot.active = true;
    s.snapshot.mode = SelectionMode::Screen;
    s.snapshot.pointer = Some([720.0, 450.0]);
    s.snapshot.selectable_windows = sample_windows();
}

const PICKER_SCENES: &[(&str, fn(&mut PickerState))] = &[
    ("picker-area", picker_scene_area),
    ("picker-window", picker_scene_window),
    ("picker-screen", picker_scene_screen),
];

fn ensure_demo_bundle_in_review(s: &mut ReviewState) {
    if s.coord.review_bundle.is_none() {
        let temp_dir = std::env::temp_dir().join(format!("{}.scrawl", crate::make_bundle_id()));
        if let Ok((dir, bundle, _)) = crate::generate_demo_bundle(Some(temp_dir)) {
            s.coord.set_review_bundle((dir, bundle), None);
        }
    }
    s.ensure_images();
}

fn review_scene_empty(s: &mut ReviewState) {
    s.coord.review_bundle = None;
    s.ensure_images();
}

fn review_scene_contact_sheet(s: &mut ReviewState) {
    ensure_demo_bundle_in_review(s);
    s.coord.selected_tab = ReviewTab::ContactSheet;
}

fn review_scene_keyframes(s: &mut ReviewState) {
    ensure_demo_bundle_in_review(s);
    s.coord.selected_tab = ReviewTab::Keyframes;
}

const REVIEW_SCENES: &[(&str, fn(&mut ReviewState))] = &[
    ("review-empty", review_scene_empty),
    ("review-contact-sheet", review_scene_contact_sheet),
    ("review-keyframes", review_scene_keyframes),
];

fn dummy_review_init() -> ReviewState {
    let (ui_tx, _ui_rx) = flume::unbounded();
    let (ev_tx, ev_rx) = flume::unbounded();
    let telemetry = SharedCaptureTelemetry::new(ui_tx);
    let coord = RuneCoordinator::new(
        telemetry,
        None,
        ev_tx,
        ev_rx,
        None,
        false,
        false,
        None,
        None,
        Arc::default(),
        Vec::new(),
        Arc::default(),
    );
    ReviewState::new(coord, true)
}

fn dummy_picker_init() -> PickerState {
    PickerState::new([0.0, 0.0, 1440.0, 900.0], None, None, true)
}

fn run_headless(
    initial_bundle: Option<(PathBuf, ScrawlBundle)>,
    options: RuneAppOptions,
    fonts: &'static [&'static [u8]],
) {
    if let Some(ref snap) = options.snap
        && snap.extension().is_none_or(|e| e != "png")
        && options.scene.is_none()
    {
        let _ = std::fs::create_dir_all(snap);
        if let Ok(exe) = std::env::current_exe() {
            for &scene in ALL_SCENES {
                let out = snap.join(format!("{scene}.png"));
                let _ = std::process::Command::new(&exe)
                    .args(["app", "--scene", scene, "--snap"])
                    .arg(&out)
                    .status();
            }
        }
        return;
    }

    let scene = options
        .scene
        .clone()
        .unwrap_or_else(|| "review-contact-sheet".to_owned());
    let mut args = Vec::new();
    if let Some(ref snap) = options.snap {
        args.push("--snap".to_owned());
        args.push(snap.to_string_lossy().into_owned());
    }
    args.push("--scene".to_owned());
    args.push(scene.clone());
    if let Some(ref script) = options.script {
        args.push("--script".to_owned());
        args.push(script.to_string_lossy().into_owned());
    }

    let families = super::rune_pill::families(fonts);
    if scene.starts_with("picker-") {
        rune::run_windows_with_args(RunOptions { accessory: true }, args, move |_| {
            let spec = AppSpec {
                title: "Scrawl Picker (Rune)",
                size: (1440.0, 900.0),
                min_size: Some((0.0, 0.0)),
                init: dummy_picker_init,
                view: picker_view,
                scenes: PICKER_SCENES,
                fonts,
                families,
                setup: Some(picker_setup),
                poll: None,
                on_key: Some(picker_on_key),
            };
            let app = App::new(spec).window_options(WindowOptions {
                transparent: true,
                borderless: true,
                ..Default::default()
            });
            vec![WindowSpec::from_app("picker", app)]
        });
        return;
    }

    let spec = AppSpec {
        title: "Scrawl",
        size: (1120.0, 740.0),
        min_size: Some((760.0, 480.0)),
        init: dummy_review_init,
        view: review_view,
        scenes: REVIEW_SCENES,
        fonts,
        families,
        setup: Some(review_setup),
        poll: Some(review_poll),
        on_key: Some(review_on_key),
    };
    let app = App::new(spec)
        .chrome(Chrome {
            full_size_content: true,
            traffic_lights: Some((16.0, 16.0)),
            drag_regions: true,
        })
        .accessory()
        .init(move || {
            let mut state = dummy_review_init();
            state.coord.review_bundle = initial_bundle;
            state
        })
        .args(args);
    app.run();
}

pub fn run_app(
    initial_bundle: Option<(PathBuf, ScrawlBundle)>,
    start_recording_immediately: bool,
    start_iphone_immediately: bool,
    screenshot_out: Option<PathBuf>,
    mut options: RuneAppOptions,
) {
    let fonts = super::rune_pill::fonts();
    if options.snap.is_none()
        && let Some(out) = screenshot_out
    {
        options.snap = Some(out);
        if options.scene.is_none() {
            options.scene = Some(if initial_bundle.is_some() {
                "review-contact-sheet".to_owned()
            } else {
                "review-empty".to_owned()
            });
        }
    }
    if options.snap.is_some() {
        run_headless(initial_bundle, options, fonts);
        return;
    }

    scrawl_log(format!("rune_app started pid={}", std::process::id()));
    let (ui_sender, ui_receiver) = flume::unbounded();
    crate::phone::start_ingest_server(ui_sender.clone());
    let telemetry = SharedCaptureTelemetry::new(ui_sender);
    install_global_hotkeys(&telemetry);
    start_global_input_tap(Arc::clone(&telemetry));
    super::platform::start_updates();
    crate::retention::start();
    crate::recover::start();
    crate::hex::watch();
    crate::speech_model::ensure();

    if std::env::var_os("SCRAWL_NO_ONBOARDING").is_none() {
        let sender = telemetry.ui_sender.clone();
        std::thread::spawn(move || {
            if let Some(grants) = crate::onboarding::read_grants() {
                let event = crate::onboarding::Event::Launch(grants);
                let _ = sender.send(LiveOverlayEvent::Onboarding(event));
            }
        });
    }

    let families = super::rune_pill::families(fonts);
    let spec = AppSpec {
        title: "Scrawl",
        size: (1120.0, 740.0),
        min_size: Some((760.0, 480.0)),
        init: dummy_review_init,
        view: review_view,
        scenes: REVIEW_SCENES,
        fonts,
        families,
        setup: Some(review_setup),
        poll: Some(review_poll),
        on_key: Some(review_on_key),
    };
    let mut app = App::new(spec)
        .chrome(Chrome {
            full_size_content: true,
            traffic_lights: Some((16.0, 16.0)),
            drag_regions: true,
        })
        .accessory()
        .keep_running()
        .on_close(on_review_close)
        .args(Vec::<String>::new());
    if initial_bundle.is_none() {
        app = app.windowless();
    }

    let app_handle = app.handle();
    let (events_tx, events_rx) = flume::unbounded();
    let persistent_excluded: Arc<Mutex<Vec<u32>>> = Arc::default();
    let pill_pending: Arc<Mutex<Option<PillSnapshot>>> = Arc::default();
    let pill_slot: Arc<std::sync::OnceLock<OverlayHandle<PillState>>> = Arc::default();

    let pill_state = PillState::new(
        Arc::clone(&pill_pending),
        Arc::clone(&pill_slot),
        Some(events_tx.clone()),
        Some(app_handle.clone()),
    );
    let pill_excluded = Arc::clone(&persistent_excluded);
    let pill_telem = Arc::clone(&telemetry);
    let pill_overlay = app.overlay(
        OverlaySpec::new("Scrawl capsule (Rune)", (PANEL_W, PANEL_H), pill_view)
            .options(OverlayOptions {
                level: OverlayLevel::AboveOverlays,
                input: OverlayInput::Regions,
                keys: KeyPolicy::None,
                exclude_from_capture: true,
                prewarm: true,
                ..Default::default()
            })
            .poll(pill_poll)
            .on_created(move |made| {
                if let Some(id) = made.window_number {
                    pill_excluded
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(id);
                    pill_telem
                        .excluded_window_ids
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(id);
                }
            }),
        pill_state,
    );
    let _ = pill_slot.set(pill_overlay.clone());

    let screens = crate::screens::current();
    let mut displays = Vec::with_capacity(screens.len());
    for screen in &screens {
        let [px, py] = crate::screens::pill_origin(screen, f64::from(PANEL_W), f64::from(PANEL_Y));
        let f = screen.frame;
        let pill_local = [(px - f[0]) as f32, (py - f[1]) as f32];
        let pill_global = [px, py, f64::from(PANEL_W), f64::from(PANEL_H)];
        let picker_state = PickerState::new(
            f.map(|v| v as f32),
            Some(events_tx.clone()),
            Some(app_handle.clone()),
            false,
        );
        let picker_excluded = Arc::clone(&persistent_excluded);
        let picker_telem = Arc::clone(&telemetry);
        let picker = app.overlay(
            OverlaySpec::new(
                "Scrawl picker (Rune)",
                (f[2] as f32, f[3] as f32),
                picker_view,
            )
            .options(OverlayOptions {
                display: DisplayTarget::Native(NativeDisplayId::MacOS(screen.id)),
                placement: Some(OverlayPlacement::FullScreen),
                level: OverlayLevel::AboveStatus,
                input: OverlayInput::None,
                keys: KeyPolicy::OnDemand,
                exclude_from_capture: true,
                prewarm: true,
                ..Default::default()
            })
            .on_key(picker_on_key)
            .on_created(move |made| {
                if let Some(id) = made.window_number {
                    picker_excluded
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(id);
                    picker_telem
                        .excluded_window_ids
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(id);
                    picker_telem
                        .passthrough_window_ids
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(id);
                }
            }),
            picker_state,
        );
        displays.push(DisplaySlot {
            id: screen.id,
            frame: f,
            pill_local,
            pill_global,
            picker,
        });
    }

    if let Some(first) = displays.first() {
        pill_overlay.set_display(DisplayTarget::Native(NativeDisplayId::MacOS(first.id)));
        pill_overlay.set_placement(Some(OverlayPlacement::Rect {
            x: first.pill_local[0],
            y: first.pill_local[1],
            width: PANEL_W,
            height: PANEL_H,
        }));
    }

    let wake_handle = app_handle.clone();
    let stop_ticker = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop_ticker);
    std::thread::Builder::new()
        .name("rune-scrawl-ticker".into())
        .spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(8));
                wake_handle.wake();
            }
        })
        .expect("ticker thread");

    let app = app.init(move || {
        let coord = RuneCoordinator::new(
            telemetry,
            Some(ui_receiver),
            events_tx,
            events_rx,
            initial_bundle,
            start_recording_immediately,
            start_iphone_immediately,
            Some(app_handle),
            Some(pill_overlay),
            pill_pending,
            displays,
            persistent_excluded,
        );
        ReviewState::new(coord, false)
    });

    app.run();
    stop_ticker.store(true, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_scenes_and_coordinator_state_transitions_are_valid() {
        assert_eq!(ALL_SCENES.len(), PICKER_SCENES.len() + REVIEW_SCENES.len());
        let mut picker = dummy_picker_init();
        for &(name, apply) in PICKER_SCENES {
            apply(&mut picker);
            assert!(picker.snapshot.active, "picker scene {name} must activate");
        }
        let mut review = dummy_review_init();
        review_scene_empty(&mut review);
        assert!(review.coord.review_bundle.is_none());
        review.coord.set_island_phase(IslandPhase::Selecting {
            mode: SelectionMode::Area,
        });
        assert!(review.coord.is_selecting());
        review.coord.toggle_selection_mode();
        assert_eq!(review.coord.selection_mode(), Some(SelectionMode::Window));
        review.coord.dismiss_island();
        assert!(!review.coord.is_selecting());
    }
}
