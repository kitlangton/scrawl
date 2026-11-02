use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};

use crate::capture::agent::context as agent_context;
use crate::capture::engine::runtime::phone;
use crate::capture::engine::runtime::audio::{self, mic};
use crate::capture::desktop::helper;
use crate::capture::desktop::workflow::{agent_capture, launch_instance};
use crate::capture::desktop as ui;
#[cfg(target_os = "linux")]
use crate::capture::desktop::{linux, playback};
#[cfg(target_os = "macos")]
use crate::capture::desktop::playback;
use crate::shared::{handoff, hex, model, windows};
use crate::capture::engine::autocrop::autocrop as autocrop;
use crate::capture::engine::{ export};
use crate::capture::engine::autocrop::autocrop_lab;
use crate::capture::engine::bundle::{recover, retention};
use crate::capture::engine::runtime::audio::transcription::{speech_model, transcribe};
#[cfg(target_os = "linux")]
use crate::capture::engine::runtime::audio::transcription::pause_transcribe;
use crate::capture::agent::zoom;
#[cfg(not(target_os = "linux"))]
use crate::capture::engine::runtime::audio::transcription::live_transcribe;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::connect::hosted;

use crate::capture::engine::runtime::audio::audio::AudioRecorder;
use crate::capture::engine::runtime::{ScreenCaptureSession, discover_target, record_target};
use crate::shared::model::{CaptureMode, InteractionEvent, Shape, parse_clock_ms};
use crate::capture::engine::sketch::summary_markdown;
use crate::shared::clock::now_epoch_ms;
use crate::shared::ids::make_bundle_id;
use crate::capture::desktop::demo::generate_demo_bundle;
use crate::capture::engine::bundle::bundle_processing::{finalize_capture_bundle, rebuild_screen_bundle};

#[derive(Parser)]
#[command(
    name = "apparent",
    version,
    about = "Shortcut-triggered screen, voice, and drawing capture for agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

fn parse_engine(name: &str) -> Result<crate::capture::engine::runtime::audio::transcription::transcribe::Engine, String> {
    crate::capture::engine::runtime::audio::transcription::transcribe::Engine::parse(name)
        .ok_or_else(|| "expected `apple`, `hex` or `command`".to_owned())
}

