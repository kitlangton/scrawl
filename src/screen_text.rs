//! What text a ⇧ mark or a drag lands on, read off the recording with Apple Vision
//! (`scrawl-helper read-text`) when the Accessibility tree (`crate::ax`) and the dev page
//! (`crate::page`) named nothing there: the `agentContext` experiment. Many apps (terminals,
//! canvas-drawn and web apps) expose only bare groups to Accessibility, but their pixels are in
//! the recording, which never shows Scrawl's ink. Each target is read from the frame where it
//! ends (a mark's last stroke, a drag's release, while a selection is still highlighted),
//! cropped to the area around it, and the text is picked by the mark's shape.

use std::path::Path;
use std::time::Instant;

use serde::Deserialize;

use crate::model::{InteractionEvent, Mark, Shape, Stroke, TargetFrame};

/// The experiment this runs under.
const EXPERIMENT: &str = "agentContext";

/// Lines Vision is less sure of than this are dropped.
const MIN_CONFIDENCE: f32 = 0.3;

/// One recognised word; `bounds` is `[x0, y0, x1, y1]`, normalized to the frame (top-left).
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Word {
    pub text: String,
    #[serde(rename = "box")]
    pub bounds: [f32; 4],
}

/// One recognised line of text, with its words.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Line {
    pub text: String,
    #[serde(default = "sure")]
    pub confidence: f32,
    #[serde(rename = "box")]
    pub bounds: [f32; 4],
    #[serde(default)]
    pub words: Vec<Word>,
}

fn sure() -> f32 {
    1.0
}

/// What `scrawl-helper read-text` found in one region: the movie time it read and its lines.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Region {
    pub t: u64,
    #[serde(default)]
    pub lines: Vec<Line>,
}

/// What a label of text read off the screen starts with: like an Accessibility label it leads
/// with a role (`text “Hold ⇧ (Shift)…”`, as `button “Area”`), so an agent tells it from speech,
/// which the summary quotes bare.
const PREFIX: &str = "text “";

/// Whether a label is text read off the screen (or an Accessibility static text, which is the
/// same thing to the reader).
pub fn is_read(label: &str) -> bool {
    label.starts_with(PREFIX)
}

/// `text “…”`: the read text (one line of at most 60 characters, from `pick`) as a label.
fn label(text: &str) -> String {
    format!("{PREFIX}{text}”")
}

/// What to look for, in points (normalized positions scaled by the frame's size).
#[derive(Clone, Debug, PartialEq)]
enum Look {
    /// Inside closed ink (a circle): the lines it wholly encloses, else the words mostly inside
    /// it, else what its box covers. A line the ink runs through is its edge, not its subject.
    Inside {
        ink: Vec<[f32; 2]>,
        bounds: [f32; 4],
    },
    /// The words mostly under an open scribble or highlight's box.
    Under([f32; 4]),
    /// The words of the nearest line just above (an underline sits under what it marks).
    Above([f32; 4]),
    /// The line at a point (an arrow's tip, a pin, a tap), within `reach` line heights.
    At { point: [f32; 2], reach: f32 },
    /// The words a path crosses in reading order, from where it starts to where it ends (a drag
    /// or a straight line).
    Across(Vec<[f32; 2]>),
}

/// Normalized positions to points.
#[derive(Clone, Copy, Debug)]
struct Space {
    w: f32,
    h: f32,
}

impl Space {
    fn point(self, [x, y]: [f32; 2]) -> [f32; 2] {
        [x * self.w, y * self.h]
    }

    fn rect(self, [x0, y0, x1, y1]: [f32; 4]) -> [f32; 4] {
        [x0 * self.w, y0 * self.h, x1 * self.w, y1 * self.h]
    }

    /// Points back to a normalized region, inside the frame.
    fn normalized(self, [x0, y0, x1, y1]: [f32; 4]) -> [f32; 4] {
        [
            (x0 / self.w).clamp(0.0, 1.0),
            (y0 / self.h).clamp(0.0, 1.0),
            (x1 / self.w).clamp(0.0, 1.0),
            (y1 / self.h).clamp(0.0, 1.0),
        ]
    }

    fn lines(self, lines: Vec<Line>) -> Vec<Line> {
        lines
            .into_iter()
            .map(|line| Line {
                bounds: self.rect(line.bounds),
                words: line
                    .words
                    .into_iter()
                    .map(|w| Word {
                        bounds: self.rect(w.bounds),
                        text: w.text,
                    })
                    .collect(),
                ..line
            })
            .collect()
    }
}

/// Text found for a capture's marks and drags.
#[derive(Debug, Default, PartialEq)]
pub struct Found {
    /// The first stroke of each mark and its label.
    strokes: Vec<(usize, String)>,
    /// Each drag's event index and its label.
    drags: Vec<(usize, String)>,
    /// How long reading took.
    pub ms: u64,
}

