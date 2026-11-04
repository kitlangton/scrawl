//! The Settings window in GPUI, drawn from `settings::View` like the SwiftUI window in
//! `capture/desktop/native/Settings.swift` (same panes, rows, copy, sizes and palette) and driven by the same
//! messages: `state`, `show`, `request`, `open-settings` and `close`. What it reports are
//! `settings::Action`s, as the helper's JSON lines parse into.
//!
//! Windows uses it in process (`settings::Session`); macOS still runs the SwiftUI window in
//! `apparent-helper settings`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, BoxShadow, ClickEvent, Context, Div, FocusHandle, FontWeight, Hsla,
    Image, ImageFormat, InteractiveElement, IntoElement, KeyDownEvent, KeyUpEvent, ModifiersChangedEvent,
    MouseButton, ParentElement, Pixels, Point, Render, Rgba, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, WindowBackgroundAppearance, WindowBounds,
    WindowControlArea, WindowKind, WindowOptions, div, img, linear_color_stop, linear_gradient,
    point, prelude::*, px, relative, rgb, rgba, size, svg,
};


use crate::capture::engine::runtime::{
    CARBON_CMD_KEY, CARBON_CONTROL_KEY, CARBON_OPTION_KEY, CARBON_SHIFT_KEY, ShortcutKind,
};
use crate::capture::desktop::settings::onboarding::{Permission, Status};
use crate::capture::desktop::settings::{Action, Event, Pane, Pressed, UpdateResult};

const WIDTH: f32 = 720.0;
const HEIGHT: f32 = 530.0;
const SIDEBAR: f32 = 210.0;
/// GPUI needs a definite content width to wrap captions, but a window manager can give us
/// less than the requested window width (notably XWayland under Hyprland).
fn column_widths(viewport: f32) -> (f32, f32) {
    let sidebar = if viewport < 650.0 { 150.0 } else { SIDEBAR };
    (sidebar, (viewport - sidebar - 1.0).max(0.0))
}
const TITLEBAR: f32 = 52.0;
const INSET: f32 = 30.0;
const ROW: f32 = 42.0;
const RADIUS: f32 = 2.0;
const WINDOWS: bool = cfg!(target_os = "windows");
const LINUX: bool = cfg!(target_os = "linux");

type Events = Arc<dyn Fn(Event) + Send + Sync>;

struct Request {
    commands: flume::Receiver<PresentationMessage>,
    pane: Option<Pane>,
    events: Events,
}

static PENDING: Mutex<Vec<Request>> = Mutex::new(Vec::new());

/// Queues a Settings window; the UI loop opens it (`poll`).
pub fn request_open(
    commands: flume::Receiver<PresentationMessage>,
    pane: Option<Pane>,
    events: Events,
) {
    PENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(Request {
            commands,
            pane,
            events,
        });
}

/// Opens queued Settings windows. Called from the UI loop.
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

use super::settings::{View as ViewState, PresentationMessage, ShortcutView as Shortcut, PermissionView as PermissionRow};
#[cfg(test)]
use super::settings::TranscriberView as Transcriber;

// Concept-derived 256×192 scenes stay at native device-pixel size in Settings.
// Moss shades belong only to this character, never to the UI/status palette.
fn update_frog(result: UpdateResult) -> Arc<Image> {
    use std::sync::OnceLock;
    static JUMP: OnceLock<Arc<Image>> = OnceLock::new();
    static LOUNGE: OnceLock<Arc<Image>> = OnceLock::new();
    static PUZZLED: OnceLock<Arc<Image>> = OnceLock::new();
    let (cache, bytes): (_, &[u8]) = match result {
        UpdateResult::Available => (&JUMP, include_bytes!("../../../capture/desktop/assets/settings/frog-jump.png")),
        UpdateResult::Current => (&LOUNGE, include_bytes!("../../../capture/desktop/assets/settings/frog-lounge.png")),
        UpdateResult::Failed => (&PUZZLED, include_bytes!("../../../capture/desktop/assets/settings/frog-puzzled.png")),
    };
    cache.get_or_init(|| Arc::new(Image::from_bytes(ImageFormat::Png, bytes.to_vec()))).clone()
}

// MARK: Look

/// Apparent's Paper workspace and Carbon navigation, following OS appearance.
#[derive(Clone, Copy)]
struct Theme {
    dark: bool,
}