/// `apparent hex status`: connection, models and what the next recording would use.
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
    println!("Discovery file: {}", hex::discovery_path().display());
    let status = match hex::status() {
        Ok(status) => status,
        Err(error) => {
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
    /// Record from this device id, or `default` to follow the system input.
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
    /// Whether Hex is connected, its version, ready models and the transcriber Apparent will use.
    Status,
}

#[cfg(target_os = "linux")]
#[derive(Subcommand)]
enum UpdateCommand {
    /// Show the selected channel, live process version, and installed update as JSON.
    Status,
    /// Select stable releases or builds from main (does not download anything).
    Channel { #[arg(value_parser = ["stable", "main"])] channel: String },
    /// Check the chosen channel now and install an update for the next launch.
    Check,
}

#[derive(Subcommand)]
enum LabCommand {
    /// Play a capsule scene: tap-then-area, tap-then-window, tap-then-screen, picker, finish.
    Pill {
        #[arg(value_enum)]
        scene: ui::LabScene,
        /// Write a motion-lint trace of every frame here (same as `APPARENT_MOTION_TRACE`).
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
    /// Preview or explicitly submit text-only feedback to Apparent's GitHub repository.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    Issue(super::issues::IssueArgs),
    /// Opt-in hosted account and upload queue management.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[command(alias = "cloud")]
    Connect { #[command(subcommand)] command: hosted::ConnectCommand },
    /// Retrieve and validate a complete hosted capture using a separate read credential.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    Fetch {
        id: String,
        #[arg(long)] out: PathBuf,
        #[arg(long, default_value_t = 0)] wait: u64,
    },
    /// Open Apparent without recording. Only apparent://open is accepted.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    Launch { url: Option<String> },
    /// Explicitly register the Linux URL handler, or inspect the desktop's selected handler.
    #[cfg(target_os = "linux")]
    LaunchHandler {
        #[command(subcommand)]
        command: LaunchHandlerCommand,
    },
    /// Inspect or explicitly end a GUI capture. Same user and machine only; never toggles or sends.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    Capture { #[command(subcommand)] command: CaptureCommand },
    /// Launch the GPUI menu-bar and review application.
    App {
        /// Start a capture session immediately on launch.
        #[arg(long)]
        record: bool,
        /// Start capturing the cabled iPhone immediately on launch.
        #[arg(long, conflicts_with = "record")]
        iphone: bool,
    },
    /// Inspect or manage Linux AppImage updates.
    #[cfg(target_os = "linux")]
    Update { #[command(subcommand)] command: UpdateCommand },
    /// Hand a shortcut press to the running app (Linux: GNOME's custom keybindings run this
    /// where there's no GlobalShortcuts portal).
    #[cfg(target_os = "linux")]
    #[command(hide = true)]
    Shortcut {
        #[arg(value_parser = ["record", "paste"])]
        which: String,
    },
    /// Generate a realistic annotated demo bundle (contact sheet + keyframes + transcript).
    Demo {
        /// Output `.apparent` bundle directory (defaults to Application Support).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Copy the generated bundle to the macOS clipboard.
        #[arg(long)]
        clipboard: bool,
    },
    /// Record for a fixed duration. Timed CLI screen capture does not collect clicks or Shift ink; use the app's recording shortcut for interaction telemetry.
    Record {
        /// Temporarily unavailable: Window recording is disabled; this option returns an error without recording.
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
        /// Output `.apparent` bundle directory.
        #[arg(long)]
        out: Option<PathBuf>,
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
    /// Verify that this Linux runtime can decode an H.264 MP4 into keyframes.
    #[cfg(target_os = "linux")]
    #[command(hide = true)]
    DecoderCheck { movie: PathBuf },
    /// Verify that this Linux runtime can encode a synthetic H.264 MP4.
    #[cfg(target_os = "linux")]
    #[command(hide = true)]
    EncoderCheck { movie: PathBuf },
    /// Transcribe a bundle's audio again with Apple Speech, in place: its transcript, each
    /// keyframe's words, `apparent.json` and `summary.md`. Apparent does this for captures
    /// made while the speech model was still downloading.
    Retranscribe {
        /// Path to `.apparent` bundle directory or `latest`.
        bundle: PathBuf,
    },
    /// Rebuild a bundle from its own video, audio, and input log with the current transcriber
    /// and keyframe rules.
    Reprocess {
        /// Path to `.apparent` bundle directory or `latest`.
        bundle: PathBuf,
        /// Output `.apparent` bundle directory (a new bundle; the original is untouched).
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
        transcriber: Option<crate::capture::engine::runtime::audio::transcription::transcribe::Engine>,
        /// Turn an experiment on for this run, as `"experiments"` in `config.json` would
        /// (e.g. `agentContext`). Repeatable.
        #[arg(long = "experiment", value_name = "NAME")]
        experiments: Vec<String>,
    },
    /// Build the bundles whose build died with the app (no `apparent.json`) in place, once each:
    /// from their input log (`unfinished.json`) if they have one, else from `recording.mp4` and
    /// `audio.wav` alone. The app runs it at launch.
    Recover {
        /// One bundle to build now, even if it was tried before.
        bundle: Option<PathBuf>,
        /// Captures folder (defaults to Apparent's).
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
    /// Tune the capsule's springs live: edits `~/.config/apparent/tune.json`, which debug builds
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
    /// Check the Hex transcriber: is it running, which models are ready, which one Apparent uses.
    Hex {
        #[command(subcommand)]
        command: HexCommand,
    },
    /// Print the agent-ready summary or JSON manifest for a `.apparent` bundle (or `latest`).
    Inspect {
        /// Path to `.apparent` bundle directory, `apparent.json`, or `latest`.
        #[arg(default_value = "latest")]
        bundle: PathBuf,
        /// Emit raw `apparent.json` instead of `summary.md` (with `--marks`, the marks as JSON).
        #[arg(long)]
        json: bool,
        /// List the marks: keyframe, number as the summary shows it (✎②), time, shape, what it
        /// was drawn on, and id, for `apparent zoom --mark`.
        #[arg(long)]
        marks: bool,
        /// Turn an experiment on for this summary (e.g. `agentContext`). Repeatable.
        #[arg(long = "experiment", value_name = "NAME")]
        experiments: Vec<String>,
    },
    /// Extract a dense burst of frames from a `.apparent` bundle's time range (`M:SS` or seconds).
    Frames {
        /// Path to `.apparent` bundle directory or `latest`.
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
        /// Path to `.apparent` bundle directory or `latest`.
        #[arg(default_value = "latest")]
        bundle: PathBuf,
        /// The moment: a time (`0:12`, `12.5`) or a phrase from the narration ("width changes").
        #[arg(long, required_unless_present = "mark", conflicts_with = "mark")]
        at: Option<String>,
        /// A mark, centred with its ink drawn: its number in the summary (`2` for ✎②) or its id
        /// (`m2`). `apparent inspect --marks` lists them.
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
    /// Export a bundle's recording as one shareable .mp4: H.264 video opening on the full capture
    /// with ink and click ripples burned in, AAC audio in sync, and smooth activity zoom by
    /// default (`--no-zoom` opts out). Encoding uses AVFoundation on Mac and FFmpeg on Linux.
    Export {
        /// Path to `.apparent` bundle directory or `latest`.
        #[arg(default_value = "latest")]
        bundle: PathBuf,
        /// Output file (defaults to ~/Movies/Apparent/<bundle>.mp4).
        #[arg(long)]
        out: Option<PathBuf>,
        /// `full`, `auto` (the activity region), or `x,y,w,h` as fractions of the capture (all
        /// ≤ 1) or pixels of the recording.
        #[arg(long, default_value = "full")]
        crop: export::crop::CropArg,
        /// Longest side of the output in pixels; it is never scaled up.
        #[arg(long, default_value_t = export::crop::DEFAULT_LONG_EDGE)]
        size: u32,
        /// Disable the default smooth camera zoom toward clicks, drags and marks.
        #[arg(long = "no-zoom", action = clap::ArgAction::SetFalse, default_value_t = true)]
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
        /// Replace recorded audio with local Kokoro bm_fable speech from a timed JSON script.
        #[arg(long, conflicts_with = "no_audio")]
        narration: Option<PathBuf>,
        /// Deliberately omit all audio, even when a recorded track exists.
        #[arg(long)]
        no_audio: bool,
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
        /// Path to `.apparent` bundle directory or `latest`.
        #[arg(default_value = "latest")]
        bundle: PathBuf,
        #[arg(long)]
        off: bool,
    },
    /// Open the auto-crop playground for a Screen capture: each keyframe with its windows,
    /// activity and crop, and the cropped contact sheet. `--serve` reruns the crop live as
    /// settings change; without it the page shows the presets.
    Autocrop {
        /// Path to `.apparent` bundle directory or `latest`.
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
    /// Build a bundle from a simulated Screen capture (`public/site/scripts/autocrop-corpus.mjs`).
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
    /// Copy a `.apparent` bundle to the clipboard.
    Copy {
        /// Path to `.apparent` bundle directory or `latest`.
        #[arg(default_value = "latest")]
        bundle: PathBuf,
    },
    /// Open a `.apparent` bundle in the native GPUI review window.
    Preview {
        /// Path to `.apparent` bundle directory or `latest`. Generates a demo bundle if none exists.
        #[arg(default_value = "latest")]
        bundle: PathBuf,
        /// Capture a window screenshot to this path and exit (for deterministic UI verification).
        #[arg(long)]
        capture_screenshot: Option<PathBuf>,
    },
    /// Print the agent skill that teaches coding agents to read captures and use this CLI (install from its direct GitHub path).
    Skill,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Subcommand)]
enum CaptureCommand {
    /// Read the running GUI's phase and opaque capture identity as JSON.
    State,
    /// Start the normal GUI screen capture without toggling or launching an app.
    Start {
        /// Wait until a real screen frame arrives (portal consent may be needed).
        #[arg(long)]
        wait: bool,
        /// Maximum wait in seconds. Timeout leaves an admitted capture intact; inspect its ID.
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout: u64,
        /// Retry a known start request without starting a second capture (32 hex characters).
        #[arg(long)]
        request_id: Option<String>,
    },
    /// Finish only a matching, already-active capture.
    Stop {
        capture_id: String,
        /// Wait for this capture's saved bundle, not merely stop acceptance.
        #[arg(long)]
        wait: bool,
        /// Maximum wait in seconds. Retry or inspect the same ID after timeout.
        #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout: u64,
    },
    /// Discard only a matching pending or active capture.
    Cancel { capture_id: String },
    /// Read this capture's result, retained only for the running GUI's lifetime.
    Inspect { capture_id: String },
}

/// `capture/agent/skills/apparent/SKILL.md`, the same text served from app.tryapparent.com/skill.md.
const SKILL: &str = include_str!("../capture/agent/skills/apparent/SKILL.md");

#[cfg(target_os = "linux")]
#[derive(Subcommand)]
enum LaunchHandlerCommand {
    Install,
    Status,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn run_owned_app(record: bool, iphone: bool) -> Result<()> {
    let Some(owner) = launch_instance::acquire()? else {
        #[cfg(target_os = "linux")]
        if record {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while !linux::ipc::running() {
                ensure!(
                    std::time::Instant::now() < deadline,
                    "running app is not ready"
                );
                std::thread::sleep(Duration::from_millis(25));
            }
            return linux::ipc::send("shortcut record");
        }
        return launch_instance::forward_open();
    };
    // Older releases do not own the new lock. Never start beside their GUI.
    #[cfg(target_os = "linux")]
    ensure!(
        !linux::ipc::running(),
        "running Apparent needs an update for native launch"
    );
    ensure!(
        !agent_capture::running_owner()?,
        "another Apparent GUI is running; update and restart it first"
    );
    owner.listen()?;
    hosted::start_worker();
    #[cfg(target_os = "linux")]
    if let Err(error) = linux::update::record_running() {
        eprintln!("Apparent could not record its running version in config: {error:#}");
    }
    ui::run_app(None, record, iphone, None);
    Ok(())
}

pub(crate) fn run() -> Result<()> {
    #[cfg(target_os = "linux")]
    linux::prepare_process();
    if std::env::var_os("APPARENT_BENCH_START").is_some() {
        ui::run_start_benchmark();
        return Ok(());
    }
    let cli = Cli::parse();
    let command = match cli.command {
        Some(command) => command,
        // `apparent` typed in a terminal (on PATH through Settings › Agents or Homebrew) would
        // start a second menu-bar app. macOS opens the app with stdin on /dev/null.
        // On Windows `apparent.exe` is a console program and Explorer gives it a console too, so a
        // bare launch always starts the app. Linux desktops start `apparent app` (apparent.desktop).
        None if cfg!(any(target_os = "macos", target_os = "linux"))
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
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        Command::Connect { command } => hosted::command(command),
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        Command::Issue(args) => super::issues::command(args),
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        Command::Fetch { id, out, wait } => hosted::fetch(&id, &out, wait),
        #[cfg(target_os = "linux")]
        Command::LaunchHandler { command } => match command {
            LaunchHandlerCommand::Install => linux::launch_handler::install(),
            LaunchHandlerCommand::Status => {
                println!("{}", linux::launch_handler::status()?);
                Ok(())
            }
        },
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        Command::Launch { url } => {
            if let Some(url) = url {
                launch_instance::request_url(&url)?;
            } else {
                launch_instance::request_open();
            }
            run_owned_app(false, false)
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        Command::Capture { command } => {
            use crate::capture::desktop::workflow::agent_capture::Action;
            let (action, id, wait, timeout) = match command {
                CaptureCommand::State => (None, None, false, 4),
                CaptureCommand::Start { wait, timeout, request_id } =>
                    (Some(Action::Start), Some(match request_id { Some(id) => id, None => agent_capture::new_id()? }), wait, timeout),
                CaptureCommand::Stop { capture_id, wait, timeout } => (Some(Action::Stop), Some(capture_id), wait, timeout),
                CaptureCommand::Cancel { capture_id } => (Some(Action::Cancel), Some(capture_id), false, 4),
                CaptureCommand::Inspect { capture_id } => (Some(Action::Inspect), Some(capture_id), false, 4),
            };
            let value = agent_capture::execute(action.zip(id.as_deref()), wait, Duration::from_secs(timeout));
            println!("{value}");
            ensure!(!value["error"].is_string(), "GUI capture control: {value}");
            ensure!(!matches!(value["outcome"].as_str(), Some("wrong_capture" | "not_ready")), "GUI capture control: {value}");
            Ok(())
        }
        Command::App { record, iphone } => {
            #[cfg(target_os = "windows")]
            {
                win::shell::release_own_console();
                win::shell::wait_for_relauncher();
            }
            // Launching Apparent again (from the app grid) opens the running one's Settings: GNOME
            // shows no tray icon without the AppIndicator extension.
            #[cfg(target_os = "linux")]
            if linux::ipc::running() {
                linux::ipc::send(if record { "shortcut record" } else { "open" })?;
                return Ok(());
            }
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            return run_owned_app(record, iphone);
            #[cfg(target_os = "windows")]
            {
                ui::run_app(None, record, iphone, None);
                Ok(())
            }
        }
        #[cfg(target_os = "linux")]
        Command::Update { command } => {
            match command {
                UpdateCommand::Status => println!("{}", serde_json::to_string_pretty(&linux::update::status()?)?),
                UpdateCommand::Channel { channel } => {
                    linux::update::set_channel_in(&handoff::default_config_path(), &channel)?;
                    println!("Update channel: {}", linux::update::status()?["channel"].as_str().unwrap_or("unknown"));
                }
                UpdateCommand::Check => println!("{}", linux::update::check_cli()?),
            }
            Ok(())
        }
        #[cfg(target_os = "linux")]
        Command::Shortcut { which } => {
            if linux::ipc::send(&format!("shortcut {which}")).is_err() && which == "record" {
                // Not running: start it, recording.
                let exe = std::env::var_os("APPIMAGE")
                    .map(PathBuf::from)
                    .map_or_else(std::env::current_exe, Ok)?;
                std::process::Command::new(exe)
                    .args(["app", "--record"])
                    .stdin(std::process::Stdio::null())
                    .spawn()?;
            }
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
                handoff::default_captures_dir().join(format!("{}.apparent", make_bundle_id()))
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
            let target = record_target(window.as_deref(), || {
                #[cfg(target_os = "macos")]
                anyhow::ensure!(
                    crate::capture::engine::runtime::screen_capture_usable(false),
                    "Screen Recording is not available to this process. Grant Apparent Screen Recording in System Settings, then quit and reopen the app."
                );
                #[cfg(target_os = "linux")]
                if let Some(warning) = linux::media::encoder_warning() {
                    anyhow::bail!("{warning}");
                }
                Ok(())
            })?;
            let bundle_dir = out.unwrap_or_else(|| {
                handoff::default_captures_dir().join(format!("{}.apparent", make_bundle_id()))
            });
            recover::begin_screen_capture(&bundle_dir)?;
            let _signals = crate::capture::desktop::workflow::shutdown::Signals::install()?;
            let video_path = bundle_dir.join("recording.mp4");
            let playback = playback::Guard::start();
            let audio = Some(AudioRecorder::start(&bundle_dir)?);
            let session = match ScreenCaptureSession::start(
                target.clone(),
                Some(video_path),
                std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            ) {
                Ok(session) => session,
                Err(error) => {
                    if let Err(cleanup) = recover::abandon_start(&bundle_dir, || drop(audio)) {
                        crate::capture::engine::runtime::apparent_log(format!("Failed capture startup cleanup at {}: {cleanup:#}", bundle_dir.display()));
                    }
                    return Err(error);
                }
            };
            let window_log = (target.mode == CaptureMode::Display).then(|| {
                windows::WindowLogger::start(
                    target.frame,
                    std::time::Instant::now(),
                    std::sync::Arc::default(),
                )
            });
            let deadline = std::time::Instant::now() + Duration::from_secs(duration.max(1));
            while std::time::Instant::now() < deadline && !crate::capture::desktop::workflow::shutdown::requested() {
                std::thread::sleep(Duration::from_millis(20));
            }
            drop(playback);
            let recording = session.stop();
            let activity = window_log
                .map(windows::WindowLogger::stop)
                .unwrap_or_default();
            let recorded_audio = audio::audio::stop_optional(audio)?;
            let (_dir, _bundle, summary) = finalize_capture_bundle(
                &bundle_dir,
                target,
                recording,
                Vec::new(),
                Vec::new(),
                recorded_audio,
                note,
                activity,
                Some(model::InteractionTelemetry::VideoOnly),
            )?;
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
                        out.with_file_name(format!("bench-{i}.apparent"))
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
                        crate::capture::engine::runtime::CANDIDATE_EDGE as u32,
                    )?;
                    let mut candidate_buf =
                        crate::capture::engine::runtime::CandidateBuffer::new(crate::capture::engine::runtime::CANDIDATE_BUDGET_BYTES);
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
                        #[cfg(target_os = "linux")]
                        let pause = match (live_speed, &iter_wav) {
                            (Some(speed), Some(wav)) if pause_transcribe::wanted() => Some(pause_transcribe::simulate(wav, speed)?),
                            _ => None,
                        };
                        #[cfg(not(target_os = "linux"))]
                        let live = match (live_speed, &iter_wav) {
                            (Some(speed), Some(wav)) => {
                                Some(live_transcribe::simulate(wav, speed)?)
                            }
                            _ => None,
                        };
                        let t0 = std::time::Instant::now();
                        let recorded = iter_wav
                            .map(audio::audio::recorded_audio_from_wav)
                            .transpose()?
                            .map(|mut recorded| {
                                #[cfg(not(target_os = "linux"))]
                                { recorded.live = live_transcribe::Slot::new(live); }
                                #[cfg(target_os = "linux")]
                                { recorded.pause = pause_transcribe::Slot::new(pause); }
                                recorded
                            });
                        iter_activity.stop_started = Some(t0);
                        iter_activity.stop_ms = Some(t0.elapsed().as_millis() as u64);
                        finalize_capture_bundle(
                            &iter_out,
                            iter_target,
                            crate::capture::engine::runtime::ScreenRecording {
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
                            old.interaction_telemetry,
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
                        interaction_telemetry: old.interaction_telemetry,
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
                    Ok(summary) if dir.join("narration-recovered.txt").exists() => print!("{summary}"),
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
                    std::env::set_var("APPARENT_MOTION_TRACE", trace);
                }
                if let Some(scale) = time_scale {
                    std::env::set_var("APPARENT_TIME_SCALE", scale.to_string());
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
                anyhow::bail!("No connected microphone has id {id}; see `apparent mic list`");
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
        #[cfg(target_os = "linux")]
        Command::DecoderCheck { movie } => {
            anyhow::ensure!(linux::media::h264_decoder_available(), "{}", linux::media::decoder_warning().unwrap_or_default());
            let frames = linux::media::frames(&movie, helper::FrameTimes::At(&[0]), 0)?;
            let frame = &frames[0];
            anyhow::ensure!(frame.width > 0 && frame.height > 0, "Decoded frame has no dimensions");
            println!("Decoded {} full-quality H.264 keyframe(s) ({}x{})", frames.len(), frame.width, frame.height);
            Ok(())
        }
        #[cfg(target_os = "linux")]
        Command::EncoderCheck { movie } => {
            linux::media::encoder_check(&movie)?;
            println!("Encoded and decoded H.264 MP4: {}", movie.display());
            Ok(())
        }
        Command::SckProbe { watch } => {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(watch.unwrap_or(0));
            loop {
                let started = std::time::Instant::now();
                let preflight = crate::capture::engine::runtime::screen_capture_granted();
                let result = crate::capture::engine::runtime::screen_capture_probe();
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
            narration,
            no_audio,
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
                audio: match (narration, no_audio) {
                    (Some(script), false) => export::AudioPolicy::Narration(script),
                    (None, true) => export::AudioPolicy::Silent,
                    _ => export::AudioPolicy::Recorded,
                },
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
                if path.join("apparent.json").is_file() {
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
        Command::Copy { bundle } => {
            let dir = handoff::resolve_bundle_dir(&bundle)?;
            handoff::copy_bundle_to_clipboard(&dir)?;
            println!("{}", copy_confirmation(&dir));
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
        Command::Skill => {
            print!("{SKILL}");
            Ok(())
        }
    }
}

#[cfg(test)]
pub(crate) fn hosted_connect_alias_command_subprocess() -> Result<()> {
    use clap::Parser;
    let Some(name) = std::env::var_os("APPARENT_TEST_CONNECT_ALIAS") else {
        return Ok(());
    };
    let cli = Cli::try_parse_from(["apparent", name.to_str().unwrap(), "disable"])?;
    let Some(Command::Connect { command }) = cli.command else {
        panic!("not a hosted command");
    };
    hosted::command(command)
}

fn copy_confirmation(dir: &std::path::Path) -> String {
    format!(
        "Copied {} to clipboard. Copy is text-only by default; it cannot confirm an image attachment. Use Apparent's Paste control for image + text, or attach {} manually; check the draft before sending.",
        dir.display(),
        dir.join("contact-sheet.png").display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn hosted_connect_is_canonical_in_help_and_usage() {
        let help = Cli::try_parse_from(["apparent", "--help"]).err().unwrap().to_string();
        assert!(help.lines().any(|line| line.trim_start().starts_with("connect ")), "{help}");
        assert!(!help.lines().any(|line| line.trim_start().starts_with("cloud ")), "{help}");
        for name in ["connect", "cloud"] {
            for tail in [vec!["--help"], vec!["login", "--help"], vec!["upload", "--help"]] {
                let mut args = vec!["apparent", name];
                args.extend(tail);
                let error = Cli::try_parse_from(args).err().unwrap();
                assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
                assert!(!error.use_stderr());
                let text = error.to_string();
                assert!(text.contains("apparent connect"), "{text}");
                assert!(!text.contains("apparent cloud"), "{text}");
            }
            let error = Cli::try_parse_from(["apparent", name, "login"]).err().unwrap().to_string();
            assert!(error.contains("apparent connect login --server <SERVER>"), "{error}");
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn hosted_connect_and_cloud_parse_identical_commands_and_flags() {
        use clap::CommandFactory;
        for tail in [
            vec!["login", "--server", "https://hosted.tryapparent.com"],
            vec!["login", "--server", "https://hosted.tryapparent.com", "--scope", "read", "--no-browser"],
            vec!["login", "--server", "https://hosted.tryapparent.com", "--scope", "upload", "--consent-upload", "--no-browser"],
            vec!["status"], vec!["status", "--account"], vec!["logout"],
            vec!["disable"], vec!["retry"], vec!["upload", "./synthetic capture.apparent"],
            vec!["shares", "list"], vec!["shares", "revoke", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
        ] {
            let parse = |name| {
                let mut args = vec!["apparent", name];
                args.extend(tail.iter().copied());
                assert!(matches!(Cli::try_parse_from(&args).unwrap().command, Some(Command::Connect { .. })));
                let matches = Cli::command().try_get_matches_from(args).unwrap();
                assert_eq!(matches.subcommand_name(), Some("connect"));
                matches.subcommand().unwrap().1.clone()
            };
            assert_eq!(parse("connect"), parse("cloud"), "{tail:?}");
        }
        for name in ["connect", "cloud"] {
            for tail in [vec!["login"], vec!["login", "--server", "https://hosted.tryapparent.com", "--scope", "admin"], vec!["status", "--consent-upload"], vec!["upload"]] {
                let mut args = vec!["apparent", name];
                args.extend(tail);
                assert!(Cli::try_parse_from(args).is_err());
            }
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn capture_owned_start_and_wait_parser() {
        for args in [
            vec!["apparent", "capture", "start", "--wait", "--timeout", "30"],
            vec!["apparent", "capture", "stop", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "--wait", "--timeout", "60"],
            vec!["apparent", "capture", "inspect", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
        ] {
            assert!(Cli::try_parse_from(&args).is_ok(), "{args:?}");
        }
        for timeout in ["0", "3601", "bad"] {
            assert!(Cli::try_parse_from(["apparent", "capture", "start", "--timeout", timeout]).is_err());
            assert!(Cli::try_parse_from(["apparent", "capture", "stop", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "--timeout", timeout]).is_err());
        }
        for command in ["start", "stop", "inspect"] {
            let error = Cli::try_parse_from(["apparent", "capture", command, "--help"]).err().unwrap();
            assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
            assert!(!error.use_stderr());
        }
    }

    #[test]
    fn copy_confirmation_does_not_imply_visuals_reached_the_agent() {
        let confirmation = copy_confirmation(std::path::Path::new("/captures/example.apparent"));
        assert!(confirmation.contains("contact-sheet.png"), "{confirmation}");
        assert!(confirmation.contains("Paste"), "{confirmation}");
        assert!(confirmation.contains("check the draft"), "{confirmation}");
    }

    #[test]
    fn cli_identifies_as_apparent() {
        assert_eq!(<Cli as clap::CommandFactory>::command().get_name(), "apparent");
    }

    #[test]
    fn export_cli_requires_explicit_audio_choices_to_be_unambiguous() {
        for args in [
            vec!["apparent", "export", "capture.apparent", "--no-audio"],
            vec!["apparent", "export", "capture.apparent", "--narration", "cues.json"],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }
        for args in [
            vec!["apparent", "export", "--narration"],
            vec!["apparent", "export", "--no-audio", "--narration", "cues.json"],
            vec!["apparent", "export", "--no-audio=false"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn export_cli_zooms_by_default_and_no_zoom_opts_out() {
        for (args, expected) in [
            (vec!["apparent", "export", "capture.apparent"], true),
            (vec!["apparent", "export", "capture.apparent", "--no-zoom"], false),
        ] {
            let cli = Cli::try_parse_from(args).unwrap();
            let Some(Command::Export { zoom, crop, .. }) = cli.command else {
                panic!("expected export command");
            };
            assert_eq!(zoom, expected);
            assert_eq!(crop, export::crop::CropArg::Full);
        }
        assert!(Cli::try_parse_from(["apparent", "export", "capture.apparent", "--zoom"]).is_err());
        use clap::CommandFactory;
        let mut command = Cli::command();
        let help = command.find_subcommand_mut("export").unwrap().render_long_help().to_string();
        assert!(help.contains("--no-zoom"));
        assert!(!help.contains("--zoom"), "retired flag must not appear in help: {help}");
    }

    #[test]
    fn timed_record_help_discloses_missing_interaction_telemetry() {
        use clap::CommandFactory;
        let mut command = Cli::command();
        let mut help = Vec::new();
        command.find_subcommand_mut("record").unwrap().write_long_help(&mut help).unwrap();
        let help = String::from_utf8(help).unwrap();
        assert!(help.contains("does not collect clicks or Shift ink"), "{help}");
        assert!(help.contains("app's recording shortcut"), "{help}");
    }

}
