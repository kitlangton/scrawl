//! The lab's scenes: what happens when, on the animation clock. Both capsules play them from
//! here (the Metal panel through the coordinator, `--rune` through its `Surface`), stepped by one
//! `Timeline`, so their traces come from the same steps at the same times.

use crate::ui::SelectionMode;

/// How long a pretend recording "processes" before it's done.
pub(in crate::ui) const PROCESSING_S: f32 = 0.8;
/// Quiet time before a scene's first step, so motion-lint's launch settle passes and a screen
/// recording starts on a still frame.
const LEAD_IN_S: f32 = 0.85;
/// Quiet time after the last step (and between repeats).
pub(crate) const TAIL_S: f32 = 0.75;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Scene {
    /// Tap: recording + chips, then the Area chip turns it into the picker.
    TapThenArea,
    /// Tap: recording + chips, then the Window chip turns it into the picker.
    TapThenWindow,
    /// Tap: recording + chips, then the Screen chip folds them back.
    TapThenScreen,
    /// The menu's picker opens on Screen, a click on Area, then Screen, then Screen again records.
    Picker,
    /// Tap, chips fold, finish: processing, then done, then dismissed.
    Finish,
    /// Tap: recording + chips, then cancel straight from the recording.
    Cancel,
    /// Open the picker, then dismiss it.
    Dismiss,
    /// Every scene above, one after another (`--repeat 0` loops them).
    All,
    /// With `--rune`: every capsule state at once, live, in one ordinary window.
    Grid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum Step {
    Tap,
    Chip(SelectionMode),
    Open,
    Click(SelectionMode),
    Cancel,
    Finish,
    Dismiss,
}

impl Scene {
    pub fn name(self) -> &'static str {
        match self {
            Self::TapThenArea => "tap-then-area",
            Self::TapThenWindow => "tap-then-window",
            Self::TapThenScreen => "tap-then-screen",
            Self::Picker => "picker",
            Self::Finish => "finish",
            Self::Cancel => "cancel",
            Self::Dismiss => "dismiss",
            Self::All => "all",
            Self::Grid => "grid",
        }
    }

    /// Every scene but `All`, in the order `All` plays them.
    pub const EACH: [Scene; 7] = [
        Self::TapThenArea,
        Self::TapThenWindow,
        Self::TapThenScreen,
        Self::Picker,
        Self::Finish,
        Self::Cancel,
        Self::Dismiss,
    ];

    /// Steps at animation seconds from the scene's start.
    fn steps(self) -> Vec<(f32, Step)> {
        use SelectionMode::{Area, Screen, Window};
        match self {
            Self::TapThenArea => vec![
                (0.0, Step::Tap),
                (1.2, Step::Chip(Area)),
                (2.4, Step::Cancel),
            ],
            Self::TapThenWindow => {
                vec![
                    (0.0, Step::Tap),
                    (1.2, Step::Chip(Window)),
                    (2.4, Step::Cancel),
                ]
            }
            Self::TapThenScreen => {
                vec![
                    (0.0, Step::Tap),
                    (1.2, Step::Chip(Screen)),
                    (2.4, Step::Cancel),
                ]
            }
            Self::Picker => vec![
                (0.0, Step::Open),
                (0.9, Step::Click(Area)),
                (1.8, Step::Click(Screen)),
                (2.7, Step::Click(Screen)),
                (3.9, Step::Cancel),
            ],
            Self::Finish => vec![
                (0.0, Step::Tap),
                (0.9, Step::Chip(Screen)),
                (2.0, Step::Finish),
                (2.0 + PROCESSING_S + 1.4, Step::Dismiss),
            ],
            Self::Cancel => vec![(0.0, Step::Tap), (1.2, Step::Cancel)],
            Self::Dismiss => vec![(0.0, Step::Open), (1.2, Step::Dismiss)],
            // The grid runs its own cells (`rune_pill::grid`).
            Self::Grid => Vec::new(),
            Self::All => {
                // Each scene starts after the last one's tail and a lead-in, as a repeat would.
                let mut steps = Vec::new();
                let mut start = 0.0;
                for scene in Self::EACH {
                    let own = scene.steps();
                    let end = own.last().map_or(0.0, |(at, _)| *at);
                    steps.extend(own.into_iter().map(|(at, step)| (start + at, step)));
                    start += end + TAIL_S + LEAD_IN_S;
                }
                steps
            }
        }
    }
}


