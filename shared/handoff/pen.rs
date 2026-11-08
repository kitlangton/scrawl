//! Validated pen preference. The renderer never writes configuration directly.
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

static COLOR: AtomicU32 = AtomicU32::new(0xff3b30);
pub fn rgb() -> [u8; 3] {
    let n = COLOR.load(Ordering::Relaxed);
    [(n >> 16) as u8, (n >> 8) as u8, n as u8]
}
pub fn palette(name: &str) -> Option<[u8; 3]> {
    match name {
        "coral" => Some([217, 76, 68]),
        "purple" => Some([122, 88, 199]),
        "blue" => Some([59, 121, 189]),
        "green" => Some([57, 123, 97]),
        _ => None,
    }
}
fn apply(rgb: [u8; 3]) {
    COLOR.store((u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]), Ordering::Relaxed);
}
pub fn refresh() {
    let config = super::load_config_from(&super::default_config_path());
    apply(config.pen_color.as_deref().and_then(palette).unwrap_or([255, 59, 48]));
}
fn persist(path: &Path, name: &str) -> anyhow::Result<[u8; 3]> {
    let rgb = palette(name).ok_or_else(|| anyhow::anyhow!("Unknown pen colour"))?;
    super::set_config_value_in(path, "penColor", name.into())?;
    Ok(rgb)
}
pub fn save(name: &str) -> anyhow::Result<()> {
    apply(persist(&super::default_config_path(), name)?);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_write_preserves_other_keys_and_rejects_unknown_pigment() {
        struct Temp(std::path::PathBuf);
        impl Drop for Temp { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
        let dir = Temp(std::env::temp_dir().join(format!("apparent-pen-{}", crate::shared::ids::make_bundle_id())));
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(&dir.0).unwrap();
        let path = dir.0.join("config.json");
        let original = br#"{"secret":"synthetic","microphoneEnabled":false}"#;
        std::fs::write(&path, original).unwrap();
        assert!(persist(&path, "#123456").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(persist(&path, "purple").unwrap(), [122, 88, 199]);
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value, serde_json::json!({"secret":"synthetic","microphoneEnabled":false,"penColor":"purple"}));
        assert_eq!(super::super::load_config_from(&path).pen_color.as_deref(), Some("purple"));
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::write(&path, b"broken config").unwrap();
        assert!(persist(&path, "blue").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"broken config");
    }
}
