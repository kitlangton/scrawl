//! Staged Engine package; its optional Linux recorder is consumed by the legacy Desktop caller.
#![forbid(unsafe_op_in_unsafe_fn)]
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("capture-engine supports Linux and macOS only");
mod finalizer;
mod identity;
mod lease;
#[cfg(all(target_os = "linux", feature = "linux-media"))]
pub mod linux;
mod locking;
mod receipt;
mod secure;
mod session;
pub use finalizer::FinalizerSuccess;
pub use identity::{new_id, new_owner_token};
pub use lease::{BundleLease, LeaseKind};
pub use receipt::{Receipt, Status};
pub use session::Binding;
pub use session::{Admission, Control, Runtime, Session};
