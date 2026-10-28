//! Shows what the microphone hears while recording (`"liveAudio": true`, Settings › Recording ›
//! Show live audio): a waveform in the recording capsule, so a dead, muted or wrong microphone
//! shows at once, and, while Apple Speech transcribes, the words as they're said under it.
//!
//! `AudioRecorder` publishes its 16 kHz buffer here (`begin`) for as long as it records, and the
//! capsule reads it as it draws: `Levels` turns new samples into bars, `Caption` holds the words.
//! Nothing here changes what's recorded: the waveform only reads the buffer, and the words come
//! from a `transcribe-stream` that's thrown away when the recording stops (unless the
//! `liveTranscribe` experiment already streams, in which case its stream shows them too).

// Only the native (macOS) capsule draws the waveform and the words.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

const SAMPLE_RATE: usize = 16_000;
/// The audio each waveform bar stands for.
pub const BUCKET: usize = SAMPLE_RATE / 20;
/// Bars kept, more than the widest waveform shows.
const HISTORY: usize = 160;
/// No new samples for this long and the waveform says the microphone stopped.
const STALL: Duration = Duration::from_millis(600);
/// Below this a bar is silence; at `LOUD` it's full height (dBFS of the bucket's RMS).
const QUIET_DB: f32 = -54.0;
const LOUD_DB: f32 = -12.0;

/// Whether recordings should show what the microphone hears. Reads the config, so once per
/// recording, never per frame.
pub fn wanted() -> bool {
    cfg!(target_os = "macos") && crate::handoff::live_audio()
}

/// Whether the words can show too: Apple Speech streams (`transcribe-stream`); Hex and a command
/// transcribe only after the recording.
pub fn captions_possible() -> bool {
    cfg!(target_os = "macos")
        && crate::transcribe::Engine::preferred() == crate::transcribe::Engine::Apple
}

/// What a recording shares with the capsule.
#[derive(Clone)]
pub struct Feed {
    pub id: u64,
    pub samples: Arc<Mutex<Vec<f32>>>,
    pub caption: Option<Arc<Mutex<Caption>>>,
    pub started: Instant,
}

static CURRENT: Mutex<Option<Feed>> = Mutex::new(None);

/// Publishes a recording's buffer (and its caption, when the words show); `end` with the
/// returned id takes it down. A newer recording replaces an older one.
pub fn begin(samples: Arc<Mutex<Vec<f32>>>, caption: Option<Arc<Mutex<Caption>>>) -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    *CURRENT.lock().unwrap_or_else(PoisonError::into_inner) = Some(Feed {
        id,
        samples,
        caption,
        started: Instant::now(),
    });
    id
}

/// Takes down the feed `begin` returned `id` for, unless a newer one replaced it.
pub fn end(id: u64) {
    let mut current = CURRENT.lock().unwrap_or_else(PoisonError::into_inner);
    if current.as_ref().is_some_and(|feed| feed.id == id) {
        *current = None;
    }
}

/// The recording being shown, if any.
pub fn current() -> Option<Feed> {
    CURRENT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// What the capsule makes room for: the waveform while a recording shows its audio, and the
/// words' row once the transcriber is listening.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rows {
    pub wave: bool,
    pub caption: bool,
}

pub fn rows() -> Rows {
    let Some(feed) = current() else {
        return Rows::default();
    };
    let caption = feed.caption.as_ref().is_some_and(|caption| {
        caption.lock().unwrap_or_else(PoisonError::into_inner).state == CaptionState::Listening
    });
    Rows {
        wave: true,
        caption,
    }
}

// MARK: Waveform

/// How loud a stretch of samples looks: 0 for silence, 1 for loud speech close to the mic.
pub fn level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mean = samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / samples.len() as f64;
    let db = 10.0 * mean.max(1e-12).log10() as f32;
    ((db - QUIET_DB) / (LOUD_DB - QUIET_DB)).clamp(0.0, 1.0)
}

