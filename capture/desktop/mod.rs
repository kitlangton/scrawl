pub(crate) mod demo;
#[cfg(target_os = "macos")]
pub(crate) mod helper;
#[cfg(target_os = "linux")]
pub(crate) use linux::helper;
pub(crate) mod icons;
pub(crate) mod ingest_http;
#[cfg(not(target_os = "macos"))]
pub(crate) mod ink_view;
#[cfg(target_os = "linux")]
pub(crate) mod linux;
pub(crate) mod native;
pub(crate) mod playback;
pub(crate) mod routing;
pub(crate) mod settings;
#[cfg(target_os = "macos")]
pub(crate) mod sparkle;
pub(crate) mod ui;
pub(crate) mod web;
pub(crate) mod workflow;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{cell::RefCell, rc::Rc};

use gpui::{
    AnyElement, App, Application, Bounds, BoxShadow, Context, Div, Entity, FocusHandle, FontWeight,
    Image, ImageFormat, IntoElement, KeyDownEvent, Keystroke, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, SharedString, Size, TextRun, Window,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, div, img, point,
    prelude::*, px, relative, rgb, rgba, size, svg,
};

// Desktop UI implementation and its platform adapters.
#[cfg(target_os = "macos")]
use ui::macos as platform;
#[cfg(target_os = "windows")]
use ui::windows as platform;
#[cfg(target_os = "linux")]
use ui::linux as platform;
use ui::{desktop_completion, lab, motion, pill_theme, recording_completion, whiteboard};
#[cfg(target_os = "linux")]
pub(crate) use platform::set_app_icon;
use platform::{
    PillChrome, install_status_bar, pointer_on_hud_pill, reduce_motion, set_status_inbox_count,
    set_status_text,
};
#[cfg(target_os = "windows")]
pub(crate) use platform::{gpui_display_for, hwnd_of, set_app_icon};

use crate::capture::engine::runtime::audio::audio::{AudioRecorder, InputWarning};
use crate::capture::engine::runtime::{
    LiveOverlayEvent, ScreenCaptureSession, SharedCaptureTelemetry, apparent_log,
    current_mouse_screen, discover_target, install_global_hotkeys, list_selectable_windows,
    record_external_frontmost_app, region_target, selectable_window_at, shortcut_from_key_and_mods,
    start_global_input_tap, update_global_shortcut,
};
use crate::capture::desktop::helper::PhoneDevice;
#[cfg(target_os = "macos")]
use crate::capture::desktop::native::laser_overlay::MetalLaserOverlay as LaserOverlay;
#[cfg(target_os = "linux")]
use crate::capture::desktop::linux::overlay::InkOverlay as LaserOverlay;
use crate::shared::model::{ApparentBundle, CaptureMode, TargetFrame, TargetInfo, format_clock};
use crate::capture::engine::runtime::phone::PhoneSession;
use crate::capture::engine::sketch::timeline;
#[cfg(target_os = "windows")]
use crate::capture::engine::runtime::platform::legacy_windows::capture::overlay::InkOverlay as LaserOverlay;

const SIDEBAR_BG: u32 = 0x111111;
const CANVAS_BG: u32 = 0x161616;
const SURFACE_BG: u32 = 0x1d1d1d;
const SURFACE_HOVER: u32 = 0x252525;
const SURFACE_SELECTED: u32 = 0x2e2e2e;
const LINE_COLOR: u32 = 0x2a2a2a;
const TEXT_COLOR: u32 = 0xf2f2f2;
const TEXT_SOFT: u32 = 0xd0d0d0;
const MUTED_COLOR: u32 = 0x969696;
const ACCENT_COLOR: u32 = 0x3b5cf6;
const ACCENT_HOVER: u32 = 0x4b6bff;
const INK_RED: u32 = 0xff3b30;

// Dynamic Island capsule geometry. GPUI boxes are border-box, so the hairline border is part of
// the inset: R_outer (20) = R_chip (16) + border (1) + pad (3), on every side.
const ISLAND_H: f32 = 40.0;
/// Width the recording pill opens from when a recording starts without the picker.
const ISLAND_OPEN_FROM_W: f32 = 120.0;
const ISLAND_RADIUS: f32 = ISLAND_H / 2.0;
const ISLAND_INSET: f32 = 4.0;
const ISLAND_PAD: f32 = ISLAND_INSET - 1.0;
const CHIP_H: f32 = ISLAND_H - 2.0 * ISLAND_INSET;
const CHIP_RADIUS: f32 = CHIP_H / 2.0;
const CHIP_PX: f32 = 12.0;
const CHIP_GAP: f32 = 2.0;
const LIVE_WARNING_END_PAD: f32 = 16.0;
const DIVIDER_MX: f32 = 6.0;
const LEAD_TRAIL: f32 = 8.0;
const HUD_TEXT: f32 = 13.0;
// Selection-mode segments: line icon, then label.
const SEG_PX: f32 = 10.0;
const SEG_ICON: f32 = 14.0;
const SEG_ICON_GAP: f32 = 5.0;
// The live warning and optional input meter need room without clipping Finish.
const PILL_WINDOW_W: f32 = 640.0;
#[cfg(target_os = "linux")]
const EQUALIZER_W: f32 = 28.0;
#[cfg(target_os = "linux")]
fn recording_equalizer_enabled() -> bool {
    true
}
const PILL_WINDOW_H: f32 = 84.0;

use ui::pill_theme::{PillTheme, SIGNAL, SIGNAL_HOVER, WARNING};

pub use ui::lab::{Options as LabOptions, Scene as LabScene, run as run_lab};
pub(crate) use ui::motion::{TuneAction, tune_command};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Spring {
    /// Visual duration in seconds.
    response: f32,
    bounce: f32,
}

impl Spring {
    /// Kit's duration + bounce: ω = 2π / duration, ζ = 1 − bounce.
    const fn visual(duration: f32, bounce: f32) -> Self {
        Self {
            response: duration,
            bounce,
        }
    }
}

// The capsule's springs and durations live in `motion::Tune` (defaults in `Tune::DEFAULT`, live
// overrides from `~/.config/apparent/tune.json` in debug builds; `apparent tune`).
//
// Switch chips on a tap-started recording (`crate::capture::desktop::settings::shortcut::Reveal`). One shape: the recording
// pill grows into the chip row and folds back. The width leads the morph palette (.42/.16 open,
// .38/.08 close) on a slightly faster spring; the lead (dot + clock) stays pinned to the left
// edge, and each trailing item fades in as the moving edge uncovers it, on the same frame. The
// chips ↔ controls swap fades in place with a sine blur pulse (sharp at both ends) and no content
// scale, so the pinned lead never travels.

/// Which width spring the current morph uses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Morph {
    /// A phase change (`Tune::island`).
    Phase,
    /// The switch chips opening (`Tune::reveal_open`).
    RevealOpen,
    /// The switch chips folding back (`Tune::reveal_close`).
    RevealClose,
    /// A switch chip turning the recording into the picker (`Tune::switch_width`).
    Switch,
}

impl Morph {
    fn spring(self, tune: &motion::Tune) -> Spring {
        match self {
            Self::Phase => tune.island,
            Self::RevealOpen => tune.reveal_open,
            Self::RevealClose => tune.reveal_close,
            Self::Switch => tune.switch_width,
        }
    }
}

/// Reduced motion: no width travel, the groups only cross-fade over this long.
const REDUCED_FADE_S: f32 = 0.18;
/// The picker's highlight moving from one mode to another cross-fades over this long.
const HIGHLIGHT_FADE_S: f32 = 0.18;

/// A switch chip turning the recording into the picker (`switch_recording`). The capsule stays
/// one shape: the chips are the picker's chips, so the picker row starts where they were and
/// slides to its place on `Tune::switch_slide` while the width springs on `Tune::switch_width`
/// (a touch faster, so it leads). The lead (dot and clock, frozen) and its divider ride along in
/// front of the row and leave under the capsule's left edge; Record, the divider and ✕ ride
/// behind it and fade in as the right edge uncovers them.
#[derive(Clone, Copy, Debug)]
struct SwitchMorph {
    /// The row's offset from its resting place and its velocity, in points. Measured by the
    /// first frame that draws it (text widths need the window).
    slide: Option<(f32, f32)>,
}

/// Advances a damped spring with fixed 4ms sub-steps so long frames stay stable.
fn spring_step(value: f32, velocity: f32, target: f32, dt: f32, spring: Spring) -> (f32, f32) {
    let omega = std::f32::consts::TAU / spring.response;
    let stiffness = omega * omega;
    let damping = 2.0 * (1.0 - spring.bounce) * omega;
    let steps = (dt / 0.004).ceil().max(1.0) as usize;
    let h = dt / steps as f32;
    let (mut x, mut v) = (value, velocity);
    for _ in 0..steps {
        v += ((target - x) * stiffness - v * damping) * h;
        x += v * h;
    }
    (x, v)
}

fn with_alpha(color: u32, alpha: f32) -> gpui::Rgba {
    rgba((color << 8) | (alpha.clamp(0.0, 1.0) * 255.0).round() as u32)
}

pub(crate) fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