impl Theme {
    fn ink(self, alpha: f32) -> Rgba {
        let base = if self.dark { 0xffffff00 } else { 0x00000000 };
        rgba(base | (alpha * 255.0).round() as u32)
    }
    fn window(self) -> Rgba {
        rgb(if self.dark { 0x201b24 } else { 0xf5f0e8 })
    }
    fn sidebar(self) -> Rgba {
        rgb(0x020102)
    }
    fn group(self) -> Rgba {
        rgb(if self.dark { 0x302a36 } else { 0xe7e0ee })
    }
    fn group_line(self) -> Rgba {
        self.ink(if self.dark { 0.075 } else { 0.08 })
    }
    fn separator(self) -> Rgba {
        self.ink(if self.dark { 0.07 } else { 0.075 })
    }
    fn text(self) -> Rgba {
        rgb(if self.dark { 0xf5f0e8 } else { 0x020102 })
    }
    fn secondary(self) -> Rgba {
        self.ink(if self.dark { 0.55 } else { 0.5 })
    }
    fn tertiary(self) -> Rgba {
        self.ink(if self.dark { 0.3 } else { 0.28 })
    }
    fn selection(self) -> Rgba {
        rgb(0x302442)
    }
    fn hover(self) -> Rgba {
        rgb(if self.dark { 0x302442 } else { 0xc4a9e6 })
    }
    fn control(self) -> Rgba {
        self.ink(if self.dark { 0.08 } else { 0.06 })
    }
    fn control_line(self) -> Rgba {
        self.ink(0.1)
    }
    fn track(self) -> Rgba {
        self.ink(if self.dark { 0.12 } else { 0.13 })
    }
    fn key(self) -> Rgba {
        if self.dark {
            rgba(0x00000047)
        } else {
            rgba(0x00000009)
        }
    }
    fn key_line(self) -> Rgba {
        self.ink(if self.dark { 0.08 } else { 0.09 })
    }
    fn menu(self) -> Rgba {
        rgb(if self.dark { 0x2b2b2b } else { 0xffffff })
    }
    fn accent(self) -> Rgba {
        rgb(0x7a58c7)
    }
    fn positive(self) -> Rgba {
        rgb(if self.dark { 0x77d6a5 } else { 0x146b43 })
    }
    fn negative(self) -> Rgba {
        rgb(if self.dark { 0xff8175 } else { 0xb42318 })
    }
}

/// An icon tile's gradient, top to bottom.
type Tint = (u32, u32);

const GREY: Tint = (0x9e9e9e, 0x737373);
const RED: Tint = (0xff6b61, 0xeb332b);
const BLUE: Tint = (0x5ca8ff, 0x2973f2);
const GREEN: Tint = (0x5cd18f, 0x29a866);
const ORANGE: Tint = (0xff9e4d, 0xf2701f);

fn pane_title(pane: Pane) -> &'static str {
    match pane {
        Pane::General => "General",
        Pane::Recording => "Recording",
        Pane::Storage => "Storage",
        Pane::Permissions => "Permissions",
        Pane::Agents => "Agents",
        Pane::About => "About",
    }
}

fn pane_icon(pane: Pane) -> (&'static str, Tint) {
    match pane {
        Pane::General => (crate::capture::desktop::icons::GEAR, GREY),
        Pane::Recording => (crate::capture::desktop::icons::RECORD, RED),
        Pane::Storage => (crate::capture::desktop::icons::DRIVE, BLUE),
        Pane::Permissions => (crate::capture::desktop::icons::HAND, GREEN),
        Pane::Agents => (crate::capture::desktop::icons::TERMINAL, (0x5c5c5c, 0x2e2e2e)),
        Pane::About => (crate::capture::desktop::icons::INFO, GREY),
    }
}

// MARK: The window

/// A popup's choices, open where it was clicked.
struct Menu {
    key: &'static str,
    options: Vec<(String, String, bool)>,
    selected: Option<String>,
    at: Point<Pixels>,
}

pub struct SettingsWindow {
    #[cfg(target_os = "linux")]
    _note_subject: Option<crate::capture::ui_process::SettingsWindow>,
    state: Option<ViewState>,
    pane: Pane,
    events: Events,
    focus: FocusHandle,
    appearance_revision: u64,
    update_later_focus: FocusHandle,
    update_now_focus: FocusHandle,
    update_dismiss_focus: FocusHandle,
    connect_focus: FocusHandle,
    connect_action_focus: [FocusHandle; 10],
    /// Modifiers held while a recorder listens, so their keycaps light as they go down.
    live: gpui::Modifiers,
    menu: Option<Menu>,
    meter: Option<(String, crate::capture::engine::runtime::audio::mic::Meter)>,
    icon: Arc<Image>,
    wordmark: Arc<Image>,
    /// The skill command was just copied (the button says Copied for a moment).
    copied: bool,
}

