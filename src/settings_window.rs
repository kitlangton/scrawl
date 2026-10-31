//! The Settings window in GPUI, drawn from `settings::View` like the SwiftUI window in
//! `helper/Settings.swift` (same panes, rows, copy, sizes and palette) and driven by the same
//! messages: `state`, `show`, `request`, `open-settings` and `close`. What it reports are
//! `settings::Action`s, as the helper's JSON lines parse into.
//!
//! Windows uses it in process (`settings::Session`); macOS still runs the SwiftUI window in
//! `scrawl-helper settings`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, BoxShadow, ClickEvent, Context, Div, FocusHandle, FontWeight, Hsla,
    Image, ImageFormat, InteractiveElement, IntoElement, KeyDownEvent, ModifiersChangedEvent,
    MouseButton, ParentElement, Pixels, Point, Render, Rgba, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, WindowAppearance, WindowBackgroundAppearance,
    WindowBounds, WindowControlArea, WindowKind, WindowOptions, div, img, linear_color_stop,
    linear_gradient, point, prelude::*, px, rgb, rgba, size, svg,
};
use serde::Deserialize;

use crate::capture::{
    CARBON_CMD_KEY, CARBON_CONTROL_KEY, CARBON_OPTION_KEY, CARBON_SHIFT_KEY, ShortcutKind,
};
use crate::onboarding::{Permission, Status};
use crate::settings::{Action, Event, Pane, Pressed};

const WIDTH: f32 = 620.0;
const HEIGHT: f32 = 448.0;
const SIDEBAR: f32 = 210.0;
/// The content pane's width: what the 620-point window keeps after the sidebar and the
/// hairline between them. Giving the pane an explicit width (rather than `flex_1`) makes the
/// space it hands its children definite, which GPUI requires to wrap text: with an indefinite
/// width a caption measures as one full line, inflates the column and pushes trailing controls
/// (the permission rows' buttons) past the window's edge.
const CONTENT: f32 = WIDTH - SIDEBAR - 1.0;
const TITLEBAR: f32 = 52.0;
const INSET: f32 = 20.0;
const ROW: f32 = 42.0;
const RADIUS: f32 = 12.0;
const WINDOWS: bool = cfg!(target_os = "windows");

type Events = Arc<dyn Fn(Event) + Send + Sync>;

struct Request {
    commands: flume::Receiver<serde_json::Value>,
    pane: Option<Pane>,
    events: Events,
}

static PENDING: Mutex<Vec<Request>> = Mutex::new(Vec::new());

