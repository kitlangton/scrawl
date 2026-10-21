//! Playback ducking uses CoreAudio on macOS; other platforms leave volume unchanged.
pub struct Guard;

pub fn warm_up() {}

impl Guard {
    pub fn start() -> Option<Self> {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn playback_does_not_need_a_helper() {
        super::warm_up();
        assert!(super::Guard::start().is_none());
    }
}
