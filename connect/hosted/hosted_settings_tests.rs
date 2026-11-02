//! Real application config transactions, not a mirror of the Connect writer.
use super::*;
use serde_json::{Value, json};
use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "settings-provider-{}",
            apparent_connect_client::test_support::fixture_id()
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        Self(root)
    }
    fn path(&self) -> PathBuf {
        self.0.join("config.json")
    }
    fn read(&self) -> Value {
        serde_json::from_slice(&fs::read(self.path()).unwrap()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn hosted_settings_provider_preserves_unknown_nested_cloud_and_experiments() {
    let fixture = Fixture::new();
    let path = fixture.path();
    let original = json!({
        "privateUnknown": {"kept": true},
        "cloud": {"server":"https://synthetic.example","future": {"kept":[1,2,3]}},
        "experiments": {"unrelated": true, "connectSettings": true}
    });
    fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    let provider = AppConfig(path.clone());
    provider
        .set_cloud_allowed(
            &CloudConfig {
                enabled: true,
                upload_consent: true,
                server: "https://synthetic.example".into(),
            },
            &|| true,
        )
        .unwrap();
    provider.set_experiment(HOSTED_SYNC, true).unwrap();
    for logout in [false, true] {
        client_at(path.clone()).settings_disable(logout).unwrap();
        let current = fixture.read();
        assert_eq!(current["privateUnknown"], original["privateUnknown"]);
        assert_eq!(current["cloud"]["future"], original["cloud"]["future"]);
        assert_eq!(current["experiments"]["unrelated"], true);
        assert_eq!(current["experiments"]["connectSettings"], true);
        assert_eq!(current["experiments"][HOSTED_SYNC], true);
        assert_eq!(current["cloud"]["enabled"], false);
        assert_eq!(current["cloud"]["uploadConsent"], false);
        assert_eq!(current["cloud"]["server"], "https://synthetic.example");
    }
}

#[test]
fn hosted_settings_provider_rejects_malformed_nested_state_without_replacing_it() {
    let fixture = Fixture::new();
    let path = fixture.path();
    for value in [
        json!({"cloud":null}),
        json!({"cloud":["keep"]}),
        json!({"experiments":false}),
    ] {
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let provider = AppConfig(path.clone());
        if value.get("cloud").is_some() {
            assert!(provider.set_cloud(&CloudConfig::default()).is_err());
        } else {
            assert!(provider.set_experiment(HOSTED_SYNC, true).is_err());
        }
        assert_eq!(fixture.read(), value);
    }
}

#[test]
fn hosted_settings_guard_is_rechecked_inside_the_actual_config_transaction() {
    let fixture = Fixture::new();
    let path = fixture.path();
    let original =
        json!({"cloud":{"enabled":false,"uploadConsent":false,"future":"keep"},"unrelated":true});
    fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    let allowed = Arc::new(AtomicBool::new(true));
    let (snapshot_tx, snapshot_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let active = allowed.clone();
    let worker = std::thread::spawn(move || {
        crate::shared::handoff::set_config_write_hook(Some(Box::new(move |point, _| {
            if point == "snapshot" {
                snapshot_tx.send(()).unwrap();
                resume_rx.recv_timeout(Duration::from_secs(8)).unwrap();
            }
        })));
        let result = AppConfig(path).set_cloud_allowed(
            &CloudConfig {
                enabled: true,
                upload_consent: true,
                server: "https://synthetic.example".into(),
            },
            &|| active.load(Ordering::SeqCst),
        );
        crate::shared::handoff::set_config_write_hook(None);
        result
    });
    snapshot_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    allowed.store(false, Ordering::SeqCst);
    resume_tx.send(()).unwrap();
    assert!(worker.join().unwrap().is_err());
    assert_eq!(
        fixture.read(),
        original,
        "cancelled authorization changed the persisted config"
    );
}

#[test]
fn hosted_settings_guarded_writer_cancels_while_waiting_for_the_shared_sidecar() {
    let fixture = Fixture::new();
    let path = fixture.path();
    let original = json!({"cloud":{"enabled":false},"unrelated":true});
    fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(fixture.0.join("config.json.lock"))
        .unwrap();
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
    let allowed = Arc::new(AtomicBool::new(true));
    let active = allowed.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        crate::shared::handoff::set_config_write_hook(Some(Box::new(move |point, _| {
            if point == "before_lock" {
                started_tx.send(()).unwrap();
            }
        })));
        let result = AppConfig(path)
            .set_cloud_allowed(&CloudConfig::default(), &|| active.load(Ordering::SeqCst));
        crate::shared::handoff::set_config_write_hook(None);
        done_tx.send(result.is_err()).unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    allowed.store(false, Ordering::SeqCst);
    assert!(
        done_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        "writer waited for a foreign lock after cancellation"
    );
    drop(lock);
    worker.join().unwrap();
    assert_eq!(fixture.read(), original);
}

#[test]
fn hosted_settings_directory_sync_failure_retains_revocation_across_stale_config_recovery() {
    let fixture = Fixture::new();
    let path = fixture.path();
    let original = json!({
        "cloud":{"enabled":true,"uploadConsent":true,"server":"https://synthetic.example","future":"keep"},
        "experiments":{"hostedSync":true}, "unrelated":true
    });
    fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    crate::shared::handoff::enable_experiment(HOSTED_SYNC);
    let client = client_at(path.clone());
    assert!(
        client.enabled(),
        "fixture must establish the enabled admission gate before failure"
    );
    struct ResetFault;
    impl Drop for ResetFault {
        fn drop(&mut self) {
            crate::shared::handoff::set_config_directory_sync_failure(false);
        }
    }
    let _reset = ResetFault;
    crate::shared::handoff::set_config_directory_sync_failure(true);
    assert!(
        client.settings_disable(false).is_err(),
        "directory sync failure was reported as success"
    );
    assert!(
        client.revocation_pending(),
        "undurable config cannot settle the durable intent"
    );
    assert_eq!(
        fixture.read()["cloud"]["enabled"],
        false,
        "fault must occur after the config rename"
    );
    assert_eq!(fixture.read()["cloud"]["future"], "keep");
    // Model recovery of the old directory entry after failed directory sync. This
    // is explicit failure injection, not a claim to have power-cycled a filesystem.
    crate::shared::handoff::set_config_value_in(&path, "cloud", original["cloud"].clone()).unwrap();
    let fresh = client_at(path.clone());
    assert!(
        !fresh.enabled(),
        "a fresh client accepted stale consent despite the retained intent"
    );
    let error = fresh
        .enqueue(&fixture.0.join("not-a-capture.apparent"))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Enable hostedSync and grant upload consent with apparent connect login --scope upload --consent-upload",
        "admission must fail at the first disabled gate, before reading the nonexistent bundle"
    );
    fresh.settings_disable(false).unwrap();
    assert!(!fresh.revocation_pending());
    assert!(!fresh.enabled());
    assert_eq!(fixture.read()["cloud"]["enabled"], false);
    assert_eq!(fixture.read()["cloud"]["future"], "keep");
}
