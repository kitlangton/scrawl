//! Tooling around `src/autocrop.rs`: the tuning playground (`scrawl autocrop`), simulated
//! captures built through the real pipeline (`scrawl fixture`), and the scorer that compares
//! crops with hand-labelled ground truth (`scrawl autocrop-eval`).

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::autocrop::{self, Crop, Input, Rect, Settings};
use crate::model::{
    InteractionEvent, KeyframeReason, RgbaFrame, ScrawlBundle, Stroke, TargetInfo, TranscriptData,
    TranscriptSegment, WordTiming,
};
use crate::windows::{ActivityLog, CursorSample, WindowSnapshot};

fn decode(path: &Path) -> Result<RgbaFrame> {
    let file =
        std::fs::File::open(path).with_context(|| format!("Could not open {}", path.display()))?;
    crate::render::decode_png(BufReader::new(file))
}

pub use crate::render::downscale;

/// Area-averaged copy at exactly `w`×`h` (what ScreenCaptureKit hands the sampler).
pub fn resize(frame: &RgbaFrame, w: u32, h: u32) -> RgbaFrame {
    let (sw, sh) = (frame.width as usize, frame.height as usize);
    let (w, h) = (w.max(1) as usize, h.max(1) as usize);
    let mut rgba = vec![0_u8; w * h * 4];
    for y in 0..h {
        let (y0, y1) = (y * sh / h, ((y + 1) * sh / h).max(y * sh / h + 1));
        for x in 0..w {
            let (x0, x1) = (x * sw / w, ((x + 1) * sw / w).max(x * sw / w + 1));
            let mut acc = [0_u32; 4];
            for row in y0..y1 {
                for px in frame.rgba[(row * sw + x0) * 4..(row * sw + x1) * 4]
                    .as_chunks::<4>()
                    .0
                {
                    for (a, v) in acc.iter_mut().zip(px) {
                        *a += u32::from(*v);
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let i = (y * w + x) * 4;
            for c in 0..4 {
                rgba[i + c] = (acc[c] / n) as u8;
            }
        }
    }
    RgbaFrame::new(frame.t_ms, w as u32, h as u32, rgba)
}

/// The crop algorithm's input for a bundle: change regions from `scrawl.json` when the bundle
/// was cropped, else from its keyframe images.
pub fn input_for_bundle(dir: &Path, bundle: &ScrawlBundle) -> Input {
    let changes = match &bundle.autocrop {
        Some(info) if !info.changes.is_empty() => info.changes.clone(),
        _ => {
            let frames: Vec<Option<RgbaFrame>> = bundle
                .keyframes
                .iter()
                .map(|kf| decode(&dir.join(&kf.file)).ok())
                .collect();
            (0..frames.len())
                .map(|i| {
                    match (
                        i.checked_sub(1).and_then(|p| frames[p].as_ref()),
                        &frames[i],
                    ) {
                        (Some(a), Some(b)) => autocrop::change_regions(a, b),
                        _ => Vec::new(),
                    }
                })
                .collect()
        }
    };
    let times: Vec<u64> = bundle.keyframes.iter().map(|k| k.t).collect();
    let mut input = Input::new(
        bundle.target.frame,
        bundle.duration_ms,
        &times,
        &bundle.strokes,
        &bundle.events,
        &bundle.windows,
        &bundle.cursor,
        &changes,
    );
    input.pin_ink(&bundle.keyframes, &bundle.strokes, &bundle.marks);
    // The full frames carry Scrawl's ink, which reads as content too; close enough for tuning.
    let full = bundle
        .autocrop
        .as_ref()
        .map_or("frames", |a| a.full_frames.as_str());
    let screen = input.screen;
    input.content = std::thread::scope(|scope| {
        let workers: Vec<_> = bundle
            .keyframes
            .iter()
            .map(|kf| {
                let file = dir
                    .join(full)
                    .join(Path::new(&kf.file).file_name().unwrap_or_default());
                scope.spawn(move || {
                    decode(&file)
                        .ok()
                        .map(|f| autocrop::Content::from_frame(&f, screen))
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|w| w.join().ok().flatten())
            .collect()
    });
    input
}

// ---------------------------------------------------------------------------------------------
// Playground
// ---------------------------------------------------------------------------------------------

const PLAYGROUND: &str = include_str!("autocrop-playground.html");
const DATA_MARKER: &str = "/*SCRAWL_AUTOCROP_DATA*/null";

/// The named settings the playground offers as presets.
pub fn presets() -> Vec<(&'static str, Settings)> {
    let d = Settings::default();
    vec![
        ("Recommended", d.clone()),
        (
            "Windows only",
            Settings {
                big_window: 1.0,
                ..d.clone()
            },
        ),
        (
            "Activity box",
            Settings {
                snap: false,
                ..d.clone()
            },
        ),
        (
            "Whole capture",
            Settings {
                scope: autocrop::Scope::Whole,
                ..d
            },
        ),
    ]
}

fn playground_data(dir: &Path, bundle: &ScrawlBundle, served: bool) -> Value {
    let input = input_for_bundle(dir, bundle);
    let full_dir = bundle
        .autocrop
        .as_ref()
        .map_or("frames".to_owned(), |a| a.full_frames.clone());
    let keyframes: Vec<Value> = bundle
        .keyframes
        .iter()
        .map(|kf| {
            let name = Path::new(&kf.file)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            json!({
                "index": kf.index, "t": kf.t, "reason": kf.reason, "event": kf.event,
                "said": kf.said, "file": format!("../{full_dir}/{name}"),
            })
        })
        .collect();
    let frame = bundle.keyframes.first().and_then(|kf| {
        let name = Path::new(&kf.file).file_name()?;
        decode(&dir.join(&full_dir).join(name)).ok()
    });
    let presets = presets();
    json!({
        "id": bundle.id,
        "mode": bundle.target.mode,
        "source": bundle.source,
        "served": served,
        "screen": {"width": input.screen[0], "height": input.screen[1]},
        "frame": frame.map(|f| json!({"width": f.width, "height": f.height})),
        "durationMs": bundle.duration_ms,
        "keyframes": keyframes,
        "input": input,
        "words": bundle.transcript.words,
        "defaults": Settings::default(),
        "presets": presets.iter().map(|(name, s)| json!({
            "name": name, "settings": s, "crops": autocrop::compute(&input, s),
        })).collect::<Vec<_>>(),
    })
}

fn playground_page(dir: &Path, bundle: &ScrawlBundle, served: bool) -> Result<String> {
    let data = playground_data(dir, bundle, served);
    let inline = serde_json::to_string(&data)?.replace("</", "<\\/");
    Ok(PLAYGROUND.replace(DATA_MARKER, &inline))
}

/// Writes `autocrop/index.html` (presets only; serve it to tune live) and returns its path.
pub fn export(dir: &Path, bundle: &ScrawlBundle) -> Result<PathBuf> {
    let out = dir.join("autocrop");
    std::fs::create_dir_all(&out)?;
    let page = out.join("index.html");
    std::fs::write(&page, playground_page(dir, bundle, false)?)?;
    Ok(page)
}

/// Serves the playground on `127.0.0.1:port`: the page, the bundle's files, and `POST /crop`,
/// which runs the crop with the posted settings.
pub fn serve(dir: &Path, bundle: &ScrawlBundle, port: u16, open: bool) -> Result<()> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
    let url = format!(
        "http://127.0.0.1:{}/autocrop/",
        listener.local_addr()?.port()
    );
    println!("{url}");
    if open {
        let _ = crate::handoff::open_path(std::path::Path::new(&url));
    }
    let input = input_for_bundle(dir, bundle);
    let page = playground_page(dir, bundle, true)?;
    let root = dir.canonicalize()?;
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let _ = (|| -> Result<()> {
            let mut reader = BufReader::new(stream.try_clone()?);
            let mut line = String::new();
            reader.read_line(&mut line)?;
            let mut parts = line.split_whitespace();
            let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
            let mut length = 0;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header)?;
                if header.trim().is_empty() {
                    break;
                }
                if let Some((k, v)) = header.split_once(':')
                    && k.eq_ignore_ascii_case("content-length")
                {
                    length = v.trim().parse().unwrap_or(0);
                }
            }
            let (status, kind, body): (&str, &str, Vec<u8>) = match (method, path) {
                ("POST", "/crop") => {
                    let mut buf = vec![0; length];
                    reader.read_exact(&mut buf)?;
                    match serde_json::from_slice::<Settings>(&buf) {
                        Ok(s) => (
                            "200 OK",
                            "application/json",
                            serde_json::to_vec(&autocrop::compute(&input, &s))?,
                        ),
                        Err(e) => ("400 Bad Request", "text/plain", e.to_string().into_bytes()),
                    }
                }
                ("GET", "/" | "/autocrop/" | "/autocrop/index.html") => (
                    "200 OK",
                    "text/html; charset=utf-8",
                    page.clone().into_bytes(),
                ),
                ("GET", p) => {
                    let rel = p.trim_start_matches('/').split('?').next().unwrap_or("");
                    let rel = rel.replace("%20", " ");
                    let file = root.join(&rel);
                    match file.canonicalize() {
                        Ok(f) if f.starts_with(&root) && f.is_file() => {
                            let kind = if rel.ends_with(".png") {
                                "image/png"
                            } else {
                                "application/octet-stream"
                            };
                            ("200 OK", kind, std::fs::read(f)?)
                        }
                        _ => ("404 Not Found", "text/plain", b"not found".to_vec()),
                    }
                }
                _ => ("405 Method Not Allowed", "text/plain", Vec::new()),
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(&body)?;
            Ok(())
        })();
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Simulated captures
// ---------------------------------------------------------------------------------------------

/// A simulated Screen capture, written by `scripts/autocrop-corpus.mjs`: what Scrawl's input
/// monitors and window log would have recorded, plus a rendered frame for each screen state.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scenario {
    pub target: TargetInfo,
    pub duration_ms: u64,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub strokes: Vec<Stroke>,
    #[serde(default)]
    pub events: Vec<InteractionEvent>,
    #[serde(default)]
    pub words: Vec<WordTiming>,
    #[serde(default)]
    pub segments: Vec<TranscriptSegment>,
    #[serde(default)]
    pub windows: Vec<WindowSnapshot>,
    #[serde(default)]
    pub cursor: Vec<CursorSample>,
    /// The screen from `t` until the next state, as a PNG relative to the scenario file.
    pub frames: Vec<ScenarioFrame>,
}

#[derive(Deserialize)]
pub struct ScenarioFrame {
    pub t: u64,
    pub file: String,
}

/// Builds a bundle from a scenario through the same pipeline a Screen capture finishes with:
/// candidate frames every 220 ms, keyframe selection, full-resolution keyframes, auto-crop.
pub fn build_fixture(
    scenario_path: &Path,
    out: &Path,
    settings: Option<Settings>,
) -> Result<PathBuf> {
    let scenario: Scenario = serde_json::from_slice(&std::fs::read(scenario_path)?)
        .with_context(|| format!("Invalid scenario {}", scenario_path.display()))?;
    let base = scenario_path.parent().unwrap_or(Path::new("."));
    ensure!(!scenario.frames.is_empty(), "The scenario has no frames");
    let states: Vec<(u64, Arc<RgbaFrame>)> = scenario
        .frames
        .iter()
        .map(|f| Ok((f.t, Arc::new(decode(&base.join(&f.file))?))))
        .collect::<Result<_>>()?;
    let small: Vec<RgbaFrame> = states.iter().map(|(_, f)| downscale(f, 640)).collect();
    let state_at = |t: u64| states.iter().rposition(|(st, _)| *st <= t).unwrap_or(0);
    let candidates: Vec<RgbaFrame> = (0..=scenario.duration_ms)
        .step_by(220)
        .map(|t| {
            let mut f = small[state_at(t)].clone();
            f.t_ms = t;
            f
        })
        .collect();
    let full_states = states.clone();
    let full_frames = move |times: &[u64]| -> Result<Vec<RgbaFrame>> {
        Ok(times
            .iter()
            .map(|&t| {
                let i = full_states
                    .iter()
                    .rposition(|(st, _)| *st <= t)
                    .unwrap_or(0);
                let mut f = (*full_states[i].1).clone();
                f.t_ms = t;
                f
            })
            .collect())
    };
    let text = scenario
        .words
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let transcript = TranscriptData {
        source: "fixture".into(),
        model: None,
        text,
        segments: scenario.segments,
        words: scenario.words,
        estimated: false,
        missing: None,
    };
    if out.exists() {
        std::fs::remove_dir_all(out)?;
    }
    std::fs::create_dir_all(out)?;
    crate::pipeline::build_bundle_from_parts(
        out,
        scenario.target,
        scenario.duration_ms,
        candidates,
        scenario.strokes,
        scenario.events,
        transcript,
        None,
        scenario.note,
        crate::pipeline::SourceExtras {
            activity: ActivityLog {
                windows: scenario.windows,
                cursor: scenario.cursor,
                ..Default::default()
            },
            full_frames: Some(Box::new(full_frames)),
            autocrop: Some(Some(settings.unwrap_or_default())),
            ..Default::default()
        },
    )?;
    let truth = base.join("truth.json");
    if truth.is_file() {
        std::fs::copy(truth, out.join("truth.json"))?;
    }
    Ok(out.to_path_buf())
}

/// What `simulate_capture` measured.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimReport {
    pub seconds: u64,
    pub legacy: bool,
    pub candidates: usize,
    pub candidate_bytes: usize,
    pub first_candidate_ms: u64,
    pub sample_interval_ms: u64,
    pub keyframes: usize,
    /// Keyframes at the movie's full width.
    pub full_size_keyframes: usize,
    pub finish_ms: u64,
}

/// A Screen capture of `seconds`, driven headless: the scenario's screen states and input loop to
/// fill the time, the sampler's candidate buffer fills on a virtual clock (no waiting), and the
/// bundle is finished as at the end of a real capture, with keyframes decoded from `movie` (a
/// recording of the same states, `mkmovie`). `legacy` replays the old sampler instead: full-size
/// candidates, the newest 300 kept, keyframes straight from them.
pub fn simulate_capture(
    scenario_path: &Path,
    seconds: u64,
    movie: Option<&Path>,
    legacy: bool,
    out: &Path,
) -> Result<SimReport> {
    use crate::capture::{
        CANDIDATE_BUDGET_BYTES, CANDIDATE_EDGE, CandidateBuffer, FRAME_SAMPLE_INTERVAL,
    };
    let scenario: Scenario = serde_json::from_slice(&std::fs::read(scenario_path)?)?;
    let base = scenario_path.parent().unwrap_or(Path::new("."));
    let states: Vec<(u64, RgbaFrame)> = scenario
        .frames
        .iter()
        .map(|f| {
            let full = decode(&base.join(&f.file))?;
            let frame = if legacy {
                full
            } else {
                let (w, h) = crate::capture::capture_size(&scenario.target, CANDIDATE_EDGE);
                resize(&full, w, h)
            };
            Ok((f.t, frame))
        })
        .collect::<Result<_>>()?;
    let period = scenario.duration_ms.max(1);
    let total = seconds * 1_000;
    let laps = total.div_ceil(period);
    let shift = |t: u64, lap: u64| t + lap * period;
    let mut strokes = Vec::new();
    let mut events = Vec::new();
    let mut words = Vec::new();
    let mut windows = Vec::new();
    let mut cursor = Vec::new();
    for lap in 0..laps {
        for s in &scenario.strokes {
            let mut s = s.clone();
            (s.t0, s.t1) = (shift(s.t0, lap), shift(s.t1, lap));
            strokes.push(s);
        }
        for e in &scenario.events {
            let mut json = serde_json::to_value(e)?;
            for key in ["t", "t1"] {
                if let Some(t) = json.get(key).and_then(Value::as_u64) {
                    json[key] = json!(shift(t, lap));
                }
            }
            events.push(serde_json::from_value::<InteractionEvent>(json)?);
        }
        for w in &scenario.words {
            words.push(WordTiming {
                t0: shift(w.t0, lap),
                t1: shift(w.t1, lap),
                text: w.text.clone(),
            });
        }
        for snap in &scenario.windows {
            let mut snap = snap.clone();
            snap.t = shift(snap.t, lap);
            windows.push(snap);
        }
        for c in &scenario.cursor {
            cursor.push(CursorSample(shift(c.0, lap), c.1, c.2));
        }
    }
    strokes.retain(|s| s.t1 < total);
    events.retain(|e| {
        serde_json::to_value(e)
            .ok()
            .and_then(|v| v["t"].as_u64())
            .is_some_and(|t| t < total)
    });
    // When the input monitors raise the sampler's urgent flag: clicks and stroke starts.
    let mut urgent: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            InteractionEvent::Click { t, .. } | InteractionEvent::Drag { t, .. } => Some(*t),
            _ => None,
        })
        .chain(strokes.iter().map(|s| s.t0))
        .collect();
    urgent.sort_unstable();
    let state_at = |t: u64| {
        let local = t % period;
        let i = states.iter().rposition(|(st, _)| *st <= local).unwrap_or(0);
        let mut f = states[i].1.clone();
        f.t_ms = t;
        f
    };
    let mut next_urgent = urgent.iter().peekable();
    let (frames, interval_ms) = if legacy {
        let mut kept = std::collections::VecDeque::with_capacity(300);
        let mut t = 0;
        while t <= total {
            while let Some(&&u) = next_urgent.peek() {
                if u > t {
                    break;
                }
                next_urgent.next();
                if kept.len() >= 300 {
                    kept.pop_front();
                }
                kept.push_back(state_at(u));
            }
            if kept.len() >= 300 {
                kept.pop_front();
            }
            kept.push_back(state_at(t));
            t += FRAME_SAMPLE_INTERVAL.as_millis() as u64;
        }
        (Vec::from(kept), FRAME_SAMPLE_INTERVAL.as_millis() as u64)
    } else {
        let mut buffer = CandidateBuffer::new(CANDIDATE_BUDGET_BYTES);
        let mut t = 0;
        while t <= total {
            while let Some(&&u) = next_urgent.peek() {
                if u > t {
                    break;
                }
                next_urgent.next();
                buffer.push(state_at(u), true);
            }
            buffer.push(state_at(t), false);
            t += buffer.interval().as_millis() as u64;
        }
        let interval = buffer.interval().as_millis() as u64;
        (buffer.into_frames(), interval)
    };
    let report_candidates = frames.len();
    let candidate_bytes = frames.iter().map(|f| f.rgba.len()).sum();
    let first_candidate_ms = frames.first().map_or(0, |f| f.t_ms);
    drop(states);

    if out.exists() {
        std::fs::remove_dir_all(out)?;
    }
    std::fs::create_dir_all(out)?;
    let video = match movie {
        Some(m) => {
            let dest = out.join("recording.mp4");
            std::fs::copy(m, &dest)?;
            Some(dest)
        }
        None => None,
    };
    let text = words
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let transcript = TranscriptData {
        source: "fixture".into(),
        model: None,
        text,
        segments: Vec::new(),
        words,
        estimated: false,
        missing: None,
    };
    let started = std::time::Instant::now();
    let (_, bundle, _) = crate::pipeline::build_bundle_from_parts(
        out,
        scenario.target,
        total,
        frames,
        strokes,
        events,
        transcript,
        video.clone(),
        None,
        crate::pipeline::SourceExtras {
            activity: ActivityLog {
                windows,
                cursor,
                ..Default::default()
            },
            full_frames: video
                .filter(|_| !legacy)
                .map(|v| crate::pipeline::movie_keyframes(v, 0)),
            ..Default::default()
        },
    )?;
    let finish_ms = started.elapsed().as_millis() as u64;
    let full_dir = bundle
        .autocrop
        .as_ref()
        .map_or("frames", |a| a.full_frames.as_str());
    let full_size_keyframes = bundle
        .keyframes
        .iter()
        .filter(|kf| {
            let name = Path::new(&kf.file).file_name().unwrap_or_default();
            std::fs::File::open(out.join(full_dir).join(name))
                .ok()
                .and_then(|f| {
                    png::Decoder::new(BufReader::new(f))
                        .read_info()
                        .ok()
                        .map(|r| r.info().width)
                })
                .is_some_and(|w| w >= 2560)
        })
        .count();
    Ok(SimReport {
        seconds,
        legacy,
        candidates: report_candidates,
        candidate_bytes,
        first_candidate_ms,
        sample_interval_ms: interval_ms,
        keyframes: bundle.keyframes.len(),
        full_size_keyframes,
        finish_ms,
    })
}

// ---------------------------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------------------------

/// Hand-labelled crops for a simulated capture, in capture points (`[x, y, width, height]`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Truth {
    pub scenario: String,
    #[serde(default)]
    pub description: String,
    pub screen: [f32; 2],
    /// The crop for the capture as a whole.
    pub whole: [f32; 4],
    pub spans: Vec<TruthSpan>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TruthSpan {
    pub t0: u64,
    pub t1: u64,
    /// Keyframes about the same thing share a subject and shouldn't jump.
    pub subject: String,
    pub ideal: [f32; 4],
    /// What the moment is about: must be inside, with some margin.
    #[serde(default)]
    pub targets: Vec<[f32; 4]>,
}

/// One scenario's ground truth with the activity each keyframe must keep, ready to score.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Case {
    pub name: String,
    pub truth: Truth,
    /// Pixels per point of the full frames.
    pub px_per_pt: f32,
    pub keyframes: Vec<CaseKeyframe>,
    pub input: Input,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseKeyframe {
    pub index: usize,
    pub t: u64,
    /// The keyframe's own ink or click, normalized: must be inside the crop.
    pub must: Vec<Rect>,
}

fn norm(b: [f32; 4], screen: [f32; 2]) -> Rect {
    [
        b[0] / screen[0],
        b[1] / screen[1],
        (b[0] + b[2]) / screen[0],
        (b[1] + b[3]) / screen[1],
    ]
}

pub fn load_case(dir: &Path) -> Result<Case> {
    let (dir, bundle) = crate::handoff::load_bundle(dir)?;
    let truth: Truth = serde_json::from_slice(&std::fs::read(dir.join("truth.json"))?)
        .with_context(|| format!("Invalid truth.json in {}", dir.display()))?;
    let frame_w = bundle
        .keyframes
        .first()
        .and_then(|kf| {
            let full = bundle
                .autocrop
                .as_ref()
                .map_or("frames".into(), |a| a.full_frames.clone());
            let name = Path::new(&kf.file).file_name()?;
            let file = std::fs::File::open(dir.join(full).join(name)).ok()?;
            let reader = png::Decoder::new(BufReader::new(file)).read_info().ok()?;
            Some(reader.info().width as f32)
        })
        .unwrap_or(2560.0);
    let keyframes = bundle
        .keyframes
        .iter()
        .map(|kf| {
            let mut must = Vec::new();
            let pad = |x: f32, y: f32, pt: f32| {
                let (px, py) = (pt / truth.screen[0], pt / truth.screen[1]);
                [x - px, y - py, x + px, y + py]
            };
            match kf.reason {
                KeyframeReason::Mark => {
                    if let Some(mark) = kf
                        .mark_id
                        .as_ref()
                        .and_then(|id| bundle.marks.iter().find(|m| &m.id == id))
                    {
                        for &i in &mark.strokes {
                            let pts = bundle.strokes[i].timed_points();
                            if let Some(b) = pts.iter().fold(None, |r: Option<Rect>, &(x, y, _)| {
                                Some(r.map_or([x, y, x, y], |r| autocrop::union(r, [x, y, x, y])))
                            }) {
                                must.push(b);
                            }
                        }
                    }
                }
                KeyframeReason::Click => {
                    let click = bundle
                        .events
                        .iter()
                        .filter_map(|e| match e {
                            InteractionEvent::Click { t, x, y, .. } => {
                                Some((t.abs_diff(kf.t), *x, *y))
                            }
                            InteractionEvent::Drag { t, x1, y1, .. } => {
                                Some((t.abs_diff(kf.t), *x1, *y1))
                            }
                            _ => None,
                        })
                        .filter(|c| c.0 <= 800)
                        .min_by_key(|c| c.0);
                    if let Some((_, x, y)) = click {
                        must.push(pad(x, y, 4.0));
                    }
                }
                _ => {}
            }
            CaseKeyframe {
                index: kf.index,
                t: kf.t,
                must,
            }
        })
        .collect();
    Ok(Case {
        name: truth.scenario.clone(),
        px_per_pt: frame_w / truth.screen[0],
        keyframes,
        input: input_for_bundle(&dir, &bundle),
        truth,
    })
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyframeScore {
    pub index: usize,
    pub covered: bool,
    pub iou: f32,
    pub context: f32,
    pub resolution: f32,
    pub precision: f32,
    pub score: f32,
    pub rect: Rect,
    pub ideal: Rect,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseScore {
    pub name: String,
    pub score: f32,
    pub keyframes: Vec<KeyframeScore>,
    pub stability: f32,
    pub covered: f32,
    pub whole_iou: f32,
}

/// How legible a crop is in a 640 px contact-sheet cell: points get at least 0.8 px, and
/// upscaling (blur) costs.
fn resolution(r: Rect, screen: [f32; 2], px_per_pt: f32) -> f32 {
    let (w, h) = (
        (r[2] - r[0]) * screen[0] * px_per_pt,
        (r[3] - r[1]) * screen[1] * px_per_pt,
    );
    let cell = if h > w { 440.0 } else { 640.0 };
    let scale = (cell / w).min(960.0 / h);
    let shown = scale.min(1.0) * px_per_pt;
    let upscale = (scale - 1.0).max(0.0);
    ((shown / 0.8).min(1.0) - 0.5 * upscale).clamp(0.0, 1.0)
}

pub fn score_case(case: &Case, crops: &[Crop], whole: Option<&Crop>) -> CaseScore {
    let screen = case.truth.screen;
    let mut scores = Vec::new();
    let mut subjects = Vec::new();
    for (kf, crop) in case.keyframes.iter().zip(crops) {
        let span = case
            .truth
            .spans
            .iter()
            .find(|s| s.t0 <= kf.t && kf.t < s.t1)
            .or_else(|| case.truth.spans.last());
        let Some(span) = span else { continue };
        let ideal = norm(span.ideal, screen);
        let targets: Vec<Rect> = span.targets.iter().map(|t| norm(*t, screen)).collect();
        let r = crop.rect;
        let tol = [2.0 / screen[0], 2.0 / screen[1]];
        let inside = |m: &Rect| {
            m[0] >= r[0] - tol[0]
                && m[1] >= r[1] - tol[1]
                && m[2] <= r[2] + tol[0]
                && m[3] <= r[3] + tol[1]
        };
        let covered = kf.must.iter().chain(&targets).all(inside);
        let context = if targets.is_empty() {
            1.0
        } else {
            targets
                .iter()
                .map(|t| {
                    let m = (0.2 * ((t[2] - t[0]) * screen[0]).max((t[3] - t[1]) * screen[1]))
                        .max(24.0);
                    let grown = autocrop::intersect(
                        [
                            t[0] - m / screen[0],
                            t[1] - m / screen[1],
                            t[2] + m / screen[0],
                            t[3] + m / screen[1],
                        ],
                        autocrop::FULL,
                    );
                    autocrop::area(autocrop::intersect(grown, r)) / autocrop::area(grown).max(1e-9)
                })
                .sum::<f32>()
                / targets.len() as f32
        };
        let iou = autocrop::iou(r, ideal);
        let precision = autocrop::area(autocrop::intersect(r, ideal)) / autocrop::area(r).max(1e-9);
        let res = resolution(r, screen, case.px_per_pt);
        let score = if covered {
            0.45 * iou + 0.2 * context + 0.15 * res + 0.2 * precision
        } else {
            0.0
        };
        subjects.push(span.subject.clone());
        scores.push(KeyframeScore {
            index: kf.index,
            covered,
            iou,
            context,
            resolution: res,
            precision,
            score,
            rect: r,
            ideal,
            reason: crop.reason.clone(),
        });
    }
    let pairs: Vec<f32> = (1..scores.len())
        .filter(|&i| subjects[i] == subjects[i - 1])
        .map(|i| autocrop::iou(scores[i].rect, scores[i - 1].rect))
        .collect();
    let stability = if pairs.is_empty() {
        1.0
    } else {
        pairs.iter().sum::<f32>() / pairs.len() as f32
    };
    let n = scores.len().max(1) as f32;
    let mean = scores.iter().map(|s| s.score).sum::<f32>() / n;
    CaseScore {
        name: case.name.clone(),
        score: 0.85 * mean + 0.15 * stability,
        covered: scores.iter().filter(|s| s.covered).count() as f32 / n,
        stability,
        whole_iou: whole.map_or(0.0, |w| {
            autocrop::iou(w.rect, norm(case.truth.whole, screen))
        }),
        keyframes: scores,
    }
}

pub fn score_all(cases: &[Case], settings: &Settings) -> Vec<CaseScore> {
    let whole = Settings {
        scope: autocrop::Scope::Whole,
        ..settings.clone()
    };
    cases
        .iter()
        .map(|case| {
            let crops = autocrop::compute(&case.input, settings);
            let w = autocrop::compute(&case.input, &whole);
            score_case(case, &crops, w.first())
        })
        .collect()
}

/// Scores for not cropping at all, for reference.
pub fn score_uncropped(cases: &[Case]) -> Vec<CaseScore> {
    cases
        .iter()
        .map(|case| {
            let crops: Vec<Crop> = case
                .keyframes
                .iter()
                .map(|_| Crop {
                    rect: autocrop::FULL,
                    kind: autocrop::CropKind::Full,
                    reason: "uncropped".into(),
                    apps: Vec::new(),
                    core: None,
                    need: None,
                    within: None,
                    slice: [0, 0],
                    shares: Vec::new(),
                    snapped: Vec::new(),
                    outliers: Vec::new(),
                    used: Vec::new(),
                    merged: 0,
                    grown: false,
                })
                .collect();
            score_case(case, &crops, crops.first())
        })
        .collect()
}

/// The corpus without images, for unit tests: inputs rounded to 0.1 pt, no content maps.
pub fn export_cases(cases: &[Case]) -> Result<String> {
    let round = |v: f32| (v * 1e4).round() / 1e4;
    let cases: Vec<Case> = cases
        .iter()
        .map(|c| {
            let mut c = c.clone();
            c.input.content.clear();
            for it in &mut c.input.items {
                for p in &mut it.pts {
                    (p.0, p.1) = (round(p.0), round(p.1));
                }
                it.dwell = round(it.dwell);
            }
            for (_, wins) in &mut c.input.snapshots {
                for w in wins {
                    w.rect = w.rect.map(round);
                }
            }
            for k in &mut c.keyframes {
                for m in &mut k.must {
                    *m = m.map(round);
                }
            }
            c
        })
        .collect();
    Ok(serde_json::to_string(&cases)?)
}

pub fn mean(scores: &[CaseScore]) -> f32 {
    scores.iter().map(|s| s.score).sum::<f32>() / scores.len().max(1) as f32
}

/// Coordinate-descent sweep from `start`: each round tries every value of every knob and keeps
/// the best, until nothing improves.
pub fn sweep(cases: &[Case], start: &Settings) -> (Settings, f32) {
    let mut best = start.clone();
    let mut best_score = mean(&score_all(cases, &best));
    type Knob = (&'static str, Vec<Value>);
    let knobs: Vec<Knob> = vec![
        ("scope", vec![json!("keyframe"), json!("grouped")]),
        (
            "reachMs",
            vec![json!(0), json!(200), json!(400), json!(800), json!(1500)],
        ),
        (
            "snapThreshold",
            [0.4, 0.5, 0.6, 0.7, 0.8, 0.9].map(|v| json!(v)).to_vec(),
        ),
        ("maxWindows", vec![json!(1), json!(2), json!(3)]),
        (
            "minShare",
            [0.0, 0.05, 0.1, 0.2, 0.3].map(|v| json!(v)).to_vec(),
        ),
        (
            "bigWindow",
            [0.35, 0.45, 0.55, 0.6, 0.7, 0.8, 1.0]
                .map(|v| json!(v))
                .to_vec(),
        ),
        (
            "context",
            [0.0, 0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.4]
                .map(|v| json!(v))
                .to_vec(),
        ),
        ("keepInput", vec![json!(true), json!(false)]),
        (
            "passiveCap",
            [0.2, 0.35, 0.5, 0.75, 1.0, 10.0].map(|v| json!(v)).to_vec(),
        ),
        (
            "minDwell",
            [0.0, 0.15, 0.25, 0.4, 0.7].map(|v| json!(v)).to_vec(),
        ),
        (
            "hitSlop",
            [0.0, 12.0, 24.0, 40.0].map(|v| json!(v)).to_vec(),
        ),
        (
            "snapOut",
            [0.0, 60.0, 120.0, 200.0].map(|v| json!(v)).to_vec(),
        ),
        (
            "snapOutFrac",
            [0.0, 0.3, 0.6, 1.0, 1.5].map(|v| json!(v)).to_vec(),
        ),
        (
            "snapIn",
            [0.0, 30.0, 60.0, 120.0].map(|v| json!(v)).to_vec(),
        ),
        ("trim", [0.0, 12.0, 24.0, 48.0].map(|v| json!(v)).to_vec()),
        (
            "causeReach",
            [0.0, 20.0, 40.0, 80.0].map(|v| json!(v)).to_vec(),
        ),
        (
            "contentReach",
            [0.0, 200.0, 360.0, 500.0].map(|v| json!(v)).to_vec(),
        ),
        (
            "contentExtra",
            [80.0, 160.0, 240.0].map(|v| json!(v)).to_vec(),
        ),
        ("chrome", vec![json!(true), json!(false)]),
        (
            "padding",
            [0.0, 8.0, 16.0, 24.0, 40.0, 64.0]
                .map(|v| json!(v))
                .to_vec(),
        ),
        (
            "minW",
            [320.0, 400.0, 480.0, 560.0, 640.0, 800.0]
                .map(|v| json!(v))
                .to_vec(),
        ),
        (
            "minH",
            [200.0, 260.0, 320.0, 400.0, 480.0]
                .map(|v| json!(v))
                .to_vec(),
        ),
        (
            "maxZoom",
            [1.5, 2.0, 2.5, 3.0, 4.0, 5.0].map(|v| json!(v)).to_vec(),
        ),
        (
            "aspect",
            ["free", "auto", "16:10", "4:3", "3:2"]
                .map(|v| json!(v))
                .to_vec(),
        ),
        (
            "wClick",
            [0.2, 0.4, 0.6, 1.0, 1.5].map(|v| json!(v)).to_vec(),
        ),
        ("wScroll", [0.0, 0.15, 0.3, 0.6].map(|v| json!(v)).to_vec()),
        (
            "wCursor",
            [0.0, 0.05, 0.1, 0.15, 0.3, 0.5].map(|v| json!(v)).to_vec(),
        ),
        (
            "wChange",
            [0.0, 0.05, 0.1, 0.2, 0.4].map(|v| json!(v)).to_vec(),
        ),
        (
            "outlier",
            [0.08, 0.12, 0.18, 0.25, 0.35, 0.5]
                .map(|v| json!(v))
                .to_vec(),
        ),
        (
            "mergeIou",
            [0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.01]
                .map(|v| json!(v))
                .to_vec(),
        ),
        (
            "zoomStep",
            [1.5, 2.0, 3.0, 4.0, 6.0].map(|v| json!(v)).to_vec(),
        ),
        (
            "fallback",
            vec![json!("previous"), json!("frontmost"), json!("screen")],
        ),
        (
            "fullAbove",
            [0.6, 0.7, 0.8, 0.85, 0.9, 1.01].map(|v| json!(v)).to_vec(),
        ),
    ];
    for round in 0..4 {
        let mut improved = false;
        for (key, values) in &knobs {
            for v in values {
                let mut candidate = serde_json::to_value(&best).unwrap();
                candidate[*key] = v.clone();
                let Ok(s) = serde_json::from_value::<Settings>(candidate) else {
                    continue;
                };
                let score = mean(&score_all(cases, &s));
                if score > best_score + 1e-4 {
                    eprintln!("round {round}: {key} = {v} → {score:.4}");
                    best_score = score;
                    best = s;
                    improved = true;
                }
            }
        }
        if !improved {
            break;
        }
    }
    (best, best_score)
}

pub fn print_scores(label: &str, scores: &[CaseScore]) {
    println!("{label}: mean {:.3}", mean(scores));
    for s in scores {
        let kfs: Vec<String> = s
            .keyframes
            .iter()
            .map(|k| {
                format!(
                    "#{}{}{:.2}",
                    k.index,
                    if k.covered { " " } else { " ✗" },
                    k.iou
                )
            })
            .collect();
        println!(
            "  {:<24} {:.3}  cover {:>4.0}%  stab {:.2}  whole {:.2}  iou [{}]",
            s.name,
            s.score,
            s.covered * 100.0,
            s.stability,
            s.whole_iou,
            kfs.join(", ")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The simulated corpus (`scripts/autocrop-corpus.mjs`, built with `scrawl fixture` and
    /// exported with `scrawl autocrop-eval --export`), without images: no content maps, so edges
    /// don't snap to white space here.
    fn corpus() -> Vec<Case> {
        serde_json::from_str(include_str!("autocrop-corpus.json")).unwrap()
    }

    fn case<'a>(cases: &'a [Case], name: &str) -> &'a Case {
        cases.iter().find(|c| c.name == name).unwrap()
    }

    #[test]
    fn default_crops_keep_every_keyframes_activity_and_beat_the_full_frame() {
        let cases = corpus();
        assert!(cases.len() >= 12);
        let scores = score_all(&cases, &Settings::default());
        let uncropped = score_uncropped(&cases);
        for (s, u) in scores.iter().zip(&uncropped) {
            for k in &s.keyframes {
                assert!(
                    k.covered,
                    "{} #{} cuts off its activity: {:?}",
                    s.name, k.index, k.rect
                );
            }
            assert!(
                s.score > u.score,
                "{}: {:.3} vs uncropped {:.3}",
                s.name,
                s.score,
                u.score
            );
        }
        let m = mean(&scores);
        eprintln!("corpus mean {m:.3} (uncropped {:.3})", mean(&uncropped));
        assert!(m >= 0.85, "corpus mean {m:.3}");
    }

    #[test]
    fn crops_snap_to_the_window_in_use() {
        let cases = corpus();
        for name in [
            "side-by-side",
            "overlapping-front",
            "two-windows",
            "three-window-hop",
        ] {
            let c = case(&cases, name);
            let crops = autocrop::compute(&c.input, &Settings::default());
            for (kf, crop) in c.keyframes.iter().zip(&crops) {
                let span = c
                    .truth
                    .spans
                    .iter()
                    .find(|s| s.t0 <= kf.t && kf.t < s.t1)
                    .unwrap();
                let iou = autocrop::iou(crop.rect, norm(span.ideal, c.truth.screen));
                assert!(
                    iou > 0.9,
                    "{name} #{}: {} ({iou:.2})",
                    kf.index,
                    crop.reason
                );
            }
        }
    }

    #[test]
    fn a_wandering_cursor_does_not_grow_the_crop() {
        let cases = corpus();
        let c = case(&cases, "stray-cursor");
        for crop in autocrop::compute(&c.input, &Settings::default()) {
            // The cursor visits the bottom corners; the crop stays on the browser.
            assert!(crop.rect[3] < 0.85 && crop.rect[2] < 0.7, "{:?}", crop.rect);
            assert!(!crop.outliers.is_empty() || crop.kind == autocrop::CropKind::Window);
        }
    }

    #[test]
    fn a_menu_opened_from_the_menu_bar_is_in_the_crop() {
        let cases = corpus();
        let c = case(&cases, "menu-bar");
        let crops = autocrop::compute(&c.input, &Settings::default());
        // Keyframe 1 is the click on View: the menu bar is in, and so is the Finder window.
        let r = crops[0].rect;
        assert!(r[1] <= 0.001, "{r:?}");
        assert!(crops[0].apps == ["Finder"], "{:?}", crops[0].apps);
    }

    #[test]
    fn inside_a_huge_window_the_crop_zooms_in_on_the_activity() {
        let cases = corpus();
        for name in ["maximized-terminal", "tiny-badge", "fullscreen-app"] {
            let c = case(&cases, name);
            for crop in autocrop::compute(&c.input, &Settings::default()) {
                assert_eq!(
                    crop.kind,
                    autocrop::CropKind::Focus,
                    "{name}: {}",
                    crop.reason
                );
                assert!(autocrop::area(crop.rect) < 0.5, "{name}: {:?}", crop.rect);
            }
        }
    }
}
