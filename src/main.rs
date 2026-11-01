mod agent_context;
mod audio;
mod autocrop;
// Windows looks elements up through UI Automation (`win/uia.rs`); other systems have no lookup.
mod autocrop_lab;
#[cfg_attr(
    not(any(target_os = "macos", target_os = "windows")),
    path = "ax_other.rs"
)]
mod ax;
mod capture;
#[cfg_attr(target_os = "windows", path = "win/command_line.rs")]
mod command_line;
mod command_transcriber;
mod delivery;
mod export;
#[cfg(test)]
mod feature_map;
mod handoff;
#[cfg_attr(target_os = "windows", path = "win/helper.rs")]
mod helper;
mod hex;
mod icons;
mod inbox;
mod ingest_http;
mod keyframes;
#[cfg(target_os = "macos")]
mod laser_overlay;
mod live_transcribe;
mod mic;
mod model;
mod native;
mod onboarding;
mod onboarding_window;
mod page;
mod phone;
// Lowers the output volume while recording on macOS; elsewhere a no-op.
#[cfg_attr(not(target_os = "macos"), path = "win/playback.rs")]
mod playback;
mod recover;
mod render;
mod retention;
mod routing;
mod screens;
mod settings;
mod settings_window;
mod shortcut;
mod sketch;
#[cfg(target_os = "macos")]
mod sparkle;
mod speech_model;
mod transcribe;
mod ui;
mod web;
#[cfg(target_os = "windows")]
mod win;
mod windows;
mod zoom;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};

use crate::audio::{AudioRecorder, RecordedAudio};
use crate::capture::{ScreenCaptureSession, discover_target, write_bundle_artifacts};
use crate::keyframes::select_keyframes;
use crate::model::{
    CaptureMode, DeviceInfo, InteractionEvent, Mark, RgbaFrame, SCALE, ScrawlBundle, Shape, Stroke,
    TargetFrame, TargetInfo, TranscriptData, TranscriptSegment, WordTiming, keyframe_budget,
    parse_clock_ms,
};
use crate::render::render_synthetic_demo_frame;
use crate::sketch::{marks_of, summary_markdown};

#[derive(Parser)]
#[command(
    name = "scrawl",
    version,
    about = "Shortcut-triggered screen, voice, and drawing capture for agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

fn parse_engine(name: &str) -> Result<crate::transcribe::Engine, String> {
    crate::transcribe::Engine::parse(name)
        .ok_or_else(|| "expected `apple`, `hex` or `command`".to_owned())
}

/// `scrawl hex status`: connection, models and what the next recording would use.
fn hex_status() {
    let preferred = transcribe::Engine::configured();
    println!("Preferred transcriber: {}", preferred.name());
    // Never runs the program; argv is not printed.
    let command = handoff::transcribe_command();
    if preferred == transcribe::Engine::Command {
        match &command {
            Ok(program) => println!("Command: {}", program.name()),
            Err(unusable) => println!("Command: not usable ({unusable})"),
        }
    }
    let next = |hex_model: Option<&str>| match (preferred, hex_model, &command) {
        (transcribe::Engine::Command, _, Ok(program)) => format!("Command ({})", program.name()),
        (transcribe::Engine::Hex, Some(model), _) => format!("Hex ({model})"),
        _ => transcribe::LOCAL_ENGINE_TITLE.to_owned(),
    };
    let configured = handoff::hex_model();
    println!(
        "Hex model setting: {}",
        configured.as_deref().unwrap_or("(automatic)")
    );
    let paths = hex::discovery_paths();
    let status = match hex::Client::connect_any(&paths)
        .and_then(|(client, path)| Ok((client.status()?, path)))
    {
        Ok((status, path)) => {
            println!("Discovery file: {}", path.display());
            status
        }
        Err(error) => {
            let tried: Vec<_> = paths
                .iter()
                .map(|path| path.display().to_string())
                .collect();
            println!("Discovery files tried: {}", tried.join(", "));
            println!("Hex: not connected ({error:#})");
            println!("Next recording uses: {}", next(None));
            return;
        }
    };
    println!("Hex: connected, version {}", status.version);
    for model in &status.models {
        let state = match (model.installed, model.verified) {
            (true, true) => "ready",
            (true, false) => "installed, not verified",
            _ => "not installed",
        };
        println!("  {}\t{state}", model.id);
    }
    let chosen = hex::pick_model(&status.models, configured.as_deref());
    println!(
        "Hex model in use: {}",
        chosen.as_deref().unwrap_or("none ready")
    );
    println!("Next recording uses: {}", next(chosen.as_deref()));
}

#[derive(Subcommand)]
enum MicCommand {
    /// The connected microphones (id, name), marking the default and the one recordings use.
    List,
    /// Record from this device id, or `default` to follow macOS.
    Set { id: String },
    /// Print the list again whenever an input is plugged in or out or the default changes.
    Watch,
}

fn mic_list() {
    let inputs = mic::inputs();
    let choice = mic::choice();
    let pick = mic::Pick::new(choice.as_ref(), &inputs);
    let using = pick.device().map(|d| d.id.clone());
    for input in &inputs.devices {
        let mut tags = Vec::new();
        if inputs.default.as_deref() == Some(input.id.as_str()) {
            tags.push("default");
        }
        if using.as_deref() == Some(input.id.as_str()) {
            tags.push("recording");
        }
        println!("{}\t{}\t{}", input.id, input.name, tags.join(","));
    }
    if let Some(note) = pick.note() {
        println!("{note}");
    }
}

#[derive(Subcommand)]
enum HexCommand {
    /// Whether Hex is connected, its version, ready models and the transcriber Scrawl will use.
    Status,
}

#[derive(Subcommand)]
enum LabCommand {
    /// Play a capsule scene: tap-then-area, tap-then-window, tap-then-screen, picker, finish.
    Pill {
        #[arg(value_enum)]
        scene: ui::LabScene,
        /// Write a motion-lint trace of every frame here (same as `SCRAWL_MOTION_TRACE`).
        #[arg(long)]
        trace: Option<PathBuf>,
        /// Times to play it; 0 repeats until stopped (tune while it loops).
        #[arg(long, default_value_t = 1)]
        repeat: u32,
        /// Animation time scale, e.g. 0.25 for slow motion (overrides tune.json's timeScale).
        #[arg(long)]
        time_scale: Option<f32>,
        /// Leave out the plain backdrop behind the capsule.
        #[arg(long)]
        no_backdrop: bool,
    },
}

#[derive(Subcommand)]
enum TuneCommand {
    /// Every key with its live and default value, and the presets (default).
    List,
    /// Set one key, e.g. `switch.slide.duration 0.4`.
    Set { key: String, value: f32 },
    /// Apply a numbered preset (1 is Today); clears individual keys.
    Preset { number: usize },
    /// Delete tune.json: back to the built-in defaults.
    Reset,
    /// Print the live values as Rust, to bake into `Tune::DEFAULT`.
    Rust,
}