impl Found {
    /// Puts the text on `Stroke.on` and `Drag.label`, where nothing named them yet.
    pub fn apply(&self, strokes: &mut [Stroke], events: &mut [InteractionEvent]) {
        for (i, text) in &self.strokes {
            if let Some(stroke) = strokes.get_mut(*i)
                && stroke.on.is_none()
            {
                stroke.on = Some(text.clone());
            }
        }
        for (i, text) in &self.drags {
            if let Some(InteractionEvent::Drag { label, .. }) = events.get_mut(*i)
                && label.is_none()
            {
                *label = Some(text.clone());
            }
        }
    }
}

/// Whether text is read at all: the experiment is on, on macOS (Vision). Callers check it before
/// starting any work for `read`.
pub fn enabled() -> bool {
    cfg!(target_os = "macos") && crate::handoff::experiment(EXPERIMENT)
}

/// Reads the text under the marks and drags nothing named yet from `movie` (whose clock starts
/// `offset_ms` into the capture's). `None` when there is no movie or nothing to read.
pub fn read(
    movie: Option<&Path>,
    offset_ms: u64,
    frame: &TargetFrame,
    strokes: &[Stroke],
    events: &[InteractionEvent],
) -> Option<Found> {
    let movie = movie?;
    let started = Instant::now();
    let space = match (frame.width, frame.height, strokes.first()) {
        (w, h, _) if w > 0.0 && h > 0.0 => Space {
            w: w as f32,
            h: h as f32,
        },
        (_, _, Some(s)) if s.fw > 0 && s.fh > 0 => Space {
            w: s.fw as f32,
            h: s.fh as f32,
        },
        _ => Space {
            w: 1000.0,
            h: 1000.0,
        },
    };
    let (jobs, regions) = requests(plan(strokes, events, space), space, offset_ms);
    if jobs.is_empty() {
        return None;
    }
    let mut found = Found::default();
    match crate::helper::read_text(movie, &regions) {
        Ok(read) => {
            for (job, region) in jobs.iter().zip(read) {
                let Some(text) = pick(&space.lines(region.lines), &job.look) else {
                    continue;
                };
                let label = label(&text);
                match job.into {
                    Field::Stroke(i) => found.strokes.push((i, label)),
                    Field::Drag(i) => found.drags.push((i, label)),
                }
            }
        }
        Err(error) => crate::capture::scrawl_log(format!("Screen text: {error:#}")),
    }
    found.ms = started.elapsed().as_millis() as u64;
    Some(found)
}

