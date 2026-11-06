use super::recording::{PixelBufferEvidence, has_usable_capture_sample};
use super::*;
use screencapturekit::cm::SCFrameStatus;
use std::fs;
use std::io::{Read, Seek};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

struct PrivateFixture {
    root: PathBuf,
    root_identity: (u64, u64),
    retained: bool,
    files: Vec<OwnedPath>,
    directories: Vec<OwnedPath>,
}

struct OwnedPath {
    path: PathBuf,
    identity: Option<(u64, u64)>,
}

impl PrivateFixture {
    fn new() -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .expect("canonical temp root")
            .join(format!(
                "apparent-sck-diagnostic-{}",
                crate::new_id().unwrap()
            ));
        fs::create_dir(&root).expect("exclusive synthetic test directory");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .expect("private test directory mode");
        let metadata = fs::symlink_metadata(&root).expect("test root identity");
        Self {
            root,
            root_identity: (metadata.dev(), metadata.ino()),
            retained: false,
            files: Vec::new(),
            directories: Vec::new(),
        }
    }

    fn own_file(&mut self, path: PathBuf) -> PathBuf {
        self.files.push(OwnedPath {
            path: path.clone(),
            identity: None,
        });
        path
    }

    fn own_directory(&mut self, path: PathBuf) -> PathBuf {
        self.directories.push(OwnedPath {
            path: path.clone(),
            identity: None,
        });
        path
    }

    fn bind_file(&mut self, path: &Path) {
        let metadata = fs::symlink_metadata(path).expect("owned file identity");
        let owned = self
            .files
            .iter_mut()
            .rev()
            .find(|entry| entry.path == path)
            .unwrap();
        let observed = (metadata.dev(), metadata.ino());
        if let Some(expected) = owned.identity {
            assert_eq!(expected, observed, "owned file identity changed");
        } else {
            owned.identity = Some(observed);
        }
    }

    fn bind_file_if_present(&mut self, path: &Path) {
        match fs::symlink_metadata(path) {
            Ok(_) => self.bind_file(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => panic!("cannot inspect owned output path: {error}"),
        }
    }

    fn bind_directory(&mut self, path: &Path) {
        let metadata = fs::symlink_metadata(path).expect("owned directory identity");
        let owned = self
            .directories
            .iter_mut()
            .rev()
            .find(|entry| entry.path == path)
            .unwrap();
        let observed = (metadata.dev(), metadata.ino());
        if let Some(expected) = owned.identity {
            assert_eq!(expected, observed, "owned directory identity changed");
        } else {
            owned.identity = Some(observed);
        }
    }

    fn finish(&mut self) {
        if std::env::var("APPARENT_TEST_SCK_RETAIN_FIXTURES").as_deref()
            == Ok("retain-synthetic-sck-proof")
        {
            self.retained = true;
            eprintln!("SCK_FIXTURE_RETAINED={}", self.root.display());
        } else {
            self.cleanup().expect("identity-checked fixture cleanup");
        }
    }

    fn cleanup(&mut self) -> std::io::Result<()> {
        for entry in self.files.iter().rev() {
            let Some(expected) = entry.identity else {
                continue;
            };
            match fs::symlink_metadata(&entry.path) {
                Ok(metadata) if (metadata.dev(), metadata.ino()) == expected => {
                    fs::remove_file(&entry.path)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Ok(_) => return Err(std::io::Error::other("owned file identity changed")),
                Err(error) => return Err(error),
            }
        }
        for entry in self.directories.iter().rev() {
            let Some(expected) = entry.identity else {
                continue;
            };
            match fs::symlink_metadata(&entry.path) {
                Ok(metadata) if (metadata.dev(), metadata.ino()) == expected => {
                    fs::remove_dir(&entry.path)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Ok(_) => {
                    return Err(std::io::Error::other("owned directory identity changed"));
                }
                Err(error) => return Err(error),
            }
        }
        let metadata = fs::symlink_metadata(&self.root)?;
        if (metadata.dev(), metadata.ino()) != self.root_identity {
            return Err(std::io::Error::other("fixture root identity changed"));
        }
        fs::remove_dir(&self.root)
    }
}

impl Drop for PrivateFixture {
    fn drop(&mut self) {
        if self.retained || std::thread::panicking() {
            eprintln!("SCK_FIXTURE_RETAINED={}", self.root.display());
            return;
        }
        // Remove only leaves and directories this fixture created or renamed.
        let _ = self.cleanup();
    }
}
#[test]
fn first_frame_readiness_requires_complete_nonempty_pixel_data() {
    let valid = Some(PixelBufferEvidence {
        width: 640,
        height: 360,
        bytes_per_row: 640 * 4,
        data_size: 640 * 360 * 4,
        mapped: true,
    });
    assert!(has_usable_capture_sample(
        Some(SCFrameStatus::Complete),
        true,
        1,
        valid
    ));
    assert!(!has_usable_capture_sample(
        Some(SCFrameStatus::Started),
        true,
        1,
        None
    ));
    assert!(!has_usable_capture_sample(
        Some(SCFrameStatus::Started),
        true,
        1,
        valid
    ));
    assert!(!has_usable_capture_sample(
        Some(SCFrameStatus::Blank),
        true,
        1,
        valid
    ));
    assert!(!has_usable_capture_sample(
        Some(SCFrameStatus::Idle),
        true,
        1,
        valid
    ));
    assert!(!has_usable_capture_sample(
        Some(SCFrameStatus::Suspended),
        true,
        1,
        None
    ));
    assert!(!has_usable_capture_sample(
        Some(SCFrameStatus::Unknown(99)),
        true,
        1,
        valid
    ));
    assert!(!has_usable_capture_sample(None, true, 1, valid));
    assert!(!has_usable_capture_sample(
        Some(SCFrameStatus::Complete),
        false,
        1,
        valid
    ));
    assert!(!has_usable_capture_sample(
        Some(SCFrameStatus::Complete),
        true,
        0,
        valid
    ));
    assert!(!has_usable_capture_sample(
        Some(SCFrameStatus::Complete),
        true,
        1,
        None
    ));
    assert!(!has_usable_capture_sample(
        Some(SCFrameStatus::Complete),
        true,
        1,
        Some(PixelBufferEvidence {
            width: 0,
            ..valid.unwrap()
        })
    ));
    assert!(!has_usable_capture_sample(
        Some(SCFrameStatus::Complete),
        true,
        1,
        Some(PixelBufferEvidence {
            bytes_per_row: 0,
            ..valid.unwrap()
        })
    ));
    assert!(!has_usable_capture_sample(
        Some(SCFrameStatus::Complete),
        true,
        1,
        Some(PixelBufferEvidence {
            data_size: 0,
            ..valid.unwrap()
        })
    ));
    assert!(!has_usable_capture_sample(
        Some(SCFrameStatus::Complete),
        true,
        1,
        Some(PixelBufferEvidence {
            mapped: false,
            ..valid.unwrap()
        })
    ));
}
fn admitted_synthetic_display() -> u32 {
    assert_eq!(
        std::env::var("APPARENT_TEST_SCK_SYNTHETIC_CAPTURE_ACK").as_deref(),
        Ok("parent-admitted-synthetic-display"),
        "parent must explicitly admit the synthetic display run"
    );
    std::env::var("APPARENT_TEST_SCK_SYNTHETIC_DISPLAY_ID")
        .expect("synthetic display id")
        .parse()
        .expect("numeric display id")
}

fn run_sck_output_observing(
    fixture: &mut PrivateFixture,
    expected_file_path: &Path,
    display_id: u32,
    output: PathBuf,
    mut observe: impl FnMut() -> std::io::Result<()>,
) -> CaptureStream {
    let admitted = Instant::now();
    let config = StreamConfiguration {
        width: 640,
        height: 360,
        source_rect: None,
    };
    let stream = CaptureStream::start_explicit_display(display_id, config, output, admitted)
        .unwrap_or_else(|error| {
            fixture.bind_file_if_present(expected_file_path);
            panic!("SCK strict output start failed: {error}");
        });
    let deadline = Instant::now() + Duration::from_secs(10);
    while stream.first_sample().is_none() && !stream.recording_failed() {
        fixture.bind_file_if_present(expected_file_path);
        observe().unwrap_or_else(|error| {
            fixture.bind_file_if_present(expected_file_path);
            panic!("SCK output guard rejected native leaf: {error}");
        });
        assert!(Instant::now() < deadline, "no timestamped content sample");
        thread::sleep(Duration::from_millis(20));
    }
    fixture.bind_file_if_present(expected_file_path);
    observe().unwrap_or_else(|error| panic!("SCK output guard rejected native leaf: {error}"));
    assert!(!stream.recording_failed(), "SCK recording delegate failed");
    assert!(
        stream.recording_started(),
        "SCK start delegate was not observed"
    );
    let sample = stream.first_sample().expect("first native sample");
    eprintln!(
        "SCK first sample: pts={}/{} flags={} epoch={} callback_observed_since_epoch_ns={} pixel={}x{} row_bytes={} data_size={}",
        sample.presentation_value,
        sample.presentation_timescale,
        sample.presentation_flags,
        sample.presentation_epoch,
        sample.callback_observed_since_epoch_ns,
        sample.pixel_width,
        sample.pixel_height,
        sample.pixel_bytes_per_row,
        sample.pixel_data_size
    );
    let observe_deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < observe_deadline {
        fixture.bind_file_if_present(expected_file_path);
        observe().unwrap_or_else(|error| {
            fixture.bind_file_if_present(expected_file_path);
            panic!("SCK output guard rejected native leaf: {error}");
        });
        thread::sleep(Duration::from_millis(20));
    }
    stream
}

fn finish_sck_output(mut stream: CaptureStream) {
    stream.stop();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !stream.recording_finished() && !stream.recording_failed() {
        assert!(Instant::now() < deadline, "SCK stop did not finish output");
        thread::sleep(Duration::from_millis(20));
    }
    assert!(stream.recording_finished(), "SCK output did not finalize");
}
/// Exercises the Store-backed output guard with an absent leaf. This remains an ignored
/// synthetic-display diagnostic; it does not make SCK output available to capture/2.
#[cfg(all(target_os = "macos", feature = "publication"))]
#[test]
#[ignore = "requires parent-admitted synthetic SCK display and capture permission"]
fn sck_store_guard_pins_and_validates_absent_directory_alias_output() {
    use sha2::{Digest, Sha256};

    let display_id = admitted_synthetic_display();
    let mut fixture = PrivateFixture::new();
    let stage_path = fixture.own_directory(fixture.root.join("sck-store-stage"));
    let store = Arc::new(crate::SecureStore::create(&stage_path).unwrap());
    fixture.bind_directory(&stage_path);
    let store_mode = store.directory_metadata().unwrap().mode() & 0o777;
    assert_eq!(store_mode, 0o700, "private stage mode changed");

    let leaf = "guarded-output.mp4";
    let mut guard = SckOutputGuard::new(store, leaf).expect("absent private SCK leaf");
    let output_path = stage_path.join(leaf);
    let _ = fixture.own_file(output_path.clone());
    let stream = run_sck_output_observing(
        &mut fixture,
        &output_path,
        display_id,
        guard.native_output_url().to_owned(),
        || guard.observe_created().map(|_| ()),
    );
    fixture.bind_file_if_present(&output_path);
    finish_sck_output(stream);

    let output = guard
        .verify_finalized()
        .expect("same pinned leaf after successful SCK finish");
    crate::publication::validate_movie(output)
        .expect("Store-guarded output has complete MP4 sample tables");
    let metadata = output.metadata().unwrap();
    let mut bytes = Vec::new();
    let mut readback = output.try_clone().unwrap();
    readback.seek(std::io::SeekFrom::Start(0)).unwrap();
    readback.read_to_end(&mut bytes).unwrap();
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let stage_metadata = fs::metadata(&stage_path).unwrap();
    eprintln!(
        "SCK_STORE_GUARD_EVIDENCE stage=({}, {}) output=({}, {}) mode={:o} nlink={} bytes={} sha256={} movie=valid",
        stage_metadata.dev(),
        stage_metadata.ino(),
        metadata.dev(),
        metadata.ino(),
        metadata.mode() & 0o777,
        metadata.nlink(),
        metadata.len(),
        digest,
    );
    fixture.bind_file(&output_path);
    drop(guard);
    fixture.finish();
}