#[derive(Subcommand)]
enum Command {
    /// Launch the GPUI menu-bar and review application (the default when macOS opens Scrawl.app).
    App {
        /// Start a capture session immediately on launch.
        #[arg(long)]
        record: bool,
        /// Start capturing the cabled iPhone immediately on launch.
        #[arg(long, conflicts_with = "record")]
        iphone: bool,
    },
    /// Generate a realistic annotated demo bundle (contact sheet + keyframes + transcript).
    Demo {
        /// Output `.scrawl` bundle directory (defaults to Application Support).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Copy the generated bundle to the macOS clipboard.
        #[arg(long)]
        clipboard: bool,
    },
    /// Record a live screen + microphone capture from the terminal for a fixed duration.
    Record {
        /// Optional window title or app substring to target.
        #[arg(long)]
        window: Option<String>,
        /// Capture the cabled iPhone instead of the screen (live mirror window + native video).
        #[arg(long, conflicts_with = "window")]
        iphone: bool,
        /// Recording duration in seconds (default: 5).
        #[arg(long, default_value_t = 5)]
        duration: u64,
        /// Optional note to attach to the bundle.
        #[arg(long)]
        note: Option<String>,
        /// Output `.scrawl` bundle directory.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Build a bundle from a ScrawlKit in-app recording (`.mp4` + timeline `.json`).
    Import {
        /// The recording or its timeline; defaults to the newest pair in ~/Downloads.
        recording: Option<PathBuf>,
        /// Timeline JSON, when it isn't beside the recording with the same name.
        #[arg(long)]
        timeline: Option<PathBuf>,
        /// Optional note to attach to the bundle.
        #[arg(long)]
        note: Option<String>,
        /// Output `.scrawl` bundle directory.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Copy the summary to the clipboard.
        #[arg(long)]
        clipboard: bool,
        /// Transcribe with `apple`, `hex` or `command` instead of the configured transcriber.
        #[arg(long, value_parser = parse_engine)]
        transcriber: Option<crate::transcribe::Engine>,
    },
    /// List cabled iPhones and iPads that can be captured.
    Devices,
    /// Try a one-frame ScreenCaptureKit capture in this process and print the result as JSON,
    /// once or (with `--watch`) every second, to see whether a Screen Recording grant reaches a
    /// running process. Asks for Screen Recording if it was never decided.
    #[command(hide = true)]
    SckProbe {
        /// Keep trying for this many seconds without restarting.
        #[arg(long)]
        watch: Option<u64>,
    },
    /// Transcribe a bundle's audio again with Apple Speech, in place: its transcript, each
    /// keyframe's words, `scrawl.json` and `summary.md`. Scrawl does this by itself for captures
    /// made while the speech model was still downloading.
    Retranscribe {
        /// Path to `.scrawl` bundle directory or `latest`.
        bundle: PathBuf,
    },
    /// Rebuild a bundle from its own video, audio, and input log with the current transcriber
    /// and keyframe rules.
    Reprocess {
        /// Path to `.scrawl` bundle directory or `latest`.
        bundle: PathBuf,
        /// Output `.scrawl` bundle directory (a new bundle; the original is untouched).
        #[arg(long)]
        out: PathBuf,
        /// Run N iterations of the finish pipeline (with candidate frames pre-loaded for Screen
        /// captures) to benchmark finish-to-done wall clock.
        #[arg(long, hide = true)]
        bench: Option<u32>,
        /// With `--bench` on a Screen capture: play the audio into a live transcription at this
        /// many times real time first, as if recording, so the finish only transcribes the tail.
        #[arg(long, hide = true, requires = "bench")]
        live_speed: Option<f64>,
        /// Transcribe with `apple`, `hex` or `command` instead of the configured transcriber.
        #[arg(long, value_parser = parse_engine)]
        transcriber: Option<crate::transcribe::Engine>,
        /// Turn an experiment on for this run, as `"experiments"` in `config.json` would
        /// (e.g. `agentContext`). Repeatable.
        #[arg(long = "experiment", value_name = "NAME")]
        experiments: Vec<String>,
    },
    /// Build the bundles whose build died with the app (no `scrawl.json`) in place, once each:
    /// from their input log (`unfinished.json`) if they have one, else from `recording.mp4` and
    /// `audio.wav` alone. The app runs it at launch.
    Recover {
        /// One bundle to build now, even if it was tried before.
        bundle: Option<PathBuf>,
        /// Captures folder (defaults to Scrawl's).
        #[arg(long, conflicts_with = "bundle")]
        captures: Option<PathBuf>,
    },
    /// Motion lab: replay the capsule's states on a fixed timeline with nothing captured, for
    /// traces (`--trace`, motion-lint's format), screen recordings and tuning.
    #[command(hide = true)]
    Lab {
        #[command(subcommand)]
        command: LabCommand,
    },
    /// Tune the capsule's springs live: edits `~/.config/scrawl/tune.json`, which debug builds
    /// (the lab and the installed app) re-read within a quarter second.
    #[command(hide = true)]
    Tune {
        #[command(subcommand)]
        command: Option<TuneCommand>,
    },
    /// List microphones or choose the one Mac captures record from (Settings › Recording).
    Mic {
        #[command(subcommand)]
        command: MicCommand,
    },
    /// Check the Hex transcriber: is it running, which models are ready, which one Scrawl uses.
    Hex {
        #[command(subcommand)]
        command: HexCommand,
    },
    /// Print the agent-ready summary or JSON manifest for a `.scrawl` bundle (or `latest`).
    Inspect {
        /// Path to `.scrawl` bundle directory, `scrawl.json`, or `latest`.
        #[arg(default_value = "latest")]
        bundle: PathBuf,
        /// Emit raw `scrawl.json` instead of `summary.md` (with `--marks`, the marks as JSON).
        #[arg(long)]
        json: bool,
        /// List the marks: keyframe, number as the summary shows it (✎②), time, shape, what it
        /// was drawn on, and id, for `scrawl zoom --mark`.
        #[arg(long)]
        marks: bool,
        /// Turn an experiment on for this summary (e.g. `agentContext`). Repeatable.
        #[arg(long = "experiment", value_name = "NAME")]
        experiments: Vec<String>,
    },
    /// Extract a dense burst of frames from a `.scrawl` bundle's time range (`M:SS` or seconds).
    Frames {
        /// Path to `.scrawl` bundle directory or `latest`.
        bundle: PathBuf,
        /// Start time (`0:02` or `2.0`).
        #[arg(long)]
        from: String,
        /// End time (`0:05` or `5.0`).
        #[arg(long)]
        to: String,
        /// Frames per second to extract when the bundle has a video.
        #[arg(long, default_value_t = 4)]
        fps: u32,
    },
    /// Zoom into one moment: a dense contact sheet of frames around a time, a spoken phrase or a
    /// mark, with the bundle's ink and touches drawn in and each frame's time, plus what was said.
    Zoom {
        /// Path to `.scrawl` bundle directory or `latest`.
        #[arg(default_value = "latest")]
        bundle: PathBuf,
        /// The moment: a time (`0:12`, `12.5`) or a phrase from the narration ("width changes").
        #[arg(long, required_unless_present = "mark", conflicts_with = "mark")]
        at: Option<String>,
        /// A mark, centred with its ink drawn: its number in the summary (`2` for ✎②) or its id
        /// (`m2`). `scrawl inspect --marks` lists them.
        #[arg(long)]
        mark: Option<String>,
        /// The keyframe whose ✎① ✎② `--mark` counts (`--frame 2` for #2), when several have one.
        #[arg(long, requires = "mark")]
        frame: Option<usize>,
        /// Seconds of video to cover (a quarter before the moment, the rest after; for a mark, at
        /// least the whole mark, centred).
        #[arg(long, default_value_t = 2.0)]
        span: f64,
        /// Frames in the sheet.
        #[arg(long, default_value_t = 12)]
        frames: usize,
        /// Show the whole screen even when the bundle's keyframes are auto-cropped.
        #[arg(long)]
        full: bool,
    },
    /// Export a bundle's recording as one shareable .mp4: H.264 video cropped to the activity
    /// with the ink and click ripples burned in, AAC audio in sync, and optionally Screen
    /// Studio-style auto-zoom (`--zoom`). Encoding runs in the helper with AVFoundation.
    Export {
        /// Path to `.scrawl` bundle directory or `latest`.
        #[arg(default_value = "latest")]
        bundle: PathBuf,
        /// Output file (defaults to ~/Movies/Scrawl/<bundle>.mp4).
        #[arg(long)]
        out: Option<PathBuf>,
        /// `full`, `auto` (the activity region), or `x,y,w,h` as fractions of the capture (all
        /// ≤ 1) or pixels of the recording.
        #[arg(long, default_value = "auto")]
        crop: export::crop::CropArg,
        /// Longest side of the output in pixels; it is never scaled up.
        #[arg(long, default_value_t = export::crop::DEFAULT_LONG_EDGE)]
        size: u32,
        /// Ease in toward clicks, drags, marks and laser points, hold while activity stays
        /// nearby, and ease back out.
        #[arg(long)]
        zoom: bool,
        /// Leave out the red ink, pins and laser.
        #[arg(long)]
        no_ink: bool,
        /// Leave out click ripples and touch trails.
        #[arg(long)]
        no_clicks: bool,
        /// Encode HEVC instead of H.264.
        #[arg(long)]
        hevc: bool,
        /// Output frame rate (default 60 for a 60 fps recording, else 30).
        #[arg(long)]
        fps: Option<u32>,
    },
    /// Clean up old captures now, as the app does daily: video and audio past the Keep
    /// Recordings setting go to the Trash, then whole bundles; native iPhone video is shrunk.
    Clean {
        /// Print what would happen without doing it.
        #[arg(long)]
        dry_run: bool,
        /// Captures folder to clean (defaults to Application Support).
        #[arg(long, hide = true)]
        captures: Option<PathBuf>,
        /// Plan as if this many days had passed.
        #[arg(long, hide = true, default_value_t = 0.0)]
        days_ahead: f64,
    },
    /// Keep a bundle forever, out of cleanup (`--off` lets it go again).
    Pin {
        /// Path to `.scrawl` bundle directory or `latest`.
        #[arg(default_value = "latest")]
        bundle: PathBuf,
        #[arg(long)]
        off: bool,
    },
    /// Open the auto-crop playground for a Screen capture: each keyframe with its windows,
    /// activity and crop, and the cropped contact sheet. `--serve` reruns the crop live as
    /// settings change; without it the page shows the presets.
    Autocrop {
        /// Path to `.scrawl` bundle directory or `latest`.
        #[arg(default_value = "latest")]
        bundle: PathBuf,
        /// Open the playground in the default browser.
        #[arg(long)]
        open: bool,
        /// Serve the playground on localhost so the sliders rerun the crop.
        #[arg(long)]
        serve: bool,
        #[arg(long, default_value_t = 0)]
        port: u16,
    },
    /// Build a bundle from a simulated Screen capture (`scripts/autocrop-corpus.mjs`).
    #[command(hide = true)]
    Fixture {
        /// The scenario's `scenario.json`.
        scenario: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Auto-crop settings JSON (defaults otherwise).
        #[arg(long)]
        settings: Option<PathBuf>,
    },
    /// Drive a Screen capture of a scenario headless for SECONDS: the sampler's candidate buffer
    /// on a virtual clock, then the finish (keyframes decoded from MOVIE), timed; prints JSON.
    #[command(name = "capture-sim", hide = true)]
    CaptureSim {
        scenario: PathBuf,
        #[arg(long)]
        seconds: u64,
        /// A recording of the scenario's states (`recording.mp4` for the bundle).
        #[arg(long)]
        movie: Option<PathBuf>,
        /// Replay the old sampler: full-size candidates, the newest 300 kept.
        #[arg(long)]
        legacy: bool,
        #[arg(long)]
        out: PathBuf,
    },
    /// Score auto-crop against simulated captures with ground truth (`truth.json`).
    #[command(name = "autocrop-eval", hide = true)]
    AutocropEval {
        /// Bundles, or directories of bundles.
        bundles: Vec<PathBuf>,
        /// Settings JSON to score (defaults otherwise).
        #[arg(long)]
        settings: Option<PathBuf>,
        /// Search for better settings, starting from these.
        #[arg(long)]
        sweep: bool,
        /// Write per-keyframe scores as JSON here.
        #[arg(long)]
        json: Option<PathBuf>,
        /// Write the corpus (inputs and ground truth, no images) here, for unit tests.
        #[arg(long)]
        export: Option<PathBuf>,
    },
    /// Print the window list the Screen-mode window log would store, and what it costs.
    #[command(hide = true)]
    Windows {
        /// Repeat the snapshot this many times and report the average time.
        #[arg(long, default_value_t = 1)]
        repeat: u32,
    },
    /// Copy a `.scrawl` bundle to the clipboard or send it to an OpenCode V2 session.
    Send {
        /// Path to `.scrawl` bundle directory or `latest`.
        #[arg(default_value = "latest")]
        bundle: PathBuf,
        /// Target OpenCode session ID (`ses_...`). If omitted, an iPhone recording goes to the
        /// session it was addressed to, a Mac capture of a dev site to the session it was routed
        /// to, anything else to the most recent active session.
        #[arg(long)]
        session: Option<String>,
        /// Send an iPhone recording to `--session` even though it was addressed to another.
        #[arg(long)]
        reroute: bool,
        /// Copy to the macOS clipboard instead of posting to OpenCode.
        #[arg(long)]
        clipboard: bool,
    },
    /// Print which OpenCode session a capture of a browser tab would be sent to, without
    /// sending: a page registered through `/sdk.js`, else the project serving a localhost port.
    ///
    /// With `--explain <bundle>`, prints the evidence smart routing sees for that capture, the
    /// sessions it weighed and their scores, and its decision (read-only). `--url` stands in for
    /// the tab when the bundle didn't save one.
    Route {
        /// The tab's URL.
        #[arg(long, conflicts_with = "browser")]
        url: Option<String>,
        /// Read the front tab of this browser instead (bundle ID, e.g. `com.brave.Browser`).
        #[arg(long)]
        browser: Option<String>,
        /// A `.scrawl` bundle (or `latest`) to explain.
        #[arg(long, value_name = "BUNDLE")]
        explain: Option<PathBuf>,
    },
    /// Open a `.scrawl` bundle in the native GPUI review window.
    Preview {
        /// Path to `.scrawl` bundle directory or `latest`. Generates a demo bundle if none exists.
        #[arg(default_value = "latest")]
        bundle: PathBuf,
        /// Capture a window screenshot to this path and exit (for deterministic UI verification).
        #[arg(long)]
        capture_screenshot: Option<PathBuf>,
    },
    /// Print a `<scheme>://scrawl?url=…&token=…` setup link (and QR code) for ScrawlKit on iPhone.
    #[command(alias = "tether-link")]
    Link {
        /// URL scheme of the iOS app (for example `updown`).
        #[arg(long)]
        scheme: String,
        /// Scrawl ingest base URL(s); defaults to this Mac's Tailscale and LAN IPs on `--port`.
        #[arg(long = "url")]
        urls: Vec<String>,
        /// Optional OpenCode session ID (`ses_...`) to route captures from this app to.
        #[arg(long)]
        session: Option<String>,
        /// Ingest port used when discovering default Tailscale / LAN URLs.
        #[arg(long, default_value_t = phone::DEFAULT_INGEST_PORT)]
        port: u16,
    },
    /// Run the headless ScrawlKit ingest server (HTTP + optional USB watcher, no GUI or clipboard).
    Serve {
        /// TCP port to listen on.
        #[arg(long, default_value_t = phone::DEFAULT_INGEST_PORT)]
        port: u16,
        /// Also poll cabled iPhones over USB (`devicectl`) for apps listed in `usbApps`.
        #[arg(long)]
        usb: bool,
        /// Build bundles without posting them to an OpenCode session.
        #[arg(long)]
        no_opencode: bool,
    },
    /// Print the agent skill that teaches coding agents to read captures and use this CLI (`bunx skills add kitlangton/scrawl` installs it).
    Skill,
}

/// `skills/scrawl/SKILL.md`, the same text `bunx skills add kitlangton/scrawl` installs.
const SKILL: &str = include_str!("../skills/scrawl/SKILL.md");

pub fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn make_bundle_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    // Two IDs in the same millisecond (parallel tests, overlapping ingests) must still differ.
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let ms = now_epoch_ms();
    let pid = std::process::id();
    match COUNT.fetch_add(1, Ordering::Relaxed) {
        0 => format!("scrawl-{ms}-{pid:x}"),
        n => format!("scrawl-{ms}-{pid:x}-{n}"),
    }
}

