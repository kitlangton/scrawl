//! Movies on Linux, through GStreamer (on every GNOME and KDE desktop): recording the screen
//! (`ximagesrc` on X11; on Wayland `pipewiresrc` fed by the ScreenCast portal or GNOME's own
//! screencast API, `super::portal`) into an H.264 MP4 while handing small candidate frames to
//! keyframe selection, and reading movies back (keyframes at native size, the audio for
//! transcription).
//!
//! The encoder is the first one installed of `x264enc` (gst-plugins-ugly), `openh264enc`
//! (gst-plugins-bad) and VA-API's. A missing encoder refuses to start a screen capture;
//! a movie that fails after capture starts still retains the candidate frames.

use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

use crate::capture::desktop::linux::helper::FrameTimes;
use crate::capture::engine::runtime::{
    CANDIDATE_EDGE, CANDIDATE_EDGE_NO_VIDEO, Sampler, apparent_log,
};
use crate::shared::model::{CaptureMode, RgbaFrame};
use apparent_capture_engine::linux::{
    Recorder, Source, can_record_video, encode_synthetic, init_gstreamer, rgba_of,
};

fn encoder_repair_for(distro: &str) -> &'static str {
    match distro {
        "arch" | "archarm" | "omarchy" | "manjaro" | "endeavouros" => {
            "sudo pacman -S gst-plugins-ugly"
        }
        "ubuntu" | "debian" | "linuxmint" | "pop" => "sudo apt install gstreamer1.0-plugins-ugly",
        "fedora" => "sudo dnf install gstreamer1-plugins-ugly",
        _ => "Install your distribution's GStreamer H.264 encoder (often gst-plugins-ugly)",
    }
}

fn host_distro() -> String {
    let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    os_release
        .lines()
        .find_map(|line| line.strip_prefix("ID="))
        .unwrap_or_default()
        .trim_matches('"')
        .to_owned()
}

/// Check before starting a screen capture, including the CLI path. Never install packages here.
pub fn encoder_warning() -> Option<String> {
    static CHECK: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    let result = CHECK.get_or_init(|| {
        init_gstreamer().map_err(|error| error.to_string())?;
        if !can_record_video() {
            return Err("No H.264 encoder plugin is installed".into());
        }
        let directory = preflight_directory().map_err(|error| error.to_string())?;
        let movie = directory.join("encoder-check.mp4");
        let result = encode_synthetic(&movie).map_err(|error| error.to_string());
        let _ = std::fs::remove_file(&movie);
        let _ = std::fs::remove_dir(directory);
        result
    });
    result.as_ref().err().map(|reason| {
        encoder_diagnostic(
            reason,
            std::env::var_os("APPIMAGE").is_some(),
            &host_distro(),
        )
    })
}