fn open(request: Request, cx: &mut App) {
    #[cfg(target_os = "linux")]
    {
        crate::capture::desktop::linux::appearance::initialize();
        crate::capture::desktop::linux::appearance::watch();
    }
    for handle in cx.windows() {
        if handle.downcast::<SettingsWindow>().is_some() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
    let bounds = Bounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx);
    let Request {
        commands,
        pane,
        events,
    } = request;
    let close_events = Arc::clone(&events);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("apparent Settings".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(20.0), px(20.0))),
            }),
            focus: true,
            show: true,
            kind: WindowKind::Normal,
            is_movable: true,
            is_resizable: false,
            is_minimizable: true,
            window_background: WindowBackgroundAppearance::Opaque,
            app_id: Some("dev.kitlangton.apparent".into()),
            ..Default::default()
        },
        move |window, cx| {
            #[cfg(not(target_os = "macos"))]
            crate::capture::desktop::set_app_icon(window);
            window.on_window_should_close(cx, move |_, _| {
                close_events(Event::Exited);
                true
            });
            cx.new(|cx| {
                let focus = cx.focus_handle();
                window.focus(&focus);
                // Like the SwiftUI window losing key: a recorder stops listening.
                cx.observe_window_activation(window, |view: &mut SettingsWindow, window, _| {
                    if !window.is_window_active() && view.recording().is_some() {
                        view.act(Action::CancelShortcut);
                    }
                })
                .detach();
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
                        let alive = this
                            .update_in(cx, |view: &mut SettingsWindow, window, cx| {
                                let changed = !messages.is_empty();
                                for message in messages {
                                    if !view.handle(message, window) {
                                        window.remove_window();
                                        return false;
                                    }
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
                                if changed || appearance_changed || view.pane == Pane::Recording && view.meter.is_some() {
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
                SettingsWindow {
                    #[cfg(target_os = "linux")]
                    _note_subject: crate::capture::desktop::linux::x11::window_id(window)
                        .map(crate::capture::ui_process::SettingsWindow::new),
                    state: None,
                    pane: pane.unwrap_or(Pane::General),
                    events,
                    focus,
                    appearance_revision: {
                        #[cfg(target_os = "linux")]
                        { crate::capture::desktop::linux::appearance::revision() }
                        #[cfg(not(target_os = "linux"))]
                        { 0 }
                    },
                    update_later_focus: cx.focus_handle(),
                    update_now_focus: cx.focus_handle(),
                    update_dismiss_focus: cx.focus_handle(),
                    connect_focus: cx.focus_handle().tab_stop(true),
                    connect_action_focus: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
                    live: gpui::Modifiers::default(),
                    menu: None,
                    meter: None,
                    icon: Arc::new(Image::from_bytes(
                        ImageFormat::Png,
                        about_icon_bytes().to_vec(),
                    )),
                    wordmark: Arc::new(Image::from_bytes(
                        ImageFormat::Png,
                        include_bytes!("../../../shared/assets/brand/apparent-wordmark.png").to_vec(),
                    )),
                    copied: false,
                }
            })
        },
    );
    if let Err(error) = opened {
        crate::capture::engine::runtime::apparent_log(format!("settings: the window failed to open: {error:#}"));
    }
}

impl SettingsWindow {
    /// Applies one message from the session; `false` when the window should close.
    fn handle(&mut self, message: PresentationMessage, window: &mut Window) -> bool {
        let message = match message {
            PresentationMessage::State(next) => { self.apply_state(next, window); return true; }
            PresentationMessage::Control(message) => message,
        };
        match message["type"].as_str().unwrap_or_default() {
            "state" => {
                if let Ok(next) = serde_json::from_value::<ViewState>(message) {
                    self.apply_state(next, window);
                }
            }
            "show" => {
                if let Some(pane) = message["pane"]
                    .as_str()
                    .and_then(|p| serde_json::from_value(p.into()).ok())
                {
                    self.pane = pane;
                    self.sync_meter();
                }
                window.activate_window();
            }
            "request" | "open-settings" => {
                if let Some(permission) = message["permission"]
                    .as_str()
                    .and_then(|p| serde_json::from_value(p.into()).ok())
                {
                    open_privacy_settings(permission);
                }
            }
            "close" => return false,
            _ => {}
        }
        true
    }

    fn apply_state(&mut self, next: ViewState, window: &mut Window) {
        if next.recording().is_none() {
            self.live = gpui::Modifiers::default();
        }
        if next.update_check.result.is_some()
            && self.state.as_ref().is_none_or(|state| state.update_check.result.is_none())
        {
            window.focus(&self.focus);
        }
        if next.update_check.result.is_none()
            && self.state.as_ref().is_some_and(|state| state.update_check.result.is_some())
        {
            window.focus(&self.focus);
        }
        self.state = Some(next);
        self.sync_meter();
    }

    fn act(&self, action: Action) {
        (self.events)(Event::Action(action));
    }

    fn set(&self, key: &str, value: serde_json::Value) {
        self.act(Action::Set {
            key: key.to_owned(),
            value,
        });
    }

    fn recording(&self) -> Option<ShortcutKind> {
        self.state.as_ref().and_then(ViewState::recording)
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(result) = self.state.as_ref().and_then(|state| state.update_check.result) {
            match event.keystroke.key.as_str() {
                "tab" => {
                    let next = if result == UpdateResult::Available {
                        if event.keystroke.modifiers.shift {
                            if self.update_later_focus.is_focused(window) { &self.update_now_focus } else { &self.update_later_focus }
                        } else if self.update_later_focus.is_focused(window) { &self.update_now_focus } else { &self.update_later_focus }
                    } else { &self.update_dismiss_focus };
                    window.focus(next);
                    cx.notify();
                }
                "enter" | "space" => self.act(if result == UpdateResult::Available {
                    if self.update_later_focus.is_focused(window) { Action::UpdateLater } else { Action::UpdateNow }
                } else { Action::DismissUpdateCheck }),
                "escape" => self.act(Action::DismissUpdateCheck),
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        let Some(kind) = self.recording() else {
            return;
        };
        let keystroke = &event.keystroke;
        if keystroke.key == "escape" {
            self.act(Action::CancelShortcut);
            return;
        }
        let m = keystroke.modifiers;
        let mut modifiers = 0;
        for (held, bit) in [
            (m.platform, CARBON_CMD_KEY),
            (m.control, CARBON_CONTROL_KEY),
            (m.alt, CARBON_OPTION_KEY),
            (m.shift, CARBON_SHIFT_KEY),
        ] {
            if held {
                modifiers |= bit;
            }
        }
        let Some(key_code) = key_code(&keystroke.key) else {
            return;
        };
        let typed = if keystroke.key.chars().count() == 1 {
            keystroke.key.clone()
        } else {
            String::new()
        };
        self.act(Action::Shortcut {
            kind,
            pressed: Pressed {
                key_code,
                modifiers,
                key: typed,
                reserved: false,
            },
        });
    }
}

/// The platform key code for a key GPUI names (`r`, `f5`, `space`), as the hotkeys register it.
pub(crate) fn key_code(key: &str) -> Option<u32> {
    #[cfg(not(target_os = "macos"))]
    let named: &[(&str, u32)] = &[
        ("space", 0x20),
        ("enter", 0x0D),
        ("tab", 0x09),
        ("backspace", 0x08),
        ("delete", 0x2E),
        ("left", 0x25),
        ("up", 0x26),
        ("right", 0x27),
        ("down", 0x28),
    ];
    #[cfg(target_os = "macos")]
    let named: &[(&str, u32)] = &[
        ("space", 49),
        ("enter", 36),
        ("tab", 48),
        ("backspace", 51),
        ("delete", 117),
        ("left", 123),
        ("right", 124),
        ("down", 125),
        ("up", 126),
    ];
    if let Some((_, code)) = named.iter().find(|(name, _)| *name == key) {
        return Some(*code);
    }
    if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<u32>().ok())
        && (1..=16).contains(&n)
    {
        #[cfg(not(target_os = "macos"))]
        return Some(0x6F + n);
        #[cfg(target_os = "macos")]
        return Some([122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111, 105, 107, 113, 106][n as usize - 1]);
    }
    let mut chars = key.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    crate::capture::engine::runtime::key_code_for_char(ch).map(u32::from)
}

#[cfg(target_os = "windows")]
fn open_privacy_settings(permission: Permission) {
    crate::capture::engine::runtime::platform::legacy_windows::platform::privacy::open_settings(permission);
}

#[cfg(target_os = "linux")]
fn open_privacy_settings(permission: Permission) {
    crate::capture::desktop::linux::privacy::open_settings(permission);
}

#[cfg(target_os = "macos")]
fn open_privacy_settings(_permission: Permission) {}

// MARK: Building blocks

fn icon(path: &'static str, size: f32, color: impl Into<Hsla>) -> gpui::Svg {
    svg()
        .path(path)
        .size(px(size))
        .flex_none()
        .text_color(color)
}

fn icon_tile(path: &'static str, tint: Tint, size: f32) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(size * 0.27))
        .bg(linear_gradient(
            180.0,
            linear_color_stop(rgb(tint.0), 0.0),
            linear_color_stop(rgb(tint.1), 1.0),
        ))
        .border_1()
        .border_color(rgba(0xffffff24))
        .flex()
        .items_center()
        .justify_center()
        .child(icon(path, size * 0.5, rgb(0xffffff)))
}

fn group(theme: Theme, rows: Vec<AnyElement>) -> Div {
    let mut column = div()
        .w_full()
        .flex()
        .flex_col()
        .rounded(px(RADIUS))
        .bg(theme.group())
        .border_1()
        .border_color(theme.group_line());
    for (index, row) in rows.into_iter().enumerate() {
        if index > 0 {
            column = column.child(div().h(px(1.0)).w_full().bg(theme.separator()));
        }
        column = column.child(row);
    }
    column
}

fn row(theme: Theme, title: &str, enabled: bool, trailing: impl IntoElement) -> Div {
    div()
        .h(px(ROW))
        .px(px(12.0))
        .flex()
        .items_center()
        .gap(px(12.0))
        .child(
            div()
                .flex_1()
                .text_size(px(14.0))
                .text_color(if enabled {
                    theme.text()
                } else {
                    theme.secondary()
                })
                .child(title.to_owned()),
        )
        .child(trailing)
}

fn header(theme: Theme, title: &str) -> Div {
    div()
        .px(px(8.0))
        .pt(px(20.0))
        .pb(px(6.0))
        .h(px(46.0))
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.secondary())
                .child(title.to_owned()),
        )
}

fn caption(theme: Theme, text: &str, error: bool) -> Div {
    div()
        .w_full()
        .px(px(8.0))
        .pt(px(7.0))
        .text_size(px(12.0))
        .line_height(px(16.0))
        .text_color(if error {
            theme.negative()
        } else {
            theme.secondary()
        })
        .child(text.to_owned())
}

fn pane_top_inset(pane: Pane) -> f32 {
    if pane == Pane::Recording { 0.0 } else { 20.0 }
}

fn about_icon_bytes() -> &'static [u8] {
    include_bytes!("../../../shared/assets/brand/apparent-a.png")
}

