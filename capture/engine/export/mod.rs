//! `apparent export`: a bundle's recording as one shareable `.mp4` (H.264 or HEVC, AAC audio).
//!
//! Rust decides everything about the picture and the helper (`capture/engine/native/Export.swift`) renders it:
//!
//! - `crop`: the part of the recording kept (`--crop`, default the whole recording);
//! - `camera`: unless `--no-zoom`, a camera rectangle per frame from the bundle's events, so the
//!   helper only interpolates and draws;
//! - `ink`: the bundle's strokes, pins, laser and click ripples as a time-stamped vector plan,
//!   drawn in the recording's own clock;
//! - audio: `audio.wav` (or an iPhone movie's own track) trimmed to start where the video does.
//!
//! The helper reads the video with AVAssetReader, composites with Core Image, draws the
//! annotations with Core Graphics into the same pixel buffers, and hardware-encodes with
//! AVAssetWriter. Nothing is decoded into Rust.

pub mod camera;
pub mod crop;
pub mod ink;
mod narration;
mod quiet;
#[cfg(target_os = "linux")]
mod linux;

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::capture::engine::autocrop::autocrop::Rect;
use crate::shared::model::{ApparentBundle, InteractionEvent};

use self::camera::{CameraKey, Focus};
use self::crop::CropArg;

