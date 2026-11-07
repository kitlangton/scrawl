use super::*;
use std::{
    io::{Seek, SeekFrom, Write},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
};

struct Fixture {
    root_path: PathBuf,
    output: Arc<SecureStore>,
    registry: JobRegistry,
    binding: Binding,
}

fn fixture() -> Fixture {
    let nonce = format!(
        "{:032x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    // Darwin's temporary root can use the /var alias. Resolve the fixture root;
    // production Store still rejects symlinked ancestors.
    let root_path = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("materials-media-registry-{nonce}"));
    std::fs::create_dir(&root_path).unwrap();
    std::fs::set_permissions(&root_path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = Arc::new(SecureStore::create(&root_path.join("runtime")).unwrap());
    let output = Arc::new(SecureStore::create(&root_path.join("output")).unwrap());
    let metadata = output.directory_metadata().unwrap();
    let registry = JobRegistry::open(runtime.clone()).unwrap();
    let binding = Binding {
        schema: "apparent.media2-job/1".into(),
        capture_id: "a".repeat(32),
        materials: MaterialsKey {
            capture_id: "a".repeat(32),
            materials_id: "b".repeat(32),
            revision: "c".repeat(64),
        },
        materials_root: root_path.join("retained").display().to_string(),
        materials_root_identity: OutputIdentity {
            device: "1".into(),
            inode: "2".into(),
        },
        lease_id: "d".repeat(32),
        source_video: SealedFile {
            relative_path: "recording.mov".into(),
            identity: FileIdentity {
                device: "1".into(),
                inode: "2".into(),
                size: 3,
                modified_ns: "4".into(),
            },
            sha256: "e".repeat(64),
        },
        video_offset_ms: 0,
        job_id: "f".repeat(32),
        receipt_id: "1".repeat(32),
        request_sha256: "2".repeat(64),
        requested_bundle_ms: vec![7],
        limits: Limits {
            max_frames: 1,
            max_edge: 64,
            max_total_pixels: 4096,
            deadline_ms: 1000,
        },
        output_root: root_path.join("output").display().to_string(),
        output_identity: OutputIdentity {
            device: metadata.dev().to_string(),
            inode: metadata.ino().to_string(),
        },
        layout: None,
    };
    Fixture {
        root_path,
        output,
        registry,
        binding,
    }
}

fn seal(store: &SecureStore, relative: &str, leaf: &str) -> SealedFile {
    let mut file = store.file(leaf, false).unwrap();
    let m = file.metadata().unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    SealedFile {
        relative_path: relative.into(),
        identity: FileIdentity {
            device: m.dev().to_string(),
            inode: m.ino().to_string(),
            size: m.len(),
            modified_ns: (i128::from(m.mtime()) * 1_000_000_000 + i128::from(m.mtime_nsec()))
                .to_string(),
        },
        sha256: hex(&Sha256::digest(bytes)),
    }
}

fn write_tracked_frame(f: &Fixture, reference: &JobReference, contents: &[u8]) -> File {
    f.registry
        .reserve_output_directory(reference, &f.output)
        .unwrap();
    let frames = f.output.create_child("frames").unwrap();
    f.registry
        .record_output_directory(reference, &f.output, &frames)
        .unwrap();
    f.registry
        .reserve_output_file(reference, &f.output, "frames/frame-001.png")
        .unwrap();
    let mut file = frames.file("frame-001.png", true).unwrap();
    f.registry
        .record_output_file(reference, &f.output, "frames/frame-001.png", &file)
        .unwrap();
    file.write_all(contents).unwrap();
    file.sync_all().unwrap();
    file
}

fn write_tracked_root_file(f: &Fixture, reference: &JobReference, name: &str, contents: &[u8]) {
    f.registry
        .reserve_output_file(reference, &f.output, name)
        .unwrap();
    let mut file = f.output.file(name, true).unwrap();
    f.registry
        .record_output_file(reference, &f.output, name, &file)
        .unwrap();
    file.write_all(contents).unwrap();
    file.sync_all().unwrap();
}

#[test]
fn reaped_partial_outputs_are_removed_for_cancel_deadline_failure_and_owner_loss() {
    for (suffix, checkpoint, expected_state) in [
        ("1", Checkpoint::Cancel, StoredJobState::Cancelled),
        ("2", Checkpoint::Deadline, StoredJobState::DeadlineExceeded),
        ("3", Checkpoint::Failed, StoredJobState::Failed),
        ("4", Checkpoint::OwnerLost, StoredJobState::OwnerLost),
    ] {
        let f = fixture();
        let mut binding = f.binding.clone();
        binding.job_id = suffix.repeat(32);
        binding.receipt_id = format!("{}{}", suffix, "a".repeat(31));
        let JobOutcome::Owned(claim) = f.registry.begin(binding.clone(), f.output.clone()).unwrap()
        else {
            panic!()
        };
        let reference = intent_from_binding(&binding).unwrap().reference();
        write_tracked_frame(&f, &reference, b"partial frame");
        write_tracked_root_file(&f, &reference, "receipt.json", b"partial receipt");
        std::fs::write(f.root_path.join("output/caller-owned.txt"), b"preserve").unwrap();
        drop(claim);

        assert!(matches!(
            f.registry
                .finish_reaped(&binding, f.output.clone(), checkpoint)
                .unwrap(),
            JobOutcome::ExistingFailed(_)
        ));
        assert_eq!(
            f.registry.inspect_reference(&reference).unwrap().state,
            expected_state
        );
        assert!(!f.root_path.join("output/frames").exists());
        assert!(!f.root_path.join("output/receipt.json").exists());
        assert_eq!(
            std::fs::read(f.root_path.join("output/caller-owned.txt")).unwrap(),
            b"preserve"
        );
        let root = f.root_path.clone();
        drop(f);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn cleanup_refuses_replaced_files_and_preserves_untracked_frames_entries() {
    let f = fixture();
    let JobOutcome::Owned(claim) = f
        .registry
        .begin(f.binding.clone(), f.output.clone())
        .unwrap()
    else {
        panic!()
    };
    let reference = intent_from_binding(&f.binding).unwrap().reference();
    let original = write_tracked_frame(&f, &reference, b"native bytes");
    let original_meta = original.metadata().unwrap();
    let frame_path = f.root_path.join("output/frames/frame-001.png");
    std::fs::remove_file(&frame_path).unwrap();
    let frames = f.output.existing_child("frames").unwrap();
    frames
        .write_new("frame-001.png", b"replacement bytes")
        .unwrap();
    drop(claim);
    assert!(
        f.registry
            .finish_reaped(&f.binding, f.output.clone(), Checkpoint::Failed)
            .is_err()
    );
    assert_eq!(std::fs::read(&frame_path).unwrap(), b"replacement bytes");
    assert_ne!(
        original_meta.ino(),
        frames
            .file("frame-001.png", false)
            .unwrap()
            .metadata()
            .unwrap()
            .ino()
    );
    let root = f.root_path.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();

    let f = fixture();
    let JobOutcome::Owned(claim) = f
        .registry
        .begin(f.binding.clone(), f.output.clone())
        .unwrap()
    else {
        panic!()
    };
    let reference = intent_from_binding(&f.binding).unwrap().reference();
    write_tracked_frame(&f, &reference, b"native bytes");
    let frames = f.output.existing_child("frames").unwrap();
    frames
        .write_new("caller-owned.txt", b"keep unknown entry")
        .unwrap();
    drop(claim);
    assert!(
        f.registry
            .finish_reaped(&f.binding, f.output.clone(), Checkpoint::Cancel)
            .is_err()
    );
    assert!(!f.root_path.join("output/frames/frame-001.png").exists());
    assert_eq!(
        std::fs::read(f.root_path.join("output/frames/caller-owned.txt")).unwrap(),
        b"keep unknown entry"
    );
    let root = f.root_path.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cleanup_preserves_reservations_without_a_bound_inode() {
    let f = fixture();
    let JobOutcome::Owned(claim) = f
        .registry
        .begin(f.binding.clone(), f.output.clone())
        .unwrap()
    else {
        panic!()
    };
    f.output.create_child("frames").unwrap();
    drop(claim);
    assert!(
        f.registry
            .finish_reaped(&f.binding, f.output.clone(), Checkpoint::Failed)
            .is_err()
    );
    assert!(f.root_path.join("output/frames").is_dir());
    let root = f.root_path.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();

    let f = fixture();
    let JobOutcome::Owned(claim) = f
        .registry
        .begin(f.binding.clone(), f.output.clone())
        .unwrap()
    else {
        panic!()
    };
    let reference = intent_from_binding(&f.binding).unwrap().reference();
    f.registry
        .reserve_output_directory(&reference, &f.output)
        .unwrap();
    f.output.create_child("frames").unwrap();
    drop(claim);
    assert!(
        f.registry
            .finish_reaped(&f.binding, f.output.clone(), Checkpoint::Failed)
            .is_err()
    );
    assert!(f.root_path.join("output/frames").is_dir());
    let root = f.root_path.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();

    let f = fixture();
    let JobOutcome::Owned(claim) = f
        .registry
        .begin(f.binding.clone(), f.output.clone())
        .unwrap()
    else {
        panic!()
    };
    let reference = intent_from_binding(&f.binding).unwrap().reference();
    f.registry
        .reserve_output_file(&reference, &f.output, "receipt.json")
        .unwrap();
    let mut receipt = f.output.file("receipt.json", true).unwrap();
    receipt.write_all(b"uncertain partial receipt").unwrap();
    receipt.sync_all().unwrap();
    drop(claim);
    assert!(
        f.registry
            .finish_reaped(&f.binding, f.output.clone(), Checkpoint::Failed)
            .is_err()
    );
    assert_eq!(
        std::fs::read(f.root_path.join("output/receipt.json")).unwrap(),
        b"uncertain partial receipt"
    );
    let root = f.root_path.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_owner_collision_and_replay_cannot_rebind_request() {
    let f = fixture();
    let JobOutcome::Owned(claim) = f
        .registry
        .begin(f.binding.clone(), f.output.clone())
        .unwrap()
    else {
        panic!()
    };
    assert!(
        f.registry
            .begin(f.binding.clone(), f.output.clone())
            .is_err()
    );
    drop(claim);
    assert!(matches!(
        f.registry
            .begin(f.binding.clone(), f.output.clone())
            .unwrap(),
        JobOutcome::ExistingRunning
    ));
    assert!(matches!(
        f.registry
            .finish_reaped(&f.binding, f.output.clone(), Checkpoint::Cancel)
            .unwrap(),
        JobOutcome::ExistingFailed("cancelled")
    ));
    let mut changed = f.binding.clone();
    changed.requested_bundle_ms[0] += 1;
    changed.request_sha256 = "3".repeat(64);
    assert!(f.registry.begin(changed, f.output.clone()).is_err());
    let root = f.root_path.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn intent_is_durable_before_source_pin_and_replay_never_refreshes_deadline() {
    let f = fixture();
    let intent = intent_from_binding(&f.binding).unwrap();
    let reference = intent.reference();
    let JobOutcome::Owned(claim) = f.registry.admit(intent.clone(), f.output.clone()).unwrap()
    else {
        panic!()
    };
    let first = f.registry.inspect_reference(&reference).unwrap();
    assert_eq!(first.state, StoredJobState::Preparing);
    assert!(first.binding.is_none());
    let admitted_at = first.intent.admitted_at_ms;
    let original_remaining = first.intent.remaining_deadline_ms();

    let mut changed_limits = intent.clone();
    changed_limits.limits.max_edge += 1;
    assert!(f.registry.admit(changed_limits, f.output.clone()).is_err());
    let mut changed_layout = intent.clone();
    changed_layout.layout = Some(super::super::contact_sheet::Layout {
        schema: "apparent.contact-sheet-layout/1".into(),
        width: 1,
        height: 1,
        background_rgb: [0, 0, 0],
        fit: "stretch".into(),
        sampling: "legacy-bilinear".into(),
        clipping: "reject".into(),
        tiles: vec![super::super::contact_sheet::TileRect {
            request_index: 1,
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        }],
    });
    assert!(f.registry.admit(changed_layout, f.output.clone()).is_err());
    assert!(matches!(
        f.registry.admit(intent.clone(), f.output.clone()).unwrap(),
        JobOutcome::ExistingInProgress
    ));
    drop(claim);

    // A free owner lock is not proof of reaping and cannot restart work.
    assert!(matches!(
        f.registry.admit(intent, f.output.clone()).unwrap(),
        JobOutcome::ExistingPreparing
    ));
    let replay = f.registry.inspect_reference(&reference).unwrap();
    assert_eq!(replay.intent.admitted_at_ms, admitted_at);
    assert!(replay.intent.remaining_deadline_ms() <= original_remaining);
    assert!(matches!(
        f.registry
            .finish_intent_reaped(&reference, f.output.clone(), Checkpoint::Deadline)
            .unwrap(),
        JobOutcome::ExistingFailed("deadline_exceeded")
    ));
    assert_eq!(
        f.registry.inspect_reference(&reference).unwrap().state,
        StoredJobState::DeadlineExceeded
    );
    let mut failed_intent = intent_from_binding(&f.binding).unwrap();
    failed_intent.job_id = "9".repeat(32);
    let failed_reference = failed_intent.reference();
    let JobOutcome::Owned(failed_claim) =
        f.registry.admit(failed_intent, f.output.clone()).unwrap()
    else {
        panic!()
    };
    drop(failed_claim);
    assert!(matches!(
        f.registry
            .finish_intent_reaped(&failed_reference, f.output.clone(), Checkpoint::Failed)
            .unwrap(),
        JobOutcome::ExistingFailed("failed")
    ));
    assert_eq!(
        f.registry
            .inspect_reference(&failed_reference)
            .unwrap()
            .state,
        StoredJobState::Failed
    );
    let mut lost_intent = intent_from_binding(&f.binding).unwrap();
    lost_intent.job_id = "8".repeat(32);
    let lost_reference = lost_intent.reference();
    let JobOutcome::Owned(lost_claim) = f.registry.admit(lost_intent, f.output.clone()).unwrap()
    else {
        panic!()
    };
    drop(lost_claim);
    assert!(matches!(
        f.registry
            .finish_intent_reaped(&lost_reference, f.output.clone(), Checkpoint::OwnerLost)
            .unwrap(),
        JobOutcome::ExistingFailed("owner_lost")
    ));
    assert_eq!(
        f.registry.inspect_reference(&lost_reference).unwrap().state,
        StoredJobState::OwnerLost
    );
    let root = f.root_path.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn losing_supervisor_reconciliation_leaves_the_live_intent_untouched() {
    let f = fixture();
    let intent = intent_from_binding(&f.binding).unwrap();
    let reference = intent.reference();
    let JobOutcome::Owned(mut claim) = f.registry.admit(intent, f.output.clone()).unwrap() else {
        panic!()
    };
    claim.bind_source(f.binding.clone()).unwrap();
    let frame = write_tracked_frame(&f, &reference, b"live owner output");
    let frame_path = f.root_path.join("output/frames/frame-001.png");
    let started = std::time::Instant::now();
    assert!(matches!(
        f.registry
            .finish_intent_reaped(&reference, f.output.clone(), Checkpoint::OwnerLost)
            .unwrap(),
        JobOutcome::ExistingInProgress
    ));
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(std::fs::read(&frame_path).unwrap(), b"live owner output");
    assert_eq!(frame.metadata().unwrap().len(), 17);
    assert_eq!(
        f.registry.inspect_reference(&reference).unwrap().state,
        StoredJobState::Running
    );
    let mut wrong = reference.clone();
    wrong.request_sha256 = "9".repeat(64);
    assert!(
        f.registry
            .finish_intent_reaped(&wrong, f.output.clone(), Checkpoint::OwnerLost)
            .is_err()
    );
    assert_eq!(
        f.registry.request_cancel(&reference).unwrap(),
        CancelRequestOutcome::Requested
    );
    let root = f.root_path.clone();
    drop(claim);
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cold_recovery_never_fences_a_live_owner_or_rebinds_an_abandoned_job() {
    let f = fixture();
    let intent = intent_from_binding(&f.binding).unwrap();
    let reference = intent.reference();
    let JobOutcome::Owned(mut claim) = f.registry.admit(intent, f.output.clone()).unwrap() else {
        panic!()
    };
    assert_eq!(
        f.registry.reconcile_if_ownerless(&reference).unwrap().state,
        StoredJobState::Preparing
    );
    claim.bind_source(f.binding.clone()).unwrap();
    write_tracked_frame(&f, &reference, b"abandoned frame");
    assert_eq!(
        f.registry.reconcile_if_ownerless(&reference).unwrap().state,
        StoredJobState::Running
    );
    drop(claim);
    let mut wrong = reference.clone();
    wrong.request_sha256 = "9".repeat(64);
    assert!(f.registry.reconcile_if_ownerless(&wrong).is_err());
    assert_eq!(
        f.registry.inspect_reference(&reference).unwrap().state,
        StoredJobState::Running
    );
    assert_eq!(
        f.registry.reconcile_if_ownerless(&reference).unwrap().state,
        StoredJobState::OwnerLost
    );
    assert!(!f.root_path.join("output/frames").exists());
    assert_eq!(
        f.registry.reconcile_if_ownerless(&reference).unwrap().state,
        StoredJobState::OwnerLost
    );
    let root = f.root_path.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn inspection_waits_for_atomic_record_link_cleanup() {
    let f = fixture();
    let intent = intent_from_binding(&f.binding).unwrap();
    let JobOutcome::Owned(claim) = f.registry.admit(intent.clone(), f.output.clone()).unwrap()
    else {
        panic!()
    };
    let directory = f
        .root_path
        .join("runtime/materials-media-2")
        .join(&intent.job_id);
    let job = f.registry.namespace.existing_child(&intent.job_id).unwrap();
    let control = control_lock(&job).unwrap();
    // Faithfully reproduce Store::write_new's link-before-unlink window.
    std::fs::hard_link(
        directory.join("intent.json"),
        directory.join("temporary-link"),
    )
    .unwrap();
    let registry = JobRegistry {
        namespace: f.registry.namespace.clone(),
    };
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        done_tx
            .send(
                registry
                    .lookup_authenticated(
                        &intent.capture_id,
                        &intent.job_id,
                        &intent.request_sha256,
                    )
                    .map(|job| job.state),
            )
            .unwrap();
    });
    started_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    let premature = done_rx.recv_timeout(std::time::Duration::from_millis(50));
    std::fs::remove_file(directory.join("temporary-link")).unwrap();
    drop(control);
    assert!(matches!(
        premature,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    assert_eq!(
        done_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .unwrap(),
        StoredJobState::Preparing
    );
    reader.join().unwrap();
    let root = f.root_path.clone();
    drop(claim);
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn authenticated_lookup_requires_the_original_capture_and_request() {
    let f = fixture();
    let intent = intent_from_binding(&f.binding).unwrap();
    let JobOutcome::Owned(_claim) = f.registry.admit(intent.clone(), f.output.clone()).unwrap()
    else {
        panic!()
    };
    let lookup =
        |capture: &str, job: &str, hash: &str| f.registry.lookup_authenticated(capture, job, hash);
    let stored = lookup(&intent.capture_id, &intent.job_id, &intent.request_sha256).unwrap();
    assert_eq!(stored.intent.materials, intent.materials);
    assert!(lookup(&"9".repeat(32), &intent.job_id, &intent.request_sha256).is_err());
    assert!(lookup(&intent.capture_id, &intent.job_id, &"9".repeat(64)).is_err());
    assert!(lookup(&intent.capture_id, &"8".repeat(32), &intent.request_sha256).is_err());
    assert!(
        !f.root_path
            .join("runtime/materials-media-2")
            .join("8".repeat(32))
            .exists()
    );
    let root = f.root_path.clone();
    drop(_claim);
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unknown_controls_do_not_provision_job_directories() {
    let f = fixture();
    let mut reference = intent_from_binding(&f.binding).unwrap().reference();
    reference.job_id = "7".repeat(32);
    assert!(f.registry.inspect_reference(&reference).is_err());
    assert!(f.registry.request_cancel(&reference).is_err());
    assert!(f.registry.observe_cancel(&reference).is_err());
    assert!(
        !f.root_path
            .join("runtime/materials-media-2")
            .join(&reference.job_id)
            .exists()
    );
    let root = f.root_path.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cold_cancel_uses_durable_intent_without_owner_lock_and_wins_reap_race() {
    let f = fixture();
    let intent = intent_from_binding(&f.binding).unwrap();
    let reference = intent.reference();
    let JobOutcome::Owned(mut claim) = f.registry.admit(intent, f.output.clone()).unwrap() else {
        panic!()
    };
    // `claim` holds the long execution lock: the cold control still commits.
    let mut wrong_reference = reference.clone();
    wrong_reference.capture_id = "9".repeat(32);
    assert!(f.registry.request_cancel(&wrong_reference).is_err());
    wrong_reference = reference.clone();
    wrong_reference.job_id = "../escape".into();
    assert!(f.registry.request_cancel(&wrong_reference).is_err());
    assert!(f.registry.observe_cancel(&wrong_reference).is_err());
    assert_eq!(
        f.registry.observe_cancel(&reference).unwrap(),
        CancelObservation::NotRequested
    );
    assert_eq!(
        f.registry.request_cancel(&reference).unwrap(),
        CancelRequestOutcome::Requested
    );
    assert_eq!(
        f.registry.request_cancel(&reference).unwrap(),
        CancelRequestOutcome::AlreadyRequested
    );
    assert_eq!(
        f.registry.observe_cancel(&reference).unwrap(),
        CancelObservation::Requested
    );
    assert!(claim.cancel_requested().unwrap());
    assert!(claim.bind_source(f.binding.clone()).is_err());
    drop(claim);
    assert!(matches!(
        f.registry
            .finish_intent_reaped(&reference, f.output.clone(), Checkpoint::Deadline)
            .unwrap(),
        JobOutcome::ExistingFailed("cancelled")
    ));
    assert_eq!(
        f.registry.inspect_reference(&reference).unwrap().state,
        StoredJobState::Cancelled
    );
    let root = f.root_path.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn supervisor_records_cancel_or_deadline_only_after_reaping_owner() {
    let f = fixture();
    let JobOutcome::Owned(claim) = f
        .registry
        .begin(f.binding.clone(), f.output.clone())
        .unwrap()
    else {
        panic!()
    };
    assert!(
        f.registry
            .finish_reaped(&f.binding, f.output.clone(), Checkpoint::Cancel)
            .is_err()
    );
    drop(claim);
    assert!(matches!(
        f.registry
            .finish_reaped(&f.binding, f.output.clone(), Checkpoint::Cancel)
            .unwrap(),
        JobOutcome::ExistingFailed("cancelled")
    ));
    assert!(matches!(
        f.registry
            .begin(f.binding.clone(), f.output.clone())
            .unwrap(),
        JobOutcome::ExistingFailed("cancelled")
    ));

    let mut deadline = f.binding.clone();
    deadline.job_id = "9".repeat(32);
    deadline.receipt_id = "8".repeat(32);
    let JobOutcome::Owned(claim) = f
        .registry
        .begin(deadline.clone(), f.output.clone())
        .unwrap()
    else {
        panic!()
    };
    drop(claim);
    assert!(matches!(
        f.registry
            .finish_reaped(&deadline, f.output.clone(), Checkpoint::Deadline)
            .unwrap(),
        JobOutcome::ExistingFailed("deadline_exceeded")
    ));
    assert!(matches!(
        f.registry
            .begin(deadline.clone(), f.output.clone())
            .unwrap(),
        JobOutcome::ExistingFailed("deadline_exceeded")
    ));
    let root = f.root_path.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn completed_authority_requires_registry_and_detects_output_tampering() {
    let f = fixture();
    let frames = f.output.create_child("frames").unwrap();
    frames
        .write_new("frame-001.png", b"native-image-bytes")
        .unwrap();
    let frame = seal(&frames, "frames/frame-001.png", "frame-001.png");
    let payload = serde_json::json!({
        "schema":"apparent.media-derivative-payload/2",
        "materials":{"captureId":f.binding.capture_id,"materialsId":f.binding.materials.materials_id,"revision":f.binding.materials.revision},
        "leaseId":f.binding.lease_id,"jobId":f.binding.job_id,"receiptId":f.binding.receipt_id,
        "root":f.binding.output_root,"requestSha256":f.binding.request_sha256,
        "frames":[{
            "requestIndex":1,"requestedBundleMs":7,"effectiveVideoMs":7,"actualVideoMs":7,
            "clampedToStart":false,"clampedToEnd":false,"width":1,"height":1,
            "encoding":"png","byteLength":frame.identity.size,"file":frame
        }],"contactSheet":null
    });
    let payload_bytes = serde_json::to_vec(&payload).unwrap();
    f.output.write_new("receipt.json", &payload_bytes).unwrap();
    let payload_file = seal(&f.output, "receipt.json", "receipt.json");
    let envelope = serde_json::to_vec(&serde_json::json!({
        "schema":"apparent.media-derivative/2",
        "materials":{"captureId":f.binding.capture_id,"materialsId":f.binding.materials.materials_id,"revision":f.binding.materials.revision},
        "leaseId":f.binding.lease_id,"jobId":f.binding.job_id,"receiptId":f.binding.receipt_id,
        "root":f.binding.output_root,"requestSha256":f.binding.request_sha256,
        "receiptFile":payload_file,"payload":payload
    })).unwrap();
    let receipt = NativeReceipt::new(&f.binding, &[frame], payload_file, &envelope, None).unwrap();
    let JobOutcome::Owned(mut claim) = f
        .registry
        .begin(f.binding.clone(), f.output.clone())
        .unwrap()
    else {
        panic!()
    };
    let reference = intent_from_binding(&f.binding).unwrap().reference();
    claim.complete(receipt, &envelope).unwrap();
    drop(claim);
    assert!(matches!(
        f.registry
            .finish_reaped(&f.binding, f.output.clone(), Checkpoint::Cancel)
            .unwrap(),
        JobOutcome::ExistingComplete(_, _)
    ));
    assert!(f.root_path.join("output/frames/frame-001.png").exists());
    assert!(f.root_path.join("output/receipt.json").exists());
    assert_eq!(
        f.registry
            .request_cancel(&intent_from_binding(&f.binding).unwrap().reference())
            .unwrap(),
        CancelRequestOutcome::Complete
    );
    f.output
        .write_new("manifest.json", br#"{"schema":"caller-forgery"}"#)
        .unwrap();
    let inspected = f.registry.inspect_reference(&reference).unwrap();
    assert_eq!(inspected.state, StoredJobState::Complete);
    assert!(inspected.binding.is_some());
    assert!(inspected.receipt.is_some());
    assert_eq!(inspected.envelope.as_deref(), Some(envelope.as_slice()));
    let mut wrong_reference = reference.clone();
    wrong_reference.capture_id = "9".repeat(32);
    assert!(f.registry.inspect_reference(&wrong_reference).is_err());
    wrong_reference = reference.clone();
    wrong_reference.lease_id = "9".repeat(32);
    assert!(f.registry.inspect_reference(&wrong_reference).is_err());
    wrong_reference = reference.clone();
    wrong_reference.request_sha256 = "0".repeat(64);
    assert!(f.registry.inspect_reference(&wrong_reference).is_err());
    assert!(
        f.registry
            .verify_completed(&f.binding, f.output.clone())
            .is_ok()
    );
    assert!(matches!(
        f.registry
            .begin(f.binding.clone(), f.output.clone())
            .unwrap(),
        JobOutcome::ExistingComplete(_, _)
    ));

    let mut tampered = frames.file("frame-001.png", false).unwrap();
    tampered.seek(SeekFrom::Start(0)).unwrap();
    tampered.write_all(b"T").unwrap();
    tampered.sync_all().unwrap();
    assert!(
        f.registry
            .verify_completed(&f.binding, f.output.clone())
            .is_err()
    );

    let forged_only = f.root_path.join("forged");
    let forged = Arc::new(SecureStore::create(&forged_only).unwrap());
    forged
        .write_new(
            "manifest.json",
            br#"{"schema":"apparent.media-derivative/2"}"#,
        )
        .unwrap();
    assert!(f.registry.verify_completed(&f.binding, forged).is_err());
    let root = f.root_path.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}
