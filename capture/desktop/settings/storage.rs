//! Linux Storage stays inside the native Settings frame. Reuse the same
//! preference transactions and bounded usage scan as the Electron presentation.
use std::path::Path;
use serde::{Deserialize, Serialize};
use crate::shared::handoff::capture_location;
use crate::capture::desktop::workflow::storage_preferences;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum Command {
    Location { path: Option<String>, expected: String },
    Retention { preset: Option<String>, expected: String },
    Refresh,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Location {
    pub display_path: String,
    pub effective_path: String,
    pub saved_path: Option<String>,
    pub custom: bool,
    pub source: String,
    pub revision: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Usage {
    pub effective_path: String,
    pub formatted: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct View {
    pub location: Option<Location>,
    pub retention_revision: Option<String>,
    pub retention_custom: bool,
    pub usage: Option<Usage>,
    pub error: Option<String>,
    pub usage_error: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct State {
    error: Option<&'static str>,
    usage: Option<Usage>,
    usage_error: Option<&'static str>,
    pub usage_version: u64,
}

impl State {
    pub fn command(&mut self, config: &Path, command: Command) -> bool {
        let result = match command {
            Command::Location { path, expected } =>
                capture_location::set_in(config, path.as_deref(), Some(&expected), || true),
            Command::Retention { preset, expected } =>
                storage_preferences::save(config, preset.as_deref(), &expected, || true).map(drop),
            Command::Refresh => Ok(()),
        };
        self.error = result.err();
        if result.is_err() { return false; }
        self.usage_version += 1;
        self.usage = None;
        self.usage_error = None;
        true
    }

    pub fn measured(&mut self, version: u64, result: Result<serde_json::Value, &'static str>) {
        if version != self.usage_version { return; }
        match result.and_then(|value| serde_json::from_value(value["usage"].clone()).map_err(|_| "usage_unavailable")) {
            Ok(usage) => { self.usage = Some(usage); self.usage_error = None; }
            Err(error) => { self.usage = None; self.usage_error = Some(error); }
        }
    }

    pub fn view(&self, config: &Path) -> View {
        let mut view = View { error: self.error.map(problem), usage_error: self.usage_error.map(problem), ..View::default() };
        match capture_location::read_in(config) {
            Ok(location) => {
                view.usage = self.usage.as_ref().filter(|usage| usage.effective_path == location.effective_path).cloned();
                view.location = Some(Location {
                    display_path: location.display_path, effective_path: location.effective_path,
                    saved_path: location.saved_path, custom: location.custom,
                    source: location.source.into(), revision: location.revision,
                });
            }
            Err(_) => view.error = Some(problem("config_read_failed")),
        }
        if let Ok(retention) = storage_preferences::read(config) {
            view.retention_revision = retention["revision"].as_str().map(str::to_owned);
            view.retention_custom = retention["custom"].as_bool().unwrap_or(false);
        }
        view
    }
}

fn problem(code: &str) -> String {
    match code {
        "conflict" => "Storage changed during this request. Refresh before trying again.",
        "invalid_path" => "That folder was not accepted. Choose an absolute local folder.",
        "config_write_failed" => "The preference could not be saved. Refresh to read the saved settings.",
        "config_read_failed" => "Storage settings could not be read. Check the config file and refresh.",
        _ => "Folder usage is unavailable or the scan exceeded its limit. Refresh to try again.",
    }.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_storage_keeps_transactions_conflicts_and_reset_without_moving_captures() {
        let root = std::env::temp_dir().join(format!("native-storage-{}", crate::shared::ids::make_bundle_id()));
        std::fs::create_dir(&root).unwrap();
        let config = root.join("config.json");
        std::fs::write(&config, r#"{"secret":"keep","retention":{"future":42}}"#).unwrap();
        let folder = root.join("Captures with é");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("keep"), b"recording").unwrap();
        let mut state = State::default();
        let before = state.view(&config);
        let expected = before.location.unwrap().revision;
        assert!(state.command(&config, Command::Location { path: Some(folder.to_str().unwrap().into()), expected: expected.clone() }));
        let chosen = state.view(&config);
        assert!(chosen.location.as_ref().unwrap().custom);
        assert!(!state.command(&config, Command::Location { path: None, expected }));
        assert!(state.view(&config).error.unwrap().contains("Storage changed"));
        let expected = chosen.retention_revision.unwrap();
        assert!(state.command(&config, Command::Retention { preset: Some("forever".into()), expected }));
        assert!(state.command(&config, Command::Retention { preset: None, expected: state.view(&config).retention_revision.unwrap() }));
        assert!(state.command(&config, Command::Location { path: None, expected: state.view(&config).location.unwrap().revision }));
        let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
        assert_eq!(raw["secret"], "keep");
        assert_eq!(raw["retention"]["future"], 42);
        assert!(raw.get("capturesDir").is_none());
        assert_eq!(std::fs::read(folder.join("keep")).unwrap(), b"recording");
        std::fs::write(&config, "broken config").unwrap();
        assert!(state.view(&config).location.is_none());
        assert!(!state.command(&config, Command::Location { path: None, expected: String::new() }));
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "broken config");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refresh_rejects_old_usage_replies() {
        let mut state = State::default();
        assert!(state.command(Path::new("unused"), Command::Refresh));
        state.measured(0, Ok(serde_json::json!({"usage":{"effectivePath":"/old","formatted":"1 GB"}})));
        assert!(state.usage.is_none());
        state.measured(1, Ok(serde_json::json!({"usage":{"effectivePath":"/new","formatted":"2 MB"}})));
        assert_eq!(state.usage.as_ref().unwrap().formatted, "2 MB");
    }
}
