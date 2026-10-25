//! What a speech recognizer hears of a recording, and when the words of a text-only answer
//! were said.
//!
//! `Plan` cuts the whole recording into chunks of at most 30 s, each cut at its quietest 50 ms
//! window, so every word is heard however quietly or briefly it was said. Only a chunk that is
//! silent or steady noise is left out, as a recognizer may invent words there.
//! Loudness never decides what is heard, only when: `transcribe` spreads a text-only answer
//! (Hex) over the voiced spans of its own chunk by length (`spread`, also used by the command
//! transcriber and Windows' Parakeet when they lack word times). Those times are estimates
//! (`TranscriptData::estimated`), so spoken cues never cut keyframes from them.

use anyhow::Result;

use crate::audio::{self, SAMPLES_PER_MS, WINDOW_MS};
use crate::model::{TranscriptData, TranscriptSegment, WordTiming};

/// The longest chunk sent in one request.
const MAX_CHUNK_MS: u64 = 30_000;
/// No cut is made earlier than this into a chunk, so each keeps enough context.
const MIN_CHUNK_MS: u64 = 15_000;
/// A window is voiced above this many times the noise floor, within these levels: more
/// sensitive than `audio::speech_islands`, as it only places words and skips dead chunks.
const VOICED_OVER_FLOOR: f64 = 2.0;
const VOICED_MIN_RMS: f64 = 0.001;
const VOICED_MAX_RMS: f64 = 0.01;
/// A chunk with no voiced window is still sent when the old planner would have sent the whole
/// recording (no 120 ms island above `audio::island_threshold`, and it lasts this long) and the
/// chunk's level moves: its loudest window is above `SILENT_RMS` and `STEADY_OVER_FLOOR` times
/// its own noise floor. So nothing the old planner heard is left out unless it is flat
/// (silence, white noise), and noise after audible speech is not sent, as before.
const UNSURE_MIN_MS: u64 = 480;
const OLD_MIN_ISLAND_MS: u64 = 120;
const SILENT_RMS: f64 = 0.000_1;
const STEADY_OVER_FLOOR: f64 = 1.25;

/// What a recognizer is given of one recording, and where its speech is.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    chunks: Vec<(u64, u64)>,
    voiced: Vec<(u64, u64)>,
}

impl Plan {
    pub fn new(samples_16k: &[f32]) -> Plan {
        let rms = audio::window_rms(samples_16k);
        let duration = samples_16k.len() as u64 / SAMPLES_PER_MS;
        let threshold =
            (audio::noise_floor(&rms) * VOICED_OVER_FLOOR).clamp(VOICED_MIN_RMS, VOICED_MAX_RMS);
        let voiced = audio::islands_above(&rms, threshold, duration);
        // The old planner's condition for sending everything: no speech island of 120 ms.
        let all_quiet = duration >= UNSURE_MIN_MS
            && audio::islands_above(&rms, audio::island_threshold(&rms), duration)
                .iter()
                .all(|&(a, b)| b - a < OLD_MIN_ISLAND_MS);
        let chunks = cuts(&rms, duration)
            .into_iter()
            .filter(|&(start, end)| {
                voiced.iter().any(|&(a, b)| a < end && b > start)
                    || (all_quiet && moves(&rms, start, end))
            })
            .collect();
        Plan { chunks, voiced }
    }

    /// The spans sent to the recognizer, in milliseconds: they tile the recording, each at most
    /// 30 s, except that silent or steady-noise chunks are left out.
    pub fn chunks(&self) -> &[(u64, u64)] {
        &self.chunks
    }

    /// Where the speech is, for timing only: energy islands at a sensitive threshold, with no
    /// minimum length.
    pub fn voiced(&self) -> &[(u64, u64)] {
        &self.voiced
    }