/// Matches full-resolution decoded frames back to selected keyframes by timestamp (within 250ms),
/// so a single frame decode failure only leaves that one keyframe at candidate resolution.
fn merge_full_keyframes(
    selected: Vec<(model::Keyframe, RgbaFrame)>,
    full: Vec<RgbaFrame>,
) -> (Vec<(model::Keyframe, RgbaFrame)>, usize) {
    let mut pool: Vec<Option<RgbaFrame>> = full.into_iter().map(Some).collect();
    let mut matched = 0;
    let merged = selected
        .into_iter()
        .map(|(key, fallback)| {
            let best = pool
                .iter()
                .enumerate()
                .filter_map(|(idx, f)| f.as_ref().map(|frame| (idx, frame.t_ms.abs_diff(key.t))))
                .filter(|&(_, diff)| diff <= 250)
                .min_by_key(|&(_, diff)| diff)
                .map(|(idx, _)| idx);
            if let Some(idx) = best
                && let Some(mut full_frame) = pool[idx].take()
            {
                full_frame.t_ms = key.t;
                matched += 1;
                (key, full_frame)
            } else {
                (key, fallback)
            }
        })
        .collect();
    (merged, matched)
}

/// Per-stage timing breakdown logged on every finish (`finish timings: ...`).
#[derive(Clone, Copy, Debug, Default)]
pub struct StageTimings {
    pub started_at: Option<std::time::Instant>,
    pub stop_ms: Option<u64>,
    pub audio_ms: Option<u64>,
    pub candidates_ms: Option<u64>,
    pub transcribe_ms: Option<u64>,
}

/// Pre-rendered keyframe visuals and encoded PNGs computed while transcription is running.
pub struct PreparedVisuals {
    pub keyframes: Vec<model::Keyframe>,
    pub autocrop: Option<model::AutocropInfo>,
    pub contact_sheet_png: Vec<u8>,
    pub frame_pngs: Vec<(String, Vec<u8>)>,
    pub full_sheet_png: Option<Vec<u8>>,
    pub full_frame_pngs: Vec<(String, Vec<u8>)>,
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_visuals(
    selected: Vec<(model::Keyframe, RgbaFrame)>,
    baseline: Option<&RgbaFrame>,
    crop_settings: Option<&autocrop::Settings>,
    target: &TargetInfo,
    duration_ms: u64,
    strokes: &[Stroke],
    marks: &[Mark],
    events: &[InteractionEvent],
    activity: &windows::ActivityLog,
) -> Option<PreparedVisuals> {
    if selected.is_empty() {
        return None;
    }
    let (mut annotated_frames, mut cropped) = autocrop::annotate_and_crop(
        selected,
        baseline,
        crop_settings,
        target,
        duration_ms,
        strokes,
        marks,
        events,
        &activity.windows,
        &activity.cursor,
    );
    let keyframes: Vec<model::Keyframe> =
        annotated_frames.iter().map(|(kf, _)| kf.clone()).collect();
    let (full_sheet, contact_sheet) = std::thread::scope(|scope| {
        let full_handle = cropped.as_ref().map(|(_, full)| {
            scope.spawn(|| render::render_contact_sheet_for_duration(duration_ms, full))
        });
        let sheet = render::render_contact_sheet_for_duration(duration_ms, &annotated_frames);
        (full_handle.map(|h| h.join().ok()).flatten(), sheet)
    });
    render::stamp_keyframe_badges(&mut annotated_frames);
    if let Some((_, full)) = &mut cropped {
        render::stamp_keyframe_badges(full);
    }
    let contact_sheet_png = render::encode_png(&contact_sheet).ok()?;
    let frame_pngs: Vec<(String, Vec<u8>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = annotated_frames
            .iter()
            .map(|(kf, frame)| {
                scope.spawn(move || render::encode_png(frame).ok().map(|b| (kf.file.clone(), b)))
            })
            .collect();
        workers
            .into_iter()
            .map(|w| w.join().ok().flatten())
            .collect::<Option<Vec<_>>>()
    })?;
    let (autocrop, full_sheet_png, full_frame_pngs) = match (cropped, full_sheet) {
        (Some((info, full)), Some(sheet)) => {
            let sheet_png = render::encode_png(&sheet).ok()?;
            let full_pngs: Vec<(String, Vec<u8>)> = std::thread::scope(|scope| {
                let workers: Vec<_> = full
                    .iter()
                    .map(|(kf, frame)| {
                        scope.spawn(move || {
                            let name = std::path::Path::new(&kf.file)
                                .file_name()
                                .and_then(|n| n.to_str())?
                                .to_owned();
                            render::encode_png(frame).ok().map(|b| (name, b))
                        })
                    })
                    .collect();
                workers
                    .into_iter()
                    .map(|w| w.join().ok().flatten())
                    .collect::<Option<Vec<_>>>()
            })?;
            (Some(info), Some(sheet_png), full_pngs)
        }
        _ => (None, None, Vec::new()),
    };
    Some(PreparedVisuals {
        keyframes,
        autocrop,
        contact_sheet_png,
        frame_pngs,
        full_sheet_png,
        full_frame_pngs,
    })
}

/// What a source other than this Mac's screen adds to a bundle.
#[derive(Default)]
pub struct SourceExtras {
    pub source: Option<String>,
    pub device: Option<DeviceInfo>,
    /// What the recorded app said about itself: values at the start, their changes, and screens.
    pub context: std::collections::BTreeMap<String, String>,
    pub context_changes: Vec<model::ContextChange>,
    pub screens: Vec<model::ScreenSpan>,
    /// Screen mode's window and cursor log.
    pub activity: windows::ActivityLog,
    /// Time-only markers (ms); each becomes a `Flag` mark with keyframe priority.
    pub flags: Vec<u64>,
    /// When the video starts on the bundle's clock (ms).
    pub video_offset_ms: u64,
    /// Screen-capture auto-crop: `None` follows the config, `Some(None)` is off.
    pub autocrop: Option<Option<autocrop::Settings>>,
    /// Decodes the chosen keyframes again at full quality, for sources that select on small
    /// candidate frames (phone movies).
    pub full_frames: Option<FullFrames>,
    pub prepared: Option<PreparedVisuals>,
    pub timings: StageTimings,
}

/// Decodes frames at these times at full quality.
pub type FullFrames = Box<dyn Fn(&[u64]) -> Result<Vec<RgbaFrame>>>;

/// Keyframes at native size from a Mac screen recording, whose clock starts `offset_ms` into the
/// capture. Each frame comes back stamped with the capture time it was asked for.
pub fn movie_keyframes(movie: PathBuf, offset_ms: u64) -> FullFrames {
    movie_keyframes_with_cache(movie, offset_ms, Vec::new())
}

fn movie_keyframes_with_cache(
    movie: PathBuf,
    offset_ms: u64,
    predecoded: Vec<RgbaFrame>,
) -> FullFrames {
    let cache = std::sync::Mutex::new(predecoded);
    Box::new(move |times: &[u64]| {
        let mut pool = cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut out = Vec::with_capacity(times.len());
        let mut missing = Vec::new();
        for &t in times {
            if let Some(idx) = pool.iter().position(|f| f.t_ms == t) {
                out.push(pool.swap_remove(idx));
            } else {
                missing.push(t);
            }
        }
        drop(pool);
        if !missing.is_empty() {
            out.extend(movie_frames_at(&movie, &missing, offset_ms, 0)?);
        }
        Ok(out)
    })
}

/// Frames of a movie whose clock starts `offset_ms` into the bundle's, at bundle times.
pub fn movie_frames_at(
    movie: &Path,
    times: &[u64],
    offset_ms: u64,
    max_edge: u32,
) -> Result<Vec<RgbaFrame>> {
    movie_frames_at_with(helper::frames, movie, times, offset_ms, max_edge)
}

fn movie_frames_at_with(
    decode: fn(&Path, helper::FrameTimes, u32) -> Result<Vec<RgbaFrame>>,
    movie: &Path,
    times: &[u64],
    offset_ms: u64,
    max_edge: u32,
) -> Result<Vec<RgbaFrame>> {
    let at: Vec<u64> = times.iter().map(|t| t.saturating_sub(offset_ms)).collect();
    let mut frames = decode(movie, helper::FrameTimes::At(&at), max_edge)?;
    // The helper stamps each frame with the time it was asked for, clamped to the movie (the
    // last keyframe can fall just past its end); map it back to the bundle time.
    for frame in &mut frames {
        if let Some(i) = (0..at.len()).min_by_key(|&i| at[i].abs_diff(frame.t_ms)) {
            frame.t_ms = times[i];
        }
    }
    Ok(frames)
}