/// Plays a scene's steps on the animation clock: the lead-in, each step at its time, the tail,
/// then again (`repeat` 0 loops) or done. The caller reads the clock and applies the steps; the
/// timeline only says which are due.
pub(in crate::ui) struct Timeline {
    steps: Vec<(f32, Step)>,
    repeat: u32,
    next: usize,
    runs: u32,
    /// Animation seconds the current run's step times count from; set by the first `advance`.
    start: Option<f64>,
}

/// What `Timeline::advance` found due.
#[derive(Debug, Default, PartialEq)]
pub(in crate::ui) struct Due {
    pub steps: Vec<Step>,
    /// The last run's tail has passed: the lab quits.
    pub done: bool,
}

impl Timeline {
    pub fn new(scene: Scene, repeat: u32) -> Self {
        Self { steps: scene.steps(), repeat, next: 0, runs: 0, start: None }
    }

    fn end(&self, start: f64) -> f64 {
        start + f64::from(self.steps.last().map_or(0.0, |(at, _)| *at) + TAIL_S)
    }

    /// The steps due at animation second `anim`, in order. The first call starts the lead-in.
    pub fn advance(&mut self, anim: f64) -> Due {
        let start = *self.start.get_or_insert(anim + f64::from(LEAD_IN_S));
        let mut due = Due::default();
        while let Some(&(at, step)) = self.steps.get(self.next)
            && start + f64::from(at) <= anim
        {
            self.next += 1;
            due.steps.push(step);
        }
        if self.next >= self.steps.len() && anim >= self.end(start) {
            self.runs += 1;
            if self.repeat != 0 && self.runs >= self.repeat {
                due.done = true;
            } else {
                self.next = 0;
                self.start = Some(anim + f64::from(LEAD_IN_S));
            }
        }
        due
    }

    /// Animation seconds of the next step, or of the end of the run's tail.
    pub fn next_at(&self) -> Option<f64> {
        let start = self.start?;
        Some(match self.steps.get(self.next) {
            Some((at, _)) => start + f64::from(*at),
            None => self.end(start),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timeline_plays_lead_in_steps_and_tail_then_stops() {
        let near = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() < 1e-4);
        let mut t = Timeline::new(Scene::Cancel, 1);
        assert_eq!(t.advance(10.0), Due::default(), "the first call starts the lead-in");
        assert!(near(t.next_at(), 10.85));
        assert!(t.advance(10.84).steps.is_empty());
        assert_eq!(t.advance(10.86).steps, [Step::Tap]);
        assert!(t.advance(12.0).steps.is_empty());
        assert_eq!(t.advance(12.06).steps, [Step::Cancel]);
        assert!(near(t.next_at(), 10.85 + 1.2 + 0.75));
        assert!(!t.advance(12.79).done);
        assert!(t.advance(12.81).done);
    }

    #[test]
    fn late_reads_still_get_every_step_in_order() {
        let mut t = Timeline::new(Scene::Picker, 0);
        t.advance(0.0);
        let due = t.advance(100.0);
        assert_eq!(due.steps.len(), 5);
        assert!(!due.done, "repeat 0 loops");
        assert!(t.next_at().is_some_and(|at| at > 100.0));
    }

    #[test]
    fn all_plays_every_scene_in_order_with_gaps() {
        let all = Scene::All.steps();
        let count: usize = Scene::EACH.iter().map(|s| s.steps().len()).sum();
        assert_eq!(all.len(), count);
        assert!(all.windows(2).all(|w| w[0].0 < w[1].0), "steps keep their order");
        let first = Scene::EACH[0].steps();
        let second_start = first.last().unwrap().0 + TAIL_S + LEAD_IN_S;
        assert_eq!(all[first.len()], (second_start, Step::Tap));
    }
}
