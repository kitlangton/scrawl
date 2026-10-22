//! Auto-crop for Screen captures: work out from the window log, the ink, clicks and the cursor
//! where each keyframe's moment happened, and crop the keyframes to it.
//!
//! The algorithm in brief, per keyframe (the slice of the timeline halfway to its neighbours):
//!
//! 1. Collect the activity: ink, clicks, drags, scrolls, laser, cursor dwell, and the regions that
//!    changed since the previous keyframe, each with a weight. Cursor samples far from every
//!    deliberate input are outliers.
//! 2. Attribute the weight to the frontmost window under each point (a menu or panel counts for
//!    its app's window, the menu bar for the frontmost app's).
//! 3. If one to `max_windows` windows hold `snap_threshold` of it, crop to them (plus any input
//!    outside, and the menu). A window bigger than `big_window` of the screen is not a useful
//!    crop, so the crop focuses on the activity inside it (input, where the cursor rested, what a
//!    click or scroll changed), with `context` of the window as margin. With no window log, the
//!    activity box gets the same margin against the screen.
//! 4. Pad, grow to the minimum size, cap the zoom, normalise the aspect, keep it on screen.
//! 5. Neighbouring crops that overlap by `merge_iou` become one, and the zoom changes at most
//!    `zoom_step` (by area) between neighbours. Keyframes with no activity reuse the previous
//!    crop or take the frontmost window.
//! 6. Crops around activity move their edges into white space found in the frame (`Content`),
//!    reach across blank space to the nearest content, and trim wide blank margins.
//!
//! `scrawl autocrop` opens a playground that runs this same code (through `--serve`) to tune it,
//! and `scrawl autocrop-eval` scores it against a corpus of simulated captures with ground truth.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::model::{InteractionEvent, Keyframe, Mark, RgbaFrame, SCALE, Stroke, TargetFrame};
use crate::windows::{CursorSample, WindowSnapshot};

/// `[x0, y0, x1, y1]`, normalized to the capture.
pub type Rect = [f32; 4];

pub const FULL: Rect = [0.0, 0.0, 1.0, 1.0];

pub fn area(r: Rect) -> f32 {
    (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0)
}

pub fn union(a: Rect, b: Rect) -> Rect {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

pub fn intersect(a: Rect, b: Rect) -> Rect {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ]
}

pub fn iou(a: Rect, b: Rect) -> f32 {
    let i = area(intersect(a, b));
    let u = area(a) + area(b) - i;
    if u <= 0.0 { 0.0 } else { i / u }
}

pub fn contains(outer: Rect, inner: Rect) -> bool {
    const E: f32 = 1e-4;
    inner[0] >= outer[0] - E
        && inner[1] >= outer[1] - E
        && inner[2] <= outer[2] + E
        && inner[3] <= outer[3] + E
}

fn clip01(r: Rect) -> Rect {
    intersect(r, FULL)
}

fn union_opt(a: Option<Rect>, b: Rect) -> Rect {
    a.map_or(b, |a| union(a, b))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// One crop for the whole capture.
    Whole,
    /// A crop per keyframe.
    Keyframe,
    /// A crop per keyframe, then neighbours merged and zoom changes limited.
    Grouped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fallback {
    /// The previous crop, else the frontmost window.
    Previous,
    /// The frontmost window, else the previous crop.
    Frontmost,
    Screen,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub scope: Scope,
    /// Widens each keyframe's slice of the timeline (slices meet halfway between keyframes).
    pub reach_ms: u64,
    pub snap: bool,
    /// Share of the weighted activity the top one to `max_windows` windows must hold.
    pub snap_threshold: f32,
    pub max_windows: usize,
    /// Each extra window in a union must hold at least this share.
    pub min_share: f32,
    /// Windows covering more of the screen than this aren't a crop; focus inside them instead.
    pub big_window: f32,
    /// Margin around the activity when focusing, as a fraction of the window (or screen).
    pub context: f32,
    /// Grow the crop to keep every deliberate input (ink, clicks, drags) of the slice.
    #[serde(alias = "keepInk")]
    pub keep_input: bool,
    /// Pull the touched part of the menu bar or Dock into the crop.
    pub chrome: bool,
    /// Ignore agent apps' windows that cover most of the screen (Cua Driver's overlay).
    pub ignore_overlays: bool,
    /// Points around the crop.
    pub padding: f32,
    pub min_w: f32,
    pub min_h: f32,
    /// The crop covers at least `1 / max_zoom` of the screen along its longer side.
    pub max_zoom: f32,
    /// `free`, `auto` (nearest common ratio), `screen`, or `W:H`.
    pub aspect: String,
    pub w_stroke: f32,
    pub w_click: f32,
    pub w_laser: f32,
    pub w_scroll: f32,
    /// Per second of dwell.
    pub w_cursor: f32,
    pub w_change: f32,
    /// When there is deliberate input, the cursor and changes together weigh at most this much
    /// of it, so resting the cursor after drawing doesn't outvote the drawing.
    pub passive_cap: f32,
    /// Cursor samples count toward the crop's box only where the cursor rested this long (s).
    pub min_dwell: f32,
    /// Points this close (pt) outside a window count for it: underlines under a window's last
    /// line, circles around something at its edge.
    pub hit_slop: f32,
    /// Cursor samples farther than this (fraction of the screen diagonal) from every deliberate
    /// input are outliers.
    pub outlier: f32,
    pub merge_iou: f32,
    /// Max area ratio between neighbouring crops; the smaller grows. 6 or more is off.
    pub zoom_step: f32,
    pub fallback: Fallback,
    /// Crops covering more of the screen than this aren't worth it: keep the full frame.
    pub full_above: f32,
    /// Move crop edges (around activity, not windows) into nearby empty space so they don't cut
    /// through text or UI: at most this far outward (pt)...
    pub snap_out: f32,
    /// ...or this fraction of the crop's size, if more: enough to finish a column of text.
    pub snap_out_frac: f32,
    /// ...or this far inward, dropping a sliver of context.
    pub snap_in: f32,
    /// Then blank margins wider than this (pt) are trimmed, down to the minimum size.
    pub trim: f32,
    /// Regions that changed on screen within this distance (pt) of a click, drag or scroll are
    /// part of the activity: the page it scrolled, the popover it opened.
    pub cause_reach: f32,
    /// When more than half of a crop around activity is blank rows on one side of it (a prompt
    /// at the bottom of an empty terminal), grow that way to the nearest content within this
    /// many points, plus `content_extra` of it.
    pub content_reach: f32,
    pub content_extra: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            scope: Scope::Grouped,
            reach_ms: 0,
            snap: true,
            snap_threshold: 0.8,
            max_windows: 2,
            min_share: 0.1,
            big_window: 0.6,
            context: 0.15,
            keep_input: true,
            chrome: true,
            ignore_overlays: true,
            padding: 0.0,
            min_w: 320.0,
            min_h: 200.0,
            max_zoom: 4.0,
            aspect: "free".into(),
            w_stroke: 1.0,
            w_click: 0.6,
            w_laser: 0.4,
            w_scroll: 0.3,
            w_cursor: 0.15,
            w_change: 0.1,
            passive_cap: 0.5,
            min_dwell: 0.15,
            hit_slop: 24.0,
            outlier: 0.12,
            merge_iou: 0.6,
            zoom_step: 6.0,
            fallback: Fallback::Previous,
            full_above: 0.85,
            snap_out: 120.0,
            snap_out_frac: 0.6,
            snap_in: 60.0,
            trim: 24.0,
            cause_reach: 40.0,
            content_reach: 360.0,
            content_extra: 160.0,
        }
    }
}

impl Settings {
    /// From `~/.config/scrawl/config.json` `"autocrop"`: absent or `true` is the defaults,
    /// `false` is off, an object overrides some settings.
    pub fn from_config(value: Option<&serde_json::Value>) -> Option<Self> {
        match value {
            None | Some(serde_json::Value::Bool(true)) | Some(serde_json::Value::Null) => {
                Some(Self::default())
            }
            Some(serde_json::Value::Bool(false)) => None,
            Some(other) => Some(serde_json::from_value(other.clone()).unwrap_or_default()),
        }
    }

    pub fn configured() -> Option<Self> {
        let config = crate::handoff::load_config_from(&crate::handoff::default_config_path());
        Self::from_config(config.autocrop.as_ref())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Stroke,
    Click,
    Drag,
    Laser,
    Scroll,
    Cursor,
    Change,
}

impl Kind {
    /// Input someone made on purpose, as opposed to where the cursor rested or what changed.
    pub fn deliberate(self) -> bool {
        !matches!(self, Kind::Cursor | Kind::Change)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub kind: Kind,
    pub t0: u64,
    pub t1: u64,
    /// `(x, y, t)`.
    pub pts: Vec<(f32, f32, u64)>,
    /// Seconds the cursor rested here.
    pub dwell: f32,
    /// Change regions, and grouped ink from `pin_ink`, belong to one keyframe.
    pub keyframe: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Win {
    pub id: u32,
    pub app: String,
    pub rect: Rect,
    pub layer: i32,
    pub regular: bool,
    pub frontmost: bool,
}

impl Win {
    pub fn chrome(&self) -> bool {
        (self.layer == 24 && self.app == "Window Server")
            || (self.layer == 20 && self.app == "Dock")
    }

    pub fn label(&self) -> &str {
        match (self.chrome(), self.layer) {
            (true, 24) => "Menu bar",
            (true, _) => "Dock",
            _ => &self.app,
        }
    }
}

/// Everything the algorithm looks at, normalized to the capture.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    /// The capture's size in points.
    pub screen: [f32; 2],
    pub duration: u64,
    pub keyframes: Vec<u64>,
    pub items: Vec<Item>,
    pub snapshots: Vec<(u64, Vec<Win>)>,
    /// Where each keyframe's screen has something on it, for snapping edges.
    #[serde(skip)]
    pub content: Vec<Option<Content>>,
}

/// Which 3 pt cells of a frame have edges in them (text, borders, icons), as an integral image.
#[derive(Clone, Debug, Default)]
pub struct Content {
    pub cols: usize,
    pub rows: usize,
    sum: Vec<u32>,
}

impl Content {
    pub const CELL_PT: f32 = 3.0;

