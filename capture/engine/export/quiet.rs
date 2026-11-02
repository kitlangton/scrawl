//! Conservative presentation-only compression. Detection uses full source frames, never keyframes.
use super::Done;
use crate::shared::model::{ApparentBundle, InteractionEvent};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{path::Path, process::Command};

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Interval {
    start_ms: u64,
    end_ms: u64,
    rate: u32,
}

fn ffmpeg(command: &mut Command) -> Result<String> {
    let output = command.output().context(
        "Quiet-stretch speedup needs ffmpeg on PATH; use --no-speedup to preserve recorded timing",
    )?;
    ensure!(
        output.status.success(),
        "Quiet-stretch processing failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8_lossy(&output.stderr).into_owned())
}

// FFmpeg reports the start retroactively once the minimum duration is reached.
fn ranges(log: &str, prefix: &str, duration: u64) -> Vec<(u64, u64)> {
    let mut start = None;
    let mut out = Vec::new();
    for line in log.lines() {
        for (suffix, opening) in [("start:", true), ("end:", false)] {
            let key = format!("{prefix}_{suffix}");
            if let Some((_, tail)) = line.split_once(&key) {
                if let Some(ms) = tail
                    .split_whitespace()
                    .next()
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    .map(|v| (v * 1000.0).round() as u64)
                {
                    if opening {
                        start = Some(ms);
                    } else if let Some(begin) = start.take() {
                        out.push((begin, ms.min(duration)));
                    }
                }
            }
        }
    }
    if let Some(begin) = start {
        out.push((begin, duration));
    }
    out
}

fn select(frozen: &[(u64, u64)], silent: &[(u64, u64)], protected: &[(u64, u64)]) -> Vec<Interval> {
    let mut out = Vec::new();
    for &(a, b) in frozen {
        for &(c, d) in silent {
            // Preserve five seconds to read each new state and five before the next change.
            let start = a.max(c).saturating_add(5000);
            let end = b.min(d).saturating_sub(5000);
            let mut pieces = vec![(start, end)];
            for &(p, q) in protected {
                pieces = pieces
                    .into_iter()
                    .flat_map(|(x, y)| {
                        if p >= y || q <= x {
                            vec![(x, y)]
                        } else {
                            vec![(x, p.max(x)), (q.min(y), y)]
                        }
                    })
                    .collect();
            }
            for (start_ms, end_ms) in pieces {
                if end_ms.saturating_sub(start_ms) >= 10_000 {
                    out.push(Interval {
                        start_ms,
                        end_ms,
                        rate: 4,
                    });
                }
            }
        }
    }
    out.sort_by_key(|i| i.start_ms);
    // Bound filter graph size; later holds conservatively retain recorded timing.
    out.truncate(64);
    out
}

pub(super) fn analyze(
    video: &Path,
    audio: Option<&Path>,
    offset: u64,
    duration: u64,
    bundle: &ApparentBundle,
) -> Result<Vec<Interval>> {
    let log = ffmpeg(
        Command::new("ffmpeg")
            .args(["-nostdin", "-hide_banner", "-i"])
            .arg(video)
            .args(["-an", "-vf", "freezedetect=n=-60dB:d=12", "-f", "null", "-"]),
    )?;
    let frozen = ranges(&log, "freeze", duration);
    if frozen.is_empty() {
        return Ok(Vec::new());
    }
    let silent = if let Some(audio) = audio {
        let log = ffmpeg(
            Command::new("ffmpeg")
                .args(["-nostdin", "-hide_banner", "-ss"])
                .arg(format!("{:.3}", offset as f64 / 1000.0))
                .arg("-i")
                .arg(audio)
                .args(["-vn", "-af", "silencedetect=n=-55dB:d=1", "-f", "null", "-"]),
        )?;
        ranges(&log, "silence", duration)
    } else {
        vec![(0, duration)]
    };
    let mut protected = Vec::new();
    let mut protect = |start: u64, end: u64| {
        let start = start.saturating_sub(bundle.video_offset_ms);
        let end = end.saturating_sub(bundle.video_offset_ms);
        protected.push((start.saturating_sub(2000), end.saturating_add(5000)));
    };
    for event in &bundle.events {
        let start = event.timestamp_ms();
        let end = if let InteractionEvent::Drag { t1, .. } = event {
            *t1
        } else {
            start
        };
        protect(start, end);
    }
    for stroke in &bundle.strokes {
        protect(stroke.t0, stroke.t1);
    }
    for mark in &bundle.marks {
        protect(mark.t0, mark.t1);
    }
    for segment in &bundle.transcript.segments {
        protect(segment.t0, segment.t1);
    }
    // Samples are emitted while moving. A gap between samples is not continuous motion.
    for sample in &bundle.cursor {
        protect(sample.0, sample.0);
    }
    let intervals = select(&frozen, &silent, &protected);
    eprintln!(
        "Quiet-stretch speedup (source video milliseconds): {}",
        serde_json::to_string(&intervals)?
    );
    Ok(intervals)
}

// A fixed vector 4x badge keeps export independent of fonts and optional drawtext builds.
fn badge_filter(enabled: &str) -> String {
    let mut filters = vec![format!(
        "drawbox=x=iw-48:y=10:w=38:h=27:color=black@0.7:t=fill:enable='{enabled}'"
    )];
    for (row, pixels) in ["1010000", "1010101", "1110010", "0010101", "0010000"]
        .iter()
        .enumerate()
    {
        for (column, pixel) in pixels.bytes().enumerate() {
            if pixel == b'1' {
                filters.push(format!(
                    "drawbox=x=iw-40+{}:y={}:w=3:h=3:color=white:t=fill:enable='{enabled}'",
                    column * 3,
                    16 + row * 3
                ));
            }
        }
    }
    filters.join(",")
}

pub(super) fn retime(
    movie: &Path,
    intervals: &[Interval],
    duration: u64,
    audio: bool,
    hevc: bool,
    fps: u32,
) -> Result<Done> {
    let output = movie.with_extension("paced.mp4");
    let result = (|| {
        let mut spans = Vec::new();
        let mut cursor = 0;
        for i in intervals {
            if cursor < i.start_ms {
                spans.push((cursor, i.start_ms, 1));
            }
            spans.push((i.start_ms, i.end_ms, i.rate));
            cursor = i.end_ms;
        }
        if cursor < duration {
            spans.push((cursor, duration, 1));
        }
        let mut filters = Vec::new();
        let mut inputs = String::new();
        let mut expected = 0.0;
        let mut savings = Vec::new();
        let mut badge_times = Vec::new();
        for (i, (start, end, rate)) in spans.iter().enumerate() {
            let a = *start as f64 / 1000.0;
            let b = *end as f64 / 1000.0;
            if *rate > 1 {
                badge_times.push(format!(
                    "between(t,{expected:.6},{:.6})",
                    expected + (b - a) / *rate as f64
                ));
                savings.push(format!(
                    "min(max(T-{a:.6},0),{:.6})*{:.6}",
                    b - a,
                    1.0 - 1.0 / *rate as f64
                ));
            }
            expected += (b - a) / *rate as f64;
            if audio {
                filters.push(format!("[0:a]apad,atrim=start={a:.3}:end={b:.3},asetpts=PTS-STARTPTS,atempo={rate}[a{i}]"));
                inputs.push_str(&format!("[a{i}]"));
            }
        }
        // One continuous source->output map avoids rounding a frame duration at every join.
        filters.push(format!(
            "[0:v]setpts='PTS-STARTPTS-({})/TB',{}[v]",
            savings.join("+"),
            badge_filter(&badge_times.join("+"))
        ));
        if audio {
            filters.push(format!("{inputs}concat=n={}:v=0:a=1[a]", spans.len()));
        }
        let mut command = Command::new("ffmpeg");
        command
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(movie)
            .args(["-filter_complex", &filters.join(";"), "-map", "[v]"]);
        if audio {
            command.args(["-map", "[a]", "-c:a", "aac"]);
        }
        command
            .args([
                "-c:v",
                if hevc { "libx265" } else { "libx264" },
                "-pix_fmt",
                "yuv420p",
                "-r",
            ])
            .arg(fps.to_string())
            .args(["-movflags", "+faststart", "-metadata"])
            .arg(format!(
                "comment=Apparent quiet-stretch source intervals: {}",
                serde_json::to_string(intervals)?
            ))
            .args(["-y"])
            .arg(&output);
        ffmpeg(&mut command)?;
        let bytes = std::fs::metadata(&output)?.len();
        ensure!(bytes > 0, "Empty paced export");
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration:stream=codec_type",
                "-of",
                "json",
            ])
            .arg(&output)
            .output()
            .context("Speedup verification needs ffprobe on PATH")?;
        ensure!(probe.status.success(), "Could not verify paced export");
        let value: serde_json::Value = serde_json::from_slice(&probe.stdout)?;
        let actual: f64 = value["format"]["duration"]
            .as_str()
            .context("Missing paced duration")?
            .parse()?;
        ensure!(
            (actual - expected).abs() < 0.25,
            "Paced export duration mismatch: {actual} vs {expected}"
        );
        let streams = value["streams"]
            .as_array()
            .context("Missing paced streams")?;
        ensure!(
            streams.iter().any(|s| s["codec_type"] == "video"),
            "Paced export lost video"
        );
        ensure!(
            !audio || streams.iter().any(|s| s["codec_type"] == "audio"),
            "Paced export lost audio"
        );
        std::fs::rename(&output, movie)?;
        Ok(Done {
            duration_ms: (actual * 1000.0).round() as u64,
            bytes,
        })
    })();
    let _ = std::fs::remove_file(output);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn holds_preserve_reading_time_and_actions() {
        assert!(select(&[(0, 19000)], &[(0, 19000)], &[]).is_empty());
        assert_eq!(
            select(&[(0, 40000)], &[(0, 40000)], &[]),
            vec![Interval {
                start_ms: 5000,
                end_ms: 35000,
                rate: 4
            }]
        );
        let selected = select(&[(0, 60000)], &[(0, 60000)], &[(25000, 35000)]);
        assert_eq!(
            selected
                .iter()
                .map(|i| (i.start_ms, i.end_ms))
                .collect::<Vec<_>>(),
            vec![(5000, 25000), (35000, 55000)]
        );
        assert!(select(&[(0, 40000)], &[], &[]).is_empty());
    }
    #[test]
    fn detector_boundaries_include_terminal_holds() {
        assert_eq!(
            ranges(
                "[freezedetect] freeze_start: 2\n[freezedetect] freeze_end: 25",
                "freeze",
                30000
            ),
            vec![(2000, 25000)]
        );
        assert_eq!(
            ranges("silence_start: 0", "silence", 30000),
            vec![(0, 30000)]
        );
    }
    #[test]
    fn retime_keeps_audio_and_video_on_one_clock() {
        let root = std::env::temp_dir().join(format!(
            "apparent-quiet-{}",
            crate::shared::ids::make_bundle_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let movie = root.join("test.mp4");
        ffmpeg(
            Command::new("ffmpeg")
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=blue:s=160x90:r=10:d=40",
                    "-f",
                    "lavfi",
                    "-i",
                    "aevalsrc=if(gte(t\\,38)\\,0.3*sin(2*PI*440*t)\\,0):s=24000:d=40",
                    "-c:v",
                    "libx264",
                    "-c:a",
                    "aac",
                    "-y",
                ])
                .arg(&movie),
        )
        .unwrap();
        let original = std::fs::read(&movie).unwrap();
        let log = ffmpeg(
            Command::new("ffmpeg")
                .args(["-nostdin", "-hide_banner", "-i"])
                .arg(&movie)
                .args(["-an", "-vf", "freezedetect=n=-60dB:d=12", "-f", "null", "-"]),
        )
        .unwrap();
        assert!(!ranges(&log, "freeze", 40000).is_empty());

        let intervals = vec![Interval {
            start_ms: 5000,
            end_ms: 35000,
            rate: 4,
        }];
        let done = retime(&movie, &intervals, 40000, true, false, 10).unwrap();
        assert!((done.duration_ms as i64 - 17500).abs() < 250);
        let rms = |time: &str| {
            let out = Command::new("ffmpeg")
                .args(["-v", "error", "-ss", time, "-i"])
                .arg(&movie)
                .args(["-t", "0.2", "-f", "s16le", "-ac", "1", "-ar", "24000", "-"])
                .output()
                .unwrap();
            assert!(out.status.success());
            out.stdout
                .chunks_exact(2)
                .map(|s| i16::from_le_bytes([s[0], s[1]]).unsigned_abs() as u64)
                .sum::<u64>()
        };
        assert!(
            rms("15.8") > rms("14.0") + 100000,
            "source sound at 38s maps to 15.5s"
        );
        // Independently exercise the real detector, including a hold ending at EOF.
        // Rendered copy changes; the raw source can be retained byte-for-byte by its caller.
        assert_ne!(std::fs::read(&movie).unwrap(), original);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn many_fractional_boundaries_do_not_accumulate_timeline_drift() {
        let root = std::env::temp_dir().join(format!(
            "apparent-quiet-boundaries-{}",
            crate::shared::ids::make_bundle_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let movie = root.join("test.mp4");
        ffmpeg(
            Command::new("ffmpeg")
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=blue:s=160x90:r=10:d=220",
                    "-f",
                    "lavfi",
                    "-i",
                    "anullsrc=r=24000:cl=mono",
                    "-t",
                    "220",
                    "-c:v",
                    "libx264",
                    "-c:a",
                    "aac",
                    "-y",
                ])
                .arg(&movie),
        )
        .unwrap();
        let intervals: Vec<_> = (0..10)
            .map(|i| Interval {
                start_ms: 5137 + i * 20000,
                end_ms: 15354 + i * 20000,
                rate: 4,
            })
            .collect();
        let done = retime(&movie, &intervals, 220000, true, false, 10).unwrap();
        assert!((done.duration_ms as i64 - 143373).abs() < 250);
        std::fs::remove_dir_all(root).unwrap();
    }
}
