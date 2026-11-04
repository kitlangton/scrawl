//! The welcome window in GPUI, drawn from `onboarding::Flow::view()` like the SwiftUI window in
//! `capture/desktop/native/Onboarding.swift` (same screens, copy, sizes and palette), and driven by the same JSON
//! messages: `state`, `request`, `open-settings`, `hide`, `show` and `close`. Its buttons report
//! the same actions (`continue`, `back`, `later`, `close`, `grant:<permission>`).
//!
//! Linux uses it in process (`onboarding::Session`); macOS runs the SwiftUI window in
//! `apparent-helper onboarding`. Retained Windows rendering keeps its old theme.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Bounds, BoxShadow, Context, Div, FocusHandle, FontWeight, Image, ImageFormat,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Transformation, Window, WindowBackgroundAppearance,
    WindowBounds, WindowControlArea, WindowKind, WindowOptions, div, hsla, img, linear_color_stop,
    linear_gradient, point, prelude::*, px, radians, relative, rgb, rgba, size, svg,
};
use serde::Deserialize;

use crate::capture::desktop::settings::onboarding::{Event, Permission, Status, Step};

fn system_dark() -> bool {
    #[cfg(target_os = "linux")]
    { crate::capture::desktop::linux::appearance::is_dark() }
    #[cfg(not(target_os = "linux"))]
    { false }
}

fn colors() -> Colors {
    let dark = system_dark();
    if cfg!(target_os = "windows") {
        Colors {
            canvas: 0x161616,
            surface: 0x1d1d1d,
            raised: 0x252525,
            line: 0xffffff17,
            text: 0xf2f2f2,
            soft: 0xd0d0d0,
            muted: 0x969696,
            ink: 0xff3b30,
            positive: 0x69d89f,
        }
    } else if dark {
        Colors {
            canvas: 0x17151b,
            surface: 0x211e27,
            raised: 0x2b2732,
            line: 0xffffff24,
            text: 0xf3eff8,
            soft: 0xd5cfdf,
            muted: 0xa9a1b4,
            ink: 0xb69aff,
            positive: 0x77d6a5,
        }
    } else {
        Colors {
            canvas: 0xf5f0e8,
            surface: 0xe7e0ee,
            raised: 0xe7e0ee,
            line: 0x0201022e,
            text: 0x020102,
            soft: 0x020102,
            muted: 0x020102,
            ink: 0x7a58c7,
            positive: 0x146b43,
        }
    }
}

struct Colors {
    canvas: u32,
    surface: u32,
    raised: u32,
    line: u32,
    text: u32,
    soft: u32,
    muted: u32,
    ink: u32,
    positive: u32,
}
const WIDTH: f32 = 620.0;
/// The Windows caption strip (drag area and close button) above the page.
const CAPTION_H: f32 = if cfg!(target_os = "windows") {
    32.0
} else {
    0.0
};

type Events = Arc<dyn Fn(Event) + Send + Sync>;

struct Request {
    commands: flume::Receiver<serde_json::Value>,
    shortcut: String,
    events: Events,
}

static PENDING: Mutex<Vec<Request>> = Mutex::new(Vec::new());

/// Queues a welcome window; the UI loop opens it (`poll`).
pub fn request_open(
    commands: flume::Receiver<serde_json::Value>,
    shortcut: String,
    events: Events,
) {
    PENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(Request {
            commands,
            shortcut,
            events,
        });
}

