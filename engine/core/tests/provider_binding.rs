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
const ID2: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const MATERIALS: &str = "dddddddddddddddddddddddddddddddd";
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

#[test]
fn live_capture_permit_is_shared_with_v1_without_changing_v1_receipts() {
    let root = Fixture::new();
    let runtime = Runtime::open(&root.0).unwrap();
    let permit = runtime.acquire_live_capture().unwrap().unwrap();
    permit.check().unwrap();
    assert!(runtime.acquire_live_capture().unwrap().is_none());
    assert!(matches!(
        runtime
            .admit_bound(ID, TOKEN, BUNDLE, binding("/tmp/a.apparent"))
            .unwrap(),
        Admission::Existing
    ));
    drop(permit);

    let Admission::Acquired(session) = runtime
        .admit_bound(ID2, TOKEN, BUNDLE, binding("/tmp/a.apparent"))
        .unwrap()
    else {
        panic!("v1 admission expected after permit release")
    };
    assert!(runtime.acquire_live_capture().unwrap().is_none());
    drop(session);
    assert!(runtime.acquire_live_capture().unwrap().is_some());
}

#[test]
fn capture2_owner_liveness_is_per_operation_not_global_capture_admission() {
    let root = Fixture::new();
    let first = Runtime::open(&root.0).unwrap();
    let second = Runtime::open(&root.0).unwrap();
    let owner = first.acquire_capture_owner(ID).unwrap().unwrap();
    assert!(second.acquire_capture_owner(ID).unwrap().is_none());
    assert!(second.acquire_capture_owner(ID2).unwrap().is_some());
    // A capture/2 control can establish the named owner's liveness while a
    // different protocol operation independently owns global live admission.
    let global = first.acquire_live_capture().unwrap().unwrap();
    assert!(second.acquire_capture_owner(ID).unwrap().is_none());
    drop(owner);
    assert!(second.acquire_capture_owner(ID).unwrap().is_some());
    drop(global);
}

#[test]
fn retained_material_maintenance_excludes_other_runtime_readers() {
    let root = Fixture::new();
    let first = Runtime::open(&root.0).unwrap();
    let second = Runtime::open(&root.0).unwrap();
    let reader = first
        .material_lease(MATERIALS, apparent_capture_engine::LeaseKind::Read)
        .unwrap()
        .unwrap();
    assert!(reader.check().is_ok());
    assert!(
        second
            .material_lease(MATERIALS, apparent_capture_engine::LeaseKind::Maintenance)
            .unwrap()
            .is_none()
    );
    drop(reader);
    let maintenance = second
        .material_lease(MATERIALS, apparent_capture_engine::LeaseKind::Maintenance)
        .unwrap()
        .unwrap();
    assert!(
        first
            .material_lease(MATERIALS, apparent_capture_engine::LeaseKind::Read)
            .unwrap()
            .is_none()
    );
    drop(maintenance);
    assert!(
        first
            .material_lease(MATERIALS, apparent_capture_engine::LeaseKind::Read)
            .unwrap()
            .is_some()
    );
}