/// Turns a recording's growing buffer into one level per `BUCKET`, newest last.
pub struct Levels {
    feed: u64,
    read: usize,
    partial: Vec<f32>,
    bars: VecDeque<f32>,
    heard_at: Option<Instant>,
    started: Option<Instant>,
    shown_stalled: bool,
}

impl Default for Levels {
    fn default() -> Self {
        Self {
            feed: 0,
            read: 0,
            partial: Vec::with_capacity(BUCKET),
            bars: VecDeque::with_capacity(HISTORY),
            heard_at: None,
            started: None,
            shown_stalled: false,
        }
    }
}

impl Levels {
    /// Reads what `feed` heard since the last call (from the start for a new feed). Returns
    /// whether the waveform changed.
    pub fn update(&mut self, feed: &Feed, now: Instant) -> bool {
        if feed.id != self.feed {
            *self = Self {
                feed: feed.id,
                started: Some(feed.started),
                ..Self::default()
            };
        }
        let fresh = {
            let samples = feed.samples.lock().unwrap_or_else(PoisonError::into_inner);
            // The recorder empties its buffer when it stops.
            samples
                .get(self.read..)
                .map(<[f32]>::to_vec)
                .unwrap_or_default()
        };
        if fresh.is_empty() {
            let stalled = self.stalled(now);
            let changed = stalled != self.shown_stalled;
            self.shown_stalled = stalled;
            return changed;
        }
        self.read += fresh.len();
        self.heard_at = Some(now);
        self.shown_stalled = false;
        self.push(&fresh);
        true
    }

    fn push(&mut self, mut fresh: &[f32]) {
        while !fresh.is_empty() {
            let take = (BUCKET - self.partial.len()).min(fresh.len());
            self.partial.extend_from_slice(&fresh[..take]);
            fresh = &fresh[take..];
            if self.partial.len() == BUCKET {
                if self.bars.len() == HISTORY {
                    self.bars.pop_front();
                }
                self.bars.push_back(level(&self.partial));
                self.partial.clear();
            }
        }
    }

    /// The last `n` bars, oldest first, padded with silence at the start.
    pub fn bars(&self, n: usize) -> impl Iterator<Item = f32> + '_ {
        let have = self.bars.len().min(n);
        std::iter::repeat_n(0.0, n - have)
            .chain(self.bars.iter().skip(self.bars.len() - have).copied())
    }

    /// How far into the next bar the audio is (0…1), so the waveform scrolls smoothly.
    pub fn progress(&self) -> f32 {
        self.partial.len() as f32 / BUCKET as f32
    }

    /// The level of the bar being filled, so the newest bar rises as it's heard.
    pub fn filling(&self) -> f32 {
        level(&self.partial)
    }

    /// Whether the microphone has gone quiet in the sense of sending nothing at all (unplugged,
    /// stuck, or never opened): no samples for `STALL`, counted from the start of the recording.
    pub fn stalled(&self, now: Instant) -> bool {
        match (self.heard_at, self.started) {
            (Some(heard), _) => now.saturating_duration_since(heard) > STALL,
            (None, Some(started)) => now.saturating_duration_since(started) > STALL,
            (None, None) => false,
        }
    }
}

// MARK: Words

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaptionState {
    /// The transcriber is starting (loading its model).
    #[default]
    Starting,
    Listening,
    /// It couldn't start or stopped; the row goes away.
    Failed,
}

/// The words heard so far: SpeechAnalyzer's finalized results, then its latest guess at the words
/// still being said, which each new guess replaces and a finalized result clears.
#[derive(Clone, Debug, Default)]
pub struct Caption {
    finalized: String,
    volatile: String,
    pub state: CaptionState,
    /// Bumps on every change, so the capsule redraws the text only when it changed.
    pub version: u64,
}

impl Caption {
    pub fn listening(&mut self) {
        if self.state == CaptionState::Starting {
            self.state = CaptionState::Listening;
            self.version += 1;
        }
    }