/// Opens queued welcome windows. Called from the UI loop.
pub fn poll(cx: &mut App) {
    let requests = std::mem::take(
        &mut *PENDING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    for request in requests {
        open(request, cx);
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct RowState {
    permission: Permission,
    status: Status,
    #[serde(default)]
    requested: bool,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    relaunch: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ViewState {
    step: Step,
    index: usize,
    count: usize,
    rows: Vec<RowState>,
    ready: bool,
    fix: bool,
    #[serde(default)]
    shortcut_editing: bool,
    #[serde(default)]
    first_capture_required: bool,
    #[serde(default)]
    shortcut_chosen: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditorState {
    recording: bool,
    externally_managed: bool,
    label: String,
    is_default: bool,
    error: Option<String>,
}

/// Permissions this platform asks for. Windows has no Screen Recording, Input Monitoring or
/// Accessibility switch, so those rows never show there. Linux shows the microphone, and on
/// Wayland the input devices (the Accessibility row); it has no cabled iPhone.
fn shown_on_this_platform(permission: Permission) -> bool {
    #[cfg(target_os = "linux")]
    return permission == Permission::Microphone
        || permission == Permission::Accessibility && crate::capture::desktop::linux::wayland_session();
    #[cfg(not(target_os = "linux"))]
    return !cfg!(target_os = "windows")
        || matches!(permission, Permission::Microphone | Permission::Camera);
}

impl ViewState {
    fn rows(&self) -> Vec<&RowState> {
        self.rows
            .iter()
            .filter(|row| shown_on_this_platform(row.permission))
            .collect()
    }

    fn height(&self) -> f32 {
        let body = if self.fix {
            if cfg!(target_os = "windows") { 318.0 + 58.0 * self.rows().len().max(1) as f32 }
            else { 360.0 + 76.0 * self.rows().len().max(1) as f32 }
        } else {
            if cfg!(target_os = "windows") { 500.0 } else { 600.0 }
        };
        body + CAPTION_H
    }
}

fn open(request: Request, cx: &mut App) {
    #[cfg(target_os = "linux")]
    {
        crate::capture::desktop::linux::appearance::initialize();
        crate::capture::desktop::linux::appearance::watch();
    }
    for handle in cx.windows() {
        if handle.downcast::<OnboardingWindow>().is_some() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
    // `Session::open` sends the first state right away, so the window opens at its size.
    let mut first = None;
    let mut early = Vec::new();
    while let Ok(message) = request.commands.try_recv() {
        if first.is_none() && message["type"] == "state" {
            first = serde_json::from_value::<ViewState>(message.clone()).ok();
        }
        early.push(message);
    }
    let initially_visible = early.iter().rev().find_map(|message| match message["type"].as_str() {
        Some("hide") => Some(false),
        Some("show") => Some(true),
        _ => None,
    }).unwrap_or(true);
    let height = first.as_ref().map_or(if cfg!(target_os = "windows") { 500.0 + CAPTION_H } else { 600.0 + CAPTION_H }, ViewState::height);
    let bounds = Bounds::centered(None, size(px(WIDTH), px(height)), cx);
    let Request {
        commands,
        shortcut,
        events,
    } = request;
    let close_events = Arc::clone(&events);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some(if cfg!(target_os = "windows") { "Welcome to Apparent" } else { "Welcome to apparent" }.into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(14.0), px(14.0))),
            }),
            focus: initially_visible,
            show: initially_visible,
            kind: WindowKind::Normal,
            is_movable: true,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Opaque,
            app_id: Some("dev.kitlangton.apparent".into()),
            ..Default::default()
        },
        move |window, cx| {
            #[cfg(not(target_os = "macos"))]
            crate::capture::desktop::set_app_icon(window);
            window.on_window_should_close(cx, move |_, _| {
                close_events(Event::Action("close".into()));
                close_events(Event::Exited);
                true
            });
            cx.new(|cx| {
                let mut view = OnboardingWindow {
                    state: None,
                    shortcut,
                    editor: None,
                    focus: Some(cx.focus_handle()),
                    events,
                    step_since: Instant::now(),
                    forward: true,
                    opened: Instant::now(),
                    appearance_revision: {
                        #[cfg(target_os = "linux")]
                        { crate::capture::desktop::linux::appearance::revision() }
                        #[cfg(not(target_os = "linux"))]
                        { 0 }
                    },
                };
                for message in early {
                    view.handle(message, None);
                }
                cx.spawn_in(window, async move |this, cx| {
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_millis(16))
                            .await;
                        let mut messages = Vec::new();
                        while let Ok(message) = commands.try_recv() {
                            messages.push(message);
                        }
                        let disconnected = commands.is_disconnected();
                        let arrived = !messages.is_empty();
                        let alive = this
                            .update_in(cx, |view: &mut OnboardingWindow, window, cx| {
                                for message in messages {
                                    if !view.handle(message, Some(window)) {
                                        window.remove_window();
                                        return false;
                                    }
                                }
                                if !window.is_window_active()
                                    && view.editor.as_ref().is_some_and(|editor| editor.recording)
                                {
                                    if let Some(editor) = view.editor.as_mut() {
                                        editor.recording = false;
                                    }
                                    (view.events)(Event::Shortcut(
                                        crate::capture::desktop::settings::Action::CancelShortcut,
                                    ));
                                }
                                if disconnected {
                                    window.remove_window();
                                    return false;
                                }
                                let appearance_changed = {
                                    #[cfg(target_os = "linux")]
                                    {
                                        let revision = crate::capture::desktop::linux::appearance::revision();
                                        let changed = revision != view.appearance_revision;
                                        view.appearance_revision = revision;
                                        changed
                                    }
                                    #[cfg(not(target_os = "linux"))]
                                    { false }
                                };
                                if arrived || view.animating() || appearance_changed {
                                    cx.notify();
                                }
                                true
                            })
                            .unwrap_or(false);
                        if !alive {
                            break;
                        }
                    }
                })
                .detach();
                view
            })
        },
    );
    if opened.is_err() {
        crate::capture::engine::runtime::apparent_log("onboarding: the welcome window failed to open");
    }
}

pub struct OnboardingWindow {
    state: Option<ViewState>,
    shortcut: String,
    editor: Option<EditorState>,
    focus: Option<FocusHandle>,
    events: Events,
    step_since: Instant,
    forward: bool,
    opened: Instant,
    appearance_revision: u64,
}

impl OnboardingWindow {
    /// Applies one message from the session; `false` when the window should close.
    fn handle(&mut self, message: serde_json::Value, window: Option<&mut Window>) -> bool {
        match message["type"].as_str().unwrap_or_default() {
            "shortcut" => {
                if let Some(label) = message["label"].as_str() {
                    self.shortcut = label.to_owned();
                }
            }
            "shortcut-editor" => {
                self.editor = serde_json::from_value(message["shortcut"].clone()).ok();
            }
            "state" => {
                let Ok(next) = serde_json::from_value::<ViewState>(message).inspect_err(|error| {
                    crate::capture::engine::runtime::apparent_log(format!(
                        "onboarding: could not decode state: {error}"
                    ));
                }) else {
                    return true;
                };
                if let Some(ref old) = self.state
                    && old.step != next.step
                {
                    self.forward = next.index >= old.index;
                    self.step_since = Instant::now();
                }
                if let Some(window) = window
                    && self
                        .state
                        .as_ref()
                        .is_none_or(|old| old.height() != next.height())
                {
                    window.resize(size(px(WIDTH), px(next.height())));
                }
                self.state = Some(next);
            }
            "request" | "open-settings" => {
                if let Some(permission) = message["permission"]
                    .as_str()
                    .and_then(|p| serde_json::from_value(p.into()).ok())
                {
                    open_settings(permission);
                }
            }
            "hide" => set_visible(window, false),
            "show" => set_visible(window, true),
            "close" => return false,
            _ => {}
        }
        true
    }

    fn animating(&self) -> bool {
        let page = self.step_since.elapsed() < Duration::from_millis(400);
        let looping = self.state.as_ref().is_some_and(|s| {
            matches!(s.step, Step::Welcome | Step::TryIt)
                || s.rows
                    .iter()
                    .any(|r| r.requested && r.status != Status::Granted)
        });
        page || looping
    }

    fn act(&self, action: &str) {
        (self.events)(Event::Action(action.to_owned()));
    }
}

#[cfg(target_os = "windows")]
fn open_settings(permission: Permission) {
    crate::capture::engine::runtime::platform::legacy_windows::platform::privacy::open_settings(permission);
}

