//! Linux export adapter: ffmpeg supplies constant-rate RGBA frames and encodes the final MP4.
//! The crop, camera and annotation timeline are the shared Rust export plan.
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result, bail, ensure};

use super::{Done, Plan};
use crate::shared::model::RgbaFrame;

/// Export uses FFmpeg, not the GStreamer keyframe decoder. A capture with
/// candidate-frame fallback can still have an intact, exportable movie.
pub(super) fn probe(movie: &Path) -> Result<crate::capture::desktop::helper::Probe> {
    Ok(probe_with_audio(movie)?.0)
}

fn probe_with_audio(movie: &Path) -> Result<(crate::capture::desktop::helper::Probe, Option<usize>)> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=codec_type,width,height,avg_frame_rate,duration,start_time",
            "-of",
            "json",
        ])
        .arg(movie)
        .output()
        .context("Linux video export needs ffprobe on PATH")?;
    ensure!(
        output.status.success(),
        "ffprobe could not read the movie: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("Invalid ffprobe result")?;
    let streams = value["streams"]
        .as_array()
        .context("ffprobe returned no streams")?;
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .context("No video track")?;
    // Container duration can include a longer audio tail. The video timeline
    // determines how many RGBA frames the decoder can actually provide.
    let duration = video["duration"]
        .as_str()
        .or_else(|| value["format"]["duration"].as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .context("Unknown video duration")?;
    ensure!(
        duration.is_finite() && duration > 0.0 && duration < u64::MAX as f64 / 1000.0,
        "Invalid video duration"
    );
    let rate = video["avg_frame_rate"].as_str().unwrap_or("0/1");
    let (num, den) = rate.split_once('/').unwrap_or((rate, "1"));
    let fps = num.parse::<f64>().unwrap_or(0.0) / den.parse::<f64>().unwrap_or(1.0);
    let usable_audio: Vec<usize> = streams
        .iter()
        .enumerate()
        .filter_map(|(index, s)| {
            let start = s["start_time"]
                .as_str()
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0);
            let length = s["duration"]
                .as_str()
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0);
            (s["codec_type"] == "audio"
                && start.is_finite()
                && length.is_finite()
                && length > 0.0
                && start < duration
                && start + length > 0.0)
                .then_some(index)
        })
        .collect();
    Ok((
        crate::capture::desktop::helper::Probe {
            duration_ms: (duration * 1000.0).round() as u64,
            width: video["width"].as_u64().unwrap_or(0) as u32,
            height: video["height"].as_u64().unwrap_or(0) as u32,
            audio_tracks: usable_audio.len() as u32,
            fps: if fps.is_finite() { fps } else { 0.0 },
        },
        usable_audio.first().copied(),
    ))
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn drain_stderr(child: &mut Child) -> std::thread::JoinHandle<String> {
    let mut stderr = child.stderr.take().expect("ffmpeg stderr is piped");
    std::thread::spawn(move || {
        let mut log = Vec::new();
        let mut buffer = [0; 4096];
        while let Ok(n) = stderr.read(&mut buffer) {
            if n == 0 {
                break;
            }
            log.extend_from_slice(&buffer[..n.min(16_384usize.saturating_sub(log.len()))]);
        }
        String::from_utf8_lossy(&log).into_owned()
    })
}

