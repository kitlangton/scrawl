//! Linux GStreamer screen recording execution shared by native capture callers.
pub mod recorder;
pub use recorder::{
    CandidateFrame, Recorder, Source, Stream, can_record_video, encode_synthetic, init_gstreamer, rgba_of,
};
