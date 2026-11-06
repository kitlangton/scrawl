//! Trusted-caller native publication primitives; capability negotiation stays
//! with the authenticated provider. The backend snapshots a validated declared
//! inventory and reconciles a journal-bound no-replace commit. Its caller must
//! authenticate before lookup, retain the exact material/source authority and
//! leases, and serialize cancellation against commit. These library operations
//! alone do not establish a capture protocol Saved response.
//!
//! The separate movie validator checks an already-open file's ISO-BMFF structure
//! and video sample extents. Its caller holds the applicable pinned bundle lease;
//! a successful movie check alone is not full-profile validation or Saved proof.
mod backend;
mod movie;

pub use backend::{
    FileIdentity, MaterialsKey, PreparedPublication, PublicationBackend, PublicationEvidence,
    SealedFile, TrustedPublicationContext, TrustedSource,
};
/// Validate a movie container and its video sample extents from an already-open file.
pub use movie::validate_movie;