/// Each job's region to read, on the movie's clock; a job whose region has no area inside the
/// frame (its target lies outside it) is dropped, so it can't spoil the others.
fn requests(jobs: Vec<Job>, space: Space, offset_ms: u64) -> (Vec<Job>, Vec<(u64, [f32; 4])>) {
    jobs.into_iter()
        .filter_map(|job| {
            let r = space.normalized(region(&job.look));
            let t = job.t.saturating_sub(offset_ms);
            (r[2] > r[0] && r[3] > r[1]).then_some((job, (t, r)))
        })
        .unzip()
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Field {
    Stroke(usize),
    Drag(usize),
}

#[derive(Clone, Debug, PartialEq)]
struct Job {
    into: Field,
    look: Look,
    /// When to read, on the capture's clock.
    t: u64,
}

/// Every mark and drag nothing named yet, and where to look for its text.
fn plan(strokes: &[Stroke], events: &[InteractionEvent], space: Space) -> Vec<Job> {
    let mut jobs: Vec<Job> = crate::sketch::marks_of(strokes)
        .iter()
        .filter(|m| m.on.is_none())
        .filter_map(|mark| {
            Some(Job {
                into: Field::Stroke(*mark.strokes.first()?),
                look: look_for(mark, strokes, space)?,
                t: mark.t1,
            })
        })
        .collect();
    for (i, event) in events.iter().enumerate() {
        if let InteractionEvent::Drag {
            t1,
            x0,
            y0,
            x1,
            y1,
            label: None,
            button,
            path,
            ..
        } = event
            && button.as_deref() != Some("touch")
        {
            let points = std::iter::once([*x0, *y0])
                .chain(path.iter().copied())
                .chain(std::iter::once([*x1, *y1]))
                .map(|p| space.point(p))
                .collect();
            jobs.push(Job {
                into: Field::Drag(i),
                look: Look::Across(points),
                t: *t1,
            });
        }
    }
    jobs
}

/// Where a mark's text is, by its shape.
fn look_for(mark: &Mark, strokes: &[Stroke], space: Space) -> Option<Look> {
    let ink: Vec<[f32; 2]> = mark
        .strokes
        .iter()
        .filter_map(|&i| strokes.get(i))
        .flat_map(Stroke::normalized_points)
        .map(|p| space.point(p))
        .collect();
    if ink.is_empty() {
        return None;
    }
    let bounds = space.rect(mark.box_norm);
    Some(match mark.shape {
        Shape::Flag => return None,
        Shape::Circle => Look::Inside { ink, bounds },
        Shape::Mark if closes(&ink, bounds) => Look::Inside { ink, bounds },
        Shape::Mark => Look::Under(bounds),
        Shape::Underline => Look::Above(bounds),
        Shape::Arrow => Look::At {
            point: space.point(mark.to?),
            reach: 1.5,
        },
        Shape::Pin | Shape::Tap => Look::At {
            point: center(bounds),
            reach: 0.75,
        },
        Shape::Line => Look::Across(vec![space.point(mark.from?), space.point(mark.to?)]),
    })
}

/// Whether ink ends near where it started (within a quarter of its box's diagonal), so it
/// encloses something.
fn closes(ink: &[[f32; 2]], [x0, y0, x1, y1]: [f32; 4]) -> bool {
    let (Some(a), Some(z)) = (ink.first(), ink.last()) else {
        return false;
    };
    distance(*a, *z) <= 0.25 * (x1 - x0).hypot(y1 - y0)
}

/// The area to crop around a target, in points: enough around it that the lines it picks
/// aren't cut off at the crop's edge.
fn region(look: &Look) -> [f32; 4] {
    match look {
        Look::Inside { bounds, .. } | Look::Under(bounds) => pad(*bounds, 48.0, 24.0),
        Look::Above([x0, y0, x1, y1]) => [x0 - 24.0, y0 - 72.0, x1 + 24.0, y1 + 8.0],
        Look::At { point: [x, y], .. } => [x - 400.0, y - 40.0, x + 400.0, y + 40.0],
        Look::Across(path) => pad(crate::sketch::bounds(path), 200.0, 24.0),
    }
}

fn pad([x0, y0, x1, y1]: [f32; 4], dx: f32, dy: f32) -> [f32; 4] {
    [x0 - dx, y0 - dy, x1 + dx, y1 + dy]
}

fn center([x0, y0, x1, y1]: [f32; 4]) -> [f32; 2] {
    [(x0 + x1) / 2.0, (y0 + y1) / 2.0]
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn height(b: [f32; 4]) -> f32 {
    (b[3] - b[1]).max(1.0)
}

fn area(b: [f32; 4]) -> f32 {
    (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0)
}

fn overlap(a: [f32; 4], b: [f32; 4]) -> f32 {
    area([
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ])
}

/// Distance from a point to a box (0 inside it).
fn to_box([x, y]: [f32; 2], [x0, y0, x1, y1]: [f32; 4]) -> f32 {
    let dx = (x0 - x).max(0.0).max(x - x1);
    let dy = (y0 - y).max(0.0).max(y - y1);
    dx.hypot(dy)
}

/// Even-odd point in polygon; the polygon closes from its last point to its first.
fn inside(p: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    let mut odd = false;
    let mut j = polygon.len().wrapping_sub(1);
    for (i, a) in polygon.iter().enumerate() {
        let b = polygon[j];
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            odd = !odd;
        }
        j = i;
    }
    odd
}

/// The share of a box inside the polygon, sampled on an 8×8 grid.
fn enclosed(b: [f32; 4], polygon: &[[f32; 2]]) -> f32 {
    const N: usize = 8;
    let mut hits = 0;
    for i in 0..N {
        for j in 0..N {
            let x = b[0] + (b[2] - b[0]) * (i as f32 + 0.5) / N as f32;
            let y = b[1] + (b[3] - b[1]) * (j as f32 + 0.5) / N as f32;
            hits += usize::from(inside([x, y], polygon));
        }
    }
    hits as f32 / (N * N) as f32
}

/// Dots a secure field shows instead of its text.
fn is_mask(word: &str) -> bool {
    word.chars()
        .all(|c| matches!(c, '•' | '●' | '∙' | '·' | '*' | '⦁' | '◦'))
}

/// Lines worth reading: sure enough, with mask dots dropped, and a word for the whole line when
/// Vision gave none. Sorted in reading order (rows top to bottom, then left to right).
fn clean(lines: &[Line]) -> Vec<Line> {
    let mut out: Vec<Line> = lines
        .iter()
        .filter(|l| l.confidence >= MIN_CONFIDENCE)
        .map(|l| {
            let mut words: Vec<Word> = if l.words.is_empty() {
                vec![Word {
                    text: l.text.clone(),
                    bounds: l.bounds,
                }]
            } else {
                l.words.clone()
            };
            words.retain(|w| !is_mask(&w.text));
            words.sort_by(|a, b| a.bounds[0].total_cmp(&b.bounds[0]));
            Line { words, ..l.clone() }
        })
        .filter(|l| {
            l.words
                .iter()
                .any(|w| w.text.chars().any(char::is_alphanumeric))
        })
        .collect();
    out.sort_by(|a, b| center(a.bounds)[1].total_cmp(&center(b.bounds)[1]));
    // Lines whose middle falls inside the previous row's height share its row.
    let mut row = 0;
    let mut span = (f32::MIN, f32::MIN);
    let mut rows = Vec::with_capacity(out.len());
    for line in &out {
        let mid = center(line.bounds)[1];
        if mid > span.1 {
            row += 1;
            span = (line.bounds[1], line.bounds[3]);
        }
        rows.push(row);
    }
    let mut keyed: Vec<(usize, Line)> = rows.into_iter().zip(out).collect();
    keyed.sort_by(|(ra, a), (rb, b)| ra.cmp(rb).then(a.bounds[0].total_cmp(&b.bounds[0])));
    keyed.into_iter().map(|(_, l)| l).collect()
}

/// The text a target lands on, one line of at most 60 characters.
fn pick(lines: &[Line], look: &Look) -> Option<String> {
    let lines = clean(lines);
    let chosen: Vec<Vec<&Word>> = match look {
        Look::Inside { ink, bounds } => {
            let whole: Vec<Vec<&Word>> = lines
                .iter()
                .filter(|l| enclosed(l.bounds, ink) >= 0.9)
                .map(|l| l.words.iter().collect())
                .collect();
            if whole.iter().any(|w| !w.is_empty()) {
                whole
            } else {
                let mostly = words_where(&lines, |w| enclosed(w.bounds, ink) >= 0.5);
                if mostly.iter().any(|w| !w.is_empty()) {
                    mostly
                } else {
                    under(&lines, *bounds)
                }
            }
        }
        Look::Under(bounds) => under(&lines, *bounds),
        Look::Above(bounds) => above(&lines, *bounds),
        Look::At { point, reach } => at(&lines, *point, *reach),
        Look::Across(path) => across(&lines, path),
    };
    let text = chosen
        .iter()
        .flatten()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    crate::ax::clip(&text)
}

fn words_where<'a>(lines: &'a [Line], keep: impl Fn(&Word) -> bool) -> Vec<Vec<&'a Word>> {
    lines
        .iter()
        .map(|l| l.words.iter().filter(|w| keep(w)).collect())
        .collect()
}

