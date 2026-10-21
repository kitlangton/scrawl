//! The capture shortcut. Press ⌥⇧S, briefly or held, and the display under the pointer starts
//! recording as the keys come up; the recording's capsule shows Area · Window · Screen · iPhone
//! as chips for a few seconds (`Reveal`), so the target can still change.

/// A press this soon after a recording starts is a double press, not "finish".
pub const DOUBLE_PRESS_MS: u64 = 400;

/// What Scrawl is doing when the shortcut goes down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    /// Nothing recording (a finished capture's island may be showing).
    Free,
    /// The picker (the menu's Choose What to Record…) is open.
    Picker,
    Recording {
        elapsed_ms: u64,
    },
    Processing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// The keys came up: record the display under the pointer, auto-cropped.
    RecordScreen,
    /// Esc while the keys were still down: nothing happens.
    Cancel,
    /// The picker is open: record what it is on.
    RecordSelected,
    Finish,
}

/// Waits for the shortcut's keys to come up, however long they're held, before recording.
#[derive(Debug, Default)]
pub struct Shortcut {
    /// ⌥⇧S went down and its modifiers aren't all up yet.
    pending: bool,
}

impl Shortcut {
    /// Whether ⌥⇧S went down and the recording waits for its keys to come up.
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// The shortcut went down.
    pub fn press(&mut self, activity: Activity) -> Action {
        if self.pending {
            return Action::None;
        }
        match activity {
            Activity::Recording { elapsed_ms } if elapsed_ms < DOUBLE_PRESS_MS => Action::None,
            Activity::Recording { .. } => Action::Finish,
            Activity::Processing => Action::None,
            Activity::Picker => Action::RecordSelected,
            Activity::Free => {
                self.pending = true;
                Action::None
            }
        }
    }

    /// Polled every few ms while pending, with whether the shortcut's modifiers are all still
    /// down.
    pub fn tick(&mut self, held: bool) -> Action {
        if self.pending && !held {
            self.pending = false;
            Action::RecordScreen
        } else {
            Action::None
        }
    }

    pub fn escape(&mut self) -> Action {
        if std::mem::take(&mut self.pending) {
            Action::Cancel
        } else {
            Action::None
        }
    }
}

/// How long a tap-started recording shows its switch chips when nothing else happens.
pub const REVEAL_MS: u64 = 3_500;
/// The chips stay at least this long even if work starts at once, so they are seen.
pub const REVEAL_MIN_MS: u64 = 900;
/// After the pointer leaves the capsule the chips stay this much longer.
pub const REVEAL_LINGER_MS: u64 = 1_200;

/// The switch chips a tap-started recording shows in its capsule (Area · Window · Screen ·
/// iPhone), so a tap teaches that the target can change. They fold back into the recording pill
/// after `REVEAL_MS`, or once the user starts working (a click, scroll or stroke), but not while
/// the pointer is on the capsule.
#[derive(Debug, Default)]
pub struct Reveal {
    open: Option<RevealOpen>,
}

#[derive(Clone, Copy, Debug)]
struct RevealOpen {
    since_ms: u64,
    /// Activity count when the chips appeared; more than this means the user got to work.
    baseline: usize,
    hovered_at_ms: Option<u64>,
}

impl Reveal {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn show(&mut self, now_ms: u64, activity: usize) {
        self.open = Some(RevealOpen {
            since_ms: now_ms,
            baseline: activity,
            hovered_at_ms: None,
        });
    }

    pub fn close(&mut self) {
        self.open = None;
    }

