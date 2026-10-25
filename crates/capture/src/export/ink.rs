//! The annotations burned into an export, as a time-stamped vector plan for the helper.
//!
//! Geometry comes from the same code as the keyframe renderer (`sketch::freehand_outline`,
//! `slice_laser_window`, `smooth_laser_points`; sizes mirror `render::annotate_keyframe_in_place`),
//! evaluated in the recording's own pixels and then normalized to the capture, so the helper can
//! draw it sharp through whatever the camera is doing. Times are on the video's clock.

use serde::Serialize;

use crate::model::{InteractionEvent, LASER_MS, ScrawlBundle, Stroke};
use crate::render::{annotation_sizes as sizes, is_long_press};
use crate::sketch::{FreehandOptions, freehand_outline, slice_laser_window, smooth_laser_points};

/// Ink stays on screen this long after its mark's last stroke, then fades.
pub const HOLD_MS: u64 = 2_500;
pub const FADE_MS: u64 = 500;
/// Progressive snapshots of a stroke being drawn are at least this far apart.
const SNAPSHOT_MS: u64 = 25;
const MAX_SNAPSHOTS: usize = 120;
/// Laser frames are sampled at about 60 Hz.
const LASER_STEP_MS: u64 = 16;

fn round(v: f32) -> f32 {
    (v * 1e5).round() / 1e5
}

#[derive(Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InkPlan {
    /// Source-pixel scale of ripples, lasers and pins (the keyframe renderer's `k`).
    pub k: f32,
    pub pin_scale: f32,
    pub strokes: Vec<InkStroke>,
    pub pins: Vec<Pin>,
    pub lasers: Vec<Laser>,
    pub ripples: Vec<Ripple>,
    pub trails: Vec<Trail>,
}

