use apparent_capture_engine::{Admission, Binding, Control, Runtime, Status, new_id};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("engine-binding-{}", new_id().unwrap()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TOKEN: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const BUNDLE: &str = "native-provider-bundle";
fn binding(path: &str) -> Binding {
    Binding {
        bundle_path: path.into(),
        screen: true,
    }
}

#[test]
fn provider_identity_binds_path_and_options_and_authenticates_before_recovery() {
    let root = Fixture::new();
    let runtime = Runtime::open(&root.0).unwrap();
    let Admission::Acquired(session) = runtime
        .admit_bound(ID, TOKEN, BUNDLE, binding("/tmp/a.apparent"))
        .unwrap()
    else {
        panic!("admission expected")
    };
    assert_eq!(
        runtime
            .request_control(ID, TOKEN, Control::Stop)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
    drop(session);
    assert_eq!(
        runtime
            .inspect_authorized(ID, "d".repeat(64).as_str())
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    let raw = std::fs::read(root.0.join(format!("request-{ID}.json"))).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&raw).unwrap()["receipt"]["status"],
        "starting"
    );
    assert_eq!(
        runtime
            .inspect_authorized(ID, TOKEN)
            .unwrap()
            .unwrap()
            .status,
        Status::Failed
    );
    assert!(
        matches!(runtime.admit_bound(ID, TOKEN, BUNDLE, binding("/tmp/other.apparent")), Err(e) if e.kind() == std::io::ErrorKind::InvalidInput)
    );
    assert!(
        matches!(runtime.admit_bound(ID, TOKEN, BUNDLE, Binding { bundle_path: "/tmp/a.apparent".into(), screen: false }), Err(e) if e.kind() == std::io::ErrorKind::InvalidInput)
    );
}

#[test]
fn stop_is_a_durable_phase_before_finalization() {
    let root = Fixture::new();
    let runtime = Runtime::open(&root.0).unwrap();
    let Admission::Acquired(mut session) = runtime
        .admit_bound(ID, TOKEN, BUNDLE, binding("/tmp/a.apparent"))
        .unwrap()
    else {
        panic!("admission expected")
    };
    session.recording().unwrap();
    let receipt = runtime.request_control(ID, TOKEN, Control::Stop).unwrap();
    assert_eq!(receipt.status, Status::Stopping);
    session.processing().unwrap();
    assert_eq!(
        runtime
            .inspect_authorized(ID, TOKEN)
            .unwrap()
            .unwrap()
            .status,
        Status::Processing
    );
    assert_eq!(session.finish(Err(())).unwrap().status, Status::Failed);
}