    /// Polled every frame; returns true on the frame the chips fold away.
    pub fn tick(&mut self, now_ms: u64, hovered: bool, activity: usize) -> bool {
        let Some(open) = self.open.as_mut() else {
            return false;
        };
        if hovered {
            open.hovered_at_ms = Some(now_ms);
            return false;
        }
        let age = now_ms.saturating_sub(open.since_ms);
        if age < REVEAL_MIN_MS {
            return false;
        }
        let linger_until = open.hovered_at_ms.map_or(0, |at| at + REVEAL_LINGER_MS);
        let deadline = (open.since_ms + REVEAL_MS).max(linger_until);
        let working = activity > open.baseline && now_ms >= linger_until;
        if now_ms >= deadline || working {
            self.open = None;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FREE: Activity = Activity::Free;

    #[test]
    fn a_tap_records_the_screen_when_the_keys_come_up() {
        let mut s = Shortcut::default();
        assert_eq!(s.press(FREE), Action::None);
        assert_eq!(s.tick(true), Action::None);
        assert_eq!(s.tick(false), Action::RecordScreen);
        assert!(!s.is_pending());
        assert_eq!(s.tick(false), Action::None);
    }

    #[test]
    fn holding_records_the_screen_too_once_let_go() {
        let mut s = Shortcut::default();
        s.press(FREE);
        for _ in 0..500 {
            assert_eq!(s.tick(true), Action::None);
        }
        // S pressed again while ⌥⇧ is still down changes nothing.
        assert_eq!(s.press(FREE), Action::None);
        assert_eq!(s.tick(false), Action::RecordScreen);
    }

    #[test]
    fn esc_before_the_keys_come_up_cancels_it() {
        let mut s = Shortcut::default();
        s.press(FREE);
        assert_eq!(s.escape(), Action::Cancel);
        assert_eq!(s.tick(false), Action::None);
        assert_eq!(s.escape(), Action::None);
    }

    #[test]
    fn a_tap_while_recording_finishes_but_not_a_double_press() {
        let mut s = Shortcut::default();
        let at = |elapsed_ms| Activity::Recording { elapsed_ms };
        assert_eq!(s.press(at(0)), Action::None);
        assert_eq!(s.press(at(399)), Action::None);
        assert_eq!(s.press(at(400)), Action::Finish);
        assert_eq!(s.press(at(60_000)), Action::Finish);
        // Releasing ⌥⇧ after finishing doesn't start anything.
        assert!(!s.is_pending());
        assert_eq!(s.tick(false), Action::None);
        assert_eq!(s.press(Activity::Processing), Action::None);
        assert!(!s.is_pending());
    }

    #[test]
    fn reveal_folds_away_after_its_time_when_ignored() {
        let mut r = Reveal::default();
        assert!(!r.tick(0, false, 0));
        r.show(1_000, 0);
        assert!(r.is_open());
        assert!(!r.tick(1_000 + REVEAL_MS - 1, false, 0));
        assert!(r.tick(1_000 + REVEAL_MS, false, 0));
        assert!(!r.is_open());
        assert!(!r.tick(1_000 + REVEAL_MS + 8, false, 0));
    }

    #[test]
    fn reveal_folds_away_once_work_starts_but_is_seen_first() {
        let mut r = Reveal::default();
        r.show(0, 3);
        // A click right away: the chips stay for their minimum.
        assert!(!r.tick(100, false, 4));
        assert!(!r.tick(REVEAL_MIN_MS - 1, false, 4));
        assert!(r.tick(REVEAL_MIN_MS, false, 4));
        // No new activity: only the timeout folds it.
        r.show(10_000, 7);
        assert!(!r.tick(10_000 + REVEAL_MIN_MS + 500, false, 7));
        assert!(r.tick(10_000 + REVEAL_MIN_MS + 600, false, 8));
    }

    #[test]
    fn reveal_stays_while_hovered_and_lingers_after() {
        let mut r = Reveal::default();
        r.show(0, 0);
        assert!(!r.tick(3_000, true, 0));
        // Hovered past the timeout: still open.
        assert!(!r.tick(6_000, true, 5));
        // Left at 6 s: lingers, even with activity, then folds.
        assert!(!r.tick(6_000 + REVEAL_LINGER_MS - 1, false, 5));
        assert!(r.tick(6_000 + REVEAL_LINGER_MS, false, 5));
        // Choosing a chip closes it outright.
        r.show(20_000, 0);
        r.close();
        assert!(!r.is_open());
    }

    #[test]
    fn in_the_picker_the_shortcut_records_its_selection() {
        let mut s = Shortcut::default();
        assert_eq!(s.press(Activity::Picker), Action::RecordSelected);
        assert!(!s.is_pending());
    }
}
