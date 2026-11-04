use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Spring {
    /// Visual duration in seconds.
    pub(in crate::capture::desktop) response: f32,
    pub(in crate::capture::desktop) bounce: f32,
}

impl Spring {
    /// Kit's duration + bounce: ω = 2π / duration, ζ = 1 − bounce.
    pub(in crate::capture::desktop) const fn visual(duration: f32, bounce: f32) -> Self {
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
pub(in crate::capture::desktop) enum Morph {
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
    pub(in crate::capture::desktop) fn spring(self, tune: &motion::Tune) -> Spring {
        match self {
            Self::Phase => tune.island,
            Self::RevealOpen => tune.reveal_open,
            Self::RevealClose => tune.reveal_close,
            Self::Switch => tune.switch_width,
        }
    }
}

/// Reduced motion: no width travel, the groups only cross-fade over this long.
pub(in crate::capture::desktop) const REDUCED_FADE_S: f32 = 0.18;
/// The picker's highlight moving from one mode to another cross-fades over this long.
pub(in crate::capture::desktop) const HIGHLIGHT_FADE_S: f32 = 0.18;

/// A switch chip turning the recording into the picker (`switch_recording`). The capsule stays
/// one shape: the chips are the picker's chips, so the picker row starts where they were and
/// slides to its place on `Tune::switch_slide` while the width springs on `Tune::switch_width`
/// (a touch faster, so it leads). The lead (dot and clock, frozen) and its divider ride along in
/// front of the row and leave under the capsule's left edge; Record, the divider and ✕ ride
/// behind it and fade in as the right edge uncovers them.
#[derive(Clone, Copy, Debug)]
pub(in crate::capture::desktop) struct SwitchMorph {
    /// The row's offset from its resting place and its velocity, in points. Measured by the
    /// first frame that draws it (text widths need the window).
    pub(in crate::capture::desktop) slide: Option<(f32, f32)>,
}

/// Advances a damped spring with fixed 4ms sub-steps so long frames stay stable.
pub(in crate::capture::desktop) fn spring_step(
    value: f32,
    velocity: f32,
    target: f32,
    dt: f32,
    spring: Spring,
) -> (f32, f32) {
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

pub(in crate::capture::desktop) fn with_alpha(color: u32, alpha: f32) -> gpui::Rgba {
    rgba((color << 8) | (alpha.clamp(0.0, 1.0) * 255.0).round() as u32)
}

pub(crate) fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

pub(in crate::capture::desktop) fn tabular_nums() -> gpui::FontFeatures {
    gpui::FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

pub(in crate::capture::desktop) fn tabular_font(weight: FontWeight) -> gpui::Font {
    gpui::Font {
        family: ".SystemUIFont".into(),
        features: tabular_nums(),
        fallbacks: None,
        weight,
        style: gpui::FontStyle::Normal,
    }
}

pub(in crate::capture::desktop) fn measure_text(
    window: &Window,
    text: &str,
    size: f32,
    weight: FontWeight,
) -> f32 {
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

pub(in crate::capture::desktop) fn chip_width(
    window: &Window,
    label: &str,
    weight: FontWeight,
) -> f32 {
    measure_text(window, label, HUD_TEXT, weight) + 2.0 * CHIP_PX
}

pub(in crate::capture::desktop) fn clock_template(clock: &str) -> String {
    clock
        .chars()
        .map(|c| if c.is_ascii_digit() { '0' } else { c })
        .collect()
}

/// Lead text of the Completed island: local captures were copied (or sent, as `note` says);
/// iPhone ones count the inbox.
pub(in crate::capture::desktop) fn completed_label(
    inbox_count: usize,
    note: Option<&str>,
) -> String {
    match inbox_count {
        0 => note.unwrap_or("Copied").to_owned(),
        1 => "From iPhone".to_owned(),
        n => format!("{n} from iPhone"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::capture::desktop) enum CompletedAudioIssue {
    NoAudio,
    NoWords,
    TranscriptionUnavailable,
}

impl CompletedAudioIssue {
    pub(in crate::capture::desktop) fn label(self) -> &'static str {
        match self {
            Self::NoAudio => "No audio",
            Self::NoWords => "No words",
            Self::TranscriptionUnavailable => "Transcript unavailable",
        }
    }
}

pub(in crate::capture::desktop) fn completed_audio_issue(
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

pub(in crate::capture::desktop) fn bundle_audio_issue(
    dir: &std::path::Path,
    bundle: &ApparentBundle,
) -> Option<CompletedAudioIssue> {
    if bundle
        .context
        .get("microphoneCapture")
        .is_some_and(|v| v == "off")
    {
        return None;
    }
    completed_audio_issue(
        bundle.source.as_deref(),
        &bundle.transcript,
        bundle.has_microphone_samples(dir),
    )
}

pub(in crate::capture::desktop) fn completed_status_label(
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
pub(in crate::capture::desktop) fn selecting_row_width(
    label_widths: &[f32],
    record_width: Option<f32>,
) -> f32 {
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

pub(in crate::capture::desktop) fn segment_width(window: &Window, mode: SelectionMode) -> f32 {
    measure_text(window, mode.label(), HUD_TEXT, FontWeight::MEDIUM)
        + 2.0 * SEG_PX
        + SEG_ICON
        + SEG_ICON_GAP
}

/// Widths of the items after a Recording capsule's divider: the switch chips while the reveal
/// is open (`crate::capture::desktop::settings::shortcut::Reveal`), else Cancel · Finish.
pub(in crate::capture::desktop) fn recording_trail_widths(
    window: &Window,
    chips: bool,
    iphone: bool,
) -> Vec<f32> {
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

pub(in crate::capture::desktop) fn whiteboard_enabled() -> bool {
    true
}

/// Trace names for a Recording capsule's trailing group, with each item's opacity.
pub(in crate::capture::desktop) fn trail_keys(
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
pub(in crate::capture::desktop) fn trail_offsets(x0: f32, widths: &[f32]) -> Vec<f32> {
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
pub(in crate::capture::desktop) fn edge_alpha(inner_now: f32, x: f32, w: f32) -> f32 {
    let t = ((inner_now - x) / w.max(1.0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The switch chips' clock: the animation clock, so they stay open as long in slow motion
/// (`motion::time_scale`).
pub(in crate::capture::desktop) fn reveal_clock_ms() -> u64 {
    (motion::now() * 1000.0) as u64
}

/// An item of the picker row sliding in from under the capsule's right edge (`outer`, the
/// shell's outer edge): invisible while the edge would still cut into its middle, opaque once
/// it is clear, so nothing shows half cut.
pub(in crate::capture::desktop) fn uncover_alpha(outer: f32, x: f32, w: f32) -> f32 {
    let inside = (outer - x) / w.max(1.0);
    let t = ((inside - 0.7) / 0.4).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Intrinsic outer width of the capsule for a phase, mirroring the layout in `HudPillWindow`.
/// `chips`: a Recording capsule showing its switch chips.
pub(in crate::capture::desktop) fn island_content_width(
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
                } else {
                    0.0
                }
                + chip_width(window, "Inspect", FontWeight::MEDIUM)
                + CHIP_H
                + 3.0 * CHIP_GAP
        }
        IslandPhase::Hidden | IslandPhase::Dismissing => return ISLAND_H,
    };
    inner + 2.0 * ISLAND_INSET
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::capture::desktop) enum MenuAction {
    ToggleCapture,
    ToggleMicrophone,
    ChooseTarget,
    OpenLatest,
    OpenInbox(PathBuf),
    ExportVideo,
    Settings,
    CheckForUpdates,
    Quit,
}

/// Text beside the menu bar icon while a video export runs (`None` goes back to the inbox count).
static EXPORT_STATUS: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
/// Remains visible beside the menu icon until the next recording; a failed capture has no paper tile.
pub(in crate::capture::desktop) static CAPTURE_STATUS: std::sync::Mutex<Option<String>> =
    std::sync::Mutex::new(None);
pub(in crate::capture::desktop) static EXPORTING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(in crate::capture::desktop) fn status_bar_text(
    export: Option<String>,
    capture: Option<String>,
) -> Option<String> {
    export.or(capture)
}

pub(in crate::capture::desktop) fn current_status_bar_text() -> Option<String> {
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

pub(in crate::capture::desktop) fn refresh_status_bar(inbox_count: usize) {
    if let Some(text) = current_status_bar_text() {
        set_status_text(&text);
    } else {
        set_status_inbox_count(inbox_count);
    }
}

/// Export the newest bundle off the main thread. Linux asks for a destination;
/// macOS keeps its automatic Movies/Apparent destination. Report and reveal the result.
pub(in crate::capture::desktop) fn start_video_export() {
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
            let result = crate::shared::handoff::load_bundle(std::path::Path::new("latest"))
                .and_then(|(dir, bundle)| {
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
                    crate::capture::engine::export::export(
                        &dir,
                        &bundle,
                        &options,
                        &mut |fraction| {
                            say(Some(format!("Exporting {}%", (fraction * 100.0) as u32)));
                        },
                    )
                    .map(Some)
                });
            match result {
                Ok(Some(exported)) => {
                    apparent_log(format!(
                        "Exported {}",
                        crate::capture::engine::export::describe(&exported)
                    ));
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
                    workflow::error_reporting::report_error(
                        workflow::error_reporting::Failure::VideoExport,
                        &error,
                    );
                    #[cfg(target_os = "linux")]
                    {
                        let detail = format!("{error:#}");
                        say(Some(format!(
                            "Export failed: {}",
                            detail.chars().take(120).collect::<String>()
                        )));
                        crate::capture::desktop::linux::shell::notify(
                            "Video export failed",
                            &detail,
                        );
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

impl ApparentCoordinator {
    /// Activity that means the user got to work: clicks, drags, scrolls and strokes. Clicks on
    /// Apparent's own windows (the capsule and its chips) are never recorded, so they don't count.
    pub(in crate::capture::desktop) fn reveal_activity(&self) -> usize {
        use std::sync::PoisonError;
        let t = &self.telemetry;
        // Each guard must end before taking the next lock. Input commits a
        // finished stroke concurrently with this GUI activity snapshot.
        let events = t
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        let strokes = t
            .strokes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        let live_ink = usize::from(
            t.live_ink
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_some(),
        );
        events + strokes + live_ink
    }

    /// The recording just started from a tap: the capsule opens onto the switch chips.
    pub(in crate::capture::desktop) fn show_reveal(&mut self, cx: &mut Context<Self>) {
        // The iPhone chip shows only while one is cabled (a USB registry read, no prompt).
        if !self.lab {
            self.refresh_iphone(cx);
        }
        self.presentation.reduced_motion = reduce_motion();
        let activity = self.reveal_activity();
        self.presentation.reveal.show(reveal_clock_ms(), activity);
        self.presentation.reveal_since = motion::now();
        self.presentation.reveal_outgoing = None;
        self.presentation.island_morph = Morph::RevealOpen;
        if self.presentation.reduced_motion {
            self.presentation.island_scale = 1.0;
            self.presentation.island_scale_velocity = 0.0;
            self.presentation.island_y = 0.0;
            self.presentation.island_y_velocity = 0.0;
        }
        self.sync_native_pill();
        cx.notify();
    }

    /// The chips fold back into the recording pill (timeout, work started, or a chip chosen).
    pub(in crate::capture::desktop) fn fold_reveal(&mut self) {
        if self.presentation.reveal.is_open() {
            self.presentation.reveal.close();
            self.play_fold();
        }
    }

    pub(in crate::capture::desktop) fn play_fold(&mut self) {
        self.presentation.reveal_since = motion::now();
        self.presentation.reveal_outgoing = Some(true);
        self.presentation.island_morph = Morph::RevealClose;
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
        let chips_shown = self.presentation.reveal.is_open();
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
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if let Some(overlay) = self.picker_metal_overlay.as_mut() {
            overlay.prepare_picker();
        }
        self.open_picker(Some(mode), cx);
        if self.is_selecting() && chips_shown && !self.presentation.reduced_motion {
            // The chips showed Screen; the picker shows `mode`.
            self.presentation.highlight_prev = Some(SelectionMode::Screen);
            self.presentation.highlight_since = motion::now();
            self.presentation.island_morph = Morph::Switch;
            self.presentation.switch = Some(SwitchMorph { slide: None });
        }
    }

    /// Stops the active recording and deletes its bundle, leaving the island as it is. Returns
    /// its target and overlay.
    pub(in crate::capture::desktop) fn discard_recording(
        &mut self,
    ) -> Option<(TargetInfo, Option<LaserOverlay>)> {
        if let Some(rec) = &self.active_recording
            && !matches!(rec.source, CaptureSource::Lab)
            && let Err(error) = crate::capture::engine::bundle::recover::cancel(&rec.bundle_dir)
        {
            apparent_log(format!(
                "Could not persist cancellation; capture is still active: {error:#}"
            ));
            return None;
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if let Some(id) = self.agent_capture_id.take() {
            self.agent_captures.set(&id, "cancelled", None, None);
        }
        let rec = self.active_recording.take()?;
        self.note_ui = None;
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
        if let Err(error) =
            crate::capture::engine::bundle::recover::delete_cancelled(&rec.bundle_dir)
        {
            apparent_log(format!(
                "Cancelled assets could not be deleted at {}: {error:#}; recovery remains forbidden",
                rec.bundle_dir.display()
            ));
        }
        Some((rec.target, rec._metal_overlay))
    }

    pub fn is_island_visible(&self) -> bool {
        !matches!(self.presentation.island_phase, IslandPhase::Hidden)
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
        self.public_popup.take();
        self.public_share.cancel_all();
        self.review_audio_issue = bundle_audio_issue(&bundle.0, &bundle.1);
        self.review_bundle = Some(bundle);
        self.review_status = status;
        self.selected_tab = ReviewTab::Keyframes;
        self.selected_keyframe = 0;
    }

    pub(in crate::capture::desktop) fn sync_picker_overlay(&self) {
        let rect = match self.presentation.island_phase {
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

    pub(in crate::capture::desktop) fn report_window_recording_disabled(
        &self,
        cx: &mut Context<Self>,
    ) {
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
        if let IslandPhase::Selecting { mode: prev_mode } = self.presentation.island_phase {
            if prev_mode != mode {
                // Crisp tactile Dynamic Island scale micro-pulse when Space toggles camera/crosshair mode.
                self.presentation.island_scale_velocity += 1.25;
            }
            if mode != SelectionMode::Iphone
                && let Some(ref mut preview) = self.iphone_preview
            {
                preview.hide_preview();
            }
            self.presentation.island_phase = IslandPhase::Selecting { mode };
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
        if let IslandPhase::Selecting { mode } = self.presentation.island_phase {
            let next = match mode {
                SelectionMode::Area => SelectionMode::Screen,
                SelectionMode::Window | SelectionMode::Screen | SelectionMode::Iphone => {
                    SelectionMode::Area
                }
            };
            self.set_selection_mode(next, cx);
        }
    }

    pub(in crate::capture::desktop) fn selection_mode(&self) -> Option<SelectionMode> {
        match self.presentation.island_phase {
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
        let mut desktop = self
            .telemetry
            .desktop_ink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let latest = strokes.iter().rposition(|s| !s.laser && s.pin.is_some());
        if desktop
            .last()
            .is_some_and(|ink| latest.is_none_or(|i| ink.t0 >= strokes[i].t0))
        {
            let ink = desktop.pop().unwrap();
            strokes.retain(|s| s.laser || s.pin.is_some() || !(ink.t0..=ink.t1).contains(&s.t0));
        } else if let Some(idx) = latest {
            if let Some(ui) = &self.note_ui {
                // Remove the mark only after durable note deletion. A failed or
                // timed-out receipt leaves its anchor available for recovery.
                if ui.undo(strokes[idx].pin.unwrap()) {
                    strokes.remove(idx);
                }
            } else {
                strokes.remove(idx);
            }
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
    pub(in crate::capture::desktop) fn set_island_phase(&mut self, phase: IslandPhase) {
        use crate::capture::desktop::settings::onboarding::Input;
        if matches!(
            phase,
            IslandPhase::Selecting { .. }
                | IslandPhase::Processing
                | IslandPhase::Recording
                | IslandPhase::Hidden
                | IslandPhase::Dismissing
        ) {
            self.public_popup.take();
            self.public_share.cancel_all();
        }
        if matches!(
            phase,
            IslandPhase::Recording | IslandPhase::Hidden | IslandPhase::Dismissing
        ) {
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
        if matches!(phase, IslandPhase::Recording)
            && !matches!(self.presentation.island_phase, IslandPhase::Recording)
        {
            self.capture_local_practice = self
                .onboarding
                .as_ref()
                .is_some_and(|session| session.local_practice());
        }
        let capture = match (self.presentation.island_phase, phase) {
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
        let changed = std::mem::discriminant(&self.presentation.island_phase)
            != std::mem::discriminant(&phase);
        if changed {
            // A new phase replaces the whole content; the chips go with it, and the outgoing
            // Recording content keeps showing them while it blurs away.
            self.presentation.reveal_prev_chips = self.presentation.reveal.is_open();
            self.presentation.reveal.close();
            self.presentation.reveal_outgoing = None;
            self.presentation.island_morph = Morph::Phase;
            self.presentation.switch = None;
            self.presentation.highlight_prev = None;
            if matches!(self.presentation.island_phase, IslandPhase::Hidden) {
                self.presentation.island_stepped = Instant::now();
            }
            self.presentation.island_prev_phase = self.presentation.island_phase;
            if matches!(phase, IslandPhase::Dismissing) {
                self.presentation.island_dismissed_from = self.presentation.island_phase;
            } else if matches!(self.presentation.island_phase, IslandPhase::Hidden) {
                self.presentation.island_scale = 0.92;
                self.presentation.island_scale_velocity = 0.0;
                self.presentation.island_y = -6.0;
                self.presentation.island_y_velocity = 0.0;
            }
            self.presentation.island_phase_since = motion::now();
            motion::trace_event(format!("phase:{}", phase.trace_name()));
        }
        let appearing = matches!(
            self.presentation.island_phase,
            IslandPhase::Hidden | IslandPhase::Dismissing
        );
        self.presentation.island_phase = phase;
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
    pub(crate) fn place_capsule(&self) {
        let centre = |[x, y, w, h]: [f64; 4]| [x + w / 2.0, y + h / 2.0];
        let anchor = match self.presentation.island_phase {
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
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if let Some(session) = &self.electron_pill {
            session.place(point);
        }
        match self.native_pill {
            Some(ref pill) => pill.place_on(point),
            None => platform::place_hud_pill(point),
        }
    }

    /// The iPhone's mirror window can be dragged to another display mid-recording: the ink
    /// overlay and the capsule go with it.
    pub(in crate::capture::desktop) fn follow_phone_mirror(&mut self) {
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
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if let Some(session) = &self.electron_pill {
            session.place(centre);
        }
    }

    /// Follow the pointer with Linux controls, not with the capture source. On Mac,
    /// a dragged capsule must stay where it was dropped for the whole recording.
    #[cfg(target_os = "linux")]
    pub(in crate::capture::desktop) fn follow_recording_pointer(&mut self) {
        if !matches!(self.presentation.island_phase, IslandPhase::Recording)
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
            if !platform::pill_user_moved()
                && !self.electron_pill.as_ref().is_some_and(|s| s.moved())
            {
                self.place_capsule();
            }
        }
    }

    /// While picking, the picker (its window, overlay and the capsule) follows the pointer to
    /// whichever display it's on, unless an area is being dragged (an area stays on one display).
    pub(in crate::capture::desktop) fn follow_pointer_display(
        &mut self,
        pointer: [f32; 2],
        cx: &mut Context<Self>,
    ) {
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
    pub(in crate::capture::desktop) fn picker_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        cx: &mut Context<Self>,
    ) {
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
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if self.electron_pill_routed {
            return;
        }
        if !crate::shared::handoff::native_pill() || self.native_pill.is_some() {
            return;
        }
        match crate::capture::desktop::native::pill::NativePill::open() {
            Ok(pill) => {
                if !crate::shared::handoff::experiment("movableRecordingControls") {
                    self.telemetry
                        .excluded_window_ids
                        .lock()
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

    pub(in crate::capture::desktop) fn completed_audio_issue(&self) -> Option<CompletedAudioIssue> {
        match (&self.completed_dir, &self.review_bundle) {
            (Some(dir), Some((review_dir, _))) if dir == review_dir => self.review_audio_issue,
            _ => None,
        }
    }

    pub(in crate::capture::desktop) fn recording_has_no_words(&self) -> bool {
        matches!(
            self.active_recording.as_ref().map(|rec| &rec.source),
            Some(CaptureSource::Screen { audio: Some(audio), .. })
                if audio.live_word_state() == Some(crate::capture::engine::runtime::audio::transcription::live_word_warning::State::NoWords)
        )
    }

    pub(in crate::capture::desktop) fn pill_snapshot(
        &self,
    ) -> crate::capture::desktop::native::pill::PillSnapshot {
        use crate::capture::desktop::native::pill::{PillSnapshot, PillStyle, model::Pres};
        let recording_ms = self
            .active_recording
            .as_ref()
            .map(ActiveRecording::elapsed_ms);
        let elapsed_ms = recording_ms.unwrap_or(self.presentation.last_elapsed_ms);
        let clock = format_clock(elapsed_ms);
        let has_iphone = self.iphone.is_some();

        let pres = match self.presentation.island_phase {
            IslandPhase::Hidden | IslandPhase::Dismissing => Pres::Hidden,
            IslandPhase::Selecting { mode } => Pres::Selecting { mode, has_iphone },
            IslandPhase::Recording
                if self.recording_has_no_words() && !self.presentation.reveal.is_open() =>
            {
                Pres::RecordingWarning {
                    chips: false,
                    has_iphone,
                }
            }
            IslandPhase::Recording => Pres::Recording {
                chips: self.presentation.reveal.is_open(),
                has_iphone,
            },
            IslandPhase::Processing => Pres::Processing,
            IslandPhase::Completed { inbox_count, .. } => {
                let issue = self.completed_audio_issue();
                Pres::Completed {
                    share: crate::shared::handoff::experiment(
                        crate::connect::hosted::HOSTED_SHARING,
                    )
                    .then_some(self.share_online && self.share_pending.is_none()),
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
            reduced_motion: self.presentation.reduced_motion,
            style: PillStyle::current(),
        }
    }

    pub(crate) fn sync_native_pill(&mut self) {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        self.sync_electron_pill();
        if self.native_pill.is_some() {
            let snap = self.pill_snapshot();
            if let Some(ref pill) = self.native_pill {
                pill.sync(snap);
            }
        }
    }

    pub(in crate::capture::desktop) fn apply_pill_action(
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
            PillAction::Notes => {
                if self.is_recording() {
                    if let Some(ui) = &self.note_ui {
                        let (focus, owner) = platform::note_focus();
                        ui.show_list(focus, owner);
                    }
                }
            }
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
            self.presentation.island_phase,
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
        self.presentation.island_opacity = 0.0;
        let entity = cx.entity().clone();
        cx.defer(move |cx| {
            close_capture_windows(cx);
            open_review_window(&entity, cx);
        });
        cx.notify();
    }

    pub(in crate::capture::desktop) fn tick_hosted_sharing(&mut self, cx: &mut Context<Self>) {
        if self
            .completed_upload_size_failure
            .as_ref()
            .is_some_and(|(bundle, generation, _)| {
                *generation != self.capture_generation
                    || self.completed_dir.as_ref() != Some(bundle)
                    || !matches!(
                        self.presentation.island_phase,
                        IslandPhase::Completed { .. }
                    )
            })
        {
            self.completed_upload_size_failure = None;
        }
        let sharing = crate::shared::handoff::experiment(crate::connect::hosted::HOSTED_SHARING);
        if (!sharing && !crate::shared::handoff::experiment(crate::connect::hosted::HOSTED_SYNC))
            || !matches!(
                self.presentation.island_phase,
                IslandPhase::Completed { .. }
            )
            || self
                .share_probe_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(5))
        {
            return;
        }
        self.share_probe_at = Some(Instant::now());
        let generation = self.capture_generation;
        let completed = self.completed_dir.clone();
        let copied = self
            .share_copied
            .clone()
            .filter(|bundle| self.completed_dir.as_ref() == Some(bundle));
        cx.spawn(async move |entity, cx| {
            let selected = completed.clone();
            let copied_capture = copied.clone();
            let (online, progress, failure) = cx
                .background_executor()
                .spawn(async move {
                    use crate::connect::hosted::{self, UploadQueueKind};
                    // Local queue inspection works offline; private uploads never probe public admission.
                    let kind = if copied.is_some() {
                        UploadQueueKind::PublicCopy
                    } else {
                        UploadQueueKind::Private
                    };
                    let failure = selected
                        .as_deref()
                        .and_then(|bundle| hosted::upload_size_failure(bundle, kind));
                    (
                        sharing.then(hosted::sharing_online),
                        copied.as_deref().map(hosted::sharing_progress),
                        failure,
                    )
                })
                .await;
            let _ = entity.update(cx, |coord, cx| {
                if coord.capture_generation != generation
                    || coord.completed_dir != completed
                    || coord.share_copied != copied_capture
                    || !matches!(
                        coord.presentation.island_phase,
                        IslandPhase::Completed { .. }
                    )
                {
                    return;
                }
                if let Some(online) = online {
                    coord.share_online = online;
                }
                if coord.share_pending.is_none() {
                    coord.completed_upload_size_failure = completed
                        .clone()
                        .zip(failure)
                        .map(|(bundle, failure)| (bundle, generation, failure));
                    if let Some(progress) = progress {
                        coord.completed_note = Some(progress);
                    } else if online == Some(false) {
                        coord.completed_note = Some(
                            if crate::connect::hosted::sharing_server().is_none() {
                                "Configure Hosted account in Settings"
                            } else {
                                "Connect to the internet to share"
                            }
                            .into(),
                        );
                    } else if online == Some(true)
                        && coord.completed_note.as_deref()
                            == Some("Connect to the internet to share")
                    {
                        coord.completed_note = None;
                    }
                }
                coord.sync_native_pill();
                cx.notify();
            });
        })
        .detach();
    }

    pub fn copy_completed_link(&mut self, cx: &mut Context<Self>) {
        apparent_log(format!(
            "public-share action completed={} online={} pending={} bundle={}",
            matches!(
                self.presentation.island_phase,
                IslandPhase::Completed { .. }
            ),
            self.share_online,
            self.share_pending.is_some(),
            self.completed_dir.is_some()
        ));
        if !crate::shared::handoff::experiment(crate::connect::hosted::HOSTED_SHARING)
            || !matches!(
                self.presentation.island_phase,
                IslandPhase::Completed { .. }
            )
            || !self.share_online
            || self.share_pending.is_some()
        {
            return;
        }
        let Some(bundle) = self.completed_dir.clone() else {
            return;
        };
        if !crate::connect::hosted::sharing_connected() {
            apparent_log("public-share connect-required");
            self.completed_note = Some("Connect in Settings, then choose Copy link again".into());
            self.open_settings(Some(crate::capture::desktop::settings::Pane::General));
            self.sync_native_pill();
            cx.notify();
            return;
        }
        self.public_popup.take();
        match crate::capture::desktop::workflow::public_sharing::popup::Popup::open(
            bundle,
            self.capture_generation,
            self.public_share.clone(),
        ) {
            Ok(popup) => {
                apparent_log("public-share popup-opened");
                let id = popup.id.clone();
                self.public_popup = Some(popup);
                cx.spawn(async move |entity, cx| {
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_millis(100))
                            .await;
                        let keep = entity
                            .update(cx, |coord, cx| {
                                let Some(popup) =
                                    coord.public_popup.as_mut().filter(|p| p.id == id)
                                else {
                                    return false;
                                };
                                let alive = popup
                                    .tick(coord.completed_dir.as_deref(), coord.capture_generation);
                                if popup.copied() && coord.share_copied != coord.completed_dir {
                                    coord.share_copied = coord.completed_dir.clone();
                                    coord.completed_note =
                                        Some("Link copied · upload may still be preparing".into());
                                    coord.sync_native_pill();
                                    cx.notify();
                                }
                                if !alive {
                                    coord.public_popup.take();
                                    if let IslandPhase::Completed { finished_at, .. } =
                                        &mut coord.presentation.island_phase
                                    {
                                        *finished_at = Instant::now();
                                    }
                                    return false;
                                }
                                true
                            })
                            .unwrap_or(false);
                        if !keep {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(_) => {
                apparent_log("public-share popup-open-failed");
                self.completed_note =
                    Some("Sharing window unavailable · Local Paste remains available".into());
            }
        }
        self.sync_native_pill();
        cx.notify();
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
            self.gate.defer(vec![
                crate::capture::desktop::settings::onboarding::Permission::Accessibility,
            ]);
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
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        self.tick_electron_pill(cx);
        self.tick_hosted_sharing(cx);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if matches!(self.presentation.island_phase, IslandPhase::Recording) {
            platform::tick_pill_drag();
        }
        if self.note_ui.is_some() {
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            if let Some(ui) = self.note_ui.as_mut() {
                ui.set_embedded(self.electron_pill_routed);
            }
            let anchor = self
                .native_pill
                .as_ref()
                .and_then(|p| p.note_badge_anchor());
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            let anchor = self
                .electron_pill
                .as_ref()
                .filter(|_| self.electron_pill_routed)
                .and_then(|pill| pill.note_badge_anchor())
                .or(anchor);
            #[cfg(target_os = "linux")]
            let anchor = anchor.or_else(|| {
                platform::note_badge_anchor(
                    self.presentation.island_width * self.presentation.island_scale,
                    self.presentation.island_y
                        + (PILL_WINDOW_H - ISLAND_H * self.presentation.island_scale) / 2.0,
                    ISLAND_H * self.presentation.island_scale,
                )
            });
            if let (Some(ui), Some(point)) = (self.note_ui.as_mut(), anchor) {
                ui.position_badge(point);
            }
        }
        let live_no_words = self.recording_has_no_words();
        if live_no_words != self.last_live_no_words {
            self.last_live_no_words = live_no_words;
            self.sync_native_pill();
            cx.notify();
        }
        if self.presentation.island_await_frame {
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
        let real = self.presentation.island_stepped.elapsed().as_secs_f32();
        if real < 0.0002 {
            return;
        }
        self.presentation.island_stepped = Instant::now();
        // A late frame slows the motion rather than skipping it ahead (a new window's second
        // frame comes ~50 ms after its first).
        let dt = (real * motion::time_scale()).min(0.025);
        if let IslandPhase::Completed { finished_at, .. } = self.presentation.island_phase
            && finished_at.elapsed() >= Duration::from_millis(15_000)
            && self.public_popup.is_none()
            && self.share_pending.is_none()
            && !matches!(
                self.completed_note.as_deref(),
                Some("Saved · paste manually" | "Saved · open bundle (copy failed)")
            )
        {
            apparent_log("completed-controls timeout-dismissal");
            self.set_island_phase(IslandPhase::Dismissing);
        }

        if let IslandPhase::Selecting { mode } = self.presentation.island_phase {
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
        let (target_w, target_scale, target_y, target_opacity, fade_rate) =
            match self.presentation.island_phase {
                IslandPhase::Hidden => return,
                IslandPhase::Dismissing => {
                    (self.presentation.island_target_width, 0.92, -6.0, 0.0, 18.0)
                }
                _ => (self.presentation.island_target_width, 1.0, 0.0, 1.0, 20.0),
            };

        if self.presentation.reveal.is_open() {
            let activity = self.reveal_activity();
            let on_pill = self
                .native_pill
                .as_ref()
                .map_or_else(pointer_on_hud_pill, |p| p.pointer_on_pill());
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            let on_pill = self
                .electron_pill
                .as_ref()
                .filter(|s| s.ready())
                .map_or(on_pill, |s| s.hovered());
            if self
                .presentation
                .reveal
                .tick(reveal_clock_ms(), on_pill, activity)
            {
                self.play_fold();
            }
        }

        let (width, velocity) =
            if self.presentation.reduced_motion && self.presentation.island_morph != Morph::Phase {
                // Reduce motion: the reveal's groups fade in place with no width travel; a shrink
                // waits for the outgoing chips to fade so none are clipped while visible.
                let fading = self.presentation.reveal_outgoing.is_some()
                    && motion::age(self.presentation.reveal_since) < REDUCED_FADE_S;
                if fading && target_w < self.presentation.island_width {
                    (self.presentation.island_width, 0.0)
                } else {
                    (target_w, 0.0)
                }
            } else {
                spring_step(
                    self.presentation.island_width,
                    self.presentation.island_velocity,
                    target_w,
                    dt,
                    self.presentation.island_morph.spring(&tune),
                )
            };
        let (scale, s_vel) = spring_step(
            self.presentation.island_scale,
            self.presentation.island_scale_velocity,
            target_scale,
            dt,
            tune.island_scale,
        );
        let (y_off, y_vel) = spring_step(
            self.presentation.island_y,
            self.presentation.island_y_velocity,
            target_y,
            dt,
            tune.island_scale,
        );
        if let Some(morph) = self.presentation.switch.as_mut()
            && let Some((x, v)) = morph.slide
        {
            let (x, v) = spring_step(x, v, 0.0, dt, tune.switch_slide);
            morph.slide = Some((x, v));
            if x.abs() < 0.2 && v.abs() < 2.0 {
                self.presentation.switch = None;
            }
        }
        self.presentation.island_width = width.clamp(120.0, PILL_WINDOW_W - 16.0);
        self.presentation.island_velocity = velocity;
        self.presentation.island_scale = scale.clamp(0.35, 1.18);
        self.presentation.island_scale_velocity = s_vel;
        self.presentation.island_y = y_off.clamp(-18.0, 14.0);
        self.presentation.island_y_velocity = y_vel;
        self.presentation.island_opacity +=
            (target_opacity - self.presentation.island_opacity) * (fade_rate * dt).min(1.0);
        self.sync_native_pill();

        let dismiss_done = if let Some(ref pill) = self.native_pill {
            pill.is_fully_hidden()
        } else {
            self.presentation.island_opacity < 0.03
        };
        if matches!(self.presentation.island_phase, IslandPhase::Dismissing) && dismiss_done {
            self.set_island_phase(IslandPhase::Hidden);
            self.presentation.island_opacity = 0.0;
            cx.defer(close_capture_windows);
        }
        if self.native_pill.is_none() {
            cx.notify();
        }
    }
}