    pub fn finalized(&mut self, text: &str) {
        push_words(&mut self.finalized, text);
        self.volatile.clear();
        self.listening();
        self.version += 1;
    }

    pub fn volatile(&mut self, text: &str) {
        self.volatile.clear();
        push_words(&mut self.volatile, text);
        self.listening();
        self.version += 1;
    }

    pub fn failed(&mut self) {
        if self.state != CaptionState::Failed {
            self.state = CaptionState::Failed;
            self.version += 1;
        }
    }

    /// Everything heard, one space between words.
    pub fn text(&self) -> String {
        let mut text = self.finalized.clone();
        push_words(&mut text, &self.volatile);
        text
    }
}

fn push_words(out: &mut String, text: &str) {
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
}

/// Lays `text` (single spaces between words) out from byte `start` in lines at most `width`
/// wide by `measure`, dropping whole lines from the top while there are more than `max_lines`.
/// Returns the new start and the lines. Passing the returned start back as the text grows keeps
/// the lines already shown where they are, so the words don't jump around as they come in.
pub fn wrap(
    text: &str,
    start: usize,
    width: f32,
    max_lines: usize,
    measure: impl Fn(&str) -> f32,
) -> (usize, Vec<String>) {
    // A guess that shrank can leave `start` past the end or inside a word: go back to a word.
    let mut start = start.min(text.len());
    while start > 0 && !(text.is_char_boundary(start) && text[..start].ends_with(' ')) {
        start -= 1;
    }
    loop {
        let mut lines: Vec<String> = Vec::new();
        for word in text[start..].split(' ').filter(|w| !w.is_empty()) {
            match lines.last_mut() {
                Some(line) if measure(&format!("{line} {word}")) <= width => {
                    line.push(' ');
                    line.push_str(word);
                }
                _ => lines.push(word.to_owned()),
            }
        }
        if lines.len() <= max_lines.max(1) {
            return (start, lines);
        }
        start += lines[0].len() + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(id: u64, samples: Vec<f32>) -> Feed {
        Feed {
            id,
            samples: Arc::new(Mutex::new(samples)),
            caption: None,
            started: Instant::now(),
        }
    }

    #[test]
    fn silence_is_flat_and_speech_is_tall() {
        assert_eq!(level(&[]), 0.0);
        assert_eq!(level(&[0.0; BUCKET]), 0.0);
        // Room noise around -60 dBFS stays flat.
        assert_eq!(level(&[0.001; BUCKET]), 0.0);
        // Ordinary speech (about -26 dBFS RMS) is well up but not clipped.
        let speech = level(&[0.05; BUCKET]);
        assert!((0.5..0.85).contains(&speech), "{speech}");
        assert_eq!(level(&[0.9; BUCKET]), 1.0);
    }

    #[test]
    fn bars_follow_the_buffer_one_bucket_at_a_time() {
        let now = Instant::now();
        let feed = feed(1, vec![0.0; BUCKET]);
        let mut levels = Levels::default();
        assert!(levels.update(&feed, now));
        assert_eq!(levels.bars(3).collect::<Vec<_>>(), vec![0.0, 0.0, 0.0]);
        feed.samples
            .lock()
            .unwrap()
            .extend([0.5; BUCKET + BUCKET / 2]);
        assert!(levels.update(&feed, now));
        let bars: Vec<f32> = levels.bars(3).collect();
        assert_eq!(bars[..2], [0.0, 0.0]);
        assert!(bars[2] > 0.9, "{bars:?}");
        assert_eq!(levels.progress(), 0.5);
        assert!(levels.filling() > 0.9);
        // Nothing new: nothing changed.
        assert!(!levels.update(&feed, now));
    }

    #[test]
    fn a_new_recording_starts_a_new_waveform_and_a_stopped_one_freezes() {
        let now = Instant::now();
        let mut levels = Levels::default();
        levels.update(&feed(1, vec![0.5; BUCKET * 4]), now);
        assert_eq!(levels.bars(8).filter(|b| *b > 0.0).count(), 4);
        let second = feed(2, vec![0.0; BUCKET]);
        levels.update(&second, now);
        assert_eq!(levels.bars(8).filter(|b| *b > 0.0).count(), 0);
        // The recorder empties its buffer when it stops; the bars stay as they were.
        second.samples.lock().unwrap().clear();
        assert!(!levels.update(&second, now));
        assert_eq!(levels.bars(1).count(), 1);
    }

    #[test]
    fn a_microphone_that_sends_nothing_shows_as_stalled() {
        let now = Instant::now();
        let quiet = feed(1, Vec::new());
        let mut levels = Levels::default();
        levels.update(&quiet, now);
        assert!(!levels.stalled(now));
        assert!(levels.stalled(quiet.started + STALL + Duration::from_millis(50)));
        // Samples arriving (even silent ones) mean it's working.
        quiet.samples.lock().unwrap().extend([0.0; 160]);
        let later = now + Duration::from_secs(2);
        levels.update(&quiet, later);
        assert!(!levels.stalled(later));
        assert!(levels.stalled(later + STALL + Duration::from_millis(1)));
    }

    #[test]
    fn guesses_replace_each_other_until_finalized() {
        let mut caption = Caption::default();
        assert_eq!(caption.state, CaptionState::Starting);
        caption.volatile("the");
        assert_eq!(caption.state, CaptionState::Listening);
        caption.volatile("the button  is");
        assert_eq!(caption.text(), "the button is");
        caption.finalized(" The button is broken.");
        assert_eq!(caption.text(), "The button is broken.");
        caption.volatile("Look");
        caption.volatile("Look here");
        assert_eq!(caption.text(), "The button is broken. Look here");
        caption.finalized("Look here.");
        assert_eq!(caption.text(), "The button is broken. Look here.");
        let before = caption.version;
        caption.failed();
        assert_eq!(caption.state, CaptionState::Failed);
        assert!(caption.version > before);
    }

    #[test]
    fn wrapping_keeps_shown_lines_still_and_scrolls_by_whole_lines() {
        // Every character is 1 wide.
        let measure = |s: &str| s.chars().count() as f32;
        let (start, lines) = wrap("one two three", 0, 9.0, 2, measure);
        assert_eq!((start, lines), (0, vec!["one two".into(), "three".into()]));
        let text = "one two three four five";
        let (start, lines) = wrap(text, 0, 9.0, 2, measure);
        assert_eq!(&text[start..], "three four five");
        assert_eq!(lines, vec!["three", "four five"]);
        // More words: the top line goes, the rest stay put.
        let text = "one two three four five six seven";
        let (start, lines) = wrap(text, start, 9.0, 2, measure);
        assert_eq!(lines, vec!["four five", "six seven"]);
        // The guess shrinks under the start: back to a word boundary, never mid-word.
        let (start, lines) = wrap("one two three fo", start, 9.0, 2, measure);
        assert!("one two three fo".is_char_boundary(start));
        assert_eq!(lines.last().unwrap(), "fo");
        // A word longer than the width gets a line of its own.
        let (_, lines) = wrap("a supercalifragilistic b", 0, 9.0, 3, measure);
        assert_eq!(lines, vec!["a", "supercalifragilistic", "b"]);
        assert_eq!(wrap("", 0, 9.0, 2, measure), (0, Vec::<String>::new()));
    }

    #[test]
    fn only_the_newest_feed_is_shown() {
        let first = begin(Arc::new(Mutex::new(Vec::new())), None);
        let second = begin(Arc::new(Mutex::new(Vec::new())), None);
        end(first);
        assert_eq!(current().map(|f| f.id), Some(second));
        assert!(rows().wave);
        assert!(!rows().caption);
        end(second);
        assert!(current().is_none());
        assert_eq!(rows(), Rows::default());
    }
}
