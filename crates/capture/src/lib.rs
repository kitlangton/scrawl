pub use scrawl_router::{
    Action, AutoSend, CandidateTarget, ConfigSource as RouterConfigSource, Decision,
    DeliverRequest, Delivered, DeliveryMode, EngineConfig, EvalOptions, IntoRouterConfig,
    MicChoice, OfferRule, Past, RecentItem, Recents, Router, RouterConfig, RouterDirs, RuleKind,
    Screen, Sent, Sink, Target as RouterTarget, Thresholds, UndoReport, Undone, agent_context,
    bundle, config, delivery, enable_file_log, enable_scrawl_host_defaults,
    engine_config_from_scrawl, env, log, model, opencode, page, parse_scrawl_config,
    parse_scrawl_config_with_warnings, parse_scrawl_router_config,
    parse_scrawl_router_config_with_warnings, router_config_from_scrawl, routing, scrawl_log,
    set_sink, set_sink_if_unset, sketch, start_metric, util, web,
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
