//! Staged Engine package; its optional Linux recorder is consumed by the legacy Desktop caller.
#![forbid(unsafe_op_in_unsafe_fn)]
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("capture-engine supports Linux and macOS only");
#[cfg(feature = "audio-codec")]
pub mod audio;
mod finalizer;
mod identity;
mod lease;
#[cfg(all(target_os = "linux", feature = "linux-media"))]
pub mod linux;
mod locking;
#[cfg(any(feature = "publication", all(target_os = "linux", feature = "linux-media")))]
mod movie;
#[cfg(feature = "recording-notes")]
pub mod recording;
pub mod ink;
pub mod observations;
#[cfg(all(target_os = "macos", feature = "macos-capture"))]
pub mod macos;
#[cfg(feature = "materials-media")]
pub mod materials_media;
#[cfg(feature = "publication")]
pub mod publication;
mod receipt;
mod secure;
mod session;
pub use finalizer::FinalizerSuccess;
pub use identity::{equal_secret as identity_equal_secret, new_id, new_owner_token};
pub use lease::{BundleLease, LeaseKind};
pub use receipt::{Receipt, Status};
pub use secure::{Store as SecureStore, directory as open_directory};
pub use session::Binding;
pub use session::{
    Admission, CaptureOwnerPermit, CaptureTransaction, Control, LiveCapturePermit, Runtime, Session,
};

/// Existing capture sizing arithmetic, with neutral frame values instead of an app target.
/// Callers choose the long edge and maximum scale; native encoders receive even dimensions.
pub fn capture_dimensions(frame: [f64; 3], long_edge: f64, maximum_scale: f64) -> (u32, u32) {
    let [width, height, frame_scale] = frame;
    let max_dim = width.max(height).max(1.0);
    let native = frame_scale.clamp(1.0, maximum_scale);
    let scale = (long_edge / max_dim).min(native);
    (
        ((width * scale).round() as u32).max(48) & !1,
        ((height * scale).round() as u32).max(48) & !1,
    )
}

#[cfg(test)]
mod capture_dimension_tests {
    #[test]
    fn source_bounds_and_encoded_dimensions_are_distinct_for_odd_sources() {
        assert_eq!(super::capture_dimensions([683.0, 384.0, 1.0], 683.0, 1.0), (682, 384));
        assert_eq!(super::capture_dimensions([641.0, 481.0, 1.0], 641.0, 1.0), (640, 480));
        assert_eq!(super::capture_dimensions([1366.0, 768.0, 1.0], 1366.0, 1.0), (1366, 768));
        assert_eq!(super::capture_dimensions([2560.0, 1440.0, 2.0], 1920.0, 2.0), (1920, 1080));
    }
}