#[derive(Clone, Debug, Default)]
pub enum AudioPolicy {
    #[default]
    Recorded,
    Silent,
    Narration(PathBuf),
    /// Preserve the existing menu contract; it has no script/silence chooser yet.
    RecordedIfAvailable,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub out: Option<PathBuf>,
    pub crop: CropArg,
    /// Long edge cap in pixels.
    pub size: u32,
    pub zoom: bool,
    /// CLI quiet-hold compression; menu callers retain their existing timing.
    pub speedup: bool,
    pub ink: bool,
    pub clicks: bool,
    pub hevc: bool,
    pub fps: Option<u32>,
    /// Run the encode at utility priority (the app, so a recording in progress isn't hurt).
    pub background: bool,
    pub audio: AudioPolicy,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            out: None,
            crop: CropArg::Full,
            size: crop::DEFAULT_LONG_EDGE,
            zoom: true,
            speedup: false,
            ink: true,
            clicks: true,
            hevc: false,
            fps: None,
            background: false,
            audio: AudioPolicy::Recorded,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Exported {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub duration_ms: u64,
    pub crop: Rect,
    pub audio: bool,
    pub camera_moves: bool,
    pub bytes: u64,
}

/// Where `~/Movies/Apparent/<bundle>.mp4` goes for a bundle.
pub fn default_output(bundle_dir: &Path) -> PathBuf {
    let name = bundle_dir
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("apparent");
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Movies/Apparent")
        .join(format!("{name}.mp4"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Plan<'a> {
    video: &'a Path,
    out: &'a Path,
    audio: Option<&'a Path>,
    audio_start_ms: u64,
    audio_length_ms: u64,
    width: u32,
    height: u32,
    fps: u32,
    codec: &'a str,
    duration_ms: u64,
    crop: Rect,
    camera: Option<Vec<[f64; 5]>>,
    ink: &'a ink::InkPlan,
}

/// What the helper's `export` reports when it is done.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Done {
    duration_ms: u64,
    bytes: u64,
}

/// Camera focuses for a bundle, on the video's clock: clicks, drags, drawn marks and laser
/// points. Activity before the video began is dropped.
pub fn focuses(bundle: &ApparentBundle) -> Vec<Focus> {
    let offset = bundle.video_offset_ms;
    let mut out = Vec::new();
    let mut push = |t0: u64, t1: u64, rect: Rect| {
        if t1 >= offset {
            out.push(Focus {
                t0: t0.saturating_sub(offset),
                t1: t1.saturating_sub(offset),
                rect,
            });
        }
    };
    let bbox = |points: &mut dyn Iterator<Item = [f32; 2]>| -> Option<Rect> {
        points.fold(None, |acc: Option<Rect>, p| {
            Some(acc.map_or([p[0], p[1], p[0], p[1]], |r| {
                [
                    r[0].min(p[0]),
                    r[1].min(p[1]),
                    r[2].max(p[0]),
                    r[3].max(p[1]),
                ]
            }))
        })
    };
    for event in &bundle.events {
        match event {
            InteractionEvent::Click { t, x, y, .. } | InteractionEvent::Point { t, x, y, .. } => {
                push(*t, *t, [*x, *y, *x, *y]);
            }
            InteractionEvent::Drag {
                t,
                t1,
                x0,
                y0,
                x1,
                y1,
                path,
                ..
            } => {
                let mut points = [[*x0, *y0], [*x1, *y1]]
                    .into_iter()
                    .chain(path.iter().copied());
                if let Some(rect) = bbox(&mut points) {
                    push(*t, *t1, rect);
                }
            }
            _ => {}
        }
    }
    for stroke in &bundle.strokes {
        let mut points = stroke.normalized_points().into_iter();
        if let Some(rect) = bbox(&mut points) {
            push(stroke.t0, stroke.t1, rect);
        }
    }
    out
}

/// The camera path for `bundle` inside `crop`, as `[t, x0, y0, x1, y1]` rows.
pub fn camera_rows(
    bundle: &ApparentBundle,
    crop: Rect,
    video_px: (u32, u32),
    duration_ms: u64,
) -> Vec<CameraKey> {
    let offset = bundle.video_offset_ms;
    let focuses = focuses(bundle);
    let cursor: Vec<(u64, f32, f32)> = bundle
        .cursor
        .iter()
        .filter(|c| c.0 >= offset)
        .map(|c| (c.0 - offset, c.1, c.2))
        .collect();
    camera::plan(&camera::Input {
        duration_ms,
        crop,
        aspect: video_px.0 as f32 / video_px.1.max(1) as f32,
        focuses: &focuses,
        cursor: &cursor,
    })
}

/// Exports `bundle` (in `dir`), reporting progress `0.0..=1.0`.
pub fn export(
    dir: &Path,
    bundle: &ApparentBundle,
    options: &Options,
    progress: &mut dyn FnMut(f32),
) -> Result<Exported> {
    let video = match &bundle.video {
        Some(name) => dir.join(name),
        None => dir.join("recording.mp4"),
    };
    if !video.is_file() {
        bail!(
            "{}",
            crate::capture::engine::bundle::retention::missing_media_reason(dir)
                .unwrap_or_else(|| format!("This bundle has no video ({})", video.display()))
        );
    }
    #[cfg(target_os = "linux")]
    let probe = linux::probe(&video).context("Could not read the recording's video")?;
    #[cfg(not(target_os = "linux"))]
    let probe = crate::capture::desktop::helper::probe(&video).context("Could not read the recording's video")?;
    #[cfg(target_os = "macos")]
    let probe = crate::capture::desktop::helper::Probe {
        duration_ms: probe.video_duration_ms.unwrap_or(probe.duration_ms),
        ..probe
    };
    ensure!(
        probe.width > 0 && probe.height > 0 && probe.duration_ms > 0,
        "The recording has no video"
    );
    let video_px = (probe.width, probe.height);

    let crop = crop::resolve(options.crop, bundle, video_px)?;
    let crop_px = (
        (crop[2] - crop[0]) * video_px.0 as f32,
        (crop[3] - crop[1]) * video_px.1 as f32,
    );
    let (width, height) = crop::output_size(crop_px, options.size)?;
    let nominal = if probe.fps > 0.0 { probe.fps } else { 30.0 };
    let fps = match options.fps {
        Some(fps) => crop::validate_fps(fps)?,
        None if nominal >= 45.0 => 60,
        None => 30,
    };

    let out = options.out.clone().unwrap_or_else(|| default_output(dir));
    if let AudioPolicy::Narration(script) = &options.audio {
        if out.exists() {
            ensure!(
                std::fs::canonicalize(&out)? != std::fs::canonicalize(script)
                    .with_context(|| format!("[NARRATION_SCRIPT] Read {}", script.display()))?,
                "Export output cannot replace narration script"
            );
        }
    }

    // The audio: an iPhone movie's own track (full quality, on the movie's clock), else
    // `audio.wav`, which Apparent records from the bundle's start, `video_offset_ms` before the
    // video's.
    let wav = bundle
        .audio
        .as_ref()
        .map(|name| dir.join(name))
        .filter(|p| p.is_file());
    let generated;
    let embedded_audio_usable = probe.audio_tracks > 0;
    #[cfg(target_os = "macos")]
    let embedded_audio_usable = embedded_audio_usable && probe.audio_overlaps_video;
    let (audio, audio_start, audio_length) = if let AudioPolicy::Narration(script) = &options.audio {
        generated = Some(narration::render(script, probe.duration_ms)?);
        (Some(generated.as_ref().unwrap().path()), 0, probe.duration_ms)
    } else if matches!(options.audio, AudioPolicy::Silent) {
        generated = None;
        (None, 0, 0)
    } else {
        generated = None;
        if bundle.source.as_deref() == Some("iphone") && embedded_audio_usable {
            (Some(video.clone()), 0, probe.duration_ms)
        } else if let Some(wav) = wav.as_ref() {
            let wav_ms = wav_duration_ms(wav)?;
            match crop::audio_window(bundle.video_offset_ms, probe.duration_ms, wav_ms) {
                Some((start, length)) => (Some(wav.clone()), start, length),
                None => (None, 0, 0),
            }
        } else {
            (None, 0, 0)
        }
    };
    if audio.is_none() && matches!(options.audio, AudioPolicy::Recorded) {
        bail!("[AUDIO_MISSING] No usable recorded audio overlaps the video in bundle {}. No export was written. Rerun this resolved bundle with --narration <script> to generate local speech, or --no-audio to deliberately export silent video. Source files are unchanged.", std::fs::canonicalize(dir)?.display());
    }

    let camera = options
        .zoom
        .then(|| camera_rows(bundle, crop, video_px, probe.duration_ms))
        .filter(|keys| !keys.iter().all(|k| k.rect == crop));
    let layers = ink::Layers {
        ink: options.ink,
        clicks: options.clicks,
    };
    let ink = ink::plan(bundle, video_px, layers);

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Could not create {}", parent.display()))?;
    }
    // A lexical comparison misses `--out bundle/./recording.mp4`, symlinked
    // directories and other aliases. Keep the original video/WAV untouched.
    let parent = std::fs::canonicalize(out.parent().unwrap_or_else(|| Path::new(".")))?;
    // An export inside a bundle must not replace its manifest, frames or other
    // existing evidence either. A new movie path there is still allowed.
    ensure!(
        !(out.exists() && parent.starts_with(std::fs::canonicalize(dir)?)),
        "Export output cannot replace an existing bundle file"
    );
    let destination = if out.exists() {
        std::fs::canonicalize(&out)?
    } else {
        parent.join(out.file_name().context("Export output needs a file name")?)
    };
    ensure!(
        destination != std::fs::canonicalize(&video)?,
        "Export output cannot replace source media"
    );
    if let Some(source) = &wav {
        ensure!(
            destination != std::fs::canonicalize(source)?,
            "Export output cannot replace source media"
        );
    }
    let partial = out.with_file_name(format!(
        ".{}.{}.partial.mp4",
        out.file_name().unwrap_or_default().to_string_lossy(),
        crate::shared::ids::make_bundle_id()
    ));
    let plan = Plan {
        video: &video,
        out: &partial,
        audio: audio.as_deref(),
        audio_start_ms: audio_start,
        audio_length_ms: audio_length,
        width,
        height,
        fps,
        codec: if options.hevc { "hevc" } else { "h264" },
        duration_ms: probe.duration_ms,
        crop,
        camera: camera.as_ref().map(|keys| {
            keys.iter()
                .map(|k| {
                    [
                        (k.t_ms * 10.0).round() / 10.0,
                        k.rect[0] as f64,
                        k.rect[1] as f64,
                        k.rect[2] as f64,
                        k.rect[3] as f64,
                    ]
                })
                .collect()
        }),
        ink: &ink,
    };
    #[cfg(target_os = "linux")]
    let result = linux::run(&plan, progress);
    #[cfg(not(target_os = "linux"))]
    let result = {
        let plan_path =
            std::env::temp_dir().join(format!("apparent-export-{}.json", crate::shared::ids::make_bundle_id()));
        std::fs::write(&plan_path, serde_json::to_vec(&plan)?)?;
        let result = run_helper(&plan_path, options.background, progress);
        let _ = std::fs::remove_file(&plan_path);
        result
    };
    let result = result.and_then(|done| {
        if !options.speedup { return Ok(done); }
        let quiet = quiet::analyze(&video, audio.as_ref().map(|_| partial.as_path()), 0, probe.duration_ms, bundle)?;
        if quiet.is_empty() { return Ok(done); }
        quiet::retime(&partial, &quiet, probe.duration_ms, audio.is_some(), options.hevc, fps)
    });
    let done = match result {
        Ok(done) => done,
        Err(error) => {
            let _ = std::fs::remove_file(&partial);
            return Err(error);
        }
    };
    if let Err(error) = std::fs::rename(&partial, &out) {
        let _ = std::fs::remove_file(&partial);
        return Err(error).with_context(|| format!("Could not write {}", out.display()));
    }
    progress(1.0);
    drop(generated);
    Ok(Exported {
        path: out,
        width,
        height,
        fps,
        duration_ms: done.duration_ms,
        crop,
        audio: audio.is_some(),
        camera_moves: camera.is_some(),
        bytes: done.bytes,
    })
}

fn wav_duration_ms(path: &Path) -> Result<u64> {
    let reader = hound::WavReader::open(path)
        .with_context(|| format!("Could not read {}", path.display()))?;
    let spec = reader.spec();
    ensure!(
        spec.sample_rate > 0 && spec.channels > 0,
        "Invalid audio file"
    );
    Ok(u64::from(reader.duration()) * 1_000 / u64::from(spec.sample_rate))
}

/// Runs `apparent-helper export PLAN`, reading its JSON lines: `{"progress": f}` and a final
/// `{"done": {...}}`.
#[cfg(not(target_os = "linux"))]
fn run_helper(plan: &Path, background: bool, progress: &mut dyn FnMut(f32)) -> Result<Done> {
    let helper = crate::capture::desktop::helper::path()?;
    let mut command = if background {
        let mut c = Command::new("/usr/sbin/taskpolicy");
        c.args(["-c", "utility"]).arg(&helper);
        c
    } else {
        Command::new(&helper)
    };
    let mut child = command
        .arg("export")
        .arg(plan)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Could not start apparent-helper")?;
    let stdout = child.stdout.take().context("No helper stdout")?;
    let mut stderr = child.stderr.take().context("No helper stderr")?;
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let mut done = None;
    for line in BufReader::new(stdout).lines() {
        let line = line?;
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(fraction) = value.get("progress").and_then(serde_json::Value::as_f64) {
            progress(fraction as f32);
        } else if let Some(summary) = value.get("done") {
            done = serde_json::from_value::<Done>(summary.clone()).ok();
        }
    }
    let status = child.wait()?;
    let message = errors.join().unwrap_or_default();
    if !status.success() {
        bail!("{}", message.trim());
    }
    done.context("apparent-helper finished without a result")
}

/// What `apparent export` prints.
pub fn describe(exported: &Exported) -> String {
    format!(
        "{} · {}×{} @ {} fps · {} · {:.1} MB{}{}",
        exported.path.display(),
        exported.width,
        exported.height,
        exported.fps,
        crate::shared::model::format_clock(exported.duration_ms),
        exported.bytes as f64 / 1_048_576.0,
        if exported.audio {
            " · audio"
        } else {
            " · no audio"
        },
        if exported.camera_moves {
            " · auto-zoom"
        } else {
            ""
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_export_enables_zoom_for_menu_and_other_callers() {
        let options = Options::default();
        assert_eq!(options.crop, CropArg::Full, "exports open on the whole recording");
        assert!(
            options.zoom,
            "menu and other default exports must enable auto-zoom"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_exports_a_real_movie_with_annotated_pixels_and_audio() {
        match std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
        {
            Ok(output) if output.status.success() => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("skipped: FFmpeg is not installed");
                return;
            }
            other => panic!("FFmpeg is installed but cannot start: {other:?}"),
        }
        let root =
            std::env::temp_dir().join(format!("apparent-export-test-{}", crate::shared::ids::make_bundle_id()));
        std::fs::create_dir_all(&root).unwrap();
        let movie = root.join("recording.mp4");
        let made = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=white:s=320x180:r=10:d=2",
                "-vf",
                "drawbox=x=156:y=86:w=8:h=8:color=green:t=fill",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(&movie)
            .status()
            .unwrap();
        assert!(made.success());
        let wav = root.join("audio.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&wav, spec).unwrap();
        for i in 0..(48_000 * 2) {
            let sample = if i < 48_000 / 2 {
                0
            } else {
                ((i as f32 / 40.0).sin() * 4000.0) as i16
            };
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
        let original_video = std::fs::read(&movie).unwrap();
        let original_audio = std::fs::read(&wav).unwrap();
        let mut b = bundle();
        b.video = Some("recording.mp4".into());
        b.audio = Some("audio.wav".into());
        b.video_offset_ms = 100;
        b.events = vec![InteractionEvent::Click {
            t: 600,
            x: 0.5,
            y: 0.5,
            button: None,
            label: None,
            role: None,
        }];
        b.strokes = serde_json::from_value(serde_json::json!([
            {"t0": 900, "t1": 900, "fw": 2560, "fh": 1440,
             "pin": 1, "pts": [640, 720, 0]},
            {"t0": 1000, "t1": 1200, "fw": 2560, "fh": 1440,
             "pts": [1900, 500, 0, 2100, 700, 200]}
        ]))
        .unwrap();
        let mut options = Options {
            out: Some(root.join("out.mp4")),
            crop: CropArg::Full,
            fps: Some(10),
            ..Default::default()
        };
        let output = export(&root, &b, &options, &mut |_| {}).unwrap();
        assert!(output.audio);
        assert!(output.bytes > 0);
        assert!(
            output.camera_moves,
            "default exports render the smooth camera path"
        );
        let plain = export(
            &root,
            &b,
            &Options {
                out: Some(root.join("plain.mp4")),
                zoom: false,
                ..options.clone()
            },
            &mut |_| {},
        )
        .unwrap();
        assert!(!plain.camera_moves, "opting out leaves a static camera");
        let frame_at = |path: &Path, time: &str| {
            let decoded = Command::new("ffmpeg")
                .args(["-v", "error", "-ss", time, "-i"])
                .arg(path)
                .args([
                    "-frames:v",
                    "1",
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "rgba",
                    "pipe:1",
                ])
                .output()
                .unwrap();
            assert!(decoded.status.success());
            crate::shared::model::RgbaFrame::new(0, output.width, output.height, decoded.stdout)
        };
        let opening = frame_at(&output.path, "0.0");
        let plain_opening = frame_at(&plain.path, "0.0");
        // Separate lossy encodes need not produce byte-identical decoded colors.
        let opening_error: u64 = opening.rgba.iter().zip(&plain_opening.rgba)
            .map(|(a, b)| a.abs_diff(*b) as u64).sum();
        assert!(opening_error < opening.rgba.len() as u64 / 10,
            "first frame matches the whole view within codec noise: {opening_error}");
        let frame = frame_at(&output.path, "0.5");
        let plain_frame = frame_at(&plain.path, "0.5");
        // The camera can reframe the click away from the original screen centre.
        let blue_pixels = |frame: &crate::shared::model::RgbaFrame| {
            frame
                .rgba
                .chunks_exact(4)
                .filter(|px| px[2] > px[0].saturating_add(40) && px[2] > px[1].saturating_add(20))
                .count()
        };
        assert!(
            blue_pixels(&plain_frame) > 0,
            "static export keeps the click ring"
        );
        assert!(
            blue_pixels(&frame) > 0,
            "opening keeps the click ring before zooming"
        );
        let green_pixels = |frame: &crate::shared::model::RgbaFrame| {
            frame.rgba.chunks_exact(4).filter(|px| {
                px[1] > px[0].saturating_add(40) && px[1] > px[2].saturating_add(40)
            }).count()
        };
        assert!(green_pixels(&plain_frame) > 0, "source marker is visible");
        let frame = frame_at(&output.path, "0.9");
        let plain_frame = frame_at(&plain.path, "0.9");
        assert!(
            green_pixels(&frame) > green_pixels(&plain_frame),
            "default camera enlarges source pixels, not only annotations"
        );
        let late = frame_at(&output.path, "1.1");
        let colored = late
            .rgba
            .chunks_exact(4)
            .filter(|px| px[0] > px[1].saturating_add(40) && px[0] > px[2].saturating_add(40))
            .count();
        assert!(
            colored > 30,
            "pin and stroke are visible after the click has faded"
        );
        let audio_at = |time: &str| {
            Command::new("ffmpeg")
                .args(["-v", "error", "-ss", time, "-i"])
                .arg(&output.path)
                .args([
                    "-t", "0.05", "-map", "0:a:0", "-f", "s16le", "-ac", "1", "-ar", "48000",
                    "pipe:1",
                ])
                .output()
                .unwrap()
                .stdout
                .chunks_exact(2)
                .map(|sample| i16::from_le_bytes([sample[0], sample[1]]).unsigned_abs() as u64)
                .sum::<u64>()
        };
        assert!(
            audio_at("0.2") < audio_at("0.8") / 5,
            "audio follows the video offset"
        );
        assert_eq!(std::fs::read(&movie).unwrap(), original_video);
        assert_eq!(std::fs::read(&wav).unwrap(), original_audio);
        let silent = export(&root, &b, &Options {
            out: Some(root.join("silent.mp4")),
            audio: AudioPolicy::Silent,
            ..options.clone()
        }, &mut |_| {}).unwrap();
        assert!(!silent.audio);
        assert_eq!(linux::probe(&silent.path).unwrap().audio_tracks, 0,
            "explicit silence removes an existing microphone track");
        options.out = Some(root.join("./recording.mp4"));
        assert!(export(&root, &b, &options, &mut |_| {}).is_err());
        options.out = Some(root.join("./audio.wav"));
        assert!(export(&root, &b, &options, &mut |_| {}).is_err());
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        options.out = Some(alias.join("recording.mp4"));
        assert!(export(&root, &b, &options, &mut |_| {}).is_err());
        b.video_offset_ms = 3_000; // WAV exists, but ends before the movie begins.
        options.out = Some(root.join("no-overlap/out.mp4"));
        let error = export(&root, &b, &options, &mut |_| {}).unwrap_err();
        assert!(format!("{error:#}").contains("[AUDIO_MISSING]"));
        assert!(!root.join("no-overlap").exists());
        options.out = Some(root.join("./audio.wav"));
        assert!(export(&root, &b, &options, &mut |_| {}).is_err());
        let manifest = root.join("apparent.json");
        std::fs::write(&manifest, "private bundle manifest").unwrap();
        options.out = Some(manifest.clone());
        assert!(export(&root, &b, &options, &mut |_| {}).is_err());
        assert_eq!(
            std::fs::read_to_string(manifest).unwrap(),
            "private bundle manifest"
        );
        assert_eq!(std::fs::read(&movie).unwrap(), original_video);
        assert_eq!(std::fs::read(&wav).unwrap(), original_audio);
        options.out = Some(root.join("missing.mp4"));
        std::fs::remove_file(&movie).unwrap();
        assert!(export(&root, &b, &options, &mut |_| {}).is_err());
        assert!(!options.out.as_ref().unwrap().exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_exports_the_video_timeline_not_a_longer_container_audio_tail() {
        use std::process::Command;
        if !Command::new("ffmpeg")
            .arg("-version")
            .output()
            .unwrap()
            .status
            .success()
        {
            return;
        }
        let root =
            std::env::temp_dir().join(format!("apparent-export-tail-{}", crate::shared::ids::make_bundle_id()));
        std::fs::create_dir_all(&root).unwrap();
        let movie = root.join("recording.mp4");
        let encoded = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=white:s=320x180:r=30:d=0.266667",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
                "-c:v",
                "libx264",
                "-c:a",
                "aac",
                "-y",
            ])
            .arg(&movie)
            .output()
            .unwrap();
        assert!(
            encoded.status.success(),
            "{}",
            String::from_utf8_lossy(&encoded.stderr)
        );
        let mut b = bundle();
        b.video = Some("recording.mp4".into());
        b.audio = None;
        b.events.clear();
        b.strokes.clear();
        let missing_out = root.join("not-created/out.mp4");
        let missing = export(&root, &b, &Options {
            out: Some(missing_out.clone()), ..Default::default()
        }, &mut |_| {});
        assert!(missing.is_err(), "missing recorded audio must not silently export");
        let message = format!("{:#}", missing.unwrap_err());
        assert!(message.contains("--narration") && message.contains("--no-audio"), "{message}");
        assert!(message.contains(&root.display().to_string()), "{message}");
        assert!(!missing_out.parent().unwrap().exists());
        let result = export(
            &root,
            &b,
            &Options {
                out: Some(root.join("out.mp4")),
                crop: CropArg::Full,
                audio: AudioPolicy::Silent,
                ..Default::default()
            },
            &mut |_| {},
        )
        .unwrap();
        assert!((result.duration_ms as i64 - 267).abs() < 100);
        assert!(!result.audio);
        // Saved iPhone movies prefer their own track; a disjoint track is not usable.
        b.source = Some("iphone".into());
        let phone = export(&root, &b, &Options {
            out: Some(root.join("phone.mp4")), ..Default::default()
        }, &mut |_| {}).unwrap();
        assert!(phone.audio);
        let delayed = root.join("delayed.mp4");
        let status = Command::new("ffmpeg").args([
            "-v", "error", "-f", "lavfi", "-i", "color=s=320x180:r=30:d=0.3",
            "-itsoffset", "2", "-f", "lavfi", "-i", "sine=frequency=440:duration=0.2",
            "-c:v", "libx264", "-c:a", "aac", "-y",
        ]).arg(&delayed).status().unwrap();
        assert!(status.success());
        b.video = Some("delayed.mp4".into());
        let error = export(&root, &b, &Options {
            out: Some(root.join("disjoint/out.mp4")), ..Default::default()
        }, &mut |_| {}).unwrap_err();
        assert!(format!("{error:#}").contains("[AUDIO_MISSING]"), "{error:#}");
        assert!(!root.join("disjoint").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn quiet_speedup_is_a_copy_and_preserves_explicit_realtime_and_marked_evidence() {
        let root = std::env::temp_dir().join(format!("apparent-paced-{}",crate::shared::ids::make_bundle_id()));
        std::fs::create_dir_all(&root).unwrap();
        let movie = root.join("recording.mp4");
        let status = Command::new("ffmpeg").args(["-v","error","-f","lavfi","-i",
            "color=c=blue:s=160x90:r=10:d=40","-c:v","libx264","-y"]).arg(&movie).status().unwrap();
        assert!(status.success());
        let source = std::fs::read(&movie).unwrap();
        let mut b = bundle();
        b.video = Some("recording.mp4".into());
        b.audio = None;
        b.video_offset_ms = 0;
        b.duration_ms = 40000;
        b.events.clear(); b.cursor.clear(); b.strokes.clear();
        let options = Options {out:Some(root.join("paced.mp4")), speedup:true, zoom:false,
            audio:AudioPolicy::Silent, fps:Some(10), ..Default::default()};
        let paced = export(&root,&b,&options,&mut |_|{}).unwrap();
        assert!((paced.duration_ms as i64-17500).abs()<250);
        assert!(!paced.audio);
        let plain = export(&root,&b,&Options {out:Some(root.join("plain.mp4")),speedup:false,..options.clone()},&mut |_|{}).unwrap();
        assert!((plain.duration_ms as i64-40000).abs()<250);
        b.cursor = serde_json::from_value(serde_json::json!([[5000,0.1,0.1],[35000,0.2,0.2]])).unwrap();
        let cursor_paced = export(&root,&b,&Options {out:Some(root.join("cursor.mp4")),..options.clone()},&mut |_|{}).unwrap();
        assert!(cursor_paced.duration_ms < 30000,"sparse cursor telemetry preserves idle compression");
        // Audible static screens are meaningful, even with no transcript. Analyze the
        // selected rendered audio, which has already applied the nonzero video offset.
        let wav = root.join("voice.wav");
        let mut writer = hound::WavWriter::create(&wav,hound::WavSpec {channels:1,sample_rate:24000,bits_per_sample:16,sample_format:hound::SampleFormat::Int}).unwrap();
        for i in 0..(24000*41) {writer.write_sample(((i as f32*0.1).sin()*4000.0) as i16).unwrap();}
        writer.finalize().unwrap();
        b.audio=Some("voice.wav".into()); b.video_offset_ms=1000;
        let audible = export(&root,&b,&Options {out:Some(root.join("audible.mp4")),audio:AudioPolicy::Recorded,..options.clone()},&mut |_|{}).unwrap();
        assert!(audible.audio);
        assert!((audible.duration_ms as i64-40000).abs()<250,"audible content cannot be accelerated");
        b.audio=None; b.video_offset_ms=0;
        b.marks = serde_json::from_value(serde_json::json!([{
            "id":"result","strokes":[],"t0":0,"t1":40000,"boxNorm":[0,0,1,1],"shape":"flag","description":"Important result"
        }])).unwrap();
        let marked = export(&root,&b,&Options {out:Some(root.join("marked.mp4")),..options},&mut |_|{}).unwrap();
        assert!((marked.duration_ms as i64-40000).abs()<250);
        assert_eq!(std::fs::read(movie).unwrap(),source);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn bundle() -> ApparentBundle {
        serde_json::from_value(serde_json::json!({
            "version": 1, "id": "t", "createdAtMs": 0, "durationMs": 20_000,
            "target": {"mode": "display", "displayId": 0,
                "frame": {"x": 0.0, "y": 0.0, "width": 2560.0, "height": 1440.0, "scale": 2.0}},
            "transcript": {"source": "test", "text": "", "segments": []},
            "strokes": [{"t0": 4000, "t1": 4300, "fw": 2560, "fh": 1440,
                "pts": [2000, 3000, 0, 2600, 3400, 300]}],
            "marks": [], "keyframes": [], "contactSheet": "c.png", "videoOffsetMs": 100,
            "events": [
                {"kind": "click", "t": 50, "x": 0.1, "y": 0.1},
                {"kind": "click", "t": 2100, "x": 0.5, "y": 0.5},
                {"kind": "drag", "t": 8000, "t1": 8500, "x0": 0.7, "y0": 0.6, "x1": 0.75, "y1": 0.65},
                {"kind": "scroll", "t": 9000, "x": 0.1, "y": 0.1, "dx": 0.0, "dy": 3.0}
            ],
            "cursor": [[50, 0.1, 0.1], [2100, 0.5, 0.5]]
        }))
        .unwrap()
    }

    #[test]
    fn camera_focuses_are_on_the_video_clock_and_skip_what_came_before_it() {
        let f = focuses(&bundle());
        assert_eq!(
            f.len(),
            3,
            "click, drag, stroke; the early click and scroll don't count"
        );
        assert_eq!((f[0].t0, f[0].rect), (2_000, [0.5, 0.5, 0.5, 0.5]));
        assert_eq!((f[1].t0, f[1].t1), (7_900, 8_400));
        assert_eq!(f[1].rect, [0.7, 0.6, 0.75, 0.65]);
        assert_eq!(f[2].t0, 3_900);
        assert!((f[2].rect[0] - 0.2).abs() < 1e-6 && (f[2].rect[3] - 0.34).abs() < 1e-6);
    }

    #[test]
    fn the_camera_path_for_a_bundle_zooms_toward_its_activity() {
        let b = bundle();
        let keys = camera_rows(&b, [0.0, 0.0, 1.0, 1.0], (2560, 1440), 15_000);
        assert!(keys.iter().any(|k| k.rect[2] - k.rect[0] < 0.6), "zooms in");
        assert_eq!(keys.last().unwrap().rect, [0.0, 0.0, 1.0, 1.0], "ends wide");
    }

    #[test]
    fn default_output_lands_in_movies_apparent() {
        let path = default_output(Path::new("/x/apparent-17-9.apparent"));
        assert!(
            path.ends_with("Movies/Apparent/apparent-17-9.mp4"),
            "{}",
            path.display()
        );
    }
}
