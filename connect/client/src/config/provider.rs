//! Production preferences use the same transaction as root application writers.
use crate::{CloudConfig, ConfigProvider, config_store};
use anyhow::Result;
use std::path::{Path, PathBuf};
pub struct ProductionConfig(pub PathBuf);
impl ProductionConfig {
    fn read(&self) -> serde_json::Value {
        std::fs::read(&self.0).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
}
impl ConfigProvider for ProductionConfig {
    fn config_path(&self) -> &Path { &self.0 }
    fn cloud(&self) -> CloudConfig { serde_json::from_value(self.read()["cloud"].clone()).unwrap_or_default() }
    fn experiment(&self, name: &str) -> bool { self.read()["experiments"][name].as_bool().unwrap_or(true) }
    fn set_cloud(&self, value: &CloudConfig) -> Result<()> { self.set_cloud_allowed(value, &|| true) }
    fn set_cloud_allowed(&self, value: &CloudConfig, allowed: &dyn Fn() -> bool) -> Result<()> {
        config_store::update_config_allowed_in(&self.0, allowed, |v| { v["cloud"] = serde_json::to_value(value)?; Ok(()) })
    }
    fn set_experiment(&self, name: &str, enabled: bool) -> Result<()> {
        config_store::update_config_in(&self.0, |v| {
            if !v["experiments"].is_object() { v["experiments"] = serde_json::json!({}); }
            v["experiments"][name] = enabled.into(); Ok(())
        })
    }
}
