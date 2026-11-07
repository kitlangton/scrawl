#![cfg(feature = "recording-notes")]

use apparent_capture_engine::recording::notes::{Note, commit, read};
use std::os::unix::fs::MetadataExt;

#[test]
fn saved_notes_commit_uses_caller_nonce_and_reads_back_the_versioned_document() {
    let directory = std::env::temp_dir().join(format!(
        "apparent-core-notes-{}",
        apparent_capture_engine::new_id().unwrap()
    ));
    std::fs::create_dir(&directory).unwrap();
    let notes = vec![Note {
        pin: 7,
        t: 125,
        x: 0.25,
        y: 0.75,
        text: "accepted note".into(),
    }];

    commit(&directory, "owner-nonce", &notes).unwrap();

    let bytes = std::fs::read(directory.join("notes.json")).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
        serde_json::json!({ "version": 1, "notes": [{
            "pin": 7, "t": 125, "x": 0.25, "y": 0.75, "text": "accepted note"
        }] })
    );
    let metadata = std::fs::metadata(directory.join("notes.json")).unwrap();
    assert_eq!(metadata.mode() & 0o777, 0o600);
    assert_eq!(read(&directory).unwrap(), notes);
    assert!(!directory.join(".notes-owner-nonce.tmp").exists());

    std::fs::remove_dir_all(directory).unwrap();
}