pub(super) fn run(plan: &Plan<'_>, progress: &mut dyn FnMut(f32)) -> Result<Done> {
    let (probe, embedded_audio) = probe_with_audio(plan.video)?;
    let source_bytes = (probe.width as usize)
        .checked_mul(probe.height as usize)
        .and_then(|n| n.checked_mul(4))
        .context("The recording is too large to decode")?;
    ensure!(
        source_bytes > 0 && source_bytes <= 256 * 1024 * 1024,
        "The recording is too large to export"
    );
    // Rounded millisecond metadata can slightly exceed an exact frame boundary.
    // Ceil would request frame 609 from a 608-frame, 30 fps recording.
    let frame_count =
        (((plan.duration_ms as u128 * plan.fps as u128 + 500) / 1000) as usize).max(1);
    let mut decoder = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(plan.video)
        .args([
            "-vf",
            &format!("fps={}", plan.fps),
            "-frames:v",
            &frame_count.to_string(),
            "-an",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-threads",
            "1",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Linux video export needs ffmpeg on PATH")?;
    let stderr = drain_stderr(&mut decoder);
    let result = encode(
        plan,
        probe.width,
        probe.height,
        frame_count,
        source_bytes,
        embedded_audio,
        &mut decoder,
        progress,
    );
    if result.is_err() {
        stop(&mut decoder);
    }
    let log = stderr.join().unwrap_or_default();
    result.with_context(|| format!("ffmpeg decoder: {log}"))
}

fn encode(
    plan: &Plan<'_>,
    sw: u32,
    sh: u32,
    count: usize,
    source_bytes: usize,
    embedded_audio: Option<usize>,
    decoder: &mut Child,
    progress: &mut dyn FnMut(f32),
) -> Result<Done> {
    let mut command = Command::new("ffmpeg");
    command
        .args([
            "-v", "error", "-nostdin", "-f", "rawvideo", "-pix_fmt", "rgba", "-s",
        ])
        .arg(format!("{}x{}", plan.width, plan.height))
        .args(["-r", &plan.fps.to_string(), "-i", "pipe:0"]);
    if let Some(audio) = plan.audio {
        command
            .args([
                "-ss",
                &format!("{:.3}", plan.audio_start_ms as f64 / 1000.0),
                "-i",
            ])
            .arg(audio);
    }
    command.args(["-map", "0:v:0"]);
    if plan.audio.is_some() {
        let map = if plan.audio == Some(plan.video) {
            format!(
                "1:{}",
                embedded_audio.context("Embedded audio no longer overlaps the video")?
            )
        } else {
            "1:a:0".into()
        };
        command.args([
            "-map",
            &map,
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-af",
            "aresample=async=1:first_pts=0",
        ]);
    }
    command
        .args([
            "-c:v",
            if plan.codec == "hevc" {
                "libx265"
            } else {
                "libx264"
            },
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            "-t",
        ])
        .arg(format!("{:.3}", plan.duration_ms as f64 / 1000.0))
        .args(["-y", "-f", "mp4"])
        .arg(plan.out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut encoder = command
        .spawn()
        .context("Could not start ffmpeg video encoder")?;
    let stderr = drain_stderr(&mut encoder);
    let result = write_frames(
        plan,
        sw,
        sh,
        count,
        source_bytes,
        decoder,
        &mut encoder,
        progress,
    );
    if result.is_err() {
        stop(&mut encoder);
    }
    let log = stderr.join().unwrap_or_default();
    result.with_context(|| format!("ffmpeg encoder: {log}"))
}

fn write_frames(
    plan: &Plan<'_>,
    sw: u32,
    sh: u32,
    count: usize,
    source_bytes: usize,
    decoder: &mut Child,
    encoder: &mut Child,
    progress: &mut dyn FnMut(f32),
) -> Result<Done> {
    let mut decoder_out = decoder.stdout.take().context("No decoded video stream")?;
    let mut encoder_in = encoder.stdin.take().context("No encoder input")?;
    let mut rgba = vec![0; source_bytes];
    for i in 0..count {
        decoder_out
            .read_exact(&mut rgba)
            .context("The recording ended before the export timeline")?;
        let ms = i as f64 * 1000.0 / plan.fps as f64;
        let rect = camera_at(plan, ms);
        let mut output = resample(&rgba, sw, sh, plan.width, plan.height, rect);
        crate::capture::engine::render::draw_export_annotations(&mut output, ms, rect, (sw, sh), plan.ink);
        encoder_in
            .write_all(&output.rgba)
            .context("Could not write an export frame")?;
        if i % 30 == 0 {
            progress(i as f32 / count as f32);
        }
    }
    drop(encoder_in);
    drop(decoder_out);
    let decoded = decoder
        .wait()
        .context("Could not finish decoding the recording")?;
    let encoded = encoder
        .wait()
        .context("Could not finish encoding the movie")?;
    if !decoded.success() || !encoded.success() {
        bail!("Video export failed (decoder: {decoded}, encoder: {encoded})");
    }
    let bytes = std::fs::metadata(plan.out)?.len();
    ensure!(bytes > 0, "The encoded movie is empty");
    let output = probe(plan.out).context("Could not verify the exported movie")?;
    ensure!(
        output.width == plan.width && output.height == plan.height,
        "The export has the wrong dimensions"
    );
    ensure!(
        (output.duration_ms as i64 - plan.duration_ms as i64).abs() < 250,
        "The export has the wrong duration"
    );
    ensure!(
        plan.audio.is_none() || output.audio_tracks > 0,
        "The export lost its audio"
    );
    Ok(Done {
        duration_ms: output.duration_ms,
        bytes,
    })
}

fn camera_at(plan: &Plan<'_>, ms: f64) -> [f32; 4] {
    let Some(keys) = plan.camera.as_ref().filter(|v| !v.is_empty()) else {
        return plan.crop;
    };
    let after = keys.partition_point(|key| key[0] <= ms);
    let (a, b) = if after == 0 {
        (&keys[0], &keys[0])
    } else if after == keys.len() {
        (&keys[after - 1], &keys[after - 1])
    } else {
        (&keys[after - 1], &keys[after])
    };
    let f = ((ms - a[0]) / (b[0] - a[0]).max(1e-6)).clamp(0.0, 1.0);
    std::array::from_fn(|i| (a[i + 1] + (b[i + 1] - a[i + 1]) * f) as f32)
}

fn resample(source: &[u8], sw: u32, sh: u32, ow: u32, oh: u32, rect: [f32; 4]) -> RgbaFrame {
    let mut pixels = vec![0; ow as usize * oh as usize * 4];
    for y in 0..oh {
        let sy = ((rect[1] + (y as f32 + 0.5) / oh as f32 * (rect[3] - rect[1])) * sh as f32)
            .floor()
            .clamp(0.0, (sh - 1) as f32) as usize;
        for x in 0..ow {
            let sx = ((rect[0] + (x as f32 + 0.5) / ow as f32 * (rect[2] - rect[0])) * sw as f32)
                .floor()
                .clamp(0.0, (sw - 1) as f32) as usize;
            let from = (sy * sw as usize + sx) * 4;
            let to = (y as usize * ow as usize + x as usize) * 4;
            pixels[to..to + 4].copy_from_slice(&source[from..from + 4]);
        }
    }
    RgbaFrame::new(0, ow, oh, pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resampling_crops_in_the_video_coordinate_space() {
        let input = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        assert_eq!(
            resample(&input, 2, 2, 2, 2, [0.5, 0.0, 1.0, 1.0]).rgba,
            [
                0, 255, 0, 255, 0, 255, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255
            ]
        );
    }
}
