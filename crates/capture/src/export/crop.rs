//! What part of the recording an export keeps, how big it comes out, and where the audio starts.

use anyhow::{Result, bail, ensure};

use crate::autocrop::{self, FULL, Rect};
use crate::model::{CaptureMode, ScrawlBundle};

/// Output long edge unless `--size` says otherwise.
pub const DEFAULT_LONG_EDGE: u32 = 1920;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CropArg {
    Full,
    /// The activity region: auto-crop's whole-recording crop (Screen captures), else the whole
    /// capture.
    Auto,
    /// `x,y,w,h`, as fractions of the capture (all ≤ 1) or pixels of the recording's video.
    Rect([f32; 4]),
}

impl std::str::FromStr for CropArg {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        match text.trim() {
            "full" => Ok(Self::Full),
            "auto" => Ok(Self::Auto),
            other => {
                let parts: Vec<f32> = other
                    .split(',')
                    .map(|p| p.trim().parse::<f32>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| format!("expected full, auto, or x,y,w,h; got “{other}”"))?;
                let [x, y, w, h] = parts[..] else {
                    return Err(format!("a crop is x,y,w,h (four numbers); got “{other}”"));
                };
                if ![x, y, w, h].iter().all(|v| v.is_finite() && *v >= 0.0) || w <= 0.0 || h <= 0.0
                {
                    return Err("crop numbers must be positive".into());
                }
                Ok(Self::Rect([x, y, w, h]))
            }
        }
    }
}

/// The crop as `[x0, y0, x1, y1]` normalized to the capture.
pub fn resolve(arg: CropArg, bundle: &ScrawlBundle, video_px: (u32, u32)) -> Result<Rect> {
    match arg {
        CropArg::Full => Ok(FULL),
        CropArg::Auto => Ok(auto(bundle)),
        CropArg::Rect([x, y, w, h]) => {
            let pixels = x > 1.0 || y > 1.0 || w > 1.0 || h > 1.0;
            let (vw, vh) = (video_px.0.max(1) as f32, video_px.1.max(1) as f32);
            let rect = if pixels {
                [x / vw, y / vh, (x + w) / vw, (y + h) / vh]
            } else {
                [x, y, x + w, y + h]
            };
            ensure!(
                rect[2] <= 1.0001 && rect[3] <= 1.0001,
                "The crop runs past the edge of the recording"
            );
            let rect = [rect[0], rect[1], rect[2].min(1.0), rect[3].min(1.0)];
            ensure!(
                (rect[2] - rect[0]) * vw >= 16.0 && (rect[3] - rect[1]) * vh >= 16.0,
                "The crop is smaller than 16 px"
            );
            Ok(rect)
        }
    }
}

/// The region a Screen capture's activity covered over the whole recording: auto-crop's
/// `Scope::Whole` crop, else the union of its keyframe crops, else everything.
pub fn auto(bundle: &ScrawlBundle) -> Rect {
    if bundle.target.mode != CaptureMode::Display {
        return FULL;
    }
    let frame = bundle.target.frame;
    let keyframes: Vec<u64> = bundle.keyframes.iter().map(|k| k.t).collect();
    let empty = Vec::new();
    let changes = bundle.autocrop.as_ref().map_or(&empty, |a| &a.changes);
    let input = autocrop::Input::new(
        frame,
        bundle.duration_ms,
        &keyframes,
        &bundle.strokes,
        &bundle.events,
        &bundle.windows,
        &bundle.cursor,
        changes,
    );
    let settings = autocrop::Settings {
        scope: autocrop::Scope::Whole,
        ..autocrop::Settings::configured().unwrap_or_default()
    };
    let whole = autocrop::compute(&input, &settings).into_iter().next();
    if let Some(crop) = &whole
        && crop.kind != autocrop::CropKind::Full
    {
        return clamp01(crop.rect);
    }
    if whole.is_some() {
        return FULL;
    }
    // No keyframes to compute for: fall back on the crops the bundle already carries.
    bundle
        .autocrop
        .as_ref()
        .filter(|a| !a.crops.is_empty())
        .map(|a| {
            a.crops
                .iter()
                .map(|c| c.rect(&frame))
                .reduce(autocrop::union)
                .map_or(FULL, clamp01)
        })
        .unwrap_or(FULL)
}

fn clamp01(r: Rect) -> Rect {
    autocrop::intersect(r, FULL)
}

fn even(v: f32) -> u32 {
    (((v / 2.0).round() as u32) * 2).max(2)
}

/// Output size for a crop of `crop_px` source pixels: the crop's aspect, even sides, long edge
/// at most `cap`, and never larger than the source (no upscaling).
pub fn output_size(crop_px: (f32, f32), cap: u32) -> Result<(u32, u32)> {
    ensure!(cap >= 64, "--size must be at least 64");
    let (w, h) = crop_px;
    ensure!(w >= 2.0 && h >= 2.0, "The crop is empty");
    let scale = (cap as f32 / w.max(h)).min(1.0);
    Ok((even(w * scale), even(h * scale)))
}