    pub fn from_frame(frame: &RgbaFrame, screen: [f32; 2]) -> Self {
        let cols = ((screen[0] / Self::CELL_PT).round() as usize).max(1);
        let rows = ((screen[1] / Self::CELL_PT).round() as usize).max(1);
        let (w, h) = (frame.width as usize, frame.height as usize);
        let mut sum = vec![0_u32; (cols + 1) * (rows + 1)];
        if w < 2 || h < 2 {
            return Self { cols, rows, sum };
        }
        // A frame can be smaller than one pixel per cell, so neighbours clamp to the edge.
        let luma = |x: usize, y: usize| {
            let i = (y.min(h - 1) * w + x.min(w - 1)) * 4;
            let p = &frame.rgba[i..i + 3];
            (i32::from(p[0]) * 3 + i32::from(p[1]) * 6 + i32::from(p[2])) / 10
        };
        let step = ((w / cols) / 3).max(1);
        for r in 0..rows {
            let (y0, y1) = (r * h / rows, ((r + 1) * h / rows).min(h - 1));
            let mut row_acc = 0_u32;
            for c in 0..cols {
                let (x0, x1) = (c * w / cols, ((c + 1) * w / cols).min(w - 1));
                let mut busy = false;
                'cell: for y in (y0..y1.max(y0 + 1)).step_by(step) {
                    for x in (x0..x1.max(x0 + 1)).step_by(step) {
                        let l = luma(x, y);
                        if (l - luma(x + 1, y)).abs() >= 28 || (l - luma(x, y + 1)).abs() >= 28 {
                            busy = true;
                            break 'cell;
                        }
                    }
                }
                row_acc += u32::from(busy);
                sum[(r + 1) * (cols + 1) + c + 1] = sum[r * (cols + 1) + c + 1] + row_acc;
            }
        }
        Self { cols, rows, sum }
    }