fn tabular_nums() -> gpui::FontFeatures {
    gpui::FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

fn tabular_font(weight: FontWeight) -> gpui::Font {
    gpui::Font {
        family: ".SystemUIFont".into(),
        features: tabular_nums(),
        fallbacks: None,
        weight,
        style: gpui::FontStyle::Normal,
    }
}

fn measure_text(window: &Window, text: &str, size: f32, weight: FontWeight) -> f32 {
    let mut font = window.text_style().font();
    font.weight = weight;
    font.features = tabular_nums();
    let run = TextRun {
        len: text.len(),
        font,
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(
        SharedString::from(text.to_string()),
        px(size),
        &[run],
        None,
    );
    f32::from(line.width).ceil()
}

fn chip_width(window: &Window, label: &str, weight: FontWeight) -> f32 {
    measure_text(window, label, HUD_TEXT, weight) + 2.0 * CHIP_PX
}

fn clock_template(clock: &str) -> String {
    clock
        .chars()
        .map(|c| if c.is_ascii_digit() { '0' } else { c })
        .collect()
}

/// Lead text of the Completed island: local captures were copied (or sent, as `note` says);
/// iPhone ones count the inbox.
fn completed_label(inbox_count: usize, note: Option<&str>) -> String {
    match inbox_count {
        0 => note.unwrap_or("Copied").to_owned(),
        1 => "From iPhone".to_owned(),
        n => format!("{n} from iPhone"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompletedAudioIssue {
    NoAudio,
    NoWords,
    TranscriptionUnavailable,
}

impl CompletedAudioIssue {
    fn label(self) -> &'static str {
        match self {
            Self::NoAudio => "No audio",
            Self::NoWords => "No words",
            Self::TranscriptionUnavailable => "Transcript unavailable",
        }
    }
}

fn completed_audio_issue(
    source: Option<&str>,
    transcript: &crate::shared::model::TranscriptData,
    audio_saved: bool,
) -> Option<CompletedAudioIssue> {
    if source == Some("iphone") {
        return None;
    }
    if !audio_saved {
        return Some(CompletedAudioIssue::NoAudio);
    }
    if !transcript.text.trim().is_empty() {
        return None;
    }
    if transcript.missing.is_some() || transcript.source == "none" {
        return Some(CompletedAudioIssue::TranscriptionUnavailable);
    }
    Some(CompletedAudioIssue::NoWords)
}

fn bundle_audio_issue(
    dir: &std::path::Path,
    bundle: &ApparentBundle,
) -> Option<CompletedAudioIssue> {
    completed_audio_issue(
        bundle.source.as_deref(),
        &bundle.transcript,
        bundle.has_microphone_samples(dir),
    )
}

fn completed_status_label(
    inbox_count: usize,
    note: Option<&str>,
    issue: Option<CompletedAudioIssue>,
) -> String {
    let label = completed_label(inbox_count, note);
    if let Some(issue) = issue {
        format!("{label} · {}", issue.label())
    } else {
        label
    }
}

/// Inner width of the Selecting row: mode segments, an optional Record chip, divider, and ✕.
fn selecting_row_width(label_widths: &[f32], record_width: Option<f32>) -> f32 {
    let segments: f32 = label_widths
        .iter()
        .map(|label| label + 2.0 * SEG_PX + SEG_ICON + SEG_ICON_GAP)
        .sum();
    let items = label_widths.len() + usize::from(record_width.is_some()) + 2;
    segments
        + record_width.unwrap_or(0.0)
        + 1.0
        + 2.0 * DIVIDER_MX
        + CHIP_H
        + (items - 1) as f32 * CHIP_GAP
}

fn segment_width(window: &Window, mode: SelectionMode) -> f32 {
    measure_text(window, mode.label(), HUD_TEXT, FontWeight::MEDIUM)
        + 2.0 * SEG_PX
        + SEG_ICON
        + SEG_ICON_GAP
}

/// Widths of the items after a Recording capsule's divider: the switch chips while the reveal
/// is open (`crate::capture::desktop::settings::shortcut::Reveal`), else Cancel · Finish.
fn recording_trail_widths(window: &Window, chips: bool, iphone: bool) -> Vec<f32> {
    if chips {
        SelectionMode::available(iphone)
            .iter()
            .map(|&mode| segment_width(window, mode))
            .collect()
    } else {
        let mut widths = vec![chip_width(window, "Cancel", FontWeight::MEDIUM)];
        if whiteboard_enabled() {
            widths.push(chip_width(window, "Whiteboard", FontWeight::MEDIUM));
        }
        widths.push(chip_width(window, "Finish", FontWeight::SEMIBOLD));
        widths
    }
}

fn whiteboard_enabled() -> bool {
    true
}

/// Trace names for a Recording capsule's trailing group, with each item's opacity.
fn trail_keys(
    chips: bool,
    iphone: bool,
    alphas: &[f32],
    prefix: &str,
) -> Vec<Option<(String, f32)>> {
    let names: Vec<String> = if chips {
        SelectionMode::available(iphone)
            .iter()
            .map(|m| format!("chip:{}", m.label()))
            .collect()
    } else {
        let mut names = vec!["cancel".to_string()];
        if whiteboard_enabled() {
            names.push("whiteboard".to_string());
        }
        names.push("finish".to_string());
        names
    };
    names
        .into_iter()
        .enumerate()
        .map(|(i, n)| {
            Some((
                format!("{prefix}{n}"),
                alphas.get(i).copied().unwrap_or(1.0),
            ))
        })
        .collect()
}

/// Left edges of a trailing group's items, starting at `x0`.
fn trail_offsets(x0: f32, widths: &[f32]) -> Vec<f32> {
    widths
        .iter()
        .scan(x0, |x, w| {
            let left = *x;
            *x += w + CHIP_GAP;
            Some(left)
        })
        .collect()
}

/// How much of an item at `x` (width `w`) the capsule's moving right edge has uncovered, eased:
/// trailing items fade in as the pill grows into them and out as it folds over them.
fn edge_alpha(inner_now: f32, x: f32, w: f32) -> f32 {
    let t = ((inner_now - x) / w.max(1.0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The switch chips' clock: the animation clock, so they stay open as long in slow motion
/// (`motion::time_scale`).
fn reveal_clock_ms() -> u64 {
    (motion::now() * 1000.0) as u64
}

/// An item of the picker row sliding in from under the capsule's right edge (`outer`, the
/// shell's outer edge): invisible while the edge would still cut into its middle, opaque once
/// it is clear, so nothing shows half cut.
fn uncover_alpha(outer: f32, x: f32, w: f32) -> f32 {
    let inside = (outer - x) / w.max(1.0);
    let t = ((inside - 0.7) / 0.4).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Intrinsic outer width of the capsule for a phase, mirroring the layout in `HudPillWindow`.
/// `chips`: a Recording capsule showing its switch chips.
fn island_content_width(
    window: &Window,
    phase: IslandPhase,
    clock: &str,
    iphone: bool,
    chips: bool,
    warning: Option<InputWarning>,
    completed_note: Option<&str>,
    completed_issue: Option<CompletedAudioIssue>,
    live_no_words: bool,
) -> f32 {
    let divider = 1.0 + 2.0 * DIVIDER_MX;
    let inner = match phase {
        IslandPhase::Selecting { mode } => {
            let labels: Vec<f32> = SelectionMode::available(iphone)
                .iter()
                .map(|m| measure_text(window, m.label(), HUD_TEXT, FontWeight::MEDIUM))
                .collect();
            let record = mode
                .shows_record()
                .then(|| chip_width(window, "Record", FontWeight::SEMIBOLD));
            selecting_row_width(&labels, record)
        }
        IslandPhase::Recording => {
            let trail = recording_trail_widths(window, chips, iphone);
            let warning_w = warning.map_or(0.0, |warning| {
                measure_text(window, warning.label(), HUD_TEXT, FontWeight::MEDIUM)
                    + 2.0 * CHIP_PX
                    + divider
                    + 2.0 * CHIP_GAP
            });
            #[cfg(target_os = "linux")]
            let meter_w = if recording_equalizer_enabled() {
                EQUALIZER_W + CHIP_GAP
            } else {
                0.0
            };
            #[cfg(not(target_os = "linux"))]
            let meter_w = 0.0;
            CHIP_H
                + measure_text(
                    window,
                    &clock_template(clock),
                    HUD_TEXT,
                    FontWeight::SEMIBOLD,
                )
                + LEAD_TRAIL
                + divider
                + warning_w
                + meter_w
                + trail.iter().sum::<f32>()
                + (trail.len() + 1) as f32 * CHIP_GAP
                + if live_no_words {
                    divider
                        + CHIP_GAP
                        + measure_text(window, "⚠ No words yet", HUD_TEXT, FontWeight::SEMIBOLD)
                        + LIVE_WARNING_END_PAD
                } else {
                    0.0
                }
        }
        IslandPhase::Processing => {
            CHIP_H
                + 6.0
                + measure_text(window, "Processing…", HUD_TEXT, FontWeight::SEMIBOLD)
                + CHIP_PX
        }
        IslandPhase::Completed { inbox_count, .. } => {
            CHIP_H
                + 8.0
                + measure_text(
                    window,
                    &completed_status_label(inbox_count, completed_note, completed_issue),
                    HUD_TEXT,
                    FontWeight::SEMIBOLD,
                )
                + LEAD_TRAIL
                + divider
                + chip_width(window, "Paste", FontWeight::SEMIBOLD)
                + if crate::shared::handoff::experiment(crate::connect::hosted::HOSTED_SHARING) {
                    CHIP_GAP + chip_width(window, "Copy link", FontWeight::MEDIUM)
                } else { 0.0 }
                + chip_width(window, "Inspect", FontWeight::MEDIUM)
                + CHIP_H
                + 3.0 * CHIP_GAP
        }
        IslandPhase::Hidden | IslandPhase::Dismissing => return ISLAND_H,
    };
    inner + 2.0 * ISLAND_INSET
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum MenuAction {
    ToggleCapture,
    GiveFeedback,
    ChooseTarget,
    OpenLatest,
    OpenInbox(PathBuf),
    ExportVideo,
    Settings,
    CheckForUpdates,
    Quit,
}

/// A public issue form, never a URL containing a capture, transcript or local path.
fn feedback_issue_url() -> String {
    "https://github.com/tryapparent/apparent/issues/new?template=bug_report.yml".into()
}

/// Text beside the menu bar icon while a video export runs (`None` goes back to the inbox count).
static EXPORT_STATUS: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
/// Remains visible beside the menu icon until the next recording; a failed capture has no paper tile.
static CAPTURE_STATUS: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static EXPORTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn status_bar_text(export: Option<String>, capture: Option<String>) -> Option<String> {
    export.or(capture)
}

fn current_status_bar_text() -> Option<String> {
    let export = EXPORT_STATUS.lock().ok().and_then(|status| status.clone());
    let capture = CAPTURE_STATUS.lock().ok().and_then(|status| status.clone());
    // Linux retains the last export result in the menu. A capture failure must
    // still be visible when no export is running.
    #[cfg(target_os = "linux")]
    if !EXPORTING.load(std::sync::atomic::Ordering::Acquire) && capture.is_some() {
        return capture;
    }
    status_bar_text(export, capture)
}

fn refresh_status_bar(inbox_count: usize) {
    if let Some(text) = current_status_bar_text() {
        set_status_text(&text);
    } else {
        set_status_inbox_count(inbox_count);
    }
}

/// Export the newest bundle off the main thread. Linux asks for a destination;
/// macOS keeps its automatic Movies/Apparent destination. Report and reveal the result.
fn start_video_export() {
    use std::sync::atomic::Ordering;
    if EXPORTING.swap(true, Ordering::SeqCst) {
        return;
    }
    let say = |text: Option<String>| {
        if let Ok(mut status) = EXPORT_STATUS.lock() {
            *status = text;
        }
    };
    #[cfg(target_os = "linux")]
    say(Some("Choosing export destination…".into()));
    #[cfg(not(target_os = "linux"))]
    say(Some("Exporting…".into()));
    let spawned = std::thread::Builder::new()
        .name("apparent-export".into())
        .spawn(move || {
            let result = crate::shared::handoff::load_bundle(std::path::Path::new("latest")).and_then(
                |(dir, bundle)| {
                    #[cfg(target_os = "linux")]
                    let out = match platform::choose_export_destination(
                        &crate::capture::engine::export::default_output(&dir),
                    )? {
                        Some(path) => Some(path),
                        None => return Ok(None),
                    };
                    #[cfg(not(target_os = "linux"))]
                    let out = None;
                    let options = crate::capture::engine::export::Options {
                        out,
                        background: true,
                        audio: crate::capture::engine::export::AudioPolicy::RecordedIfAvailable,
                        ..Default::default()
                    };
                    crate::capture::engine::export::export(&dir, &bundle, &options, &mut |fraction| {
                        say(Some(format!("Exporting {}%", (fraction * 100.0) as u32)));
                    })
                    .map(Some)
                },
            );
            match result {
                Ok(Some(exported)) => {
                    apparent_log(format!("Exported {}", crate::capture::engine::export::describe(&exported)));
                    say(Some("Exported".into()));
                    #[cfg(target_os = "linux")]
                    crate::capture::desktop::linux::shell::notify(
                        "Video exported",
                        &format!("Saved {}", exported.path.display()),
                    );
                    let _ = crate::shared::handoff::reveal_path(&exported.path);
                }
                Ok(None) => say(Some("Export cancelled".into())),
                Err(error) => {
                    apparent_log(format!("Video export failed: {error:#}"));
                    #[cfg(target_os = "linux")]
                    {
                        let detail = format!("{error:#}");
                        say(Some(format!(
                            "Export failed: {}",
                            detail.chars().take(120).collect::<String>()
                        )));
                        crate::capture::desktop::linux::shell::notify("Video export failed", &detail);
                    }
                    #[cfg(not(target_os = "linux"))]
                    say(Some("Export failed".into()));
                }
            }
            #[cfg(not(target_os = "linux"))]
            {
                std::thread::sleep(Duration::from_secs(4));
                say(None);
            }
            EXPORTING.store(false, Ordering::SeqCst);
        });
    if let Err(error) = spawned {
        #[cfg(target_os = "linux")]
        {
            say(Some("Export failed: could not start worker; retry".into()));
            crate::capture::desktop::linux::shell::notify(
                "Video export failed",
                &format!("Could not start export worker: {error}"),
            );
        }
        #[cfg(not(target_os = "linux"))]
        say(None);
        EXPORTING.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionMode {
    Area,
    Window,
    /// The whole display the picker covers.
    Screen,
    Iphone,
}

impl SelectionMode {
    const ALL: [SelectionMode; 3] = [Self::Area, Self::Screen, Self::Iphone];

    fn key(self) -> &'static str {
        match self {
            Self::Area => "area",
            Self::Window => "window",
            Self::Screen => "screen",
            Self::Iphone => "iphone",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == key.trim())
    }

    fn last_used_path() -> Option<PathBuf> {
        crate::shared::handoff::builtin_captures_dir()
            .parent()
            .map(|dir| dir.join("last-mode"))
    }

    /// The mode the last recording used; the picker opens on it.
    fn load_last_used() -> Self {
        Self::last_used_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| Self::from_key(&text))
            .unwrap_or(Self::Area)
    }

    fn save_last_used(self) {
        if let Some(path) = Self::last_used_path() {
            let _ = std::fs::write(path, self.key());
        }
    }

    /// Modes shown in the HUD, in order; iPhone only while one is cabled.
    pub(crate) fn available(has_iphone: bool) -> &'static [SelectionMode] {
        // A cabled iPhone's screen is an AVFoundation capture device, which only macOS has.
        if has_iphone && cfg!(target_os = "macos") {
            &Self::ALL
        } else {
            &Self::ALL[..2]
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Area => "Area",
            Self::Window => "Window",
            Self::Screen => "Screen",
            Self::Iphone => "iPhone",
        }
    }

    pub(crate) fn icon(self) -> &'static str {
        match self {
            Self::Area => crate::capture::desktop::icons::AREA,
            Self::Window => crate::capture::desktop::icons::WINDOW,
            Self::Screen => crate::capture::desktop::icons::SCREEN,
            Self::Iphone => crate::capture::desktop::icons::IPHONE,
        }
    }

    fn chip_id(self) -> &'static str {
        match self {
            Self::Area => "sel-area",
            Self::Window => "sel-window",
            Self::Screen => "sel-screen",
            Self::Iphone => "sel-iphone",
        }
    }

    /// Screen and iPhone need no pointer gesture, so the HUD offers a Record chip for them.
    pub(crate) fn shows_record(self) -> bool {
        matches!(self, Self::Screen | Self::Iphone)
    }

    /// Digits pick a mode by its position in the HUD.
    fn from_digit(key: &str, has_iphone: bool) -> Option<Self> {
        let index = key.parse::<usize>().ok()?.checked_sub(1)?;
        Self::available(has_iphone).get(index).copied()
    }

    /// The neighbouring mode in HUD order, wrapping at either end.
    fn cycled(self, forward: bool, has_iphone: bool) -> Self {
        let modes = Self::available(has_iphone);
        let Some(index) = modes.iter().position(|m| *m == self) else {
            return if forward {
                modes[0]
            } else {
                modes[modes.len() - 1]
            };
        };
        let next = if forward {
            (index + 1) % modes.len()
        } else {
            (index + modes.len() - 1) % modes.len()
        };
        modes[next]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IslandPhase {
    Hidden,
    Selecting {
        mode: SelectionMode,
    },
    Recording,
    Processing,
    Completed {
        finished_at: Instant,
        frame_count: usize,
        duration_ms: u64,
        /// Unhandled iPhone captures including this one; 0 for a local capture.
        inbox_count: usize,
    },
    Dismissing,
}

impl IslandPhase {
    fn trace_name(self) -> &'static str {
        match self {
            Self::Hidden => "hidden",
            Self::Selecting { .. } => "selecting",
            Self::Recording => "recording",
            Self::Processing => "processing",
            Self::Completed { .. } => "completed",
            Self::Dismissing => "dismissing",
        }
    }
}

enum CaptureSource {
    Screen {
        session: ScreenCaptureSession,
        audio: Option<AudioRecorder>,
        /// Screen mode only: which windows were where, for `apparent autocrop`.
        windows: Option<crate::shared::windows::WindowLogger>,
    },
    /// The helper's mirror window records the phone's native video and the narration.
    Phone(PhoneSession),
    /// `apparent lab pill`: nothing is captured.
    Lab,
}

struct ActiveRecording {
    started: Instant,
    target: TargetInfo,
    bundle_dir: PathBuf,
    source: CaptureSource,
    _metal_overlay: Option<LaserOverlay>,
}

impl ActiveRecording {
    fn mic_warning(&self) -> Option<InputWarning> {
        match &self.source {
            CaptureSource::Screen {
                audio: Some(audio), ..
            } => audio.warning(),
            CaptureSource::Screen { audio: None, .. } => Some(InputWarning::Unavailable),
            CaptureSource::Phone(_) | CaptureSource::Lab => None,
        }
    }

    fn elapsed_ms(&self) -> u64 {
        match &self.source {
            CaptureSource::Screen { session, .. } => session.elapsed_ms(),
            CaptureSource::Phone(session) => session.elapsed_ms(),
            CaptureSource::Lab => self.started.elapsed().as_millis() as u64,
        }
    }
}

/// Shared coordinator owned by the GPUI app.
pub struct ApparentCoordinator {
    telemetry: Arc<SharedCaptureTelemetry>,
    whiteboard: Rc<RefCell<whiteboard::Board>>,
    active_recording: Option<ActiveRecording>,
    /// Identifies the recording whose finalization may update the current UI.
    capture_generation: u64,
    /// Media workers outlive their capsule and still own unfinished bundles.
    pending_finalizers: usize,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    agent_capture_id: Option<String>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    agent_captures: crate::capture::desktop::workflow::agent_capture::Registry,
    #[cfg(target_os = "linux")]
    agent_finishing: bool,
    last_live_no_words: bool,
    playback: Option<crate::capture::desktop::playback::Guard>,
    picker_metal_overlay: Option<LaserOverlay>,
    native_pill: Option<crate::capture::desktop::native::pill::NativePill>,
    iphone_preview: Option<PhoneSession>,
    island_phase: IslandPhase,
    island_prev_phase: IslandPhase,
    /// `motion::now()` when the phase last changed.
    island_phase_since: f64,
    island_dismissed_from: IslandPhase,
    island_width: f32,
    island_target_width: f32,
    island_velocity: f32,
    island_scale: f32,
    island_scale_velocity: f32,
    island_y: f32,
    island_y_velocity: f32,
    island_opacity: f32,
    /// Which width spring the current morph uses.
    island_morph: Morph,
    /// Switch chips on a tap-started recording (`crate::capture::desktop::settings::shortcut::Reveal`).
    reveal: crate::capture::desktop::settings::shortcut::Reveal,
    /// When the chips last opened or folded, for the in-place swap of the trailing group.
    reveal_since: f64,
    /// The trailing group fading out after that change: `Some(true)` the chips, `Some(false)` the
    /// controls, `None` when the capsule opened straight into its state (nothing to swap).
    reveal_outgoing: Option<bool>,
    /// The chips were open when the island last changed phase, so a Recording capsule that is
    /// blurring out or dismissing keeps them.
    reveal_prev_chips: bool,
    /// macOS Reduce motion, read when a reveal starts.
    reduced_motion: bool,
    switch: Option<SwitchMorph>,
    /// When the springs were last advanced (`step_island`).
    island_stepped: Instant,
    /// A new capsule window has drawn its first frame and not its second (which comes late):
    /// only its frames advance the springs, so the hitch slows the opening instead of eating it.
    island_await_frame: bool,
    /// The recording clock last shown, so the lead keeps its time while it leaves.
    last_elapsed_ms: u64,
    /// The highlighted mode before the last change and when it changed (`HIGHLIGHT_FADE_S`).
    highlight_prev: Option<SelectionMode>,
    highlight_since: f64,
    selectable_windows: Vec<TargetInfo>,
    selection_generation: u64,
    windows_loading: bool,
    selection_pointer: Option<[f32; 2]>,
    hovered_window: Option<TargetInfo>,
    animated_window_rect: Option<[f32; 4]>,
    selection_drag: Option<([f32; 2], [f32; 2])>,
    /// Global frame of the display the picker covers, highlighted in `SelectionMode::Screen`.
    display_rect: [f32; 4],
    paste_drag_start: Option<[f32; 2]>,
    recording_shortcut_key: bool,
    review_bundle: Option<(PathBuf, ApparentBundle)>,
    /// Read the WAV once when selecting a bundle, never on the capsule's animation tick.
    review_audio_issue: Option<CompletedAudioIssue>,
    review_status: Option<String>,
    feedback_capture: bool,
    review_feedback: bool,
    selected_tab: ReviewTab,
    selected_keyframe: usize,
    /// A cabled iPhone, refreshed each time the picker opens.
    iphone: Option<PhoneDevice>,
    /// The bundle the Completed island is showing.
    completed_dir: Option<PathBuf>,
    /// The Completed island's lead text.
    completed_note: Option<String>,
    share_online: bool,
    share_probe_at: Option<Instant>,
    share_pending: Option<(PathBuf, u64, std::sync::Arc<std::sync::atomic::AtomicBool>)>,
    share_waiting_account: bool,
    share_login: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    share_copied: Option<PathBuf>,
    /// When this Mac last finished a capture, so ⌥⇧V prefers it over older inbox items.
    last_local_capture_ms: u64,
    inbox: crate::capture::desktop::workflow::inbox::Inbox,
    /// The mode the last recording used; the picker opens on it.
    last_mode: SelectionMode,
    /// The picker opened on Area because the last recording was the iPhone and no phone had been
    /// seen yet: switch to the phone when discovery finds one, unless Kit picked a mode himself.
    open_iphone_when_found: bool,
    /// The welcome window, while it's open (`crate::onboarding`).
    onboarding: Option<crate::capture::desktop::settings::onboarding::Session>,
    onboarding_editor: crate::capture::desktop::settings::Panel,
    /// The Settings window, while it's open (`crate::settings`).
    settings: Option<crate::capture::desktop::settings::Session>,
    /// The last fresh read of what macOS has granted (at launch, then while onboarding is open).
    grants: Option<crate::capture::desktop::settings::onboarding::Grants>,
    /// Holds the permissions window until the shortcut's modifiers are up.
    gate: crate::capture::desktop::settings::onboarding::Gate,
    /// ⌥⇧S waiting for its keys to come up (`crate::shortcut`).
    shortcut: crate::capture::desktop::settings::shortcut::Shortcut,
    /// The modifiers held when the shortcut went down; releasing any of them starts the recording.
    hold_mods: u64,
    /// `apparent lab pill`: recordings are pretend (`CaptureSource::Lab`), the picker takes no
    /// input and opens no overlay, and nothing looks for windows or phones.
    lab: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewTab {
    ContactSheet,
    Keyframes,
}

impl ApparentCoordinator {
    fn new(
        telemetry: Arc<SharedCaptureTelemetry>,
        initial_bundle: Option<(PathBuf, ApparentBundle)>,
    ) -> Self {
        crate::capture::desktop::playback::warm_up();
        Self {
            telemetry,
            whiteboard: Rc::new(RefCell::new(whiteboard::Board::default())),
            active_recording: None,
            capture_generation: 0,
            pending_finalizers: 0,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            agent_capture_id: None,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            agent_captures: crate::capture::desktop::workflow::agent_capture::Registry::default(),
            #[cfg(target_os = "linux")]
            agent_finishing: false,
            last_live_no_words: false,
            playback: None,
            picker_metal_overlay: None,
            native_pill: None,
            iphone_preview: None,
            island_phase: IslandPhase::Hidden,
            island_prev_phase: IslandPhase::Hidden,
            island_phase_since: motion::now(),
            island_dismissed_from: IslandPhase::Hidden,
            island_width: 205.0,
            island_target_width: 205.0,
            island_velocity: 0.0,
            island_scale: 0.92,
            island_scale_velocity: 0.0,
            island_y: -6.0,
            island_y_velocity: 0.0,
            island_opacity: 0.0,
            island_morph: Morph::Phase,
            reveal: crate::capture::desktop::settings::shortcut::Reveal::default(),
            reveal_since: motion::now(),
            reveal_outgoing: None,
            reveal_prev_chips: false,
            reduced_motion: false,
            switch: None,
            island_stepped: Instant::now(),
            island_await_frame: false,
            last_elapsed_ms: 0,
            highlight_prev: None,
            highlight_since: motion::now(),
            selectable_windows: Vec::new(),
            selection_generation: 0,
            windows_loading: false,
            selection_pointer: None,
            hovered_window: None,
            animated_window_rect: None,
            selection_drag: None,
            display_rect: [0.0, 0.0, 1440.0, 900.0],
            paste_drag_start: None,
            recording_shortcut_key: false,
            review_audio_issue: initial_bundle
                .as_ref()
                .and_then(|(dir, bundle)| bundle_audio_issue(dir, bundle)),
            review_bundle: initial_bundle,
            review_status: None,
            feedback_capture: false,
            review_feedback: false,
            selected_tab: ReviewTab::Keyframes,
            selected_keyframe: 0,
            iphone: None,
            completed_dir: None,
            completed_note: None,
            share_online: false,
            share_probe_at: None,
            share_pending: None,
            share_waiting_account: false,
            share_login: None,
            share_copied: None,
            last_local_capture_ms: 0,
            inbox: crate::capture::desktop::workflow::inbox::Inbox::load_default(),
            last_mode: SelectionMode::load_last_used(),
            open_iphone_when_found: false,
            onboarding: None,
            onboarding_editor: crate::capture::desktop::settings::Panel::default(),
            settings: None,
            grants: None,
            gate: crate::capture::desktop::settings::onboarding::Gate::default(),
            shortcut: crate::capture::desktop::settings::shortcut::Shortcut::default(),
            hold_mods: 0,
            lab: false,
        }
    }

    /// Remembers the mode a recording is starting in, so the next picker opens on it.
    fn remember_mode(&mut self, mode: SelectionMode) {
        if self.last_mode != mode {
            self.last_mode = mode;
            mode.save_last_used();
        }
    }

    /// Kit opened, copied, pasted or dismissed `dir`, so it leaves the inbox.
    fn mark_handled(&mut self, dir: &std::path::Path) {
        if self.inbox.remove(dir) {
            refresh_status_bar(self.inbox.len());
        }
    }

    /// What ⌥⇧V pastes: the bundle on the Completed island, else the newest unhandled iPhone
    /// capture unless this Mac captured something since, else the latest bundle.
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

    pub fn paste_latest(&mut self, cx: &mut Context<Self>) {
        // Without Accessibility macOS drops the posted ⌘V (onboarding asks for it, but it can be
        // turned off later); bring the permissions window instead.
        if !crate::capture::engine::runtime::accessibility_granted() {
            apparent_log("⌥⇧V without Accessibility; asking for it");
            self.gate
                .defer(vec![crate::capture::desktop::settings::onboarding::Permission::Accessibility]);
            return;
        }
        let dir = self.take_paste_target();
        if matches!(self.island_phase, IslandPhase::Completed { .. }) {
            self.dismiss_island(cx);
        }
        std::thread::spawn(move || {
            if let Err(err) = crate::shared::handoff::paste_bundle_image_and_text(&dir) {
                apparent_log(format!("paste_bundle_image_and_text error: {err}"));
                #[cfg(target_os = "linux")]
                crate::capture::desktop::linux::shell::notify(
                    "Paste stopped",
                    &format!(
                        "{err:#}. Manually attach the contact sheet and paste summary.md from {}",
                        dir.display()
                    ),
                );
            }
        });
    }

    /// Looks for a cabled iPhone in the background; the picker shows "iPhone" when one is there.
    fn refresh_iphone(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |entity, cx| {
            let found = cx
                .background_executor()
                .spawn(async {
                    crate::capture::desktop::helper::devices()
                        .inspect_err(|error| apparent_log(format!("iPhone discovery: {error:#}")))
                        .ok()
                        .and_then(|devices| devices.into_iter().next())
                })
                .await;
            let _ = entity.update(cx, |coord, cx| {
                if coord.iphone != found {
                    coord.iphone = found;
                    cx.notify();
                }
                if std::mem::take(&mut coord.open_iphone_when_found)
                    && coord.iphone.is_some()
                    && matches!(
                        coord.island_phase,
                        IslandPhase::Selecting {
                            mode: SelectionMode::Area | SelectionMode::Iphone
                        }
                    )
                {
                    coord.preview_iphone(cx);
                }
            });
        })
        .detach();
    }

    pub fn is_recording(&self) -> bool {
        self.active_recording.is_some()
    }

    pub fn is_processing(&self) -> bool {
        matches!(self.island_phase, IslandPhase::Processing)
    }

    fn refuse_capture_restart(&self) -> bool {
        if self.capture_busy() {
            let message = "Finish or cancel the capture before restarting Apparent.";
            apparent_log(message);
            if let Ok(mut status) = CAPTURE_STATUS.lock() { *status = Some(message.into()); }
            true
        } else { false }
    }

    fn request_quit(&mut self, cx: &mut Context<Self>) {
        crate::capture::desktop::workflow::shutdown::request();
        self.lab = false;
        self.recording_shortcut_key = false;
        self.iphone_preview.take();
        if self.is_recording() {
            self.finish_recording(cx);
        } else if !self.is_processing() {
            self.cancel_recording(cx);
        }
        self.publish_capture_busy();
        if !self.capture_busy() {
            cx.quit();
        }
    }

    /// Native callbacks cannot borrow the GPUI entity. This is only a copy of the
    /// coordinator-owned admission decision, never separate lifecycle accounting.
    fn publish_capture_busy(&self) {
        crate::capture::desktop::workflow::shutdown::set_phase_busy(self.capture_busy());
    }

    pub fn is_selecting(&self) -> bool {
        matches!(self.island_phase, IslandPhase::Selecting { .. })
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn agent_state(&self) -> crate::capture::desktop::workflow::agent_capture::State {
        use crate::capture::desktop::workflow::agent_capture::{Phase, State};
        if matches!(
            self.active_recording.as_ref().map(|recording| &recording.source),
            Some(CaptureSource::Lab)
        ) {
            return State {
                phase: Phase::Idle,
                capture_id: None,
            };
        }
        let phase = if let Some(recording) = &self.active_recording {
            match &recording.source {
                CaptureSource::Screen { session, .. } if !session.has_frames() => Phase::Starting,
                CaptureSource::Phone(session) if !session.has_recording() => Phase::Starting,
                _ => Phase::Recording,
            }
        } else {
            match self.island_phase {
                IslandPhase::Selecting { .. } => Phase::Selecting,
                IslandPhase::Processing => Phase::Processing,
                IslandPhase::Completed { .. } => Phase::Completed,
                _ => Phase::Idle,
            }
        };
        State {
            phase,
            capture_id: self.agent_capture_id.clone(),
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn agent_request(&mut self, request: crate::capture::desktop::workflow::agent_capture::Request, cx: &mut Context<Self>) {
        use crate::capture::desktop::workflow::agent_capture::{Action, Outcome};
        // Report an ended sampler, but never discard another session as a side
        // effect of State, Inspect or a competing Start. Cancellation stays explicit.
        if matches!(self.active_recording.as_ref().map(|r| &r.source),
            Some(CaptureSource::Screen { session, .. }) if session.failed_to_start()) {
            if let Some(id) = &self.agent_capture_id {
                self.agent_captures.set(&id, "failed", None, Some("start_failed"));
            }
        }
        let before = self.agent_state();
        if before.phase == crate::capture::desktop::workflow::agent_capture::Phase::Recording
            && let Some(id) = &before.capture_id {
            self.agent_captures.set(id, "recording", None, None);
        }
        if let Some((Action::Start, id)) = &request.action {
            let occupied = self.lab || self.active_recording.is_some()
                || self.is_selecting() || self.is_processing()
                || self.iphone_preview.is_some() || self.agent_capture_id.is_some()
                || self.recording_shortcut_key || crate::capture::desktop::workflow::shutdown::requested();
            if self.agent_captures.admit(id, occupied, request.expired()) {
                self.agent_capture_id = Some(id.clone());
                self.publish_capture_busy();
                self.record_screen_now(cx);
                if self.active_recording.is_none() {
                    self.agent_capture_id = None;
                    self.agent_captures.set(id, "failed", None, Some("start_failed"));
                }
            }
            let _ = request.reply.send(self.agent_captures.inspect(id).to_string());
            return;
        }
        if let Some((Action::Inspect, id)) = &request.action {
            let _ = request.reply.send(self.agent_captures.inspect(id).to_string());
            return;
        }
        if request.action.is_some() && request.expired() {
            let _ = request.reply.send(serde_json::json!({"error": "admission_expired", "state": before}).to_string());
            return;
        }
        let outcome = request
            .action
            .as_ref()
            .map(|(action, id)| self.agent_captures.decide(&before, *action, id));
        match (&request.action, outcome) {
            (Some((Action::Stop, _)), Some(Outcome::Stopping)) => {
                #[cfg(target_os = "linux")]
                { self.agent_finishing = true; }
                self.finish_recording(cx)
            }
            (Some((Action::Cancel, _)), Some(Outcome::Cancelled)) => self.cancel_recording(cx),
            _ => {}
        }
        let response = match outcome {
            Some(outcome) => {
                let mut response = self.agent_captures.inspect(&request.action.as_ref().unwrap().1);
                // Preserve non-wait Stop/Cancel's existing outcome semantics.
                response["outcome"] = serde_json::json!(outcome);
                response["state"] = serde_json::json!(self.agent_state());
                if outcome == Outcome::AlreadyEnded && response["status"] == "unknown" {
                    response["error"] = serde_json::Value::Null;
                }
                response
            },
            None => serde_json::json!({"state": before}),
        };
        let _ = request.reply.send(response.to_string());
    }

    pub fn open_onboarding(&mut self, flow: crate::capture::desktop::settings::onboarding::Flow) {
        // Drop the old window before the new one opens.
        self.cancel_onboarding_editor();
        self.onboarding = None;
        let sender = self.telemetry.ui_sender.clone();
        let events = move |event| {
            let _ = sender.send(LiveOverlayEvent::Onboarding(event));
        };
        match crate::capture::desktop::settings::onboarding::Session::open(flow, &self.shortcut_label(), events) {
            Ok(session) => {
                self.onboarding = Some(session);
                self.sync_onboarding_editor();
            }
            Err(error) => apparent_log(format!("onboarding could not open: {error:#}")),
        }
    }

    /// Opens Settings, or brings it forward on `pane`.
    pub fn open_settings(&mut self, pane: Option<crate::capture::desktop::settings::Pane>) {
        if let Some(session) = self.settings.as_mut() {
            session.show(pane);
            return;
        }
        let sender = self.telemetry.ui_sender.clone();
        let events = move |event| {
            let _ = sender.send(LiveOverlayEvent::Settings(event));
        };
        match crate::capture::desktop::settings::Session::open(pane, self.grants.clone(), events) {
            Ok(mut session) => {
                session.sync(&crate::capture::desktop::settings::AppHost {
                    telemetry: &self.telemetry,
                });
                self.settings = Some(session);
            }
            Err(error) => apparent_log(format!("Settings could not open: {error:#}")),
        }
    }

    fn settings_event(&mut self, event: crate::capture::desktop::settings::Event, cx: &mut Context<Self>) {
        use crate::capture::desktop::settings::{Effect, Event};
        #[cfg(target_os = "linux")]
        if self.defer_update_restart(&event) {
            // Check before Session spawns its relaunch helper. Capture admission
            // and this decision run on the same UI thread.
            apparent_log("Update restart deferred until capture is idle");
            crate::capture::desktop::linux::shell::notify("Update ready", "Finish or cancel the capture before restarting Apparent.");
            return;
        }
        if let Event::Grants(grants) = &event {
            self.note_grants(grants);
        }
        let Some(session) = self.settings.as_mut() else {
            return;
        };
        let mut host = crate::capture::desktop::settings::AppHost {
            telemetry: &self.telemetry,
        };
        let effect = session.event(event, &mut host);
        let shortcut = self.shortcut_label();
        if let Some(onboarding) = self.onboarding.as_mut() {
            onboarding.shortcut_changed(&shortcut);
        }
        match effect {
            Effect::Close => {
                self.settings = None;
                self.cancel_share_login();
                if let Some((_, _, active)) = self.share_pending.take() {
                    active.store(false, std::sync::atomic::Ordering::SeqCst);
                }
                self.share_waiting_account = false;
            }
            Effect::ConnectHosted if self.share_waiting_account => self.reserve_completed_link(true, cx),
            Effect::ConnectHosted => {
                if self.share_login.is_none() && crate::shared::handoff::experiment(crate::connect::hosted::HOSTED_SHARING) {
                    let active = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
                    self.share_login = Some(active.clone());
                    cx.spawn(async move |entity, cx| {
                        let worker_active = active.clone();
                        let result = cx.background_executor().spawn(async move {
                            crate::connect::hosted::connect_sharing(&|| worker_active.load(std::sync::atomic::Ordering::SeqCst))
                        }).await;
                        let _ = entity.update(cx, |coord, _| {
                            if coord.finish_share_login(&active, result.is_ok()) { crate::connect::hosted::start_worker(); }
                        });
                    }).detach();
                }
            }
            Effect::ManageSharedLinks => {
                if let Some(url) = crate::connect::hosted::shared_links_url() {
                    let _ = crate::shared::handoff::open_path(std::path::Path::new(&url));
                }
            }
            Effect::Tour => self.open_onboarding(crate::capture::desktop::settings::onboarding::Flow::tour(
                crate::capture::desktop::settings::onboarding::Step::Welcome,
                self.grants.clone(),
            )),
            Effect::Relaunch => {
                if self.refuse_capture_restart() { return; }
                self.settings = None;
                crate::capture::desktop::settings::onboarding::relaunch();
                cx.quit();
            }
            Effect::QuitForUpdate => {
                self.settings = None;
                cx.quit();
            }
            _ => {}
        }
    }

    #[cfg(target_os = "linux")]
    fn defer_update_restart(&self, event: &crate::capture::desktop::settings::Event) -> bool {
        matches!(event, crate::capture::desktop::settings::Event::Action(crate::capture::desktop::settings::Action::UpdateNow))
            && self.capture_busy()
    }

    /// UI-thread snapshot for restart/termination admission, not just visible UI.
    pub(crate) fn capture_busy(&self) -> bool {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if self.agent_capture_id.is_some() { return true; }
        self.pending_finalizers > 0 || self.lab || self.active_recording.is_some()
            || self.is_selecting() || self.is_processing() || self.iphone_preview.is_some()
            || self.recording_shortcut_key
    }

    fn capture_finalizer_started(&mut self) {
        self.pending_finalizers += 1;
    }

    fn capture_finalizer_finished(&mut self) {
        self.pending_finalizers = self.pending_finalizers.checked_sub(1)
            .expect("finalizer completion must own a pending worker");
    }

    fn cancel_share_login(&mut self) {
        if let Some(active) = self.share_login.take() {
            active.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }

    fn finish_share_login(&mut self, active: &std::sync::Arc<std::sync::atomic::AtomicBool>, succeeded: bool) -> bool {
        // A cancelled request can finish after Settings has started another login.
        // Only its owner may clear the cancellation handle or start the worker.
        if !self.share_login.as_ref().is_some_and(|current| std::sync::Arc::ptr_eq(current, active)) {
            return false;
        }
        self.share_login = None;
        succeeded && active.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Keeps a fresh read of the grants and remembers what was granted. (The key tap follows
    /// Accessibility by itself, in the HID poller.)
    fn note_grants(&mut self, grants: &crate::capture::desktop::settings::onboarding::Grants) {
        use crate::capture::desktop::settings::onboarding::Progress;
        let mut progress = Progress::load();
        if progress.note(grants) {
            progress.save();
        }
        self.grants = Some(grants.clone());
    }

    /// Required permissions that keep a capture from starting now.
    fn missing_to_record(&mut self) -> Vec<crate::capture::desktop::settings::onboarding::Permission> {
        use crate::capture::desktop::settings::onboarding::{Permission, missing_to_record};
        let screen = crate::capture::engine::runtime::screen_capture_usable(false);
        let accessibility = crate::capture::engine::runtime::accessibility_granted();
        let missing = missing_to_record(self.grants.as_ref(), screen, accessibility);
        // The last read may be old; read again before standing in the way.
        if missing.contains(&Permission::Microphone)
            && let Some(fresh) = crate::capture::desktop::settings::onboarding::read_grants()
        {
            self.note_grants(&fresh);
            return missing_to_record(self.grants.as_ref(), screen, accessibility);
        }
        missing
    }

    /// Whether this may go on: with a required permission off it starts nothing, and the
    /// permissions window comes forward once the modifiers are up.
    fn ready_to_record(&mut self) -> bool {
        if self.lab {
            return true;
        }
        let missing = self.missing_to_record();
        if missing.is_empty() {
            return true;
        }
        apparent_log(format!(
            "Not recording: {missing:?} off; opening onboarding"
        ));
        self.gate.defer(missing);
        false
    }

    /// Whether `permission`, asked for on first use (Camera, Accessibility), is on. If not, its
    /// window comes forward once the modifiers are up.
    fn ready_for(&mut self, permission: crate::capture::desktop::settings::onboarding::Permission) -> bool {
        let granted = |g: &crate::capture::desktop::settings::onboarding::Grants| g.granted(permission);
        if self.lab || self.grants.as_ref().is_none_or(granted) {
            return true;
        }
        if let Some(fresh) = crate::capture::desktop::settings::onboarding::read_grants() {
            self.note_grants(&fresh);
            if granted(&fresh) {
                return true;
            }
        }
        apparent_log(format!("{permission:?} is off; asking for it"));
        self.gate.defer(vec![permission]);
        false
    }

    /// Polled while the gate holds a request: shows the permissions window once ⌥⇧ is up.
    pub fn tick_gate(&mut self) {
        use crate::capture::desktop::settings::onboarding::{Flow, Input, Mode, Permission};
        let Some(asks) = self.gate.tick(crate::capture::engine::runtime::modifier_flags() != 0) else {
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

    fn onboarding_event(&mut self, event: crate::capture::desktop::settings::onboarding::Event, cx: &mut Context<Self>) {
        use crate::capture::desktop::settings::onboarding::{Effect, Event, Progress};
        let effect = match event {
            Event::Launch(grants) => {
                self.note_grants(&grants);
                let mut progress = Progress::load();
                let plan = crate::capture::desktop::settings::onboarding::launch_plan(&progress, &grants);
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
                // A single app-wide Mac poller feeds both windows and the capture gate.
                #[cfg(target_os = "macos")]
                if let Some(session) = self.settings.as_mut() {
                    session.event(
                        crate::capture::desktop::settings::Event::Grants(grants.clone()),
                        &mut crate::capture::desktop::settings::AppHost {
                            telemetry: &self.telemetry,
                        },
                    );
                }
                self.onboarding
                    .as_mut()
                    .map_or(Effect::None, |session| session.grants(grants))
            }
            Event::Action(action) if action == "use-desktop-shortcut" => {
                let host = crate::capture::desktop::settings::AppHost { telemetry: &self.telemetry };
                if self.onboarding_editor.record_shortcut(&host).externally_managed
                    && let Some(session) = self.onboarding.as_mut()
                {
                    session.input(crate::capture::desktop::settings::onboarding::Input::ShortcutChosen);
                }
                Effect::None
            }
            Event::Action(action) => self
                .onboarding
                .as_mut()
                .map_or(Effect::None, |session| session.action(&action)),
            Event::Shortcut(action) => {
                if self
                    .onboarding
                    .as_ref()
                    .is_some_and(|session| session.editing_shortcut())
                    && matches!(
                        action,
                        crate::capture::desktop::settings::Action::Shortcut {
                            kind: crate::capture::engine::runtime::ShortcutKind::Record,
                            ..
                        } | crate::capture::desktop::settings::Action::ResetShortcut {
                            kind: crate::capture::engine::runtime::ShortcutKind::Record
                        } | crate::capture::desktop::settings::Action::CancelShortcut
                    )
                {
                    let mut host = crate::capture::desktop::settings::AppHost {
                        telemetry: &self.telemetry,
                    };
                    let choosing = matches!(action, crate::capture::desktop::settings::Action::Shortcut { .. })
                        && self.onboarding_editor.record_shortcut(&host).recording;
                    self.onboarding_editor.on(action, &mut host);
                    let editor = self.onboarding_editor.record_shortcut(&host);
                    if choosing && !editor.recording && editor.error.is_none()
                        && let Some(session) = self.onboarding.as_mut()
                    {
                        session.input(crate::capture::desktop::settings::onboarding::Input::ShortcutChosen);
                    }
                    self.sync_onboarding_editor();
                }
                Effect::None
            }
            Event::Exited => {
                if let Some(session) = self.onboarding.as_ref() {
                    session.leave(true);
                }
                self.cancel_onboarding_editor();
                self.onboarding = None;
                Effect::None
            }
        };
        match effect {
            Effect::EditShortcut => {
                let mut host = crate::capture::desktop::settings::AppHost {
                    telemetry: &self.telemetry,
                };
                self.onboarding_editor.on(
                    crate::capture::desktop::settings::Action::RecordShortcut {
                        kind: crate::capture::engine::runtime::ShortcutKind::Record,
                    },
                    &mut host,
                );
                self.sync_onboarding_editor();
            }
            Effect::Close | Effect::Settings => {
                if let Some(session) = self.onboarding.as_ref() {
                    session.leave(false);
                }
                self.cancel_onboarding_editor();
                if effect == Effect::Settings {
                    self.open_settings(None);
                }
                self.onboarding = None;
            }
            Effect::Relaunch { resume } => {
                if self.refuse_capture_restart() { return; }
                self.cancel_onboarding_editor();
                apparent_log(format!(
                    "Screen Recording granted but not usable here; relaunching (resume {resume:?})"
                ));
                let mut progress = Progress::load();
                progress.relaunching(resume, crate::capture::desktop::settings::onboarding::unix_ms());
                progress.save();
                self.onboarding = None;
                crate::capture::desktop::settings::onboarding::relaunch();
                cx.quit();
            }
            _ => {}
        }
        if self
            .onboarding
            .as_ref()
            .is_some_and(|session| !session.editing_shortcut())
        {
            self.cancel_onboarding_editor();
        }
    }

    fn sync_onboarding_editor(&mut self) {
        use crate::capture::desktop::settings::Host as _;
        let host = crate::capture::desktop::settings::AppHost {
            telemetry: &self.telemetry,
        };
        if let Some(session) = self.onboarding.as_mut() {
            session.shortcut_changed(&host.shortcuts().record.label);
            session.shortcut_editor(&self.onboarding_editor.record_shortcut(&host));
        }
    }

    fn cancel_onboarding_editor(&mut self) {
        let mut host = crate::capture::desktop::settings::AppHost {
            telemetry: &self.telemetry,
        };
        self.onboarding_editor
            .on(crate::capture::desktop::settings::Action::CancelShortcut, &mut host);
    }

    fn activity(&self) -> crate::capture::desktop::settings::shortcut::Activity {
        use crate::capture::desktop::settings::shortcut::Activity;
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
        if !self.lab && self.playback.is_none() {
            self.playback = crate::capture::desktop::playback::Guard::start();
        }
    }

    /// The capture shortcut went down (from Carbon, the event tap or the HID poller).
    pub fn press_shortcut(&mut self, cx: &mut Context<Self>) {
        let activity = self.activity();
        if self.gate.is_pending() {
            return;
        }
        if !self.shortcut.is_pending() && activity == crate::capture::desktop::settings::shortcut::Activity::Free {
            // No picker or overlay before the permissions are on: they sit above
            // macOS's permission prompts and take the clicks meant for them.
            let missing = self.missing_to_record();
            if !self.gate.press(missing) {
                apparent_log("Shortcut held back: a required permission is off");
                return;
            }
            self.begin_playback();
            self.hold_mods = crate::capture::engine::runtime::modifier_flags();
            // An Esc from before this press doesn't cancel it.
            self.telemetry.take_escape();
        }
        let action = self.shortcut.press(activity);
        self.apply_shortcut(action, cx);
    }

    /// Polled from the UI loop while a press is pending: watches for ⌥⇧ coming up and Esc,
    /// whichever app or window has focus.
    pub fn tick_shortcut(&mut self, cx: &mut Context<Self>) {
        if !self.shortcut.is_pending() {
            return;
        }
        let action = if self.telemetry.take_escape() {
            self.shortcut.escape()
        } else {
            let mods = crate::capture::engine::runtime::modifier_flags();
            let held = self.hold_mods != 0 && mods & self.hold_mods == self.hold_mods;
            self.shortcut.tick(held)
        };
        self.apply_shortcut(action, cx);
    }

    fn apply_shortcut(&mut self, action: crate::capture::desktop::settings::shortcut::Action, cx: &mut Context<Self>) {
        use crate::capture::desktop::settings::shortcut::Action;
        if action != Action::None {
            apparent_log(format!("shortcut -> {action:?}"));
        }
        match action {
            Action::None => {}
            Action::Cancel => {
                if let Some(session) = self.onboarding.as_mut() {
                    session.input(crate::capture::desktop::settings::onboarding::Input::CaptureEnded { completed: false });
                }
                // Esc can cancel before the picker or a recording exists.
                self.playback.take();
            }
            Action::RecordScreen => {
                if let Some(session) = self.onboarding.as_mut() {
                    session.input(crate::capture::desktop::settings::onboarding::Input::ShortcutTriggered);
                }
                self.record_screen_now(cx);
            }
            Action::RecordSelected => {
                if let Some(session) = self.onboarding.as_mut() {
                    session.input(crate::capture::desktop::settings::onboarding::Input::ShortcutTriggered);
                }
                self.record_selected_mode(cx);
            }
            Action::Finish => self.finish_recording(cx),
        }
    }

    /// The menu's Record Screen item, the review window's Record button and `app --record`.
    pub fn record_or_finish(&mut self, cx: &mut Context<Self>) {
        if self.is_recording() {
            self.finish_recording(cx);
        } else if self.is_selecting() {
            self.record_selected_mode(cx);
        } else if !self.is_processing() {
            self.record_screen_now(cx);
        }
    }

    /// The feedback entry point records immediately, then opens a local review.
    pub fn record_feedback(&mut self, cx: &mut Context<Self>) {
        if self.is_recording() || self.is_selecting() || self.is_processing() {
            return;
        }
        self.record_screen_now(cx);
        self.feedback_capture = self.is_recording();
    }

    /// Records the display under the cursor straight away (no picker), auto-cropped.
    pub fn record_screen_now(&mut self, cx: &mut Context<Self>) {
        self.feedback_capture = false;
        let target = if self.lab {
            lab::target()
        } else {
            discover_target(None)
        };
        self.start_recording_with_target(target, cx);
        if self.is_recording() {
            self.show_reveal(cx);
        }
    }

    /// Activity that means the user got to work: clicks, drags, scrolls and strokes. Clicks on
    /// Apparent's own windows (the capsule and its chips) are never recorded, so they don't count.
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

    /// The recording just started from a tap: the capsule opens onto the switch chips.
    fn show_reveal(&mut self, cx: &mut Context<Self>) {
        // The iPhone chip shows only while one is cabled (a USB registry read, no prompt).
        if !self.lab {
            self.refresh_iphone(cx);
        }
        self.reduced_motion = reduce_motion();
        let activity = self.reveal_activity();
        self.reveal.show(reveal_clock_ms(), activity);
        self.reveal_since = motion::now();
        self.reveal_outgoing = None;
        self.island_morph = Morph::RevealOpen;
        if self.reduced_motion {
            self.island_scale = 1.0;
            self.island_scale_velocity = 0.0;
            self.island_y = 0.0;
            self.island_y_velocity = 0.0;
        }
        self.sync_native_pill();
        cx.notify();
    }

    /// The chips fold back into the recording pill (timeout, work started, or a chip chosen).
    fn fold_reveal(&mut self) {
        if self.reveal.is_open() {
            self.reveal.close();
            self.play_fold();
        }
    }

    fn play_fold(&mut self) {
        self.reveal_since = motion::now();
        self.reveal_outgoing = Some(true);
        self.island_morph = Morph::RevealClose;
        self.sync_native_pill();
    }

    /// A switch chip on a tap-started recording. Screen keeps it. Anything else drops the few
    /// seconds recorded so far and starts over on the new target, exactly as if it had been
    /// chosen first: Window and Area morph the capsule into the picker (hover and click a window,
    /// or drag an area), iPhone goes straight into the phone's mirror.
    pub fn switch_recording(&mut self, mode: SelectionMode, cx: &mut Context<Self>) {
        if mode == SelectionMode::Window {
            self.report_window_recording_disabled(cx);
            return;
        }
        if !self.is_recording() {
            return;
        }
        if mode == SelectionMode::Screen {
            self.fold_reveal();
            cx.notify();
            return;
        }
        apparent_log(format!("switch_recording -> {mode:?}"));
        // The morph continues the chips, so it needs them on screen.
        let chips_shown = self.reveal.is_open();
        let Some((_, overlay)) = self.discard_recording() else {
            return;
        };
        if mode == SelectionMode::Iphone {
            // Recording → Recording: the chips fold into the pill as the phone starts.
            self.fold_reveal();
            drop(overlay);
            self.start_recording_iphone(cx);
            return;
        }
        // Window and Area: the chips stay up while the capsule morphs into the picker.
        // The recording's overlay becomes the picker's (moved if the picker opens on another
        // display), so the border never swaps windows (as the picker → recording path does).
        self.picker_metal_overlay = overlay;
        self.open_picker(Some(mode), cx);
        if self.is_selecting() && chips_shown && !self.reduced_motion {
            // The chips showed Screen; the picker shows `mode`.
            self.highlight_prev = Some(SelectionMode::Screen);
            self.highlight_since = motion::now();
            self.island_morph = Morph::Switch;
            self.switch = Some(SwitchMorph { slide: None });
        }
    }

    /// Stops the active recording and deletes its bundle, leaving the island as it is. Returns
    /// its target and overlay.
    fn discard_recording(&mut self) -> Option<(TargetInfo, Option<LaserOverlay>)> {
        if let Some(rec) = &self.active_recording
            && !matches!(rec.source, CaptureSource::Lab)
            && let Err(error) = crate::capture::engine::bundle::recover::cancel(&rec.bundle_dir)
        {
            apparent_log(format!("Could not persist cancellation; capture is still active: {error:#}"));
            return None;
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if let Some(id) = self.agent_capture_id.take() {
            self.agent_captures.set(&id, "cancelled", None, None);
        }
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
        if let Err(error) = crate::capture::engine::bundle::recover::delete_cancelled(&rec.bundle_dir) {
            apparent_log(format!("Cancelled assets could not be deleted at {}: {error:#}; recovery remains forbidden", rec.bundle_dir.display()));
        }
        Some((rec.target, rec._metal_overlay))
    }

    pub fn is_island_visible(&self) -> bool {
        !matches!(self.island_phase, IslandPhase::Hidden)
    }

    pub fn shortcut_label(&self) -> String {
        self.telemetry
            .shortcut
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .label
            .clone()
    }

    pub fn set_review_bundle(&mut self, bundle: (PathBuf, ApparentBundle), status: Option<String>) {
        self.review_audio_issue = bundle_audio_issue(&bundle.0, &bundle.1);
        self.review_bundle = Some(bundle);
        self.review_status = status;
        self.review_feedback = false;
        self.selected_tab = ReviewTab::Keyframes;
        self.selected_keyframe = 0;
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

    fn report_window_recording_disabled(&self, cx: &mut Context<Self>) {
        let message = crate::capture::engine::runtime::WINDOW_RECORDING_DISABLED;
        apparent_log(message);
        if let Ok(mut status) = CAPTURE_STATUS.lock() {
            *status = Some(message.into());
        }
        cx.notify();
    }

    pub fn set_selection_mode(&mut self, mode: SelectionMode, cx: &mut Context<Self>) {
        if mode == SelectionMode::Window {
            self.report_window_recording_disabled(cx);
            return;
        }
        self.open_iphone_when_found = false;
        if let IslandPhase::Selecting { mode: prev_mode } = self.island_phase {
            if prev_mode != mode {
                // Crisp tactile Dynamic Island scale micro-pulse when Space toggles camera/crosshair mode.
                self.island_scale_velocity += 1.25;
            }
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
            cx.notify();
        }
    }

    /// Opens the live iPhone mirror window in preview mode so the user can see their phone before
    /// starting a recording. Clicking `iPhone` a second time or clicking `Record` starts recording.
    pub fn preview_iphone(&mut self, cx: &mut Context<Self>) {
        if self.is_recording() || self.is_processing() {
            return;
        }
        apparent_log("preview_iphone invoked");
        // The mirror would raise the camera prompt under the picker, which takes its clicks.
        if !self.ready_for(crate::capture::desktop::settings::onboarding::Permission::Camera) {
            self.cancel_recording(cx);
            return;
        }
        self.set_selection_mode(SelectionMode::Iphone, cx);
        if let Some(ref mut preview) = self.iphone_preview {
            preview.show_preview();
        } else {
            let dev_id = self.iphone.as_ref().map(|d| d.id.as_str());
            match PhoneSession::start_preview(dev_id, false, Some(Arc::clone(&self.telemetry))) {
                Ok(session) => {
                    self.iphone_preview = Some(session);
                }
                Err(error) => {
                    apparent_log(format!("iPhone preview could not start: {error:#}"));
                    self.set_selection_mode(SelectionMode::Area, cx);
                    return;
                }
            }
        }
        self.sync_picker_overlay();
        cx.notify();
    }

    pub fn toggle_selection_mode(&mut self, cx: &mut Context<Self>) {
        if let IslandPhase::Selecting { mode } = self.island_phase {
            let next = match mode {
                SelectionMode::Area => SelectionMode::Screen,
                SelectionMode::Window | SelectionMode::Screen | SelectionMode::Iphone => {
                    SelectionMode::Area
                }
            };
            self.set_selection_mode(next, cx);
        }
    }

    fn selection_mode(&self) -> Option<SelectionMode> {
        match self.island_phase {
            IslandPhase::Selecting { mode } => Some(mode),
            _ => None,
        }
    }

    /// Switches the picker to `mode`; choosing iPhone opens its live preview.
    pub fn choose_selection_mode(&mut self, mode: SelectionMode, cx: &mut Context<Self>) {
        if self.selection_mode().is_none() {
            return;
        }
        if mode == SelectionMode::Iphone {
            self.preview_iphone(cx);
        } else {
            self.set_selection_mode(mode, cx);
        }
    }

    /// A HUD segment click: picks the mode, or records when Screen/iPhone is already selected.
    pub fn click_selection_mode(&mut self, mode: SelectionMode, cx: &mut Context<Self>) {
        if self.selection_mode() == Some(mode) && mode.shows_record() {
            self.record_selected_mode(cx);
        } else {
            self.choose_selection_mode(mode, cx);
        }
    }

    /// Return, `s`, the capture shortcut, or Record: iPhone records the phone, anything else
    /// records the full screen.
    pub fn record_selected_mode(&mut self, cx: &mut Context<Self>) {
        match self.selection_mode() {
            Some(SelectionMode::Iphone) => self.start_recording_iphone(cx),
            Some(_) => self.start_recording_full_screen(cx),
            None => {}
        }
    }

    /// Picker keyboard map, shared by the picker window and the HUD pill. Returns whether the
    /// key was handled.
    /// ⌘Q and ⌘, in Apparent's own windows: a menu bar (accessory) app has no main menu to
    /// carry them (Ctrl+Q and Ctrl+, on Windows).
    pub fn handle_app_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        let mods = keystroke.modifiers;
        // `secondary` is ⌘ on macOS and Ctrl elsewhere; the other of the two must be up.
        if !mods.secondary() || (mods.control && mods.platform) || mods.alt || mods.shift {
            return false;
        }
        match keystroke.key.as_str() {
            "q" => self.request_quit(cx),
            "," => {
                if self.is_selecting() {
                    self.cancel_recording(cx);
                }
                self.open_settings(None);
            }
            _ => return false,
        }
        true
    }

    /// `repeat`: the key is held and repeating, which never records.
    pub fn handle_selection_key(
        &mut self,
        keystroke: &Keystroke,
        repeat: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(mode) = self.selection_mode() else {
            return false;
        };
        let mods = keystroke.modifiers;
        // Esc cancels even with ⌥⇧ still held from the shortcut.
        if keystroke.key == "escape" {
            self.cancel_recording(cx);
            return true;
        }
        if self.handle_app_key(keystroke, cx) {
            return true;
        }
        // The shortcut itself, if it reaches a picker window instead of Carbon.
        if let Some(cfg) = shortcut_from_key_and_mods(
            &keystroke.key,
            mods.platform,
            mods.control,
            mods.alt,
            mods.shift,
        ) && self.telemetry.matches_shortcut(
            cfg.key_code as u16,
            mods.platform,
            mods.control,
            mods.alt,
            mods.shift,
        ) {
            if !repeat {
                self.telemetry.emit_capture_shortcut();
            }
            return true;
        }
        if mods.platform || mods.control || mods.alt {
            return false;
        }
        let has_iphone = self.iphone.is_some();
        // The record shortcut's key ("S") also records on its own while the picker is up.
        let label = self.shortcut_label();
        let record_key = crate::capture::engine::runtime::shortcut_parts(&label).1;
        let records = |key: &str| key == "enter" || key.eq_ignore_ascii_case(record_key);
        match keystroke.key.as_str() {
            key if repeat && records(key) => {}
            "space" | " " => self.toggle_selection_mode(cx),
            "escape" => self.cancel_recording(cx),
            "enter" => self.record_selected_mode(cx),
            key if key.eq_ignore_ascii_case(record_key) => self.record_selected_mode(cx),
            "tab" => self.choose_selection_mode(mode.cycled(!mods.shift, has_iphone), cx),
            "right" => self.choose_selection_mode(mode.cycled(true, has_iphone), cx),
            "left" => self.choose_selection_mode(mode.cycled(false, has_iphone), cx),
            key => match SelectionMode::from_digit(key, has_iphone) {
                Some(next) => self.choose_selection_mode(next, cx),
                None => return false,
            },
        }
        true
    }

    pub fn undo_stroke(&mut self, cx: &mut Context<Self>) {
        let mut strokes = self
            .telemetry
            .strokes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(idx) = strokes.iter().rposition(|s| !s.laser) {
            strokes.remove(idx);
        }
        drop(strokes);
        cx.notify();
    }

    pub fn request_immediate_frame(&self) {
        if let Some(ActiveRecording {
            source: CaptureSource::Screen { session, .. },
            ..
        }) = &self.active_recording
        {
            session.request_immediate_frame();
        }
    }

    /// Moves the island to `phase`, triggering a `.blurReplace` cross-transition on the inner content
    /// while the outer shell springs its width smoothly (`stiffness: 400, damping: 30` equivalent).
    fn set_island_phase(&mut self, phase: IslandPhase) {
        use crate::capture::desktop::settings::onboarding::Input;
        if matches!(phase, IslandPhase::Recording | IslandPhase::Hidden | IslandPhase::Dismissing) {
            if let Some((_, _, active)) = self.share_pending.take() {
                active.store(false, std::sync::atomic::Ordering::SeqCst);
            }
            self.share_waiting_account = false;
            self.share_copied = None;
            self.share_probe_at = None;
            // A new recording or dismissal revokes detached workers' UI ownership.
            // Their finalized bundles remain on disk; stale results must not copy to
            // the clipboard, replace the current review, or finish a newer practice.
            self.capture_generation = self.capture_generation.wrapping_add(1);
        }
        let capture = match (self.island_phase, phase) {
            (IslandPhase::Recording, IslandPhase::Recording) => None,
            (_, IslandPhase::Recording) => Some(Input::CaptureStarted),
            (IslandPhase::Recording | IslandPhase::Processing, IslandPhase::Completed { .. }) => {
                Some(Input::CaptureEnded { completed: true })
            }
            // Startup can fail before Recording is presented. Clear the shortcut attempt
            // then too, so a later menu capture cannot inherit it.
            (_, IslandPhase::Hidden | IslandPhase::Dismissing) => {
                Some(Input::CaptureEnded { completed: false })
            }
            _ => None,
        };
        if let Some(input) = capture
            && let Some(session) = self.onboarding.as_mut()
        {
            session.input(input);
        }
        let changed = std::mem::discriminant(&self.island_phase) != std::mem::discriminant(&phase);
        if changed {
            // A new phase replaces the whole content; the chips go with it, and the outgoing
            // Recording content keeps showing them while it blurs away.
            self.reveal_prev_chips = self.reveal.is_open();
            self.reveal.close();
            self.reveal_outgoing = None;
            self.island_morph = Morph::Phase;
            self.switch = None;
            self.highlight_prev = None;
            if matches!(self.island_phase, IslandPhase::Hidden) {
                self.island_stepped = Instant::now();
            }
            self.island_prev_phase = self.island_phase;
            if matches!(phase, IslandPhase::Dismissing) {
                self.island_dismissed_from = self.island_phase;
            } else if matches!(self.island_phase, IslandPhase::Hidden) {
                self.island_scale = 0.92;
                self.island_scale_velocity = 0.0;
                self.island_y = -6.0;
                self.island_y_velocity = 0.0;
            }
            self.island_phase_since = motion::now();
            motion::trace_event(format!("phase:{}", phase.trace_name()));
        }
        let appearing = matches!(
            self.island_phase,
            IslandPhase::Hidden | IslandPhase::Dismissing
        );
        self.island_phase = phase;
        self.publish_capture_busy();
        let place = changed
            && (appearing
                || matches!(
                    phase,
                    IslandPhase::Selecting { .. } | IslandPhase::Recording
                ));
        if crate::shared::handoff::native_pill() {
            if !matches!(phase, IslandPhase::Hidden) {
                self.ensure_native_pill();
            }
            if place {
                self.place_capsule();
            }
            self.sync_native_pill();
        } else if place {
            self.place_capsule();
        }
    }

    /// Puts the capsule on the pointer's display during screen recording or selection,
    /// else the one under the pointer (the phone mirror places its capsule separately).
    fn place_capsule(&self) {
        let centre = |[x, y, w, h]: [f64; 4]| [x + w / 2.0, y + h / 2.0];
        let anchor = match self.island_phase {
            IslandPhase::Recording => self
                .active_recording
                .as_ref()
                .filter(|rec| rec.target.mode != CaptureMode::Device)
                .map(|_| centre(self.display_rect.map(f64::from))),
            IslandPhase::Selecting { .. } => Some(centre(self.display_rect.map(f64::from))),
            _ => None,
        };
        let Some(point) = anchor.or_else(crate::capture::engine::runtime::screens::pointer) else {
            return;
        };
        match self.native_pill {
            Some(ref pill) => pill.place_on(point),
            None => platform::place_hud_pill(point),
        }
    }

    /// The iPhone's mirror window can be dragged to another display mid-recording: the ink
    /// overlay and the capsule go with it.
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
        if crate::capture::engine::runtime::screens::contains(overlay.screen(), centre)
            || !overlay.move_to([f.x, f.y, f.width, f.height])
        {
            return;
        }
        self.display_rect = overlay.screen().map(|v| v as f32);
        if let Some(ref pill) = self.native_pill {
            pill.place_on(centre);
        }
    }

    /// Follow the pointer with Linux controls, not with the capture source. On Mac,
    /// a dragged capsule must stay where it was dropped for the whole recording.
    #[cfg(target_os = "linux")]
    fn follow_recording_pointer(&mut self) {
        if !matches!(self.island_phase, IslandPhase::Recording)
            || !self
                .active_recording
                .as_ref()
                .is_some_and(|rec| rec.target.mode != CaptureMode::Device)
        {
            return;
        }
        let Some(pointer) = current_mouse_screen() else {
            return;
        };
        let current = self.display_rect.map(f64::from);
        if crate::capture::engine::runtime::screens::contains(current, pointer.map(f64::from)) {
            return;
        }
        let screens = crate::capture::engine::runtime::screens::current();
        if let Some(frame) =
            recording_pointer_destination(current, pointer.map(f64::from), &screens)
        {
            if let Some(overlay) = self
                .active_recording
                .as_mut()
                .and_then(|rec| rec._metal_overlay.as_mut())
            {
                overlay.move_to(frame);
            }
            self.display_rect = frame.map(|v| v as f32);
            // A manually dragged Linux capsule remains where it was dropped.
            // Continue following with the ink overlay, but leave the pill alone.
            if !platform::pill_user_moved() {
                self.place_capsule();
            }
        }
    }

    /// While picking, the picker (its window, overlay and the capsule) follows the pointer to
    /// whichever display it's on, unless an area is being dragged (an area stays on one display).
    fn follow_pointer_display(&mut self, pointer: [f32; 2], cx: &mut Context<Self>) {
        // The iPhone's mirror window stays where it is, and so does the highlight around it.
        if self.selection_drag.is_some()
            || self.lab
            || self.selection_mode() == Some(SelectionMode::Iphone)
        {
            return;
        }
        let p = pointer.map(f64::from);
        let current = self.display_rect.map(f64::from);
        if crate::capture::engine::runtime::screens::contains(current, p) {
            return;
        }
        let screens = crate::capture::engine::runtime::screens::current();
        let Some(i) = crate::capture::engine::runtime::screens::index_at(&screens, p) else {
            return;
        };
        let frame = screens[i].frame;
        if (0..4).all(|k| (frame[k] - current[k]).abs() < 1.0) {
            return;
        }
        apparent_log(format!("picker follows the pointer to display {frame:?}"));
        self.display_rect = frame.map(|v| v as f32);
        if let Some(ref mut overlay) = self.picker_metal_overlay {
            overlay.move_to(frame);
        }
        platform::move_picker_window(frame, cx);
        self.place_capsule();
        self.sync_picker_overlay();
        // The picker draws relative to `display_rect`; with the native capsule nothing else
        // redraws it, and a same-size display doesn't resize its window.
        cx.notify();
    }

    /// The picker's mouse-up, inside its window or not: ends an area drag.
    fn picker_mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        let [sx, sy] = current_mouse_screen()
            .unwrap_or([f32::from(event.position.x), f32::from(event.position.y)]);
        if let Some((start, _)) = self.selection_drag.take() {
            if let Some([x, y, w, h]) = area_rect(start, [sx, sy], self.display_rect) {
                let target = region_target(x, y, w, h, &self.selectable_windows);
                self.start_recording_with_target(target, cx);
            } else {
                self.sync_picker_overlay();
                cx.notify();
            }
        }
    }

    pub(crate) fn ensure_native_pill(&mut self) {
        if !crate::shared::handoff::native_pill() || self.native_pill.is_some() {
            return;
        }
        match crate::capture::desktop::native::pill::NativePill::open() {
            Ok(pill) => {
                if !crate::shared::handoff::experiment("movableRecordingControls") {
                    self.telemetry.excluded_window_ids.lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(pill.window_number());
                }
                self.native_pill = Some(pill);
            }
            Err(err) => {
                apparent_log(format!("NativePill::open error: {err}"));
            }
        }
    }

    fn completed_audio_issue(&self) -> Option<CompletedAudioIssue> {
        match (&self.completed_dir, &self.review_bundle) {
            (Some(dir), Some((review_dir, _))) if dir == review_dir => self.review_audio_issue,
            _ => None,
        }
    }

    fn recording_has_no_words(&self) -> bool {
        matches!(
            self.active_recording.as_ref().map(|rec| &rec.source),
            Some(CaptureSource::Screen { audio: Some(audio), .. })
                if audio.live_word_state() == Some(crate::capture::engine::runtime::audio::transcription::live_word_warning::State::NoWords)
        )
    }

    fn pill_snapshot(&self) -> crate::capture::desktop::native::pill::PillSnapshot {
        use crate::capture::desktop::native::pill::{PillSnapshot, PillStyle, model::Pres};
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
            IslandPhase::Recording if self.recording_has_no_words() && !self.reveal.is_open() => {
                Pres::RecordingWarning {
                    chips: false,
                    has_iphone,
                }
            }
            IslandPhase::Recording => Pres::Recording {
                chips: self.reveal.is_open(),
                has_iphone,
            },
            IslandPhase::Processing => Pres::Processing,
            IslandPhase::Completed { inbox_count, .. } => {
                let issue = self.completed_audio_issue();
                Pres::Completed {
                    share: crate::shared::handoff::experiment(crate::connect::hosted::HOSTED_SHARING).then_some(self.share_online && self.share_pending.is_none()),
                    warning: issue.is_some(),
                    label: completed_status_label(
                        inbox_count,
                        self.completed_note.as_deref(),
                        issue,
                    ),
                    routes: Vec::new(),
                }
            }
        };
        PillSnapshot {
            pres,
            clock,
            mic_warning: self
                .active_recording
                .as_ref()
                .and_then(ActiveRecording::mic_warning),
            reduced_motion: self.reduced_motion,
            style: PillStyle::current(),
        }
    }

    pub(crate) fn sync_native_pill(&mut self) {
        if self.native_pill.is_some() {
            let snap = self.pill_snapshot();
            if let Some(ref pill) = self.native_pill {
                pill.sync(snap);
            }
        }
    }

    fn apply_pill_action(
        &mut self,
        action: crate::capture::desktop::native::pill::PillAction,
        cx: &mut Context<Self>,
    ) {
        use crate::capture::desktop::native::pill::PillAction;
        match action {
            PillAction::SelectMode(mode) => self.click_selection_mode(mode, cx),
            PillAction::SwitchRecording(mode) => self.switch_recording(mode, cx),
            PillAction::RecordSelected => self.record_selected_mode(cx),
            PillAction::Cancel => self.cancel_recording(cx),
            PillAction::Whiteboard => {
                let coord = cx.entity().clone();
                cx.defer(move |cx| whiteboard::open(&coord, cx));
            }
            PillAction::Finish => self.finish_recording(cx),
            PillAction::Paste(target) => self.paste_completed_bundle(target, cx),
            PillAction::CopyLink => self.copy_completed_link(cx),
            PillAction::Inspect => self.inspect_completed_bundle(cx),
            PillAction::DismissCompleted => self.dismiss_completed_bundle(cx),
            PillAction::Route(_) => {}
        }
    }

    pub fn dismiss_island(&mut self, cx: &mut Context<Self>) {
        if !self.is_recording() {
            self.playback.take();
        }
        if matches!(
            self.island_phase,
            IslandPhase::Hidden | IslandPhase::Dismissing
        ) {
            return;
        }
        self.telemetry
            .selecting
            .store(false, std::sync::atomic::Ordering::Release);
        self.telemetry.set_picker_rect(None);
        self.picker_metal_overlay = None;
        self.iphone_preview = None;
        self.selection_drag = None;
        self.hovered_window = None;
        self.animated_window_rect = None;
        self.set_island_phase(IslandPhase::Dismissing);
        cx.defer(close_picker_window);
        cx.notify();
    }

    pub fn dismiss_completed_bundle(&mut self, cx: &mut Context<Self>) {
        if let Some(dir) = self.completed_dir.clone() {
            self.mark_handled(&dir);
        }
        self.dismiss_island(cx);
    }

    pub fn inspect_completed_bundle(&mut self, cx: &mut Context<Self>) {
        if let Some(dir) = self.completed_dir.clone() {
            self.mark_handled(&dir);
        }
        self.set_island_phase(IslandPhase::Hidden);
        self.island_opacity = 0.0;
        let entity = cx.entity().clone();
        cx.defer(move |cx| {
            close_capture_windows(cx);
            open_review_window(&entity, cx);
        });
        cx.notify();
    }

    fn tick_hosted_sharing(&mut self, cx: &mut Context<Self>) {
        if !crate::shared::handoff::experiment(crate::connect::hosted::HOSTED_SHARING)
            || !matches!(self.island_phase, IslandPhase::Completed { .. })
            || self.share_probe_at.is_some_and(|at| at.elapsed() < Duration::from_secs(5)) {
            return;
        }
        self.share_probe_at = Some(Instant::now());
        let generation = self.capture_generation;
        let copied = self.share_copied.clone();
        cx.spawn(async move |entity, cx| {
            let (online, progress) = cx.background_executor().spawn(async move {
                (crate::connect::hosted::sharing_online(), copied.as_deref().map(crate::connect::hosted::sharing_progress))
            }).await;
            let _ = entity.update(cx, |coord, cx| {
                if coord.capture_generation != generation || !matches!(coord.island_phase, IslandPhase::Completed { .. }) { return; }
                coord.share_online = online;
                if coord.share_pending.is_none() {
                    if let Some(progress) = progress {
                        coord.completed_note = Some(progress);
                    } else if !online {
                        coord.completed_note = Some(if crate::connect::hosted::sharing_server().is_none() {
                            "Configure Hosted account in Settings"
                        } else { "Connect to the internet to share" }.into());
                    } else if coord.completed_note.as_deref() == Some("Connect to the internet to share") {
                        coord.completed_note = None;
                    }
                }
                coord.sync_native_pill();
                cx.notify();
            });
        }).detach();
    }

    pub fn copy_completed_link(&mut self, cx: &mut Context<Self>) {
        if !crate::shared::handoff::experiment(crate::connect::hosted::HOSTED_SHARING)
            || !matches!(self.island_phase, IslandPhase::Completed { .. })
            || !self.share_online || self.share_pending.is_some() { return; }
        let Some(bundle) = self.completed_dir.clone() else { return };
        self.share_pending = Some((bundle, self.capture_generation, std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true))));
        if !crate::connect::hosted::sharing_connected() {
            self.share_waiting_account = true;
            self.completed_note = Some("Connect Hosted account to share".into());
            self.open_settings(Some(crate::capture::desktop::settings::Pane::General));
        } else {
            self.reserve_completed_link(false, cx);
        }
        self.sync_native_pill();
        cx.notify();
    }

    fn reserve_completed_link(&mut self, connect: bool, cx: &mut Context<Self>) {
        let Some((bundle, generation, active)) = self.share_pending.clone() else { return };
        self.share_waiting_account = false;
        self.completed_note = Some(if connect { "Approve hosted upload in your browser…" } else { "Reserving link…" }.into());
        cx.spawn(async move |entity, cx| {
            let worker_bundle = bundle.clone();
            let worker_active = active.clone();
            let result = cx.background_executor().spawn(async move {
                let allowed = || worker_active.load(std::sync::atomic::Ordering::SeqCst);
                if connect { crate::connect::hosted::connect_sharing(&allowed)?; }
                crate::connect::hosted::copy_link_handoff(&worker_bundle, &allowed)
            }).await;
            let _ = entity.update(cx, |coord, cx| {
                if !active.load(std::sync::atomic::Ordering::SeqCst)
                    || !coord.share_pending.as_ref().is_some_and(|(_, _, current)| std::sync::Arc::ptr_eq(current, &active))
                    || coord.capture_generation != generation
                    || coord.completed_dir.as_ref() != Some(&bundle)
                    || !matches!(coord.island_phase, IslandPhase::Completed { .. }) { return; }
                coord.share_pending = None;
                coord.completed_note = Some(match result {
                    Ok(link) if link.authorized() => match crate::shared::handoff::copy_string_to_clipboard(&link.text) {
                        Ok(()) => { coord.share_copied = Some(bundle.clone()); crate::connect::hosted::sharing_progress(&bundle) }
                        Err(_) => "Copy failed · click Copy link to retry".into(),
                    },
                    Ok(_) | Err(_) => "Sharing stopped · click Copy link to retry".into(),
                });
                coord.sync_native_pill();
                cx.notify();
            });
        }).detach();
    }

    pub fn paste_completed_bundle(
        &mut self,
        click_target: Option<[f32; 2]>,
        cx: &mut Context<Self>,
    ) {
        let dir = self
            .completed_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("latest"));
        self.dismiss_island(cx);
        if !crate::capture::engine::runtime::accessibility_granted() {
            apparent_log("Paste chip without Accessibility; asking for it");
            self.gate
                .defer(vec![crate::capture::desktop::settings::onboarding::Permission::Accessibility]);
            return;
        }
        self.mark_handled(&dir);
        std::thread::spawn(move || {
            if let Err(err) = crate::shared::handoff::paste_bundle_at_target(&dir, click_target) {
                apparent_log(format!("paste_bundle_at_target error: {err}"));
                #[cfg(target_os = "linux")]
                crate::capture::desktop::linux::shell::notify(
                    "Paste stopped",
                    &format!(
                        "{err:#}. Manually attach the contact sheet and paste summary.md from {}",
                        dir.display()
                    ),
                );
            }
        });
    }

    /// The UI loop's tick: advances the springs unless a new capsule window awaits its second
    /// frame (`island_await_frame`).
    pub fn island_tick(&mut self, cx: &mut Context<Self>) {
        self.tick_hosted_sharing(cx);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if matches!(self.island_phase, IslandPhase::Recording) {
            platform::tick_pill_drag();
        }
        let live_no_words = self.recording_has_no_words();
        if live_no_words != self.last_live_no_words {
            self.last_live_no_words = live_no_words;
            self.sync_native_pill();
            cx.notify();
        }
        if self.island_await_frame {
            // Ask for the frame that ends the wait.
            cx.notify();
        } else {
            self.step_island(cx);
        }
    }

    /// Advances the capsule's springs to now. The UI loop calls it every few ms to keep frames
    /// coming, and the capsule calls it as it draws, so every frame shows its own moment rather
    /// than the loop's last tick (up to 8 ms stale, which made the steps uneven).
    pub fn step_island(&mut self, cx: &mut Context<Self>) {
        if let Some(actions) = self.native_pill.as_ref().map(|p| p.drain_actions()) {
            for action in actions {
                self.apply_pill_action(action, cx);
            }
        }
        let tune = motion::tune();
        let real = self.island_stepped.elapsed().as_secs_f32();
        if real < 0.0002 {
            return;
        }
        self.island_stepped = Instant::now();
        // A late frame slows the motion rather than skipping it ahead (a new window's second
        // frame comes ~50 ms after its first).
        let dt = (real * motion::time_scale()).min(0.025);
        if let IslandPhase::Completed { finished_at, .. } = self.island_phase
            && finished_at.elapsed() >= Duration::from_millis(25_000)
            && self.share_pending.is_none()
            && !matches!(
                self.completed_note.as_deref(),
                Some("Saved · paste manually" | "Saved · open bundle (copy failed)")
            )
        {
            self.set_island_phase(IslandPhase::Dismissing);
        }

        if let IslandPhase::Selecting { mode } = self.island_phase {
            if let Some(pos) = current_mouse_screen() {
                self.follow_pointer_display(pos, cx);
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
                let target_rect = [
                    win.frame.x as f32,
                    win.frame.y as f32,
                    win.frame.width as f32,
                    win.frame.height as f32,
                ];
                self.animated_window_rect = Some(target_rect);
            } else {
                self.animated_window_rect = None;
            }
            self.sync_picker_overlay();
        }
        self.follow_phone_mirror();
        #[cfg(target_os = "linux")]
        self.follow_recording_pointer();

        // Keep layout width at `island_target_width` during open and dismiss so the capsule never
        // clips its inner chips; open/dismiss use a restrained 0.92 <-> 1.0 scale.
        let (target_w, target_scale, target_y, target_opacity, fade_rate) = match self.island_phase
        {
            IslandPhase::Hidden => return,
            IslandPhase::Dismissing => (self.island_target_width, 0.92, -6.0, 0.0, 18.0),
            _ => (self.island_target_width, 1.0, 0.0, 1.0, 20.0),
        };

        if self.reveal.is_open() {
            let activity = self.reveal_activity();
            let on_pill = self
                .native_pill
                .as_ref()
                .map_or_else(pointer_on_hud_pill, |p| p.pointer_on_pill());
            if self.reveal.tick(reveal_clock_ms(), on_pill, activity) {
                self.play_fold();
            }
        }

        let (width, velocity) = if self.reduced_motion && self.island_morph != Morph::Phase {
            // Reduce motion: the reveal's groups fade in place with no width travel; a shrink
            // waits for the outgoing chips to fade so none are clipped while visible.
            let fading =
                self.reveal_outgoing.is_some() && motion::age(self.reveal_since) < REDUCED_FADE_S;
            if fading && target_w < self.island_width {
                (self.island_width, 0.0)
            } else {
                (target_w, 0.0)
            }
        } else {
            spring_step(
                self.island_width,
                self.island_velocity,
                target_w,
                dt,
                self.island_morph.spring(&tune),
            )
        };
        let (scale, s_vel) = spring_step(
            self.island_scale,
            self.island_scale_velocity,
            target_scale,
            dt,
            tune.island_scale,
        );
        let (y_off, y_vel) = spring_step(
            self.island_y,
            self.island_y_velocity,
            target_y,
            dt,
            tune.island_scale,
        );
        if let Some(morph) = self.switch.as_mut()
            && let Some((x, v)) = morph.slide
        {
            let (x, v) = spring_step(x, v, 0.0, dt, tune.switch_slide);
            morph.slide = Some((x, v));
            if x.abs() < 0.2 && v.abs() < 2.0 {
                self.switch = None;
            }
        }
        self.island_width = width.clamp(120.0, PILL_WINDOW_W - 16.0);
        self.island_velocity = velocity;
        self.island_scale = scale.clamp(0.35, 1.18);
        self.island_scale_velocity = s_vel;
        self.island_y = y_off.clamp(-18.0, 14.0);
        self.island_y_velocity = y_vel;
        self.island_opacity += (target_opacity - self.island_opacity) * (fade_rate * dt).min(1.0);
        self.sync_native_pill();

        let dismiss_done = if let Some(ref pill) = self.native_pill {
            pill.is_fully_hidden()
        } else {
            self.island_opacity < 0.03
        };
        if matches!(self.island_phase, IslandPhase::Dismissing) && dismiss_done {
            self.set_island_phase(IslandPhase::Hidden);
            self.island_opacity = 0.0;
            cx.defer(close_capture_windows);
        }
        if self.native_pill.is_none() {
            cx.notify();
        }
    }

    /// The menu bar's picker, opening on the mode the last recording used.
    pub fn begin_target_selection(&mut self, cx: &mut Context<Self>) {
        if self.ready_to_record() {
            self.feedback_capture = false;
            self.open_picker(None, cx);
        }
    }

    /// Opens the picker on `mode`, else on the mode the last recording used.
    fn open_picker(&mut self, mode: Option<SelectionMode>, cx: &mut Context<Self>) {
        if mode == Some(SelectionMode::Window) {
            self.report_window_recording_disabled(cx);
            return;
        }
        if self.is_recording() || self.is_processing() {
            return;
        }
        apparent_log(format!("open_picker invoked (mode: {mode:?})"));
        let started = Instant::now();
        self.recording_shortcut_key = false;
        self.selectable_windows.clear();
        self.hovered_window = None;
        self.animated_window_rect = None;
        self.selection_pointer = current_mouse_screen();
        // The picker opens on the display under the pointer and follows it there
        // (`follow_pointer_display`); Screen mode highlights all of it.
        let screens = crate::capture::engine::runtime::screens::current();
        if let Some(i) =
            crate::capture::engine::runtime::screens::pointer().and_then(|p| crate::capture::engine::runtime::screens::index_at(&screens, p))
        {
            self.display_rect = screens[i].frame.map(|v| v as f32);
        }
        if let Some(ref mut overlay) = self.picker_metal_overlay {
            overlay.move_to(self.display_rect.map(f64::from));
        }
        self.selection_generation += 1;
        self.windows_loading = !self.lab;
        if !self.lab {
            self.refresh_iphone(cx);
        }
        let generation = self.selection_generation;
        cx.spawn(async move |entity, cx| {
            if entity.read_with(cx, |coord, _| coord.lab).unwrap_or(true) {
                return;
            }
            let windows = cx
                .background_executor()
                .spawn(async { list_selectable_windows() })
                .await;
            let _ = entity.update(cx, |coord, cx| {
                // A previous picker request must not overwrite a reopened picker or recording.
                if coord.selection_generation != generation || !coord.is_selecting() {
                    return;
                }
                coord.windows_loading = false;
                let ptr = current_mouse_screen().or(coord.selection_pointer);
                coord.selection_pointer = ptr;
                coord.hovered_window = ptr
                    .and_then(|[x, y]| selectable_window_at(&windows, f64::from(x), f64::from(y)))
                    .cloned();
                if matches!(
                    coord.island_phase,
                    IslandPhase::Selecting {
                        mode: SelectionMode::Window
                    }
                ) {
                    coord.animated_window_rect = coord.hovered_window.as_ref().map(|w| {
                        [
                            w.frame.x as f32,
                            w.frame.y as f32,
                            w.frame.width as f32,
                            w.frame.height as f32,
                        ]
                    });
                }
                coord.selectable_windows = windows;
                coord.sync_picker_overlay();
                start_metric("picker_ready_ms", started);
                cx.notify();
            });
        })
        .detach();
        self.selection_drag = None;
        if matches!(
            self.island_phase,
            IslandPhase::Hidden | IslandPhase::Dismissing
        ) {
            // Enter near full size (0.92 -> 1.0) at full intrinsic layout width so inner buttons
            // are never clipped.
            self.island_width = self.island_target_width.max(195.0);
            self.island_velocity = 0.0;
            self.island_scale = 0.92;
            self.island_scale_velocity = 0.0;
            self.island_y = -6.0;
            self.island_y_velocity = 0.0;
            self.island_opacity = 0.0;
            self.island_phase = IslandPhase::Hidden;
        }
        // Open on the mode the last recording used. The phone needs a live preview, started once
        // the picker is up; Window starts on Area's crosshair until the window list arrives.
        let opening = match mode.unwrap_or(self.last_mode) {
            SelectionMode::Iphone => SelectionMode::Area,
            mode => mode,
        };
        self.set_island_phase(IslandPhase::Selecting { mode: opening });
        self.open_iphone_when_found = false;
        if mode.is_none() && self.last_mode == SelectionMode::Iphone {
            if self.iphone.is_some() {
                self.preview_iphone(cx);
            } else {
                self.open_iphone_when_found = true;
            }
        }
        self.telemetry.begin_picker_session();
        self.sync_picker_overlay();
        if self.picker_metal_overlay.is_none() && !self.lab {
            let [x, y, w, h] = self.display_rect.map(f64::from);
            let overlay =
                platform::open_overlay(Arc::clone(&self.telemetry), Some([x, y, w, h]), cx)
                    .map_err(|error| apparent_log(format!("Metal overlay failed: {error}")))
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

        let entity = cx.entity().clone();
        cx.defer(move |cx| {
            close_review_window(cx);
            open_picker_windows(&entity, cx);
            start_metric("picker_open_ms", started);
        });
        cx.notify();
        start_metric("picker_prep_ms", started);
    }

    /// From the picker: the full screen it highlights.
    pub fn start_recording_full_screen(&mut self, cx: &mut Context<Self>) {
        let [x, y, w, h] = self.display_rect;
        let centre = [f64::from(x + w / 2.0), f64::from(y + h / 2.0)];
        let target = if self.lab {
            lab::target()
        } else {
            crate::capture::engine::runtime::discover_target_at(None, Some(centre))
        };
        self.start_recording_with_target(target, cx);
    }

    pub fn start_recording_with_target(&mut self, target: TargetInfo, cx: &mut Context<Self>) {
        if crate::capture::desktop::workflow::shutdown::requested() { return; }
        if crate::capture::engine::runtime::ensure_recording_mode(target.mode).is_err() {
            self.report_window_recording_disabled(cx);
            return;
        }
        if self.is_recording() || self.is_processing() {
            return;
        }
        if self.lab {
            self.start_lab_recording(target, cx);
            return;
        }
        // No surprise prompt mid-capture: the permissions are granted in onboarding.
        if !self.ready_to_record() {
            self.cancel_recording(cx);
            return;
        }
        #[cfg(target_os = "linux")]
        if let Some(warning) = crate::capture::desktop::linux::media::encoder_warning() {
            apparent_log(&warning);
            crate::capture::desktop::linux::shell::notify("Apparent setup: H.264 encoder missing", &warning);
            self.cancel_recording(cx);
            return;
        }
        whiteboard::reset_for_capture(&self.whiteboard);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let Ok(id) = self.agent_capture_id.clone().map_or_else(crate::capture::desktop::workflow::agent_capture::new_id, Ok) else {
                apparent_log("Could not create capture identity; refusing an untracked recording");
                self.cancel_recording(cx);
                return;
            };
            self.agent_captures.begin(&id);
            self.agent_capture_id = Some(id);
        }
        self.begin_playback();
        if let Ok(mut status) = CAPTURE_STATUS.lock() {
            *status = None;
        }
        let started = Instant::now();
        apparent_log(format!(
            "start_recording_with_target invoked: mode={:?} frame={:?}",
            target.mode, target.frame
        ));
        // Without the picker the start needs its own cue: the overlay flashes an edge glow.
        let quick = !self.is_selecting();
        self.telemetry.start_glow.store(
            quick && target.mode == CaptureMode::Display,
            std::sync::atomic::Ordering::Release,
        );
        if let IslandPhase::Selecting { mode } = self.island_phase {
            self.remember_mode(mode);
        }
        self.recording_shortcut_key = false;
        self.iphone_preview = None;
        self.selection_drag = None;
        self.hovered_window = None;
        self.animated_window_rect = None;
        let id = crate::shared::ids::make_bundle_id();
        let root = std::env::var_os("APPARENT_BENCH_START")
            .map(PathBuf::from)
            .unwrap_or_else(crate::shared::handoff::default_captures_dir);
        let bundle_dir = root.join(format!("{id}.apparent"));
        if let Err(error) = crate::capture::engine::bundle::recover::begin_screen_capture(&bundle_dir) {
            apparent_log(format!("Capture could not protect raw video: {error:#}"));
            self.cancel_recording(cx);
            if let Ok(mut status) = CAPTURE_STATUS.lock() {
                *status = Some("Capture failed — could not protect raw video".into());
            }
            return;
        }
        let video_path = bundle_dir.join("recording.mp4");

        self.telemetry.set_capture_mode(target.mode);
        self.telemetry.reset_for_session(target.frame);
        // The picker stays open until present_recording_island defers its close.
        // Keep it out of the first ScreenCaptureKit frames after the session reset.
        #[cfg(target_os = "macos")]
        restore_picker_exclusion(
            &mut self.telemetry.excluded_window_ids.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            platform::picker_window_number(),
        );
        let stage = Instant::now();
        // Reuse the continuous 120Hz Metal overlay opened in `begin_target_selection` so the
        // cutout and border never swap windows or jump a single pixel when recording starts.
        // It moves to the target's display if that's another one (a window picked there).
        let f = target.frame;
        let target_rect = [f.x, f.y, f.width, f.height];
        let screens = crate::capture::engine::runtime::screens::current();
        self.display_rect =
            crate::capture::engine::runtime::screens::index_at(&screens, [f.x + f.width / 2.0, f.y + f.height / 2.0])
                .map_or(target_rect, |i| screens[i].frame)
                .map(|v| v as f32);
        if let Some(ref pill) = self.native_pill {
            pill.place_on([f.x + f.width / 2.0, f.y + f.height / 2.0]);
        }
        #[cfg(target_os = "macos")]
        {
            platform::cancel_pill_drag();
            if self.native_pill.is_none() {
                platform::place_hud_pill([f.x + f.width / 2.0, f.y + f.height / 2.0]);
            }
        }
        #[cfg(target_os = "linux")]
        {
            platform::cancel_pill_drag();
            platform::place_hud_pill([f.x + f.width / 2.0, f.y + f.height / 2.0]);
        }
        let metal_overlay = self.picker_metal_overlay.take().map(|mut overlay| {
            overlay.move_to(target_rect);
            // reset_for_session cleared the picker's exclusion. Reusing its window
            // must not turn the ink layer into captured screen content on macOS.
            self.telemetry.excluded_window_ids.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(overlay.window_number());
            overlay
        });
        let metal_overlay = metal_overlay.or_else(|| {
            let ov = platform::open_overlay(Arc::clone(&self.telemetry), Some(target_rect), cx)
                .map_err(|error| apparent_log(format!("Metal overlay failed: {error}")))
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
        self.publish_capture_busy();
        let audio_recorder = match AudioRecorder::start(&bundle_dir) {
            Ok(recorder) => Some(recorder),
            Err(error) => {
                let message = format!("Capture refused: narration could not be saved: {error:#}");
                apparent_log(&message);
                self.cancel_recording(cx);
                if let Ok(mut status) = CAPTURE_STATUS.lock() { *status = Some(message); }
                return;
            }
        };
        start_metric("audio_start_ms", stage);
        let stage = Instant::now();
        let screen_session = match ScreenCaptureSession::start(
            target.clone(),
            Some(video_path),
            Arc::clone(&self.telemetry.excluded_window_ids),
        ) {
            Ok(session) => session,
            Err(error) => {
                apparent_log(format!("Capture refused: {error:#}"));
                if let Err(cleanup) = crate::capture::engine::bundle::recover::abandon_start(&bundle_dir, || drop(audio_recorder)) {
                    apparent_log(format!("Failed capture startup cleanup at {}: {cleanup:#}", bundle_dir.display()));
                }
                self.cancel_recording(cx);
                return;
            }
        };
        start_metric("sck_start_ms", stage);
        let window_log = (target.mode == CaptureMode::Display).then(|| {
            let started_at = *self
                .telemetry
                .started_at
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            crate::shared::windows::WindowLogger::start(
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
        self.present_recording_island(cx);
        start_metric("recording_start_ms", started);
    }

    /// The island turns into the recording pill: from the picker it morphs, from nothing it opens
    /// from a compact capsule so the shell springs out to its width, Dynamic Island style.
    fn present_recording_island(&mut self, cx: &mut Context<Self>) {
        if matches!(self.island_phase, IslandPhase::Hidden) {
            self.island_width = ISLAND_OPEN_FROM_W;
            self.island_velocity = 0.0;
            self.island_scale = 0.92;
            self.island_scale_velocity = 0.0;
            self.island_y = -6.0;
            self.island_y_velocity = 0.0;
        }
        self.set_island_phase(IslandPhase::Recording);
        let entity = cx.entity().clone();
        cx.defer(move |cx| {
            close_review_window(cx);
            close_picker_window(cx);
            ensure_hud_pill_open(&entity, cx);
        });
        cx.notify();
    }

    /// `apparent lab pill`: a pretend recording with no capture, microphone, files or overlay, so
    /// the capsule goes through exactly the states a real one does.
    fn start_lab_recording(&mut self, target: TargetInfo, cx: &mut Context<Self>) {
        whiteboard::reset_for_capture(&self.whiteboard);
        if let IslandPhase::Selecting { mode } = self.island_phase {
            motion::trace_event(format!("record:{}", mode.label()));
        }
        self.iphone_preview = None;
        self.selection_drag = None;
        self.hovered_window = None;
        self.animated_window_rect = None;
        self.active_recording = Some(ActiveRecording {
            started: Instant::now(),
            target,
            bundle_dir: PathBuf::new(),
            source: CaptureSource::Lab,
            _metal_overlay: None,
        });
        self.present_recording_island(cx);
    }

    /// Mirrors the cabled iPhone in the helper's window and records its native video. The
    /// window's on-screen rect is the capture target, so the laser, ink, pins and clicks over it
    /// normalize straight into phone-screen coordinates (kept current as the window moves).
    pub fn start_recording_iphone(&mut self, cx: &mut Context<Self>) {
        if self.is_recording() || self.is_processing() {
            return;
        }
        if let Ok(mut status) = CAPTURE_STATUS.lock() {
            *status = None;
        }
        apparent_log("start_recording_iphone invoked");
        if self.lab {
            self.start_lab_recording(crate::capture::engine::runtime::phone::phone_target("iPhone", None, None, 0, 0), cx);
            return;
        }
        if !self.ready_for(crate::capture::desktop::settings::onboarding::Permission::Camera) {
            self.cancel_recording(cx);
            return;
        }
        whiteboard::reset_for_capture(&self.whiteboard);
        self.begin_playback();
        self.remember_mode(SelectionMode::Iphone);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let Ok(id) = crate::capture::desktop::workflow::agent_capture::new_id() else {
                apparent_log("Could not create capture identity; refusing an untracked recording");
                self.cancel_recording(cx);
                return;
            };
            self.agent_captures.begin(&id);
            self.agent_capture_id = Some(id);
        }
        self.recording_shortcut_key = false;
        self.selection_drag = None;
        self.hovered_window = None;
        self.animated_window_rect = None;
        let bundle_dir = crate::shared::handoff::default_captures_dir()
            .join(format!("{}.apparent", crate::shared::ids::make_bundle_id()));
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
        let target = crate::capture::engine::runtime::phone::phone_target(&name, None, None, 0, 0);
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
        // Hold interaction telemetry inactive until the helper's `recording` event arrives.
        self.telemetry
            .active
            .store(false, std::sync::atomic::Ordering::Release);
        let session = if let Some(mut preview) = existing_preview {
            preview.begin_recording(&movie_path);
            preview
        } else {
            let dev_id = device.as_ref().map(|d| d.id.as_str());
            match PhoneSession::start(&movie_path, dev_id, Some(Arc::clone(&self.telemetry))) {
                Ok(session) => session,
                Err(error) => {
                    apparent_log(format!("iPhone capture could not start: {error:#}"));
                    #[cfg(any(target_os = "macos", target_os = "linux"))]
                    {
                        if let Some(id) = self.agent_capture_id.take() {
                            self.agent_captures.set(&id, "failed", None, Some("start_failed"));
                        }
                    }
                    self.telemetry.finish_session();
                    let _ = std::fs::remove_dir_all(&bundle_dir);
                    self.dismiss_island(cx);
                    return;
                }
            }
        };
        // Over the mirror window's display (the pointer's until the window reports its rect).
        let mirror = (initial_frame.x > -10_000.0).then_some([
            initial_frame.x,
            initial_frame.y,
            initial_frame.width,
            initial_frame.height,
        ]);
        let metal_overlay = self.picker_metal_overlay.take().or_else(|| {
            platform::open_overlay(Arc::clone(&self.telemetry), mirror, cx)
                .map_err(|error| apparent_log(format!("Metal overlay failed: {error}")))
                .ok()
        });
        self.active_recording = Some(ActiveRecording {
            started: Instant::now(),
            target,
            bundle_dir,
            source: CaptureSource::Phone(session),
            _metal_overlay: metal_overlay,
        });
        self.present_recording_island(cx);
    }

    /// The mirror ended without being asked: keep what was recorded, or drop an empty capture.
    fn phone_ended(&mut self, cx: &mut Context<Self>) {
        if self.iphone_preview.is_some() {
            self.iphone_preview = None;
            self.set_selection_mode(SelectionMode::Area, cx);
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
            self.finish_recording(cx);
        } else {
            self.cancel_recording(cx);
        }
    }

    pub fn cancel_recording(&mut self, cx: &mut Context<Self>) {
        self.cancel_capture_attempt();
        if self.is_recording() { return; }
        cx.defer(whiteboard::close);
        self.dismiss_island(cx);
    }

    /// Cancellation must also work before a visible island or recording exists.
    fn cancel_capture_attempt(&mut self) {
        // dismiss_island is a no-op while Hidden: startup failure must revoke the
        // shortcut attempt here, not wait for a visible phase transition.
        if let Some(session) = self.onboarding.as_mut() {
            session.input(crate::capture::desktop::settings::onboarding::Input::CaptureEnded { completed: false });
        }
        self.playback.take();
        apparent_log("cancel_recording invoked");
        self.feedback_capture = false;
        self.discard_recording();
    }

    pub fn finish_recording(&mut self, cx: &mut Context<Self>) {
        self.playback.take();
        apparent_log("finish_recording invoked");
        let Some(rec) = self.active_recording.take() else {
            return;
        };
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let finishing_id = self.agent_capture_id.take();
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if let Some(id) = &finishing_id {
            self.agent_captures.set(id, "processing", None, None);
        }
        let feedback_capture = std::mem::take(&mut self.feedback_capture);
        #[cfg(target_os = "linux")]
        let auto_active = !std::mem::take(&mut self.agent_finishing)
            && crate::shared::handoff::experiment(crate::capture::desktop::linux::auto_paste::EXPERIMENT)
            && crate::capture::desktop::linux::hyprland::running()
            && crate::capture::desktop::linux::wayland_session()
            && !self.lab
            && !feedback_capture
            && matches!(&rec.source, CaptureSource::Screen { .. });
        #[cfg(not(target_os = "linux"))]
        let auto_active = false;
        let (events, strokes) = self.telemetry.finish_session();
        let bundle_dir = rec.bundle_dir;
        let target = rec.target;
        let source = rec.source;
        // Drop the Metal overlay immediately on the main thread so the screen frame clears,
        // and morph the Dynamic Island into the compact spinning "Processing…" state while
        // media teardown, transcription, and PNG encoding run on a background worker.
        drop(rec._metal_overlay);
        self.set_island_phase(IslandPhase::Processing);
        let generation = self.capture_generation;
        self.completed_note = None;
        cx.notify();
        if matches!(source, CaptureSource::Lab) {
            cx.defer(whiteboard::close);
            let duration_ms = rec.started.elapsed().as_millis() as u64;
            let wait = Duration::from_secs_f32(lab::PROCESSING_S / motion::time_scale());
            cx.spawn(async move |entity, cx| {
                cx.background_executor().timer(wait).await;
                let _ = entity.update(cx, |coord, cx| {
                    if !coord.accepts_capture_result(generation) {
                        return;
                    }
                    coord.set_island_phase(IslandPhase::Completed {
                        finished_at: Instant::now(),
                        frame_count: 6,
                        duration_ms,
                        inbox_count: 0,
                    });
                    cx.notify();
                });
            })
            .detach();
            return;
        }

        let final_whiteboard = whiteboard::finish_snapshot(&self.whiteboard, || whiteboard::close(cx));

        self.capture_finalizer_started();
        self.publish_capture_busy();
        cx.spawn(async move |entity, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {

                    recording_completion::finalize(
                        source, target, bundle_dir, events, strokes, final_whiteboard,
                    )
                })
                .await;

            let _ = entity
                .update(cx, |coord, cx| {
                    let (finalized, duration_ms) = result;
                    if finalized.is_err() {
                        crate::capture::desktop::workflow::shutdown::clear_request();
                    }
                    // Release the worker even when its capsule was dismissed or
                    // a newer capture owns the UI. A completion never auto-quits.
                    coord.capture_finalizer_finished();
                    coord.publish_capture_busy();
                    coord.receive_capture_result(
                        generation,
                        #[cfg(any(target_os = "macos", target_os = "linux"))]
                        finishing_id.as_deref(),
                        feedback_capture,
                        finalized,
                        desktop_completion::connect_completed,
                        |coord, finalized| {
                            coord.present_capture_result(
                                finalized, duration_ms, feedback_capture, auto_active, &entity, cx,
                            );
                        },
                    );
                });
        })
        .detach();
    }

    fn accepts_capture_result(&self, generation: u64) -> bool {
        generation == self.capture_generation && self.is_processing()
    }
}

pub struct HudPillWindow {
    coordinator: Entity<ApparentCoordinator>,
    focus: FocusHandle,
    /// The capsule's native shell, scale and blur where the platform draws them.
    chrome: PillChrome,
    /// Frames drawn so far, counting to 2.
    frames: u8,
    #[cfg(target_os = "linux")]
    equalizer: InputEqualizer,
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct InputEqualizer {
    last_count: usize,
    level: f32,
}

#[cfg(target_os = "linux")]
impl InputEqualizer {
    fn step(&mut self, input: Option<(usize, f32)>, reduced_motion: bool) {
        if input.is_none_or(|(count, _)| count < self.last_count) {
            self.last_count = 0;
        }
        let target = match input {
            Some((count, level)) if count > self.last_count => level,
            _ => 0.0,
        };
        self.last_count = input.map_or(0, |(count, _)| count);
        let alpha = if reduced_motion {
            1.0
        } else if target > self.level {
            0.24
        } else {
            0.12
        };
        self.level += (target - self.level) * alpha;
    }
}

impl Render for HudPillWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tracing = motion::tracing();
        let frame = self.frames;
        if tracing && frame == 0 {
            motion::trace_skip_frame();
        } else if tracing {
            motion::trace_frame();
        }
        self.frames = self.frames.saturating_add(1).min(2);
        if frame == 0 {
            // A new window shows a few frames after it's asked for; its opening fade and blur
            // start from the frame that is seen, not from the request.
            self.coordinator.update(cx, |coord, _| {
                if coord.island_opacity < 0.98
                    && !matches!(coord.island_phase, IslandPhase::Dismissing)
                {
                    coord.island_opacity = 0.0;
                    coord.island_phase_since = motion::now();
                    coord.island_stepped = Instant::now();
                    coord.island_await_frame = true;
                }
            });
        }
        self.coordinator.update(cx, |coord, cx| {
            if frame == 1 {
                coord.island_await_frame = false;
            }
            if coord.is_island_visible() {
                coord.step_island(cx);
            }
        });
        let coord = self.coordinator.read(cx);
        let phase = coord.island_phase;
        let prev_phase = coord.island_prev_phase;
        let target_phase = match phase {
            IslandPhase::Dismissing => coord.island_dismissed_from,
            other => other,
        };
        let phase_age = motion::age(coord.island_phase_since);
        let width = coord.island_width;
        let scale = coord.island_scale;
        let y_off = coord.island_y;
        let opacity = coord.island_opacity.clamp(0.0, 1.0);
        let recording_ms = coord
            .active_recording
            .as_ref()
            .map(ActiveRecording::elapsed_ms);
        let mic_warning = if matches!(phase, IslandPhase::Recording) {
            coord
                .active_recording
                .as_ref()
                .and_then(ActiveRecording::mic_warning)
        } else {
            None
        };
        #[cfg(target_os = "linux")]
        let equalizer_enabled = recording_equalizer_enabled();
        #[cfg(target_os = "linux")]
        if matches!(phase, IslandPhase::Recording) && equalizer_enabled {
            let input =
                coord
                    .active_recording
                    .as_ref()
                    .and_then(|recording| match &recording.source {
                        CaptureSource::Screen {
                            audio: Some(audio), ..
                        } => audio.input_activity(),
                        _ => None,
                    });
            self.equalizer.step(input, coord.reduced_motion);
        }
        let elapsed_ms = recording_ms.unwrap_or(coord.last_elapsed_ms);
        let switch = coord
            .switch
            .filter(|_| matches!(phase, IslandPhase::Selecting { .. }));
        let island_velocity = coord.island_velocity;
        let highlight_prev = coord.highlight_prev;
        let highlight_p = ease_out_cubic(motion::age(coord.highlight_since) / HIGHLIGHT_FADE_S);
        let clock = format_clock(elapsed_ms);
        let has_iphone = coord.iphone.is_some();
        let completed_note = coord.completed_note.clone();
        let completed_issue = coord.completed_audio_issue();
        let completed_warning = completed_issue.is_some();
        let live_no_words = coord.recording_has_no_words() && !coord.reveal.is_open();
        let pill_theme = PillTheme::for_window(window);
        let recording_warning = mic_warning.is_some() || live_no_words;
        let warning_focus = recording_warning || completed_warning;
        // The Recording capsule's trailing group: switch chips or controls, and the swap between.
        let chips = if matches!(phase, IslandPhase::Recording) {
            coord.reveal.is_open()
        } else {
            coord.reveal_prev_chips
        };
        let reduced = coord.reduced_motion;
        let tune = motion::tune();
        let swap_age = motion::age(coord.reveal_since);
        let (enter_s, exit_s) = if reduced {
            (REDUCED_FADE_S, REDUCED_FADE_S * 0.8)
        } else {
            (tune.reveal_enter_s, tune.reveal_exit_s)
        };
        let swap = matches!(phase, IslandPhase::Recording)
            .then_some(coord.reveal_outgoing)
            .flatten()
            .filter(|_| swap_age < enter_s.max(exit_s));
        let target_width = island_content_width(
            window,
            target_phase,
            &clock,
            has_iphone,
            chips,
            mic_warning,
            completed_note.as_deref(),
            completed_issue,
            live_no_words,
        );
        let snap_initial_width =
            phase_age < 0.025 && matches!(phase, IslandPhase::Selecting { .. }) && opacity < 0.35;
        let switch_layout = switch.map(|morph| {
            let clock_w = measure_text(
                window,
                &clock_template(&clock),
                HUD_TEXT,
                FontWeight::SEMIBOLD,
            );
            SwitchLayout::new(morph, width, target_width, clock_w)
        });
        self.coordinator.update(cx, |coord, _| {
            coord.island_target_width = target_width;
            if snap_initial_width {
                coord.island_width = target_width;
            }
            if recording_ms.is_some() {
                coord.last_elapsed_ms = elapsed_ms;
            }
            if let (Some(morph), Some(layout)) = (coord.switch.as_mut(), switch_layout)
                && morph.slide.is_none()
            {
                // The chips were pinned to the left edge, so they moved at half the width's
                // speed: the row starts there, at that speed.
                morph.slide = Some((layout.dx, -0.5 * island_velocity));
            }
        });
        let render_width = if snap_initial_width {
            target_width
        } else {
            width
        };

        // Two-beat `.blurReplace` cross-transition inside the springing shell:
        // - When morphing between two visible states (`has_prev`):
        //   * 0..55ms (exit): outgoing `prev_phase` stays mounted while blurring `0 -> 3.5px` and fading `1 -> 0.15`.
        //   * 55..140ms (enter): incoming `target_phase` mounts and unblurs `3.5 -> 0px` while fading `0.15 -> 1`.
        // - When opening/dismissing: subtle `3.5px -> 0px` blur over 110ms.
        // A switch chip's morph keeps its items, so nothing is replaced (`SwitchMorph`).
        let has_prev = !matches!(prev_phase, IslandPhase::Hidden | IslandPhase::Dismissing)
            && !matches!(phase, IslandPhase::Hidden | IslandPhase::Dismissing)
            && switch_layout.is_none();
        // Reduce motion keeps the cross-fade and drops the blur.
        let swap_blur = if reduced { 0.0 } else { 3.5 };
        let (content_phase, blur_radius, content_alpha) =
            if matches!(phase, IslandPhase::Dismissing) {
                (
                    target_phase,
                    ((1.0 - opacity) * 3.5).clamp(0.0, 3.5),
                    opacity,
                )
            } else if switch_layout.is_some() {
                (target_phase, 0.0, 1.0)
            } else if has_prev && phase_age < 0.055 {
                let t_out = (phase_age / 0.055).clamp(0.0, 1.0);
                (prev_phase, swap_blur * t_out, 1.0 - 0.85 * t_out)
            } else if has_prev && phase_age < 0.145 {
                let t_in = ease_out_cubic(((phase_age - 0.055) / 0.090).clamp(0.0, 1.0));
                (target_phase, swap_blur * (1.0 - t_in), 0.15 + 0.85 * t_in)
            } else if swap.is_some() && !reduced {
                // Chips ↔ controls in place: a sine blur pulse, sharp at both ends.
                let t = (swap_age / enter_s).clamp(0.0, 1.0);
                (
                    target_phase,
                    tune.reveal_blur_px * (std::f32::consts::PI * t).sin(),
                    1.0,
                )
            } else if !has_prev && matches!(target_phase, IslandPhase::Recording) {
                // Opening straight into Recording: the shell springs out from a compact capsule
                // with the lead pinned to its left edge, and each trailing item fades in as the
                // edge uncovers it (`edge_alpha`), so nothing waits for the shell.
                (target_phase, 0.0, 1.0)
            } else if !has_prev && opacity < 0.999 {
                // Opening: the content sharpens as the capsule fades in (both step-driven).
                (target_phase, swap_blur * (1.0 - opacity), 1.0)
            } else {
                (target_phase, 0.0, 1.0)
            };
        // The swap's blur leaves the content's scale alone so the pinned lead stays put.
        let content_shrink = if swap.is_some() { 0.0 } else { 0.015 };
        let is_completed = matches!(target_phase, IslandPhase::Completed { .. });

        self.chrome.apply(
            render_width,
            ISLAND_H,
            scale,
            y_off,
            opacity,
            is_completed,
            blur_radius,
            content_shrink,
        );
        let trace_xf = tracing
            .then(|| {
                self.chrome.trace(
                    render_width,
                    scale,
                    y_off,
                    opacity,
                    content_alpha,
                    blur_radius,
                    content_shrink,
                )
            })
            .flatten();

        let c_paste_down = self.coordinator.clone();
        let c_paste_up = self.coordinator.clone();
        let c_paste_out = self.coordinator.clone();
        let c_inspect = self.coordinator.clone();
        let c_dismiss = self.coordinator.clone();
        let c_record = self.coordinator.clone();
        let c_sel_cancel = self.coordinator.clone();

        // Keep typography (`HUD_TEXT`) and spacing (`CHIP_GAP`) 100% constant so glyphs never
        // re-hint or jump; all scaling happens in the GPU CALayer transform.
        let row = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(CHIP_GAP))
            .text_size(px(HUD_TEXT))
            .whitespace_nowrap()
            .opacity(content_alpha);
        // Traces a row's children under these names (`APPARENT_MOTION_TRACE`).
        let traced = |group: Div, keys: Vec<Option<(String, f32)>>| match trace_xf {
            Some(xf) => group.on_children_prepainted(motion::trace_children(keys, xf, "row")),
            None => group,
        };
        let named = |names: &[&str]| -> Vec<Option<(String, f32)>> {
            names.iter().map(|n| Some(((*n).to_owned(), 1.0))).collect()
        };

        // The highlight cross-fades from the mode it left.
        let highlight = |m: SelectionMode, mode: SelectionMode| -> f32 {
            match highlight_prev.filter(|prev| *prev != mode) {
                _ if m == mode && highlight_prev.is_none() => 1.0,
                Some(_) if m == mode => highlight_p,
                Some(prev) if m == prev => 1.0 - highlight_p,
                _ => f32::from(u8::from(m == mode)),
            }
        };
        // Per-item opacity of the picker row while it slides in (`SwitchMorph`).
        let selecting_alphas = |mode: SelectionMode| -> Vec<f32> {
            let Some(layout) = switch_layout else {
                return Vec::new();
            };
            let modes = SelectionMode::available(has_iphone);
            let mut widths: Vec<f32> = modes.iter().map(|&m| segment_width(window, m)).collect();
            if mode.shows_record() {
                widths.push(chip_width(window, "Record", FontWeight::SEMIBOLD));
            }
            widths.extend([layout.divider_w, CHIP_H]);
            trail_offsets(layout.row_left(), &widths)
                .iter()
                .zip(&widths)
                .map(|(&x, &w)| uncover_alpha(layout.inner_now + ISLAND_INSET, x, w))
                .collect()
        };
        let ghost = switch_layout.map(|layout| {
            let pieces = layout.ghost_alphas();
            div()
                .absolute()
                .top(px(ISLAND_INSET))
                .left(px(ISLAND_INSET + layout.ghost_left()))
                .h(px(CHIP_H))
                .flex()
                .items_center()
                .gap(px(CHIP_GAP))
                .text_size(px(HUD_TEXT))
                .whitespace_nowrap()
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .pr(px(LEAD_TRAIL))
                        .child(
                            div()
                                .size(px(CHIP_H))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .opacity(pieces[0])
                                .child(div().size(px(8.0)).rounded_full().bg(rgb(INK_RED))),
                        )
                        .child(
                            div()
                                .w(px(layout.clock_w))
                                .font(tabular_font(FontWeight::SEMIBOLD))
                                .opacity(pieces[1])
                                .child(clock.clone()),
                        ),
                )
                .child(island_divider(pill_theme).opacity(pieces[2]))
        });
        let ghost = match (ghost, trace_xf) {
            (Some(g), Some(xf)) => {
                let lead_alpha = switch_layout.map_or(0.0, |l| l.ghost_alphas()[1]);
                let divider_alpha = switch_layout.map_or(0.0, |l| l.ghost_alphas()[2]);
                Some(g.on_children_prepainted(motion::trace_children(
                    vec![
                        Some(("lead".into(), lead_alpha)),
                        Some(("divider.lead".into(), divider_alpha)),
                    ],
                    xf,
                    "row",
                )))
            }
            (ghost, _) => ghost,
        };

        let capsule_content = match content_phase {
            IslandPhase::Selecting { mode } => {
                let alphas = selecting_alphas(mode);
                let alpha = |i: usize| alphas.get(i).copied().unwrap_or(1.0);
                let n_modes = SelectionMode::available(has_iphone).len();
                let record = mode.shows_record();
                let after = n_modes + usize::from(record);
                traced(
                    row.when_some(switch_layout, |row, layout| {
                        row.relative().left(px(layout.dx))
                    }),
                    SelectionMode::available(has_iphone)
                        .iter()
                        .map(|m| m.label())
                        .map(|label| format!("chip:{label}"))
                        .chain(record.then(|| "record".to_owned()))
                        .chain(["divider.close".to_owned(), "close".to_owned()])
                        .enumerate()
                        .map(|(i, key)| Some((key, alpha(i))))
                        .collect(),
                )
                .children(SelectionMode::available(has_iphone).iter().enumerate().map(
                    |(i, &segment)| {
                        let c_segment = self.coordinator.clone();
                        mode_segment_level(segment, highlight(segment, mode), pill_theme, !record)
                            .opacity(alpha(i))
                            .on_click(move |_, _, cx| {
                                c_segment.update(cx, |coord, cx| {
                                    coord.click_selection_mode(segment, cx)
                                });
                            })
                    },
                ))
                .when(record, |row| {
                    row.child(
                        primary_chip("sel-record", "Record", pill_theme, false)
                            .opacity(alpha(n_modes))
                            .on_click(move |_, _, cx| {
                                c_record.update(cx, |coord, cx| coord.record_selected_mode(cx));
                            }),
                    )
                })
                .child(island_divider(pill_theme).opacity(alpha(after)))
                .child(
                    icon_circle_chip("sel-cancel", "✕", pill_theme)
                        .opacity(alpha(after + 1))
                        .on_click(move |_, _, cx| {
                            c_sel_cancel.update(cx, |coord, cx| coord.cancel_recording(cx));
                        }),
                )
            }
            IslandPhase::Completed { inbox_count, .. } => traced(
                row,
                ["done.lead", "divider.lead", "paste"]
                    .into_iter()
                    .map(str::to_owned)
                    .chain(["inspect".to_owned(), "close".to_owned()])
                    .map(|name| Some((name, 1.0)))
                    .collect(),
            )
            .child(
                island_lead()
                    .child(
                        div()
                            .size(px(CHIP_H))
                            .rounded_full()
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgb(if completed_warning {
                                WARNING
                            } else {
                                pill_theme.neutral()
                            }))
                            .text_color(rgb(if completed_warning {
                                pill_theme::CARBON
                            } else {
                                pill_theme.text()
                            }))
                            .text_size(px(13.0))
                            .font_weight(FontWeight::BOLD)
                            .child(if completed_warning { "!" } else { "✓" }),
                    )
                    .child(
                        div()
                            .ml(px(8.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(pill_theme.text()))
                            .child(completed_status_label(
                                inbox_count,
                                completed_note.as_deref(),
                                completed_issue,
                            )),
                    ),
            )
            .child(island_divider(pill_theme))
            .child(
                primary_chip("hud-paste", "Paste", pill_theme, completed_warning)
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        let start = current_mouse_screen();
                        c_paste_down.update(cx, |coord, _| {
                            coord.paste_drag_start = start;
                        });
                    })
                    .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                        c_paste_up.update(cx, |coord, cx| {
                            coord.paste_drag_start = None;
                            coord.paste_completed_bundle(None, cx);
                        });
                    })
                    .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                        let end = current_mouse_screen();
                        c_paste_out.update(cx, |coord, cx| {
                            if let (Some(start), Some(drop_pt)) =
                                (coord.paste_drag_start.take(), end)
                            {
                                let dist = (drop_pt[0] - start[0]).hypot(drop_pt[1] - start[1]);
                                if dist >= 24.0 {
                                    coord.paste_completed_bundle(Some(drop_pt), cx);
                                }
                            }
                        });
                    }),
            )
            .when(crate::shared::handoff::experiment(crate::connect::hosted::HOSTED_SHARING), |row| {
                let enabled = self.coordinator.read(cx).share_online && self.coordinator.read(cx).share_pending.is_none();
                let coordinator = self.coordinator.clone();
                row.child(pill_chip("hud-copy-link", "Copy link", pill_theme)
                    .opacity(if enabled { 1.0 } else { 0.4 })
                    .on_click(move |_, _, cx| {
                        if enabled { coordinator.update(cx, |coord, cx| coord.copy_completed_link(cx)); }
                    }))
            })
            .child(
                pill_chip("hud-inspect", "Inspect", pill_theme).on_click(move |_, _, cx| {
                    c_inspect.update(cx, |coord, cx| coord.inspect_completed_bundle(cx));
                }),
            )
            .child(
                icon_circle_chip("hud-dismiss", "✕", pill_theme).on_click(move |_, _, cx| {
                    c_dismiss.update(cx, |coord, cx| coord.dismiss_completed_bundle(cx));
                }),
            ),
            IslandPhase::Recording => {
                // A soft ripple leaves the dot every 1.6s while the dot itself breathes.
                #[cfg(target_os = "linux")]
                let meter_w = if equalizer_enabled {
                    EQUALIZER_W + CHIP_GAP
                } else {
                    0.0
                };
                #[cfg(not(target_os = "linux"))]
                let meter_w = 0.0;
                #[cfg(target_os = "linux")]
                let meter = equalizer_enabled.then(|| {
                    div()
                        .w(px(EQUALIZER_W))
                        .h(px(18.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(2.0))
                        .children([0.6, 0.9, 1.0, 0.8, 0.5].map(|weight| {
                            div()
                                .w(px(3.0))
                                .h(px(3.0 + 15.0 * self.equalizer.level * weight))
                                .rounded(px(1.5))
                                .bg(rgb(0xff6b67))
                        }))
                });
                #[cfg(not(target_os = "linux"))]
                let meter: Option<Div> = None;
                let cycle = 1.6;
                let ripple = (phase_age / cycle).fract();
                let breath = 0.5 + 0.5 * (std::f32::consts::TAU * phase_age / cycle).cos();
                let halo = 8.0 + 12.0 * ease_out_cubic(ripple);
                let clock_w = measure_text(
                    window,
                    &clock_template(&clock),
                    HUD_TEXT,
                    FontWeight::SEMIBOLD,
                );
                let lead = island_lead_base();
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                let lead = lead.on_mouse_down(MouseButton::Left, |_, _, _| {
                    platform::begin_pill_drag();
                });
                traced(
                    row,
                    vec![
                        Some(("lead".into(), 1.0)),
                        Some(("divider.lead".into(), 1.0)),
                        None,
                    ],
                )
                .child(
                    lead
                        .child(
                            div()
                                .size(px(CHIP_H))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    div()
                                        .size(px(halo))
                                        .rounded_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .bg(with_alpha(INK_RED, 0.34 * (1.0 - ripple).powi(2)))
                                        .child(
                                            div()
                                                .size(px(8.0))
                                                .rounded_full()
                                                .bg(with_alpha(INK_RED, 0.72 + 0.28 * breath)),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .w(px(clock_w))
                                .font(tabular_font(FontWeight::SEMIBOLD))
                                .child(clock),
                        ),
                )
                .child(island_divider(pill_theme))
                .when_some(meter, |row, meter| row.child(meter))
                .when_some(mic_warning, |row, warning| {
                    row.child(
                        div()
                            .flex_none()
                            .px(px(CHIP_PX))
                            .text_size(px(HUD_TEXT))
                            .text_color(rgb(pill_theme.warning_text()))
                            .bg(rgb(if pill_theme.dark { 0x4a360e } else { 0xf5d98a }))
                            .rounded(px(4.0))
                            .child(warning.label()),
                    )
                    .child(island_divider(pill_theme))
                })
                .child({
                    // The trailing slot: switch chips or controls. The lead is pinned to the
                    // shell's left edge (`justify_start` below), so each item's fade follows how
                    // far the springing right edge has uncovered it.
                    let trail_x0 = CHIP_H
                        + clock_w
                        + LEAD_TRAIL
                        + CHIP_GAP
                        + (1.0 + 2.0 * DIVIDER_MX)
                        + CHIP_GAP
                        + mic_warning.map_or(0.0, |warning| {
                            measure_text(window, warning.label(), HUD_TEXT, FontWeight::MEDIUM)
                                + 2.0 * CHIP_PX
                                + (1.0 + 2.0 * DIVIDER_MX)
                                + 2.0 * CHIP_GAP
                        })
                        + meter_w;
                    let inner_now = render_width - 2.0 * ISLAND_INSET;
                    let alphas = |group_chips: bool, fade: f32| -> (Vec<f32>, f32) {
                        let widths = recording_trail_widths(window, group_chips, has_iphone);
                        let alphas = trail_offsets(trail_x0, &widths)
                            .iter()
                            .zip(&widths)
                            .map(|(&x, &w)| fade * edge_alpha(inner_now, x, w))
                            .collect();
                        let total = widths.iter().sum::<f32>()
                            + widths.len().saturating_sub(1) as f32 * CHIP_GAP;
                        (alphas, total)
                    };
                    let enter = swap.map_or(1.0, |_| ease_out_cubic(swap_age / enter_s));
                    let (in_alphas, in_w) = alphas(chips, enter);
                    let outgoing = swap.and_then(|out_chips| {
                        let fade = 1.0 - ease_out_cubic(swap_age / exit_s);
                        (fade > 0.002).then(|| {
                            let (out_alphas, _) = alphas(out_chips, fade);
                            traced(
                                self.recording_trail(
                                    out_chips,
                                    has_iphone,
                                    &out_alphas,
                                    false,
                                    pill_theme,
                                    recording_warning,
                                ),
                                trail_keys(out_chips, has_iphone, &out_alphas, "out:"),
                            )
                        })
                    });
                    div()
                        .relative()
                        .flex()
                        .flex_none()
                        .items_center()
                        .h(px(CHIP_H))
                        .w(px(in_w))
                        .when_some(outgoing, |slot, group| {
                            slot.child(div().absolute().left(px(0.0)).top(px(0.0)).child(group))
                        })
                        .child(traced(
                            self.recording_trail(
                                chips,
                                has_iphone,
                                &in_alphas,
                                true,
                                pill_theme,
                                recording_warning,
                            ),
                            trail_keys(chips, has_iphone, &in_alphas, ""),
                        ))
                })
                .when(live_no_words, |row| {
                    row.child(island_divider(pill_theme)).child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(pill_theme.warning_text()))
                            .pr(px(LIVE_WARNING_END_PAD))
                            .child("⚠ No words yet"),
                    )
                })
            }
            IslandPhase::Processing => {
                // 120Hz 8-dot orbiting progress ring inside the leading 32px slot.
                let dots = (0..8).map(|i| {
                    let frac = i as f32 / 8.0;
                    let angle = frac * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                    let turn = (phase_age * 2.1 - frac).rem_euclid(1.0);
                    let intensity = (1.0 - turn).powi(2);
                    let dot_r = 1.35 + 0.65 * intensity;
                    let cx_dot = 16.0 + 7.2 * angle.cos() - dot_r;
                    let cy_dot = 16.0 + 7.2 * angle.sin() - dot_r;
                    div()
                        .absolute()
                        .left(px(cx_dot))
                        .top(px(cy_dot))
                        .size(px(dot_r * 2.0))
                        .rounded_full()
                        .bg(with_alpha(pill_theme.text(), 0.20 + 0.80 * intensity))
                });
                traced(row, named(&["processing"])).child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .pr(px(CHIP_PX))
                        .whitespace_nowrap()
                        .child(div().size(px(CHIP_H)).flex_none().relative().children(dots))
                        .child(
                            div()
                                .ml(px(6.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(pill_theme.text()))
                                .child("Processing…"),
                        ),
                )
            }
            IslandPhase::Hidden | IslandPhase::Dismissing => row,
        };

        let has_shell = self.chrome.has_shell();
        let c_keys = self.coordinator.clone();
        let pin_lead = matches!(content_phase, IslandPhase::Recording);
        // Without a native shell the capsule itself springs: its size follows the shell's scale
        // and it rides the shell's y offset, with the shell's shadow.
        let (shell_w, shell_h, shell_y) = if has_shell {
            (render_width, ISLAND_H, 0.0)
        } else {
            let s = scale.clamp(0.5, 1.15);
            (render_width * s, ISLAND_H * s, y_off)
        };

        div()
            .track_focus(&self.focus)
            // Clicking a chip can make the pill the key window; keep the picker's keyboard map.
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if c_keys.update(cx, |coord, cx| {
                    coord.handle_selection_key(&event.keystroke, event.is_held, cx)
                }) {
                    cx.stop_propagation();
                }
            })
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .opacity(opacity)
            .child(
                div()
                    .relative()
                    .top(px(shell_y))
                    .w(px(shell_w))
                    .h(px(shell_h))
                    .flex_none()
                    .p(px(if has_shell { ISLAND_INSET } else { ISLAND_PAD }))
                    .rounded(px(if has_shell {
                        ISLAND_RADIUS
                    } else {
                        shell_h / 2.0
                    }))
                    .when(!has_shell, |d| {
                        d.shadow(vec![BoxShadow {
                            color: gpui::hsla(0.0, 0.0, 0.0, 0.40 * opacity),
                            offset: point(px(0.0), px(5.0)),
                            blur_radius: px(14.0),
                            spread_radius: px(0.0),
                        }])
                    })
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .when(pin_lead, |d| d.justify_start())
                    .when(!pin_lead, |d| d.justify_center())
                    .when(!has_shell, |d| {
                        d.bg(rgb(pill_theme.surface()))
                            .border_1()
                            .border_color(with_alpha(
                                pill_theme.line(),
                                if warning_focus { 0.40 } else { 0.24 },
                            ))
                    })
                    .text_color(rgb(pill_theme.text()))
                    .text_size(px(HUD_TEXT))
                    .whitespace_nowrap()
                    .relative()
                    .when_some(ghost, |d, ghost| d.child(ghost))
                    .child(capsule_content),
            )
    }
}

/// Where a switch chip's morph is this frame (`SwitchMorph`), in points from the capsule's
/// inner left edge.
#[derive(Clone, Copy, Debug)]
struct SwitchLayout {
    /// The picker row's offset from its resting place.
    dx: f32,
    clock_w: f32,
    lead_w: f32,
    divider_w: f32,
    /// The capsule's inner width now, and the picker row's width.
    inner_now: f32,
    inner_row: f32,
}

impl SwitchLayout {
    fn new(morph: SwitchMorph, width: f32, target_width: f32, clock_w: f32) -> Self {
        let lead_w = CHIP_H + clock_w + LEAD_TRAIL;
        let divider_w = 1.0 + 2.0 * DIVIDER_MX;
        let inner_row = target_width - 2.0 * ISLAND_INSET;
        let dx = morph.slide.map_or_else(
            || {
                // From the capsule's centre: where the chips were, pinned behind the lead, and
                // where the centred picker row rests.
                let chips = -width / 2.0 + ISLAND_INSET + lead_w + divider_w + 2.0 * CHIP_GAP;
                chips + inner_row / 2.0
            },
            |(x, _)| x,
        );
        Self {
            dx,
            clock_w,
            lead_w,
            divider_w,
            inner_now: width - 2.0 * ISLAND_INSET,
            inner_row,
        }
    }

    fn row_left(&self) -> f32 {
        (self.inner_now - self.inner_row) / 2.0 + self.dx
    }

    /// The leaving lead and divider sit just in front of the row, as they did in the pill.
    fn ghost_left(&self) -> f32 {
        self.row_left() - 2.0 * CHIP_GAP - self.divider_w - self.lead_w
    }

    /// Dot, clock and divider each fade out as they near the capsule's left edge and are gone
    /// when they reach it, so nothing is cut by a hard edge while it still shows.
    fn ghost_alphas(&self) -> [f32; 3] {
        // Each piece's left edge in the pill, from the shell's outer edge.
        let rests = [
            ISLAND_INSET + CHIP_H / 2.0 - 4.0,
            ISLAND_INSET + CHIP_H,
            ISLAND_INSET + self.lead_w + CHIP_GAP + DIVIDER_MX,
        ];
        rests.map(|rest| {
            let left = self.ghost_left() + rest;
            let t = (left / rest.min(40.0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        })
    }
}

impl HudPillWindow {
    /// One trailing group of the Recording capsule, each item faded by `alphas`: the switch chips
    /// (Screen selected, as a tap records the screen) or Cancel · Finish. An outgoing
    /// group isn't `interactive`.
    fn recording_trail(
        &self,
        chips: bool,
        has_iphone: bool,
        alphas: &[f32],
        interactive: bool,
        theme: PillTheme,
        warning: bool,
    ) -> Div {
        let alpha = |i: usize| alphas.get(i).copied().unwrap_or(1.0);
        let group = div().flex().flex_none().items_center().gap(px(CHIP_GAP));
        if chips {
            return group.children(SelectionMode::available(has_iphone).iter().enumerate().map(
                |(i, &mode)| {
                    let c_switch = self.coordinator.clone();
                    mode_segment(mode, mode == SelectionMode::Screen, theme, true)
                        .opacity(alpha(i))
                        .when(interactive, |d| {
                            d.on_click(move |_, _, cx| {
                                c_switch.update(cx, |coord, cx| coord.switch_recording(mode, cx));
                            })
                        })
                },
            ));
        }
        let c_cancel = self.coordinator.clone();
        let c_finish = self.coordinator.clone();
        let group = group.child(
            pill_chip("hud-cancel", "Cancel", theme)
                .opacity(alpha(0))
                .when(interactive, |d| {
                    d.on_click(move |_, _, cx| {
                        c_cancel.update(cx, |coord, cx| coord.cancel_recording(cx));
                    })
                }),
        );
        let group = group.when(whiteboard_enabled(), |group| {
            let c_board = self.coordinator.clone();
            group.child(
                pill_chip("hud-whiteboard", "Whiteboard", theme)
                    .opacity(alpha(1))
                    .when(interactive, |d| {
                        d.on_click(move |_, _, cx| {
                            let coord = c_board.clone();
                            cx.defer(move |cx| whiteboard::open(&coord, cx));
                        })
                    }),
            )
        });
        group.child(
            primary_chip("hud-done", "Finish", theme, warning)
                .opacity(alpha(if whiteboard_enabled() { 2 } else { 1 }))
                .when(interactive, |d| {
                    d.on_click(move |_, _, cx| {
                        c_finish.update(cx, |coord, cx| coord.finish_recording(cx));
                    })
                }),
        )
    }
}

/// Leading status cluster: a 32px slot concentric with the capsule's left cap, then text.
/// Also acts as the drag handle for the floating island.
fn island_lead() -> Div {
    island_lead_base().map(platform::window_drag_region)
}

fn island_lead_base() -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .pr(px(LEAD_TRAIL))
        .whitespace_nowrap()
}

fn island_divider(theme: PillTheme) -> Div {
    div()
        .w(px(1.0))
        .h(px(16.0))
        .mx(px(DIVIDER_MX))
        .flex_none()
        .bg(with_alpha(theme.text(), 0.24))
}

fn hud_chip(id: impl Into<gpui::ElementId>) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .h(px(CHIP_H))
        .px(px(CHIP_PX))
        .rounded(px(CHIP_RADIUS))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .whitespace_nowrap()
}

fn pill_chip(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
    theme: PillTheme,
) -> gpui::Stateful<Div> {
    hud_chip(id)
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme.soft()))
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(theme.hover())).text_color(rgb(theme.text())))
        .child(label.into())
}