#[cfg(target_os = "linux")]
fn open_settings(permission: Permission) {
    crate::capture::desktop::linux::privacy::open_settings(permission);
}

#[cfg(target_os = "macos")]
fn open_settings(_permission: Permission) {}

#[cfg(target_os = "windows")]
fn set_visible(window: Option<&mut Window>, visible: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOW, ShowWindow};
    if let Some(hwnd) = window.and_then(|w| crate::capture::desktop::hwnd_of(w)) {
        let _ = unsafe { ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE }) };
    }
}

#[cfg(target_os = "linux")]
fn set_visible(window: Option<&mut Window>, visible: bool) {
    if let Some(id) = window.and_then(|w| crate::capture::desktop::linux::x11::window_id(w)) {
        crate::capture::desktop::linux::x11::set_visible(id, visible);
    }
}

#[cfg(target_os = "macos")]
fn set_visible(_window: Option<&mut Window>, _visible: bool) {}

use crate::capture::desktop::ease_out_cubic;

fn number_word(n: usize) -> String {
    ["zero", "one", "two", "three", "four", "five"]
        .get(n)
        .map_or_else(|| n.to_string(), |w| (*w).to_owned())
}

// MARK: Building blocks

fn keycap(label: &str, lit: bool, size: f32) -> Div {
    let wide = label.chars().count() > 1;
    let windows = cfg!(target_os = "windows");
    div()
        .flex_none()
        .min_w(px(size))
        .h(px(size))
        .when(wide, |d| d.px(px(8.0)))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(if windows { size * 0.24 } else { 2.0 }))
        .bg(linear_gradient(
            180.0,
            linear_color_stop(if windows { rgba(0xffffff21) } else { rgb(colors().raised) }, 0.0),
            linear_color_stop(if windows { rgba(0xffffff0f) } else { rgb(colors().raised) }, 1.0),
        ))
        .border_1()
        .border_color(if lit {
            rgba(if windows { 0xff3b30b3 } else { (colors().ink << 8) | 0xff })
        } else {
            rgba(if windows { 0xffffff24 } else { colors().line })
        })
        .when(windows, |d| d.shadow(vec![BoxShadow {
            color: hsla(0.0, 0.0, 0.0, 0.45),
            offset: point(px(0.0), px(2.0)),
            blur_radius: px(1.5),
            spread_radius: px(0.0),
        }]))
        .text_size(px(if windows { size * 0.44 } else { 18.0 }))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(if lit { colors().ink } else { colors().text }))
        .child(label.to_owned())
}

/// One keycap per key; with `lit_mods`, every key but the last is lit.
fn keycaps(label: &str, lit_mods: bool, size: f32) -> Div {
    let keys = crate::capture::engine::runtime::shortcut_keys(label);
    let last = keys.len().saturating_sub(1);
    div().flex().items_center().gap(px(size * 0.18)).children(
        keys.iter()
            .enumerate()
            .map(|(i, key)| keycap(key, lit_mods && i < last, size)),
    )
}

fn primary_button(id: &'static str, title: &str, enabled: bool) -> Stateful<Div> {
    let windows = cfg!(target_os = "windows");
    div()
        .id(id)
        .min_w(px(180.0))
        .h(px(if windows { 34.0 } else { 42.0 }))
        .px(px(22.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(if windows { 9.0 } else { 2.0 }))
        .bg(rgb(if enabled { colors().ink } else { colors().raised }))
        .border_1()
        .border_color(if enabled {
            rgba(if windows { 0xffffff29 } else { colors().line })
        } else {
            rgba(0xffffff14)
        })
        .when(enabled, |d| {
            let d = d.cursor_pointer().hover(|s| s.opacity(0.92));
            if !windows { return d; }
            d.shadow(vec![BoxShadow {
                color: hsla(0.0, 1.0, 0.59, 0.35),
                offset: point(px(0.0), px(3.0)),
                blur_radius: px(10.0),
                spread_radius: px(0.0),
            }])
        })
        .when(!enabled, |d| d.opacity(0.55))
        .text_size(px(if windows { 13.0 } else { 18.0 }))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(if windows { 0xffffff } else { colors().canvas }))
        .child(title.to_owned())
}

fn quiet_button(id: &'static str, title: &str) -> Stateful<Div> {
    div()
        .id(id)
        .px(px(10.0))
        .py(px(4.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|s| s.text_color(rgb(colors().soft)))
        .text_size(px(if cfg!(target_os = "windows") { 12.0 } else { 18.0 }))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(colors().muted))
        .child(title.to_owned())
}

fn row_button(id: SharedString, title: &str) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(if cfg!(target_os = "windows") { 26.0 } else { 32.0 }))
        .px(px(12.0))
        .flex()
        .items_center()
        .rounded(px(if cfg!(target_os = "windows") { 13.0 } else { 2.0 }))
        .bg(if cfg!(target_os = "windows") { rgba(0xffffff1a) } else { rgb(colors().raised) })
        .border_1()
        .border_color(rgba(if cfg!(target_os = "windows") { 0xffffff24 } else { colors().line }))
        .cursor_pointer()
        .hover(|s| s.bg(if cfg!(target_os = "windows") {
            rgba(0xffffff26)
        } else if system_dark() {
            rgb(0x51405e)
        } else {
            rgb(0xb7a5d8)
        }))
        .text_size(px(if cfg!(target_os = "windows") { 12.0 } else { 16.0 }))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(colors().text))
        .child(title.to_owned())
}

fn icon(path: &'static str, size: f32, color: u32) -> gpui::Svg {
    svg()
        .path(path)
        .size(px(size))
        .flex_none()
        .text_color(rgb(color))
}

fn spinner(phase: f32) -> gpui::Svg {
    icon(crate::capture::desktop::icons::SPINNER, 11.0, colors().muted).with_transformation(Transformation::rotate(radians(
        phase / 0.9 * std::f32::consts::TAU,
    )))
}

// MARK: Pages

