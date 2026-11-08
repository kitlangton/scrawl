pub(crate) mod connect;
pub(crate) mod onboarding;
pub(crate) mod onboarding_window;
pub(crate) mod settings;
pub(in crate::capture::desktop) mod presentation;
#[cfg(target_os = "linux")]
mod storage;
pub(crate) mod settings_window;
pub(crate) mod shortcut;

pub(crate) use settings::*;

pub(crate) mod practice;