/// Core pipeline that turns raw frames, strokes, events, and audio/transcript into a complete `.scrawl` bundle on disk.
#[allow(clippy::too_many_arguments)]
///
/// The recording's candidate frames are small; the keyframes are decoded again from its video at
/// full size, falling back to the candidates if that fails.
pub fn finalize_capture_bundle(
    bundle_dir: &Path,
    target: TargetInfo,
    recording: capture::ScreenRecording,
    strokes: Vec<Stroke>,
    events: Vec<InteractionEvent>,
    recorded_audio: Option<RecordedAudio>,
    note: Option<String>,
    activity: windows::ActivityLog,
) -> Result<(PathBuf, ScrawlBundle, String)> {
    let started_at = activity
        .stop_started
        .unwrap_or_else(std::time::Instant::now);
    let stop_ms = activity.stop_ms;
    let capture::ScreenRecording {
        frames,
        video: video_path,
        duration_ms,
        video_offset_ms,
    } = recording;
    if let Some(ref audio) = recorded_audio {
        let wav_path = bundle_dir.join("audio.wav");
        let _ = std::fs::create_dir_all(bundle_dir);
        let _ = std::fs::write(&wav_path, &audio.wav_bytes);
    }
    let wav_path = bundle_dir.join("audio.wav");
    let has_wav = wav_path.exists();
    recover::write(
        bundle_dir,
        &recover::Unfinished {
            target: target.clone(),
            duration_ms,
            video: video_path
                .as_ref()
                .and_then(|v| v.file_name())
                .map(|name| name.to_string_lossy().into_owned()),
            video_offset_ms,
            strokes: strokes.clone(),
            events: events.clone(),
            note: note.clone(),
            windows: activity.windows.clone(),
            cursor: activity.cursor.clone(),
        },
    );
    let crop_settings = (target.mode == CaptureMode::Display)
        .then(autocrop::Settings::configured)
        .flatten();
    let ((transcript, transcribe_ms), (predecoded, prepared)) = std::thread::scope(|scope| {
        let transcribe_handle = scope.spawn(|| {
            let t0 = std::time::Instant::now();
            let tr = transcribe::transcribe(
                has_wav.then_some(wav_path.as_path()),
                recorded_audio.as_ref(),
            );
            (tr, t0.elapsed().as_millis() as u64)
        });
        let pre = if let Some(ref movie) = video_path
            && !frames.is_empty()
        {
            let mut preview_marks = marks_of(&strokes);
            let budget = keyframe_budget(duration_ms);
            let preview = select_keyframes(
                &frames,
                &strokes,
                &mut preview_marks,
                &events,
                &TranscriptData::default(),
                budget,
            );
            // Marks alone filled the budget, so speech can't change the selection.
            let marks_fill = preview.len() == budget
                && preview
                    .iter()
                    .all(|(k, _)| k.reason == model::KeyframeReason::Mark);
            let times: Vec<u64> = preview.iter().map(|(k, _)| k.t).collect();
            let decoded = movie_frames_at(movie, &times, video_offset_ms, 0).unwrap_or_default();
            if duration_ms >= 20_000 {
                let cache = if marks_fill {
                    Vec::new()
                } else {
                    decoded.clone()
                };
                let (merged, _) = merge_full_keyframes(preview, decoded);
                let vis = prepare_visuals(
                    merged,
                    frames.first(),
                    crop_settings.as_ref(),
                    &target,
                    duration_ms,
                    &strokes,
                    &preview_marks,
                    &events,
                    &activity,
                );
                (cache, vis)
            } else {
                (decoded, None)
            }
        } else {
            (Vec::new(), None)
        };
        (transcribe_handle.join().unwrap(), pre)
    });

    let built = build_bundle_from_parts(
        bundle_dir,
        target,
        duration_ms,
        frames,
        strokes,
        events,
        transcript,
        video_path.clone(),
        note,
        SourceExtras {
            activity,
            video_offset_ms,
            autocrop: Some(crop_settings),
            full_frames: video_path
                .map(|movie| movie_keyframes_with_cache(movie, video_offset_ms, predecoded)),
            prepared,
            timings: StageTimings {
                started_at: Some(started_at),
                stop_ms,
                transcribe_ms: Some(transcribe_ms),
                ..StageTimings::default()
            },
            ..SourceExtras::default()
        },
    );
    if built.is_ok() {
        recover::finished(bundle_dir);
    }
    built
}

/// Builds a Screen capture's bundle in `out`, which holds its video and `audio.wav`, from them
/// and what the recording logged: `scrawl reprocess` and `scrawl recover`.
pub fn rebuild_screen_bundle(out: &Path, old: recover::Unfinished) -> Result<String> {
    rebuild_screen_bundle_with(out, old, None, &recover::Media::SYSTEM)
}

/// `autocrop` as in `SourceExtras`: `None` follows the config, `Some(None)` keeps the whole frame.
pub fn rebuild_screen_bundle_with(
    out: &Path,
    old: recover::Unfinished,
    autocrop: Option<Option<autocrop::Settings>>,
    media: &recover::Media,
) -> Result<String> {
    let started_at = std::time::Instant::now();
    let video = old
        .video
        .as_ref()
        .map(|name| out.join(name))
        .filter(|p| p.is_file())
        .context("The bundle has no video")?;
    let wav = out.join("audio.wav");
    let recorded = if wav.is_file() {
        Some(audio::recorded_audio_from_wav(std::fs::read(&wav)?)?)
    } else {
        None
    };
    let offset = old.video_offset_ms;
    let ((transcript, transcribe_ms), joined) = std::thread::scope(|scope| {
        let frames = scope.spawn(|| {
            let t0 = std::time::Instant::now();
            let res = (media.frames)(&video, helper::FrameTimes::Every(220), 640);
            (res, t0.elapsed().as_millis() as u64)
        });
        let t0 = std::time::Instant::now();
        let tr = (media.transcribe)(recorded.as_ref().map(|_| wav.as_path()), recorded.as_ref());
        ((tr, t0.elapsed().as_millis() as u64), frames.join())
    });
    let (frames, candidates_ms) = joined.map_err(|_| anyhow::anyhow!("Frame decoding panicked"))?;
    let mut frames = frames?;
    for frame in &mut frames {
        frame.t_ms += offset;
    }
    let (decode, movie) = (media.frames, video.clone());
    let full_frames: Option<FullFrames> = Some(Box::new(move |times: &[u64]| {
        movie_frames_at_with(decode, &movie, times, offset, 0)
    }));
    Ok(build_bundle_from_parts(
        out,
        old.target,
        old.duration_ms,
        frames,
        old.strokes,
        old.events,
        transcript,
        Some(video),
        old.note,
        SourceExtras {
            activity: windows::ActivityLog {
                windows: old.windows,
                cursor: old.cursor,
                ..Default::default()
            },
            full_frames,
            video_offset_ms: offset,
            autocrop,
            timings: StageTimings {
                started_at: Some(started_at),
                candidates_ms: Some(candidates_ms),
                transcribe_ms: Some(transcribe_ms),
                ..Default::default()
            },
            ..SourceExtras::default()
        },
    )?
    .2)
}