impl InkPlan {
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.strokes.is_empty()
            && self.pins.is_empty()
            && self.lasers.is_empty()
            && self.ripples.is_empty()
            && self.trails.is_empty()
    }
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InkStroke {
    /// The outline as drawn up to each moment, flat `x, y` pairs.
    pub snaps: Vec<Snap>,
    pub fade_at: u64,
    pub fade_ms: u64,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Snap {
    pub t: u64,
    pub poly: Vec<f32>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pin {
    pub x: f32,
    pub y: f32,
    pub n: u32,
    pub t: u64,
    pub fade_at: u64,
    pub fade_ms: u64,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Laser {
    pub frames: Vec<LaserFrame>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct LaserFrame {
    pub t: u64,
    /// Flat `x, y, life` triples.
    pub pts: Vec<f32>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Ripple {
    pub x: f32,
    pub y: f32,
    pub t: u64,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Trail {
    /// Flat `x, y` pairs.
    pub pts: Vec<f32>,
    pub t: u64,
    pub t1: u64,
}

/// What to draw: ink (strokes, pins, laser) and click ripples with touch trails.
#[derive(Clone, Copy, Debug)]
pub struct Layers {
    pub ink: bool,
    pub clicks: bool,
}

/// Builds the plan for `bundle` on a video of `video_px`, whose first frame is `offset_ms` into
/// the bundle's clock. Anything that ended before the video began is left out.
pub fn plan(bundle: &ScrawlBundle, video_px: (u32, u32), layers: Layers) -> InkPlan {
    let offset = bundle.video_offset_ms;
    let (w, h) = (video_px.0 as f32, video_px.1 as f32);
    let (k, size, pin_scale) = sizes(w, h);
    let mut plan = InkPlan {
        k,
        pin_scale,
        ..Default::default()
    };
    let vt = |t: u64| t.saturating_sub(offset);

    if layers.ink {
        // A mark's strokes fade together, after the last of them.
        let mut group_end: Vec<u64> = bundle.strokes.iter().map(|s| s.t1).collect();
        for mark in &bundle.marks {
            let end = mark
                .strokes
                .iter()
                .filter_map(|&i| bundle.strokes.get(i))
                .map(|s| s.t1)
                .max();
            if let Some(end) = end {
                for &i in &mark.strokes {
                    if let Some(slot) = group_end.get_mut(i) {
                        *slot = end;
                    }
                }
            }
        }
        for (index, stroke) in bundle.strokes.iter().enumerate() {
            if stroke.t1 < offset && group_end[index] + HOLD_MS + FADE_MS < offset {
                continue;
            }
            let fade_at = vt(group_end[index] + HOLD_MS);
            if stroke.laser {
                if let Some(laser) = laser(stroke, (w, h), vt) {
                    plan.lasers.push(laser);
                }
            } else if let Some(number) = stroke.pin {
                if let Some(&(x, y, _)) = stroke.timed_points().first() {
                    plan.pins.push(Pin {
                        x: round(x),
                        y: round(y),
                        n: number,
                        t: vt(stroke.t0),
                        fade_at,
                        fade_ms: FADE_MS,
                    });
                }
            } else if let Some(ink) = ink_stroke(stroke, (w, h), size, fade_at, vt) {
                plan.strokes.push(ink);
            }
        }
    }

    if layers.clicks {
        for event in &bundle.events {
            match event {
                InteractionEvent::Click { t, x, y, .. } if *t >= offset => {
                    plan.ripples.push(Ripple {
                        x: round(*x),
                        y: round(*y),
                        t: vt(*t),
                    });
                }
                InteractionEvent::Drag {
                    t,
                    t1,
                    x0,
                    y0,
                    label,
                    ..
                } if is_long_press(label.as_deref()) && *t >= offset => {
                    plan.ripples.push(Ripple {
                        x: round(*x0),
                        y: round(*y0),
                        t: vt(*t1),
                    });
                }
                InteractionEvent::Drag {
                    t,
                    t1,
                    x0,
                    y0,
                    x1,
                    y1,
                    path,
                    button,
                    label,
                } if button.as_deref() == Some("touch")
                    && !is_long_press(label.as_deref())
                    && *t >= offset =>
                {
                    let pts: Vec<[f32; 2]> = if path.len() >= 2 {
                        path.clone()
                    } else {
                        vec![[*x0, *y0], [*x1, *y1]]
                    };
                    plan.trails.push(Trail {
                        pts: pts
                            .iter()
                            .flat_map(|p| [round(p[0]), round(p[1])])
                            .collect(),
                        t: vt(*t),
                        t1: vt(*t1),
                    });
                }
                _ => {}
            }
        }
    }
    plan
}

fn ink_stroke(
    stroke: &Stroke,
    (w, h): (f32, f32),
    size: f32,
    fade_at: u64,
    vt: impl Fn(u64) -> u64,
) -> Option<InkStroke> {
    let timed = stroke.timed_points();
    if timed.is_empty() {
        return None;
    }
    let px: Vec<[f32; 2]> = timed.iter().map(|&(x, y, _)| [x * w, y * h]).collect();
    let options = FreehandOptions {
        size,
        ..Default::default()
    };
    let outline_to = |n: usize| -> Vec<f32> {
        freehand_outline(&px[..n], &options)
            .iter()
            .flat_map(|p| [round(p[0] / w), round(p[1] / h)])
            .collect()
    };
    // Which prefixes to keep: far enough apart in time, evenly thinned past the cap.
    let mut keep: Vec<usize> = Vec::new();
    let mut last = None;
    for (i, &(_, _, t)) in timed.iter().enumerate() {
        if last.is_none_or(|l| t.saturating_sub(l) >= SNAPSHOT_MS) {
            keep.push(i + 1);
            last = Some(t);
        }
    }
    if keep.last() != Some(&timed.len()) {
        keep.push(timed.len());
    }
    if keep.len() > MAX_SNAPSHOTS {
        let n = keep.len();
        keep = (0..MAX_SNAPSHOTS)
            .map(|i| keep[(i * (n - 1)) / (MAX_SNAPSHOTS - 1)])
            .collect();
        keep.dedup();
    }
    let snaps: Vec<Snap> = keep
        .into_iter()
        .map(|n| Snap {
            t: vt(timed[n - 1].2),
            poly: outline_to(n),
        })
        .filter(|s| s.poly.len() >= 6)
        .collect();
    (!snaps.is_empty()).then_some(InkStroke {
        snaps,
        fade_at,
        fade_ms: FADE_MS,
    })
}

fn laser(stroke: &Stroke, (w, h): (f32, f32), vt: impl Fn(u64) -> u64) -> Option<Laser> {
    let timed = stroke.timed_points();
    let first = timed.first()?.2;
    let last = timed.last()?.2 + LASER_MS;
    let mut frames = Vec::new();
    let mut t = first;
    loop {
        let raw = slice_laser_window(&timed, t, LASER_MS, |x, y| [x * w, y * h]);
        let smoothed = smooth_laser_points(&raw);
        frames.push(LaserFrame {
            t: vt(t),
            pts: smoothed
                .iter()
                .flat_map(|(p, life)| [round(p[0] / w), round(p[1] / h), round(*life)])
                .collect(),
        });
        if t >= last {
            break;
        }
        t = (t + LASER_STEP_MS).min(last);
    }
    Some(Laser { frames })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Mark, Shape};

    fn bundle() -> ScrawlBundle {
        serde_json::from_value(serde_json::json!({
            "version": 1, "id": "t", "createdAtMs": 0, "durationMs": 20_000,
            "target": {"mode": "display", "displayId": 0,
                "frame": {"x": 0.0, "y": 0.0, "width": 2560.0, "height": 1440.0, "scale": 2.0}},
            "transcript": {"source": "test", "text": "", "segments": []},
            "strokes": [], "marks": [], "events": [], "keyframes": [], "contactSheet": "c.png",
            "videoOffsetMs": 100
        }))
        .unwrap()
    }

    fn stroke(t0: u64, pts: &[(i32, i32, i32)]) -> Stroke {
        Stroke {
            t0,
            t1: t0 + pts.last().map_or(0, |p| p.2 as u64),
            fw: 2560,
            fh: 1440,
            on: None,
            pin: None,
            laser: false,
            pts: pts.iter().flat_map(|&(x, y, t)| [x, y, t]).collect(),
        }
    }

    fn mark(strokes: Vec<usize>) -> Mark {
        Mark {
            id: "m1".into(),
            strokes,
            t0: 0,
            t1: 0,
            box_norm: [0.0; 4],
            shape: Shape::Mark,
            from: None,
            to: None,
            pin: None,
            on: None,
            description: String::new(),
            frame: None,
        }
    }

    fn drawn() -> Vec<(i32, i32, i32)> {
        (0..30)
            .map(|i| (3_000 + i * 100, 4_000 + (i * 37) % 900, i * 20))
            .collect()
    }

    #[test]
    fn ink_is_drawn_progressively_on_the_video_clock_and_fades_after_its_mark() {
        let mut b = bundle();
        b.strokes = vec![stroke(2_000, &drawn()), stroke(3_000, &drawn())];
        b.marks = vec![mark(vec![0, 1])];
        let layers = Layers {
            ink: true,
            clicks: false,
        };
        let plan = plan(&b, (2560, 1440), layers);
        assert_eq!(plan.strokes.len(), 2);
        let first = &plan.strokes[0];
        assert_eq!(
            first.snaps.first().unwrap().t,
            2_000 - 100,
            "bundle clock to video clock"
        );
        assert!(first.snaps.windows(2).all(|p| p[0].t < p[1].t));
        assert!(
            first
                .snaps
                .windows(2)
                .all(|p| p[0].poly.len() <= p[1].poly.len() + 8),
            "the outline grows as the stroke does"
        );
        // Both strokes of the mark fade after the later one ends (580 ms into the second).
        let end = 3_000 + 29 * 20;
        assert_eq!(first.fade_at, end + HOLD_MS - 100);
        assert_eq!(plan.strokes[1].fade_at, first.fade_at);
        assert!(
            first
                .snaps
                .iter()
                .flat_map(|s| &s.poly)
                .all(|v| (-0.1..=1.1).contains(v))
        );
        assert!(plan.ripples.is_empty());
    }

    #[test]
    fn layers_can_be_turned_off() {
        let mut b = bundle();
        b.strokes = vec![stroke(2_000, &drawn())];
        b.events = vec![InteractionEvent::Click {
            t: 5_000,
            x: 0.3,
            y: 0.4,
            button: None,
            label: None,
            role: None,
        }];
        let off = Layers {
            ink: false,
            clicks: false,
        };
        assert!(plan(&b, (2560, 1440), off).is_empty());
        let clicks = plan(
            &b,
            (2560, 1440),
            Layers {
                ink: false,
                clicks: true,
            },
        );
        assert_eq!(
            clicks.ripples,
            vec![Ripple {
                x: 0.3,
                y: 0.4,
                t: 4_900
            }]
        );
        assert!(clicks.strokes.is_empty());
    }

    #[test]
    fn pins_lasers_and_touch_trails_get_their_own_items() {
        let mut b = bundle();
        let mut pin = stroke(4_000, &[(2_000, 3_000, 0)]);
        pin.pin = Some(2);
        let mut beam = stroke(
            6_000,
            &[(1_000, 1_000, 0), (3_000, 2_000, 100), (5_000, 3_000, 200)],
        );
        beam.laser = true;
        b.strokes = vec![pin, beam];
        b.events = vec![InteractionEvent::Drag {
            t: 8_000,
            t1: 8_200,
            x0: 0.2,
            y0: 0.2,
            x1: 0.4,
            y1: 0.5,
            label: None,
            button: Some("touch".into()),
            path: Vec::new(),
        }];
        let plan = plan(
            &b,
            (1320, 2868),
            Layers {
                ink: true,
                clicks: true,
            },
        );
        assert_eq!(plan.pins.len(), 1);
        assert_eq!((plan.pins[0].n, plan.pins[0].t), (2, 3_900));
        assert_eq!(plan.lasers.len(), 1);
        let frames = &plan.lasers[0].frames;
        assert_eq!(frames.first().unwrap().t, 5_900);
        assert!(frames.last().unwrap().t >= 6_000 + 200 + LASER_MS - 100);
        assert!(frames.iter().any(|f| f.pts.len() >= 6));
        assert_eq!(plan.trails.len(), 1);
        assert_eq!(plan.trails[0].pts, vec![0.2, 0.2, 0.4, 0.5]);
    }

    #[test]
    fn long_strokes_are_thinned_to_a_snapshot_budget() {
        let long: Vec<(i32, i32, i32)> =
            (0..600).map(|i| (1_000 + i * 10, 5_000, i * 10)).collect();
        let mut b = bundle();
        b.strokes = vec![stroke(1_000, &long)];
        let plan = plan(
            &b,
            (2560, 1440),
            Layers {
                ink: true,
                clicks: false,
            },
        );
        let snaps = &plan.strokes[0].snaps;
        assert!(snaps.len() <= MAX_SNAPSHOTS);
        assert_eq!(snaps.last().unwrap().t, 1_000 + 5_990 - 100);
    }

    #[test]
    fn sizes_follow_the_keyframe_renderer() {
        // A 2560×1440 screen: ink 12 px wide (6 × k = 2).
        let (k, size, pin) = sizes(2560.0, 1440.0);
        assert_eq!((k, size), (2.0, 12.0));
        assert_eq!(pin, 2.0);
        let (k, size, _) = sizes(640.0, 400.0);
        assert_eq!(k, 1.0);
        assert!((size - 4.2).abs() < 1e-3);
    }
}