/// Queues a Settings window; the UI loop opens it (`poll`).
pub fn request_open(
    commands: flume::Receiver<serde_json::Value>,
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

// MARK: What the session sends (`settings::View`, as JSON)

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
struct Login {
    on: bool,
    available: bool,
    needs_approval: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Shortcut {
    kind: ShortcutKind,
    title: String,
    label: String,
    key: String,
    shift: bool,
    control: bool,
    option: bool,
    command: bool,
    is_default: bool,
    #[serde(default)]
    clearable: bool,
    recording: bool,
    error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct ModelOption {
    id: String,
    title: String,
    chosen: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
struct Transcriber {
    engine: String,
    hex_title: String,
    hex_enabled: bool,
    models: Vec<ModelOption>,
    speech: Speech,
}

/// `speech_model::Status`: the local model's download.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
struct Speech {
    state: String,
    progress: f64,
    error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct RetentionOption {
    id: String,
    label: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
struct Retention {
    preset: Option<String>,
    options: Vec<RetentionOption>,
    note: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct PermissionRow {
    permission: Permission,
    status: Status,
    required: bool,
    requested: bool,
    relaunch: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
struct ViewState {
    version: String,
    login: Login,
    auto_send: bool,
    shortcuts: Vec<Shortcut>,
    transcriber: Transcriber,
    video_export: bool,
    retention: Retention,
    captures_size: Option<String>,
    permissions: Vec<PermissionRow>,
    updates: bool,
    skill_command: String,
}

impl ViewState {
    fn recording(&self) -> Option<ShortcutKind> {
        self.shortcuts.iter().find(|s| s.recording).map(|s| s.kind)
    }
}

// MARK: Look

/// `SettingsTheme` from the SwiftUI window: the same greys, as alpha over the window.
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
        rgb(if self.dark { 0x1e1e1e } else { 0xf3f3f3 })
    }
    fn sidebar(self) -> Rgba {
        rgb(if self.dark { 0x262626 } else { 0xe9e9e9 })
    }
    fn group(self) -> Rgba {
        if self.dark {
            rgba(0xffffff0b)
        } else {
            rgb(0xffffff)
        }
    }
    fn group_line(self) -> Rgba {
        self.ink(if self.dark { 0.075 } else { 0.08 })
    }
    fn separator(self) -> Rgba {
        self.ink(if self.dark { 0.07 } else { 0.075 })
    }
    fn text(self) -> Rgba {
        rgb(if self.dark { 0xf2f2f2 } else { 0x1a1a1a })
    }
    fn secondary(self) -> Rgba {
        self.ink(if self.dark { 0.55 } else { 0.5 })
    }
    fn tertiary(self) -> Rgba {
        self.ink(if self.dark { 0.3 } else { 0.28 })
    }
    fn selection(self) -> Rgba {
        self.ink(if self.dark { 0.08 } else { 0.07 })
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
        rgb(0x0a84ff)
    }
    fn positive(self) -> Rgba {
        rgb(0x33bd6b)
    }
    fn negative(self) -> Rgba {
        rgb(0xff453b)
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
        Pane::General => (crate::icons::GEAR, GREY),
        Pane::Recording => (crate::icons::RECORD, RED),
        Pane::Storage => (crate::icons::DRIVE, BLUE),
        Pane::Permissions => (crate::icons::HAND, GREEN),
        Pane::Agents => (crate::icons::TERMINAL, (0x5c5c5c, 0x2e2e2e)),
        Pane::About => (crate::icons::INFO, GREY),
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
    state: Option<ViewState>,
    pane: Pane,
    events: Events,
    focus: FocusHandle,
    /// Modifiers held while a recorder listens, so their keycaps light as they go down.
    live: gpui::Modifiers,
    menu: Option<Menu>,
    icon: Arc<Image>,
    /// The skill command was just copied (the button says Copied for a moment).
    copied: bool,
}

fn open(request: Request, cx: &mut App) {
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
                title: Some("Scrawl Settings".into()),
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
            app_id: Some("dev.kitlangton.scrawl".into()),
            ..Default::default()
        },
        move |window, cx| {
            #[cfg(target_os = "windows")]
            crate::ui::set_app_icon(window);
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
                                if changed {
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
                    state: None,
                    pane: pane.unwrap_or(Pane::General),
                    events,
                    focus,
                    live: gpui::Modifiers::default(),
                    menu: None,
                    icon: Arc::new(Image::from_bytes(
                        ImageFormat::Png,
                        include_bytes!("../assets/app-icon.png").to_vec(),
                    )),
                    copied: false,
                }
            })
        },
    );
    if opened.is_err() {
        crate::capture::scrawl_log("settings: the window failed to open");
    }
}

impl SettingsWindow {
    /// Applies one message from the session; `false` when the window should close.
    fn handle(&mut self, message: serde_json::Value, window: &mut Window) -> bool {
        match message["type"].as_str().unwrap_or_default() {
            "state" => {
                if let Ok(next) = serde_json::from_value::<ViewState>(message) {
                    if next.recording().is_none() {
                        self.live = gpui::Modifiers::default();
                    }
                    self.state = Some(next);
                }
            }
            "show" => {
                if let Some(pane) = message["pane"]
                    .as_str()
                    .and_then(|p| serde_json::from_value(p.into()).ok())
                {
                    self.pane = pane;
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

    fn key_down(&mut self, event: &KeyDownEvent) {
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
fn key_code(key: &str) -> Option<u32> {
    #[cfg(target_os = "windows")]
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
    #[cfg(not(target_os = "windows"))]
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
        #[cfg(target_os = "windows")]
        return Some(0x6F + n);
        #[cfg(not(target_os = "windows"))]
        return Some(
            [
                122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111, 105, 107, 113, 106,
            ][n as usize - 1],
        );
    }
    let mut chars = key.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    crate::capture::key_code_for_char(ch).map(u32::from)
}

#[cfg(target_os = "windows")]
fn open_privacy_settings(permission: Permission) {
    crate::win::privacy::open_settings(permission);
}

#[cfg(not(target_os = "windows"))]
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
            d.cursor_pointer().hover(|s| s.bg(theme.selection()))
        })
        .when(!enabled, |d| d.opacity(0.45))
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
        .child(
            div()
                .size(px(18.0))
                .rounded_full()
                .bg(rgb(0xffffff))
                .shadow(vec![BoxShadow {
                    color: gpui::hsla(0.0, 0.0, 0.0, 0.22),
                    offset: point(px(0.0), px(0.5)),
                    blur_radius: px(1.0),
                    spread_radius: px(0.0),
                }]),
        )
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
                .child(icon(crate::icons::CHEVRONS, 10.0, theme.secondary())),
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
            .text_size(px(if glyph.chars().count() > 2 {
                14.0
            } else {
                20.0
            }))
            .font_weight(FontWeight::MEDIUM)
            .child(glyph.to_owned());
    }
    cap.px(px(10.0))
        .py(px(8.0))
        .flex()
        .flex_col()
        .justify_between()
        .when_some(name, |d, _| {
            d.child(div().text_size(px(15.0)).child(glyph.to_owned()))
        })
        .child(
            div()
                .text_size(px(if name.is_some() { 13.0 } else { 15.0 }))
                .child(name.unwrap_or(glyph).to_owned()),
        )
}

// MARK: Panes

impl SettingsWindow {
    fn sidebar(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let section = |title: &str| {
            div()
                .pl(px(10.0))
                .pt(px(16.0))
                .pb(px(4.0))
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.secondary())
                .child(title.to_owned())
        };
        div()
            .flex_none()
            .w(px(SIDEBAR))
            .h_full()
            .bg(theme.sidebar())
            .flex()
            .flex_col()
            .child(drag_strip("settings-drag-sidebar"))
            .child(
                div()
                    .px(px(10.0))
                    .flex()
                    .flex_col()
                    .child(self.item(theme, Pane::General, cx))
                    .child(section("Settings"))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(self.item(theme, Pane::Recording, cx))
                            .child(self.item(theme, Pane::Storage, cx))
                            .child(self.item(theme, Pane::Permissions, cx))
                            .child(self.item(theme, Pane::Agents, cx)),
                    )
                    .child(section("Scrawl"))
                    .child(self.item(theme, Pane::About, cx)),
            )
    }

    fn item(&self, theme: Theme, pane: Pane, cx: &mut Context<Self>) -> Stateful<Div> {
        let (path, tint) = pane_icon(pane);
        let selected = self.pane == pane;
        div()
            .id(SharedString::from(format!(
                "settings-pane-{}",
                pane_title(pane)
            )))
            .h(px(36.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .rounded(px(9.0))
            .when(selected, |d| d.bg(theme.selection()))
            .when(!selected, |d| d.hover(|s| s.bg(theme.ink(0.04))))
            .cursor_pointer()
            .child(icon_tile(path, tint, 24.0))
            .child(
                div()
                    .text_size(px(14.0))
                    .text_color(theme.text())
                    .child(pane_title(pane)),
            )
            .on_click(cx.listener(move |view, _, _, cx| {
                if view.recording().is_some() {
                    view.act(Action::CancelShortcut);
                }
                view.menu = None;
                view.pane = pane;
                cx.notify();
            }))
    }

    fn content(&self, theme: Theme, state: &ViewState, cx: &mut Context<Self>) -> Stateful<Div> {
        let (path, tint) = pane_icon(self.pane);
        let body = match self.pane {
            Pane::General => self.general(theme, state, cx),
            Pane::Recording => self.recording_pane(theme, state, cx),
            Pane::Storage => self.storage(theme, state, cx),
            Pane::Permissions => self.permissions(theme, state, cx),
            Pane::Agents => self.agents(theme, state, cx),
            Pane::About => self.about(theme, state, cx),
        };
        div()
            .id("settings-content")
            .w(px(CONTENT))
            .h_full()
            .bg(theme.window())
            .relative()
            .flex()
            .flex_col()
            .on_click(cx.listener(|view, _, _, _| {
                if view.recording().is_some() {
                    view.act(Action::CancelShortcut);
                }
            }))
            .child(
                div()
                    .relative()
                    .h(px(TITLEBAR))
                    .flex_none()
                    .px(px(INSET))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(icon_tile(path, tint, 24.0))
                    .child(
                        div()
                            .text_size(px(16.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text())
                            .child(pane_title(self.pane)),
                    )
                    .child(
                        drag_strip("settings-drag-title")
                            .absolute()
                            .top_0()
                            .left_0(),
                    ),
            )
            .child(div().w_full().px(px(INSET)).flex().flex_col().child(body))
    }

    fn general(&self, theme: Theme, state: &ViewState, cx: &mut Context<Self>) -> Div {
        let login_on = state.login.on;
        let auto_send = state.auto_send;
        let mut column = div().flex().flex_col().child(group(
            theme,
            vec![
                row(
                    theme,
                    "Launch at login",
                    state.login.available,
                    switch(theme, "settings-login", login_on, state.login.available).when(
                        state.login.available,
                        |d| {
                            d.on_click(cx.listener(move |view, _, _, _| {
                                view.set("launchAtLogin", (!login_on).into());
                            }))
                        },
                    ),
                )
                .into_any_element(),
                row(
                    theme,
                    "Send dev-site captures to OpenCode",
                    true,
                    switch(theme, "settings-auto-send", auto_send, true).on_click(
                        cx.listener(move |view, _, _, _| view.set("autoSend", (!auto_send).into())),
                    ),
                )
                .into_any_element(),
            ],
        ));
        if state.login.needs_approval {
            column = column.child(caption(
                theme,
                if WINDOWS {
                    "Turn Scrawl on in Settings › Apps › Startup."
                } else {
                    "Allow Scrawl in System Settings › General › Login Items."
                },
                false,
            ));
        } else if !state.login.available {
            column = column.child(caption(
                theme,
                "Launch at login works in the installed Scrawl.app.",
                false,
            ));
        }
        for shortcut in &state.shortcuts {
            column = column.child(self.recorder(theme, shortcut, cx));
        }
        column
    }

    fn recorder(&self, theme: Theme, shortcut: &Shortcut, cx: &mut Context<Self>) -> Div {
        let kind = shortcut.kind;
        let recording = shortcut.recording;
        let live = self.live;
        let lit = |on: bool, held: bool| if recording { held } else { on };
        let reset_hidden = shortcut.is_default || recording;
        let off = shortcut.label.is_empty();
        let clear_shown = shortcut.clearable && !off && !recording;
        let caption_text = if recording && shortcut.clearable {
            format!(
                "Press the new shortcut. Esc cancels, {} turns it off.",
                if WINDOWS { "Backspace" } else { "Delete" }
            )
        } else if recording {
            "Press the new shortcut. Esc cancels.".to_owned()
        } else if let Some(error) = &shortcut.error {
            error.clone()
        } else if kind == ShortcutKind::Paste {
            "Pastes the last capture into the app in front.".to_owned()
        } else {
            "Records the screen. Press again to finish.".to_owned()
        };
        let key = if recording {
            "…".to_owned()
        } else {
            shortcut.key.clone()
        };
        // macOS draws ⇧ ⌃ ⌥ ⌘; Windows names its keys, in the order they sit on the keyboard.
        let caps = if off && !recording {
            div()
                .h(px(52.0))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(14.0))
                .text_color(theme.secondary())
                .child("Set shortcut")
        } else if WINDOWS {
            div()
                .flex()
                .gap(px(6.0))
                .child(keycap(
                    theme,
                    "Shift",
                    None,
                    lit(shortcut.shift, live.shift),
                    false,
                ))
                .child(keycap(
                    theme,
                    "Ctrl",
                    None,
                    lit(shortcut.control, live.control),
                    false,
                ))
                .child(keycap(
                    theme,
                    "Win",
                    None,
                    lit(shortcut.command, live.platform),
                    false,
                ))
                .child(keycap(
                    theme,
                    "Alt",
                    None,
                    lit(shortcut.option, live.alt),
                    false,
                ))
                .child(keycap(theme, &key, None, !recording, true))
        } else {
            div()
                .flex()
                .gap(px(6.0))
                .child(keycap(
                    theme,
                    "⇧",
                    None,
                    lit(shortcut.shift, live.shift),
                    false,
                ))
                .child(keycap(
                    theme,
                    "⌃",
                    Some("ctrl"),
                    lit(shortcut.control, live.control),
                    false,
                ))
                .child(keycap(
                    theme,
                    "⌥",
                    Some("opt"),
                    lit(shortcut.option, live.alt),
                    false,
                ))
                .child(keycap(
                    theme,
                    "⌘",
                    Some("cmd"),
                    lit(shortcut.command, live.platform),
                    false,
                ))
                .child(keycap(theme, &key, None, !recording, true))
        };
        let id = |name: &str| SharedString::from(format!("settings-{name}-{kind:?}"));
        div()
            .flex()
            .flex_col()
            .child(
                header(theme, &shortcut.title).child(
                    div()
                        .flex()
                        .gap(px(6.0))
                        .child(
                            div()
                                .id(id("reset"))
                                .size(px(20.0))
                                .rounded_full()
                                .bg(theme.control())
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon(crate::icons::RESET, 10.0, theme.secondary()))
                                .when(reset_hidden, |d| d.opacity(0.0))
                                .when(!reset_hidden, |d| {
                                    d.cursor_pointer().on_click(cx.listener(
                                        move |view, _, _, _| {
                                            view.act(Action::ResetShortcut { kind });
                                        },
                                    ))
                                }),
                        )
                        .when(clear_shown, |d| {
                            d.child(
                                div()
                                    .id(id("clear"))
                                    .size(px(20.0))
                                    .rounded_full()
                                    .bg(theme.control())
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .child(icon(crate::icons::X, 10.0, theme.secondary()))
                                    .on_click(cx.listener(move |view, _, _, _| {
                                        view.act(Action::ClearShortcut { kind });
                                    })),
                            )
                        }),
                ),
            )
            .child(
                div()
                    .id(id("recorder"))
                    .p(px(7.0))
                    .rounded(px(RADIUS + 2.0))
                    .bg(theme.group())
                    .border_1()
                    .border_color(if recording {
                        theme.accent()
                    } else {
                        theme.group_line()
                    })
                    .cursor_pointer()
                    .child(caps)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        cx.stop_propagation();
                        view.menu = None;
                        if recording {
                            view.act(Action::CancelShortcut);
                        } else {
                            window.focus(&view.focus);
                            view.act(Action::RecordShortcut { kind });
                        }
                    })),
            )
            .child(div().h(px(26.0)).child(caption(
                theme,
                &caption_text,
                shortcut.error.is_some() && !recording,
            )))
    }

    fn recording_pane(&self, theme: Theme, state: &ViewState, cx: &mut Context<Self>) -> Div {
        let t = &state.transcriber;
        let hex = t.engine == "hex";
        let local = crate::transcribe::LOCAL_ENGINE_TITLE;
        let engines = vec![
            ("apple".to_owned(), local.to_owned(), true),
            ("hex".to_owned(), t.hex_title.clone(), t.hex_enabled),
        ];
        let chosen = t.models.iter().find(|m| m.chosen);
        let mut rows = vec![
            row(
                theme,
                "Transcription",
                true,
                popup(
                    theme,
                    "settings-transcriber",
                    if hex { "Hex" } else { local },
                )
                .on_click(cx.listener({
                    let selected = t.engine.clone();
                    move |view, event: &ClickEvent, _, cx| {
                        view.open_menu(
                            "transcriber",
                            engines.clone(),
                            Some(selected.clone()),
                            event,
                        );
                        cx.notify();
                    }
                })),
            )
            .into_any_element(),
        ];
        if hex && !t.models.is_empty() {
            let options: Vec<(String, String, bool)> = t
                .models
                .iter()
                .map(|m| (m.id.clone(), m.title.clone(), true))
                .collect();
            let selected = chosen.map(|m| m.id.clone());
            rows.push(
                row(
                    theme,
                    "Hex model",
                    true,
                    popup(
                        theme,
                        "settings-hex-model",
                        chosen.map_or("Choose", |m| m.title.as_str()),
                    )
                    .on_click(cx.listener(
                        move |view, event: &ClickEvent, _, cx| {
                            view.open_menu("hexModel", options.clone(), selected.clone(), event);
                            cx.notify();
                        },
                    )),
                )
                .into_any_element(),
            );
        }
        let note = match (WINDOWS, t.hex_enabled || hex) {
            (true, true) => {
                "Parakeet runs on this PC and times each word. Hex uses a model you have in Hex, and its word times are estimates."
            }
            (true, false) => {
                "Parakeet runs on this PC and times each word. Scrawl downloads its 730 MB model once, unless Hex already has it. Hex shows up here while Hex is running."
            }
            (false, true) => {
                "Apple Speech times each word. Hex uses a model you have in Hex, and its word times are estimates."
            }
            (false, false) => {
                "Apple Speech runs on this Mac and times each word. Hex shows up here while Hex is running."
            }
        };
        let speech = match t.speech.state.as_str() {
            _ if !WINDOWS || hex => None,
            "downloading" => Some((
                format!(
                    "Downloading the speech model: {:.0}%",
                    (t.speech.progress * 100.0).clamp(0.0, 100.0)
                ),
                false,
            )),
            "failed" => Some((
                format!(
                    "The speech model couldn't be downloaded{}. Scrawl tries again in a few minutes.",
                    t.speech
                        .error
                        .as_deref()
                        .map(|e| format!(" ({e})"))
                        .unwrap_or_default()
                ),
                true,
            )),
            _ => None,
        };
        let video = state.video_export;
        div()
            .flex()
            .flex_col()
            .child(group(theme, rows))
            .child(caption(theme, note, false))
            .when_some(speech, |pane, (text, error)| {
                pane.child(caption(theme, &text, error))
            })
            .child(header(theme, "Experimental"))
            .child(group(
                theme,
                vec![
                    row(
                        theme,
                        "Video export",
                        true,
                        switch(theme, "settings-video-export", video, true).on_click(cx.listener(
                            move |view, _, _, _| {
                                view.set("videoExport", (!video).into());
                            },
                        )),
                    )
                    .into_any_element(),
                ],
            ))
            .child(caption(
                theme,
                "Adds Export Last Capture as Video… to the menu.",
                false,
            ))
    }

    fn storage(&self, theme: Theme, state: &ViewState, cx: &mut Context<Self>) -> Div {
        let r = &state.retention;
        let current = r
            .options
            .iter()
            .find(|o| Some(&o.id) == r.preset.as_ref())
            .map_or("Custom", |o| o.label.as_str());
        let options: Vec<(String, String, bool)> = r
            .options
            .iter()
            .map(|o| (o.id.clone(), o.label.clone(), true))
            .collect();
        let selected = r.preset.clone();
        let show = if WINDOWS {
            "Show in File Explorer"
        } else {
            "Show in Finder"
        };
        div()
            .flex()
            .flex_col()
            .child(group(
                theme,
                vec![
                    row(
                        theme,
                        "Keep recordings",
                        true,
                        popup(theme, "settings-retention", current).on_click(cx.listener(
                            move |view, event: &ClickEvent, _, cx| {
                                view.open_menu(
                                    "retention",
                                    options.clone(),
                                    selected.clone(),
                                    event,
                                );
                                cx.notify();
                            },
                        )),
                    )
                    .into_any_element(),
                ],
            ))
            .when(!r.note.is_empty(), |d| {
                d.child(caption(theme, &r.note, false))
            })
            .child(header(theme, "Captures"))
            .child(group(
                theme,
                vec![
                    row(
                        theme,
                        "Captures folder",
                        true,
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .text_color(theme.secondary())
                                    .child(
                                        state
                                            .captures_size
                                            .clone()
                                            .unwrap_or_else(|| "Measuring…".to_owned()),
                                    ),
                            )
                            .child(
                                button(theme, "settings-show-captures", show, true).on_click(
                                    cx.listener(|view, _, _, _| view.act(Action::ShowCaptures)),
                                ),
                            ),
                    )
                    .into_any_element(),
                ],
            ))
    }

    fn permissions(&self, theme: Theme, state: &ViewState, cx: &mut Context<Self>) -> Div {
        let paste = state
            .shortcuts
            .iter()
            .find(|s| s.kind == ShortcutKind::Paste)
            .map_or_else(crate::settings::paste_label, |s| s.label.clone());
        let required: Vec<&PermissionRow> =
            state.permissions.iter().filter(|p| p.required).collect();
        let optional: Vec<&PermissionRow> =
            state.permissions.iter().filter(|p| !p.required).collect();
        let mut column = div().flex().flex_col();
        if !required.is_empty() {
            let required_rows = self.permission_lines(theme, &required, &paste, cx);
            column = column.child(group(theme, required_rows)).child(caption(
                theme,
                if WINDOWS {
                    "Scrawl needs the microphone to hear you. Recording the screen, clicks and shortcuts need no permission on Windows."
                } else {
                    "Scrawl needs these three to record."
                },
                false,
            ));
        }
        if !optional.is_empty() {
            let optional_rows = self.permission_lines(theme, &optional, &paste, cx);
            column = column
                .child(header(theme, "When you use them"))
                .child(group(theme, optional_rows));
        }
        column
    }

    fn permission_lines(
        &self,
        theme: Theme,
        rows: &[&PermissionRow],
        paste: &str,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        rows.iter()
            .map(|row| {
                self.permission_line(theme, row, paste, cx)
                    .into_any_element()
            })
            .collect()
    }

    fn permission_line(
        &self,
        theme: Theme,
        row: &PermissionRow,
        paste: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let permission = row.permission;
        let (name, why, path, tint) = match permission {
            Permission::ScreenRecording => (
                "Screen Recording",
                "To see your screen".to_owned(),
                crate::icons::SCREEN,
                (0x9e80ff, 0x7052eb),
            ),
            Permission::Microphone => (
                "Microphone",
                "To hear you".to_owned(),
                crate::icons::MIC,
                ORANGE,
            ),
            Permission::InputMonitoring => (
                "Input Monitoring",
                "To see clicks and shortcuts".to_owned(),
                crate::icons::CLICK,
                GREY,
            ),
            Permission::Accessibility => (
                "Accessibility",
                format!(
                    "To press {} when you use {paste}",
                    crate::capture::system_paste_label()
                ),
                crate::icons::PERSON,
                BLUE,
            ),
            Permission::Camera => (
                "Camera",
                "To show a cabled iPhone".to_owned(),
                crate::icons::IPHONE,
                GREEN,
            ),
            Permission::SpeechRecognition => (
                "Speech Recognition",
                "To write down what you say".to_owned(),
                crate::icons::MIC,
                ORANGE,
            ),
        };
        let granted = row.status == Status::Granted;
        let id = SharedString::from(format!("settings-grant-{permission:?}"));
        let trailing: AnyElement = if granted && row.relaunch {
            button(theme, id, "Reopen Scrawl", true)
                .on_click(cx.listener(|view, _, _, _| view.act(Action::Reopen)))
                .into_any_element()
        } else if granted {
            div()
                .flex()
                .items_center()
                .gap(px(5.0))
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.secondary())
                .child(icon(crate::icons::CHECK_CIRCLE, 14.0, theme.positive()))
                .child("Allowed")
                .into_any_element()
        } else {
            let title = if row.requested || row.status == Status::Denied {
                "Open Settings"
            } else {
                "Allow"
            };
            button(theme, id, title, true)
                .on_click(cx.listener(move |view, _, _, _| view.act(Action::Grant { permission })))
                .into_any_element()
        };
        div()
            .h(px(52.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(icon_tile(path, tint, 26.0))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .text_color(theme.text())
                            .child(name),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme.secondary())
                            .child(why),
                    ),
            )
            .child(trailing)
    }

    /// The agent skill's install command, with Copy. `scrawl.exe` is a console program, so the
    /// CLI needs no install step on Windows (macOS links it onto PATH here).
    fn agents(&self, theme: Theme, state: &ViewState, cx: &mut Context<Self>) -> Div {
        let command = state.skill_command.clone();
        let line = div()
            .h(px(ROW))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.0))
                    .font_family("Consolas")
                    .text_color(theme.text())
                    .child(command.clone()),
            )
            .child(
                button(
                    theme,
                    "settings-copy-skill",
                    if self.copied { "Copied" } else { "Copy" },
                    true,
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    if crate::handoff::copy_string_to_clipboard(&command).is_ok() {
                        view.copied = true;
                        cx.notify();
                        cx.spawn(async move |view, cx| {
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(1500))
                                .await;
                            let _ = view.update(cx, |view, cx| {
                                view.copied = false;
                                cx.notify();
                            });
                        })
                        .detach();
                    }
                })),
            )
            .into_any_element();
        div()
            .flex()
            .flex_col()
            .child(header(theme, "Agent skill"))
            .child(group(theme, vec![line]))
            .child(caption(
                theme,
                "Run it in a terminal. It teaches Claude Code, OpenCode, Codex and other agents to read your captures and use scrawl.",
                false,
            ))
    }

    fn about(&self, theme: Theme, state: &ViewState, cx: &mut Context<Self>) -> Div {
        let link = |id: &'static str, title: &str, shown: &str, url: &'static str| {
            div()
                .id(id)
                .cursor_pointer()
                .hover(|s| s.bg(theme.ink(0.03)))
                .child(row(
                    theme,
                    title,
                    true,
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .child(
                            div()
                                .text_size(px(14.0))
                                .text_color(theme.secondary())
                                .child(shown.to_owned()),
                        )
                        .child(icon(crate::icons::ARROW_UP_RIGHT, 10.0, theme.tertiary())),
                ))
                .on_click(move |_, _, _| {
                    let _ = crate::handoff::open_path(std::path::Path::new(url));
                })
        };
        let mut column = div().flex().flex_col().child(group(
            theme,
            vec![
                div()
                    .h(px(64.0))
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(img(Arc::clone(&self.icon)).size(px(40.0)).flex_none())
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(1.0))
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.text())
                                    .child("Scrawl"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(theme.secondary())
                                    .child(format!("Version {}", state.version)),
                            ),
                    )
                    .child(
                        button(
                            theme,
                            "settings-updates",
                            "Check for Updates…",
                            state.updates,
                        )
                        .when(state.updates, |d| {
                            d.on_click(
                                cx.listener(|view, _, _, _| view.act(Action::CheckForUpdates)),
                            )
                        }),
                    )
                    .into_any_element(),
            ],
        ));
        if !state.updates {
            column = column.child(caption(
                theme,
                "Updates work in the installed Scrawl.app.",
                false,
            ));
        }
        column.child(header(theme, "Links")).child(group(
            theme,
            vec![
                link(
                    "settings-website",
                    "Website",
                    "scrawl.kitlangton.com",
                    "https://scrawl.kitlangton.com",
                )
                .into_any_element(),
                link(
                    "settings-source",
                    "Source",
                    "github.com/kitlangton/scrawl",
                    "https://github.com/kitlangton/scrawl",
                )
                .into_any_element(),
                row(
                    theme,
                    "Welcome tour",
                    true,
                    button(theme, "settings-tour", "Show", true)
                        .on_click(cx.listener(|view, _, _, _| view.act(Action::Tour))),
                )
                .into_any_element(),
            ],
        ))
    }

    fn open_menu(
        &mut self,
        key: &'static str,
        options: Vec<(String, String, bool)>,
        selected: Option<String>,
        event: &ClickEvent,
    ) {
        self.menu = Some(Menu {
            key,
            options,
            selected,
            at: event.position(),
        });
    }

    /// The open popup's choices, under the pointer, and a catcher for clicks outside it.
    fn menu_layer(
        &self,
        theme: Theme,
        viewport: gpui::Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let menu = self.menu.as_ref()?;
        let item_h = 26.0;
        let w = 220.0;
        let h = menu.options.len() as f32 * item_h + 10.0;
        let x = f32::from(menu.at.x)
            .min(f32::from(viewport.width) - w - 8.0)
            .max(8.0);
        let y = (f32::from(menu.at.y) - item_h * 0.5)
            .min(f32::from(viewport.height) - h - 8.0)
            .max(8.0);
        let key = menu.key;
        let mut list = div()
            .id("settings-menu")
            .occlude()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(w))
            .py(px(5.0))
            .rounded(px(8.0))
            .bg(theme.menu())
            .border_1()
            .border_color(theme.control_line())
            .shadow(vec![BoxShadow {
                color: gpui::hsla(0.0, 0.0, 0.0, 0.3),
                offset: point(px(0.0), px(6.0)),
                blur_radius: px(18.0),
                spread_radius: px(0.0),
            }])
            .flex()
            .flex_col();
        for (index, (id, title, enabled)) in menu.options.iter().enumerate() {
            let chosen = menu.selected.as_ref() == Some(id);
            let id = id.clone();
            let enabled = *enabled;
            list = list.child(
                div()
                    .id(("settings-menu-item", index))
                    .h(px(item_h))
                    .mx(px(5.0))
                    .px(px(8.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(13.0))
                    .text_color(theme.text())
                    .when(!enabled, |d| d.opacity(0.4))
                    .when(enabled, |d| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(theme.accent()).text_color(rgb(0xffffff)))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                cx.stop_propagation();
                                view.menu = None;
                                view.set(key, id.clone().into());
                                cx.notify();
                            }))
                    })
                    .child(div().w(px(14.0)).flex_none().when(chosen, |d| {
                        d.child(icon(crate::icons::CHECK, 12.0, theme.text()))
                    }))
                    .child(title.clone()),
            );
        }
        // The catcher sits under the list, so a press on a choice doesn't close the menu first.
        Some(
            div()
                .id("settings-menu-layer")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(
                    div()
                        .id("settings-menu-catcher")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .occlude()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|view, _, _, cx| {
                                view.menu = None;
                                cx.notify();
                            }),
                        ),
                )
                .child(list),
        )
    }
}

/// The top strip of a column, which moves the window like a title bar.
fn drag_strip(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .w_full()
        .h(px(TITLEBAR))
        .flex_none()
        .when(WINDOWS, |d| {
            d.occlude().window_control_area(WindowControlArea::Drag)
        })
}

/// Windows' minimize and close buttons in the top right corner (macOS has its traffic lights).
fn caption_buttons(theme: Theme) -> Option<Div> {
    if !WINDOWS {
        return None;
    }
    #[cfg(target_os = "windows")]
    let glyphs = if crate::win::shell::windows_11() {
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
            dark: matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        };
        let root = div()
            .id("settings-root")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
                if view.menu.take().is_some() {
                    cx.notify();
                }
                view.key_down(event);
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
            .font_family(".SystemUIFont")
            .text_color(theme.text());
        let Some(state) = self.state.clone() else {
            return root.children(caption_buttons(theme));
        };
        let viewport = window.viewport_size();
        root.child(self.sidebar(theme, cx))
            .child(div().w(px(1.0)).h_full().flex_none().bg(theme.separator()))
            .child(self.content(theme, &state, cx))
            .children(caption_buttons(theme))
            .children(self.menu_layer(theme, viewport, cx))
    }
}