fn primary_chip(
    id: &'static str,
    label: &'static str,
    theme: PillTheme,
    warning: bool,
) -> gpui::Stateful<Div> {
    hud_chip(id)
        .bg(rgb(if warning { theme.surface() } else { SIGNAL }))
        .when(warning, |d| d.border_1().border_color(rgb(theme.text())))
        .text_color(rgb(if warning {
            theme.text()
        } else {
            theme.active_text()
        }))
        .font_weight(FontWeight::SEMIBOLD)
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(if warning { theme.hover() } else { SIGNAL_HOVER })))
        .child(label)
}

fn icon_circle_chip(id: &'static str, icon: &'static str, theme: PillTheme) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .size(px(CHIP_H))
        .rounded_full()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .text_size(px(12.0))
        .text_color(rgb(theme.muted()))
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(theme.hover())).text_color(rgb(theme.text())))
        .child(icon)
}

/// One segment of the selection-mode control: line icon + label. Weight stays constant across
/// states so switching never shifts the measured capsule width (`selecting_row_width`).
fn mode_segment(
    mode: SelectionMode,
    active: bool,
    theme: PillTheme,
    accent: bool,
) -> gpui::Stateful<Div> {
    mode_segment_level(mode, f32::from(u8::from(active)), theme, accent)
}

