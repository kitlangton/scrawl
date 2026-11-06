//! Existing Linux file decoding, probing and audio extraction.
//!
//! This is deliberately a small native execution boundary. Callers retain frame selection,
//! product metadata, WAV encoding and output policy.
use std::path::Path;

use anyhow::{Context, Result, anyhow, ensure};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

use super::recorder::{init_gstreamer, rgba_of};

#[derive(Clone, Copy, Debug)]
pub enum FrameTimes<'a> {
    Every(u64),
    At(&'a [u64]),
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub t_ms: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub audio_tracks: u32,
    pub fps: f64,
}

struct VideoReader {
    pipeline: gst::Pipeline,
    sink: gst_app::AppSink,
    duration_ms: u64,
}

impl VideoReader {
    fn open(movie: &Path, max_edge: u32) -> Result<Self> {
        init_gstreamer()?;
        let caps = if max_edge > 0 {
            "video/x-raw,format=RGBA,pixel-aspect-ratio=1/1"
        } else {
            "video/x-raw,format=RGBA"
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

    /// Return the last frame at or before the requested movie time.
    fn at(&self, ms: u64) -> Result<Frame> {
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
        let (width, height) = (info.width(), info.height());
        let rgba = rgba_of(&sample, width, height).context("Short frame")?;
        let t_ms = sample
            .buffer()
            .and_then(gst::BufferRef::pts)
            .map_or(target, gst::ClockTime::mseconds);
        Ok(Frame {
            t_ms,
            width,
            height,
            rgba,
        })
    }
}

impl Drop for VideoReader {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// Keep the existing box-filter scaling and alpha behavior byte-for-byte.
fn shrink_to_edge(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    max_edge: u32,
) -> (u32, u32, Vec<u8>) {
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

pub fn frames(movie: &Path, times: FrameTimes<'_>, max_edge: u32) -> Result<Vec<Frame>> {
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
    let mut out: Vec<Frame> = Vec::with_capacity(requested.len());
    for want in requested {
        let frame = video.at(want)?;
        let (width, height, rgba) = shrink_to_edge(frame.width, frame.height, frame.rgba, max_edge);
        let t_ms = if sampled { frame.t_ms } else { want };
        if sampled && out.last().is_some_and(|f| f.t_ms == t_ms) {
            continue;
        }
        out.push(Frame {
            t_ms,
            width,
            height,
            rgba,
        });
    }
    ensure!(
        !out.is_empty(),
        "No frames decoded from {}",
        movie.display()
    );
    Ok(out)
}

pub fn probe(movie: &Path) -> Result<Probe> {
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
    Ok(Probe {
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

/// First audio track as 16 kHz mono PCM, including movie-time leading silence.
pub fn audio_samples(movie: &Path) -> Result<(u64, Vec<f32>)> {
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
    let mut samples = Vec::new();
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

#[cfg(test)]
mod tests {
    use super::*;

    struct TempMovie(std::path::PathBuf);
    impl Drop for TempMovie {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
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
    fn reads_real_synthetic_video_frames_in_movie_time_order() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let path = std::env::temp_dir().join(format!(
            "apparent-engine-media-{}-{}.mp4",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _cleanup = TempMovie(path.clone());
        assert!(super::super::recorder::can_record_video(),
            "native media fixture requires GStreamer and a working H.264 encoder");
        super::super::recorder::encode_synthetic(&path).unwrap();
        let probe = probe(&path).unwrap();
        assert_eq!((probe.width, probe.height), (160, 90));
        let frames = frames(&path, FrameTimes::Every(100), 0).unwrap();
        assert!(frames.len() >= 2);
        assert!(frames.windows(2).all(|pair| pair[0].t_ms < pair[1].t_ms));
        assert!(frames.iter().all(|f| (f.width, f.height) == (160, 90)));
        assert!(frames.iter().all(|f| f.rgba.len() == 160 * 90 * 4));
        let colors: std::collections::HashSet<_> = frames[0].rgba.chunks_exact(4)
            .map(|pixel| [pixel[0], pixel[1], pixel[2]]).collect();
        assert!(colors.len() > 4, "decoded SMPTE fixture must retain multiple colors");
        assert!(frames[0].rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
        let requested = [700, 0, 700];
        let selected = super::frames(&path, FrameTimes::At(&requested), 0).unwrap();
        assert_eq!(selected.iter().map(|frame| frame.t_ms).collect::<Vec<_>>(), requested);
        assert_eq!(selected[0].rgba, selected[2].rgba);
    }

    #[test]
    fn audio_samples_preserve_leading_movie_time_silence() {
        use std::process::Command;
        use std::time::{SystemTime, UNIX_EPOCH};
        let version = Command::new("ffmpeg").arg("-version").output()
            .expect("native audio fixture requires FFmpeg with libx264 and AAC encoders");
        assert!(version.status.success(), "FFmpeg fixture generator must be usable");
        let path = std::env::temp_dir().join(format!(
            "apparent-engine-audio-{}-{}.mp4",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _cleanup = TempMovie(path.clone());
        let status = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                r"aevalsrc=if(lt(t\,0.5)\,0\,sin(2*PI*440*t)):s=16000:d=1",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=64x48:r=10:d=2",
                "-map",
                "1:v:0",
                "-map",
                "0:a:0",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-shortest",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "FFmpeg fixture generation requires libx264 and AAC encoder support"
        );
        let (first_ms, samples) = audio_samples(&path).unwrap();
        assert!(
            (50..=100).contains(&first_ms),
            "decoder priming offset {first_ms} ms"
        );
        assert!(samples.len() > 9_000);
        assert!(samples[..7_000].iter().all(|sample| sample.abs() < 0.0001));
        assert!(
            samples
                .get(9_500)
                .is_some_and(|sample| sample.abs() > 0.001)
        );
    }
}