    /// The voiced spans inside `start..end`, else the whole of it.
    fn voiced_in(&self, start: u64, end: u64) -> Vec<(u64, u64)> {
        let spans: Vec<(u64, u64)> = self
            .voiced()
            .iter()
            .map(|&(a, b)| (a.max(start), b.min(end)))
            .filter(|&(a, b)| b > a)
            .collect();
        if spans.is_empty() {
            vec![(start, end)]
        } else {
            spans
        }
    }
}

/// Whether the level of `start..end` moves enough to be quiet speech (see `UNSURE_MIN_MS`).
fn moves(rms: &[f64], start: u64, end: u64) -> bool {
    let first = (start / WINDOW_MS) as usize;
    let last = (end.div_ceil(WINDOW_MS) as usize).min(rms.len());
    let Some(windows) = rms.get(first..last) else {
        return false;
    };
    let loudest = windows.iter().copied().fold(0.0, f64::max);
    loudest > SILENT_RMS && loudest > audio::noise_floor(windows) * STEADY_OVER_FLOOR
}

/// `0..duration` cut into chunks of at most `MAX_CHUNK_MS`, each cut in the middle of the
/// quietest window (the latest of equals) from `MIN_CHUNK_MS` into its chunk.
fn cuts(rms: &[f64], duration: u64) -> Vec<(u64, u64)> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while duration - start > MAX_CHUNK_MS {
        let level = |window: u64| rms.get(window as usize).copied().unwrap_or(f64::INFINITY);
        let windows = (start + MIN_CHUNK_MS) / WINDOW_MS..(start + MAX_CHUNK_MS) / WINDOW_MS;
        let Some(quietest) = windows.min_by(|&a, &b| level(a).total_cmp(&level(b)).then(b.cmp(&a)))
        else {
            break;
        };
        let cut = quietest * WINDOW_MS + WINDOW_MS / 2;
        chunks.push((start, cut));
        start = cut;
    }
    if duration > start {
        chunks.push((start, duration));
    }
    chunks
}

/// What a recognizer made of one chunk.
pub enum Heard {
    /// Text without times (Hex): its words are spread over the chunk's voiced spans.
    Text(String),
    /// Text with the times the recognizer measured, in milliseconds from the chunk's start
    /// (Windows' Parakeet). Without words, the text is spread as `Text` is.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    Timed {
        text: String,
        words: Vec<WordTiming>,
        segments: Vec<TranscriptSegment>,
    },
}

/// Runs `recognize` on each chunk of `plan` in order (given that chunk's samples) and joins what
/// it heard into one transcript, measured times moved to the recording's clock. Any chunk
/// spread from text makes the whole transcript `estimated`. Any error fails the whole
/// recording, so the caller can use another transcriber: skipping a refused chunk would
/// silently lose up to 30 s of speech.
pub fn transcribe(
    samples_16k: &[f32],
    plan: &Plan,
    source: String,
    model: Option<String>,
    mut recognize: impl FnMut(&[f32]) -> Result<Heard>,
) -> Result<TranscriptData> {
    let mut transcript = TranscriptData {
        source,
        model,
        ..Default::default()
    };
    let mut texts = Vec::new();
    for &(start, end) in plan.chunks() {
        let from = (start * SAMPLES_PER_MS) as usize;
        let to = ((end * SAMPLES_PER_MS) as usize).min(samples_16k.len());
        if from >= to {
            continue;
        }
        let (text, words, segments) = match recognize(&samples_16k[from..to])? {
            Heard::Text(text) => (text, Vec::new(), Vec::new()),
            Heard::Timed {
                text,
                words,
                segments,
            } => (text, words, segments),
        };
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        if words.is_empty() {
            let (segments, words) = spread(&text, &plan.voiced_in(start, end));
            transcript.estimated = true;
            transcript.segments.extend(segments);
            transcript.words.extend(words);
        } else {
            let at = |t0: u64, t1: u64| (start + t0, start + t1.max(t0));
            let words: Vec<WordTiming> = words
                .into_iter()
                .map(|word| {
                    let (t0, t1) = at(word.t0, word.t1);
                    WordTiming { t0, t1, ..word }
                })
                .collect();
            let segments: Vec<TranscriptSegment> = segments
                .into_iter()
                .map(|segment| {
                    let (t0, t1) = at(segment.t0, segment.t1);
                    TranscriptSegment { t0, t1, ..segment }
                })
                .collect();
            transcript.segments.extend(if segments.is_empty() {
                vec![TranscriptSegment {
                    t0: words[0].t0,
                    t1: words[words.len() - 1].t1,
                    text: text.clone(),
                }]
            } else {
                segments
            });
            transcript.words.extend(words);
        }
        texts.push(text);
    }
    transcript.text = texts.join(" ");
    Ok(transcript)
}

