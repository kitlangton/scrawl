//! Native publication extraction boundary. Capability negotiation stays with the provider.
//!
//! This API validates only an already-open movie file's ISO-BMFF structure and
//! video sample extents. The caller must obtain the file from its pinned staged
//! bundle and hold the applicable bundle lease through validation and commit.
//! This check is not full-profile validation, capture authorization, or Saved
//! evidence; those remain responsibilities of the publication transaction owner.
mod movie;

/// Validate a movie container and its video sample extents from an already-open file.
pub use movie::validate_movie;
