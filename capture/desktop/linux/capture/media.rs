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

use crate::capture::engine::runtime::{
    CANDIDATE_EDGE, CANDIDATE_EDGE_NO_VIDEO, Sampler, apparent_log,
};
use crate::shared::model::{CaptureMode, RgbaFrame};
use apparent_capture_engine::linux::{
    Recorder, Source, can_record_video, encode_synthetic, init_gstreamer,
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
    let frames = frames(
        movie,
        crate::capture::desktop::linux::helper::FrameTimes::At(&[0]),
        0,
    )?;
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
    let recorder = super::portal::source_for(&target, &stopping).and_then(|source| {
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

pub fn frames(
    movie: &Path,
    times: crate::capture::desktop::linux::helper::FrameTimes,
    max_edge: u32,
) -> Result<Vec<crate::shared::model::RgbaFrame>> {
    let times = match times {
        crate::capture::desktop::linux::helper::FrameTimes::Every(ms) => {
            apparent_capture_engine::linux::media::FrameTimes::Every(ms)
        }
        crate::capture::desktop::linux::helper::FrameTimes::At(times) => {
            apparent_capture_engine::linux::media::FrameTimes::At(times)
        }
    };
    apparent_capture_engine::linux::media::frames(movie, times, max_edge).map(|frames| {
        frames
            .into_iter()
            .map(|f| crate::shared::model::RgbaFrame::new(f.t_ms, f.width, f.height, f.rgba))
            .collect()
    })
}

pub fn probe(movie: &Path) -> Result<crate::capture::desktop::linux::helper::Probe> {
    apparent_capture_engine::linux::media::probe(movie).map(|p| {
        crate::capture::desktop::linux::helper::Probe {
            duration_ms: p.duration_ms,
            width: p.width,
            height: p.height,
            audio_tracks: p.audio_tracks,
            fps: p.fps,
        }
    })
}

/// Retain Desktop's existing WAV writer and error context around Engine PCM extraction.
pub fn export_audio(movie: &Path, wav: &Path) -> Result<()> {
    let (_, samples) = apparent_capture_engine::linux::media::audio_samples(movie)?;
    anyhow::ensure!(!samples.is_empty(), "{} has no audio", movie.display());
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