/// Spreads `text` over `spans` (in order, not overlapping), each word's share weighted by its
/// length. A span gets the words whose middle falls in its share of the spans' total length,
/// and they tile it exactly, so no word is placed between spans. No spans place nothing.
pub fn spread(text: &str, spans: &[(u64, u64)]) -> (Vec<TranscriptSegment>, Vec<WordTiming>) {
    let (mut segments, mut words) = (Vec::new(), Vec::new());
    if spans.is_empty() {
        return (segments, words);
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let total: u128 = tokens.iter().map(|token| weight(token)).sum();
    let speech: u128 = spans
        .iter()
        .map(|&(start, end)| u128::from(end.saturating_sub(start)))
        .sum();
    let mut groups: Vec<Vec<&str>> = vec![Vec::new(); spans.len()];
    let (mut before, mut span, mut span_end) = (0_u128, 0, 0_u128);
    for token in tokens {
        // Where the word's middle falls in the speech, scaled by 2 × `total` to stay whole.
        let middle = (2 * before + weight(token)) * speech;
        before += weight(token);
        while span + 1 < spans.len() {
            let end = span_end + u128::from(spans[span].1.saturating_sub(spans[span].0));
            if middle < 2 * total * end {
                break;
            }
            span_end = end;
            span += 1;
        }
        groups[span].push(token);
    }
    for (&(start, end), group) in spans.iter().zip(groups) {
        spread_one(&group, start, end, &mut segments, &mut words);
    }
    (segments, words)
}

fn weight(token: &str) -> u128 {
    token.chars().count().max(1) as u128
}

fn ends_sentence(word: &str) -> bool {
    word.trim_end_matches(['"', '\'', '”', '’', ')', ']'])
        .ends_with(['.', '!', '?', ';', ':'])
}

/// Spreads `tokens` over `start..end` by length: they tile it exactly (no gaps, no overlap), so
/// the first begins at `start` and the last ends at `end`. A segment ends at each sentence end.
fn spread_one(
    tokens: &[&str],
    start: u64,
    end: u64,
    segments: &mut Vec<TranscriptSegment>,
    words: &mut Vec<WordTiming>,
) {
    let end = end.max(start);
    let total: u128 = tokens.iter().map(|token| weight(token)).sum();
    let mut segment_start = words.len();
    let mut weight_so_far: u128 = 0;
    for (index, token) in tokens.iter().enumerate() {
        let at = |share: u128| start + (u128::from(end - start) * share / total) as u64;
        let t0 = at(weight_so_far);
        weight_so_far += weight(token);
        words.push(WordTiming {
            t0,
            t1: at(weight_so_far),
            text: (*token).to_owned(),
        });
        if ends_sentence(token) || index + 1 == tokens.len() {
            let phrase = &words[segment_start..];
            segments.push(TranscriptSegment {
                t0: phrase[0].t0,
                t1: phrase[phrase.len() - 1].t1,
                text: phrase
                    .iter()
                    .map(|word| word.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            });
            segment_start = words.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spreading_tiles_the_span_by_word_length() {
        let (segments, words) = spread("Hi there. Longer words follow", &[(1_000, 2_000)]);
        assert_eq!(words.first().unwrap().t0, 1_000);
        assert_eq!(words.last().unwrap().t1, 2_000);
        assert!(words.windows(2).all(|pair| pair[0].t1 == pair[1].t0));
        assert!(words.iter().all(|word| word.t0 <= word.t1));
        let length = |i: usize| words[i].t1 - words[i].t0;
        assert!(length(3) > length(0), "longer words get longer spans");
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "Hi there.");
        assert_eq!((segments[0].t0, segments[1].t1), (1_000, 2_000));
        assert_eq!(segments[0].t1, words[1].t1);
        assert_eq!(spread("  ", &[(0, 10)]), (vec![], vec![]));
        let (_, instant) = spread("a b", &[(500, 500)]);
        assert!(instant.iter().all(|word| word.t0 == 500 && word.t1 == 500));
    }

    #[test]
    fn spreading_over_spans_never_places_a_word_between_them() {
        let spans = [(1_000, 2_000), (5_000, 6_000), (9_000, 9_500)];
        let (segments, words) = spread("Open the menu. Then click the button now.", &spans);
        assert_eq!(words.len(), 8);
        assert!(words.iter().all(|word| {
            spans
                .iter()
                .any(|&(start, end)| word.t0 >= start && word.t1 <= end)
        }));
        assert!(words.windows(2).all(|pair| pair[0].t1 <= pair[1].t0));
        assert_eq!((words[0].t0, words[7].t1), (1_000, 9_500));
        assert!(segments.iter().all(|segment| segment.t0 <= segment.t1));
        // Two spans of equal length take half the words each.
        let (_, halves) = spread("aa bb cc dd", &[(0, 100), (1_000, 1_100)]);
        assert!(halves[..2].iter().all(|word| word.t1 <= 100));
        assert!(halves[2..].iter().all(|word| word.t0 >= 1_000));
        assert_eq!(spread("Hi.", &[]), (vec![], vec![]));
        assert_eq!(spread("", &spans), (vec![], vec![]));
    }

    /// `ms` of a square wave whose RMS level is `level`.
    fn tone(level: f32, ms: u64) -> Vec<f32> {
        (0..ms * SAMPLES_PER_MS)
            .map(|i| if i % 2 == 0 { level } else { -level })
            .collect()
    }

    fn recording(parts: &[(f32, u64)]) -> Vec<f32> {
        parts
            .iter()
            .flat_map(|&(level, ms)| tone(level, ms))
            .collect()
    }

    /// The millisecond spans of `samples` that `transcribe` hands its recognizer.
    fn heard(samples: &[f32], plan: &Plan) -> Vec<(u64, u64)> {
        let mut spans = Vec::new();
        transcribe(samples, plan, "test".into(), None, |chunk| {
            let from =
                (chunk.as_ptr() as usize - samples.as_ptr() as usize) / std::mem::size_of::<f32>();
            let to = from + chunk.len();
            spans.push((from as u64 / SAMPLES_PER_MS, to as u64 / SAMPLES_PER_MS));
            Ok(Heard::Text(String::new()))
        })
        .unwrap();
        spans
    }

    fn covers(spans: &[(u64, u64)], start: u64, end: u64) -> bool {
        spans.iter().any(|&(a, b)| a <= start && end <= b)
    }

    #[test]
    fn chunks_tile_the_whole_recording_and_stay_under_30_s() {
        let samples = recording(&[(0.05, 95_000)]);
        let plan = Plan::new(&samples);
        let chunks = plan.chunks();
        assert!(chunks.len() >= 4, "{chunks:?}");
        assert_eq!(chunks[0].0, 0);
        assert_eq!(chunks.last().unwrap().1, 95_000);
        assert!(chunks.windows(2).all(|pair| pair[0].1 == pair[1].0));
        assert!(chunks.iter().all(|&(a, b)| b > a && b - a <= MAX_CHUNK_MS));
        let sent: u64 = heard(&samples, &plan).iter().map(|(a, b)| b - a).sum();
        assert_eq!(sent, 95_000);
        assert_eq!(Plan::new(&tone(0.05, 2_000)).chunks(), [(0, 2_000)]);
        assert_eq!(Plan::new(&[]), Plan::default());
    }

    #[test]
    fn quiet_speech_beside_loud_speech_is_heard() {
        // #89: a phrase at about a quarter of the level of the one before it, over a noise floor.
        let noise = 0.004;
        let samples = recording(&[
            (noise, 1_000),
            (0.04, 1_500),
            (noise, 1_000),
            (0.011, 1_500),
            (noise, 1_000),
        ]);
        // The old planner's islands miss the quiet phrase, so Hex never got it.
        let islands = crate::audio::speech_islands(&samples);
        assert!(islands.iter().all(|&(_, end)| end <= 2_500), "{islands:?}");
        let plan = Plan::new(&samples);
        assert!(covers(&heard(&samples, &plan), 3_500, 5_000));
        assert!(covers(plan.voiced(), 3_500, 5_000), "{:?}", plan.voiced());
    }

    #[test]
    fn a_lone_short_word_is_heard() {
        // #90: a 100 ms word on its own, shorter than any island the old planner kept.
        let samples = recording(&[
            (0.0, 500),
            (0.05, 1_000),
            (0.0, 1_500),
            (0.05, 100),
            (0.0, 900),
        ]);
        assert!(crate::audio::speech_islands(&samples).contains(&(3_000, 3_100)));
        let plan = Plan::new(&samples);
        assert!(covers(&heard(&samples, &plan), 3_000, 3_100));
        assert!(covers(plan.voiced(), 3_000, 3_100));
    }

    #[test]
    fn a_cut_lands_on_the_quietest_window() {
        let mut samples = tone(0.05, 40_000);
        let window =
            |ms: u64| (ms * SAMPLES_PER_MS) as usize..((ms + 50) * SAMPLES_PER_MS) as usize;
        samples[window(18_000)].iter_mut().for_each(|s| *s *= 0.2);
        samples[window(21_000)].iter_mut().for_each(|s| *s *= 0.02);
        samples[window(35_000)].fill(0.0); // Past 30 s: out of reach of the first cut.
        assert_eq!(
            Plan::new(&samples).chunks(),
            [(0, 21_025), (21_025, 40_000)]
        );
    }

    #[test]
    fn silence_and_steady_noise_send_nothing() {
        for samples in [vec![0.0; 16_000 * 10], tone(0.004, 10_000)] {
            let plan = Plan::new(&samples);
            assert!(plan.chunks().is_empty() && plan.voiced().is_empty());
            let transcript = transcribe(&samples, &plan, "test".into(), None, |_| {
                panic!("nothing should be sent")
            })
            .unwrap();
            assert!(transcript.text.is_empty() && transcript.words.is_empty());
        }
        // A noise-only chunk after speech is left out too.
        let samples = recording(&[(0.05, 10_000), (0.004, 40_000)]);
        assert_eq!(Plan::new(&samples).chunks(), [(0, 29_975)]);
    }

    #[test]
    fn text_is_spread_over_the_voiced_spans_of_its_own_chunk() {
        let plan = Plan {
            chunks: vec![(0, 10_000), (10_000, 20_000), (20_000, 30_000)],
            voiced: vec![(1_000, 2_000), (4_000, 5_000)],
        };
        let samples = vec![0.0; 16_000 * 30];
        let mut answers = ["First part here. Then more", "  ", "No voiced span"].into_iter();
        let transcript = transcribe(&samples, &plan, "hex:m".into(), Some("m".into()), |_| {
            Ok(Heard::Text(answers.next().unwrap().into()))
        })
        .unwrap();
        assert_eq!(transcript.source, "hex:m");
        assert_eq!(transcript.model.as_deref(), Some("m"));
        assert!(transcript.estimated);
        assert_eq!(transcript.text, "First part here. Then more No voiced span");
        let (voiced, unvoiced) = transcript.words.split_at(5);
        assert!(voiced.iter().all(|word| {
            (1_000..=2_000).contains(&word.t0) && word.t1 <= 2_000
                || (4_000..=5_000).contains(&word.t0) && word.t1 <= 5_000
        }));
        assert_eq!((unvoiced[0].t0, unvoiced[2].t1), (20_000, 30_000));
        assert!(
            transcript
                .words
                .windows(2)
                .all(|pair| pair[0].t1 <= pair[1].t0)
        );
    }

    #[test]
    fn any_failure_fails_the_whole_recording() {
        let samples = recording(&[(0.05, 45_000)]);
        let plan = Plan::new(&samples);
        assert_eq!(plan.chunks().len(), 2);
        let mut calls = 0;
        let result = transcribe(&samples, &plan, "test".into(), None, |_| {
            calls += 1;
            if calls == 2 {
                anyhow::bail!("refused")
            }
            Ok(Heard::Text("kept".into()))
        });
        assert!(result.is_err());
        assert_eq!(calls, 2);
    }

    #[test]
    fn timed_answers_move_to_the_recording_clock() {
        let plan = Plan {
            chunks: vec![(0, 10_000), (10_000, 20_000), (20_000, 30_000)],
            voiced: vec![(22_000, 23_000)],
        };
        let samples = vec![0.0; 16_000 * 30];
        let word = |t0, t1, text: &str| WordTiming {
            t0,
            t1,
            text: text.into(),
        };
        let mut answers = [
            Heard::Timed {
                text: "Look here.".into(),
                words: vec![word(100, 300, "Look"), word(300, 600, "here.")],
                segments: vec![TranscriptSegment {
                    t0: 100,
                    t1: 600,
                    text: "Look here.".into(),
                }],
            },
            Heard::Timed {
                text: "Now this".into(),
                words: vec![word(50, 40, "Now"), word(200, 400, "this")],
                segments: Vec::new(),
            },
        ]
        .into_iter();
        let transcript = transcribe(&samples, &plan, "parakeet:m".into(), None, |_| {
            Ok(answers.next().unwrap_or(Heard::Text(String::new())))
        })
        .unwrap();
        assert!(!transcript.estimated);
        let times: Vec<_> = transcript.words.iter().map(|w| (w.t0, w.t1)).collect();
        assert_eq!(
            times,
            [(100, 300), (300, 600), (10_050, 10_050), (10_200, 10_400)]
        );
        assert_eq!(transcript.segments.len(), 2);
        assert_eq!(
            (transcript.segments[1].t0, transcript.segments[1].t1),
            (10_050, 10_400)
        );
        assert_eq!(transcript.text, "Look here. Now this");
        // A timed answer without words is spread like text, and says so.
        let mut once = Some(Heard::Timed {
            text: "no times".into(),
            words: Vec::new(),
            segments: Vec::new(),
        });
        let spread_out = transcribe(&samples, &plan, "parakeet:m".into(), None, |_| {
            Ok(once.take().unwrap_or(Heard::Text(String::new())))
        })
        .unwrap();
        assert!(spread_out.estimated);
        assert_eq!(spread_out.words.len(), 2);
    }

    #[test]
    fn low_gain_speech_under_the_voiced_threshold_is_still_heard() {
        // Speech so quiet that no window reaches twice the noise floor: syllables at 1.6x a
        // 0.004 floor, and a whisper at 0.0008 over digital silence (under 0.001).
        let syllables: Vec<(f32, u64)> =
            (0..8).flat_map(|_| [(0.0065, 200), (0.004, 200)]).collect();
        let whisper: Vec<(f32, u64)> = (0..8).flat_map(|_| [(0.0008, 200), (0.0, 200)]).collect();
        for samples in [recording(&syllables), recording(&whisper)] {
            let plan = Plan::new(&samples);
            assert!(plan.voiced().is_empty(), "{:?}", plan.voiced());
            assert_eq!(heard(&samples, &plan), [(0, 3_200)]);
        }
        // Too short to be worth a request, as before.
        assert!(Plan::new(&recording(&syllables[..2])).chunks().is_empty());
    }

    /// `ms` of Gaussian noise at RMS `level`, low-passed at `cutoff_hz` (0 for white), from a
    /// fixed seed.
    fn noise(level: f64, ms: u64, cutoff_hz: f64) -> Vec<f32> {
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut uniform = || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            ((seed >> 11) as f64 + 1.0) / (1u64 << 53) as f64
        };
        let white: Vec<f64> = (0..ms * SAMPLES_PER_MS)
            .map(|_| (-2.0 * uniform().ln()).sqrt() * (std::f64::consts::TAU * uniform()).cos())
            .collect();
        let shaped = if cutoff_hz > 0.0 {
            let a = (-std::f64::consts::TAU * cutoff_hz / 16_000.0).exp();
            let mut y = 0.0;
            white
                .iter()
                .map(|&x| {
                    y = a * y + (1.0 - a) * x;
                    y
                })
                .collect()
        } else {
            white
        };
        let rms = (shaped.iter().map(|x| x * x).sum::<f64>() / shaped.len() as f64).sqrt();
        shaped.iter().map(|x| (x / rms * level) as f32).collect()
    }

    #[test]
    fn white_noise_and_a_rumbling_tail_after_speech_send_nothing_extra() {
        assert!(Plan::new(&noise(0.004, 30_000, 0.0)).chunks().is_empty());
        let mut samples = tone(0.05, 10_000);
        samples.extend(noise(0.004, 40_000, 100.0));
        let chunks = Plan::new(&samples).chunks().to_vec();
        assert_eq!(chunks.len(), 1, "{chunks:?}");
        assert_eq!(chunks[0].0, 0);
    }

    #[test]
    fn quiet_speech_the_old_planner_sent_whole_is_still_heard_beside_voiced_speech() {
        // A 0.002 phrase is voiced but under the old island level, so the old planner sent the
        // whole recording; the later whisper's chunk has no voiced window of its own.
        let samples = recording(&[(0.002, 1_000), (0.0, 29_000), (0.0008, 200), (0.0, 4_800)]);
        let plan = Plan::new(&samples);
        assert!(crate::audio::speech_islands(&samples).is_empty());
        assert!(
            covers(&heard(&samples, &plan), 30_000, 30_200),
            "{:?}",
            plan.chunks()
        );
    }

    #[test]
    fn quiet_speech_under_the_old_island_level_keeps_the_old_fallback() {
        // Voiced speech at 0.003 that the old planner never counted as an island, then a word
        // under the voiced level more than 30 s later: the old planner sent all of it.
        let mut samples = noise(0.0003, 60_000, 0.0);
        let louder = |range: std::ops::Range<u64>, level: f32, samples: &mut Vec<f32>| {
            for (i, s) in samples
                [(range.start * SAMPLES_PER_MS) as usize..(range.end * SAMPLES_PER_MS) as usize]
                .iter_mut()
                .enumerate()
            {
                *s += if i % 2 == 0 { level } else { -level };
            }
        };
        louder(0..10_000, 0.003, &mut samples);
        louder(45_000..46_000, 0.0008, &mut samples);
        let plan = Plan::new(&samples);
        assert!(!plan.voiced().is_empty());
        assert!(
            covers(&heard(&samples, &plan), 45_000, 46_000),
            "{:?}",
            plan.chunks()
        );
        // A recording of rumble alone is still sent, as the old planner sent it whole.
        let rumble = noise(0.004, 30_000, 100.0);
        assert_eq!(Plan::new(&rumble).chunks(), [(0, 30_000)]);
    }

    #[test]
    fn a_quiet_word_after_a_long_silence_is_heard() {
        // The cut leaves a last chunk of a few hundred ms holding the only (quiet) word.
        let samples = recording(&[(0.0, 30_000), (0.0008, 200), (0.0, 100)]);
        let plan = Plan::new(&samples);
        assert!(plan.voiced().is_empty());
        assert!(covers(&heard(&samples, &plan), 30_000, 30_200));
    }
}
