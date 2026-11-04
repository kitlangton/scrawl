use std::{io::Write, process::{Command, Stdio}};
use apparent_connect_client::{ConfigProvider, CloudConfig, config_provider::ProductionConfig};
#[test]
fn executable_rejects_protocol_without_config_access() {
    for request in [r#"{"protocol":2,"operation":{"kind":"status","account":false}}"#, r#"{"protocol":1,"operation":{"kind":"upload"}}"#] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_apparent-connect-recipient")).env("APPARENT_CONFIG", "/nonexistent/private/config.json").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(request.as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["protocol"], 1);
        assert!(value["error"]["message"].is_string());
        assert!(!String::from_utf8(output.stdout).unwrap().contains("/nonexistent/private"));
    }
}
#[test]
fn provider_preserves_unknown_secrets_and_cancelled_transaction() {
    let dir = std::env::current_dir().unwrap().join(format!("recipient-config-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap(); }
    let path = dir.join("config.json");
    let original = br#"{"secret":"keep-private","unknown":{"nested":42},"experiments":{"other":false}}"#;
    std::fs::write(&path, original).unwrap();
    let provider = ProductionConfig(path.clone());
    assert!(provider.set_cloud_allowed(&CloudConfig::default(), &|| false).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    provider.set_cloud(&CloudConfig {server:"https://example.test".into(), ..Default::default()}).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(value["secret"], "keep-private");
    assert_eq!(value["unknown"]["nested"], 42);
    assert_eq!(value["experiments"]["other"], false);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn executable_status_is_json_and_never_discloses_config_secrets() {
    let dir = std::env::current_dir().unwrap().join(format!("recipient-status-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap(); }
    let path = dir.join("config.json");
    std::fs::write(&path, br#"{"secret":"synthetic-secret-never-print","cloud":{"server":"https://example.test"}}"#).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_apparent-connect-recipient")).env("APPARENT_CONFIG", &path).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(br#"{"protocol":1,"operation":{"kind":"status","account":false}}"#).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["result"]["readSignedIn"], false);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic-secret"));
    std::fs::remove_dir_all(dir).unwrap();
}