fn under(lines: &[Line], bounds: [f32; 4]) -> Vec<Vec<&Word>> {
    words_where(lines, |w| overlap(w.bounds, bounds) >= 0.5 * area(w.bounds))
}

fn above(lines: &[Line], [x0, y0, x1, y1]: [f32; 4]) -> Vec<Vec<&Word>> {
    let ink = (y0 + y1) / 2.0;
    lines
        .iter()
        .filter_map(|l| {
            let gap = ink - l.bounds[3];
            let h = height(l.bounds);
            if gap < -0.5 * h || gap > 1.5 * h || center(l.bounds)[1] > ink {
                return None;
            }
            let words: Vec<&Word> = l
                .words
                .iter()
                .filter(|w| {
                    let width = (w.bounds[2] - w.bounds[0]).max(1.0);
                    (w.bounds[2].min(x1) - w.bounds[0].max(x0)) >= 0.5 * width
                })
                .collect();
            (!words.is_empty()).then_some((gap.abs(), words))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, words)| vec![words])
        .unwrap_or_default()
}

fn at(lines: &[Line], point: [f32; 2], reach: f32) -> Vec<Vec<&Word>> {
    lines
        .iter()
        .map(|l| {
            (
                to_box(point, l.bounds),
                distance(point, center(l.bounds)),
                l,
            )
        })
        .filter(|(d, _, l)| *d <= reach * height(l.bounds))
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)))
        .map(|(_, _, l)| vec![l.words.iter().collect()])
        .unwrap_or_default()
}