    /// Busy cells in columns `c0..c1` and rows `r0..r1`.
    pub fn count(&self, c0: usize, r0: usize, c1: usize, r1: usize) -> u32 {
        let (c1, r1) = (c1.min(self.cols), r1.min(self.rows));
        if c0 >= c1 || r0 >= r1 {
            return 0;
        }
        let at = |c: usize, r: usize| self.sum[r * (self.cols + 1) + c];
        at(c1, r1) + at(c0, r0) - at(c0, r1) - at(c1, r0)
    }
}

fn subsample<T: Clone>(v: &[T], max: usize) -> Vec<T> {
    if v.len() <= max || max < 2 {
        return v.to_vec();
    }
    (0..max)
        .map(|i| v[(i * (v.len() - 1) + (max - 1) / 2) / (max - 1)].clone())
        .collect()
}

impl Input {
    /// `changes[i]` are the regions keyframe `i` changed since keyframe `i - 1`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        frame: TargetFrame,
        duration: u64,
        keyframes: &[u64],
        strokes: &[Stroke],
        events: &[InteractionEvent],
        windows: &[WindowSnapshot],
        cursor: &[CursorSample],
        changes: &[Vec<Rect>],
    ) -> Self {
        let (sw, sh) = (frame.width.max(1.0) as f32, frame.height.max(1.0) as f32);
        let mut items = Vec::new();
        for s in strokes {
            let pts: Vec<_> = s.timed_points();
            if pts.is_empty() {
                continue;
            }
            items.push(Item {
                kind: if s.laser { Kind::Laser } else { Kind::Stroke },
                t0: s.t0,
                t1: s.t1,
                pts: subsample(&pts, 48),
                dwell: 0.0,
                keyframe: None,
            });
        }
        for e in events {
            let (kind, t0, t1, pts) = match e {
                InteractionEvent::Click { t, x, y, .. } => {
                    (Kind::Click, *t, *t, vec![(*x, *y, *t)])
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
                    let mid = (t + t1) / 2;
                    let mut pts = vec![(*x0, *y0, *t)];
                    pts.extend(path.iter().map(|p| (p[0], p[1], mid)));
                    pts.push((*x1, *y1, *t1));
                    (Kind::Drag, *t, *t1, subsample(&pts, 16))
                }
                InteractionEvent::Scroll { t, x, y, .. } => {
                    (Kind::Scroll, *t, *t, vec![(*x, *y, *t)])
                }
                InteractionEvent::Point { t, x, y, .. } => {
                    (Kind::Laser, *t, *t, vec![(*x, *y, *t)])
                }
                _ => continue,
            };
            items.push(Item {
                kind,
                t0,
                t1,
                pts,
                dwell: 0.0,
                keyframe: None,
            });
        }
        for (i, c) in cursor.iter().enumerate() {
            let next = cursor.get(i + 1).map_or(duration.max(c.0), |n| n.0);
            items.push(Item {
                kind: Kind::Cursor,
                t0: c.0,
                t1: next,
                pts: vec![(c.1, c.2, c.0)],
                dwell: (next.saturating_sub(c.0).min(4_000)) as f32 / 1_000.0,
                keyframe: None,
            });
        }
        for (k, regions) in changes.iter().enumerate() {
            let Some(&t) = keyframes.get(k) else { continue };
            for b in regions {
                items.push(Item {
                    kind: Kind::Change,
                    t0: t,
                    t1: t,
                    pts: vec![
                        (b[0], b[1], t),
                        (b[2], b[1], t),
                        (b[0], b[3], t),
                        (b[2], b[3], t),
                        ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0, t),
                    ],
                    dwell: area(*b).sqrt(),
                    keyframe: Some(k),
                });
            }
        }
        let snapshots = windows
            .iter()
            .map(|s| {
                let wins = s
                    .windows
                    .iter()
                    .map(|w| {
                        let [x, y, ww, wh] = w.bounds.map(|v| v as f32);
                        Win {
                            id: w.id,
                            app: w.app.clone(),
                            rect: [x / sw, y / sh, (x + ww) / sw, (y + wh) / sh],
                            layer: w.layer,
                            regular: w.regular,
                            frontmost: w.frontmost,
                        }
                    })
                    .collect();
                (s.t, wins)
            })
            .collect();
        Self {
            screen: [sw, sh],
            duration,
            keyframes: keyframes.to_vec(),
            items,
            snapshots,
            content: Vec::new(),
        }
    }

    /// Gives the ink of every mark grouped into a keyframe to that keyframe's crop alone, however
    /// long before it was drawn. `strokes` are the ones passed to `new`.
    pub fn pin_ink<'a>(
        &mut self,
        keyframes: impl IntoIterator<Item = &'a Keyframe>,
        strokes: &[Stroke],
        marks: &[Mark],
    ) {
        // `new` adds an item for each stroke with points, first and in order.
        let item_of: Vec<Option<usize>> = strokes
            .iter()
            .scan(0, |next, s| {
                Some((s.pts.len() >= 3).then(|| {
                    *next += 1;
                    *next - 1
                }))
            })
            .collect();
        for (k, keyframe) in keyframes.into_iter().enumerate() {
            let grouped = marks
                .iter()
                .filter(|m| m.frame.as_ref() == Some(&keyframe.file))
                .flat_map(|m| &m.strokes);
            for &i in grouped {
                if let Some(item) = item_of
                    .get(i)
                    .copied()
                    .flatten()
                    .map(|i| &mut self.items[i])
                    && item.kind == Kind::Stroke
                {
                    item.keyframe = Some(k);
                }
            }
        }
    }

    fn windows_at(&self, t: u64, s: &Settings) -> Vec<&Win> {
        let Some(first) = self.snapshots.first() else {
            return Vec::new();
        };
        let snap = self
            .snapshots
            .iter()
            .take_while(|(st, _)| *st <= t)
            .last()
            .unwrap_or(first);
        snap.1
            .iter()
            .filter(|w| {
                !(s.ignore_overlays && !w.regular && w.layer == 0 && area(clip01(w.rect)) > 0.85)
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CropKind {
    /// One window (plus any ink outside it).
    Window,
    /// Two or more windows.
    Windows,
    /// Around the activity inside a window too big to crop to.
    Focus,
    /// Around the activity, with no window to snap to.
    Activity,
    /// No activity: the previous crop, the frontmost window or the screen.
    Fallback,
    /// Not worth cropping.
    Full,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Share {
    pub id: u32,
    pub app: String,
    pub share: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Crop {
    pub rect: Rect,
    pub kind: CropKind,
    /// Why, for people (no window titles).
    pub reason: String,
    /// The apps the crop is about.
    pub apps: Vec<String>,
    /// The activity (and snapped windows) before padding and shaping.
    pub core: Option<Rect>,
    /// The activity itself: deliberate input, where the cursor rested, what a click changed.
    pub need: Option<Rect>,
    /// The window a Focus crop sits in: its edges may grow up to here to finish lines of text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub within: Option<Rect>,
    pub slice: [u64; 2],
    pub shares: Vec<Share>,
    /// Snapped window ids.
    pub snapped: Vec<u32>,
    /// Cursor samples dropped as outliers.
    pub outliers: Vec<[f32; 2]>,
    /// Items (by index into `Input::items`) that counted.
    pub used: Vec<usize>,
    #[serde(skip_serializing_if = "is_zero")]
    pub merged: usize,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub grown: bool,
}

fn is_zero(v: &usize) -> bool {
    *v == 0
}

fn weight(item: &Item, s: &Settings) -> f32 {
    match item.kind {
        Kind::Stroke => s.w_stroke,
        Kind::Click | Kind::Drag => s.w_click,
        Kind::Laser => s.w_laser,
        Kind::Scroll => s.w_scroll,
        Kind::Cursor => s.w_cursor * item.dwell,
        Kind::Change => s.w_change * (item.dwell * 3.0).clamp(0.2, 1.0),
    }
}

#[derive(Clone, Copy)]
struct Pt {
    x: f32,
    y: f32,
    t: u64,
    w: f32,
    dwell: f32,
    kind: Kind,
    item: usize,
    outlier: bool,
}

fn bbox(pts: impl Iterator<Item = (f32, f32)>) -> Option<Rect> {
    pts.fold(None, |r: Option<Rect>, (x, y)| {
        Some(union_opt(r, [x, y, x, y]))
    })
}

fn wmedian(pts: &[&Pt], key: impl Fn(&Pt) -> f32) -> f32 {
    let mut v: Vec<(f32, f32)> = pts.iter().map(|p| (key(p), p.w)).collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let half = v.iter().map(|p| p.1).sum::<f32>() / 2.0;
    let mut acc = 0.0;
    for (value, w) in &v {
        acc += w;
        if acc >= half {
            return *value;
        }
    }
    v.last().map_or(0.5, |p| p.0)
}

struct Ctx<'a> {
    input: &'a Input,
    s: &'a Settings,
    sw: f32,
    sh: f32,
}

impl Ctx<'_> {
    fn slice(&self, k: Option<usize>) -> [u64; 2] {
        let kfs = &self.input.keyframes;
        let Some(k) = k else {
            return [0, self.input.duration];
        };
        let t = kfs[k];
        let a = if k == 0 { 0 } else { (kfs[k - 1] + t) / 2 };
        let b = if k + 1 == kfs.len() {
            self.input.duration.max(t)
        } else {
            (t + kfs[k + 1]) / 2
        };
        [a.saturating_sub(self.s.reach_ms), b + self.s.reach_ms]
    }

    fn pad(&self, r: Rect, pt: f32) -> Rect {
        let (px, py) = (pt / self.sw, pt / self.sh);
        [r[0] - px, r[1] - py, r[2] + px, r[3] + py]
    }

    /// `core` grown by `context` of `within`'s size (the geometric mean of its sides, in points).
    fn margin(&self, core: Rect, within: Rect) -> Rect {
        let pt = self.s.context
            * (((within[2] - within[0]) * self.sw) * ((within[3] - within[1]) * self.sh)).sqrt();
        self.pad(core, pt)
    }

    fn aspect(&self, w: f32, h: f32) -> Option<f32> {
        match self.s.aspect.as_str() {
            "free" | "" => None,
            "screen" => Some(self.sw / self.sh),
            "auto" => {
                let cur = w / h;
                [
                    16.0 / 9.0,
                    1.6,
                    1.5,
                    4.0 / 3.0,
                    1.0,
                    0.75,
                    2.0 / 3.0,
                    9.0 / 16.0,
                ]
                .into_iter()
                .min_by(|a, b| (a / cur).ln().abs().total_cmp(&(b / cur).ln().abs()))
            }
            ratio => {
                let (a, b) = ratio.split_once(':')?;
                Some(a.trim().parse::<f32>().ok()? / b.trim().parse::<f32>().ok()?)
            }
        }
    }

    /// Minimum size, maximum zoom and aspect (only ever growing), then kept on screen and, when
    /// it fits, inside `within`.
    fn shape_in(&self, r: Rect, within: Rect) -> Rect {
        let s = self.s;
        let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
        let mut w = (r[2] - r[0]).max(1e-4) * self.sw;
        let mut h = (r[3] - r[1]).max(1e-4) * self.sh;
        w = w.max(s.min_w.min(self.sw));
        h = h.max(s.min_h.min(self.sh));
        let need = 1.0 / s.max_zoom.max(1.0);
        let cover = (w / self.sw).max(h / self.sh);
        if cover < need {
            w *= need / cover;
            h *= need / cover;
        }
        if let Some(ratio) = self.aspect(w, h) {
            if w / h < ratio {
                w = h * ratio;
            } else {
                h = w / ratio;
            }
        }
        let (w, h) = ((w / self.sw).min(1.0), (h / self.sh).min(1.0));
        let keep = |c: f32, half: f32, lo: f32, hi: f32| {
            let (lo, hi) = if hi - lo >= 2.0 * half {
                (lo, hi)
            } else {
                (0.0, 1.0)
            };
            // Not `clamp`: rounding can leave the bounds a hair out of order.
            c.max(lo + half).min(hi - half)
        };
        let cx = keep(cx, w / 2.0, within[0].max(0.0), within[2].min(1.0));
        let cy = keep(cy, h / 2.0, within[1].max(0.0), within[3].min(1.0));
        [cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0]
    }

    fn shape(&self, r: Rect) -> Rect {
        self.shape_in(r, FULL)
    }

    fn finish_in(&self, r: Rect, within: Rect) -> Rect {
        let r = self.shape_in(self.pad(r, self.s.padding), within);
        if area(r) >= self.s.full_above {
            FULL
        } else {
            r
        }
    }

    fn finish(&self, r: Rect) -> Rect {
        self.finish_in(r, FULL)
    }

    fn reach_content(&self, r: Rect, content: &Content, keep: Rect) -> Option<Rect> {
        if self.s.content_reach <= 0.0 {
            return None;
        }
        let (cols, rows) = (content.cols as f32, content.rows as f32);
        let c0 = (r[0] * cols).floor().max(0.0) as usize;
        let c1 = (r[2] * cols).ceil() as usize;
        let r0 = (r[1] * rows).floor().max(0.0) as usize;
        let r1 = ((r[3] * rows).ceil() as usize).min(content.rows);
        let tol = ((c1.saturating_sub(c0)) / 60) as u32;
        let empty = |row: usize| content.count(c0, row, c1, row + 1) <= tol;
        let (mut best, mut run) = ((0, 0), 0);
        for row in r0..r1 {
            run = if empty(row) { run + 1 } else { 0 };
            if run > best.1 - best.0 {
                best = (row + 1 - run, row + 1);
            }
        }
        if (best.1 - best.0) * 2 < r1.saturating_sub(r0) {
            return None;
        }
        let reach = (self.s.content_reach / Content::CELL_PT) as usize;
        let extra = self.s.content_extra / Content::CELL_PT;
        let (k0, k1) = ((keep[1] * rows) as usize, (keep[3] * rows).ceil() as usize);
        if best.1 <= k0 + 1 {
            // Blank above the activity: reach up.
            let found = (r0.saturating_sub(reach)..r0)
                .rev()
                .find(|&row| !empty(row))?;
            let top = (found as f32 - extra).max(0.0) / rows;
            Some([r[0], top, r[2], r[3]])
        } else if best.0 + 1 >= k1 {
            let found = (r1..(r1 + reach).min(content.rows)).find(|&row| !empty(row))?;
            let bottom = (found as f32 + 1.0 + extra).min(rows) / rows;
            Some([r[0], r[1], r[2], bottom])
        } else {
            None
        }
    }

    /// Widens `r` so its left and right edges don't cut through lines of text: each line that
    /// crosses an edge is followed to its end (a 12 pt gap), and the edge moves past the
    /// furthest end, at most as far as `snap` reaches and never out of `within`. Runs longer
    /// than that are rules, borders or wrapped-off text, not lines to finish.
    fn finish_lines(&self, r: Rect, content: &Content, within: Rect) -> Rect {
        let cell = Content::CELL_PT;
        let (cols, rows) = (content.cols as i64, content.rows as i64);
        let reach = (self
            .s
            .snap_out
            .max(self.s.snap_out_frac * (r[2] - r[0]) * self.sw)
            / cell)
            .round() as i64;
        let gap = (12.0 / cell).round() as i64;
        if reach <= 0 {
            return r;
        }
        let busy = |c: i64, row: i64| {
            (0..cols).contains(&c)
                && content.count(c as usize, row as usize, c as usize + 1, row as usize + 1) > 0
        };
        let mut c0 = (r[0] * content.cols as f32).floor() as i64;
        let mut c1 = (r[2] * content.cols as f32).ceil() as i64;
        let r0 = ((r[1] * content.rows as f32).floor() as i64).max(0);
        let r1 = ((r[3] * content.rows as f32).ceil() as i64).min(rows);
        let w0 = ((within[0].max(0.0) * content.cols as f32).floor() as i64).max(0);
        let w1 = ((within[2].min(1.0) * content.cols as f32).ceil() as i64).min(cols);
        // Where the line on `row` that crosses boundary `edge` ends going `dir`, if it does
        // within `limit`. `None` when nothing crosses there.
        let line_end = |row: i64, edge: i64, dir: i64, limit: i64| -> Option<Option<i64>> {
            // Cells just inside and just outside the boundary.
            let outside = if dir > 0 { edge } else { edge - 1 };
            let inside = outside - dir;
            let near = |from: i64, step: i64| (0..gap).any(|k| busy(from + step * k, row));
            if !near(inside, -dir) || !near(outside, dir) {
                return None;
            }
            let (mut c, mut last, mut blank) = (outside, inside, 0);
            while blank < gap {
                if (c - limit) * dir > 0 || !(0..cols).contains(&c) {
                    return Some(None);
                }
                if busy(c, row) {
                    last = c;
                    blank = 0;
                } else {
                    blank += 1;
                }
                c += dir;
            }
            // Ending right at the window's edge is a border, not the end of a line.
            Some(((limit - last) * dir >= 2 * gap).then_some(last))
        };
        let mut right = c1;
        let mut left = c0;
        for row in r0..r1 {
            if let Some(Some(end)) = line_end(row, c1, 1, (c1 + reach).min(w1) - 1) {
                right = right.max(end + 1 + gap);
            }
            if let Some(Some(start)) = line_end(row, c0, -1, (c0 - reach).max(w0)) {
                left = left.min(start - gap);
            }
        }
        c1 = right.min(w1);
        c0 = left.max(w0);
        clip01([
            c0 as f32 / content.cols as f32,
            r[1],
            c1 as f32 / content.cols as f32,
            r[3],
        ])
    }

    /// Moves each edge of `r` to the nearest stretch of empty screen (a column gap of 12 pt, or
    /// any empty row) within `snap_out` outward or `snap_in` inward, never into `keep`.
    fn snap(&self, r: Rect, content: &Content, keep: Rect) -> Rect {
        let (cols, rows) = (content.cols as f32, content.rows as f32);
        let cell = Content::CELL_PT;
        let reach =
            |size: f32| (self.s.snap_out.max(self.s.snap_out_frac * size) / cell).round() as i64;
        let (out_x, out_y) = (
            reach((r[2] - r[0]) * self.sw),
            reach((r[3] - r[1]) * self.sh),
        );
        let inn = (self.s.snap_in / cell).round() as i64;
        if out_x <= 0 && inn <= 0 {
            return r;
        }
        // One edge: `pos` is the edge's cell boundary, `empty(i)` whether line `i` (the cells
        // between boundaries i and i + 1) is empty, `dir` -1 for a low edge, +1 for a high one.
        let snap_edge = |pos: i64,
                         limit: i64,
                         n: i64,
                         gap: i64,
                         dir: i64,
                         out: i64,
                         empty: &dyn Fn(i64) -> bool|
         -> i64 {
            let clean = |b: i64| {
                // The `gap` lines just inside boundary b are empty, so nothing straddles it and
                // the crop keeps a little white space at that edge.
                (0..gap).all(|k| {
                    let line = if dir < 0 { b + k } else { b - 1 - k };
                    line < 0 || line >= n || empty(line)
                })
            };
            if clean(pos) {
                return pos;
            }
            for d in 1..=out.max(inn) {
                let o = pos + dir * d;
                if d <= out && (0..=n).contains(&o) && clean(o) {
                    return o;
                }
                let i = pos - dir * d;
                let inside_keep = if dir < 0 { i > limit } else { i < limit };
                if d <= inn && !inside_keep && (0..=n).contains(&i) && clean(i) {
                    return i;
                }
            }
            pos
        };
        let mut c0 = (r[0] * cols).floor() as i64;
        let mut c1 = (r[2] * cols).ceil() as i64;
        let mut r0 = (r[1] * rows).floor() as i64;
        let mut r1 = (r[3] * rows).ceil() as i64;
        let k = [
            (keep[0] * cols).floor() as i64,
            (keep[1] * rows).floor() as i64,
            (keep[2] * cols).ceil() as i64,
            (keep[3] * rows).ceil() as i64,
        ];
        // A line is empty when at most a tenth of it is busy: rules and borders that cross the
        // whole crop don't make a gutter look full.
        let tol = |span: i64| ((span / 10).max(0)) as u32;
        let (rr0, rr1) = (r0.max(0) as usize, r1.max(0) as usize);
        let col_empty =
            |c: i64| content.count(c as usize, rr0, c as usize + 1, rr1) <= tol(r1 - r0);
        let n = content.cols as i64;
        let gap_cols = (12.0 / cell).round() as i64;
        c0 = snap_edge(c0, k[0], n, gap_cols, -1, out_x, &col_empty);
        c1 = snap_edge(c1, k[2], n, gap_cols, 1, out_x, &col_empty);
        let (cc0, cc1) = (c0.max(0) as usize, c1.max(0) as usize);
        let row_empty =
            |r: i64| content.count(cc0, r as usize, cc1, r as usize + 1) <= tol(c1 - c0);
        let n = content.rows as i64;
        r0 = snap_edge(r0, k[1], n, 1, -1, out_y, &row_empty);
        r1 = snap_edge(r1, k[3], n, 1, 1, out_y, &row_empty);
        // Trim blank margins, a line at a time from the wider side, keeping `trim` of white.
        let margin = (self.s.trim / cell).round() as i64;
        if self.s.trim > 0.0 {
            let min_c =
                (self.s.min_w.max(self.sw / self.s.max_zoom.max(1.0)) / cell).round() as i64;
            let min_r = (self.s.min_h / cell).round() as i64;
            let (rr0, rr1) = (r0.max(0) as usize, r1.max(0) as usize);
            let few = |span: i64| ((span / 30).max(0)) as u32;
            let col_empty = |c: i64| {
                content.count(c.max(0) as usize, rr0, c.max(0) as usize + 1, rr1) <= few(r1 - r0)
            };
            let blank = |from: i64, step: i64, limit: i64, empty: &dyn Fn(i64) -> bool| {
                let mut n = 0;
                while (from + step * n - limit) * step < 0 && empty(from + step * n) {
                    n += 1;
                }
                n
            };
            let left = blank(c0, 1, k[0], &col_empty);
            let right = blank(c1 - 1, -1, k[2] - 1, &col_empty);
            let spare = (c1 - c0 - min_c).max(0);
            let cut_l = (left - margin).clamp(0, spare);
            let cut_r = (right - margin).clamp(0, spare - cut_l);
            c0 += cut_l;
            c1 -= cut_r;
            let (cc0, cc1) = (c0.max(0) as usize, c1.max(0) as usize);
            let row_empty = |r: i64| {
                content.count(cc0, r.max(0) as usize, cc1, r.max(0) as usize + 1) <= few(c1 - c0)
            };
            let top = blank(r0, 1, k[1], &row_empty);
            let bottom = blank(r1 - 1, -1, k[3] - 1, &row_empty);
            let spare = (r1 - r0 - min_r).max(0);
            let cut_t = (top - margin).clamp(0, spare);
            let cut_b = (bottom - margin).clamp(0, spare - cut_t);
            r0 += cut_t;
            r1 -= cut_b;
        }
        clip01([
            c0 as f32 / cols,
            r0 as f32 / rows,
            c1 as f32 / cols,
            r1 as f32 / rows,
        ])
    }

    fn frontmost(&self, t: u64) -> Option<&Win> {
        let wins = self.input.windows_at(t, self.s);
        let normal: Vec<&Win> = wins.into_iter().filter(|w| !w.chrome()).collect();
        normal
            .iter()
            .find(|w| w.frontmost)
            .or_else(|| normal.iter().find(|w| w.regular))
            .copied()
    }

    /// The crop for keyframe `k` (or the whole capture), before grouping. `None` when the slice
    /// has no activity.
    fn crop(&self, k: Option<usize>) -> Option<Crop> {
        let s = self.s;
        let slice = self.slice(k);
        let t_kf = k.map_or(self.input.duration / 2, |k| self.input.keyframes[k]);
        let mut pts: Vec<Pt> = Vec::new();
        for (i, item) in self.input.items.iter().enumerate() {
            let in_slice = item.t1 >= slice[0] && item.t0 <= slice[1];
            match (item.kind, item.keyframe) {
                // A grouped mark's ink belongs to its keyframe alone, whenever it was drawn.
                (Kind::Stroke, Some(_)) if k.is_some() && item.keyframe != k => continue,
                (Kind::Stroke, Some(_)) => {}
                (Kind::Change, Some(_)) if k.is_some() && item.keyframe != k => continue,
                _ if !in_slice => continue,
                _ => {}
            }
            let w = weight(item, s);
            if w <= 0.0 || item.pts.is_empty() {
                continue;
            }
            let each = w / item.pts.len() as f32;
            pts.extend(item.pts.iter().map(|&(x, y, t)| Pt {
                x,
                y,
                t,
                w: each,
                dwell: item.dwell,
                kind: item.kind,
                item: i,
                outlier: false,
            }));
        }
        let anchors: Vec<Pt> = subsample(
            &pts.iter()
                .filter(|p| p.kind.deliberate())
                .copied()
                .collect::<Vec<_>>(),
            300,
        );
        let diag = self.sw.hypot(self.sh);
        let dist = |a: &Pt, x: f32, y: f32| ((a.x - x) * self.sw).hypot((a.y - y) * self.sh) / diag;
        let center = if anchors.is_empty() {
            let cur: Vec<&Pt> = pts.iter().filter(|p| p.kind == Kind::Cursor).collect();
            (!cur.is_empty()).then(|| (wmedian(&cur, |p| p.x), wmedian(&cur, |p| p.y)))
        } else {
            None
        };
        for p in pts.iter_mut().filter(|p| p.kind == Kind::Cursor) {
            let d = if anchors.is_empty() {
                center.map_or(0.0, |(x, y)| dist(p, x, y))
            } else {
                anchors
                    .iter()
                    .map(|a| dist(p, a.x, a.y))
                    .fold(f32::MAX, f32::min)
            };
            p.outlier = d > s.outlier;
        }
        let outliers = pts
            .iter()
            .filter(|p| p.outlier)
            .map(|p| [p.x, p.y])
            .collect();
        let deliberate: f32 = pts
            .iter()
            .filter(|p| p.kind.deliberate())
            .map(|p| p.w)
            .sum();
        let passive: f32 = pts
            .iter()
            .filter(|p| !p.kind.deliberate() && !p.outlier)
            .map(|p| p.w)
            .sum();
        if deliberate > 0.0 && passive > s.passive_cap * deliberate {
            let f = s.passive_cap * deliberate / passive;
            for p in pts.iter_mut().filter(|p| !p.kind.deliberate()) {
                p.w *= f;
            }
        }
        let (slop_x, slop_y) = (s.hit_slop / self.sw, s.hit_slop / self.sh);

        let at_kf = self.input.windows_at(t_kf, s);
        let mut shares: HashMap<u32, (f32, Win)> = HashMap::new();
        let mut chrome_hits: HashMap<u32, (Win, Vec<(f32, f32)>)> = HashMap::new();
        let mut kept: Vec<Pt> = Vec::new();
        let mut panels: Option<Rect> = None;
        let mut total = 0.0;
        for p in pts.iter().filter(|p| !p.outlier) {
            let wins = self.input.windows_at(p.t, s);
            let inside = |w: &&&Win, mx: f32, my: f32| {
                p.x >= w.rect[0] - mx
                    && p.x <= w.rect[2] + mx
                    && p.y >= w.rect[1] - my
                    && p.y <= w.rect[3] + my
            };
            let hit = wins.iter().find(|w| inside(w, 0.0, 0.0)).or_else(|| {
                wins.iter()
                    .find(|w| !w.chrome() && inside(w, slop_x, slop_y))
            });
            match hit {
                Some(w) if w.chrome() => {
                    if s.chrome && p.kind.deliberate() {
                        chrome_hits
                            .entry(w.id)
                            .or_insert_with(|| ((*w).clone(), Vec::new()))
                            .1
                            .push((p.x, p.y));
                        total += p.w;
                        // The menu bar is the frontmost app's: it counts for that app's window.
                        if w.layer == 24
                            && let Some(front) = wins.iter().find(|o| o.frontmost && !o.chrome())
                        {
                            let now = at_kf
                                .iter()
                                .find(|x| x.id == front.id)
                                .copied()
                                .unwrap_or(front);
                            shares
                                .entry(front.id)
                                .or_insert_with(|| (0.0, (*now).clone()))
                                .0 += p.w;
                        }
                    }
                }
                Some(hit) => {
                    total += p.w;
                    kept.push(*p);
                    // A menu or panel belongs to its app's window: it counts for that window,
                    // and the crop grows to show it.
                    let owner = (hit.layer != 0)
                        .then(|| wins.iter().find(|o| o.layer == 0 && o.app == hit.app))
                        .flatten();
                    let w = match owner {
                        Some(owner) => {
                            if p.kind.deliberate() || p.dwell >= s.min_dwell {
                                panels = Some(union_opt(panels, clip01(hit.rect)));
                            }
                            owner
                        }
                        None => hit,
                    };
                    let now = at_kf.iter().find(|x| x.id == w.id).copied().unwrap_or(w);
                    shares
                        .entry(w.id)
                        .or_insert_with(|| (0.0, (*now).clone()))
                        .0 += p.w;
                }
                None => {
                    total += p.w;
                    kept.push(*p);
                }
            }
        }
        if total <= 0.0 {
            return None;
        }
        let mut ranked: Vec<(f32, Win)> = shares.into_values().collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
        let share_list = ranked
            .iter()
            .map(|(w, win)| Share {
                id: win.id,
                app: win.app.clone(),
                share: w / total,
            })
            .collect();
        let used = {
            let mut v: Vec<usize> = kept.iter().map(|p| p.item).collect();
            v.extend(
                pts.iter()
                    .filter(|p| {
                        !p.outlier && chrome_hits.values().any(|(_, c)| c.contains(&(p.x, p.y)))
                    })
                    .map(|p| p.item),
            );
            v.sort_unstable();
            v.dedup();
            v
        };
        // The box: deliberate input and where the cursor rested; what changed on screen only
        // when there is nothing else.
        let ink = bbox(
            kept.iter()
                .filter(|p| p.kind.deliberate())
                .map(|p| (p.x, p.y)),
        );
        let rested = bbox(
            kept.iter()
                .filter(|p| {
                    p.kind.deliberate() || (p.kind == Kind::Cursor && p.dwell >= s.min_dwell)
                })
                .map(|p| (p.x, p.y)),
        );
        // Changes near a click, drag or scroll are what it did: the scrolled page, a popover.
        let pokes: Vec<(f32, f32)> = pts
            .iter()
            .filter(|p| matches!(p.kind, Kind::Click | Kind::Drag | Kind::Scroll))
            .map(|p| (p.x, p.y))
            .collect();
        let (rx, ry) = (s.cause_reach / self.sw, s.cause_reach / self.sh);
        let caused = self
            .input
            .items
            .iter()
            .filter(|it| it.kind == Kind::Change && (k.is_none() || it.keyframe == k))
            .filter(|it| it.t0 >= slice[0] && it.t0 <= slice[1])
            .filter_map(|it| bbox(it.pts.iter().map(|&(x, y, _)| (x, y))))
            .filter(|b| {
                s.cause_reach > 0.0
                    && pokes.iter().any(|&(x, y)| {
                        x >= b[0] - rx && x <= b[2] + rx && y >= b[1] - ry && y <= b[3] + ry
                    })
            })
            .fold(None, |r: Option<Rect>, b| Some(union_opt(r, b)));
        let rested = match (rested, caused) {
            (Some(a), Some(b)) => Some(union(a, b)),
            (a, b) => a.or(b),
        };
        let activity = rested.or_else(|| bbox(kept.iter().map(|p| (p.x, p.y))));

        let mut snapped: Vec<&Win> = Vec::new();
        let mut cum = 0.0;
        if s.snap {
            for (n, (w, _)) in ranked.iter().enumerate().take(s.max_windows.max(1)) {
                if n > 0 && w / total < s.min_share {
                    break;
                }
                cum += w / total;
                if cum >= s.snap_threshold {
                    snapped = ranked[..=n].iter().map(|(_, win)| win).collect();
                    break;
                }
            }
        }
        let apps = |wins: &[&Win]| {
            let mut v: Vec<String> = Vec::new();
            for w in wins {
                if !v.contains(&w.app) {
                    v.push(w.app.clone());
                }
            }
            v
        };
        let pct = (cum * 100.0).round();
        let mut within = FULL;
        let (mut rect, mut kind, mut reason, crop_apps) = if !snapped.is_empty() {
            let u = snapped
                .iter()
                .fold(None, |r, w| Some(union_opt(r, clip01(w.rect))))
                .unwrap();
            if area(u) <= s.big_window {
                let kind = if snapped.len() == 1 {
                    CropKind::Window
                } else {
                    CropKind::Windows
                };
                let reason = if snapped.len() == 1 {
                    format!("{} ({pct}% of the activity)", snapped[0].app)
                } else {
                    format!(
                        "{} windows: {} ({pct}% of the activity)",
                        snapped.len(),
                        apps(&snapped).join(" + ")
                    )
                };
                (u, kind, reason, apps(&snapped))
            } else {
                // The window is most of the screen: look at the activity inside it.
                let core = activity.unwrap_or(u);
                let r = intersect(self.margin(core, u), u);
                within = u;
                let reason = format!("around the activity in {}", apps(&snapped).join(" + "));
                (r, CropKind::Focus, reason, apps(&snapped))
            }
        } else {
            let core = activity.or(ink).unwrap_or(FULL);
            let r = self.margin(core, FULL);
            let names: Vec<&Win> = ranked.iter().take(2).map(|(_, w)| w).collect();
            (
                r,
                CropKind::Activity,
                "around the activity".to_owned(),
                apps(&names),
            )
        };
        if s.keep_input
            && let Some(ink) = ink
            && !contains(rect, ink)
        {
            rect = union(rect, ink);
            reason.push_str(" + input outside it");
        }
        if let Some(panel) = panels
            && !contains(rect, panel)
        {
            rect = union(rect, panel);
            reason.push_str(" + its menu");
        }
        for (win, hits) in chrome_hits.values() {
            let r = clip01(win.rect);
            let b = bbox(hits.iter().copied()).unwrap();
            let horizontal = (r[2] - r[0]) * self.sw >= (r[3] - r[1]) * self.sh;
            let cr = if horizontal {
                [b[0] - 40.0 / self.sw, r[1], b[2] + 40.0 / self.sw, r[3]]
            } else {
                [r[0], b[1] - 40.0 / self.sh, r[2], b[3] + 40.0 / self.sh]
            };
            rect = union(rect, cr);
            reason.push_str(&format!(" + {}", win.label().to_lowercase()));
        }
        let core = rect;
        rect = self.finish_in(rect, within);
        if rect == FULL {
            kind = CropKind::Full;
        }
        Some(Crop {
            rect,
            kind,
            reason,
            apps: crop_apps,
            core: Some(core),
            need: activity,
            within: (kind == CropKind::Focus).then_some(within),
            slice,
            shares: share_list,
            snapped: snapped.iter().map(|w| w.id).collect(),
            outliers,
            used,
            merged: 0,
            grown: false,
        })
    }

    fn fallback(&self, t: u64, slice: [u64; 2], previous: Option<&Crop>) -> Crop {
        let s = self.s;
        let empty = |rect, kind, reason: String, apps| Crop {
            rect,
            kind,
            reason,
            apps,
            core: None,
            need: None,
            within: None,
            slice,
            shares: Vec::new(),
            snapped: Vec::new(),
            outliers: Vec::new(),
            used: Vec::new(),
            merged: 0,
            grown: false,
        };
        let prev = previous.map(|p| {
            empty(
                p.rect,
                CropKind::Fallback,
                "talk only: the previous crop".into(),
                p.apps.clone(),
            )
        });
        let front = self.frontmost(t).and_then(|w| {
            let r = self.finish(clip01(w.rect));
            (r != FULL && area(clip01(w.rect)) <= s.big_window).then(|| {
                let mut c = empty(
                    r,
                    CropKind::Fallback,
                    format!("talk only: the frontmost window ({})", w.app),
                    vec![w.app.clone()],
                );
                c.snapped = vec![w.id];
                c
            })
        });
        let screen = || {
            empty(
                FULL,
                CropKind::Full,
                "talk only: the whole screen".into(),
                Vec::new(),
            )
        };
        match s.fallback {
            Fallback::Previous => prev.or(front).unwrap_or_else(screen),
            Fallback::Frontmost => front.or(prev).unwrap_or_else(screen),
            Fallback::Screen => screen(),
        }
    }
}

/// The crop for every keyframe of `input`.
pub fn compute(input: &Input, settings: &Settings) -> Vec<Crop> {
    let n = input.keyframes.len();
    if n == 0 {
        return Vec::new();
    }
    let ctx = Ctx {
        input,
        s: settings,
        sw: input.screen[0].max(1.0),
        sh: input.screen[1].max(1.0),
    };
    if settings.scope == Scope::Whole {
        let crop = ctx
            .crop(None)
            .unwrap_or_else(|| ctx.fallback(input.duration / 2, [0, input.duration], None));
        return vec![crop; n];
    }
    let raw: Vec<Option<Crop>> = (0..n).map(|k| ctx.crop(Some(k))).collect();
    let mut out: Vec<Crop> = Vec::with_capacity(n);
    for (k, crop) in raw.iter().enumerate() {
        let crop = match crop {
            Some(c) => c.clone(),
            None => {
                // The nearest earlier crop with activity, else the next one.
                let previous = (0..k)
                    .rev()
                    .find_map(|j| raw[j].as_ref())
                    .or_else(|| raw[k + 1..].iter().flatten().next());
                ctx.fallback(input.keyframes[k], ctx.slice(Some(k)), previous)
            }
        };
        out.push(crop);
    }
    if settings.scope == Scope::Grouped {
        let mut start = 0;
        for k in 1..=n {
            let u = out[start..k]
                .iter()
                .fold(None, |r, c| Some(union_opt(r, c.rect)))
                .unwrap();
            if k < n && iou(u, out[k].rect) >= settings.merge_iou {
                continue;
            }
            if k - start > 1 {
                let merged = ctx.shape(u);
                let merged = if area(merged) >= settings.full_above {
                    FULL
                } else {
                    merged
                };
                for c in &mut out[start..k] {
                    c.rect = merged;
                    c.merged = k - start;
                }
            }
            start = k;
        }
        if settings.zoom_step < 6.0 {
            for _ in 0..2 {
                for k in 1..n {
                    let (a, b) = (area(out[k - 1].rect), area(out[k].rect));
                    let small = if a < b { k - 1 } else { k };
                    let ratio = a.max(b) / a.min(b).max(1e-6);
                    if ratio <= settings.zoom_step + 1e-4 {
                        continue;
                    }
                    let f = (ratio / settings.zoom_step).sqrt();
                    let r = out[small].rect;
                    let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
                    let (hw, hh) = ((r[2] - r[0]) * f / 2.0, (r[3] - r[1]) * f / 2.0);
                    let grown = ctx.shape([cx - hw, cy - hh, cx + hw, cy + hh]);
                    out[small].rect = if area(grown) >= settings.full_above {
                        FULL
                    } else {
                        grown
                    };
                    out[small].grown = true;
                }
            }
        }
    }
    for (k, c) in out.iter_mut().enumerate() {
        let loose = matches!(c.kind, CropKind::Focus | CropKind::Activity);
        if loose
            && c.rect != FULL
            && let Some(Some(content)) = input.content.get(k)
        {
            let r = c.rect;
            let mid = [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0];
            let keep = c
                .need
                .map_or([mid[0], mid[1], mid[0], mid[1]], |n| intersect(n, r));
            let mut snapped = ctx.snap(c.rect, content, keep);
            if let Some(grown) = ctx.reach_content(snapped, content, keep) {
                snapped = ctx.snap(grown, content, keep);
            }
            let snapped = ctx.finish_lines(snapped, content, c.within.unwrap_or(FULL));
            c.rect = if area(snapped) >= settings.full_above {
                FULL
            } else {
                snapped
            };
        }
        if c.rect == FULL {
            c.kind = CropKind::Full;
        }
    }
    out
}

/// The summary's name for what the capture was cropped to (no window titles).
pub fn label(crops: &[Crop]) -> Option<String> {
    let cropped: Vec<&Crop> = crops.iter().filter(|c| c.kind != CropKind::Full).collect();
    if cropped.is_empty() {
        return None;
    }
    let mut apps: Vec<&str> = Vec::new();
    for c in &cropped {
        for a in &c.apps {
            if !apps.contains(&a.as_str()) {
                apps.push(a);
            }
        }
    }
    let focus = cropped
        .iter()
        .all(|c| matches!(c.kind, CropKind::Focus | CropKind::Activity));
    Some(match (apps.is_empty(), focus) {
        (true, _) => "the activity".to_owned(),
        (false, true) => format!("the activity in {}", apps.join(" + ")),
        (false, false) => apps.join(" + "),
    })
}

// ---------------------------------------------------------------------------------------------
// Applying a crop
// ---------------------------------------------------------------------------------------------

/// The pixel rect `[x, y, w, h]` of `r` in a `width`×`height` frame.
pub fn pixel_rect(r: Rect, width: u32, height: u32) -> [u32; 4] {
    let (w, h) = (width as f32, height as f32);
    // A hair of tolerance so 0.6 × 100 = 60.000004 doesn't take an extra column.
    const E: f32 = 1e-3;
    let x0 = (r[0] * w + E).floor().clamp(0.0, w - 1.0) as u32;
    let y0 = (r[1] * h + E).floor().clamp(0.0, h - 1.0) as u32;
    let x1 = (r[2] * w - E).ceil().clamp(x0 as f32 + 1.0, w) as u32;
    let y1 = (r[3] * h - E).ceil().clamp(y0 as f32 + 1.0, h) as u32;
    [x0, y0, x1 - x0, y1 - y0]
}

pub fn crop_frame(frame: &RgbaFrame, r: Rect) -> RgbaFrame {
    let [x, y, w, h] = pixel_rect(r, frame.width, frame.height);
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for row in y..y + h {
        let start = (row as usize * frame.width as usize + x as usize) * 4;
        rgba.extend_from_slice(&frame.rgba[start..start + w as usize * 4]);
    }
    RgbaFrame::new(frame.t_ms, w, h, rgba)
}

/// Strokes, marks, events and the keyframe moved into the crop's coordinates, so annotating the
/// cropped frame draws the same ink in the same place.
pub struct CropView {
    pub strokes: Vec<Stroke>,
    pub marks: Vec<Mark>,
    pub events: Vec<InteractionEvent>,
    pub keyframe: Keyframe,
}

pub fn crop_view(
    r: Rect,
    keyframe: &Keyframe,
    strokes: &[Stroke],
    marks: &[Mark],
    events: &[InteractionEvent],
) -> CropView {
    let (w, h) = ((r[2] - r[0]).max(1e-6), (r[3] - r[1]).max(1e-6));
    let fx = |x: f32| (x - r[0]) / w;
    let fy = |y: f32| (y - r[1]) / h;
    let strokes = strokes
        .iter()
        .map(|s| {
            let mut s = s.clone();
            for c in s.pts.as_chunks_mut::<3>().0 {
                c[0] = (fx(c[0] as f32 / SCALE as f32) * SCALE as f32).round() as i32;
                c[1] = (fy(c[1] as f32 / SCALE as f32) * SCALE as f32).round() as i32;
            }
            s.fw = ((s.fw as f32) * w).round().max(1.0) as u32;
            s.fh = ((s.fh as f32) * h).round().max(1.0) as u32;
            s
        })
        .collect();
    let marks = marks
        .iter()
        .map(|m| {
            let mut m = m.clone();
            let b = m.box_norm;
            m.box_norm = [fx(b[0]), fy(b[1]), fx(b[2]), fy(b[3])];
            m.from = m.from.map(|[x, y]| [fx(x), fy(y)]);
            m.to = m.to.map(|[x, y]| [fx(x), fy(y)]);
            m
        })
        .collect();
    let events = events
        .iter()
        .map(|e| {
            let mut e = e.clone();
            match &mut e {
                InteractionEvent::Click { x, y, .. }
                | InteractionEvent::Scroll { x, y, .. }
                | InteractionEvent::Point { x, y, .. } => {
                    *x = fx(*x);
                    *y = fy(*y);
                }
                InteractionEvent::Drag {
                    x0,
                    y0,
                    x1,
                    y1,
                    path,
                    ..
                } => {
                    *x0 = fx(*x0);
                    *y0 = fy(*y0);
                    *x1 = fx(*x1);
                    *y1 = fy(*y1);
                    for p in path.iter_mut() {
                        *p = [fx(p[0]), fy(p[1])];
                    }
                }
                _ => {}
            }
            e
        })
        .collect();
    let mut keyframe = keyframe.clone();
    keyframe.cursor = keyframe.cursor.map(|[x, y]| [fx(x), fy(y)]);
    CropView {
        strokes,
        marks,
        events,
        keyframe,
    }
}

/// Coarse regions that differ between two frames (of any sizes, compared by position), as
/// normalized `[x0, y0, x1, y1]` boxes, largest first.
pub fn change_regions(prev: &RgbaFrame, cur: &RgbaFrame) -> Vec<Rect> {
    const COLS: usize = 64;
    const SAMPLES: usize = 8;
    if prev.width < COLS as u32 || cur.width < COLS as u32 || prev.height < 8 || cur.height < 8 {
        return Vec::new();
    }
    let (w, h) = (cur.width as usize, cur.height as usize);
    let cell = w / COLS;
    let rows = (h / cell).max(1);
    let luma = |f: &RgbaFrame, nx: f32, ny: f32| {
        let x = ((nx * f.width as f32) as usize).min(f.width as usize - 1);
        let y = ((ny * f.height as f32) as usize).min(f.height as usize - 1);
        let i = (y * f.width as usize + x) * 4;
        let p = &f.rgba[i..i + 3];
        (u32::from(p[0]) * 3 + u32::from(p[1]) * 6 + u32::from(p[2])) / 10
    };
    let mut changed = vec![false; COLS * rows];
    for row in 0..rows {
        for col in 0..COLS {
            let mut diff = 0_u64;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let nx = (col as f32 + (sx as f32 + 0.5) / SAMPLES as f32) / COLS as f32;
                    let ny = (row as f32 + (sy as f32 + 0.5) / SAMPLES as f32) / rows as f32;
                    diff += u64::from(luma(prev, nx, ny).abs_diff(luma(cur, nx, ny)));
                }
            }
            let n = (SAMPLES * SAMPLES) as u64;
            // Scrawl's keyframe badge sits in a top corner and differs on every frame.
            let badge =
                row * cell < h * 6 / 100 && !(COLS * 6 / 100..COLS * 94 / 100).contains(&col);
            changed[row * COLS + col] = !badge && diff / n >= 10;
        }
    }
    // Connected changed cells (8-neighbour) become one box; lone cells are noise.
    let mut seen = vec![false; changed.len()];
    let mut boxes = Vec::new();
    for start in 0..changed.len() {
        if !changed[start] || seen[start] {
            continue;
        }
        let mut stack = vec![start];
        seen[start] = true;
        let (mut c0, mut r0, mut c1, mut r1, mut count) = (COLS, rows, 0, 0, 0);
        while let Some(i) = stack.pop() {
            let (r, c) = (i / COLS, i % COLS);
            (c0, r0, c1, r1) = (c0.min(c), r0.min(r), c1.max(c), r1.max(r));
            count += 1;
            for dr in -1_i32..=1 {
                for dc in -1_i32..=1 {
                    let (nr, nc) = (r as i32 + dr, c as i32 + dc);
                    if nr < 0 || nc < 0 || nr >= rows as i32 || nc >= COLS as i32 {
                        continue;
                    }
                    let j = nr as usize * COLS + nc as usize;
                    if changed[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        if count >= 2 {
            boxes.push((
                (c1 - c0 + 1) * (r1 - r0 + 1),
                [
                    c0 as f32 / COLS as f32,
                    r0 as f32 / rows as f32,
                    (c1 + 1) as f32 / COLS as f32,
                    (r1 + 1) as f32 / rows as f32,
                ],
            ));
        }
    }
    boxes.sort_by_key(|b| std::cmp::Reverse(b.0));
    boxes.into_iter().take(8).map(|(_, b)| b).collect()
}

/// Keyframes ready to write: annotated, and cropped when the capture is a Screen capture and the
/// crop is worth it. The second value is the info for `scrawl.json` and the full annotated
/// frames (for `frames-full/`), present only when something was cropped.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn annotate_and_crop(
    selected: Vec<(Keyframe, RgbaFrame)>,
    baseline: Option<&RgbaFrame>,
    settings: Option<&Settings>,
    target: &crate::model::TargetInfo,
    duration_ms: u64,
    strokes: &[Stroke],
    marks: &[Mark],
    events: &[InteractionEvent],
    windows: &[WindowSnapshot],
    cursor: &[CursorSample],
) -> (
    Vec<(Keyframe, RgbaFrame)>,
    Option<(crate::model::AutocropInfo, Vec<(Keyframe, RgbaFrame)>)>,
) {
    use crate::render::annotate_keyframe_marks_in_place;
    let plan = settings
        .filter(|_| target.mode == crate::model::CaptureMode::Display && !selected.is_empty())
        .and_then(|settings| {
            let changes: Vec<Vec<Rect>> = (0..selected.len())
                .map(|i| match (i, baseline) {
                    (0, Some(first)) if selected[0].0.t > first.t_ms => {
                        change_regions(first, &selected[0].1)
                    }
                    (0, _) => Vec::new(),
                    _ => change_regions(&selected[i - 1].1, &selected[i].1),
                })
                .collect();
            let times: Vec<u64> = selected.iter().map(|(k, _)| k.t).collect();
            let mut input = Input::new(
                target.frame,
                duration_ms,
                &times,
                strokes,
                events,
                windows,
                cursor,
                &changes,
            );
            input.pin_ink(selected.iter().map(|(k, _)| k), strokes, marks);
            let screen = input.screen;
            input.content = std::thread::scope(|scope| {
                let workers: Vec<_> = selected
                    .iter()
                    .map(|(_, frame)| scope.spawn(move || Content::from_frame(frame, screen)))
                    .collect();
                workers.into_iter().map(|w| w.join().ok()).collect()
            });
            // A crop bug must never cost the capture: fall back to the full frames.
            let crops = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                compute(&input, settings)
            }))
            .ok()?;
            crops
                .iter()
                .any(|c| c.kind != CropKind::Full)
                .then_some((crops, changes))
        });
    let Some((crops, changes)) = plan else {
        let frames = std::thread::scope(|scope| {
            let workers: Vec<_> = selected
                .into_iter()
                .map(|(kf, mut frame)| {
                    scope.spawn(move || {
                        annotate_keyframe_marks_in_place(&mut frame, &kf, strokes, marks, events);
                        (kf, frame)
                    })
                })
                .collect();
            workers.into_iter().map(|w| w.join().unwrap()).collect()
        });
        return (frames, None);
    };
    let mut cropped = Vec::with_capacity(selected.len());
    let mut full = Vec::with_capacity(selected.len());
    let mut infos = Vec::with_capacity(selected.len());
    let (w, h) = (target.frame.width, target.frame.height);
    let results: Vec<_> = std::thread::scope(|scope| {
        let workers: Vec<_> = selected
            .into_iter()
            .zip(&crops)
            .map(|((kf, mut raw), crop)| {
                scope.spawn(move || {
                    let out = if crop.kind == CropKind::Full {
                        annotate_keyframe_marks_in_place(&mut raw, &kf, strokes, marks, events);
                        raw.clone()
                    } else {
                        let mut out = crop_frame(&raw, crop.rect);
                        let view = crop_view(crop.rect, &kf, strokes, marks, events);
                        annotate_keyframe_marks_in_place(
                            &mut out,
                            &view.keyframe,
                            &view.strokes,
                            &view.marks,
                            &view.events,
                        );
                        annotate_keyframe_marks_in_place(&mut raw, &kf, strokes, marks, events);
                        out
                    };
                    let r = crop.rect.map(f64::from);
                    let crop_info = crate::model::KeyframeCrop {
                        index: kf.index,
                        x: (r[0] * w).round(),
                        y: (r[1] * h).round(),
                        width: ((r[2] - r[0]) * w).round(),
                        height: ((r[3] - r[1]) * h).round(),
                        kind: crop.kind,
                        reason: crop.reason.clone(),
                        apps: crop.apps.clone(),
                    };
                    (crop_info, (kf.clone(), out), (kf, raw))
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).collect()
    });
    for (crop_info, crop_pair, full_pair) in results {
        infos.push(crop_info);
        cropped.push(crop_pair);
        full.push(full_pair);
    }
    let info = crate::model::AutocropInfo {
        label: label(&crops).unwrap_or_else(|| "the activity".into()),
        full_frames: "frames-full".into(),
        full_contact_sheet: "contact-sheet-full.png".into(),
        crops: infos,
        changes,
    };
    (cropped, Some((info, full)))
}

/// Writes the full keyframes and contact sheet that sit beside a cropped bundle's.
pub fn write_full_frames(
    dir: &std::path::Path,
    info: &crate::model::AutocropInfo,
    frames: &[(Keyframe, RgbaFrame)],
    sheet: &RgbaFrame,
) -> anyhow::Result<()> {
    let out = dir.join(&info.full_frames);
    std::fs::create_dir_all(&out)?;
    std::thread::scope(|scope| -> anyhow::Result<()> {
        let workers: Vec<_> = frames
            .iter()
            .map(|(kf, frame)| {
                let out = &out;
                scope.spawn(move || -> anyhow::Result<()> {
                    let name = std::path::Path::new(&kf.file)
                        .file_name()
                        .unwrap_or_default();
                    crate::render::save_png(frame, &out.join(name))
                })
            })
            .collect();
        crate::render::save_png(sheet, &dir.join(&info.full_contact_sheet))?;
        for worker in workers {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("PNG encoding worker panicked"))??;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CaptureMode, KeyframeReason, TargetInfo};
    use crate::windows::WindowEntry;

    const FRAME: TargetFrame = TargetFrame {
        x: 0.0,
        y: 0.0,
        width: 1728.0,
        height: 1117.0,
        scale: 2.0,
    };

    fn solid(w: u32, h: u32, v: u8) -> RgbaFrame {
        RgbaFrame::new(0, w, h, [v, v, v, 255].repeat((w * h) as usize))
    }

    fn keyframe(index: usize, t: u64) -> Keyframe {
        Keyframe {
            index,
            t,
            reason: KeyframeReason::Click,
            file: format!("frames/frame-{index:02}.png"),
            event: String::new(),
            said: None,
            mark_id: None,
            cursor: Some([0.5, 0.5]),
        }
    }

    #[test]
    fn grouped_ink_belongs_to_its_keyframe_alone() {
        let stroke = |t0: u64, x: i32| Stroke {
            t0,
            t1: t0 + 100,
            fw: 1000,
            fh: 800,
            on: None,
            pin: None,
            laser: false,
            pts: vec![x, 5000, 0, x + 500, 5000, 100],
        };
        let strokes = [stroke(1_000, 1000), stroke(9_000, 8000)];
        let mark = |id: &str, stroke: usize| Mark {
            id: id.into(),
            strokes: vec![stroke],
            t0: strokes[stroke].t0,
            t1: strokes[stroke].t1,
            box_norm: [0.0, 0.0, 1.0, 1.0],
            shape: crate::model::Shape::Line,
            from: None,
            to: None,
            pin: None,
            on: None,
            description: String::new(),
            frame: Some("frames/frame-02.png".into()),
        };
        let keyframes = [keyframe(1, 500), keyframe(2, 9_100)];
        let mut input = Input::new(FRAME, 10_000, &[500, 9_100], &strokes, &[], &[], &[], &[]);
        let before = input.items.len();
        input.pin_ink(&keyframes, &strokes, &[mark("m1", 0), mark("m2", 1)]);
        assert_eq!(input.items.len(), before);
        let ink: Vec<_> = input
            .items
            .iter()
            .filter(|item| item.kind == Kind::Stroke)
            .map(|item| item.keyframe)
            .collect();
        assert_eq!(ink, [Some(1), Some(1)]);
    }

    fn window(id: u32, app: &str, bounds: [i32; 4], frontmost: bool) -> WindowEntry {
        WindowEntry {
            id,
            app: app.into(),
            bundle_id: None,
            regular: true,
            title: "Secret title".into(),
            bounds,
            layer: 0,
            frontmost,
        }
    }

    fn click(t: u64, x: f32, y: f32) -> InteractionEvent {
        InteractionEvent::Click {
            t,
            x,
            y,
            button: None,
            label: None,
            role: None,
        }
    }

    #[test]
    fn crop_view_moves_ink_and_clicks_into_the_crop() {
        let r = [0.25, 0.5, 0.75, 1.0];
        let stroke = Stroke {
            t0: 0,
            t1: 10,
            fw: 1000,
            fh: 800,
            on: None,
            pin: None,
            laser: false,
            pts: vec![(0.5 * SCALE as f32) as i32, (0.75 * SCALE as f32) as i32, 0],
        };
        let view = crop_view(r, &keyframe(1, 0), &[stroke], &[], &[click(0, 0.25, 1.0)]);
        let p = view.strokes[0].timed_points()[0];
        assert!(
            (p.0 - 0.5).abs() < 1e-3 && (p.1 - 0.5).abs() < 1e-3,
            "{p:?}"
        );
        assert_eq!((view.strokes[0].fw, view.strokes[0].fh), (500, 400));
        assert!(
            matches!(view.events[0], InteractionEvent::Click { x, y, .. } if x == 0.0 && y == 1.0)
        );
        assert_eq!(view.keyframe.cursor, Some([0.5, 0.0]));
    }

    #[test]
    fn crop_frame_takes_the_pixels_inside_the_rect() {
        let mut frame = solid(100, 50, 0);
        for y in 10..20 {
            for x in 40..60 {
                frame.rgba[(y * 100 + x) * 4] = 255;
            }
        }
        let out = crop_frame(&frame, [0.4, 0.2, 0.6, 0.4]);
        assert_eq!((out.width, out.height), (20, 10));
        assert!(out.rgba.chunks(4).all(|p| p[0] == 255));
    }

    #[test]
    fn change_regions_compare_frames_of_different_sizes_by_position() {
        let small = solid(640, 414, 30);
        let mut big = solid(2560, 1654, 30);
        for y in 400..800 {
            for x in 1280..1920 {
                let i = (y * 2560 + x) * 4;
                big.rgba[i..i + 3].copy_from_slice(&[230, 230, 230]);
            }
        }
        let regions = change_regions(&small, &big);
        assert_eq!(regions.len(), 1, "{regions:?}");
        let r = regions[0];
        assert!(
            r[0] <= 0.5 && r[2] >= 0.75 && r[1] <= 0.25 && r[3] >= 0.48,
            "{r:?}"
        );
        assert!(r[0] >= 0.45 && r[2] <= 0.8, "{r:?}");
        assert!(change_regions(&small, &solid(1280, 828, 30)).is_empty());
    }

    #[test]
    fn content_snapping_moves_an_edge_out_of_a_line_of_text() {
        // A text block from x 0.30 to 0.70 of the screen; a crop whose right edge cuts through
        // it moves out to the white space after it.
        let mut frame = solid(1728, 1117, 255);
        for y in (300..600).step_by(12) {
            for x in 518..1210 {
                if x % 7 < 4 {
                    for dy in 0..4 {
                        let i = ((y + dy) * 1728 + x) * 4;
                        frame.rgba[i..i + 3].copy_from_slice(&[20, 20, 20]);
                    }
                }
            }
        }
        let content = Content::from_frame(&frame, [1728.0, 1117.0]);
        let input = Input {
            screen: [1728.0, 1117.0],
            ..Default::default()
        };
        let s = Settings::default();
        let ctx = Ctx {
            input: &input,
            s: &s,
            sw: 1728.0,
            sh: 1117.0,
        };
        let r = ctx.snap([0.28, 0.25, 0.6, 0.55], &content, [0.4, 0.3, 0.5, 0.4]);
        assert!(r[2] >= 0.70 && r[2] < 0.76, "right edge {r:?}");
        assert!(r[0] <= 0.30 && r[0] > 0.26, "left edge {r:?}");
    }

    /// A Linux capture without a movie: 480x270 candidates of a 1920x1080 screen, fewer
    /// pixels than 3 pt cells, read past the frame's last row.
    #[test]
    fn content_of_a_frame_smaller_than_its_cells() {
        let frame = solid(480, 270, 255);
        let content = Content::from_frame(&frame, [1920.0, 1080.0]);
        assert_eq!((content.cols, content.rows), (640, 360));
    }

    /// Kit's capture: a tall Focus crop whose right edge ran through a ragged paragraph,
    /// cutting "…instead of 5 GB, a" mid-line, because each column there is only a few
    /// percent text.
    #[test]
    fn crop_edges_finish_the_lines_they_cross() {
        let mut frame = RgbaFrame::new(0, 1728, 1117, vec![30; 1728 * 1117 * 4]);
        let mut fill = |x0: usize, x1: usize, y0: usize, h: usize| {
            for y in y0..y0 + h {
                for x in x0..x1 {
                    // Letters with a 4 px space between words.
                    if x % 48 < 44 {
                        let i = (y * 1728 + x) * 4;
                        frame.rgba[i..i + 3].copy_from_slice(&[220, 220, 220]);
                    }
                }
            }
        };
        // Paragraph lines from x 646 to between 1180 and 1300, every 26 pt.
        for (i, end) in [1300, 1250, 1290, 1180, 1300, 1270, 1240, 1300]
            .iter()
            .enumerate()
        {
            fill(646, *end, 120 + i * 26, 12);
        }
        // A rule across the whole window.
        fill(310, 1700, 400, 2);
        let content = Content::from_frame(&frame, [1728.0, 1117.0]);
        let input = Input {
            screen: [1728.0, 1117.0],
            ..Default::default()
        };
        let s = Settings::default();
        let ctx = Ctx {
            input: &input,
            s: &s,
            sw: 1728.0,
            sh: 1117.0,
        };
        let crop = [
            621.0 / 1728.0,
            27.0 / 1117.0,
            1248.0 / 1728.0,
            832.0 / 1117.0,
        ];
        let window = [310.0 / 1728.0, 38.0 / 1117.0, 1990.0 / 1728.0, 1.0];
        let r = ctx.finish_lines(crop, &content, window);
        let right = r[2] * 1728.0;
        assert!((1300.0..1330.0).contains(&right), "right edge {right}");
        assert_eq!(r[0], crop[0], "nothing crosses the left edge");
        assert_eq!((r[1], r[3]), (crop[1], crop[3]));
        // Never past the window; lines that run to (or nearly to) its edge are left alone,
        // like the rule.
        let narrow = [310.0 / 1728.0, 0.0, 1270.0 / 1728.0, 1.0];
        let right = ctx.finish_lines(crop, &content, narrow)[2] * 1728.0;
        assert!((1245.0..=1270.0).contains(&right), "{right}");
    }

    #[test]
    fn config_turns_it_off_or_overrides_settings() {
        assert_eq!(Settings::from_config(None), Some(Settings::default()));
        assert_eq!(
            Settings::from_config(Some(&serde_json::json!(true))),
            Some(Settings::default())
        );
        assert_eq!(Settings::from_config(Some(&serde_json::json!(false))), None);
        let s = Settings::from_config(Some(
            &serde_json::json!({ "padding": 32, "scope": "whole" }),
        ))
        .unwrap();
        assert_eq!((s.padding, s.scope), (32.0, Scope::Whole));
        assert_eq!(s.min_w, Settings::default().min_w);
    }

    #[test]
    fn screen_captures_crop_to_the_window_used_and_keep_the_full_frames() {
        let target = TargetInfo {
            mode: CaptureMode::Display,
            app: None,
            bundle_id: None,
            window_title: None,
            window_id: None,
            display_id: 1,
            frame: FRAME,
        };
        let windows = vec![WindowSnapshot {
            t: 0,
            windows: vec![
                window(1, "Finder", [100, 100, 600, 400], true),
                window(2, "Safari", [900, 100, 700, 900], false),
            ],
        }];
        let events = vec![click(1_000, 0.25, 0.3), click(1_300, 0.3, 0.35)];
        let selected = vec![(keyframe(1, 1_000), solid(1728, 1117, 40))];
        let (frames, cropped) = annotate_and_crop(
            selected.clone(),
            None,
            Some(&Settings::default()),
            &target,
            2_000,
            &[],
            &[],
            &events,
            &windows,
            &[],
        );
        let (info, full) = cropped.expect("cropped");
        assert_eq!((frames[0].1.width, frames[0].1.height), (600, 400));
        assert_eq!((full[0].1.width, full[0].1.height), (1728, 1117));
        let c = &info.crops[0];
        assert_eq!((c.x, c.y, c.width, c.height), (100.0, 100.0, 600.0, 400.0));
        assert_eq!((c.kind, info.label.as_str()), (CropKind::Window, "Finder"));
        assert!(!serde_json::to_string(&info).unwrap().contains("Secret"));

        // Other capture modes and auto-crop off leave the frames alone.
        for (mode, settings) in [
            (CaptureMode::Window, Some(Settings::default())),
            (CaptureMode::Display, None),
        ] {
            let target = TargetInfo {
                mode,
                ..target.clone()
            };
            let (frames, cropped) = annotate_and_crop(
                selected.clone(),
                None,
                settings.as_ref(),
                &target,
                2_000,
                &[],
                &[],
                &events,
                &windows,
                &[],
            );
            assert!(cropped.is_none());
            assert_eq!(frames[0].1.width, 1728);
        }
    }
}
