use apparent_capture_engine::{
    Admission, FinalizerSuccess, LeaseKind, Runtime, Status, new_id, new_owner_token,
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("engine-review-{}", new_id().unwrap()));
        fs::create_dir(&root).unwrap();
        mode(&root, 0o700);
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn mode(path: &Path, value: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(value)).unwrap();
}
fn output(root: &Path, bundle: &str, padding: usize) -> (PathBuf, Vec<u8>) {
    let path = root.join("output.apparent");
    fs::create_dir(&path).unwrap();
    mode(&path, 0o700);
    let bytes = serde_json::to_vec(&serde_json::json!({"version":1,"id":bundle,"events":[],"strokes":[],"transcript":"x".repeat(padding)})).unwrap();
    fs::write(path.join("apparent.json"), &bytes).unwrap();
    mode(&path.join("apparent.json"), 0o600);
    (path, bytes)
}
#[test]
fn safe_manifest_does_not_make_a_writable_directory_trusted() {
    let f = Fixture::new();
    let request = new_id().unwrap();
    let bundle = new_id().unwrap();
    let (path, bytes) = output(&f.0, &bundle, 0);
    for unsafe_mode in [0o777, 0o770, 0o702] {
        mode(&path, unsafe_mode);
        assert!(FinalizerSuccess::validate(&request, &bundle, &path, &bytes).is_err());
    }
    mode(&path, 0o755);
    assert!(FinalizerSuccess::validate(&request, &bundle, &path, &bytes).is_ok());
}
#[test]
fn directory_permissions_are_rechecked_before_saved() {
    for ancestor in [false, true] {
        let f = Fixture::new();
        let runtime = Runtime::open(&f.0).unwrap();
        let request = new_id().unwrap();
        let bundle = new_id().unwrap();
        let Admission::Acquired(mut session) = runtime
            .admit(&request, &new_owner_token().unwrap(), &bundle)
            .unwrap()
        else {
            panic!("not admitted")
        };
        session.recording().unwrap();
        session.processing().unwrap();
        let parent = f.0.join("outputs");
        fs::create_dir(&parent).unwrap();
        mode(&parent, 0o700);
        let (path, bytes) = output(&parent, &bundle, 0);
        let proof = FinalizerSuccess::validate(&request, &bundle, &path, &bytes).unwrap();
        mode(if ancestor { &parent } else { &path }, 0o777);
        assert_eq!(session.finish(Ok(proof)).unwrap().status, Status::Failed);
    }
}
#[test]
fn writable_output_ancestor_is_not_trusted() {
    let f = Fixture::new();
    let parent = f.0.join("shared");
    fs::create_dir(&parent).unwrap();
    let bundle = new_id().unwrap();
    let (path, bytes) = output(&parent, &bundle, 0);
    for ancestor_mode in [0o777, 0o1777] {
        mode(&parent, ancestor_mode);
        // Sticky grants an exception only to root-owned ancestors, never to an
        // ordinary user's writable directory. Root test runners exercise that
        // exception without changing any fixture ownership.
        let root_sticky = ancestor_mode == 0o1777 && unsafe { libc::geteuid() } == 0;
        assert_eq!(
            FinalizerSuccess::validate(&new_id().unwrap(), &bundle, &path, &bytes).is_ok(),
            root_sticky,
            "ancestor mode {ancestor_mode:o}"
        );
    }
}
#[test]
fn durable_production_and_caller_identities_roundtrip() {
    for bundle in [
        "apparent-1793444400123-1a2b-3",
        "Customer demo — 東京",
        "demo_01",
    ] {
        let f = Fixture::new();
        let runtime = Runtime::open(&f.0).unwrap();
        let request = new_id().unwrap();
        let token = new_owner_token().unwrap();
        let Admission::Acquired(mut session) = runtime.admit(&request, &token, bundle).unwrap()
        else {
            panic!("not admitted")
        };
        assert!(
            runtime
                .lease(bundle, LeaseKind::Maintenance)
                .unwrap()
                .is_none()
        );
        session.recording().unwrap();
        session.processing().unwrap();
        let (path, bytes) = output(&f.0, bundle, 0);
        let renamed = f.0.join("renamed.apparent");
        fs::rename(path, &renamed).unwrap();
        let proof = FinalizerSuccess::validate(&request, bundle, &renamed, &bytes).unwrap();
        let receipt = session.finish(Ok(proof)).unwrap();
        assert_eq!(receipt.status, Status::Saved);
        assert_eq!(receipt.bundle_id, bundle);
        assert_eq!(
            Runtime::open(&f.0)
                .unwrap()
                .inspect(&request)
                .unwrap()
                .unwrap(),
            receipt
        );
        assert!(
            runtime
                .lease(bundle, LeaseKind::Maintenance)
                .unwrap()
                .is_some()
        );
        assert!(
            runtime.admit(bundle, &token, bundle).is_err(),
            "request authority format weakened"
        );
        assert!(
            runtime.admit(&new_id().unwrap(), bundle, bundle).is_err(),
            "token format weakened"
        );
    }
}
#[test]
fn manifests_larger_than_a_receipt_are_supported_but_bounded() {
    let f = Fixture::new();
    let bundle = new_id().unwrap();
    let request = new_id().unwrap();
    let runtime = Runtime::open(&f.0).unwrap();
    let Admission::Acquired(mut session) = runtime
        .admit(&request, &new_owner_token().unwrap(), &bundle)
        .unwrap()
    else {
        panic!("not acquired")
    };
    session.recording().unwrap();
    session.processing().unwrap();
    let (path, _) = output(&f.0, &bundle, 0);
    // Production-shaped high-cardinality fields, still not a full media bundle.
    let points: Vec<_> = (0..10_000).flat_map(|t| [5000, 5000, t]).collect();
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({"version":1,"id":bundle,"strokes":[{"t0":0,"t1":9999,"fw":1920,"fh":1080,"pts":points}],"events":[]})).unwrap();
    assert!(bytes.len() > 65536);
    fs::write(path.join("apparent.json"), &bytes).unwrap();
    let proof = FinalizerSuccess::validate(&request, &bundle, &path, &bytes).unwrap();
    assert_eq!(session.finish(Ok(proof)).unwrap().status, Status::Saved);
    // A valid over-budget manifest must fail, not be silently truncated.
    fs::create_dir(f.0.join("large")).unwrap();
    mode(&f.0.join("large"), 0o700);
    let (_, oversized) = output(&f.0.join("large"), &bundle, 64 * 1024 * 1024);
    assert!(
        FinalizerSuccess::validate(
            &new_id().unwrap(),
            &bundle,
            &f.0.join("large/output.apparent"),
            &oversized
        )
        .is_err()
    );
}