fn mix_rgb(from: u32, to: u32, t: f32) -> gpui::Rgba {
    let t = t.clamp(0.0, 1.0);
    let channel = |shift: u32| {
        let a = ((from >> shift) & 0xff) as f32;
        let b = ((to >> shift) & 0xff) as f32;
        ((a + (b - a) * t).round() as u32) << shift
    };
    rgb(channel(16) | channel(8) | channel(0))
}

/// A mode chip `level` of the way to selected, so the highlight can cross-fade between chips.
fn mode_segment_level(
    mode: SelectionMode,
    level: f32,
    theme: PillTheme,
    accent: bool,
) -> gpui::Stateful<Div> {
    let group = mode.chip_id();
    let level = level.clamp(0.0, 1.0);
    let active = level > 0.5;
    hud_chip(group)
        .group(group)
        .px(px(SEG_PX))
        .gap(px(SEG_ICON_GAP))
        .font_weight(FontWeight::MEDIUM)
        .bg(with_alpha(
            if accent { SIGNAL } else { theme.neutral() },
            level,
        ))
        .text_color(mix_rgb(
            theme.soft(),
            if accent {
                theme.active_text()
            } else {
                theme.text()
            },
            level,
        ))
        .cursor_pointer()
        .hover(move |s| {
            s.bg(rgb(if active && accent {
                SIGNAL_HOVER
            } else {
                theme.hover()
            }))
            .text_color(rgb(if active && accent {
                theme.active_text()
            } else {
                theme.text()
            }))
        })
        // `svg()` tints with its own text colour, so hover is mirrored through the group.
        .child(
            svg()
                .path(mode.icon())
                .size(px(SEG_ICON))
                .flex_none()
                .text_color(mix_rgb(
                    theme.muted(),
                    if accent {
                        theme.active_text()
                    } else {
                        theme.text()
                    },
                    level,
                ))
                .group_hover(group, move |s| {
                    s.text_color(rgb(if active && accent {
                        theme.active_text()
                    } else {
                        theme.text()
                    }))
                }),
        )
        .child(mode.label())
}