impl OnboardingWindow {
    fn labels(&self) -> (String, String, String) {
        let paste = crate::capture::engine::runtime::system_paste_label().to_owned();
        let shift = if cfg!(not(target_os = "macos")) {
            "Shift"
        } else {
            "⇧"
        }
        .to_owned();
        (self.shortcut.clone(), paste, shift)
    }

    fn centered(&self, art: AnyElement, art_h: f32, title: &str, body: String) -> Div {
        div()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(34.0))
            .child(
                div()
                    .h(px(art_h))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(art),
            )
            .child(
                div()
                    .pt(px(18.0))
                    .text_size(px(if cfg!(target_os = "windows") { 22.0 } else { 36.0 }))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(colors().text))
                    .child(title.to_owned()),
            )
            .child(
                div()
                    .pt(px(10.0))
                    .max_w(px(440.0))
                    .text_center()
                    .text_size(px(if cfg!(target_os = "windows") { 13.5 } else { 18.0 }))
                    .line_height(px(if cfg!(target_os = "windows") { 20.0 } else { 27.0 }))
                    .text_color(if cfg!(target_os = "windows") { rgba(0xd0d0d0d9) } else { rgb(colors().text) })
                    .child(body),
            )
    }

    fn welcome_art(&self, phase: f32) -> AnyElement {
        if !cfg!(target_os = "windows") {
            return img(Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../../../shared/assets/brand/apparent-wordmark.png").to_vec(),
            )))
            .w(px(208.0))
            .h(px(48.0))
            .into_any_element();
        }
        let bob = -3.0 * (phase / 4.0 * std::f32::consts::TAU).sin();
        div()
            .relative()
            .size(px(150.0))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .absolute()
                    .size(px(150.0))
                    .rounded_full()
                    .bg(rgba(0xff3b3014))
                    .shadow(vec![BoxShadow {
                        color: hsla(0.0, 1.0, 0.59, 0.18),
                        offset: point(px(0.0), px(0.0)),
                        blur_radius: px(46.0),
                        spread_radius: px(0.0),
                    }]),
            )
            .child(
                div().relative().top(px(bob)).child(
                    img(Arc::new(Image::from_bytes(
                        ImageFormat::Png,
                        include_bytes!("../../../capture/desktop/assets/app-icon.png").to_vec(),
                    )))
                    .size(px(128.0)),
                ),
            )
            .into_any_element()
    }

    fn try_art(&self, phase: f32) -> AnyElement {
        let t = (phase / 2.2).fract();
        div()
            .relative()
            .w(px(260.0))
            .h(px(146.0))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .absolute()
                    .w(px(200.0 + 60.0 * t))
                    .h(px(86.0 + 60.0 * t))
                    .rounded(px(22.0))
                    .border_2()
                    .border_color(if cfg!(target_os = "windows") {
                        hsla(0.0, 1.0, 0.59, 0.55 * (1.0 - t))
                    } else {
                        hsla(262.0 / 360.0, 0.48, 0.56, 0.55 * (1.0 - t))
                    }),
            )
            .child(keycaps(&self.shortcut, true, 52.0))
            .into_any_element()
    }

    fn done_art(&self) -> AnyElement {
        let appear = ease_out_cubic(self.step_since.elapsed().as_secs_f32() / 0.3);
        div()
            .relative()
            .size(px(150.0))
            .flex()
            .items_center()
            .justify_center()
            .opacity(appear)
            .child(
                div()
                    .absolute()
                    .size(px(150.0))
                    .rounded_full()
                    .bg(rgba(if cfg!(target_os = "windows") { 0x69d89f14 } else { 0x146b4314 }))
                    .when(cfg!(target_os = "windows"), |d| d.shadow(vec![BoxShadow {
                        color: hsla(150.0 / 360.0, 0.58, 0.63, 0.16),
                        offset: point(px(0.0), px(0.0)),
                        blur_radius: px(30.0),
                        spread_radius: px(0.0),
                    }])),
            )
            .child(
                div()
                    .size(px(104.0 * (0.96 + 0.04 * appear)))
                    .rounded_full()
                    .bg(rgba(if cfg!(target_os = "windows") { 0x69d89f24 } else { 0x146b4324 }))
                    .border_1()
                    .border_color(rgba(if cfg!(target_os = "windows") { 0x69d89f8c } else { 0x146b438c }))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(crate::capture::desktop::icons::CHECK, 42.0, colors().positive)),
            )
            .into_any_element()
    }

    fn how_it_works(&self) -> Div {
        let (s, paste, shift) = self.labels();
        let pick = if cfg!(not(target_os = "macos")) {
            "a window or an area"
        } else {
            "a window, an area or your iPhone"
        };
        let row = |keys: Div, text: String| {
            div()
                .flex()
                .items_center()
                .gap(px(16.0))
                .px(px(16.0))
                .py(px(14.0))
                .child(
                    div()
                        .w(px(132.0))
                        .flex_none()
                        .flex()
                        .justify_center()
                        .child(keys),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(if cfg!(target_os = "windows") { 13.5 } else { 18.0 }))
                        .line_height(px(if cfg!(target_os = "windows") { 19.0 } else { 27.0 }))
                        .text_color(if cfg!(target_os = "windows") { rgba(0xd0d0d0e6) } else { rgb(colors().text) })
                        .child(text),
                )
        };
        let divider = || div().ml(px(16.0)).h(px(1.0)).bg(rgba(colors().line));
        div()
            .flex()
            .flex_col()
            .pt(px(44.0))
            .child(
                div()
                    .text_size(px(22.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(colors().text))
                    .text_center()
                    .child(if cfg!(target_os = "windows") { "How Apparent works" } else { "How apparent works" }),
            )
            .child(
                div()
                    .mt(px(24.0))
                    .rounded(px(if cfg!(target_os = "windows") { 12.0 } else { 2.0 }))
                    .bg(rgb(colors().surface))
                    .border_1()
                    .border_color(rgba(colors().line))
                    .flex()
                    .flex_col()
                    .child(row(
                        keycaps(&s, false, 26.0),
                        format!(
                            "Tap {s} to record the screen under your pointer, or hold it to pick {pick}."
                        ),
                    ))
                    .child(divider())
                    .child(row(
                        keycap(&shift, true, 26.0),
                        format!(
                            "Talk as you go, and hold {shift} and drag to draw in red. {} records your clicks too.",
                            if cfg!(target_os = "windows") { "Apparent" } else { "apparent" }
                        ),
                    ))
                    .child(divider())
                    .child(row(
                        keycaps(&paste, false, 26.0),
                        format!(
                            "Tap {s} again to finish. The capture is on your clipboard, ready to paste into your agent with {paste}."
                        ),
                    )),
            )
    }

    fn permission_row(&self, row: &RowState, phase: f32, cx: &mut Context<Self>) -> Div {
        let (glyph, name, why) = match row.permission {
            Permission::ScreenRecording => (
                crate::capture::desktop::icons::SCREEN,
                "Screen & System Audio Recording",
                "See your screen while you record".to_owned(),
            ),
            Permission::Microphone => (
                crate::capture::desktop::icons::MIC,
                "Microphone",
                "Hear what you say while you record".to_owned(),
            ),
            Permission::InputMonitoring => (
                crate::capture::desktop::icons::CLICK,
                "Input Monitoring",
                "Notice your clicks and shortcuts in every app".to_owned(),
            ),
            Permission::Accessibility if cfg!(target_os = "linux") => (
                crate::capture::desktop::icons::CLICK,
                "Input devices",
                "See ⇧ ink, clicks and Esc over other apps (the input group)".to_owned(),
            ),
            Permission::Accessibility => (
                crate::capture::desktop::icons::PERSON,
                "Accessibility",
                format!(
                    "Press {} for you when you use {}",
                    crate::capture::engine::runtime::system_paste_label(),
                    crate::capture::desktop::settings::paste_label()
                ),
            ),
            Permission::Camera => (
                crate::capture::desktop::icons::IPHONE,
                "Camera",
                "Show and record a cabled iPhone".to_owned(),
            ),
            Permission::SpeechRecognition => (
                crate::capture::desktop::icons::MIC,
                "Speech Recognition",
                "Write down what you say, on this computer".to_owned(),
            ),
        };
        let granted = row.status == Status::Granted;
        let denied = row.status == Status::Denied;
        let state = self.state.as_ref();
        let optional = !row.required && !state.is_some_and(|s| s.fix);
        let trailing: AnyElement = if granted && row.relaunch {
            div()
                .flex()
                .items_center()
                .gap(px(5.0))
                .child(icon(crate::capture::desktop::icons::REOPEN, 12.0, colors().positive))
                .child("On after reopen")
                .into_any_element()
        } else if granted {
            div()
                .flex()
                .items_center()
                .gap(px(5.0))
                .child(icon(crate::capture::desktop::icons::CHECK_CIRCLE, 13.0, colors().positive))
                .child("Allowed")
                .into_any_element()
        } else {
            let events = Arc::clone(&self.events);
            let action = format!(
                "grant:{}",
                serde_json::to_value(row.permission)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default()
            );
            // Windows and Linux have no prompt for desktop apps: the button always opens
            // Settings (on Linux, the guide to the input group).
            let title = if cfg!(target_os = "linux") && row.permission == Permission::Accessibility {
                "Copy command"
            } else if row.requested || denied || cfg!(not(target_os = "macos")) {
                "Open Settings"
            } else {
                "Allow"
            };
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .when(row.requested && !denied, |d| d.child(spinner(phase)))
                .child(
                    row_button(SharedString::from(action.clone()), title).on_click(
                        cx.listener(move |_, _, _, _| {
                            #[cfg(target_os = "linux")]
                            if title == "Copy command" {
                                crate::capture::desktop::linux::privacy::copy_input_command();
                                return;
                            }
                            events(Event::Action(action.clone()));
                        }),
                    ),
                )
                .into_any_element()
        };
        div()
            .h(px(if cfg!(target_os = "windows") { 58.0 } else { 76.0 }))
            .px(px(14.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(
                div()
                    .size(px(32.0))
                    .flex_none()
                    .rounded_full()
                    .bg(if granted {
                        rgba(if cfg!(target_os = "windows") { 0x69d89f24 } else { 0x146b4324 })
                    } else {
                        if cfg!(target_os = "windows") { rgba(0xffffff12) } else { rgb(colors().raised) }
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(glyph, 14.0, if granted { colors().positive } else { colors().text })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .text_size(px(if cfg!(target_os = "windows") { 13.0 } else { 18.0 }))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(colors().text))
                            .child(name)
                            .when(optional, |d| {
                                d.child(
                                    div()
                                        .px(px(6.0))
                                        .py(px(1.5))
                                        .rounded_full()
                                        .bg(if cfg!(target_os = "windows") { rgba(0xffffff12) } else { rgb(colors().raised) })
                                        .text_size(px(if cfg!(target_os = "windows") { 10.0 } else { 16.0 }))
                                        .text_color(rgb(colors().muted))
                                        .child("Optional"),
                                )
                            }),
                    )
                    .child(div().text_size(px(if cfg!(target_os = "windows") { 12.0 } else { 18.0 })).text_color(rgb(colors().muted)).child(why)),
            )
            .child(
                div()
                    .flex_none()
                    .ml(px(8.0))
                    .text_size(px(if cfg!(target_os = "windows") { 12.0 } else { 18.0 }))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(colors().soft))
                    .child(trailing),
            )
    }

    fn permissions(&self, state: &ViewState, phase: f32, cx: &mut Context<Self>) -> Div {
        let rows = state.rows();
        let windows = cfg!(target_os = "windows");
        let note = if cfg!(not(target_os = "macos")) {
            ""
        } else {
            "If macOS offers to Quit & Reopen apparent, choose Later; apparent reopens once when you're done."
        };
        let settings = if windows {
            "Windows Settings"
        } else if cfg!(target_os = "linux") {
            "a short guide"
        } else {
            "the macOS prompt or System Settings"
        };
        let only = |p: Permission| rows.len() == 1 && rows[0].permission == p;
        let (title, body) = if state.fix && only(Permission::Camera) {
            (
                "Camera, for your iPhone".to_owned(),
                "macOS treats a cabled iPhone's screen as a camera, so apparent needs Camera access to show and record it. Pick iPhone again once it's on.".to_owned(),
            )
        } else if state.fix && only(Permission::Accessibility) && cfg!(target_os = "linux") {
            (
                "The input devices, for ⇧ ink".to_owned(),
                "On Wayland, clicks and Shift ink need access to input devices.".to_owned(),
            )
        } else if state.fix && only(Permission::Accessibility) {
            (
                format!(
                    "Accessibility, for {}",
                    crate::capture::desktop::settings::paste_label()
                ),
                format!(
                    "{} pastes a capture into the app in front by pressing {} for you, and that needs Accessibility. Pressing {} yourself works without it.",
                    crate::capture::desktop::settings::paste_label(),
                    crate::capture::engine::runtime::system_paste_label(),
                    crate::capture::engine::runtime::system_paste_label()
                ),
            )
        } else if state.fix {
            (
                if windows { "Apparent can't record yet" } else { "apparent can't record yet" }.to_owned(),
                format!("Turn these on, then press {} again. {note}", self.shortcut)
                    .trim_end()
                    .to_owned(),
            )
        } else if cfg!(target_os = "linux") {
            ("Ready to record".to_owned(), "On Wayland, clicks and Shift ink need access to input devices.".to_owned())
        } else {
            let count = rows.iter().filter(|r| r.required).count();
            (
                format!(
                    "{} needs {} permission{}",
                    if windows { "Apparent" } else { "apparent" },
                    number_word(count),
                    if count == 1 { "" } else { "s" }
                ),
                format!("Each button opens {settings}. {note}")
                    .trim_end()
                    .to_owned(),
            )
        };
        let mut card = div()
            .mt(px(22.0))
            .rounded(px(if cfg!(target_os = "windows") { 12.0 } else { 2.0 }))
            .bg(rgb(colors().surface))
            .border_1()
            .border_color(rgba(colors().line))
            .flex()
            .flex_col();
        for (i, row) in rows.iter().enumerate() {
            if i > 0 {
                card = card.child(div().ml(px(14.0)).h(px(1.0)).bg(rgba(colors().line)));
            }
            card = card.child(self.permission_row(row, phase, cx));
        }
        #[cfg(target_os = "linux")]
        if rows.iter().any(|r| r.permission == Permission::Accessibility && r.status != Status::Granted) {
            card = card.child(
                div().px(px(14.0)).pb(px(14.0)).text_size(px(18.0)).text_color(rgb(colors().soft))
                    .child(div().child(crate::capture::desktop::linux::privacy::INPUT_COMMAND))
                    .child(div().mt(px(6.0)).child("Run this in a terminal, then log out and back in. Reopen apparent to continue; access is checked automatically."))
            );
        }
        div()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(44.0))
            .child(
                div()
                    .text_size(px(if cfg!(target_os = "windows") { 22.0 } else { 36.0 }))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(colors().text))
                    .child(title),
            )
            .child(
                div()
                    .pt(px(8.0))
                    .max_w(px(480.0))
                    .text_center()
                    .text_size(px(if cfg!(target_os = "windows") { 13.0 } else { 18.0 }))
                    .line_height(px(if cfg!(target_os = "windows") { 19.0 } else { 27.0 }))
                    .text_color(if cfg!(target_os = "windows") { rgba(0xd0d0d0d9) } else { rgb(colors().text) })
                    .child(body),
            )
            .child(card.w_full())
    }

    fn footer(&self, state: &ViewState, cx: &mut Context<Self>) -> Div {
        let primary: Option<(&str, &'static str, bool)> = match state.step {
            Step::Welcome => Some(("Get Started", "continue", true)),
            Step::HowItWorks => Some(("Continue", "continue", true)),
            Step::Permissions => Some((
                if state.fix { "Done" } else { "Continue" },
                "continue",
                state.ready,
            )),
            Step::Shortcut => Some(("Continue", "continue", !state.first_capture_required || state.shortcut_chosen)),
            Step::Done => Some(("Done", "continue", true)),
            Step::TryIt => None,
        };
        let secondary: Option<(&str, &'static str)> = match state.step {
            Step::Permissions => {
                Some((if state.fix { "Not now" } else { "Set up later" }, "later"))
            }
            Step::TryIt => Some((if state.first_capture_required { "Try later" } else { "Close" }, if state.first_capture_required { "later" } else { "continue" })),
            Step::Done => Some(("Open Settings", "settings")),
            _ => None,
        };
        let show_dots = state.step != Step::Done && !state.fix;
        let show_back = state.index > 0 && !state.fix && state.step != Step::Done;
        let mut column = div().flex().flex_col().items_center().gap(px(10.0));
        if let Some((title, action, enabled)) = primary {
            column = column.child(
                primary_button("onboarding-primary", title, enabled).when(enabled, |d| {
                    d.on_click(cx.listener(move |view, _, _, _| view.act(action)))
                }),
            );
        }
        column = match secondary {
            Some((title, action)) => column.child(
                quiet_button("onboarding-secondary", title)
                    .on_click(cx.listener(move |view, _, _, _| view.act(action))),
            ),
            None => column.child(div().h(px(22.0))),
        };
        column.child(
            div()
                .relative()
                .w_full()
                .h(px(22.0))
                .mt(px(8.0))
                .flex()
                .items_center()
                .justify_center()
                .when(show_dots, |d| {
                    d.child(div().flex().items_center().gap(px(6.0)).children(
                        (0..state.count).map(|i| {
                            let current = i == state.index;
                            div()
                                .h(px(6.0))
                                .w(px(if current { 16.0 } else { 6.0 }))
                                .rounded_full()
                                .bg(if current { rgb(colors().ink) } else { rgba(if cfg!(target_os = "windows") { 0xffffff2e } else { colors().line }) })
                        }),
                    ))
                })
                .when(show_back, |d| {
                    d.child(
                        div().absolute().left_0().child(
                            quiet_button("onboarding-back", "Back")
                                .on_click(cx.listener(|view, _, _, _| view.act("back"))),
                        ),
                    )
                }),
        )
    }

    fn caption(&self, cx: &mut Context<Self>) -> Option<Div> {
        if CAPTION_H == 0.0 {
            return None;
        }
        #[cfg(target_os = "windows")]
        let glyph_font = if crate::capture::engine::runtime::platform::legacy_windows::platform::shell::windows_11() {
            "Segoe Fluent Icons"
        } else {
            "Segoe MDL2 Assets"
        };
        #[cfg(not(target_os = "windows"))]
        let glyph_font = ".SystemUIFont";
        let _ = cx;
        Some(
            div()
                .flex_none()
                .h(px(CAPTION_H))
                .w_full()
                .flex()
                .child(
                    div()
                        .id("onboarding-drag")
                        .flex_1()
                        .h_full()
                        .occlude()
                        .window_control_area(WindowControlArea::Drag),
                )
                .child(
                    div()
                        .id("onboarding-close")
                        .occlude()
                        .window_control_area(WindowControlArea::Close)
                        .w(px(46.0))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .font_family(glyph_font)
                        .text_size(px(10.0))
                        .text_color(rgb(colors().text))
                        .hover(|s| s.bg(rgb(0xc42b1c)).text_color(rgb(0xffffff)))
                        .child("\u{E8BB}"),
                ),
        )
    }
}

