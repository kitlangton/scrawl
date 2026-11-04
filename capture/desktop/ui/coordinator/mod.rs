use super::super::*;

pub(in crate::capture::desktop) mod capsule;
mod policy;
pub(in crate::capture::desktop) use capsule::*;
pub(in crate::capture::desktop) mod hud;
mod recording;
pub(in crate::capture::desktop) use hud::*;
pub(in crate::capture::desktop) mod picker;
pub(in crate::capture::desktop) use picker::*;
pub(in crate::capture::desktop) mod review;
pub(in crate::capture::desktop) use review::*;
mod windows;
pub(in crate::capture::desktop) use windows::*;

#[cfg(test)]
mod tests;

/// Capsule animation state; capture generations, finalizers and grants remain coordinator-owned.
pub(in crate::capture::desktop) struct CapsulePresentation {
    pub(in crate::capture::desktop) island_phase: IslandPhase,
    pub(in crate::capture::desktop) island_prev_phase: IslandPhase,
    /// `motion::now()` when the phase last changed.
    pub(in crate::capture::desktop) island_phase_since: f64,
    pub(in crate::capture::desktop) island_dismissed_from: IslandPhase,
    pub(in crate::capture::desktop) island_width: f32,
    pub(in crate::capture::desktop) island_target_width: f32,
    pub(in crate::capture::desktop) island_velocity: f32,
    pub(in crate::capture::desktop) island_scale: f32,
    pub(in crate::capture::desktop) island_scale_velocity: f32,
    pub(in crate::capture::desktop) island_y: f32,
    pub(in crate::capture::desktop) island_y_velocity: f32,
    pub(in crate::capture::desktop) island_opacity: f32,
    /// Which width spring the current morph uses.
    pub(in crate::capture::desktop) island_morph: Morph,
    /// Switch chips on a tap-started recording (`crate::capture::desktop::settings::shortcut::Reveal`).
    pub(in crate::capture::desktop) reveal: crate::capture::desktop::settings::shortcut::Reveal,
    /// When the chips last opened or folded, for the in-place swap of the trailing group.
    pub(in crate::capture::desktop) reveal_since: f64,
    /// The trailing group fading out after that change: `Some(true)` the chips, `Some(false)` the
    /// controls, `None` when the capsule opened straight into its state (nothing to swap).
    pub(in crate::capture::desktop) reveal_outgoing: Option<bool>,
    /// The chips were open when the island last changed phase, so a Recording capsule that is
    /// blurring out or dismissing keeps them.
    pub(in crate::capture::desktop) reveal_prev_chips: bool,
    /// macOS Reduce motion, read when a reveal starts.
    pub(in crate::capture::desktop) reduced_motion: bool,
    pub(in crate::capture::desktop) switch: Option<SwitchMorph>,
    /// When the springs were last advanced (`step_island`).
    pub(in crate::capture::desktop) island_stepped: Instant,
    /// A new capsule window has drawn its first frame and not its second (which comes late):
    /// only its frames advance the springs, so the hitch slows the opening instead of eating it.
    pub(in crate::capture::desktop) island_await_frame: bool,
    /// The recording clock last shown, so the lead keeps its time while it leaves.
    pub(in crate::capture::desktop) last_elapsed_ms: u64,
    /// The highlighted mode before the last change and when it changed (`HIGHLIGHT_FADE_S`).
    pub(in crate::capture::desktop) highlight_prev: Option<SelectionMode>,
    pub(in crate::capture::desktop) highlight_since: f64,
}

impl CapsulePresentation {
    pub(in crate::capture::desktop) fn new() -> Self {
        Self {
            island_phase: IslandPhase::Hidden,
            island_prev_phase: IslandPhase::Hidden,
            island_phase_since: motion::now(),
            island_dismissed_from: IslandPhase::Hidden,
            island_width: 205.0,
            island_target_width: 205.0,
            island_velocity: 0.0,
            island_scale: 0.92,
            island_scale_velocity: 0.0,
            island_y: -6.0,
            island_y_velocity: 0.0,
            island_opacity: 0.0,
            island_morph: Morph::Phase,
            reveal: crate::capture::desktop::settings::shortcut::Reveal::default(),
            reveal_since: motion::now(),
            reveal_outgoing: None,
            reveal_prev_chips: false,
            reduced_motion: false,
            switch: None,
            island_stepped: Instant::now(),
            island_await_frame: false,
            last_elapsed_ms: 0,
            highlight_prev: None,
            highlight_since: motion::now(),
        }
    }
}
