pub use scrawl_router::{
    CandidateTarget, Decision, DeliverHooks, DeliverRequest, Delivered, DeliveryMode,
    DeliveryOrigin, EngineConfig, MicChoice, OfferRule, Past, Recents, Router, RouterConfig,
    RouterDirs, RuleKind, Screen, Sent, Sink, Thresholds, UndoReport, Undone, agent_context,
    delivery, enable_file_log, enable_scrawl_host_defaults, log, model, page, parse_scrawl_config,
    routing, scrawl_log, set_sink, set_sink_if_unset, sketch, start_metric, web,
};

pub mod audio;
pub mod autocrop;
pub mod autocrop_lab;
// Windows looks elements up through UI Automation (`win/uia.rs`); other systems have no lookup.
#[cfg_attr(
    not(any(target_os = "macos", target_os = "windows")),
    path = "ax_other.rs"
)]
pub mod ax;
pub mod capture;
pub mod command_transcriber;
pub mod export;
pub mod handoff;
#[cfg_attr(target_os = "windows", path = "win/helper.rs")]
pub mod helper;
#[cfg(feature = "hex-client")]
pub mod hex;
pub mod ingest_http;
pub mod keyframes;
pub mod live_transcribe;
pub mod mic;
pub mod permissions;
pub mod phone;
pub mod pipeline;
pub mod recover;
pub mod render;
pub mod retention;
pub mod screens;
pub mod speech_model;
pub mod transcribe;
#[cfg(target_os = "windows")]
pub mod win;
pub mod window_log;
pub use window_log as windows;
pub mod zoom;