/// CleanShot-style floating camera cursor rendered at `(x, y)` in `SelectionMode::Window`.
fn camera_cursor_badge(x: f32, y: f32) -> Div {
    div()
        .absolute()
        .left(px(x - 20.0))
        .top(px(y - 20.0))
        .size(px(40.0))
        .rounded_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .bg(rgba(0x161619eb))
        .border_1()
        .border_color(rgba(0xffffff38))
        .shadow(vec![BoxShadow {
            color: gpui::hsla(0.0, 0.0, 0.0, 0.45),
            offset: point(px(0.0), px(3.0)),
            blur_radius: px(10.0),
            spread_radius: px(0.0),
        }])
        .child(
            // Top viewfinder hump
            div()
                .w(px(9.0))
                .h(px(2.5))
                .rounded_t(px(1.5))
                .bg(rgb(0xf5f5f7)),
        )
        .child(
            // Camera body + lens
            div()
                .w(px(21.0))
                .h(px(14.5))
                .rounded(px(3.5))
                .bg(rgb(0xf5f5f7))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .size(px(9.0))
                        .rounded_full()
                        .bg(rgb(0x161619))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(div().size(px(4.5)).rounded_full().bg(rgb(0xf5f5f7))),
                ),
        )
}

pub struct TargetPickerWindow {
    coordinator: Entity<ApparentCoordinator>,
    focus: FocusHandle,
}

