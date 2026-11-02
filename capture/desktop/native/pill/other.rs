//! The native pill is an AppKit panel drawn with Metal. Other platforms keep the GPUI capsule
//! (`handoff::native_pill` is always off there); this keeps the shared values and a pill that
//! never opens, so the coordinator's code is the same everywhere.

#[path = "model.rs"]
pub mod model;
#[path = "types.rs"]
mod types;

pub use self::types::{PillAction, PillSnapshot, PillStyle};

pub struct NativePill;

impl NativePill {
    pub fn open() -> Result<Self, String> {
        Err("the native pill needs macOS".to_owned())
    }

    pub fn window_number(&self) -> u32 {
        0
    }

    pub fn place_on(&self, _point: [f64; 2]) {}

    pub fn sync(&self, _snapshot: PillSnapshot) {}

    pub fn drain_actions(&self) -> Vec<PillAction> {
        Vec::new()
    }

    pub fn is_fully_hidden(&self) -> bool {
        true
    }

    pub fn note_badge_anchor(&self) -> Option<[f64; 4]> { None }

    pub fn pointer_on_pill(&self) -> bool {
        false
    }
}