/// Which part of an audio file the video's timeline covers. The video starts `video_offset_ms`
/// into the bundle's clock, which is also the audio file's clock, so the first
/// `video_offset_ms` of audio come off the front. Returns `(start_ms, length_ms)`; the length is
/// what the video can use, and the audio may be shorter than the video (the tail is silent).
pub fn audio_window(
    video_offset_ms: u64,
    video_duration_ms: u64,
    audio_duration_ms: u64,
) -> Option<(u64, u64)> {
    if audio_duration_ms <= video_offset_ms {
        return None;
    }
    Some((
        video_offset_ms,
        (audio_duration_ms - video_offset_ms).min(video_duration_ms),
    ))
}

pub fn validate_fps(fps: u32) -> Result<u32> {
    if !(1..=120).contains(&fps) {
        bail!("--fps must be between 1 and 120");
    }
    Ok(fps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_arguments_parse() {
        assert_eq!("full".parse(), Ok(CropArg::Full));
        assert_eq!("auto".parse(), Ok(CropArg::Auto));
        assert_eq!(
            "0.1, 0.2,0.5,0.6".parse(),
            Ok(CropArg::Rect([0.1, 0.2, 0.5, 0.6]))
        );
        assert!("1,2,3".parse::<CropArg>().is_err());
        assert!("a,b,c,d".parse::<CropArg>().is_err());
        assert!("0.1,0.1,0,0.5".parse::<CropArg>().is_err());
        assert!("zoomed".parse::<CropArg>().is_err());
    }

    fn bundle() -> ScrawlBundle {
        serde_json::from_value(serde_json::json!({
            "version": 1, "id": "t", "createdAtMs": 0, "durationMs": 10_000,
            "target": {"mode": "display", "displayId": 0,
                "frame": {"x": 0.0, "y": 0.0, "width": 2560.0, "height": 1440.0, "scale": 2.0}},
            "transcript": {"source": "test", "text": "", "segments": []},
            "strokes": [], "marks": [], "events": [], "keyframes": [], "contactSheet": "c.png"
        }))
        .unwrap()
    }

    #[test]
    fn rect_crops_are_fractions_or_video_pixels() {
        let b = bundle();
        let fractions = resolve(CropArg::Rect([0.25, 0.25, 0.5, 0.5]), &b, (2560, 1440)).unwrap();
        assert_eq!(fractions, [0.25, 0.25, 0.75, 0.75]);
        let pixels = resolve(
            CropArg::Rect([640.0, 360.0, 1280.0, 720.0]),
            &b,
            (2560, 1440),
        )
        .unwrap();
        assert_eq!(pixels, [0.25, 0.25, 0.75, 0.75]);
        assert!(
            resolve(
                CropArg::Rect([2000.0, 0.0, 1280.0, 720.0]),
                &b,
                (2560, 1440)
            )
            .is_err()
        );
        assert!(resolve(CropArg::Rect([0.0, 0.0, 0.001, 0.5]), &b, (2560, 1440)).is_err());
        assert_eq!(resolve(CropArg::Full, &b, (2560, 1440)).unwrap(), FULL);
    }

    #[test]
    fn auto_crop_needs_a_screen_capture_with_activity() {
        let mut b = bundle();
        assert_eq!(auto(&b), FULL, "nothing happened");
        b.target.mode = CaptureMode::Device;
        assert_eq!(auto(&b), FULL, "a phone is never cropped");
    }

    #[test]
    fn output_sizes_keep_the_aspect_with_even_sides_and_never_upscale() {
        assert_eq!(output_size((2560.0, 1440.0), 1920).unwrap(), (1920, 1080));
        assert_eq!(output_size((2023.0, 1059.0), 1920).unwrap(), (1920, 1006));
        let (w, h) = output_size((733.0, 309.0), 1920).unwrap();
        assert_eq!(
            (w, h),
            (734, 310),
            "small crops stay at native size, rounded to even"
        );
        let (w, h) = output_size((1320.0, 2868.0), 1920).unwrap();
        assert_eq!(h, 1920);
        assert_eq!(w % 2, 0);
        assert!((w as f32 / h as f32 - 1320.0 / 2868.0).abs() < 0.002);
        assert_eq!(output_size((2560.0, 1440.0), 1280).unwrap(), (1280, 720));
        assert!(output_size((2560.0, 1440.0), 8).is_err());
    }

    #[test]
    fn audio_starts_where_the_video_does() {
        // A Mac capture: the video starts 52 ms into the bundle clock; the audio started at 0.
        assert_eq!(audio_window(52, 67_540, 67_594), Some((52, 67_540)));
        // Long audio is cut to the video; short audio keeps its own length.
        assert_eq!(audio_window(52, 10_000, 67_594), Some((52, 10_000)));
        assert_eq!(audio_window(0, 268_306, 268_329), Some((0, 268_306)));
        // No audio left after the offset.
        assert_eq!(audio_window(500, 10_000, 400), None);
    }

    #[test]
    fn fps_is_bounded() {
        assert_eq!(validate_fps(60).unwrap(), 60);
        assert!(validate_fps(0).is_err());
        assert!(validate_fps(240).is_err());
    }
}