#[test]
fn distinct_bundle_identities_never_share_a_lock() {
    let f = Fixture::new();
    let runtime = Runtime::open(&f.0).unwrap();
    let ids = [
        "a".into(),
        "a".repeat(60),
        "a".repeat(61),
        "a".repeat(255),
        "Demo".into(),
        "demo".into(),
        "é".into(),
        "e\u{301}".into(),
        "61".into(),
        "a".repeat(32),
    ];
    for (i, id) in ids.iter().enumerate() {
        let _held = runtime.lease(id, LeaseKind::Read).unwrap().unwrap();
        assert!(runtime.lease(id, LeaseKind::Maintenance).unwrap().is_none());
        for (j, other) in ids.iter().enumerate() {
            if i != j {
                assert!(
                    runtime
                        .lease(other, LeaseKind::Maintenance)
                        .unwrap()
                        .is_some(),
                    "aliased {id:?} and {other:?}"
                );
            }
        }
    }
    for invalid in [
        "",
        ".",
        "..",
        "../escape",
        "/absolute",
        "a/b",
        "a\\b",
        "nul\0byte",
        "line\nfeed",
        &"x".repeat(256),
    ] {
        assert!(
            runtime.lease(invalid, LeaseKind::Read).is_err(),
            "accepted {invalid:?}"
        );
        assert!(
            runtime
                .admit(&new_id().unwrap(), &new_owner_token().unwrap(), invalid)
                .is_err()
        );
    }
}

#[test]
fn replacing_an_output_ancestor_invalidates_the_proof() {
    let f = Fixture::new();
    let parent = f.0.join("outputs");
    fs::create_dir(&parent).unwrap();
    mode(&parent, 0o700);
    let bundle = new_id().unwrap();
    let request = new_id().unwrap();
    let runtime = Runtime::open(&f.0).unwrap();
    let Admission::Acquired(mut session) = runtime
        .admit(&request, &new_owner_token().unwrap(), &bundle)
        .unwrap()
    else {
        panic!("not acquired")
    };
    session.recording().unwrap();
    session.processing().unwrap();
    let (path, bytes) = output(&parent, &bundle, 0);
    let proof = FinalizerSuccess::validate(&request, &bundle, &path, &bytes).unwrap();
    fs::rename(&parent, f.0.join("old-outputs")).unwrap();
    fs::create_dir(&parent).unwrap();
    mode(&parent, 0o700);
    output(&parent, &bundle, 0);
    assert_eq!(session.finish(Ok(proof)).unwrap().status, Status::Failed);
}