impl Render for OnboardingWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let phase = self.opened.elapsed().as_secs_f32();
        let root = div()
            .id("onboarding-root")
            .relative()
            .when_some(self.focus.as_ref(), |d, focus| d.track_focus(focus))
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
                if !view
                    .state
                    .as_ref()
                    .is_some_and(|state| state.shortcut_editing)
                    || !view.editor.as_ref().is_some_and(|editor| editor.recording)
                {
                    return;
                }
                let key = &event.keystroke.key;
                if key == "escape" {
                    (view.events)(Event::Shortcut(crate::capture::desktop::settings::Action::CancelShortcut));
                } else if let Some(key_code) = crate::capture::desktop::settings::settings_window::key_code(key) {
                    let m = event.keystroke.modifiers;
                    let modifiers = [
                        (m.platform, crate::capture::engine::runtime::CARBON_CMD_KEY),
                        (m.control, crate::capture::engine::runtime::CARBON_CONTROL_KEY),
                        (m.alt, crate::capture::engine::runtime::CARBON_OPTION_KEY),
                        (m.shift, crate::capture::engine::runtime::CARBON_SHIFT_KEY),
                    ]
                    .into_iter()
                    .filter(|(held, _)| *held)
                    .fold(0, |bits, (_, bit)| bits | bit);
                    (view.events)(Event::Shortcut(crate::capture::desktop::settings::Action::Shortcut {
                        kind: crate::capture::engine::runtime::ShortcutKind::Record,
                        pressed: crate::capture::desktop::settings::Pressed {
                            key_code,
                            modifiers,
                            key: if key.chars().count() == 1 {
                                key.clone()
                            } else {
                                String::new()
                            },
                            reserved: false,
                        },
                    }));
                }
                cx.stop_propagation();
            }))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(colors().canvas))
            .child(
                // The red glow from the top edge, behind the caption too.
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(if cfg!(target_os = "windows") { 360.0 } else { 0.0 }))
                    .bg(linear_gradient(
                        180.0,
                        linear_color_stop(hsla(3.0 / 360.0, 1.0, 0.59, 0.10), 0.0),
                        linear_color_stop(hsla(3.0 / 360.0, 1.0, 0.59, 0.0), 1.0),
                    )),
            )
            .text_color(rgb(colors().text))
            .font_family(if cfg!(target_os = "windows") { ".SystemUIFont" } else { "Host Grotesk" })
            .children(self.caption(cx));
        let Some(state) = self.state.clone() else {
            return root.child(crate::capture::desktop::running_build_badge());
        };
        let (s, paste, _) = self.labels();
        let tray = if cfg!(not(target_os = "macos")) {
            "system tray"
        } else {
            "menu bar"
        };
        let page: AnyElement = match state.step {
            Step::Welcome => self
                .centered(
                    self.welcome_art(phase),
                    200.0,
                    if cfg!(target_os = "windows") { "Welcome to Apparent" } else { "Welcome to apparent" },
                    if cfg!(target_os = "windows") {
                        "Record your screen and your voice, draw on what matters, and paste it all into your agent."
                    } else {
                        "Record your screen and voice. Mark what matters, then paste the capture into your agent."
                    }.into(),
                )
                .into_any_element(),
            Step::HowItWorks => self.how_it_works().into_any_element(),
            Step::Permissions => self.permissions(&state, phase, cx).into_any_element(),
            Step::Shortcut => div()
                .flex()
                .flex_col()
                .items_center()
                .child(self.centered(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(keycaps(&self.shortcut, true, 52.0))
                        .into_any_element(),
                    200.0,
                    "Choose your shortcut",
                    if state.first_capture_required {
                        if state.shortcut_chosen {
                            "Shortcut saved. Continue to make your first capture.".to_owned()
                        } else if self.editor.as_ref().is_some_and(|editor| editor.externally_managed) {
                            "Your desktop manages this shortcut. Choose it below, then try it.".to_owned()
                        } else {
                            "Choose a key combination and press it here. You will use it to start and finish recordings.".to_owned()
                        }
                    } else if state.shortcut_editing {
                        if self.editor.as_ref().is_some_and(|editor| editor.recording) {
                            "Press the new shortcut now. Esc cancels; Continue keeps the current one.".to_owned()
                        } else {
                            "Continue to try it, or change it again.".to_owned()
                        }
                    } else {
                        format!("Record with {s}. You can change it now, or keep it and move on.")
                    },
                ))
                .when(state.shortcut_editing, |d| d.child(
                    div().flex().flex_col().items_center().gap(px(10.0))
                        .when_some(self.editor.as_ref(), |d, editor| d.child(
                            div().px(px(18.0)).py(px(10.0))
                                .bg(rgb(colors().raised)).border_1().border_color(rgba(colors().line))
                                .child(keycaps(&editor.label, editor.recording, 32.0))
                        ))
                        .child(self.editor.as_ref().map_or_else(
                            || "Waiting for recorder…".to_owned(),
                            |editor| if editor.externally_managed {
                                "This shortcut belongs to your desktop. Change it in your compositor settings.".to_owned()
                            } else if editor.recording {
                                "Press your new key combination".to_owned()
                            } else {
                                format!("Current shortcut: {}", editor.label)
                            },
                        ))
                        .children(self.editor.as_ref().and_then(|editor| editor.error.clone()))
                        .when(self.editor.as_ref().is_some_and(|editor| editor.recording), |d| {
                            d.child(quiet_button("onboarding-cancel-shortcut", "Cancel")
                                .on_click(cx.listener(|view, _, _, _| (view.events)(Event::Shortcut(crate::capture::desktop::settings::Action::CancelShortcut)))))
                        })
                        .when(!state.first_capture_required && self.editor.as_ref().is_some_and(|editor| !editor.is_default && !editor.externally_managed), |d| {
                            d.child(quiet_button("onboarding-reset-shortcut", "Reset to default")
                                .on_click(cx.listener(|view, _, _, _| (view.events)(Event::Shortcut(crate::capture::desktop::settings::Action::ResetShortcut { kind: crate::capture::engine::runtime::ShortcutKind::Record })))))
                        }),
                ))
                .child(
                    quiet_button("onboarding-change-shortcut", if !state.first_capture_required { "Change shortcut" } else if self.editor.as_ref().is_some_and(|editor| editor.externally_managed) { "Use desktop shortcut" } else { "Set shortcut" })
                        .on_click(cx.listener(|view, _, window, _| {
                            if let Some(focus) = &view.focus { window.focus(focus); }
                            view.act(if view.state.as_ref().is_some_and(|state| state.first_capture_required) && view.editor.as_ref().is_some_and(|editor| editor.externally_managed) { "use-desktop-shortcut" } else { "change-shortcut" });
                        })),
                )
                .into_any_element(),
            Step::TryIt => self
                .centered(
                    self.try_art(phase),
                    164.0,
                    if cfg!(target_os = "windows") { "Make your first Apparent" } else { "Make your first capture" },
                    if state.first_capture_required {
                        format!("Tap {s} to start. Try giving feedback on this onboarding: talk and point at something. Tap {s} again to finish. The capture stays local; nothing is sent to us.")
                    } else {
                        format!("Tap {s} now. Talk, point at something, then tap {s} again.")
                    },
                )
                .into_any_element(),
            Step::Done => self
                .centered(
                    self.done_art(),
                    164.0,
                    "It's on your clipboard",
                    format!(
                        "Press {paste} in your agent. {} stays in the {tray}; tap {s} whenever you want to show something.",
                        if cfg!(target_os = "windows") { "Apparent" } else { "apparent" }
                    ),
                )
                .into_any_element(),
        };
        let t = ease_out_cubic(self.step_since.elapsed().as_secs_f32() / 0.3);
        let shift = (1.0 - t) * if self.forward { 24.0 } else { -24.0 };
        root.child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .px(px(40.0))
                .pb(px(20.0))
                .child(
                    div()
                        .relative()
                        .w_full()
                        .left(px(shift))
                        .opacity(t)
                        .child(page),
                )
                .child(div().flex_1())
                .child(self.footer(&state, cx)),
        )
        .child(crate::capture::desktop::running_build_badge())
        .w(relative(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::desktop::settings::onboarding::{Flow, Grants, Input};

    #[test]
    fn accepts_states_from_the_onboarding_flow() {
        let now = Instant::now();
        let mut window = OnboardingWindow {
            state: None,
            shortcut: "Alt+Shift+S".into(),
            editor: None,
            focus: None,
            events: Arc::new(|_| {}),
            step_since: now,
            forward: true,
            opened: now,
            appearance_revision: 0,
        };
        let mut flow = Flow::tour(Step::Welcome, Some(Grants::default()));
        let granted = Grants {
            statuses: Permission::REQUIRED
                .into_iter()
                .map(|permission| (permission, Status::Granted))
                .collect(),
            screen_needs_relaunch: false,
        };
        for (step, ready, input) in [
            (Step::Welcome, false, Input::Continue),
            (Step::HowItWorks, false, Input::Continue),
            (
                Step::Permissions,
                false,
                Input::Grants {
                    grants: granted,
                    now_ms: 0,
                },
            ),
            (Step::Permissions, true, Input::Continue),
            (Step::Shortcut, true, Input::Continue),
            (Step::TryIt, true, Input::CaptureEnded { completed: true }),
            (Step::Done, true, Input::Continue),
        ] {
            let view = flow.view();
            let mut message = serde_json::to_value(&view).unwrap();
            message["type"] = "state".into();
            assert!(window.handle(message, None));
            let state = window
                .state
                .as_ref()
                .expect("the flow's state was accepted");
            assert_eq!(state.step, step);
            assert_eq!(state.ready, ready);
            assert_eq!(state.index, view.index);
            assert_eq!(state.count, view.count);
            assert_eq!(state.fix, view.fix);
            assert_eq!(state.rows.len(), view.rows.len());
            for (received, sent) in state.rows.iter().zip(&view.rows) {
                assert_eq!(received.permission, sent.permission);
                assert_eq!(received.status, sent.status);
                assert_eq!(received.requested, sent.requested);
                assert_eq!(received.required, sent.required);
                assert_eq!(received.relaunch, sent.relaunch);
            }
            flow.on(input);
        }
        assert!(window.handle(
            serde_json::json!({"type": "shortcut", "label": "Ctrl+Alt+K"}),
            None
        ));
        assert_eq!(window.shortcut, "Ctrl+Alt+K");
    }
}