/// Exclusively create a private directory before GStreamer's filesink opens its output.
fn preflight_directory() -> Result<PathBuf> {
    for attempt in 0..16 {
        let directory = std::env::temp_dir().join(format!(
            "apparent-encoder-check-{}-{}-{attempt}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => return Ok(directory),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    bail!("Could not create a private H.264 encoder-check directory")
}

fn encoder_diagnostic(reason: &str, packaged: bool, distro: &str) -> String {
    if packaged {
        format!(
            "H.264 encoder failed in this Apparent AppImage ({reason}). Install a fresh Apparent AppImage; host GStreamer packages cannot repair its bundled runtime. No recording was started."
        )
    } else {
        format!(
            "H.264 encoder failed ({reason}). Install: {}. Restart Apparent, then retry. No recording was started.",
            encoder_repair_for(distro)
        )
    }
}

/// Exercise the same encoder selection as real capture, then verify the resulting movie decodes.
/// Runs in the packaged process so an installed host plugin cannot mask a missing bundled plugin.
pub fn encoder_check(movie: &Path) -> Result<()> {
    init_gstreamer()?;
    if !can_record_video() {
        bail!("No H.264 encoder plugin is installed");
    }
    encode_synthetic(movie)?;
    let frames = frames(movie, FrameTimes::At(&[0]), 0)?;
    ensure!(
        frames
            .first()
            .is_some_and(|frame| frame.width == 160 && frame.height == 90),
        "H.264 movie cannot be decoded at 160x90"
    );
    Ok(())
}

/// Check this process's registry, not the host's gst-inspect executable.
pub fn h264_decoder_available() -> bool {
    if init_gstreamer().is_err() {
        return false;
    }
    let caps = gst::Caps::builder("video/x-h264").build();
    gst::ElementFactory::factories_with_type(
        gst::ElementFactoryType::DECODER | gst::ElementFactoryType::MEDIA_VIDEO,
        gst::Rank::MARGINAL,
    )
    .iter()
    .any(|factory| factory.can_sink_any_caps(&caps) && factory.create().build().is_ok())
}

fn decoder_repair_for(distro: &str) -> &'static str {
    match distro {
        "arch" | "archarm" | "omarchy" | "manjaro" | "endeavouros" => "sudo pacman -S gst-libav",
        "ubuntu" | "debian" | "linuxmint" | "pop" => "sudo apt install gstreamer1.0-libav",
        "fedora" => "sudo dnf install gstreamer1-plugin-libav",
        _ => "Install your distribution's GStreamer H.264 decoder (often gst-libav)",
    }
}

pub fn decoder_warning() -> Option<String> {
    if h264_decoder_available() {
        return None;
    }
    Some(format!(
        "H.264 decoder missing. Install: {}. Restart Apparent. MP4 recording may work, but full-quality keyframes use smaller candidate frames until fixed.",
        decoder_repair_for(&host_distro())
    ))
}

fn mask(rgba: &mut [u8], width: u32, height: u32, rect: [f64; 4]) {
    let (w, h) = (width as usize, height as usize);
    let x0 = (rect[0].floor().max(0.0) as usize).min(w);
    let y0 = (rect[1].floor().max(0.0) as usize).min(h);
    let x1 = ((rect[0] + rect[2]).ceil().max(0.0) as usize).min(w);
    let y1 = ((rect[1] + rect[3]).ceil().max(0.0) as usize).min(h);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let source = if y1 < h {
        y1
    } else if y0 > 0 {
        y0 - 1
    } else {
        return;
    };
    let row: Vec<u8> = rgba[(source * w + x0) * 4..(source * w + x1) * 4].to_vec();
    for y in y0..y1 {
        rgba[(y * w + x0) * 4..(y * w + x1) * 4].copy_from_slice(&row);
    }
}

/// The recording thread: see `crate::capture::engine::runtime::Sampler`.
pub fn run_sampler(sampler: Sampler) -> Option<PathBuf> {
    let Sampler {
        target,
        video_output_path,
        started_at,
        stopping,
        urgent,
        frames,
        real_frame_seen,
        startup_failed,
        video_offset_ms,
        excluded_window_ids,
    } = sampler;
    let long_edge = if target.mode == CaptureMode::Display {
        2560.0
    } else {
        1280.0
    };
    let out = crate::capture::engine::runtime::capture_size(&target, long_edge);
    let movie = video_output_path.as_deref().filter(|_| can_record_video());
    let edge = if movie.is_some() {
        CANDIDATE_EDGE
    } else {
        CANDIDATE_EDGE_NO_VIDEO
    };
    let candidate = crate::capture::engine::runtime::capture_size(&target, edge);
    let recorder =
        super::portal::source_for(&target, &stopping).and_then(|source| {
            Recorder::new(&source, out, candidate, movie, started_at)
                .map_err(|error| anyhow!("{error:#}"))
                // The source (a portal session) lives as long as the recording.
                .map(|recorder| (recorder, source))
        });
    let mut recorder = match recorder {
        Ok(recorder) => {
            apparent_log(format!(
                "Recording {}x{} ({:?}){}",
                out.0,
                out.1,
                target.mode,
                if recorder.0.has_video() {
                    ""
                } else {
                    ", no movie"
                }
            ));
            Some(recorder)
        }
        Err(error) => {
            apparent_log(format!("Screen capture failed to start: {error:#}"));
            startup_failed.store(true, Ordering::Release);
            None
        }
    };
    crate::capture::desktop::start_metric("sck_ready_ms", started_at);
    let mut last_sample = Instant::now()
        .checked_sub(crate::capture::engine::runtime::FRAME_SAMPLE_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut offset_set = false;
    let records_others = !matches!(
        recorder.as_ref().map(|(_, source)| source),
        Some(Source::X11Window { .. })
    );
    let f = target.frame;
    let mut masks = (Instant::now(), Vec::new());
    let mut masks_read = false;
    while !stopping.load(Ordering::Acquire) {
        if records_others && (!masks_read || masks.0.elapsed() >= Duration::from_millis(500)) {
            let ids = excluded_window_ids
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let [fx, fy, fw, fh] = [f.x, f.y, f.width, f.height];
            let (sx, sy) = (f64::from(candidate.0) / fw, f64::from(candidate.1) / fh);
            masks = (
                Instant::now(),
                ids.iter()
                    .filter_map(|&id| super::super::window_info(id as isize))
                    .filter(|w| w.on_screen() && w.rect[2] * w.rect[3] < fw * fh * 0.5)
                    .map(|w| {
                        let [x, y, ww, wh] = w.rect;
                        [(x - fx) * sx, (y - fy) * sy, ww * sx, wh * sy]
                    })
                    .collect(),
            );
            masks_read = true;
        }
        if let Some((r, _)) = recorder.as_ref() {
            if let Some(error) = r.error() {
                apparent_log(format!("Recording failed: {error}"));
                // Publish before native teardown, which need not end the sampler thread.
                startup_failed.store(true, Ordering::Release);
                if let Some((r, _)) = recorder.take() {
                    r.stop();
                }
            } else if !offset_set {
                if let Some(first) = r.first_frame_ms() {
                    video_offset_ms.store(first, Ordering::Release);
                    offset_set = true;
                }
            }
        }
        let is_urgent = urgent.swap(false, Ordering::AcqRel);
        let interval = frames
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .interval();
        if is_urgent || last_sample.elapsed() >= interval {
            last_sample = Instant::now();
            let t_ms = started_at.elapsed().as_millis() as u64;
            let frame = match recorder.as_mut() {
                // No frame from the compositor yet: look again on the next turn.
                Some((r, _)) => r
                    .candidate(|rgba, width, height| {
                        for &rect in &masks.1 {
                            mask(rgba, width, height, rect);
                        }
                    })
                    .map(|candidate| {
                        RgbaFrame::new(t_ms, candidate.width, candidate.height, candidate.rgba)
                    }),
                None => None,
            };
            match frame {
                Some(frame) => crate::capture::engine::runtime::push_candidate(
                    &frames,
                    &real_frame_seen,
                    frame,
                    is_urgent,
                    true,
                ),
                None if is_urgent => urgent.store(true, Ordering::Release),
                None => {}
            }
        }
        std::thread::park_timeout(Duration::from_millis(8));
    }
    let (recorder, _source) = recorder?;
    if recorder.finish(apparent_log) {
        video_output_path
    } else {
        if let Some(path) = &video_output_path {
            let _ = std::fs::remove_file(path);
        }
        None
    }
}

// MARK: Reading movies

/// A paused decoding pipeline that hands out frames at the times it's sought to.
struct VideoReader {
    pipeline: gst::Pipeline,
    sink: gst_app::AppSink,
    duration_ms: u64,
}

impl VideoReader {
    fn open(movie: &Path, max_edge: u32) -> Result<Self> {
        init_gstreamer()?;
        let caps = if max_edge > 0 {
            // Keep the aspect: scale to fit a max_edge square, never up.
            "video/x-raw,format=RGBA,pixel-aspect-ratio=1/1".to_owned()
        } else {
            "video/x-raw,format=RGBA".to_owned()
        };
        let launch = format!(
            "filesrc location=\"{}\" ! decodebin ! videoconvert ! {caps} ! appsink name=sink sync=false max-buffers=1",
            movie.display().to_string().replace('"', "\\\"")
        );
        let pipeline = gst::parse::launch(&launch)?
            .downcast::<gst::Pipeline>()
            .map_err(|_| anyhow!("Not a pipeline"))?;
        let sink = pipeline
            .by_name("sink")
            .context("No sink")?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| anyhow!("Not an appsink"))?;
        pipeline.set_state(gst::State::Paused)?;
        let (result, _, _) = pipeline.state(gst::ClockTime::from_seconds(10));
        result.with_context(|| format!("Could not open {}", movie.display()))?;
        let duration_ms = pipeline
            .query_duration::<gst::ClockTime>()
            .map_or(0, gst::ClockTime::mseconds);
        Ok(Self {
            pipeline,
            sink,
            duration_ms,
        })
    }

    /// The frame on screen at `ms`: the last one at or before it.
    fn at(&self, ms: u64) -> Result<(u64, u32, u32, Vec<u8>)> {
        let target = ms.min(self.duration_ms.saturating_sub(1));
        self.pipeline.seek_simple(
            gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
            gst::ClockTime::from_mseconds(target),
        )?;
        let _ = self.pipeline.state(gst::ClockTime::from_seconds(10));
        let sample = self
            .sink
            .try_pull_preroll(gst::ClockTime::from_seconds(10))
            .context("No frame decoded")?;
        let caps = sample.caps().context("No caps")?;
        let info = gstreamer_video::VideoInfo::from_caps(caps)?;
        let (w, h) = (info.width(), info.height());
        let rgba = rgba_of(&sample, w, h).context("Short frame")?;
        let t = sample
            .buffer()
            .and_then(gst::BufferRef::pts)
            .map_or(target, gst::ClockTime::mseconds);
        Ok((t, w, h, rgba))
    }
}

impl Drop for VideoReader {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// Box-filters RGBA down so its long edge is at most `max_edge` (0 keeps it).
fn shrink_to_edge(width: u32, height: u32, rgba: Vec<u8>, max_edge: u32) -> (u32, u32, Vec<u8>) {
    let long = width.max(height);
    if max_edge == 0 || long <= max_edge {
        return (width, height, rgba);
    }
    let scale = f64::from(max_edge) / f64::from(long);
    let w = ((f64::from(width) * scale).round() as u32).max(1);
    let h = ((f64::from(height) * scale).round() as u32).max(1);
    let (sw, sh) = (width as usize, height as usize);
    let mut out = vec![0_u8; w as usize * h as usize * 4];
    for y in 0..h as usize {
        let y0 = y * sh / h as usize;
        let y1 = ((y + 1) * sh / h as usize).max(y0 + 1);
        for x in 0..w as usize {
            let x0 = x * sw / w as usize;
            let x1 = ((x + 1) * sw / w as usize).max(x0 + 1);
            let mut acc = [0_u32; 3];
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            for sy in y0..y1 {
                for px in rgba[(sy * sw + x0) * 4..(sy * sw + x1) * 4].chunks_exact(4) {
                    acc[0] += u32::from(px[0]);
                    acc[1] += u32::from(px[1]);
                    acc[2] += u32::from(px[2]);
                }
            }
            out[(y * w as usize + x) * 4..][..4].copy_from_slice(&[
                (acc[0] / n) as u8,
                (acc[1] / n) as u8,
                (acc[2] / n) as u8,
                255,
            ]);
        }
    }
    (w, h, out)
}

pub fn frames(movie: &Path, times: FrameTimes, max_edge: u32) -> Result<Vec<RgbaFrame>> {
    let video = VideoReader::open(movie, max_edge)?;
    let (requested, sampled): (Vec<u64>, bool) = match times {
        FrameTimes::At(times) => {
            if times.is_empty() {
                return Ok(Vec::new());
            }
            (times.to_vec(), false)
        }
        FrameTimes::Every(ms) => (
            (0..video.duration_ms.max(1))
                .step_by(ms.max(1) as usize)
                .collect(),
            true,
        ),
    };
    let mut out: Vec<RgbaFrame> = Vec::with_capacity(requested.len());
    for want in requested {
        let (t, w, h, rgba) = video.at(want)?;
        let (w, h, rgba) = shrink_to_edge(w, h, rgba, max_edge);
        let t = if sampled { t } else { want };
        if sampled && out.last().is_some_and(|f| f.t_ms == t) {
            continue;
        }
        out.push(RgbaFrame::new(t, w, h, rgba));
    }
    ensure!(
        !out.is_empty(),
        "No frames decoded from {}",
        movie.display()
    );
    Ok(out)
}

pub fn probe(movie: &Path) -> Result<crate::capture::desktop::linux::helper::Probe> {
    let video = VideoReader::open(movie, 0)?;
    let caps = video
        .sink
        .static_pad("sink")
        .and_then(|pad| pad.current_caps());
    let info = caps
        .as_ref()
        .and_then(|caps| gstreamer_video::VideoInfo::from_caps(caps).ok());
    let audio_tracks =
        audio_samples(movie).map_or(0, |(_, samples)| u32::from(!samples.is_empty()));
    Ok(crate::capture::desktop::linux::helper::Probe {
        duration_ms: video.duration_ms,
        width: info.as_ref().map_or(0, gstreamer_video::VideoInfo::width),
        height: info.as_ref().map_or(0, gstreamer_video::VideoInfo::height),
        audio_tracks,
        fps: info.as_ref().map_or(0.0, |i| {
            let f = i.fps();
            f64::from(f.numer()) / f64::from(f.denom().max(1))
        }),
    })
}

/// The movie's first audio track, 16 kHz mono, with silence before its first sample so audio
/// time is movie time.
fn audio_samples(movie: &Path) -> Result<(u64, Vec<f32>)> {
    init_gstreamer()?;
    let launch = format!(
        "filesrc location=\"{}\" ! decodebin ! audioconvert ! audioresample ! audio/x-raw,format=F32LE,rate=16000,channels=1,layout=interleaved ! appsink name=sink sync=false",
        movie.display().to_string().replace('"', "\\\"")
    );
    let pipeline = gst::parse::launch(&launch)?
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow!("Not a pipeline"))?;
    let sink = pipeline
        .by_name("sink")
        .context("No sink")?
        .downcast::<gst_app::AppSink>()
        .map_err(|_| anyhow!("Not an appsink"))?;
    pipeline.set_state(gst::State::Playing)?;
    let mut samples: Vec<f32> = Vec::new();
    let mut first_ms = None;
    while let Some(sample) = sink.try_pull_sample(gst::ClockTime::from_seconds(10)) {
        let Some(buffer) = sample.buffer() else {
            continue;
        };
        if first_ms.is_none() {
            let start = buffer.pts().map_or(0, gst::ClockTime::mseconds);
            first_ms = Some(start);
            samples.resize((start * 16) as usize, 0.0);
        }
        let map = buffer.map_readable()?;
        samples.extend(
            map.chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        );
    }
    let _ = pipeline.set_state(gst::State::Null);
    Ok((first_ms.unwrap_or(0), samples))
}

/// Mixes a movie's audio to 16 kHz mono WAV, aligned to the movie's start.
pub fn export_audio(movie: &Path, wav: &Path) -> Result<()> {
    let (_, samples) = audio_samples(movie)?;
    ensure!(!samples.is_empty(), "{} has no audio", movie.display());
    std::fs::write(
        wav,
        crate::capture::engine::runtime::audio::audio::encode_wav_16k(&samples)?,
    )
    .with_context(|| format!("Could not write {}", wav.display()))
}

pub fn shrink(
    _movie: &Path,
    _out: &Path,
    _max_edge: u32,
) -> Result<crate::capture::engine::bundle::retention::Shrunk> {
    bail!("Only cabled-iPhone recordings (macOS) are re-encoded")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_repair_names_the_host_package_without_installing_it() {
        assert_eq!(decoder_repair_for("arch"), "sudo pacman -S gst-libav");
        assert_eq!(decoder_repair_for("omarchy"), "sudo pacman -S gst-libav");
        assert_eq!(
            decoder_repair_for("ubuntu"),
            "sudo apt install gstreamer1.0-libav"
        );
        assert_eq!(
            decoder_repair_for("fedora"),
            "sudo dnf install gstreamer1-plugin-libav"
        );
        assert!(decoder_repair_for("other").contains("distribution"));
    }

    #[test]
    fn encoder_repair_names_the_host_package_without_installing_it() {
        assert_eq!(
            encoder_repair_for("arch"),
            "sudo pacman -S gst-plugins-ugly"
        );
        assert_eq!(
            encoder_repair_for("omarchy"),
            "sudo pacman -S gst-plugins-ugly"
        );
        assert_eq!(
            encoder_repair_for("ubuntu"),
            "sudo apt install gstreamer1.0-plugins-ugly"
        );
        assert_eq!(
            encoder_repair_for("fedora"),
            "sudo dnf install gstreamer1-plugins-ugly"
        );
        assert!(encoder_repair_for("other").contains("distribution"));
        assert!(
            encoder_diagnostic("not found", false, "omarchy")
                .contains("sudo pacman -S gst-plugins-ugly")
        );
        let packaged = encoder_diagnostic("not found", true, "omarchy");
        assert!(packaged.contains("fresh Apparent AppImage"));
        assert!(!packaged.contains("sudo pacman"));
    }

    #[test]
    fn encoder_preflight_uses_a_private_directory() {
        use std::os::unix::fs::PermissionsExt;
        let directory = preflight_directory().unwrap();
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(!directory.join("encoder-check.mp4").exists());
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn frames_shrink_to_their_long_edge() {
        let (w, h, rgba) = shrink_to_edge(4, 2, vec![255; 4 * 2 * 4], 2);
        assert_eq!((w, h), (2, 1));
        assert_eq!(rgba, vec![255; 2 * 4]);
        let (w, h, _) = shrink_to_edge(4, 2, vec![0; 32], 0);
        assert_eq!((w, h), (4, 2));
    }

    #[test]
    fn masks_take_the_row_below() {
        // 3x3, each row its own grey; the pill over the top two rows' middle.
        let mut rgba: Vec<u8> = (0..3).flat_map(|y| [y as u8 * 100; 12]).collect();
        mask(&mut rgba, 3, 3, [1.0, 0.0, 1.0, 2.0]);
        assert_eq!(rgba[4], 200);
        assert_eq!(rgba[16], 200);
        assert_eq!(rgba[0], 0);
        // At the bottom it takes the row above.
        mask(&mut rgba, 3, 3, [0.0, 2.0, 1.0, 1.0]);
        assert_eq!(rgba[24], 100);
    }
}