fn across<'a>(lines: &'a [Line], path: &[[f32; 2]]) -> Vec<Vec<&'a Word>> {
    let (Some(&first), Some(&last)) = (path.first(), path.last()) else {
        return Vec::new();
    };
    let mut heights: Vec<f32> = lines.iter().map(|l| height(l.bounds)).collect();
    heights.sort_by(f32::total_cmp);
    let h = heights.get(heights.len() / 2).copied().unwrap_or(16.0);
    // Read from whichever end comes first on the page: a selection can be dragged backwards.
    let backwards = last[1] < first[1] - 0.5 * h
        || ((last[1] - first[1]).abs() <= 0.5 * h && last[0] < first[0]);
    let (start, end) = if backwards {
        (last, first)
    } else {
        (first, last)
    };
    let [px0, _, px1, _] = crate::sketch::bounds(path);
    let (top, bottom) = (start[1].min(end[1]), start[1].max(end[1]));
    let holds = |l: &Line, p: [f32; 2]| l.bounds[1] <= p[1] && p[1] <= l.bounds[3];
    lines
        .iter()
        .filter(|l| {
            l.bounds[1] <= bottom && l.bounds[3] >= top && l.bounds[0] <= px1 && l.bounds[2] >= px0
        })
        .map(|l| {
            let (from_start, to_end) = (holds(l, start), holds(l, end));
            l.words
                .iter()
                .filter(|w| !from_start || w.bounds[2] > start[0])
                .filter(|w| !to_end || w.bounds[0] < end[0])
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SCALE;

    /// A line of words laid out left to right from `x`, 10 points per character plus a space.
    fn line(text: &str, x: f32, y: f32) -> Line {
        let mut words = Vec::new();
        let mut left = x;
        for word in text.split(' ') {
            let right = left + 10.0 * word.chars().count() as f32;
            words.push(Word {
                text: word.into(),
                bounds: [left, y, right, y + 16.0],
            });
            left = right + 10.0;
        }
        Line {
            text: text.into(),
            confidence: 1.0,
            bounds: [x, y, left - 10.0, y + 16.0],
            words,
        }
    }

    fn page() -> Vec<Line> {
        vec![
            line("Title of the page", 100.0, 40.0),
            line("Hold Shift and move", 100.0, 100.0),
            line("the mouse", 100.0, 120.0),
            line("Draws a red line", 400.0, 100.0),
            line("Click normally", 100.0, 200.0),
        ]
    }

    /// A rough ellipse around `[x0, y0, x1, y1]`.
    fn ring([x0, y0, x1, y1]: [f32; 4]) -> Vec<[f32; 2]> {
        let (cx, cy, rx, ry) = (
            (x0 + x1) / 2.0,
            (y0 + y1) / 2.0,
            (x1 - x0) / 2.0,
            (y1 - y0) / 2.0,
        );
        (0..=36)
            .map(|i| {
                let a = i as f32 / 36.0 * std::f32::consts::TAU;
                [cx + rx * a.cos(), cy + ry * a.sin()]
            })
            .collect()
    }

    #[test]
    fn a_circle_names_the_lines_it_encloses_not_the_ones_its_edge_crosses() {
        // The ring runs through "Title of the page" and around both lines below it.
        let ink = ring([80.0, 50.0, 320.0, 150.0]);
        let look = Look::Inside {
            bounds: crate::sketch::bounds(&ink),
            ink,
        };
        assert_eq!(
            pick(&page(), &look).as_deref(),
            Some("Hold Shift and move the mouse")
        );
        // Around one word of a line: that word.
        let ink = ring([138.0, 88.0, 212.0, 128.0]);
        let look = Look::Inside {
            bounds: crate::sketch::bounds(&ink),
            ink,
        };
        assert_eq!(pick(&page(), &look).as_deref(), Some("Shift"));
    }

    #[test]
    fn an_open_mark_names_the_words_under_its_box() {
        let look = Look::Under([395.0, 95.0, 515.0, 118.0]);
        assert_eq!(pick(&page(), &look).as_deref(), Some("Draws a red"));
    }

    #[test]
    fn an_underline_names_the_words_just_above_it() {
        let look = Look::Above([95.0, 140.0, 190.0, 143.0]);
        assert_eq!(pick(&page(), &look).as_deref(), Some("the mouse"));
        // Nothing within reach above it.
        let look = Look::Above([95.0, 300.0, 190.0, 303.0]);
        assert_eq!(pick(&page(), &look), None);
    }

    #[test]
    fn an_arrow_names_the_line_at_its_tip_and_a_pin_the_line_under_it() {
        let tip = Look::At {
            point: [395.0, 110.0],
            reach: 1.5,
        };
        assert_eq!(pick(&page(), &tip).as_deref(), Some("Draws a red line"));
        let far = Look::At {
            point: [395.0, 170.0],
            reach: 1.5,
        };
        assert_eq!(pick(&page(), &far), None);
        let pin = Look::At {
            point: [150.0, 208.0],
            reach: 0.75,
        };
        assert_eq!(pick(&page(), &pin).as_deref(), Some("Click normally"));
    }

    #[test]
    fn a_drag_names_the_words_it_crosses_from_start_to_end() {
        // From "Shift" on the first line to the end of "the" on the next.
        let path = vec![[145.0, 108.0], [130.0, 128.0]];
        let words = Some("Shift and move the");
        assert_eq!(pick(&page(), &Look::Across(path.clone())).as_deref(), words);
        // Dragged backwards, the same words in reading order.
        let back: Vec<_> = path.into_iter().rev().collect();
        assert_eq!(pick(&page(), &Look::Across(back)).as_deref(), words);
        // Along one line, only the words between its ends.
        let path = vec![[395.0, 108.0], [505.0, 108.0]];
        assert_eq!(
            pick(&page(), &Look::Across(path)).as_deref(),
            Some("Draws a red")
        );
        // Over empty space.
        let path = vec![[100.0, 170.0], [300.0, 175.0]];
        assert_eq!(pick(&page(), &Look::Across(path)), None);
    }

    #[test]
    fn text_is_one_short_line_and_mask_dots_are_never_read() {
        let long = line(&"word ".repeat(20).trim_end().to_owned(), 0.0, 0.0);
        let text = pick(&[long], &Look::Under([0.0, 0.0, 2000.0, 20.0])).unwrap();
        assert_eq!(text.chars().count(), 60);
        assert!(text.ends_with('…'));
        // The 60 characters are the quoted text; the role and quotes come on top.
        let labelled = label(&text);
        assert!(labelled.starts_with("text “word word") && labelled.ends_with("…”"));
        assert!(is_read(&labelled));
        assert_eq!(labelled.chars().count(), "text “”".chars().count() + 60);
        let field = vec![line("Password ••••••••", 0.0, 0.0), line("••••", 0.0, 30.0)];
        assert_eq!(
            pick(&field, &Look::Under([0.0, 0.0, 400.0, 60.0])).as_deref(),
            Some("Password")
        );
        let unsure = Line {
            confidence: 0.1,
            ..line("garbage", 0.0, 0.0)
        };
        assert_eq!(pick(&[unsure], &Look::Under([0.0, 0.0, 400.0, 60.0])), None);
    }

    fn stroke(t0: u64, points: &[[f32; 2]]) -> Stroke {
        Stroke {
            t0,
            t1: t0 + 500,
            fw: 1728,
            fh: 1117,
            on: None,
            pin: None,
            laser: false,
            pts: points
                .iter()
                .enumerate()
                .flat_map(|(i, p)| {
                    [
                        (p[0] * SCALE as f32) as i32,
                        (p[1] * SCALE as f32) as i32,
                        i as i32 * 10,
                    ]
                })
                .collect(),
        }
    }

    const SPACE: Space = Space {
        w: 1728.0,
        h: 1117.0,
    };

    #[test]
    fn each_shape_looks_in_its_own_place() {
        let mut mark = Mark {
            id: "m1".into(),
            strokes: vec![0],
            t0: 0,
            t1: 500,
            box_norm: [0.1, 0.1, 0.3, 0.2],
            shape: Shape::Circle,
            from: Some([0.1, 0.1]),
            to: Some([0.3, 0.2]),
            pin: None,
            on: None,
            description: String::new(),
            frame: None,
        };
        let strokes = [stroke(0, &[[0.1, 0.1], [0.3, 0.2]])];
        let look = |mark: &Mark| look_for(mark, &strokes, SPACE).unwrap();
        assert!(matches!(look(&mark), Look::Inside { .. }));
        mark.shape = Shape::Underline;
        assert_eq!(look(&mark), Look::Above(SPACE.rect(mark.box_norm)));
        mark.shape = Shape::Arrow;
        assert_eq!(
            look(&mark),
            Look::At {
                point: SPACE.point([0.3, 0.2]),
                reach: 1.5
            }
        );
        mark.shape = Shape::Line;
        assert_eq!(
            look(&mark),
            Look::Across(vec![SPACE.point([0.1, 0.1]), SPACE.point([0.3, 0.2])])
        );
        mark.shape = Shape::Pin;
        assert!(matches!(look(&mark), Look::At { reach: 0.75, .. }));
        // An open freeform mark covers; one that closes on itself encloses.
        mark.shape = Shape::Mark;
        assert_eq!(look(&mark), Look::Under(SPACE.rect(mark.box_norm)));
        let closed = [stroke(
            0,
            &[[0.1, 0.1], [0.3, 0.1], [0.3, 0.2], [0.1, 0.2], [0.11, 0.11]],
        )];
        assert!(matches!(
            look_for(&mark, &closed, SPACE),
            Some(Look::Inside { .. })
        ));
        mark.shape = Shape::Flag;
        assert_eq!(look_for(&mark, &strokes, SPACE), None);
    }

    /// Stroke 1 of the real capture `scrawl-1792536608591-641c-2`: a ring around the table cell
    /// “Hold ⇧ (Shift) and move the mouse” that cuts through the “Do this” header above it.
    const RING: [i32; 138] = [
        3617, 3170, 3611, 3184, 3595, 3194, 3582, 3200, 3560, 3210, 3529, 3217, 3495, 3225, 3449,
        3229, 3399, 3229, 3363, 3229, 3321, 3229, 3276, 3229, 3249, 3229, 3214, 3225, 3192, 3218,
        3153, 3207, 3136, 3199, 3093, 3172, 3069, 3152, 3051, 3132, 3040, 3117, 3033, 3103, 3026,
        3084, 3024, 3051, 3024, 3030, 3024, 3009, 3024, 2971, 3024, 2928, 3026, 2890, 3041, 2841,
        3073, 2779, 3139, 2696, 3197, 2640, 3228, 2616, 3330, 2563, 3398, 2544, 3477, 2533, 3550,
        2529, 3623, 2529, 3702, 2539, 3780, 2579, 3852, 2634, 3913, 2676, 3945, 2698, 3983, 2729,
        4011, 2746, 4035, 2767, 4055, 2790, 4078, 2823, 4088, 2848, 4099, 2885, 4103, 2918, 4103,
        2990, 4081, 3027, 4045, 3067, 4002, 3105, 3921, 3159, 3900, 3171, 3849, 3192, 3803, 3205,
        3762, 3217, 3720, 3225, 3678, 3225, 3636, 3225, 3594, 3225, 3566, 3225, 3535, 3225, 3516,
        3222, 3502, 3218,
    ];

    /// Stroke 2 of the same capture: a one-stroke arrow (shaft, then a flicked head) whose tip
    /// lands on “you let go, but it's saved on the keyframes.”
    const ARROW: [i32; 80] = [
        4057, 2996, 4047, 2996, 4075, 2996, 4130, 3004, 4236, 3044, 4312, 3073, 4406, 3106, 4490,
        3121, 4567, 3131, 4606, 3131, 4633, 3131, 4653, 3131, 4621, 3111, 4570, 3090, 4507, 3062,
        4404, 3018, 4373, 3007, 4345, 3000, 4331, 3000, 4322, 2997, 4337, 2995, 4376, 2995, 4436,
        2999, 4509, 3023, 4549, 3039, 4607, 3061, 4638, 3076, 4652, 3083, 4663, 3086, 4674, 3092,
        4662, 3107, 4648, 3115, 4609, 3147, 4530, 3217, 4512, 3233, 4494, 3249, 4483, 3259, 4467,
        3275, 4458, 3283, 4481, 3284,
    ];

    fn real_stroke(t0: u64, flat: &[i32]) -> Stroke {
        Stroke {
            pts: flat
                .chunks_exact(2)
                .enumerate()
                .flat_map(|(i, p)| [p[0], p[1], i as i32 * 17])
                .collect(),
            ..stroke(t0, &[])
        }
    }

    /// What `scrawl-helper read-text` returned for the real capture: the table where each mark
    /// ends (0:07 and 0:11; “⇧” reads as “t”), and the paragraph where the drag at 0:15 let go.
    const READ: &str = include_str!("screen-text-fixture.json");

    #[test]
    fn the_real_capture_names_the_circled_cell_the_arrowed_line_and_the_selected_sentence() {
        let regions: Vec<Region> = serde_json::from_str(READ).unwrap();
        assert_eq!(
            regions.iter().map(|r| r.t).collect::<Vec<_>>(),
            [7_307, 11_512, 15_693]
        );
        let first = SPACE.lines(regions[0].lines.clone());
        let table = SPACE.lines(regions[1].lines.clone());
        let paragraph = SPACE.lines(regions[2].lines.clone());
        let strokes = [real_stroke(6_208, &RING), real_stroke(10_283, &ARROW)];

        let circle = Mark {
            id: "m1".into(),
            strokes: vec![0],
            t0: 6_208,
            t1: 7_386,
            box_norm: [0.3024, 0.2529, 0.4103, 0.3229],
            shape: Shape::Circle,
            from: None,
            to: None,
            pin: None,
            on: None,
            description: String::new(),
            frame: None,
        };
        let cell = Some("Hold t (Shift) and move the mouse");
        let look = look_for(&circle, &strokes, SPACE).unwrap();
        assert_eq!(pick(&first, &look).as_deref(), cell);
        // Main's detector calls the ring a freeform mark; it closes, so it encloses the same.
        let freeform = Mark {
            shape: Shape::Mark,
            ..circle.clone()
        };
        let look = look_for(&freeform, &strokes, SPACE).unwrap();
        assert_eq!(pick(&first, &look).as_deref(), cell);

        let arrow = Mark {
            id: "m2".into(),
            strokes: vec![1],
            t0: 10_283,
            t1: 11_591,
            box_norm: [0.4047, 0.2995, 0.4674, 0.3284],
            shape: Shape::Arrow,
            from: Some([0.4057, 0.2996]),
            to: Some([0.4674, 0.3092]),
            ..circle
        };
        let look = look_for(&arrow, &strokes, SPACE).unwrap();
        assert_eq!(
            pick(&table, &look).as_deref(),
            Some("you let go, but it's saved on the keyframes.")
        );
        // As main's freeform mark, the open stroke names the words under its box.
        let freeform = Mark {
            shape: Shape::Mark,
            ..arrow
        };
        let look = look_for(&freeform, &strokes, SPACE).unwrap();
        assert_eq!(pick(&table, &look).as_deref(), Some("you let go, but"));

        let drag = InteractionEvent::Drag {
            t: 15_005,
            t1: 15_772,
            x0: 0.308_406_1,
            y0: 0.588_717_7,
            x1: 0.615_989_4,
            y1: 0.614_683_6,
            label: None,
            button: None,
            path: Vec::new(),
        };
        let jobs = plan(&[], std::slice::from_ref(&drag), SPACE);
        assert_eq!(jobs.len(), 1);
        assert_eq!(
            pick(&paragraph, &jobs[0].look).as_deref(),
            Some("They'd work out most of it. A paste contains the contact-sh…")
        );
    }

    #[test]
    fn found_text_fills_only_what_nothing_named() {
        let mut strokes = vec![
            stroke(0, &[[0.1, 0.1], [0.3, 0.1]]),
            Stroke {
                on: Some("button “Area”".into()),
                ..stroke(5_000, &[[0.5, 0.5], [0.6, 0.5]])
            },
        ];
        let drag = |label: Option<&str>| InteractionEvent::Drag {
            t: 9_000,
            t1: 9_500,
            x0: 0.1,
            y0: 0.5,
            x1: 0.6,
            y1: 0.5,
            label: label.map(Into::into),
            button: None,
            path: Vec::new(),
        };
        let mut events = vec![drag(None), drag(Some("slider “Playback”"))];
        let jobs = plan(&strokes, &events, SPACE);
        assert_eq!(
            jobs.iter().map(|j| j.into).collect::<Vec<_>>(),
            [Field::Stroke(0), Field::Drag(0)],
            "named marks and drags are not read"
        );
        assert_eq!(jobs[0].t, 500, "a mark is read where its last stroke ends");
        assert_eq!(jobs[1].t, 9_500, "a drag is read where it was let go");
        let found = Found {
            strokes: vec![(0, "text “Title”".into()), (1, "text “Other”".into())],
            drags: vec![(0, "text “They'd work”".into()), (1, "text “Other”".into())],
            ms: 0,
        };
        found.apply(&mut strokes, &mut events);
        assert_eq!(strokes[0].on.as_deref(), Some("text “Title”"));
        assert_eq!(strokes[1].on.as_deref(), Some("button “Area”"));
        assert_eq!(
            events,
            vec![
                drag(Some("text “They'd work”")),
                drag(Some("slider “Playback”"))
            ]
        );
        assert!(is_read("text “They'd work”"));
        assert!(!is_read("slider “Playback”") && !is_read("“Area”") && !is_read("text field"));
    }

    #[test]
    fn nothing_to_read_asks_nothing_and_empty_regions_are_dropped() {
        let frame = TargetFrame {
            x: 0.0,
            y: 0.0,
            width: 1728.0,
            height: 1117.0,
            scale: 2.0,
        };
        let missing = Path::new("/nonexistent/recording.mp4");
        assert_eq!(
            read(Some(missing), 0, &frame, &[], &[]),
            None,
            "no job, no helper, no timing"
        );
        let job = |x: f32| Job {
            into: Field::Drag(0),
            look: Look::Across(vec![[x, 100.0], [x + 50.0, 100.0]]),
            t: 900,
        };
        // The second lies wholly right of the frame: after clamping it has no area.
        let (jobs, regions) = requests(vec![job(100.0), job(5_000.0)], SPACE, 79);
        assert_eq!(jobs, vec![job(100.0)]);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].0, 821);
    }

    /// `SCRAWL_TEXT_MOVIE=…/recording.mp4 cargo test --locked screen_text -- --ignored`: one
    /// helper call with 24 regions over 8 moments answers every region in a few seconds (blocking
    /// recognitions once starved Swift's cooperative pool and hung at 16).
    #[test]
    #[ignore = "runs scrawl-helper on a real recording (SCRAWL_TEXT_MOVIE)"]
    #[cfg(target_os = "macos")]
    fn many_regions_in_one_call_finish_quickly() {
        let movie = std::env::var_os("SCRAWL_TEXT_MOVIE").expect("set SCRAWL_TEXT_MOVIE");
        let regions: Vec<(u64, [f32; 4])> = (0..24)
            .map(|i| {
                let x = 0.05 + 0.1 * (i % 8) as f32;
                let y = 0.1 + 0.25 * (i / 8) as f32;
                (1_000 + 2_000 * (i % 8) as u64, [x, y, x + 0.25, y + 0.2])
            })
            .collect();
        let started = Instant::now();
        let read = crate::helper::read_text(Path::new(&movie), &regions).unwrap();
        let elapsed = started.elapsed();
        eprintln!("read-text: 24 regions in {} ms", elapsed.as_millis());
        assert_eq!(read.len(), 24);
        assert!(elapsed.as_secs_f32() < 5.0, "{elapsed:?}");
    }

    #[test]
    fn regions_surround_their_targets_inside_the_frame() {
        let r = SPACE.normalized(region(&Look::At {
            point: [10.0, 10.0],
            reach: 1.5,
        }));
        assert_eq!(r[0], 0.0);
        assert_eq!(r[1], 0.0);
        assert!(r[2] > 0.2 && r[3] > 0.04);
        let r = region(&Look::Above([100.0, 200.0, 300.0, 204.0]));
        assert!(
            r[1] < 140.0 && r[3] > 204.0,
            "an underline reads above itself"
        );
    }
}