fn button(theme: Theme, id: impl Into<SharedString>, title: &str, enabled: bool) -> Stateful<Div> {
    div()
        .id(id.into())
        .flex_none()
        .h(px(24.0))
        .px(px(11.0))
        .flex()
        .items_center()
        .rounded_full()
        .bg(theme.control())
        .border_1()
        .border_color(theme.control_line())
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.text())
        .when(enabled, |d| {
            d.cursor_pointer().hover(|s| s.bg(theme.hover()))
        })
        .when(!enabled, |d| d.opacity(0.45))
        .child(title.to_owned())
}

/// The update decision uses the brand's primary and secondary action roles,
/// rather than the compact pill buttons used by Settings rows.
fn update_choice_button(theme: Theme, id: &'static str, title: &str, primary: bool, focus: &FocusHandle, selected: bool) -> Stateful<Div> {
    let paper = rgb(0xf5f0e8);
    let signal = theme.accent();
    div()
        .id(id)
        .track_focus(focus)
        .tab_index(0)
        .flex_none()
        .h(px(44.0))
        .px(px(16.0))
        .flex().items_center().justify_center()
        .bg(if primary { signal } else { theme.window() })
        .border_2()
        .border_color(if selected { if primary { theme.text() } else { signal } } else if primary { signal } else { theme.text() })
        .text_size(px(18.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(if primary { paper } else { theme.text() })
        .cursor_pointer()
        .when(primary, |d| d.hover(move |s| s.border_color(theme.text())))
        .when(!primary, |d| d.hover(move |s| s.bg(theme.group())))
        .child(title.to_owned())
}

fn switch(theme: Theme, id: &'static str, on: bool, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .w(px(38.0))
        .h(px(22.0))
        .p(px(2.0))
        .rounded_full()
        .bg(if on { theme.accent() } else { theme.track() })
        .flex()
        .when(on, |d| d.justify_end())
        .when(enabled, |d| d.cursor_pointer())
        .when(!enabled, |d| d.opacity(0.4))
        .child(div().size(px(18.0)).rounded_full().bg(rgb(0xffffff)).shadow(vec![
            BoxShadow {
                color: gpui::hsla(0.0, 0.0, 0.0, 0.22),
                offset: point(px(0.0), px(0.5)),
                blur_radius: px(1.0),
                spread_radius: px(0.0),
            },
        ]))
}

fn popup(theme: Theme, id: &'static str, value: &str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.0))
        .cursor_pointer()
        .child(
            div()
                .text_size(px(14.0))
                .text_color(theme.text())
                .opacity(0.85)
                .child(value.to_owned()),
        )
        .child(
            div()
                .size(px(20.0))
                .rounded_full()
                .border_1()
                .border_color(theme.control_line())
                .flex()
                .items_center()
                .justify_center()
                .child(icon(crate::capture::desktop::icons::CHEVRONS, 10.0, theme.secondary())),
        )
}

/// One key, Klack style: a modifier shows its glyph top left and its name below; the key itself
/// is a centred letter. Lit when part of the shortcut, dim otherwise.
fn keycap(theme: Theme, glyph: &str, name: Option<&str>, lit: bool, center: bool) -> Div {
    let color = if lit { theme.text() } else { theme.tertiary() };
    let cap = div()
        .flex_1()
        .h(px(52.0))
        .rounded(px(9.0))
        .bg(theme.key())
        .border_1()
        .border_color(theme.key_line())
        .text_color(color);
    if center {
        return cap
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(if glyph.chars().count() > 2 { 14.0 } else { 20.0 }))
            .font_weight(FontWeight::MEDIUM)
            .child(glyph.to_owned());
    }
    cap.px(px(10.0))
        .py(px(8.0))
        .flex()
        .flex_col()
        .justify_between()
        .when_some(name, |d, _| d.child(div().text_size(px(15.0)).child(glyph.to_owned())))
        .child(
            div()
                .text_size(px(if name.is_some() { 13.0 } else { 15.0 }))
                .child(name.unwrap_or(glyph).to_owned()),
        )
}

include!("settings_window/panes.rs");

/// The top strip of a column, which moves the window like a title bar.
fn drag_strip(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .w_full()
        .h(px(TITLEBAR))
        .flex_none()
        .when(WINDOWS, |d| {
            d.occlude()
                .window_control_area(WindowControlArea::Drag)
        })
}

/// Windows' minimize and close buttons in the top right corner (macOS has its traffic lights).
fn caption_buttons(theme: Theme) -> Option<Div> {
    if !WINDOWS {
        return None;
    }
    #[cfg(target_os = "windows")]
    let glyphs = if crate::capture::engine::runtime::platform::legacy_windows::platform::shell::windows_11() {
        "Segoe Fluent Icons"
    } else {
        "Segoe MDL2 Assets"
    };
    #[cfg(not(target_os = "windows"))]
    let glyphs = ".SystemUIFont";
    let control = |id: &'static str, area: WindowControlArea, glyph: &'static str, close: bool| {
        div()
            .id(id)
            .occlude()
            .window_control_area(area)
            .w(px(46.0))
            .h(px(32.0))
            .flex()
            .items_center()
            .justify_center()
            .font_family(glyphs)
            .text_size(px(10.0))
            .text_color(theme.text())
            .when(close, |d| {
                d.hover(|s| s.bg(rgb(0xc42b1c)).text_color(rgb(0xffffff)))
            })
            .when(!close, |d| d.hover(|s| s.bg(theme.selection())))
            .child(glyph)
    };
    Some(
        div()
            .absolute()
            .top_0()
            .right_0()
            .flex()
            .child(control(
                "settings-minimize",
                WindowControlArea::Min,
                "\u{E921}",
                false,
            ))
            .child(control(
                "settings-close",
                WindowControlArea::Close,
                "\u{E8BB}",
                true,
            )),
    )
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme {
            dark: {
                #[cfg(target_os = "linux")]
                { crate::capture::desktop::linux::appearance::is_dark() }
                #[cfg(not(target_os = "linux"))]
                { false }
            },
        };
        let root = div()
            .id("settings-root")
            .relative()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "tab" && view.recording().is_none() {
                    if event.keystroke.modifiers.shift { window.focus_prev(); } else { window.focus_next(); }
                    cx.stop_propagation(); return;
                }
                // GPUI clicks focused controls on key-up. Preserve the menu until
                // then so its account trigger cannot reopen it after activation.
                if matches!(event.keystroke.key.as_str(), "enter" | "space")
                    && view.menu.as_ref().is_some_and(|menu| menu.key == "connect") {
                    cx.stop_propagation(); return;
                }
                if !matches!(event.keystroke.key.as_str(), "enter" | "space")
                    && view.connect_key(event.keystroke.key.as_str(), window, cx) { cx.stop_propagation(); return; }
                if view.menu.take().is_some() {
                    cx.notify();
                }
                view.key_down(event, window, cx);
            }))
            .capture_key_up(cx.listener(|view, event: &KeyUpEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space")
                    && view.menu.as_ref().is_some_and(|menu| menu.key == "connect")
                    && view.connect_key(event.keystroke.key.as_str(), window, cx) {
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }))
            .on_modifiers_changed(cx.listener(|view, event: &ModifiersChangedEvent, _, cx| {
                if view.recording().is_some() {
                    view.live = event.modifiers;
                    cx.notify();
                }
            }))
            .relative()
            .size_full()
            .flex()
            .bg(theme.window())
            .font_family("Host Grotesk")
            .text_color(theme.text());
        let Some(state) = self.state.clone() else {
            return root
                .children(caption_buttons(theme))
                .child(crate::capture::desktop::running_build_badge());
        };
        let viewport = window.viewport_size();
        let (sidebar, content) = column_widths(f32::from(viewport.width));
        root.child(self.sidebar(theme, sidebar, cx))
            .child(div().w(px(1.0)).h_full().flex_none().bg(theme.separator()))
            .child(self.content(theme, &state, content, window, cx))
            .children(caption_buttons(theme))
            .children(self.menu_layer(theme, viewport, cx))
            .children(self.update_result_layer(theme, &state, window, cx))
            .child(crate::capture::desktop::running_build_badge())
    }
}

