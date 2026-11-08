use apparent_capture_engine::{
    Admission, FinalizerSuccess, Runtime, Status, new_id, new_owner_token,
};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

#[test]
fn finalizer_rechecks_exact_bytes_directory_and_completion_markers() {
    let root = std::env::temp_dir().join(format!("engine-finalizer-{}", new_id().unwrap()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = Runtime::open(&root).unwrap();
    for mutation in [
        "bytes",
        "rename",
        "unsafe",
        "unfinished",
        "symlink",
        "missing",
    ] {
        let request = new_id().unwrap();
        let bundle = new_id().unwrap();
        let token = new_owner_token().unwrap();
        let Admission::Acquired(mut session) = runtime.admit(&request, &token, &bundle).unwrap()
        else {
            panic!("not acquired")
        };
        assert!(session.processing().is_err());
        session.recording().unwrap();
        assert!(session.recording().is_err());
        session.processing().unwrap();
        let path = root.join(&bundle);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let manifest = format!(r#"{{"version":1,"id":"{bundle}"}}"#);
        let leaf = path.join("apparent.json");
        fs::write(&leaf, &manifest).unwrap();
        fs::set_permissions(&leaf, fs::Permissions::from_mode(0o600)).unwrap();
        let proof =
            FinalizerSuccess::validate(&request, &bundle, &path, manifest.as_bytes()).unwrap();
        match mutation {
            "bytes" => fs::write(&leaf, b"{}").unwrap(),
            "rename" => {
                fs::rename(&path, root.join("moved")).unwrap();
                fs::create_dir(&path).unwrap();
            }
            "unsafe" => fs::write(path.join(".unsafe-video"), b"").unwrap(),
            "unfinished" => fs::write(path.join("unfinished.json"), b"{}").unwrap(),
            "symlink" => {
                fs::rename(&leaf, path.join("original")).unwrap();
                symlink(path.join("original"), &leaf).unwrap();
            }
            "missing" => fs::remove_file(&leaf).unwrap(),
            _ => unreachable!(),
        }
        let receipt = session.finish(Ok(proof)).unwrap();
        assert_eq!(receipt.status, Status::Failed, "{mutation}");
        assert!(receipt.bundle_path.is_none());
        assert_eq!(runtime.inspect(&request).unwrap().unwrap(), receipt);
    }
    fs::remove_dir_all(root).unwrap();
}
