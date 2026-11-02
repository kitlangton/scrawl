//! Microphone preference shared by the native menu and Electron Recording control.
use serde::Deserialize;
use std::path::Path;
#[derive(Debug, Deserialize)]
#[serde(tag = "microphone", rename_all = "lowercase", deny_unknown_fields)]
pub(crate) enum Request {
    Read { deadline: u64 },
    Set { enabled: bool, deadline: u64 },
}
impl Request {
    pub fn deadline(&self) -> u64 {
        match self {
            Self::Read { deadline } | Self::Set { deadline, .. } => *deadline,
        }
    }
    pub fn execute(&self, path: &Path, deadline: u64) -> serde_json::Value {
        let allowed = || super::super::agent_capture::storage_clock_ms() < deadline;
        if !allowed() {
            return serde_json::json!({"error":"admission_expired"});
        }
        if let Self::Set { enabled, .. } = self {
            if crate::shared::handoff::update_config_allowed_in(path, &allowed, |config| {
                config["microphoneEnabled"] = serde_json::json!(enabled);
                Ok(())
            })
            .is_err()
            {
                return serde_json::json!({"error":"config_write_failed"});
            }
        }
        serde_json::json!({"microphone":{"enabled": crate::capture::engine::runtime::audio::mic::enabled_in(path)}})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn silent_capture_preference_preserves_device_and_unknown_config_and_deadline() {
        let dir = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
        std::fs::create_dir(&dir).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, br#"{"microphone":{"id":"chosen","name":"Chosen input"},"privateKey":"synthetic","future":{"value":7}}"#).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let future = super::super::super::agent_capture::storage_clock_ms() + 3000;
        assert_eq!(
            Request::Set {
                enabled: false,
                deadline: future
            }
            .execute(&path, future)["microphone"]["enabled"],
            false
        );
        let data: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(data["microphone"]["id"], "chosen");
        assert_eq!(data["privateKey"], "synthetic");
        assert_eq!(data["future"]["value"], 7);
        let before = std::fs::read(&path).unwrap();
        assert_eq!(
            Request::Set {
                enabled: true,
                deadline: 0
            }
            .execute(&path, 0)["error"],
            "admission_expired"
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