fn menu_setting_value(key: &str, id: &str) -> Option<serde_json::Value> {
    if key == "playbackLevel" {
        id.parse::<u8>().ok().filter(|level| *level <= 100).map(Into::into)
    } else {
        Some(id.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_reporting_preference_reaches_the_gpui_window() {
        for on in [true, false] {
            let state: ViewState = serde_json::from_value(serde_json::json!({
                "errorReporting": on
            })).unwrap();
            assert_eq!(state.error_reporting, on);
        }
    }

    #[test]
    fn shared_connect_view_and_commands_reach_the_gpui_transport() {
        use crate::capture::desktop::settings::connect::{Button, Command, View};
        assert!(serde_json::from_value::<ViewState>(serde_json::json!({})).unwrap().connect.is_none());
        let connect = View {
            title: "Apparent Connect".into(),
            status: "Waiting for browser approval".into(),
            detail: "Check the code before approving.".into(),
            server: Some("https://synthetic.example".into()),
            verification_uri: Some("https://synthetic.example/connect/device?code=TEST-CODE".into()),
            user_code: Some("TEST-CODE".into()),
            buttons: vec![Button { command: Command::OpenBrowser, title: "Open browser".into(), enabled: true }],
            note: "Synthetic transport contract.".into(),
            presentation: Default::default(),
        };
        let state: ViewState = serde_json::from_value(serde_json::json!({"type":"state", "connect":connect})).unwrap();
        assert_eq!(state.connect.as_ref(), Some(&connect));
        let action: Action = serde_json::from_value(serde_json::json!({"action":"connect", "command":connect.buttons[0].command})).unwrap();
        assert_eq!(action, Action::Connect { command: Command::OpenBrowser });
    }

    #[test]
    fn playback_preferences_reach_the_gpui_window() {
        let state: ViewState = serde_json::from_str(
            r#"{"lowerPlayback":true,"playbackLevel":35,"playbackAvailable":true}"#,
        ).unwrap();
        assert!(state.lower_playback);
        assert_eq!(state.playback_level, 35);
        assert!(state.playback_available);
    }

    #[test]
    fn playback_menu_sends_a_bounded_number_not_a_string() {
        for level in [0, 20, 35, 100] {
            assert_eq!(menu_setting_value("playbackLevel", &level.to_string()), Some(serde_json::json!(level)));
        }
        for invalid in ["101", "-1", "0.5", "garbage"] {
            assert_eq!(menu_setting_value("playbackLevel", invalid), None);
        }
        assert_eq!(menu_setting_value("transcriber", "hex"), Some(serde_json::json!("hex")));
    }

    #[test]
    fn update_check_has_one_progress_indicator() {
        let source = concat!(include_str!("settings_window/panes.rs"), include_str!("settings_window.rs"));
        let source = source.split("mod tests {").next().unwrap();
        assert!(source.contains("\"Checking…\""));
        assert!(!source.contains("\"Checking for updates…\""), "the button already shows progress");
    }

    #[test]
    fn settings_renderer_tracks_the_linux_system_appearance() {
        let source = concat!(include_str!("settings_window/panes.rs"), include_str!("settings_window.rs"));
        let source = source.split("mod tests {").next().unwrap();
        assert!(source.contains("crate::capture::desktop::linux::appearance::initialize()"));
        assert!(source.contains("crate::capture::desktop::linux::appearance::watch()"));
        assert!(source.contains("crate::capture::desktop::linux::appearance::revision()"));
        assert!(source.contains("crate::capture::desktop::linux::appearance::is_dark()"));
        assert!(!source.contains("let theme = Theme { dark: false };"));
    }

    #[test]
    fn update_frog_assets_preserve_the_approved_pond_scenes_without_baked_labels() {
        let mut decoded = Vec::new();
        for pose in ["jump", "lounge", "puzzled"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("capture/desktop/assets/settings/frog-{pose}.png"));
            let bytes = std::fs::read(&path).expect("each update outcome needs an actual sprite");
            assert!(bytes.len() < 60_000, "the three illustrations should stay compact");
            let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            let frame = reader.next_frame(&mut pixels).unwrap();
            assert_eq!((frame.width, frame.height), (256, 192));
            assert_eq!(frame.color_type, png::ColorType::Rgba);
            let pixels = &pixels[..frame.buffer_size()];
            assert!(pixels.chunks_exact(4).any(|p| p[3] == 255));
            assert_eq!(&pixels[..4], &[0, 0, 0, 0]);
            let counts = pixels.chunks_exact(4).filter(|p| p[3] > 220).fold((0, 0, 0), |(green, coral, purple), p| {
                (green + usize::from(p[1] > p[0] && p[0] > p[2]),
                 coral + usize::from(p[0] > 175 && i16::from(p[0]) > i16::from(p[1]) + 40 && p[1] > p[2]),
                 purple + usize::from(p[2] > p[0] && p[0] > p[1] && p[2] > 120))
            });
            assert!(counts.0 > 1_000, "{pose} lost its broad green lily pad");
            assert!(counts.1 > 8, "{pose} lost its coral cheek");
            assert!(counts.2 > 100, "{pose} lost its lavender pond details");
            assert!(pixels[(190 * 256 * 4)..].chunks_exact(4).all(|p| p[3] == 0), "{pose} includes a baked status label");
            decoded.push(pixels.to_vec());
        }
        assert!(decoded[0] != decoded[1] && decoded[1] != decoded[2] && decoded[0] != decoded[2]);
    }

    #[test]
    fn about_uses_the_lilac_a_mark_instead_of_the_brush_icon() {
        let decoder = png::Decoder::new(std::io::Cursor::new(about_icon_bytes()));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(frame.color_type, png::ColorType::Rgba);
        assert_eq!(&pixels[..4], &[0xb7, 0xa5, 0xd8, 0xff]);
        let center = (frame.width as usize * frame.height as usize / 2
            + frame.width as usize / 2) * 4;
        assert_eq!(&pixels[center..center + 4], &[0x02, 0x01, 0x02, 0xff]);
    }

    #[test]
    fn first_settings_group_has_room_below_the_title_rule() {
        for pane in [Pane::General, Pane::Storage, Pane::Permissions, Pane::Agents, Pane::About] {
            assert_eq!(pane_top_inset(pane), 20.0, "{pane:?}");
        }
        // Recording already starts with a section header, which supplies its own spacing.
        assert_eq!(pane_top_inset(Pane::Recording), 0.0);
    }

    #[test]
    fn settings_columns_fit_the_actual_narrow_viewport() {
        // XWayland can give the window less space than its requested size.
        for viewport in [440.0, 560.0, WIDTH] {
            let (sidebar, content) = column_widths(viewport);
            assert!(sidebar + 1.0 + content <= viewport);
            assert!(content > 0.0);
        }
    }

    #[test]
    fn the_command_transcriber_is_read_from_the_view() {
        let t: Transcriber =
            serde_json::from_str(r#"{"engine":"command","command":"whisper-apparent"}"#).unwrap();
        assert_eq!(t.engine, "command");
        assert_eq!(t.command.as_deref(), Some("whisper-apparent"));
        assert_eq!(Transcriber::default().command, None);
    }

    #[test]
    fn microphone_selection_and_fallback_reach_the_gpui_window() {
        let state: ViewState = serde_json::from_str(r#"{"microphone":{"selected":"usb-1","value":"USB mic","options":[{"id":"default","title":"System default (Built-in)","enabled":true},{"id":"usb-1","title":"USB mic (not connected)","enabled":false}],"note":"USB mic isn't connected. Using Built-in.","meter":"built-in"}}"#).unwrap();
        assert_eq!(state.microphone.selected, "usb-1");
        assert_eq!(
            state.microphone.note.as_deref(),
            Some("USB mic isn't connected. Using Built-in.")
        );
        assert_eq!(state.microphone.meter.as_deref(), Some("built-in"));
        assert!(!state.microphone.options[1].enabled);
    }

    #[test]
    fn the_linux_update_channel_is_read_from_the_view() {
        let state: ViewState = serde_json::from_str(
            r#"{"type":"state","updateChannel":"main","mainUpdates":true,"updates":true}"#,
        )
        .unwrap();
        assert_eq!(state.update_channel, "main");
        assert!(state.main_updates && state.updates);
    }

    #[test]
    fn update_check_progress_and_result_reach_the_window() {
        let checking: ViewState = serde_json::from_str(
            r#"{"updateCheck":{"checking":true,"result":null}}"#,
        ).unwrap();
        assert!(checking.update_check.checking);
        assert!(checking.update_check.result.is_none());
        for (status, label) in [
            ("available", "Update available"),
            ("current", "No update available"),
            ("failed", "Update check failed"),
        ] {
            let finished: ViewState = serde_json::from_value(serde_json::json!({
                "updateCheck": {"checking": false, "result": status}
            })).unwrap();
            assert!(!finished.update_check.checking);
            assert_eq!(finished.update_check.result.unwrap().label(), label);
        }
    }

    #[test]
    fn settings_text_and_update_status_contrast_in_both_themes() {
        fn luminance(color: Rgba) -> f32 {
            let channel = |x: f32| {
                if x <= 0.04045 {
                    x / 12.92
                } else {
                    ((x + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
        }
        for dark in [false, true] {
            let theme = Theme { dark };
            assert_eq!(theme.accent(), rgb(crate::capture::desktop::ui::pill_theme::SIGNAL));
            for (surface, text) in [
                (theme.hover(), theme.text()),
                (theme.window(), theme.text()),
                (theme.window(), theme.negative()),
                (theme.window(), theme.positive()),
            ] {
                let a = luminance(surface);
                let b = luminance(text);
                let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                assert!(contrast >= 4.5, "Settings text must remain readable in {dark:?} mode: {contrast}");
            }
            let a = luminance(theme.window());
            let b = luminance(theme.accent());
            let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
            assert!(contrast >= 3.0, "Update later focus border must be visible in {dark:?} mode: {contrast}");
            let paper = luminance(rgb(0xf5f0e8));
            let contrast = (paper.max(b) + 0.05) / (paper.min(b) + 0.05);
            assert!(contrast >= 4.5, "Paper text on Signal update action must remain readable: {contrast}");
        }
    }
    #[test]
    fn owned_speech_projection_preserves_native_wire_fields_for_every_state() {
        use crate::capture::engine::runtime::audio::transcription::speech_model::{State, Status};
        use super::super::settings::Speech;
        for state in [State::Unknown, State::Checking, State::Downloading, State::Ready,
            State::Unavailable, State::NeedsPermission, State::Failed] {
            for populated in [false, true] {
                let status = Status {
                    state: state.clone(), progress: 0.25,
                    engine: populated.then(|| "synthetic-engine".into()),
                    locale: populated.then(|| "en_US".into()),
                    locales: if populated { vec!["en_US".into()] } else { vec![] },
                    error: populated.then(|| "synthetic-error".into()),
                };
                let expected = serde_json::to_value(&status).unwrap();
                let owned = Speech::from(status);
                assert_eq!(serde_json::to_value(&owned).unwrap(), expected);
                assert_eq!(serde_json::from_value::<Speech>(expected).unwrap(), owned);
            }
        }
    }

    #[test]
    fn owned_view_keeps_renderer_defaults_and_unrendered_wire_fields() {
        let view: ViewState = serde_json::from_value(serde_json::json!({
            "capturesDir": {"path":"~/synthetic-captures", "custom":true},
            "commandLine": {"button":"install"},
            "transcriber": {"language":"de", "appleLocale":"de_DE"},
            "updateCheck": {"result":"current"}
        })).unwrap();
        assert_eq!(view.captures_dir.path, "~/synthetic-captures");
        assert!(view.captures_dir.custom);
        assert_eq!(view.command_line.button.as_deref(), Some("install"));
        assert_eq!(view.transcriber.language, "de");
        assert_eq!(view.transcriber.apple_locale, "de_DE");
        assert!(!view.update_check.checking);
        assert_eq!(view.update_check.result, Some(UpdateResult::Current));
        assert!(!view.playback_note.is_empty());
        assert!(view.transcriber.speech.is_none());
        let wire = serde_json::to_value(view).unwrap();
        assert!(wire.get("playbackAvailable").is_none());
        assert!(wire.get("playbackNote").is_none());
        assert!(wire["transcriber"].get("speech").is_none());
    }

}
