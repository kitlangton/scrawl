pub(crate) use ui::coordinator::capsule::{Spring, ease_out_cubic};
pub use ui::coordinator::hud::HudPillWindow;
pub use ui::coordinator::picker::TargetPickerWindow;
pub use ui::coordinator::review::BundleReviewWindow;
use ui::coordinator::*;

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
#[cfg(target_os = "linux")]
pub(crate) use platform::set_app_icon;
use platform::{
    PillChrome, install_status_bar, pointer_on_hud_pill, reduce_motion, set_status_inbox_count,
    set_status_text,
};
#[cfg(target_os = "windows")]
pub(crate) use platform::{gpui_display_for, hwnd_of, set_app_icon};
#[cfg(target_os = "linux")]
use ui::linux as platform;
#[cfg(target_os = "macos")]
use ui::macos as platform;
#[cfg(target_os = "windows")]
use ui::windows as platform;
use ui::{desktop_completion, lab, motion, pill_theme, recording_completion, whiteboard};

use crate::capture::desktop::helper::PhoneDevice;
#[cfg(target_os = "linux")]
use crate::capture::desktop::linux::overlay::InkOverlay as LaserOverlay;
#[cfg(target_os = "macos")]
use crate::capture::desktop::native::laser_overlay::MetalLaserOverlay as LaserOverlay;
use crate::capture::engine::runtime::audio::audio::{AudioRecorder, InputWarning};
use crate::capture::engine::runtime::phone::PhoneSession;
#[cfg(target_os = "windows")]
use crate::capture::engine::runtime::platform::legacy_windows::capture::overlay::InkOverlay as LaserOverlay;
use crate::capture::engine::runtime::{
    LiveOverlayEvent, ScreenCaptureSession, SharedCaptureTelemetry, apparent_log,
    current_mouse_screen, discover_target, install_global_hotkeys, list_selectable_windows,
    record_external_frontmost_app, region_target, selectable_window_at, shortcut_from_key_and_mods,
    start_global_input_tap, update_global_shortcut,
};
use crate::capture::engine::sketch::timeline;
use crate::shared::model::{ApparentBundle, CaptureMode, TargetFrame, TargetInfo, format_clock};

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

/// The build identity compiled into the running backend, shared by native UI surfaces.
pub(super) fn running_build_label() -> String {
    let full = env!("APPARENT_VERSION_LABEL");
    full.split_once(" · ")
        .map(|(_, revision)| revision.to_owned())
        .unwrap_or_else(|| "build unknown".to_owned())
}

/// Small, noninteractive marker for owner-local GPUI windows and overlays.
pub(super) fn running_build_badge() -> Div {
    div()
        .absolute()
        .top(px(5.0))
        .right(px(5.0))
        .px(px(5.0))
        .py(px(2.0))
        .rounded(px(5.0))
        .bg(rgba(0x17151bcc))
        .text_size(px(10.0))
        .text_color(rgb(0xf3eff8))
        .font_family("monospace")
        .child(running_build_label())
}

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
    microphone_enabled: bool,
}

impl ActiveRecording {
    fn mic_warning(&self) -> Option<InputWarning> {
        if !self.microphone_enabled {
            return None;
        }
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
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) native_host_epoch: String,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) native_host_revision: u64,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) native_host_fingerprint: Option<String>,
    note_ui: Option<workflow::notes::Session>,
    admitted_microphone: Option<bool>,
    whiteboard: Rc<RefCell<whiteboard::Board>>,
    active_recording: Option<ActiveRecording>,
    /// Identifies the recording whose finalization may update the current UI.
    capture_generation: u64,
    capture_local_practice: bool,
    /// Media workers outlive their capsule and still own unfinished bundles.
    pending_finalizers: usize,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    agent_capture_id: Option<String>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    agent_captures: crate::capture::desktop::workflow::agent_capture::Registry,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    public_share: crate::capture::desktop::workflow::public_sharing::Controller,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    public_popup: Option<crate::capture::desktop::workflow::public_sharing::popup::Popup>,
    #[cfg(target_os = "linux")]
    agent_finishing: bool,
    last_live_no_words: bool,
    playback: Option<crate::capture::desktop::playback::Guard>,
    picker_metal_overlay: Option<LaserOverlay>,
    native_pill: Option<crate::capture::desktop::native::pill::NativePill>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    electron_pill: Option<ui::electron_pill::Session>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    electron_pill_failed: bool,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    electron_pill_routed: bool,
    #[cfg(target_os = "macos")]
    electron_pill_exclusions: Vec<u32>,
    #[cfg(target_os = "linux")]
    electron_equalizer: InputEqualizer,
    iphone_preview: Option<PhoneSession>,
    presentation: CapsulePresentation,
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
    completed_upload_size_failure:
        Option<(PathBuf, u64, crate::connect::hosted::UploadSizeFailure)>,
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
        .inspect_err(|error| {
            apparent_log(format!("Graceful signal handling unavailable: {error:#}"))
        })
        .ok();
    workflow::error_reporting::install();
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
        // Warm the hidden Linux controls before the first capture. Capture
        // admission and shortcuts do not wait for renderer startup.
        #[cfg(target_os = "linux")]
        coordinator.update(cx, |coord, _| coord.ensure_electron_pill());
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
                                MenuAction::ToggleMicrophone => {
                                    let enabled = !crate::capture::engine::runtime::audio::mic::enabled();
                                    if let Err(error) = crate::shared::handoff::set_config_value_in(&crate::shared::handoff::default_config_path(), "microphoneEnabled", serde_json::json!(enabled)) {
                                        apparent_log(format!("Microphone preference was not saved: {error:#}"));
                                    }
                                }
                                MenuAction::ToggleCapture => {
                                    loop_coord.update(cx, |coord, cx| coord.record_or_finish(cx));
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
                            LiveOverlayEvent::NotesUnavailable => {
                                apparent_log("Typed notes UI exited; recording continues. See this capture's notes-ui.log.");
                                if let Ok(mut status) = CAPTURE_STATUS.lock() { *status = Some("Typed notes unavailable · recording continues".into()); }
                            }
                            LiveOverlayEvent::NoteClosed { focus, owner } => platform::restore_note_focus(focus, owner),
                            LiveOverlayEvent::NoteOutside => { if let Some(ui) = coord.note_ui.as_ref() { ui.outside(); } },
                            LiveOverlayEvent::NoteResumeDrawing => { if let Some(ui) = coord.note_ui.as_ref() { ui.resume_drawing(); } },
                            LiveOverlayEvent::Note { pin, t, x, y, screen_x, screen_y } => {
                                coord.request_immediate_frame();
                                let (focus, owner) = platform::note_focus();
                                if let Some(ui) = &coord.note_ui { ui.open(workflow::notes::Anchor { pin, t, x, y, screen_x, screen_y, focus, owner }); }
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
