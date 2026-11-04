//! Root callers share the recipient durable transaction and error identity.
pub use apparent_connect_client::config_store::*;
use std::path::PathBuf;
pub fn default_config_path() -> PathBuf {
    std::env::var_os("APPARENT_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config/apparent/config.json")
        })
}