/// A bundle without its transcript yet (the speech model was still downloading) gets it later.
fn mark_transcript_pending(bundle_dir: &Path, bundle: &ScrawlBundle) {
    if bundle.transcript.missing.as_ref().is_some_and(|m| m.retry) {
        speech_model::mark_pending(bundle_dir);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_bundle_from_parts(
    bundle_dir: &Path,
    target: TargetInfo,
    duration_ms: u64,
    frames: Vec<RgbaFrame>,
    strokes: Vec<Stroke>,
    events: Vec<InteractionEvent>,
    transcript: TranscriptData,
    video_path: Option<PathBuf>,
    note: Option<String>,
    extras: SourceExtras,
) -> Result<(PathBuf, ScrawlBundle, String)> {
    let total_started = extras
        .timings
        .started_at
        .unwrap_or_else(std::time::Instant::now);
    let mut marks = marks_of(&strokes);
    for (i, &t) in extras.flags.iter().enumerate() {
        marks.push(Mark {
            id: format!("f{}", i + 1),
            strokes: Vec::new(),
            t0: t,
            t1: t,
            box_norm: [0.0, 0.0, 1.0, 1.0],
            shape: Shape::Flag,
            from: None,
            to: None,
            pin: None,
            on: None,
            description: "flagged this moment".into(),
            frame: None,
        });
    }
    let bench = std::env::var_os("SCRAWL_BENCH").is_some();
    let started = std::time::Instant::now();
    let selected = select_keyframes(
        &frames,
        &strokes,
        &mut marks,
        &events,
        &transcript,
        keyframe_budget(duration_ms),
    );
    let select_only_at = std::time::Instant::now();
    let matches_prepared = extras.prepared.as_ref().is_some_and(|prep| {
        prep.keyframes.len() == selected.len()
            && prep.keyframes.iter().zip(&selected).all(|(a, (b, _))| {
                a.index == b.index
                    && a.t == b.t
                    && a.mark_id == b.mark_id
                    && a.cursor == b.cursor
                    && a.file == b.file
            })
    });
    enum BundleOutput {
        Prepared(PreparedVisuals),
        Rendered {
            annotated: Vec<(model::Keyframe, RgbaFrame)>,
            cropped: Option<(model::AutocropInfo, Vec<(model::Keyframe, RgbaFrame)>)>,
            full_sheet: Option<RgbaFrame>,
            contact_sheet: RgbaFrame,
        },
    }

    let mut extras = extras;
    let (keyframes, autocrop, selected_at, annotated_at, rendered_at, output) = if matches_prepared
    {
        drop(frames);
        let mut prep = extras.prepared.take().unwrap();
        let keyframes: Vec<model::Keyframe> = selected.into_iter().map(|(kf, _)| kf).collect();
        let selected_at = std::time::Instant::now();
        let autocrop = prep.autocrop.take();
        (
            keyframes,
            autocrop,
            selected_at,
            selected_at,
            selected_at,
            BundleOutput::Prepared(prep),
        )
    } else {
        // Only the first candidate is needed from here on (auto-crop's baseline); free the rest
        // before the full-size keyframes arrive.
        let baseline = {
            let mut frames = frames;
            frames.truncate(1);
            frames.pop()
        };
        let selected = match &extras.full_frames {
            Some(decode) if !selected.is_empty() => {
                let times: Vec<u64> = selected.iter().map(|(k, _)| k.t).collect();
                match decode(&times) {
                    Ok(full) => {
                        let total = selected.len();
                        let (merged, matched) = merge_full_keyframes(selected, full);
                        if matched < total {
                            capture::scrawl_log(format!(
                                "Full-quality keyframes: decoded {matched} of {total}"
                            ));
                        }
                        merged
                    }
                    Err(error) => {
                        capture::scrawl_log(format!("Full-quality keyframes failed: {error:#}"));
                        selected
                    }
                }
            }
            _ => selected,
        };
        let selected_at = std::time::Instant::now();

        let crop_settings = match extras.autocrop {
            Some(settings) => settings,
            None if target.mode == CaptureMode::Display => autocrop::Settings::configured(),
            None => None,
        };
        let (mut annotated_frames, mut cropped) = autocrop::annotate_and_crop(
            selected,
            baseline.as_ref(),
            crop_settings.as_ref(),
            &target,
            duration_ms,
            &strokes,
            &marks,
            &events,
            &extras.activity.windows,
            &extras.activity.cursor,
        );
        let keyframes: Vec<model::Keyframe> =
            annotated_frames.iter().map(|(kf, _)| kf.clone()).collect();
        let annotated_at = std::time::Instant::now();

        let (full_sheet, contact_sheet) = std::thread::scope(|scope| {
            let full_handle = cropped.as_ref().map(|(_, full)| {
                scope.spawn(|| render::render_contact_sheet_for_duration(duration_ms, full))
            });
            let sheet = render::render_contact_sheet_for_duration(duration_ms, &annotated_frames);
            (full_handle.map(|h| h.join().unwrap()), sheet)
        });
        render::stamp_keyframe_badges(&mut annotated_frames);
        if let Some((_, full)) = &mut cropped {
            render::stamp_keyframe_badges(full);
        }
        let rendered_at = std::time::Instant::now();
        let autocrop = cropped.as_ref().map(|(info, _)| info.clone());
        (
            keyframes,
            autocrop,
            selected_at,
            annotated_at,
            rendered_at,
            BundleOutput::Rendered {
                annotated: annotated_frames,
                cropped,
                full_sheet,
                contact_sheet,
            },
        )
    };

    let id = bundle_dir
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("scrawl-bundle")
        .to_string();
    let video_rel = video_path.and_then(|p| {
        p.exists()
            .then(|| p.file_name()?.to_str().map(str::to_owned))
            .flatten()
    });
    let audio_rel = bundle_dir
        .join("audio.wav")
        .exists()
        .then(|| "audio.wav".to_string());

    let bundle = ScrawlBundle {
        version: 1,
        id,
        created_at_ms: now_epoch_ms(),
        duration_ms,
        target,
        transcript,
        note,
        strokes,
        marks,
        events,
        keyframes,
        contact_sheet: "contact-sheet.png".into(),
        video: video_rel,
        video_offset_ms: extras.video_offset_ms,
        audio: audio_rel,
        source: extras.source,
        device: extras.device,
        context: extras.context,
        context_changes: extras.context_changes,
        screens: extras.screens,
        windows: extras.activity.windows,
        cursor: extras.activity.cursor,
        autocrop,
        destination: None,
    };
    let summary = summary_markdown(&bundle, bundle_dir);

    match output {
        BundleOutput::Prepared(prep) => {
            std::fs::create_dir_all(bundle_dir.join("frames"))
                .with_context(|| format!("could not create {}", bundle_dir.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(bundle_dir, std::fs::Permissions::from_mode(0o700));
            }
            std::fs::write(
                bundle_dir.join(&bundle.contact_sheet),
                &prep.contact_sheet_png,
            )?;
            for (rel, bytes) in &prep.frame_pngs {
                std::fs::write(bundle_dir.join(rel), bytes)?;
            }
            if let (Some(info), Some(full_sheet_png)) = (&bundle.autocrop, &prep.full_sheet_png) {
                let full_dir = bundle_dir.join(&info.full_frames);
                std::fs::create_dir_all(&full_dir)?;
                std::fs::write(bundle_dir.join(&info.full_contact_sheet), full_sheet_png)?;
                for (name, bytes) in &prep.full_frame_pngs {
                    std::fs::write(full_dir.join(name), bytes)?;
                }
            }
            let manifest_bytes = serde_json::to_vec_pretty(&bundle)?;
            std::fs::write(bundle_dir.join("scrawl.json"), manifest_bytes)?;
            std::fs::write(bundle_dir.join("summary.md"), summary.as_bytes())?;
        }
        BundleOutput::Rendered {
            annotated,
            cropped,
            full_sheet,
            contact_sheet,
        } => {
            std::thread::scope(|scope| -> Result<()> {
                let full_write = match (&cropped, &full_sheet) {
                    (Some((info, full)), Some(sheet)) => Some(
                        scope.spawn(|| autocrop::write_full_frames(bundle_dir, info, full, sheet)),
                    ),
                    _ => None,
                };
                write_bundle_artifacts(bundle_dir, &bundle, &annotated, &contact_sheet, &summary)?;
                if let Some(handle) = full_write {
                    handle
                        .join()
                        .map_err(|_| anyhow::anyhow!("Full-frame writer panicked"))??;
                }
                Ok(())
            })?;
        }
    }

    let finished = std::time::Instant::now();
    let select_ms = (select_only_at - started).as_millis() as u64;
    let decode_ms = (selected_at - select_only_at).as_millis() as u64;
    let autocrop_ms = (annotated_at - selected_at).as_millis() as u64;
    let sheet_ms = (rendered_at - annotated_at).as_millis() as u64;
    let write_ms = (finished - rendered_at).as_millis() as u64;
    let total_ms = (finished - total_started).as_millis() as u64;
    let mut parts = Vec::with_capacity(10);
    if let Some(ms) = extras.timings.stop_ms {
        parts.push(format!("stop {ms}ms"));
    }
    if let Some(ms) = extras.timings.audio_ms {
        parts.push(format!("audio {ms}ms"));
    }
    if let Some(ms) = extras.timings.candidates_ms {
        parts.push(format!("candidates {ms}ms"));
    }
    if let Some(ms) = extras.timings.transcribe_ms {
        parts.push(format!("transcribe {ms}ms"));
    }
    parts.extend([
        format!("select {select_ms}ms"),
        format!("decode {decode_ms}ms"),
        format!("autocrop {autocrop_ms}ms"),
        format!("sheet {sheet_ms}ms"),
        format!("write {write_ms}ms"),
        format!("total {total_ms}ms"),
    ]);
    capture::scrawl_log(format!("finish timings: {}", parts.join(" · ")));
    if bench && !matches_prepared {
        for (name, elapsed) in [
            ("selection_ms", selected_at - started),
            ("annotation_ms", annotated_at - selected_at),
            ("contact_sheet_ms", rendered_at - annotated_at),
            ("artifacts_ms", finished - rendered_at),
            ("pipeline_ms", finished - started),
        ] {
            eprintln!("BENCH {name}={:.3}", elapsed.as_secs_f64() * 1000.0);
        }
    }

    mark_transcript_pending(bundle_dir, &bundle);
    Ok((bundle_dir.to_path_buf(), bundle, summary))
}

/// Generates a rich, deterministic multi-frame demo bundle showcasing clicks, circles, arrows, pins, and aligned speech.
pub fn generate_demo_bundle(out: Option<PathBuf>) -> Result<(PathBuf, ScrawlBundle, String)> {
    let bundle_dir = out.unwrap_or_else(|| {
        handoff::default_captures_dir().join(format!("{}.scrawl", make_bundle_id()))
    });
    let (fw, fh) = (1280_u32, 800_u32);
    let target = TargetInfo {
        mode: CaptureMode::Window,
        app: Some("OpenCode".into()),
        bundle_id: Some("ai.opencode.desktop".into()),
        window_title: Some("session/tool-running — OpenCode".into()),
        window_id: Some(4210),
        display_id: 1,
        frame: TargetFrame {
            x: 100.0,
            y: 60.0,
            width: f64::from(fw),
            height: f64::from(fh),
            scale: 2.0,
        },
    };

    let frames = vec![
        render_synthetic_demo_frame(fw, fh, 400, 0),
        render_synthetic_demo_frame(fw, fh, 1_500, 1),
        render_synthetic_demo_frame(fw, fh, 3_600, 1),
        render_synthetic_demo_frame(fw, fh, 6_400, 2),
        render_synthetic_demo_frame(fw, fh, 9_200, 2),
    ];

    // 1. Circle around the shifted header band (cx=0.52, cy=0.14, r_x=0.14, r_y=0.06)
    let mut circle_pts = Vec::new();
    for i in 0..=36 {
        let angle = (i as f32 / 36.0) * std::f32::consts::TAU;
        let x = ((0.52 + 0.13 * angle.cos()) * SCALE as f32).round() as i32;
        let y = ((0.15 + 0.055 * angle.sin()) * SCALE as f32).round() as i32;
        circle_pts.extend_from_slice(&[x, y, i * 18]);
    }
    let circle_stroke = Stroke {
        t0: 2_900,
        t1: 2_900 + 36 * 18,
        fw,
        fh,
        on: Some("Band header".into()),
        pin: None,
        laser: false,
        pts: circle_pts,
    };

    // 2. Arrow pointing from sidebar item to the tool card (shaft + arrowhead)
    let mut shaft_pts = Vec::new();
    for i in 0..=12 {
        let t = i as f32 / 12.0;
        let x = ((0.22 + (0.44 - 0.22) * t) * SCALE as f32).round() as i32;
        let y = ((0.48 + (0.36 - 0.48) * t) * SCALE as f32).round() as i32;
        shaft_pts.extend_from_slice(&[x, y, i * 16]);
    }
    let arrow_shaft = Stroke {
        t0: 5_800,
        t1: 5_800 + 12 * 16,
        fw,
        fh,
        on: Some("Used 12 tools".into()),
        pin: None,
        laser: false,
        pts: shaft_pts,
    };
    let arrow_head = Stroke {
        t0: 6_100,
        t1: 6_160,
        fw,
        fh,
        on: Some("Used 12 tools".into()),
        pin: None,
        laser: false,
        pts: vec![
            (0.41 * SCALE as f32) as i32,
            (0.35 * SCALE as f32) as i32,
            0,
            (0.44 * SCALE as f32) as i32,
            (0.36 * SCALE as f32) as i32,
            30,
            (0.42 * SCALE as f32) as i32,
            (0.40 * SCALE as f32) as i32,
            60,
        ],
    };

    // 3. Numbered pin on the composer send button
    let pin_stroke = Stroke {
        t0: 8_900,
        t1: 8_900,
        fw,
        fh,
        on: Some("Send button".into()),
        pin: Some(1),
        laser: false,
        pts: vec![
            (0.88 * SCALE as f32) as i32,
            (0.88 * SCALE as f32) as i32,
            0,
        ],
    };

    let strokes = vec![circle_stroke, arrow_shaft, arrow_head, pin_stroke];
    let events = vec![
        InteractionEvent::Click {
            t: 1_250,
            x: 0.46,
            y: 0.36,
            button: Some("left".into()),
            label: Some("Used 12 tools".into()),
            role: Some("button".into()),
        },
        InteractionEvent::Scroll {
            t: 4_800,
            x: 0.55,
            y: 0.55,
            dx: 0.0,
            dy: -240.0,
            label: Some("Transcript".into()),
        },
    ];

    let transcript = TranscriptData {
        source: "speech-analyzer".into(),
        model: Some("SpeechTranscriber en_US".into()),
        text: "When I click Used 12 tools this band header jumps down by four pixels, and the row spacing under the card feels too tight.".into(),
        segments: vec![
            TranscriptSegment {
                t0: 600,
                t1: 4_100,
                text: "When I click Used 12 tools this band header jumps down by four pixels,".into(),
            },
            TranscriptSegment {
                t0: 5_400,
                t1: 9_400,
                text: "and the row spacing under the card feels too tight.".into(),
            },
        ],
        words: vec![
            WordTiming { t0: 600, t1: 850, text: "When".into() },
            WordTiming { t0: 850, t1: 980, text: "I".into() },
            WordTiming { t0: 980, t1: 1_320, text: "click".into() },
            WordTiming { t0: 1_320, t1: 1_650, text: "Used".into() },
            WordTiming { t0: 1_650, t1: 1_900, text: "12".into() },
            WordTiming { t0: 1_900, t1: 2_300, text: "tools".into() },
            WordTiming { t0: 2_450, t1: 2_700, text: "this".into() },
            WordTiming { t0: 2_700, t1: 3_050, text: "band".into() },
            WordTiming { t0: 3_050, t1: 3_400, text: "header".into() },
            WordTiming { t0: 3_400, t1: 3_700, text: "jumps".into() },
            WordTiming { t0: 3_700, t1: 3_900, text: "down".into() },
            WordTiming { t0: 3_900, t1: 4_100, text: "by four pixels,".into() },
            WordTiming { t0: 5_400, t1: 5_800, text: "and the".into() },
            WordTiming { t0: 5_800, t1: 6_300, text: "row spacing".into() },
            WordTiming { t0: 6_300, t1: 7_200, text: "under the card".into() },
            WordTiming { t0: 7_800, t1: 9_400, text: "feels too tight.".into() },
        ],
        estimated: false,
        missing: None,
    };

    build_bundle_from_parts(
        &bundle_dir,
        target,
        9_800,
        frames,
        strokes,
        events,
        transcript,
        None,
        Some("Header inset shifts on expand".into()),
        SourceExtras::default(),
    )
}

fn main() -> Result<()> {
    if std::env::var_os("SCRAWL_BENCH_START").is_some() {
        ui::run_start_benchmark();
        return Ok(());
    }
    let cli = Cli::parse();
    let command = match cli.command {
        Some(command) => command,
        // `scrawl` typed in a terminal (on PATH through Settings › Agents or Homebrew) would
        // start a second menu-bar app. macOS opens the app with stdin on /dev/null.
        // On Windows `scrawl.exe` is a console program and Explorer gives it a console too, so a
        // bare launch always starts the app.
        None if cfg!(target_os = "macos")
            && std::io::IsTerminal::is_terminal(&std::io::stdin()) =>
        {
            <Cli as clap::CommandFactory>::command().print_help()?;
            return Ok(());
        }
        None => Command::App {
            record: false,
            iphone: false,
        },
    };
    match command {
        Command::App { record, iphone } => {
            #[cfg(target_os = "windows")]
            {
                win::shell::release_own_console();
                win::shell::wait_for_relauncher();
            }
            ui::run_app(None, record, iphone, None);
            Ok(())
        }
        Command::Demo { out, clipboard } => {
            let (dir, _bundle, summary) = generate_demo_bundle(out)?;
            if clipboard {
                let _ = handoff::copy_bundle_to_clipboard(&dir);
            }
            print!("{summary}");
            Ok(())
        }
        Command::Record {
            window,
            iphone: true,
            duration,
            note,
            out,
        } => {
            let bundle_dir = out.unwrap_or_else(|| {
                handoff::default_captures_dir().join(format!("{}.scrawl", make_bundle_id()))
            });
            std::fs::create_dir_all(&bundle_dir)?;
            let dev_id = window.or_else(|| {
                helper::devices()
                    .ok()
                    .and_then(|mut d| d.drain(..).next().map(|p| p.id))
            });
            let playback = playback::Guard::start();
            let session = phone::PhoneSession::start(
                &bundle_dir.join("recording.mov"),
                dev_id.as_deref(),
                None,
            )?;
            let started = std::time::Instant::now();
            while !session.has_recording()
                && !session.has_ended()
                && started.elapsed() < Duration::from_secs(15)
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            let record_started = std::time::Instant::now();
            let target_dur = Duration::from_secs(duration.max(1));
            while !session.has_ended() && record_started.elapsed() < target_dur {
                std::thread::sleep(Duration::from_millis(50));
            }
            drop(playback);
            let recording = session.stop()?;
            let (_dir, _bundle, summary) =
                phone::finalize_live_capture(&bundle_dir, recording, Vec::new(), Vec::new(), note)?;
            print!("{summary}");
            Ok(())
        }
        Command::Record {
            window,
            iphone: false,
            duration,
            note,
            out,
        } => {
            let target = discover_target(window.as_deref());
            let bundle_dir = out.unwrap_or_else(|| {
                handoff::default_captures_dir().join(format!("{}.scrawl", make_bundle_id()))
            });
            std::fs::create_dir_all(&bundle_dir)?;
            let video_path = bundle_dir.join("recording.mp4");
            let playback = playback::Guard::start();
            let audio = AudioRecorder::start().ok();
            let session = ScreenCaptureSession::start(
                target.clone(),
                Some(video_path),
                std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            );
            let window_log = (target.mode == CaptureMode::Display).then(|| {
                windows::WindowLogger::start(
                    target.frame,
                    std::time::Instant::now(),
                    std::sync::Arc::default(),
                )
            });
            std::thread::sleep(Duration::from_secs(duration.max(1)));
            drop(playback);
            let recording = session.stop();
            let activity = window_log
                .map(windows::WindowLogger::stop)
                .unwrap_or_default();
            let recorded_audio = audio.and_then(|a| a.stop().ok());
            let (_dir, _bundle, summary) = finalize_capture_bundle(
                &bundle_dir,
                target,
                recording,
                Vec::new(),
                Vec::new(),
                recorded_audio,
                note,
                activity,
            )?;
            print!("{summary}");
            Ok(())
        }
        Command::Import {
            recording,
            timeline,
            note,
            out,
            clipboard,
            transcriber,
        } => {
            if let Some(engine) = transcriber {
                transcribe::set_override(engine);
            }
            let (movie, timeline) =
                phone::resolve_import(recording.as_deref(), timeline.as_deref())?;
            let bundle_dir = out.unwrap_or_else(|| {
                handoff::default_captures_dir().join(format!("{}.scrawl", make_bundle_id()))
            });
            let (dir, _bundle, summary) =
                phone::import(&movie, timeline.as_deref(), &bundle_dir, note)?;
            if clipboard {
                handoff::copy_bundle_to_clipboard(&dir)?;
            }
            print!("{summary}");
            Ok(())
        }
        Command::Retranscribe { bundle } => {
            let (dir, _) = handoff::load_bundle(&bundle)?;
            if speech_model::transcribe_again(&dir)? {
                print!("{}", std::fs::read_to_string(dir.join("summary.md"))?);
                Ok(())
            } else {
                anyhow::bail!("No speech model yet; try again once it has downloaded")
            }
        }
        Command::Reprocess {
            bundle,
            out,
            bench,
            live_speed,
            transcriber,
            experiments,
        } => {
            if let Some(engine) = transcriber {
                transcribe::set_override(engine);
            }
            for name in &experiments {
                handoff::enable_experiment(name);
            }
            let (dir, old) = handoff::load_bundle(&bundle)?;
            let video = old
                .video
                .as_ref()
                .map(|v| dir.join(v))
                .filter(|p| p.is_file())
                .with_context(|| {
                    retention::missing_media_reason(&dir)
                        .unwrap_or_else(|| "The bundle has no video to reprocess".to_owned())
                })?;
            let flags: Vec<u64> = old
                .marks
                .iter()
                .filter(|m| m.shape == Shape::Flag)
                .map(|m| m.t0)
                .collect();
            if let Some(iters) = bench {
                let iters = iters.max(1);
                let video_name = video.file_name().context("Bad video name")?.to_owned();
                let prep_iter = |i: u32| -> Result<(PathBuf, PathBuf)> {
                    let iter_out = if i + 1 == iters {
                        out.clone()
                    } else {
                        out.with_file_name(format!("bench-{i}.scrawl"))
                    };
                    let _ = std::fs::remove_dir_all(&iter_out);
                    std::fs::create_dir_all(&iter_out)?;
                    let linked = iter_out.join(&video_name);
                    std::fs::hard_link(&video, &linked)
                        .or_else(|_| std::fs::copy(&video, &linked).map(|_| ()))?;
                    Ok((iter_out, linked))
                };
                let mut runs_ms = Vec::with_capacity(iters as usize);
                if let Some(device) = old.device.clone() {
                    for i in 0..iters {
                        let (iter_out, linked) = prep_iter(i)?;
                        let t0 = std::time::Instant::now();
                        phone::build_movie_bundle(
                            &iter_out,
                            phone::MovieCapture {
                                movie: linked,
                                target: old.target.clone(),
                                device: device.clone(),
                                events: old.events.clone(),
                                strokes: old.strokes.clone(),
                                flags: flags.clone(),
                                note: old.note.clone(),
                                context: old.context.clone(),
                                context_changes: old.context_changes.clone(),
                                screens: old.screens.clone(),
                            },
                        )?;
                        let _ = handoff::copy_bundle_to_clipboard(&iter_out);
                        runs_ms.push(t0.elapsed().as_secs_f64() * 1_000.0);
                        if i + 1 != iters {
                            let _ = std::fs::remove_dir_all(&iter_out);
                        }
                    }
                } else {
                    let offset = old.video_offset_ms;
                    let raw_candidates = helper::frames(
                        &video,
                        helper::FrameTimes::Every(220),
                        capture::CANDIDATE_EDGE as u32,
                    )?;
                    let mut candidate_buf =
                        capture::CandidateBuffer::new(capture::CANDIDATE_BUDGET_BYTES);
                    for mut frame in raw_candidates {
                        frame.t_ms += offset;
                        let urgent = old.strokes.iter().any(|s| s.t1.abs_diff(frame.t_ms) <= 150)
                            || old.events.iter().any(|e| match e {
                                InteractionEvent::Click { t, .. }
                                | InteractionEvent::Point { t, .. } => {
                                    t.abs_diff(frame.t_ms) <= 150
                                }
                                InteractionEvent::Drag { t1, .. } => t1.abs_diff(frame.t_ms) <= 150,
                                _ => false,
                            });
                        candidate_buf.push(frame, urgent);
                    }
                    let candidates = candidate_buf.into_frames();
                    let wav_bytes = old
                        .audio
                        .as_ref()
                        .map(|a| dir.join(a))
                        .filter(|p| p.is_file())
                        .map(std::fs::read)
                        .transpose()?;
                    for i in 0..iters {
                        let (iter_out, linked) = prep_iter(i)?;
                        let iter_frames = candidates.clone();
                        let iter_strokes = old.strokes.clone();
                        let iter_events = old.events.clone();
                        let iter_target = old.target.clone();
                        let iter_note = old.note.clone();
                        let mut iter_activity = windows::ActivityLog {
                            windows: old.windows.clone(),
                            cursor: old.cursor.clone(),
                            ..Default::default()
                        };
                        let iter_wav = wav_bytes.clone();
                        let live = match (live_speed, &iter_wav) {
                            (Some(speed), Some(wav)) => {
                                Some(live_transcribe::simulate(wav, speed)?)
                            }
                            _ => None,
                        };
                        let t0 = std::time::Instant::now();
                        let recorded = iter_wav
                            .map(audio::recorded_audio_from_wav)
                            .transpose()?
                            .map(|mut recorded| {
                                recorded.live = live_transcribe::Slot::new(live);
                                recorded
                            });
                        iter_activity.stop_started = Some(t0);
                        iter_activity.stop_ms = Some(t0.elapsed().as_millis() as u64);
                        finalize_capture_bundle(
                            &iter_out,
                            iter_target,
                            capture::ScreenRecording {
                                frames: iter_frames,
                                video: Some(linked),
                                duration_ms: old.duration_ms,
                                video_offset_ms: offset,
                            },
                            iter_strokes,
                            iter_events,
                            recorded,
                            iter_note,
                            iter_activity,
                        )?;
                        let _ = handoff::copy_bundle_to_clipboard(&iter_out);
                        runs_ms.push(t0.elapsed().as_secs_f64() * 1_000.0);
                        if i + 1 != iters {
                            let _ = std::fs::remove_dir_all(&iter_out);
                        }
                    }
                }
                let mut sorted = runs_ms.clone();
                sorted.sort_by(|a, b| a.total_cmp(b));
                let median = sorted[sorted.len() / 2];
                eprintln!(
                    "BENCH_FINISH runs_ms={:?} median_ms={:.1}",
                    runs_ms
                        .iter()
                        .map(|v| (*v * 10.0).round() / 10.0)
                        .collect::<Vec<_>>(),
                    median
                );
                return Ok(());
            }
            ensure!(!out.exists(), "{} already exists", out.display());
            std::fs::create_dir_all(&out)?;
            let copied = out.join(video.file_name().context("Bad video name")?);
            std::fs::copy(&video, &copied)?;
            let timeline_src = dir.join("timeline.json");
            if timeline_src.is_file() {
                let _ = std::fs::copy(&timeline_src, out.join("timeline.json"));
            }
            let summary = if let Some(device) = old.device.clone() {
                phone::build_movie_bundle(
                    &out,
                    phone::MovieCapture {
                        movie: copied,
                        target: old.target,
                        device,
                        events: old.events,
                        strokes: old.strokes,
                        flags,
                        note: old.note,
                        context: old.context,
                        context_changes: old.context_changes,
                        screens: old.screens,
                    },
                )?
                .2
            } else {
                if let Some(audio) = old
                    .audio
                    .as_ref()
                    .map(|a| dir.join(a))
                    .filter(|p| p.is_file())
                {
                    std::fs::copy(audio, out.join("audio.wav"))?;
                }
                rebuild_screen_bundle(
                    &out,
                    recover::Unfinished {
                        target: old.target,
                        duration_ms: old.duration_ms,
                        video: copied
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned()),
                        video_offset_ms: old.video_offset_ms,
                        strokes: old.strokes,
                        events: old.events,
                        note: old.note,
                        windows: old.windows,
                        cursor: old.cursor,
                    },
                )?
            };
            print!("{summary}");
            Ok(())
        }
        Command::Recover {
            bundle: Some(bundle),
            ..
        } => {
            print!("{}", recover::rebuild(&bundle, &recover::Media::SYSTEM)?);
            Ok(())
        }
        Command::Recover {
            bundle: None,
            captures,
        } => {
            let captures = captures.unwrap_or_else(handoff::default_captures_dir);
            let results = recover::run(&captures);
            let failed = results.iter().filter(|(_, r)| r.is_err()).count();
            for (dir, result) in results {
                match result {
                    Ok(_) => println!("rebuilt {}", dir.display()),
                    Err(error) => println!("failed  {}: {error:#}", dir.display()),
                }
            }
            ensure!(failed == 0, "{failed} bundle(s) could not be rebuilt");
            Ok(())
        }
        Command::Lab {
            command:
                LabCommand::Pill {
                    scene,
                    trace,
                    repeat,
                    time_scale,
                    no_backdrop,
                },
        } => {
            // Set before the app starts any thread; the lab reads them once.
            unsafe {
                if let Some(trace) = trace {
                    std::env::set_var("SCRAWL_MOTION_TRACE", trace);
                }
                if let Some(scale) = time_scale {
                    std::env::set_var("SCRAWL_TIME_SCALE", scale.to_string());
                }
            }
            ui::run_lab(ui::LabOptions {
                scene,
                repeat,
                backdrop: !no_backdrop,
            });
            Ok(())
        }
        Command::Tune { command } => ui::tune_command(match command.unwrap_or(TuneCommand::List) {
            TuneCommand::List => ui::TuneAction::List,
            TuneCommand::Set { key, value } => ui::TuneAction::Set(key, value),
            TuneCommand::Preset { number } => ui::TuneAction::Preset(number),
            TuneCommand::Reset => ui::TuneAction::Reset,
            TuneCommand::Rust => ui::TuneAction::Rust,
        }),
        Command::Mic {
            command: MicCommand::List,
        } => {
            mic_list();
            Ok(())
        }
        Command::Mic {
            command: MicCommand::Watch,
        } => {
            let _watch = mic::Watch::start(|| {
                println!("[{}]", now_epoch_ms());
                mic_list();
            });
            loop {
                std::thread::park();
            }
        }
        Command::Mic {
            command: MicCommand::Set { id },
        } => {
            let inputs = mic::inputs();
            if id != "default" && inputs.find(&id).is_none() {
                anyhow::bail!("No connected microphone has id {id}; see `scrawl mic list`");
            }
            mic::save_in(&handoff::default_config_path(), &id, &inputs)?;
            mic_list();
            Ok(())
        }
        Command::Hex {
            command: HexCommand::Status,
        } => {
            hex_status();
            Ok(())
        }
        Command::Devices => {
            for device in helper::devices()? {
                println!("{}\t{}", device.name, device.id);
            }
            Ok(())
        }
        Command::SckProbe { watch } => {
            let end =
                std::time::Instant::now() + std::time::Duration::from_secs(watch.unwrap_or(0));
            loop {
                let started = std::time::Instant::now();
                let preflight = capture::screen_capture_granted();
                let result = capture::screen_capture_probe();
                let mut line = serde_json::json!({
                    "pid": std::process::id(),
                    "preflight": preflight,
                    "ok": result.is_ok(),
                    "ms": started.elapsed().as_millis() as u64,
                });
                match result {
                    Ok((w, h)) => line["screenshot"] = format!("{w}x{h}").into(),
                    Err(error) => line["error"] = error.into(),
                }
                println!("{line}");
                if std::time::Instant::now() >= end {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            Ok(())
        }
        Command::Inspect {
            bundle,
            json,
            marks,
            experiments,
        } => {
            for name in &experiments {
                handoff::enable_experiment(name);
            }
            let (dir, loaded) = handoff::load_bundle(&bundle)?;
            if marks {
                let entries = agent_context::mark_entries(&loaded);
                if json {
                    let list: Vec<_> = entries.iter().map(|e| e.json()).collect();
                    println!("{}", serde_json::to_string_pretty(&list)?);
                } else if entries.is_empty() {
                    println!("No marks");
                } else {
                    for entry in &entries {
                        println!("{}", entry.line());
                    }
                }
            } else if json {
                println!("{}", serde_json::to_string_pretty(&loaded)?);
            } else {
                print!("{}", summary_markdown(&loaded, &dir));
            }
            Ok(())
        }
        Command::Clean {
            dry_run,
            captures,
            days_ahead,
        } => {
            let captures = captures.unwrap_or_else(handoff::default_captures_dir);
            let policy = retention::Policy::load();
            let ahead = Duration::from_secs_f64(days_ahead.max(0.0) * 86_400.0);
            let (actions, report) = retention::run_pass(&captures, &policy, dry_run, ahead, true);
            for action in &actions {
                let line = match action {
                    retention::Action::TrashMedia { bundle, .. } => {
                        format!("trash media  {}", bundle.display())
                    }
                    retention::Action::TrashBundle(bundle) => {
                        format!("trash bundle {}", bundle.display())
                    }
                    retention::Action::Shrink { video, .. } => {
                        format!("shrink       {}", video.display())
                    }
                };
                println!("{line}");
            }
            if !dry_run {
                println!(
                    "{} freed; {} is now {}",
                    retention::format_bytes(report.freed),
                    captures.display(),
                    retention::format_bytes(retention::captures_size(&captures))
                );
            }
            Ok(())
        }
        Command::Pin { bundle, off } => {
            let dir = handoff::resolve_bundle_dir(&bundle)?;
            let marker = dir.join(retention::PINNED);
            if off {
                let _ = std::fs::remove_file(&marker);
                println!("Unpinned {}", dir.display());
            } else {
                std::fs::write(&marker, "")?;
                println!("Pinned {}", dir.display());
            }
            Ok(())
        }
        Command::Frames {
            bundle,
            from,
            to,
            fps,
        } => {
            let from_ms =
                parse_clock_ms(&from).with_context(|| format!("invalid --from time: {from}"))?;
            let to_ms = parse_clock_ms(&to).with_context(|| format!("invalid --to time: {to}"))?;
            let extracted = handoff::extract_frames_range(&bundle, from_ms, to_ms, fps)?;
            for path in extracted {
                println!("{}", path.display());
            }
            Ok(())
        }
        Command::Export {
            bundle,
            out,
            crop,
            size,
            zoom,
            no_ink,
            no_clicks,
            hevc,
            fps,
        } => {
            let (dir, loaded) = handoff::load_bundle(&bundle)?;
            let options = export::Options {
                out,
                crop,
                size,
                zoom,
                ink: !no_ink,
                clicks: !no_clicks,
                hevc,
                fps,
                background: false,
            };
            let mut last = -1;
            let exported = export::export(&dir, &loaded, &options, &mut |fraction| {
                let percent = (fraction * 100.0) as i32;
                if percent != last && std::io::IsTerminal::is_terminal(&std::io::stderr()) {
                    last = percent;
                    eprint!("\rExporting {percent}%");
                }
            });
            if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
                eprintln!();
            }
            println!("{}", export::describe(&exported?));
            Ok(())
        }
        Command::Zoom {
            bundle,
            at,
            mark,
            frame,
            span,
            frames,
            full,
        } => {
            let (dir, loaded) = handoff::load_bundle(&bundle)?;
            ensure!(span.is_finite() && span > 0.0, "--span must be positive");
            let focus = if let Some(query) = &mark {
                let entry =
                    agent_context::find_mark(&loaded, query, frame).map_err(anyhow::Error::msg)?;
                println!("{}", entry.line());
                zoom::Focus::Mark(entry)
            } else {
                zoom::Focus::At(at.as_deref().context("Pass --at or --mark")?)
            };
            let sheet = zoom::zoom(&dir, &loaded, focus, (span * 1_000.0) as u64, frames, !full)?;
            println!("{}", sheet.path.display());
            println!(
                "{} → {}, {} frames: {}",
                zoom::clock(sheet.from_ms),
                zoom::clock(sheet.to_ms),
                sheet.cells.len(),
                sheet
                    .cells
                    .iter()
                    .map(|(i, t)| format!("#{i} {}", zoom::clock(*t)))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if !sheet.said.is_empty() {
                println!("Said: “{}”", sheet.said);
            }
            Ok(())
        }
        Command::Autocrop {
            bundle,
            open,
            serve,
            port,
        } => {
            let (dir, loaded) = handoff::load_bundle(&bundle)?;
            if loaded.windows.is_empty() {
                eprintln!(
                    "No window log in this bundle (record in Screen mode to get one); \
                     the playground falls back to activity only."
                );
            }
            if serve {
                return autocrop_lab::serve(&dir, &loaded, port, open);
            }
            let page = autocrop_lab::export(&dir, &loaded)?;
            println!("{}", page.display());
            if open {
                handoff::open_path(&page)?;
            }
            Ok(())
        }
        Command::Fixture {
            scenario,
            out,
            settings,
        } => {
            let settings = settings
                .map(|p| -> Result<autocrop::Settings> {
                    Ok(serde_json::from_slice(&std::fs::read(p)?)?)
                })
                .transpose()?;
            let dir = autocrop_lab::build_fixture(&scenario, &out, settings)?;
            println!("{}", dir.display());
            Ok(())
        }
        Command::CaptureSim {
            scenario,
            seconds,
            movie,
            legacy,
            out,
        } => {
            let report =
                autocrop_lab::simulate_capture(&scenario, seconds, movie.as_deref(), legacy, &out)?;
            println!("{}", serde_json::to_string(&report)?);
            Ok(())
        }
        Command::AutocropEval {
            bundles,
            settings,
            sweep,
            json,
            export,
        } => {
            let settings: autocrop::Settings = match settings {
                Some(p) => serde_json::from_slice(&std::fs::read(p)?)?,
                None => autocrop::Settings::default(),
            };
            let mut dirs = Vec::new();
            for path in bundles {
                if path.join("scrawl.json").is_file() {
                    dirs.push(path);
                } else {
                    let mut found: Vec<PathBuf> = std::fs::read_dir(&path)?
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| p.join("truth.json").is_file())
                        .collect();
                    found.sort();
                    dirs.extend(found);
                }
            }
            let cases = dirs
                .iter()
                .map(|d| autocrop_lab::load_case(d))
                .collect::<Result<Vec<_>>>()?;
            if let Some(path) = export {
                std::fs::write(path, autocrop_lab::export_cases(&cases)?)?;
            }
            autocrop_lab::print_scores("uncropped", &autocrop_lab::score_uncropped(&cases));
            let scores = autocrop_lab::score_all(&cases, &settings);
            autocrop_lab::print_scores("settings", &scores);
            let scores = if sweep {
                let (best, _) = autocrop_lab::sweep(&cases, &settings);
                let scores = autocrop_lab::score_all(&cases, &best);
                autocrop_lab::print_scores("swept", &scores);
                println!("{}", serde_json::to_string_pretty(&best)?);
                scores
            } else {
                scores
            };
            if let Some(path) = json {
                std::fs::write(path, serde_json::to_vec_pretty(&scores)?)?;
            }
            Ok(())
        }
        Command::Windows { repeat } => {
            let target = discover_target(None);
            let dock = windows::dock_rect(target.frame);
            let mut cache = std::collections::HashMap::new();
            let started = std::time::Instant::now();
            let mut windows = Vec::new();
            for _ in 0..repeat.max(1) {
                windows = windows::snapshot(target.frame, dock, &mut cache);
            }
            let each = started.elapsed().as_secs_f64() * 1_000.0 / f64::from(repeat.max(1));
            println!("{}", serde_json::to_string_pretty(&windows)?);
            eprintln!(
                "{} windows on display {} ({}×{} pt), {each:.2} ms per snapshot",
                windows.len(),
                target.display_id,
                target.frame.width,
                target.frame.height
            );
            Ok(())
        }
        Command::Send {
            bundle,
            session,
            reroute,
            clipboard,
        } => {
            let dir = handoff::resolve_bundle_dir(&bundle)?;
            if clipboard {
                handoff::copy_bundle_to_clipboard(&dir)?;
                println!("Copied {} to clipboard", dir.display());
            } else {
                let target_session = handoff::send_to_opencode(&dir, session.as_deref(), reroute)?;
                println!(
                    "Sent {} to OpenCode session {target_session}",
                    dir.display()
                );
            }
            Ok(())
        }
        Command::Route {
            url,
            browser,
            explain,
        } => {
            let url = match (url, browser) {
                (Some(url), _) => Some(url),
                (None, Some(browser)) => Some(
                    routing::browser_tab_url(&browser)
                        .with_context(|| format!("Couldn't read the front tab of {browser}"))?,
                ),
                (None, None) => None,
            };
            if let Some(bundle) = explain {
                print!("{}", routing::explain_bundle(&bundle, url.as_deref())?);
                return Ok(());
            }
            let url = url.context("Pass --url or --browser")?;
            match routing::route_with_sites(&url, &handoff::routing_sites(), &routing::Hooks::default()) {
                Ok(dest) => println!("{}", serde_json::to_string_pretty(&dest)?),
                Err(reason) => println!("{url}: clipboard only ({reason})"),
            }
            Ok(())
        }
        Command::Preview {
            bundle,
            capture_screenshot,
        } => {
            let initial = handoff::load_bundle(&bundle).ok().or_else(|| {
                generate_demo_bundle(None)
                    .ok()
                    .map(|(dir, loaded, _)| (dir, loaded))
            });
            ui::run_app(initial, false, false, capture_screenshot);
            Ok(())
        }
        Command::Link {
            scheme,
            urls,
            session,
            port,
        } => {
            let token = handoff::ensure_ingest_token()?;
            let resolved_urls = if urls.is_empty() {
                phone::discover_default_ingest_urls(port)
            } else {
                urls
            };
            ensure!(
                !resolved_urls.is_empty(),
                "Could not discover a Tailscale or LAN IP; pass --url http://<host>:{port}"
            );
            let link =
                phone::format_setup_link(&scheme, &resolved_urls, &token, session.as_deref())?;
            println!("{link}");
            if let Ok(qr) = helper::qr(&link) {
                eprintln!("\n{qr}");
            }
            Ok(())
        }
        Command::Serve {
            port,
            usb,
            no_opencode,
        } => phone::run_ingest_server(port, usb, no_opencode),
        Command::Skill => {
            print!("{SKILL}");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_failures_do_not_publish_a_manifest() {
        let root = std::env::temp_dir().join(make_bundle_id());
        let (_, mut bundle, _) = generate_demo_bundle(Some(root.join("demo.scrawl"))).unwrap();
        let frame = RgbaFrame::new(0, 1, 1, vec![20, 30, 40, 255]);
        let mut key = bundle.keyframes[0].clone();
        key.file = "frames/missing/frame.png".into();
        let frame_failure = root.join("frame-failure.scrawl");
        assert!(
            write_bundle_artifacts(
                &frame_failure,
                &bundle,
                &[(key, frame.clone())],
                &frame,
                "summary"
            )
            .is_err()
        );
        assert!(!frame_failure.join("scrawl.json").exists());

        bundle.contact_sheet = "missing/contact-sheet.png".into();
        let sheet_failure = root.join("sheet-failure.scrawl");
        assert!(write_bundle_artifacts(&sheet_failure, &bundle, &[], &frame, "summary").is_err());
        assert!(!sheet_failure.join("scrawl.json").exists());

        bundle.contact_sheet = "contact-sheet.png".into();
        let empty_frames = root.join("empty-frames.scrawl");
        write_bundle_artifacts(&empty_frames, &bundle, &[], &frame, "summary").unwrap();
        assert!(empty_frames.join("scrawl.json").exists());
        assert!(empty_frames.join("contact-sheet.png").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn full_keyframes_match_by_time_and_preserve_partial_decodes() {
        let kf = |index: usize, t: u64| model::Keyframe {
            index,
            t,
            reason: model::KeyframeReason::Click,
            file: format!("frames/frame-{index:02}.png"),
            event: "tap".into(),
            said: None,
            mark_id: None,
            cursor: None,
        };
        let small = |t: u64| RgbaFrame::new(t, 32, 32, vec![10; 32 * 32 * 4]);
        let large = |t: u64| RgbaFrame::new(t, 160, 160, vec![200; 160 * 160 * 4]);

        let selected = vec![
            (kf(1, 1_000), small(1_000)),
            (kf(2, 2_000), small(2_000)),
            (kf(3, 3_000), small(3_000)),
        ];
        // Frame at 2_000 failed to decode and frames arrived out of order (3_000 clamped to 2_999).
        let full = vec![large(2_999), large(1_000)];
        let (merged, matched) = merge_full_keyframes(selected, full);
        assert_eq!(matched, 2);
        assert_eq!(merged[0].1.width, 160);
        assert_eq!(merged[0].1.t_ms, 1_000);
        assert_eq!(merged[1].1.width, 32);
        assert_eq!(merged[1].1.t_ms, 2_000);
        assert_eq!(merged[2].1.width, 160);
        assert_eq!(merged[2].1.t_ms, 3_000);
    }
}