impl Render for TargetPickerWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let coord = self.coordinator.read(cx);
        let mode = match coord.island_phase {
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

        root.when_some(badge, |d, ([bx, by], text_w, label)| {
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
                    let cur_mode = match coord.island_phase {
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
                let cur_mode = match coord.island_phase {
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

const BADGE_H: f32 = 22.0;
const BADGE_PX: f32 = 8.0;
const BADGE_TEXT: f32 = 12.0;
const BADGE_GAP: f32 = 8.0;
const BADGE_MARGIN: f32 = 8.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BadgeAnchor {
    /// Just outside the bottom-right corner (Area drags), flipping above or inside near edges.
    BelowTrailing,
    /// Centered over the target (Window hover).
    Center,
}

/// The smallest area the picker records, points a side (`screens::MIN_AREA` is the floor
/// after trimming).
const MIN_AREA_SIDE: f32 = 36.0;

fn review_neighbor<'a>(
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

fn review_notice(
    history_error: Option<&str>,
    status: Option<&str>,
    feedback: bool,
) -> Option<String> {
    status
        .filter(|message| feedback || message.starts_with("Could not open capture:"))
        .map(str::to_owned)
        .or_else(|| history_error.map(|error| format!("Could not read captures: {error}")))
}

/// Pair the frame with its words; stack them when the window cannot fit both.
fn review_workspace_widths(window_width: f32) -> (f32, f32) {
    if window_width < 800.0 {
        ((window_width - 40.0).max(1.0), window_width)
    } else {
        let words = (window_width * 0.4).clamp(320.0, 520.0);
        ((window_width - words - 40.0).max(1.0), words)
    }
}

fn review_frame_speech<'a>(frames: &[Option<&'a str>], selected: usize) -> &'a str {
    frames
        .get(selected)
        .and_then(|said| *said)
        .filter(|said| !said.trim().is_empty())
        .unwrap_or("No words at this moment.")
}

fn review_speech_status(no_words: bool, missing_reason: Option<&str>) -> Option<String> {
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
fn clamp_point([x, y]: [f32; 2], [dx, dy, dw, dh]: [f32; 4]) -> [f32; 2] {
    [x.clamp(dx, dx + dw), y.clamp(dy, dy + dh)]
}

/// A dragged area in global points, kept on the picker's `display` (so the cutout, the size
/// badge and the recording agree), if it's big enough to record.
fn area_rect(a: [f32; 2], b: [f32; 2], display: [f32; 4]) -> Option<[f64; 4]> {
    let [a, b] = [a, b].map(|p| clamp_point(p, display));
    let w = (a[0] - b[0]).abs();
    let h = (a[1] - b[1]).abs();
    (w >= MIN_AREA_SIDE && h >= MIN_AREA_SIDE)
        .then(|| [a[0].min(b[0]), a[1].min(b[1]), w, h].map(f64::from))
}

fn recording_pointer_destination(
    current: [f64; 4],
    pointer: [f64; 2],
    screens: &[crate::capture::engine::runtime::screens::Screen],
) -> Option<[f64; 4]> {
    if crate::capture::engine::runtime::screens::contains(current, pointer) {
        return None;
    }
    let frame = screens
        .get(crate::capture::engine::runtime::screens::index_at(screens, pointer)?)?
        .frame;
    ((0..4).any(|i| (frame[i] - current[i]).abs() >= 1.0)).then_some(frame)
}

fn clip_rect([x, y, w, h]: [f32; 4], [sw, sh]: [f32; 2]) -> [f32; 4] {
    let x0 = x.clamp(0.0, sw);
    let y0 = y.clamp(0.0, sh);
    let x1 = (x + w).clamp(0.0, sw);
    let y1 = (y + h).clamp(0.0, sh);
    [x0, y0, x1 - x0, y1 - y0]
}

/// Positions a badge of `size` relative to `rect`, keeping it fully on screen.
fn place_badge(
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

pub struct BundleReviewWindow {
    coordinator: Entity<ApparentCoordinator>,
    focus: FocusHandle,
    cached_sheet_path: Option<PathBuf>,
    cached_sheet_image: Option<Arc<Image>>,
    cached_frames: Vec<Option<Arc<Image>>>,
    history: Vec<PathBuf>,
    history_error: Option<String>,
}

impl BundleReviewWindow {
    fn new(coordinator: Entity<ApparentCoordinator>, focus: FocusHandle) -> Self {
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

    fn update_history(&mut self, result: anyhow::Result<Vec<PathBuf>>, cx: &mut Context<Self>) {
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

    fn ensure_images(&mut self, dir: &std::path::Path, bundle: &ApparentBundle) {
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
        let feedback = coord_read.review_feedback;
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
                                    if let Ok((dir, bundle, _)) = crate::capture::desktop::demo::generate_demo_bundle(None)
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
        let review_notice =
            review_notice(self.history_error.as_deref(), status.as_deref(), feedback);
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
        let external_shortcut =
            crate::capture::desktop::settings::external_record_owned(crate::capture::engine::runtime::ShortcutKind::Record);
        let c_copy = self.coordinator.clone();
        let c_feedback = self.coordinator.clone();
        let c_send = self.coordinator.clone();
        let c_tab_sheet = self.coordinator.clone();
        let c_tab_frames = self.coordinator.clone();
        let c_newer = self.coordinator.clone();
        let c_older = self.coordinator.clone();
        let copy_dir = dir.clone();
        let send_dir = dir.clone();

        div()
            .id("apparent-review-root")
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
                    .pr(px(platform::HEADER_TRAIL))
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
                            )
                            .when(feedback, |actions| actions.child(
                                action_button("btn-github-issue", "Open GitHub issue…", false)
                                    .on_click(move |_, _, cx| {
                                        let result = crate::shared::handoff::open_path(std::path::Path::new(&feedback_issue_url()));
                                        c_feedback.update(cx, |coord, cx| {
                                            coord.review_status = Some(match result {
                                                Ok(()) => "Asked your browser to open GitHub. Attach only media you choose to share.".into(),
                                                Err(error) => format!("Could not open GitHub: {error}"),
                                            });
                                            cx.notify();
                                        });
                                    }),
                            )),
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
            .into_any_element()
    }
}

impl BundleReviewWindow {
    fn render_contact_sheet_stage(&self, stage_w: f32) -> AnyElement {
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

    fn render_keyframes_stage(
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

fn sidebar_heading(label: &'static str) -> Div {
    div()
        .mb(px(8.0))
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(MUTED_COLOR))
        .child(label)
}

/// Splits a `sketch::timeline` line ("0:03 · Clicked…", "0:04 ✎ Circle…", "0:05 “said”") into
/// a time column and a body, flagging annotation marks.
fn split_timeline_line(line: &str) -> (&str, &str, bool) {
    let (time, rest) = line.split_once(' ').unwrap_or(("", line));
    if let Some(body) = rest.strip_prefix("✎ ") {
        (time, body, true)
    } else {
        (time, rest.strip_prefix("· ").unwrap_or(rest), false)
    }
}

fn timeline_row(index: usize, line: &str) -> gpui::Stateful<Div> {
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

fn tab_chip(id: &'static str, label: &'static str, selected: bool) -> gpui::Stateful<Div> {
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

fn button_base(id: &'static str, primary: bool) -> gpui::Stateful<Div> {
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

fn action_button(id: &'static str, label: &str, primary: bool) -> gpui::Stateful<Div> {
    button_base(id, primary).child(SharedString::from(label.to_string()))
}

/// Button with a trailing muted hint, e.g. a keyboard shortcut.
fn hint_button(
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

#[cfg(any(test, target_os = "macos"))]
fn restore_picker_exclusion(ids: &mut Vec<u32>, picker: Option<u32>) {
    if let Some(id) = picker.filter(|id| *id > 0)
        && !ids.contains(&id)
    {
        ids.push(id);
    }
}

fn open_picker_windows(coord_entity: &Entity<ApparentCoordinator>, cx: &mut App) {
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
fn restack_soon(cx: &mut App) {
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

fn ensure_hud_pill_open(coord_entity: &Entity<ApparentCoordinator>, cx: &mut App) {
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
    let in_use = match (&coord.active_recording, coord.island_phase) {
        (Some(rec), _) if rec.target.mode != CaptureMode::Device => {
            let f = rec.target.frame;
            Some([f.x, f.y, f.width, f.height].map(|v| v as f32))
        }
        (_, IslandPhase::Selecting { .. }) => Some(coord.display_rect),
        _ => None,
    };
    let near =
        in_use.or_else(|| crate::capture::engine::runtime::screens::pointer().map(|[x, y]| [x as f32, y as f32, 1.0, 1.0]));
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
                    pill_coord.read(cx).telemetry.excluded_window_ids.lock()
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

fn close_picker_window(cx: &mut App) {
    for handle in cx.windows() {
        if handle.downcast::<TargetPickerWindow>().is_some() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}

fn close_capture_windows(cx: &mut App) {
    for handle in cx.windows() {
        if handle.downcast::<HudPillWindow>().is_some()
            || handle.downcast::<TargetPickerWindow>().is_some()
        {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}

fn close_review_window(cx: &mut App) {
    for handle in cx.windows() {
        if handle.downcast::<BundleReviewWindow>().is_some() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}

fn open_review_capture(coord: &Entity<ApparentCoordinator>, path: &std::path::Path, cx: &mut App) {
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
fn open_review_window(coord_entity: &Entity<ApparentCoordinator>, cx: &mut App) {
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

pub(crate) fn start_metric(name: &str, started: Instant) {
    if std::env::var_os("APPARENT_BENCH_START").is_some() {
        eprintln!(
            "START_METRIC {name}={:.6}",
            started.elapsed().as_secs_f64() * 1_000.0
        );
    }
}

/// Real startup paths, without registering hotkeys or publishing personal captures.
pub fn run_start_benchmark() {
    let app = Application::new().with_assets(crate::capture::desktop::icons::Assets);
    app.run(|cx: &mut App| {
        let quick_stop = std::env::var("APPARENT_BENCH_QUICK_STOP").as_deref() == Ok("1");
        let (sender, _receiver) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(sender);
        let coordinator = cx.new(|_| ApparentCoordinator::new(telemetry, None));
        cx.spawn(async move |cx| {
            for run in 0..8 {
                coordinator.update(cx, |coord, cx| coord.begin_target_selection(cx)).unwrap();
                // Allow deferred picker creation and async discovery to settle.
                cx.background_executor().timer(Duration::from_millis(600)).await;
                coordinator.update(cx, |coord, cx| {
                    let stage = Instant::now();
                    assert!(!coord.windows_loading, "Picker discovery did not finish");
                    let target = region_target(100.0, 100.0, 480.0, 320.0, &coord.selectable_windows);
                    start_metric("target_discovery_ms", stage);
                    coord.start_recording_with_target(target, cx);
                }).unwrap();
                if !quick_stop {
                    cx.background_executor().timer(Duration::from_millis(1_500)).await;
                }
                coordinator.update(cx, |coord, cx| {
                    let rec = coord.active_recording.take().expect("Recording did not start");
                    assert!(rec._metal_overlay.is_some(), "Metal overlay failed");
                    coord.telemetry.finish_session();
                    let CaptureSource::Screen { session, audio, .. } = rec.source else {
                        unreachable!("The benchmark records the screen")
                    };
                    let (recording, audio) = std::thread::scope(|scope| {
                        let screen = scope.spawn(|| session.stop());
                        let audio = audio.expect("Microphone failed to start").stop().expect("Microphone failed to stop");
                        (screen.join().unwrap(), audio)
                    });
                    if !quick_stop {
                        assert!(recording.video.is_some(), "MP4 missing");
                        assert!(audio.sample_count > 0, "Microphone returned no samples");
                        // Candidates are small (480 px long edge); the MP4 keeps full size.
                        assert!(recording.frames.iter().any(|f| f.width == 480 && f.height == 320), "No real SCK frames (synthetic fallback only)");
                    }
                    let wav = hound::WavReader::new(std::io::Cursor::new(&audio.wav_bytes)).unwrap();
                    assert_eq!(wav.duration() as usize, audio.sample_count);
                    std::fs::write(rec.bundle_dir.join("audio.wav"), &audio.wav_bytes).unwrap();
                    println!("START_RUN {}", serde_json::json!({"run": run, "frames": recording.frames.len(), "first_frame_ms": recording.frames[0].t_ms, "audio_samples": audio.sample_count, "duration_ms": recording.duration_ms, "video_offset_ms": recording.video_offset_ms}));
                    coord.set_island_phase(IslandPhase::Hidden);
                    close_capture_windows(cx);
                }).unwrap();
            }
            cx.update(|cx| cx.quit()).unwrap();
        }).detach();
    });
}

/// Runs the GPUI desktop app, optionally opening an initial `.apparent` bundle in the Review window
/// or starting recording immediately.
pub fn run_app(
    initial_bundle: Option<(PathBuf, ApparentBundle)>,
    start_recording_immediately: bool,
    start_iphone_immediately: bool,
    screenshot_out: Option<PathBuf>,
) {
    let _signals = crate::capture::desktop::workflow::shutdown::Signals::install()
        .inspect_err(|error| apparent_log(format!("Graceful signal handling unavailable: {error:#}"))).ok();
    let app = Application::new().with_assets(crate::capture::desktop::icons::Assets);
    #[cfg(target_os = "macos")]
    app.on_open_urls(|urls| {
        for url in urls {
            if crate::capture::desktop::workflow::launch_instance::request_url(&url).is_err() {
                apparent_log("Rejected unsupported or disabled Apparent launch URL");
            }
        }
    });
    app.run(move |cx: &mut App| {
        if let Err(error) =
            cx.text_system()
                .add_fonts(vec![std::borrow::Cow::Borrowed(include_bytes!(
                    "../../shared/assets/fonts/HostGrotesk-Variable.ttf"
                ))])
        {
            apparent_log(format!("Host Grotesk unavailable: {error}"));
        }
        apparent_log(format!("run_app started pid={}", std::process::id()));
        platform::keep_alive(cx);
        crate::capture::desktop::settings::reconcile_login_item();
        // ⌘W, Esc, the close button or Apparent itself: every way a window closes ends up here.
        cx.on_window_closed(|cx| cx.defer(platform::hand_back_keys))
            .detach();
        let (ui_sender, ui_receiver) = flume::unbounded();
        crate::capture::desktop::web::start_server();
        let telemetry = SharedCaptureTelemetry::new(ui_sender);
        install_global_hotkeys(&telemetry);
        start_global_input_tap(Arc::clone(&telemetry));
        platform::start_updates();
        crate::capture::engine::bundle::retention::start();
        crate::capture::engine::bundle::recover::start();
        crate::shared::hex::watch();
        crate::capture::engine::runtime::audio::transcription::speech_model::ensure();
        let menu_rx = install_status_bar();
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let agent_rx = crate::capture::desktop::workflow::agent_capture::listen().map_err(|error| apparent_log(format!("Agent capture control unavailable: {error:#}"))).ok();

        let mut onboarding_initialized = screenshot_out.is_some()
            || std::env::var_os("APPARENT_NO_ONBOARDING").is_some();
        // Keep the Mac gate fresh even with Settings/onboarding closed. Helper processes
        // run off the UI thread; only the first successful read may open onboarding.
        #[cfg(target_os = "macos")]
        if screenshot_out.is_none() {
            let sender = telemetry.ui_sender.clone();
            let initialized_sender = sender.clone();
            let mut first_poll = true;
            let mut launch = std::env::var_os("APPARENT_NO_ONBOARDING").is_none();
            std::thread::spawn(move || {
                crate::capture::desktop::settings::onboarding::poll_grants(
                    crate::capture::desktop::settings::onboarding::read_grants,
                    move |grants| {
                        let event = if std::mem::take(&mut launch) {
                            crate::capture::desktop::settings::onboarding::Event::Launch(grants)
                        } else {
                            crate::capture::desktop::settings::onboarding::Event::Grants(grants)
                        };
                        sender.send(LiveOverlayEvent::Onboarding(event)).is_ok()
                    },
                    move || {
                        // Preserve native URL launch's startup barrier, even if the
                        // first helper read fails. Successful Launch goes first.
                        if std::mem::take(&mut first_poll) {
                            let _ =
                                initialized_sender.send(LiveOverlayEvent::OnboardingInitialized);
                        }
                        std::thread::sleep(Duration::from_millis(500));
                    },
                );
            });
        }
        // Onboarding opens on first launch, or for a permission that went away.
        #[cfg(not(target_os = "macos"))]
        if !onboarding_initialized {
            let sender = telemetry.ui_sender.clone();
            std::thread::spawn(move || {
                if let Some(grants) = crate::capture::desktop::settings::onboarding::read_grants() {
                    let event = crate::capture::desktop::settings::onboarding::Event::Launch(grants);
                    let _ = sender.send(LiveOverlayEvent::Onboarding(event));
                }
                let _ = sender.send(LiveOverlayEvent::OnboardingInitialized);
            });
        }
        let has_initial = initial_bundle.is_some();
        let coordinator = cx.new(|_| ApparentCoordinator::new(telemetry, initial_bundle));
        set_status_inbox_count(coordinator.read(cx).inbox.len());
        #[cfg(target_os = "linux")]
        if let Err(error) = crate::capture::desktop::linux::update::mark_relaunch_ready() {
            apparent_log(format!("Linux update relaunch readiness: {error:#}"));
        }

        if has_initial {
            open_review_window(&coordinator, cx);
        }
        if start_recording_immediately {
            let coord_clone = coordinator.clone();
            coord_clone.update(cx, |coord, cx| coord.record_or_finish(cx));
        }
        if start_iphone_immediately {
            coordinator.update(cx, |coord, cx| coord.start_recording_iphone(cx));
        }

        if let Some(out_path) = screenshot_out {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(650))
                    .await;
                let window_num = cx.update(|_| platform::front_own_window()).ok().flatten();
                let saved =
                    cx.background_executor()
                        .spawn(async move {
                            platform::capture_own_window_screenshot(window_num, &out_path)
                        })
                        .await;
                if let Err(error) = saved {
                    apparent_log(format!("Window screenshot failed: {error:#}"));
                }
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        }

        let loop_coord = coordinator.clone();
        cx.spawn(async move |cx| {
            let mut shown_status: Option<String> = None;
            let mut shown_export_busy = false;
            let mut last_login_check = std::time::Instant::now();
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(8))
                    .await;
                let _ = cx.update(|cx| {
                    if last_login_check.elapsed() >= Duration::from_secs(2) {
                        crate::capture::desktop::settings::reconcile_login_item();
                        last_login_check = std::time::Instant::now();
                    }
                    let status = current_status_bar_text();
                    let export_busy = EXPORTING.load(std::sync::atomic::Ordering::Acquire);
                    if status != shown_status
                        || (cfg!(target_os = "linux") && export_busy != shown_export_busy)
                    {
                        refresh_status_bar(loop_coord.read(cx).inbox.len());
                        shown_status = status;
                        shown_export_busy = export_busy;
                    }
                    loop_coord.read(cx).publish_capture_busy();
                    platform::poll_status_bar();
                    crate::capture::desktop::settings::onboarding_window::poll(cx);
                    crate::capture::desktop::settings::settings_window::poll(cx);
                    #[cfg(any(target_os = "macos", target_os = "linux"))]
                    if crate::capture::desktop::workflow::launch_instance::take_open(onboarding_initialized) {
                        loop_coord.update(cx, |coord, _| {
                            if !crate::capture::desktop::workflow::launch_url::can_present(
                                coord.is_recording(),
                                coord.is_processing(),
                                coord.is_selecting(),
                            ) {
                                apparent_log("Launch requested; active capture left unchanged");
                            } else if let Some(session) = coord.onboarding.as_mut() {
                                session.show();
                            } else {
                                coord.open_settings(None);
                            }
                        });
                    }
                    #[cfg(any(target_os = "macos", target_os = "linux"))]
                    if let Some(ref rx) = agent_rx {
                        while let Ok(request) = rx.try_recv() {
                            loop_coord.update(cx, |coord, cx| coord.agent_request(request, cx));
                        }
                    }
                    if let Some(ref rx) = menu_rx {
                        while let Ok(action) = rx.try_recv() {
                            match action {
                                MenuAction::ToggleCapture => {
                                    loop_coord.update(cx, |coord, cx| coord.record_or_finish(cx));
                                }
                                MenuAction::GiveFeedback => {
                                    if crate::shared::handoff::experiment("feedbackIssue") {
                                        loop_coord
                                            .update(cx, |coord, cx| coord.record_feedback(cx));
                                    }
                                }
                                MenuAction::ChooseTarget => {
                                    loop_coord.update(cx, |coord, cx| {
                                        if !coord.is_recording()
                                            && !coord.is_processing()
                                            && !coord.is_selecting()
                                        {
                                            coord.begin_target_selection(cx);
                                        }
                                    });
                                }
                                MenuAction::OpenLatest => {
                                    if let Ok((dir, bundle)) =
                                        crate::shared::handoff::load_bundle(std::path::Path::new("latest"))
                                    {
                                        loop_coord.update(cx, |coord, cx| {
                                            coord.mark_handled(&dir);
                                            coord.set_review_bundle((dir, bundle), None);
                                            cx.notify();
                                        });
                                        open_review_window(&loop_coord, cx);
                                    }
                                }
                                MenuAction::OpenInbox(path) => {
                                    let loaded = crate::shared::handoff::load_bundle(&path).ok();
                                    let opened = loaded.is_some();
                                    loop_coord.update(cx, |coord, cx| {
                                        coord.mark_handled(&path);
                                        if let Some(bundle) = loaded {
                                            coord.set_review_bundle(bundle, None);
                                        }
                                        cx.notify();
                                    });
                                    if opened {
                                        open_review_window(&loop_coord, cx);
                                    }
                                }
                                MenuAction::ExportVideo => start_video_export(),
                                MenuAction::Settings => {
                                    loop_coord.update(cx, |coord, _| coord.open_settings(None));
                                }
                                MenuAction::CheckForUpdates => {
                                    platform::check_for_updates();
                                }
                                MenuAction::Quit => {
                                    loop_coord.update(cx, |coord, cx| coord.request_quit(cx));
                                }
                            }
                        }
                    }

                    while let Ok(ev) = ui_receiver.try_recv() {
                        loop_coord.update(cx, |coord, cx| match ev {
                            LiveOverlayEvent::CaptureShortcut => coord.press_shortcut(cx),
                            LiveOverlayEvent::PasteLatestShortcut => {
                                if !coord.is_recording() && !coord.is_selecting() {
                                    coord.paste_latest(cx);
                                }
                            }
                            LiveOverlayEvent::UndoStroke => {
                                coord.undo_stroke(cx);
                            }
                            LiveOverlayEvent::RequestSnapshot => {
                                coord.request_immediate_frame();
                                cx.notify();
                            }
                            LiveOverlayEvent::PhoneEnded => coord.phone_ended(cx),
                            LiveOverlayEvent::Onboarding(event) => {
                                coord.onboarding_event(event, cx)
                            }
                            LiveOverlayEvent::OnboardingInitialized => {
                                onboarding_initialized = true;
                            }
                            LiveOverlayEvent::Settings(event) => coord.settings_event(event, cx),
                        });
                    }

                    motion::poll_tune();
                    if crate::capture::desktop::workflow::shutdown::requested() {
                        loop_coord.update(cx, |coord, cx| coord.request_quit(cx));
                    }
                    if loop_coord.read(cx).shortcut.is_pending() {
                        loop_coord.update(cx, |coord, cx| coord.tick_shortcut(cx));
                    } else if loop_coord.read(cx).telemetry.take_escape()
                        && loop_coord.read(cx).is_selecting()
                    {
                        // The picker may not have the keys (another app is active), but the
                        // key tap sees Esc anyway.
                        loop_coord.update(cx, |coord, cx| coord.cancel_recording(cx));
                    }
                    if loop_coord.read(cx).gate.is_pending() {
                        loop_coord.update(cx, |coord, _| coord.tick_gate());
                    }
                    if loop_coord.read(cx).is_island_visible() {
                        record_external_frontmost_app();
                        loop_coord.update(cx, |coord, cx| coord.island_tick(cx));
                    }
                });

            }
        })
        .detach();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn update_restart_waits_for_capture_admission_and_processing() {
        use crate::capture::desktop::settings::{Action, Event};
        let (sender, _receiver) = flume::unbounded();
        let mut coord = ApparentCoordinator::new(SharedCaptureTelemetry::new(sender), None);
        let restart = Event::Action(Action::UpdateNow);
        assert!(!coord.defer_update_restart(&restart));
        coord.agent_capture_id = Some("admitted-capture".into());
        assert!(coord.defer_update_restart(&restart));
        assert!(!coord.defer_update_restart(&Event::Action(Action::CheckForUpdates)));
        coord.agent_capture_id = None;
        coord.recording_shortcut_key = true;
        assert!(coord.defer_update_restart(&restart));
        coord.recording_shortcut_key = false;
        coord.set_island_phase(IslandPhase::Processing);
        assert!(coord.defer_update_restart(&restart));
        coord.set_island_phase(IslandPhase::Selecting { mode: SelectionMode::Screen });
        assert!(coord.defer_update_restart(&restart));
        coord.set_island_phase(IslandPhase::Hidden);
        assert!(!coord.defer_update_restart(&restart));
        coord.lab = true;
        assert!(coord.defer_update_restart(&restart));
    }

    #[test]
    fn dismissed_and_stale_finalizers_remain_busy_until_every_worker_returns() {
        let (sender, _receiver) = flume::unbounded();
        let mut coord = ApparentCoordinator::new(SharedCaptureTelemetry::new(sender), None);
        assert!(!coord.capture_busy());
        coord.set_island_phase(IslandPhase::Processing);
        let generation = coord.capture_generation;
        coord.capture_finalizer_started();
        // Escape cancels UI ownership, not the detached encoding/bundle worker.
        coord.cancel_capture_attempt();
        coord.set_island_phase(IslandPhase::Dismissing);
        coord.set_island_phase(IslandPhase::Hidden);
        assert!(!coord.accepts_capture_result(generation));
        assert!(coord.capture_busy());
        #[cfg(target_os = "linux")]
        assert!(coord.defer_update_restart(&crate::capture::desktop::settings::Event::Action(
            crate::capture::desktop::settings::Action::UpdateNow)));
        coord.capture_finalizer_started();
        assert_eq!(coord.pending_finalizers, 2);
        // Use the same reception order as finish_recording's callback: release
        // one worker, then dispatch its result even though it no longer owns UI.
        for remaining in [1, 0] {
            coord.capture_finalizer_finished();
            coord.receive_capture_result(generation,
                #[cfg(any(target_os = "macos", target_os = "linux"))]
                None, false, Err(anyhow::anyhow!("deferred finalizer failed")),
                |_| panic!("failed finalizer must not publish"),
                |_, _| panic!("dismissed finalizer must not regain UI ownership"));
            assert_eq!(coord.pending_finalizers, remaining);
            assert_eq!(coord.capture_busy(), remaining > 0);
            assert_eq!(coord.island_phase, IslandPhase::Hidden);
        }
        #[cfg(target_os = "linux")]
        assert!(!coord.defer_update_restart(&crate::capture::desktop::settings::Event::Action(
            crate::capture::desktop::settings::Action::UpdateNow)));
    }


    #[test]
    fn broader_restart_gate_waits_for_dismissed_finalizers_too() {
        let (sender, _receiver) = flume::unbounded();
        let mut coord = ApparentCoordinator::new(SharedCaptureTelemetry::new(sender), None);
        coord.capture_finalizer_started();
        coord.set_island_phase(IslandPhase::Hidden);
        assert!(coord.refuse_capture_restart());
        coord.capture_finalizer_finished();
        assert!(!coord.refuse_capture_restart());
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn deferred_finalizer_success_survives_dismissal_and_new_capture() {
        for processing_b in [false, true] {
            deferred_finalizer_receipt_survives_dismissal_and_new_capture(true, processing_b, false);
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn deferred_finalizer_failure_survives_dismissal_and_new_capture() {
        for processing_b in [false, true] {
            deferred_finalizer_receipt_survives_dismissal_and_new_capture(false, processing_b, false);
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn deferred_feedback_finalizer_stays_local_after_dismissal() {
        for succeeded in [false, true] {
            for processing_b in [false, true] {
                deferred_finalizer_receipt_survives_dismissal_and_new_capture(succeeded, processing_b, true);
            }
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn deferred_finalizer_receipt_survives_dismissal_and_new_capture(succeeded: bool, processing_b: bool, feedback: bool) {
        use crate::capture::desktop::settings::onboarding::{Effect, Flow, Grants, Input, Permission, Session, Status, Step};
        let (sender, _receiver) = flume::unbounded();
        let mut coordinator = ApparentCoordinator::new(SharedCaptureTelemetry::new(sender), None);
        let grants = Grants {
            statuses: Permission::REQUIRED.into_iter().map(|p| (p, Status::Granted)).collect(),
            ..Default::default()
        };
        coordinator.onboarding = Some(Session::for_test(
            Flow::tour(Step::TryIt, Some(grants)).with_first_capture(true, true, false),
        ));
        let bundle: ApparentBundle = serde_json::from_value(serde_json::json!({
            "version": 1, "id": "a", "createdAtMs": 0, "durationMs": 1000,
            "target": {"mode": "display", "displayId": 0,
                "frame": {"x": 0.0, "y": 0.0, "width": 640.0, "height": 480.0, "scale": 1.0}},
            "transcript": {"source": "test", "text": "", "segments": []},
            "strokes": [], "marks": [], "events": [], "keyframes": [], "contactSheet": "c.png"
        })).unwrap();
        let exact_path = PathBuf::from("/synthetic/finalizer/a-exact.apparent");
        // Hold A's result until B starts, then deliver through finish_recording's
        // production coordinator dispatch, not a test recreation of its guard.
        let deferred = if succeeded {
            Ok((exact_path.clone(), bundle.clone(), String::new()))
        } else {
            Err(anyhow::anyhow!("A's deferred finalizer failed"))
        };
        assert!(coordinator.agent_captures.admit("a", false, false));
        coordinator.agent_capture_id = Some("a".into());
        coordinator.set_island_phase(IslandPhase::Recording);
        // finish_recording detaches A's ID before starting its finalizer.
        let finishing_id = coordinator.agent_capture_id.take().unwrap();
        coordinator.agent_captures.set(&finishing_id, "processing", None, None);
        coordinator.set_island_phase(IslandPhase::Processing);
        let generation_a = coordinator.capture_generation;

        // Native Escape uses cancel_recording: its state-only cancellation and
        // dismiss_island's phase transition revoke UI ownership, not A's worker.
        coordinator.cancel_capture_attempt();
        coordinator.set_island_phase(IslandPhase::Dismissing);
        coordinator.set_island_phase(IslandPhase::Hidden);
        assert_eq!(coordinator.agent_captures.inspect("a")["status"], "processing");
        assert!(coordinator.agent_captures.admit("b", false, false));
        coordinator.agent_capture_id = Some("b".into());
        coordinator.onboarding.as_mut().unwrap().input(Input::ShortcutTriggered);
        coordinator.set_island_phase(IslandPhase::Recording);
        coordinator.agent_captures.set("b", "recording", None, None);
        if processing_b {
            // Also cover B already finalizing: a phase-only guard must not accept A.
            coordinator.agent_capture_id.take();
            coordinator.agent_captures.set("b", "processing", None, None);
            coordinator.set_island_phase(IslandPhase::Processing);
        }
        let state_b = coordinator.agent_state();
        let receipt_b = coordinator.agent_captures.inspect("b");
        let generation_b = coordinator.capture_generation;
        coordinator.completed_dir = Some(PathBuf::from("/synthetic/b-review.apparent"));
        coordinator.completed_note = Some("B's note".into());
        coordinator.review_bundle = Some((PathBuf::from("/synthetic/b-review.apparent"), bundle));
        let review_b = coordinator.review_bundle.clone();

        let mut completed = Vec::new();
        coordinator.receive_capture_result(generation_a, Some(&finishing_id), feedback, deferred, |dir| {
            completed.push(dir.to_path_buf());
        }, |_, _| {
            panic!("stale A must not enter UI/clipboard/onboarding effects");
        });
        assert_eq!(completed, if succeeded && !feedback { vec![exact_path.clone()] } else { vec![] });
        let receipt_a = coordinator.agent_captures.inspect("a");
        if succeeded {
            assert_eq!(receipt_a, serde_json::json!({
                "capture_id": "a", "status": "saved", "bundle_path": exact_path, "error": null
            }));
        } else {
            assert_eq!(receipt_a, serde_json::json!({
                "capture_id": "a", "status": "failed", "bundle_path": null,
                "error": "finalization_failed", "message": "A's deferred finalizer failed"
            }));
        }
        assert_eq!(coordinator.agent_state(), state_b);
        assert_eq!(coordinator.agent_captures.inspect("b"), receipt_b);
        assert_eq!(coordinator.capture_generation, generation_b);
        assert_eq!(coordinator.completed_dir, Some(PathBuf::from("/synthetic/b-review.apparent")));
        assert_eq!(coordinator.completed_note.as_deref(), Some("B's note"));
        assert_eq!(coordinator.review_bundle, review_b);
        assert_eq!(coordinator.onboarding.as_mut().unwrap().action("continue"), Effect::None);
    }

    #[test]
    fn current_finalizer_completes_once_before_ui_for_agent_and_manual_captures() {
        for owned in [false, true] {
            for feedback in [false, true] {
                let (sender, _receiver) = flume::unbounded();
                let mut coordinator = ApparentCoordinator::new(SharedCaptureTelemetry::new(sender), None);
                coordinator.set_island_phase(IslandPhase::Processing);
                let generation = coordinator.capture_generation;
                if owned {
                    assert!(coordinator.agent_captures.admit("a", false, false));
                    coordinator.agent_captures.set("a", "processing", None, None);
                }
                let bundle = serde_json::from_value(serde_json::json!({
                    "version": 1, "id": "a", "createdAtMs": 0, "durationMs": 1000,
                    "target": {"mode": "display", "displayId": 0,
                        "frame": {"x": 0.0, "y": 0.0, "width": 640.0, "height": 480.0, "scale": 1.0}},
                    "transcript": {"source": "test", "text": "", "segments": []},
                    "strokes": [], "marks": [], "events": [], "keyframes": [], "contactSheet": "c.png"
                })).unwrap();
                let path = PathBuf::from("/synthetic/current-exact.apparent");
                let effects = std::cell::RefCell::new(Vec::new());
                coordinator.receive_capture_result(generation, owned.then_some("a"), feedback,
                    Ok((path.clone(), bundle, String::new())),
                    |dir| { assert_eq!(dir, path); effects.borrow_mut().push("completed"); },
                    |coord, result| {
                        assert_eq!(result.unwrap().0, path);
                        if owned {
                            assert_eq!(coord.agent_captures.inspect("a")["status"], "saved");
                        }
                        effects.borrow_mut().push("ui");
                    });
                assert_eq!(*effects.borrow(), if feedback { vec!["ui"] } else { vec!["completed", "ui"] });
            }
        }
    }

    #[test]
    fn hosted_settings_late_login_cannot_erase_newer_cancellation() {
        use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
        let (sender, _receiver) = flume::unbounded();
        let mut coord = ApparentCoordinator::new(SharedCaptureTelemetry::new(sender), None);
        let a = Arc::new(AtomicBool::new(true));
        coord.share_login = Some(a.clone());
        coord.cancel_share_login(); // Close Settings A.
        assert!(!a.load(Ordering::SeqCst));
        let b = Arc::new(AtomicBool::new(true));
        coord.share_login = Some(b.clone()); // Reopen Settings and start B.
        let mut worker_starts = 0;
        let result = crate::connect::hosted::hosted_tests::deferred_cancelled_settings_login(b.clone(), || {
            worker_starts += usize::from(coord.finish_share_login(&a, false));
            coord.cancel_share_login(); // Close Settings B before approval.
        });
        worker_starts += usize::from(coord.finish_share_login(&b, result.is_ok()));
        assert!(!b.load(Ordering::SeqCst));
        assert!(result.is_err());
        assert_eq!(worker_starts, 0, "cancelled completion started the upload worker");
        assert!(coord.share_login.is_none());
        let c = Arc::new(AtomicBool::new(true));
        coord.share_login = Some(c.clone());
        assert!(!coord.finish_share_login(&a, true), "stale success must not start a worker");
        assert!(Arc::ptr_eq(coord.share_login.as_ref().unwrap(), &c));
        assert!(coord.finish_share_login(&c, true), "the current approved login still starts its worker");
        assert!(!coord.finish_share_login(&c, true), "duplicate completion must not restart a worker");
    }

    #[test]
    fn failed_hidden_shortcut_attempt_cannot_lend_eligibility_to_menu_capture() {
        use crate::capture::desktop::settings::onboarding::{Effect, Flow, Grants, Input, Permission, Session, Status, Step};
        let (sender, _receiver) = flume::unbounded();
        let mut coordinator = ApparentCoordinator::new(SharedCaptureTelemetry::new(sender), None);
        let grants = Grants {
            statuses: Permission::REQUIRED.into_iter().map(|p| (p, Status::Granted)).collect(),
            ..Default::default()
        };
        coordinator.onboarding = Some(Session::for_test(
            Flow::tour(Step::TryIt, Some(grants)).with_first_capture(true, true, false),
        ));
        coordinator.onboarding.as_mut().unwrap().input(Input::ShortcutTriggered);
        // The same cancellation path used by journal/startup failure, before any island appears.
        coordinator.cancel_capture_attempt();
        assert!(matches!(coordinator.island_phase, IslandPhase::Hidden));
        assert!(coordinator.active_recording.is_none());
        // A later menu recording has no ShortcutTriggered event.
        coordinator.set_island_phase(IslandPhase::Recording);
        coordinator.set_island_phase(IslandPhase::Processing);
        coordinator.set_island_phase(IslandPhase::Completed {
            finished_at: Instant::now(), frame_count: 1, duration_ms: 1000, inbox_count: 0,
        });
        assert_eq!(coordinator.onboarding.as_mut().unwrap().action("continue"), Effect::None);
    }

    #[test]
    fn stale_finalization_cannot_complete_a_new_practice_recording() {
        use crate::capture::desktop::settings::onboarding::{Effect, Flow, Grants, Input, Permission, Step};
        let (sender, _receiver) = flume::unbounded();
        let mut coordinator = ApparentCoordinator::new(SharedCaptureTelemetry::new(sender), None);
        let mut grants = Grants::default();
        for permission in Permission::REQUIRED {
            grants.statuses.insert(permission, crate::capture::desktop::settings::onboarding::Status::Granted);
        }
        let mut flow = Flow::tour(Step::TryIt, Some(grants))
            .with_first_capture(true, true, false);
        coordinator.set_island_phase(IslandPhase::Recording);
        let first = coordinator.capture_generation;
        coordinator.set_island_phase(IslandPhase::Processing);
        assert!(coordinator.accepts_capture_result(first));
        coordinator.set_island_phase(IslandPhase::Dismissing);
        // The detached worker is still running, but dismissal revokes its UI result.
        assert!(!coordinator.accepts_capture_result(first));
        coordinator.set_island_phase(IslandPhase::Hidden);
        flow.on(Input::ShortcutTriggered);
        assert_eq!(flow.on(Input::CaptureStarted), Effect::Hide);
        coordinator.set_island_phase(IslandPhase::Recording);
        let second = coordinator.capture_generation;
        let deliver = |coordinator: &mut ApparentCoordinator, flow: &mut Flow, generation| {
            if coordinator.accepts_capture_result(generation) {
                coordinator.set_island_phase(IslandPhase::Completed {
                    finished_at: Instant::now(), frame_count: 1, duration_ms: 1000, inbox_count: 0,
                });
                flow.on(Input::CaptureEnded { completed: true });
            }
        };
        deliver(&mut coordinator, &mut flow, first);
        assert!(matches!(coordinator.island_phase, IslandPhase::Recording));
        assert_eq!(flow.step(), Step::TryIt);
        coordinator.set_island_phase(IslandPhase::Processing);
        deliver(&mut coordinator, &mut flow, first);
        assert!(coordinator.is_processing());
        assert_eq!(flow.step(), Step::TryIt);
        deliver(&mut coordinator, &mut flow, second);
        assert_eq!(flow.step(), Step::Done);
        assert!(!coordinator.accepts_capture_result(second), "a duplicate result is stale too");
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn failed_capture_start_discards_its_identity_without_an_active_recording() {
        let (sender, _receiver) = flume::unbounded();
        let mut coordinator = ApparentCoordinator::new(SharedCaptureTelemetry::new(sender), None);
        coordinator.agent_capture_id = Some("pending-capture".into());

        assert!(coordinator.discard_recording().is_none());
        assert_eq!(coordinator.agent_state().capture_id, None);
    }

    #[test]
    fn feedback_issue_url_opens_a_public_template_without_capture_data() {
        assert_eq!(
            feedback_issue_url(),
            "https://github.com/tryapparent/apparent/issues/new?template=bug_report.yml"
        );
    }

    #[test]
    fn capture_failure_status_survives_inbox_changes() {
        let failure = Some("Capture failed — no screen frames".to_owned());
        assert_eq!(status_bar_text(None, failure.clone()), failure);
        assert_eq!(status_bar_text(Some("Exporting…".into()), failure), Some("Exporting…".into()));
    }

    #[test]
    fn picker_exclusion_survives_session_reset_until_deferred_close() {
        // reset_for_session clears the prior session's IDs before the initial
        // ScreenCaptureKit filter is built, while the picker is still open.
        let mut excluded = Vec::new();
        restore_picker_exclusion(&mut excluded, Some(41));
        restore_picker_exclusion(&mut excluded, Some(41));
        restore_picker_exclusion(&mut excluded, None);
        assert_eq!(excluded, vec![41]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn recording_equalizer_is_available_without_an_experiment() {
        assert!(recording_equalizer_enabled());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn equalizer_only_moves_for_new_samples_and_decays_on_stall() {
        let mut meter = InputEqualizer::default();
        meter.step(None, false); // lab recording / missing microphone
        assert_eq!(meter.level, 0.0);
        meter.step(Some((1280, 0.8)), false);
        assert!(meter.level > 0.0 && meter.level < 0.8);
        let last = meter.level;
        meter.step(Some((1280, 0.8)), false); // same samples, not proof of continued input
        assert!(meter.level < last);
        meter.step(Some((2560, 0.0)), true); // fresh silent callback
        assert_eq!(meter.level, 0.0);
        meter.step(Some((128, 0.5)), true); // new capture resets count
        assert_eq!(meter.level, 0.5);
    }

    #[test]
    fn completed_capture_distinguishes_missing_audio_from_no_words_and_model_failure() {
        let empty = crate::shared::model::TranscriptData::default();
        let spoken = crate::shared::model::TranscriptData {
            text: "hello".into(),
            ..Default::default()
        };
        let skipped = crate::shared::model::TranscriptData {
            source: "none".into(),
            ..Default::default()
        };
        let unavailable = crate::shared::model::TranscriptData {
            missing: Some(crate::shared::model::MissingTranscript {
                reason: "Model unavailable".into(),
                retry: true,
            }),
            ..Default::default()
        };
        assert_eq!(
            completed_audio_issue(None, &empty, false),
            Some(CompletedAudioIssue::NoAudio)
        );
        assert_eq!(
            completed_audio_issue(None, &empty, true),
            Some(CompletedAudioIssue::NoWords)
        );
        assert_eq!(
            completed_audio_issue(None, &unavailable, true),
            Some(CompletedAudioIssue::TranscriptionUnavailable)
        );
        assert_eq!(
            completed_audio_issue(None, &unavailable, false),
            Some(CompletedAudioIssue::NoAudio)
        );
        assert_eq!(
            completed_audio_issue(None, &skipped, false),
            Some(CompletedAudioIssue::NoAudio)
        );
        assert_eq!(
            completed_audio_issue(None, &skipped, true),
            Some(CompletedAudioIssue::TranscriptionUnavailable)
        );
        assert_eq!(completed_audio_issue(Some("iphone"), &empty, false), None);
        assert_eq!(completed_audio_issue(Some("iphone"), &empty, true), None);
        assert_eq!(completed_audio_issue(None, &spoken, true), None);
        assert_eq!(
            completed_audio_issue(None, &spoken, false),
            Some(CompletedAudioIssue::NoAudio)
        );
        assert_eq!(
            completed_status_label(0, None, Some(CompletedAudioIssue::NoAudio)),
            "Copied · No audio"
        );
        assert_eq!(
            completed_status_label(0, None, Some(CompletedAudioIssue::NoWords)),
            "Copied · No words"
        );
        assert_eq!(
            completed_status_label(0, None, Some(CompletedAudioIssue::TranscriptionUnavailable)),
            "Copied · Transcript unavailable"
        );
    }

    #[test]
    fn review_history_stops_at_both_ends() {
        let paths = vec![
            PathBuf::from("new"),
            PathBuf::from("middle"),
            PathBuf::from("old"),
        ];
        assert_eq!(review_neighbor(&paths, &paths[0], true), Some(&paths[1]));
        assert_eq!(review_neighbor(&paths, &paths[1], false), Some(&paths[0]));
        assert_eq!(review_neighbor(&paths, &paths[2], true), None);
        assert_eq!(review_neighbor(&paths, &paths[0], false), None);
        assert_eq!(
            review_neighbor(&paths, &PathBuf::from("elsewhere"), true),
            None
        );
    }

    #[test]
    fn review_navigation_failure_takes_priority_over_history_scan_failure() {
        assert_eq!(
            review_notice(
                Some("scan failed"),
                Some("Could not open capture: removed"),
                false,
            ),
            Some("Could not open capture: removed".to_owned())
        );
        assert_eq!(
            review_notice(Some("scan failed"), Some("Ready"), false),
            Some("Could not read captures: scan failed".to_owned())
        );
    }

    #[test]
    fn feedback_review_displays_privacy_notice_and_browser_errors() {
        let warning = "Review before sharing publicly. Nothing is uploaded automatically.";
        assert_eq!(
            review_notice(None, Some(warning), true).as_deref(),
            Some(warning)
        );
        let error = "Could not open GitHub: browser unavailable";
        assert_eq!(
            review_notice(None, Some(error), true).as_deref(),
            Some(error)
        );
    }

    #[test]
    fn review_workspace_gives_words_reading_room_without_cropping_the_stage() {
        let (stage, words) = review_workspace_widths(1120.0);
        assert!(words >= 420.0, "words panel too narrow: {words}");
        assert!(stage >= 600.0, "image stage too narrow: {stage}");
        let (stage, words) = review_workspace_widths(760.0);
        assert_eq!((stage, words), (720.0, 760.0));
        let (stage, words) = review_workspace_widths(320.0);
        assert_eq!((stage, words), (280.0, 320.0));
    }

    #[test]
    fn selected_frame_speech_follows_selection_not_the_whole_capture() {
        let frames = [Some("first moment"), Some("second moment")];
        assert_eq!(review_frame_speech(&frames, 1), "second moment");
        assert_eq!(review_frame_speech(&frames, 2), "No words at this moment.");
        assert_eq!(review_speech_status(false, None), None);
        assert!(
            review_speech_status(true, None)
                .unwrap()
                .contains("audio was saved")
        );
        assert_eq!(
            review_speech_status(false, Some("speech failed")),
            Some("Transcription unavailable: speech failed".into())
        );
    }

    #[test]
    fn recording_controls_offer_whiteboard_on_every_desktop() {
        assert!(whiteboard_enabled());
        let controls = trail_keys(false, false, &[], "hud-");
        assert_eq!(
            controls.iter().any(|item| item
                .as_ref()
                .is_some_and(|(name, _)| name == "hud-whiteboard")),
            true
        );
    }

    #[test]
    fn island_geometry_is_concentric() {
        assert_eq!(ISLAND_RADIUS, CHIP_RADIUS + ISLAND_INSET);
        assert_eq!(ISLAND_PAD + 1.0, ISLAND_INSET);
        assert_eq!(CHIP_H + 2.0 * ISLAND_INSET, ISLAND_H);
        assert!(ISLAND_H <= PILL_WINDOW_H);
    }

    #[test]
    fn a_held_drag_needs_room_for_an_area() {
        let laptop = [0.0, 0.0, 1728.0, 1117.0];
        assert_eq!(
            area_rect([100.0, 80.0], [40.0, 20.0], laptop),
            Some([40.0, 20.0, 60.0, 60.0])
        );
        assert_eq!(area_rect([0.0, 0.0], [30.0, 200.0], laptop), None);
    }

    #[test]
    fn recording_controls_follow_the_pointer_without_retargeting_the_capture() {
        let first = crate::capture::engine::runtime::screens::Screen {
            id: 1,
            frame: [0.0, 0.0, 1920.0, 1200.0],
            visible: [0.0, 0.0, 1920.0, 1200.0],
            scale: 1.25,
            safe_top: 0.0,
        };
        let second = crate::capture::engine::runtime::screens::Screen {
            id: 2,
            frame: [1920.0, 0.0, 1280.0, 800.0],
            visible: [1920.0, 0.0, 1280.0, 800.0],
            scale: 1.0,
            safe_top: 0.0,
        };
        let screens = [first, second];
        let selected_capture = second.frame;
        assert_eq!(
            recording_pointer_destination(second.frame, [2400.0, 300.0], &screens),
            None
        );
        assert_eq!(
            recording_pointer_destination(second.frame, [350.0, 300.0], &screens),
            Some(first.frame)
        );
        assert_eq!(
            recording_pointer_destination(first.frame, [350.0, 300.0], &screens),
            None
        );
        assert_eq!(
            recording_pointer_destination(first.frame, [2400.0, 300.0], &screens),
            Some(second.frame)
        );
        assert_eq!(
            recording_pointer_destination(second.frame, [3300.0, 300.0], &screens),
            None
        );
        assert_eq!(
            selected_capture, second.frame,
            "the selected capture must not move with the UI"
        );
    }

    /// Kit's review: a drag that starts at x -150 on a left display and overshoots to x 160 on
    /// the laptop keeps to the left display, so it's recorded there (`screens::area_on_display`).
    #[test]
    fn an_area_drag_stays_on_the_display_it_started_on() {
        let left = [-1920.0, 37.0, 1920.0, 1080.0];
        assert_eq!(
            area_rect([-150.0, 200.0], [160.0, 400.0], left),
            Some([-150.0, 200.0, 150.0, 200.0])
        );
        // Past its bottom-left corner too.
        assert_eq!(
            area_rect([-1800.0, 1000.0], [-2100.0, 1300.0], left),
            Some([-1920.0, 1000.0, 120.0, 117.0])
        );
        // What's left on the display can be too small to record.
        assert_eq!(area_rect([-20.0, 200.0], [400.0, 400.0], left), None);
        let screens = [
            crate::capture::engine::runtime::screens::Screen {
                id: 1,
                frame: [0.0, 0.0, 1728.0, 1117.0],
                visible: [0.0, 0.0, 1728.0, 1117.0],
                scale: 2.0,
                safe_top: 0.0,
            },
            crate::capture::engine::runtime::screens::Screen {
                id: 2,
                frame: left.map(f64::from),
                visible: left.map(f64::from),
                scale: 1.0,
                safe_top: 0.0,
            },
        ];
        let area = area_rect([-150.0, 200.0], [160.0, 400.0], left).unwrap();
        let (i, rect) = crate::capture::engine::runtime::screens::area_on_display(&screens, area).unwrap();
        assert_eq!((screens[i].id, rect), (2, area));
    }

    #[test]
    fn selection_modes_cycle_in_hud_order() {
        use SelectionMode::{Area, Iphone, Screen, Window};
        let walk = |has_iphone: bool, forward: bool| {
            let mut mode = Area;
            let mut seen = Vec::new();
            for _ in 0..5 {
                mode = mode.cycled(forward, has_iphone);
                seen.push(mode);
            }
            seen
        };
        if cfg!(target_os = "macos") {
            assert_eq!(walk(true, true), [Screen, Iphone, Area, Screen, Iphone]);
            assert_eq!(walk(true, false), [Iphone, Screen, Area, Iphone, Screen]);
        } else {
            // No cabled iPhone off macOS, even when one is plugged in.
            assert_eq!(walk(true, true), walk(false, true));
        }
        assert_eq!(walk(false, true), [Screen, Area, Screen, Area, Screen]);
        assert_eq!(walk(false, false), [Screen, Area, Screen, Area, Screen]);
        assert!(!SelectionMode::available(true).contains(&Window));
        assert!(!SelectionMode::available(false).contains(&Window));
        assert_eq!(Window.cycled(true, false), Area);
        assert_eq!(Window.cycled(false, false), Screen);
        // An unplugged phone leaves the picker on a mode that is no longer listed.
        assert_eq!(Iphone.cycled(true, false), Area);
        assert_eq!(Iphone.cycled(false, false), Screen);
    }

    #[test]
    fn digits_pick_modes_by_hud_position() {
        use SelectionMode::{Area, Iphone, Screen};
        let pick = |key, has_iphone| SelectionMode::from_digit(key, has_iphone);
        assert_eq!(pick("1", true), Some(Area));
        assert_eq!(pick("2", true), Some(Screen));
        assert_eq!(pick("3", false), None);
        assert_eq!(pick("3", true), cfg!(target_os = "macos").then_some(Iphone));
        assert_eq!(pick("4", true), None);
        assert_eq!(pick("4", false), None);
        assert_eq!(pick("0", true), None);
        assert_eq!(pick("5", true), None);
        assert_eq!(pick("a", true), None);
    }

    #[test]
    fn selecting_row_width_mirrors_layout() {
        let segment = |label: f32| label + 2.0 * SEG_PX + SEG_ICON + SEG_ICON_GAP;
        let divider = 1.0 + 2.0 * DIVIDER_MX;
        // Three segments, divider, ✕: four gaps between five items.
        assert_eq!(
            selecting_row_width(&[30.0, 50.0, 45.0], None),
            segment(30.0) + segment(50.0) + segment(45.0) + divider + CHIP_H + 4.0 * CHIP_GAP
        );
        // A fourth segment and the Record chip each add their width and one gap.
        assert_eq!(
            selecting_row_width(&[30.0, 50.0, 45.0, 42.0], Some(70.0)),
            selecting_row_width(&[30.0, 50.0, 45.0], None) + segment(42.0) + 70.0 + 2.0 * CHIP_GAP
        );
        // The widest row (generous label estimates) still fits the pill window.
        let widest = selecting_row_width(&[34.0, 56.0, 50.0, 46.0], Some(2.0 * CHIP_PX + 52.0));
        assert!(
            widest + 2.0 * ISLAND_INSET <= PILL_WINDOW_W - 16.0,
            "{widest}"
        );
    }

    #[test]
    fn island_spring_settles_without_large_overshoot() {
        let (mut x, mut v) = (200.0_f32, 0.0_f32);
        let mut peak = x;
        for _ in 0..90 {
            (x, v) = spring_step(x, v, 300.0, 1.0 / 60.0, motion::Tune::DEFAULT.island);
            peak = peak.max(x);
        }
        assert!((x - 300.0).abs() < 0.5, "settled at {x}");
        assert!(peak < 300.0 + 100.0 * 0.05, "overshoot peak {peak}");
    }

    #[test]
    fn reveal_springs_lead_the_morph_palette_and_settle() {
        let t = motion::Tune::DEFAULT;
        // Width leads morph .42/.16 open and .38/.08 close on a slightly faster spring.
        assert!(t.reveal_open.response < 0.42);
        assert!(t.reveal_close.response < 0.38);
        assert!(t.reveal_close.response < t.reveal_open.response);
        assert!((t.reveal_open.bounce - 0.16).abs() < 1e-6);
        assert!(t.reveal_exit_s < t.reveal_enter_s && t.reveal_exit_s >= 0.75 * t.reveal_enter_s);
        assert!(t.reveal_blur_px <= 4.0);
        for (spring, from, to) in [
            (t.reveal_open, ISLAND_OPEN_FROM_W, 430.0),
            (t.reveal_close, 430.0, 330.0),
        ] {
            let (mut x, mut v) = (from, 0.0_f32);
            for _ in 0..60 {
                (x, v) = spring_step(x, v, to, 1.0 / 60.0, spring);
            }
            assert!((x - to).abs() < 0.5, "settled at {x}");
        }
    }

    #[test]
    fn trailing_items_fade_in_as_the_edge_uncovers_them() {
        let offsets = trail_offsets(100.0, &[50.0, 60.0]);
        assert_eq!(offsets, vec![100.0, 100.0 + 50.0 + CHIP_GAP]);
        assert_eq!(edge_alpha(100.0, 100.0, 50.0), 0.0);
        assert!((edge_alpha(125.0, 100.0, 50.0) - 0.5).abs() < 1e-6);
        assert_eq!(edge_alpha(150.0, 100.0, 50.0), 1.0);
        assert_eq!(edge_alpha(400.0, 100.0, 50.0), 1.0);
    }

    #[test]
    fn a_switch_chip_morph_starts_where_the_pill_was_and_rests_centred() {
        let (width, target, clock_w) = (336.0, 298.0, 35.0);
        let start = SwitchLayout::new(SwitchMorph { slide: None }, width, target, clock_w);
        // The lead is still pinned to the left edge and the chips start behind its divider.
        assert!(start.ghost_left().abs() < 1e-3, "{}", start.ghost_left());
        let chips_x = CHIP_H + clock_w + LEAD_TRAIL + 2.0 * CHIP_GAP + 1.0 + 2.0 * DIVIDER_MX;
        assert!((start.row_left() - chips_x).abs() < 1e-3);
        assert_eq!(start.ghost_alphas(), [1.0, 1.0, 1.0]);
        // At rest the picker row is centred in the capsule, the lead past its edge and gone.
        let rest = SwitchLayout::new(
            SwitchMorph {
                slide: Some((0.0, 0.0)),
            },
            target,
            target,
            clock_w,
        );
        assert!(rest.row_left().abs() < 1e-3);
        assert_eq!(rest.ghost_alphas(), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn island_spring_is_stable_for_long_frames() {
        let (x, _) = spring_step(200.0, 0.0, 300.0, 0.05, motion::Tune::DEFAULT.island);
        assert!(x > 200.0 && x < 300.0);
    }

    #[test]
    fn area_badge_sits_below_and_flips_near_edges() {
        let screen = [1440.0, 900.0];
        let badge = [90.0, BADGE_H];
        let [bx, by] = place_badge(
            [100.0, 100.0, 300.0, 200.0],
            badge,
            screen,
            BadgeAnchor::BelowTrailing,
        );
        assert_eq!([bx, by], [310.0, 308.0]);

        let [_, by] = place_badge(
            [100.0, 600.0, 300.0, 290.0],
            badge,
            screen,
            BadgeAnchor::BelowTrailing,
        );
        assert_eq!(by, 600.0 - BADGE_GAP - BADGE_H);

        let [bx, by] = place_badge(
            [0.0, 0.0, 40.0, 900.0],
            badge,
            screen,
            BadgeAnchor::BelowTrailing,
        );
        assert_eq!(bx, BADGE_MARGIN);
        assert!(by + BADGE_H <= 900.0 - BADGE_MARGIN);
    }

    #[test]
    fn window_badge_centers_and_clamps_on_screen() {
        let screen = [1440.0, 900.0];
        let [bx, by] = place_badge(
            [200.0, 100.0, 600.0, 400.0],
            [200.0, BADGE_H],
            screen,
            BadgeAnchor::Center,
        );
        assert_eq!([bx, by], [400.0, 289.0]);

        let clipped = clip_rect([-300.0, -50.0, 400.0, 200.0], screen);
        assert_eq!(clipped, [0.0, 0.0, 100.0, 150.0]);
    }

    #[test]
    fn timeline_lines_split_into_time_and_body() {
        assert_eq!(
            split_timeline_line("0:04 ✎ Circle around button — frame #2"),
            ("0:04", "Circle around button — frame #2", true)
        );
        assert_eq!(
            split_timeline_line("0:05 · Clicked Save"),
            ("0:05", "Clicked Save", false)
        );
        assert_eq!(
            split_timeline_line("0:01 “hello”"),
            ("0:01", "“hello”", false)
        );
    }

    #[test]
    fn last_used_mode_keys_round_trip() {
        for mode in SelectionMode::ALL {
            assert_eq!(SelectionMode::from_key(mode.key()), Some(mode));
        }
        assert_eq!(
            SelectionMode::from_key("iphone\n"),
            Some(SelectionMode::Iphone)
        );
        assert_eq!(SelectionMode::from_key("fullscreen"), None);
        assert_eq!(SelectionMode::from_key("window"), None);
    }

    #[test]
    fn clock_template_is_width_stable() {
        assert_eq!(clock_template("12:34"), "00:00");
        assert_eq!(completed_label(0, None), "Copied");
        assert_eq!(
            completed_label(0, Some("Sent to APPARENT")),
            "Sent to APPARENT"
        );
        assert_eq!(completed_label(1, Some("Sent to APPARENT")), "From iPhone");
        assert_eq!(completed_label(3, None), "3 from iPhone");
        assert_eq!(
            completed_status_label(0, None, Some(CompletedAudioIssue::NoWords)),
            "Copied · No words"
        );
        assert_eq!(
            completed_status_label(
                0,
                Some("Sent to APPARENT"),
                Some(CompletedAudioIssue::NoWords)
            ),
            "Sent to APPARENT · No words"
        );
        assert_eq!(completed_status_label(0, None, None), "Copied");
        assert_eq!(
            completed_status_label(
                0,
                Some("Saved · review"),
                Some(CompletedAudioIssue::NoAudio)
            ),
            "Saved · review · No audio"
        );
    }
}
