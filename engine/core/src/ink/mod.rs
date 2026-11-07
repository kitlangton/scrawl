use serde::{Deserialize, Serialize};

/// Coordinates in `Stroke::pts` are scaled to `0..=10_000` (1/10000 of target width/height).
pub const SCALE: i32 = 10_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stroke {
    pub t0: u64,
    pub t1: u64,
    pub fw: u32,
    pub fh: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub laser: bool,
    /// Exact recorded sRGB bytes. Absent retains the renderer's unnamed legacy default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rgb: Option<[u8; 3]>,
    /// Flat triples `[x, y, dt_ms, ...]` where `x, y` are in `0..=SCALE` and `dt_ms` is ms since `t0`.
    pub pts: Vec<i32>,
}

impl Stroke {
    /// Moves the stroke `ms` earlier onto a clock that started at `ms`, shifting point `dt_ms`
    /// by any clipped prefix and dropping points before `ms`. Returns `false` if the stroke
    /// ended before `ms` or has no remaining points.
    pub fn rebase(&mut self, ms: u64) -> bool {
        if self.t1 < ms {
            self.pts.clear();
            return false;
        }
        let lost = ms.saturating_sub(self.t0) as i64;
        self.t0 = self.t0.saturating_sub(ms);
        self.t1 = self.t1.saturating_sub(ms);
        if lost > 0 {
            let mut kept = Vec::with_capacity(self.pts.len());
            for chunk in self.pts.chunks_exact(3) {
                let dt = i64::from(chunk[2]) - lost;
                if dt >= 0 {
                    kept.extend_from_slice(&[chunk[0], chunk[1], dt as i32]);
                }
            }
            self.pts = kept;
        }
        !self.pts.is_empty()
    }

    /// Returns normalized `(x, y)` points in `0.0..=1.0`.
    pub fn normalized_points(&self) -> Vec<[f32; 2]> {
        self.pts
            .chunks_exact(3)
            .map(|c| [c[0] as f32 / SCALE as f32, c[1] as f32 / SCALE as f32])
            .collect()
    }

    /// Returns `(x, y, absolute_t_ms)` points.
    pub fn timed_points(&self) -> Vec<(f32, f32, u64)> {
        self.pts
            .chunks_exact(3)
            .map(|c| {
                (
                    c[0] as f32 / SCALE as f32,
                    c[1] as f32 / SCALE as f32,
                    self.t0.saturating_add(c[2].max(0) as u64),
                )
            })
            .collect()
    }
}

/// Clip each segment to the recording rectangle. Split on excursions so an outside
/// detour never becomes a fictitious line along the edge or across the picture.
pub fn clip_recording_ink(stroke: &Stroke) -> Vec<Stroke> {
    let mut result: Vec<Stroke> = Vec::new();
    let mut connected = false;
    for pair in stroke.pts.chunks_exact(3).collect::<Vec<_>>().windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let mut enter = 0.0_f64;
        let mut exit = 1.0_f64;
        let mut visible = true;
        for axis in 0..2 {
            let start = a[axis] as f64;
            let delta = b[axis] as f64 - start;
            if delta == 0.0 {
                visible &= (0.0..=SCALE as f64).contains(&start);
            } else {
                let t0 = -start / delta;
                let t1 = (SCALE as f64 - start) / delta;
                enter = enter.max(t0.min(t1));
                exit = exit.min(t0.max(t1));
            }
        }
        if !visible || enter >= exit {
            connected = false;
            continue;
        }
        let point = |t: f64| -> [i32; 3] {
            std::array::from_fn(|i| (a[i] as f64 + (b[i] as f64 - a[i] as f64) * t).round() as i32)
        };
        let start = point(enter);
        let end = point(exit);
        if !connected || enter > 0.0 {
            let mut part = stroke.clone();
            part.pts.clear();
            result.push(part);
        }
        let part = result.last_mut().unwrap();
        if part.pts.is_empty() {
            part.pts.extend(start);
        }
        part.pts.extend(end);
        part.t1 = stroke.t0.saturating_add(end[2].max(0) as u64);
        connected = exit == 1.0;
    }
    for part in &mut result {
        let offset = part.pts[2];
        part.t0 = part.t0.saturating_add(offset.max(0) as u64);
        for point in part.pts.chunks_exact_mut(3) {
            point[2] -= offset;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{clip_recording_ink, Stroke, SCALE};

    #[test]
    fn stroke_rgb_preserves_legacy_bytes_and_exact_colored_rebase() {
        let old = r#"{"t0":100,"t1":200,"fw":100,"fh":100,"pts":[100,200,0,300,400,100]}"#;
        let mut stroke: Stroke = serde_json::from_str(old).unwrap();
        assert_eq!(stroke.rgb, None);
        assert_eq!(serde_json::to_string(&stroke).unwrap(), old);
        stroke.rgb = Some([0, 203, 255]);
        let saved = serde_json::to_string(&stroke).unwrap();
        assert!(saved.contains("\"rgb\":[0,203,255]"));
        let mut restored: Stroke = serde_json::from_str(&saved).unwrap();
        assert!(restored.rebase(150));
        assert_eq!(restored.rgb, Some([0, 203, 255]));
        for invalid in [
            "[-1,0,0]",
            "[256,0,0]",
            "[0.5,0,0]",
            "[0,0]",
            "[0,0,0,0]",
            "\"coral\"",
        ] {
            let bad = old.replace("\"pts\":", &format!("\"rgb\":{invalid},\"pts\":"));
            assert!(serde_json::from_str::<Stroke>(&bad).is_err(), "{invalid}");
        }
    }

    #[test]
    fn clipping_splits_outside_detours_without_phantom_edge_marks() {
        let s = SCALE;
        let stroke = Stroke {
            t0: 100,
            t1: 150,
            fw: 1000,
            fh: 1000,
            on: None,
            pin: None,
            laser: false,
            rgb: None,
            pts: vec![
                s / 2,
                s / 4,
                0,
                2 * s,
                s / 4,
                10,
                2 * s,
                3 * s / 4,
                20,
                s / 2,
                3 * s / 4,
                30,
            ],
        };
        let parts = clip_recording_ink(&stroke);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].pts[4], s / 4);
        assert_eq!(parts[1].pts[1], 3 * s / 4);
        assert!(parts[1].t0 > parts[0].t1);
        let mut tail = stroke.clone();
        tail.pts = vec![-s, s / 2, 0, 1, s / 2, 100];
        assert_eq!(
            clip_recording_ink(&tail).len(),
            1,
            "a real stroke retains even a sub-6pt visible tail"
        );
        let mut outside = stroke.clone();
        outside.pts = vec![-s, 0, 0, -s, s, 10];
        assert!(clip_recording_ink(&outside).is_empty());
        outside.pts = vec![-s, s / 2, 0, 2 * s, s / 2, 30];
        let crossing = clip_recording_ink(&outside);
        assert_eq!(crossing[0].t0, 110);
        assert_eq!(crossing[0].pts, vec![0, s / 2, 0, s, s / 2, 10]);
    }

    #[test]
    fn stroke_geometry_keeps_legacy_malformed_tail_behavior() {
        let stroke = Stroke {
            t0: 10,
            t1: 20,
            fw: 2,
            fh: 2,
            on: None,
            pin: None,
            laser: false,
            rgb: None,
            pts: vec![5_000, 2_500, 5, 9_000, 9_000],
        };
        assert_eq!(stroke.normalized_points(), vec![[0.5, 0.25]]);
        assert_eq!(stroke.timed_points(), vec![(0.5, 0.25, 15)]);
        assert!(clip_recording_ink(&stroke).is_empty());
    }
}
