pub use scrawl_router::{
    Action, AutoSend, Decision, DeliverRequest, Delivered, DeliveryMode, OfferRule, Router,
    RouterConfig, RouterDirs, RuleKind, Sent, Target as RouterTarget, UndoReport, Undone, delivery,
    model, routing,
};

pub mod agent_context;
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
pub mod log;
pub mod mic;
pub mod page;
pub mod permissions;
pub mod phone;
pub mod pipeline;
pub mod recover;
pub mod render;
pub mod retention;
pub mod screens;
pub mod sketch;
pub mod speech_model;
pub mod transcribe;
pub mod web;
#[cfg(target_os = "windows")]
pub mod win;
pub mod window_log;
pub use window_log as windows;
pub mod zoom;

pub use handoff::{
    EngineConfig, enable_scrawl_host_defaults, engine_config_from_scrawl, parse_scrawl_config,
};
pub use log::{Sink, enable_file_log, scrawl_log, set_sink, set_sink_if_unset, start_metric};
pub use mic::Choice as MicChoice;
