use super::recording::{
    CheckedStopOutcome, PixelBufferEvidence, RecordingFinalization, StopOperation,
    has_usable_capture_sample, recording_finalization,
};
use super::*;
use screencapturekit::cm::SCFrameStatus;
use std::fs;
use std::io::{Read, Seek, Write};
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

fn base64_profile(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0] as usize;
        let second = chunk.get(1).copied().unwrap_or(0) as usize;
        let third = chunk.get(2).copied().unwrap_or(0) as usize;
        encoded.push(TABLE[first >> 2] as char);
        encoded.push(TABLE[((first & 0x03) << 4) | (second >> 4)] as char);
        encoded.push(if chunk.len() > 1 {
            TABLE[((second & 0x0f) << 2) | (third >> 6)] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            TABLE[third & 0x3f] as char
        } else {
            '='
        });
    }
    encoded
}

#[test]
fn sck_icc_profile_base64_has_standard_padding() {
    assert_eq!(base64_profile(b"f"), "Zg==");
    assert_eq!(base64_profile(b"fo"), "Zm8=");
    assert_eq!(base64_profile(b"foo"), "Zm9v");
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

#[test]
fn checked_stop_requires_successful_native_operations_and_finish_callback() {
    let successful = CheckedStopOutcome {
        stop_capture: StopOperation::Succeeded,
        remove_recording_output: StopOperation::Succeeded,
        recording_finalization: RecordingFinalization::Finished,
    };
    assert!(successful.is_finalized());

    for outcome in [
        CheckedStopOutcome {
            recording_finalization: RecordingFinalization::Pending,
            ..successful.clone()
        },
        CheckedStopOutcome {
            recording_finalization: RecordingFinalization::Failed,
            ..successful.clone()
        },
        CheckedStopOutcome {
            stop_capture: StopOperation::Failed("stop failed".into()),
            ..successful.clone()
        },
        CheckedStopOutcome {
            remove_recording_output: StopOperation::Failed("remove failed".into()),
            ..successful.clone()
        },
        CheckedStopOutcome {
            stop_capture: StopOperation::NotAttempted,
            ..successful.clone()
        },
        CheckedStopOutcome {
            remove_recording_output: StopOperation::NotAttempted,
            ..successful.clone()
        },
    ] {
        assert!(
            !outcome.is_finalized(),
            "unexpected finalization: {outcome:?}"
        );
    }
}

#[test]
fn recording_failure_dominates_finish_and_untracked_output_stays_unproven() {
    assert_eq!(
        recording_finalization(true, true, true),
        RecordingFinalization::Failed
    );
    assert_eq!(
        recording_finalization(true, false, false),
        RecordingFinalization::Pending
    );
    assert_eq!(
        recording_finalization(false, true, false),
        RecordingFinalization::NotApplicable
    );
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
    let admission = CaptureAdmissionTime::now().expect("CoreMedia host clock admission time");
    let observation_epoch = Instant::now();
    let setup_delay = Duration::from_millis(250);
    thread::sleep(setup_delay);
    let config = StreamConfiguration {
        width: 640,
        height: 360,
        source_rect: None,
    };
    let stream = CaptureStream::start_explicit_display_with_clock(
        display_id,
        config,
        output,
        observation_epoch,
        admission,
    )
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
    let clock_sample = stream
        .first_sample_capture_clock()
        .expect("SCK-to-host clock conversion")
        .expect("first sample capture clock mapping");
    assert!(
        clock_sample.capture_relative_ms >= setup_delay.as_millis() as u64,
        "capture-relative source timestamp omitted deliberate pre-start delay"
    );
    eprintln!(
        "SCK_CLOCK_EVIDENCE setup_delay_ms={} admission={:?} source_pts={:?} host_pts={:?} capture_relative_ms={} callback_observed_since_epoch_ns={} pixel={}x{} row_bytes={} data_size={} encoded_origin=unproven",
        setup_delay.as_millis(),
        admission.timestamp(),
        clock_sample.source_pts,
        clock_sample.host_pts,
        clock_sample.capture_relative_ms,
        sample.callback_observed_since_epoch_ns,
        sample.pixel_width,
        sample.pixel_height,
        sample.pixel_bytes_per_row,
        sample.pixel_data_size
    );
    let observe_deadline = Instant::now() + Duration::from_millis(1_500);
    while Instant::now() < observe_deadline
        && stream.test_sample_count().is_ok_and(|count| count < 12)
    {
        fixture.bind_file_if_present(expected_file_path);
        observe().unwrap_or_else(|error| {
            fixture.bind_file_if_present(expected_file_path);
            panic!("SCK output guard rejected native leaf: {error}");
        });
        thread::sleep(Duration::from_millis(20));
    }
    let observations = stream
        .test_sample_pixels()
        .expect("bounded supported SCK pixel observations");
    assert!(
        observations.len() >= 8,
        "too few source samples to disambiguate the movie"
    );
    let mappings: Vec<_> = observations
        .iter()
        .map(|sample| {
            stream
                .test_sample_clock(sample.pts)
                .expect("source sample clock conversion")
        })
        .collect();
    let source_span_ms = mappings
        .last()
        .unwrap()
        .capture_relative_ms
        .saturating_sub(mappings[0].capture_relative_ms);
    assert!(
        source_span_ms >= 500,
        "source observations do not span scene changes"
    );

    let evidence_path = fixture.own_file(fixture.root.join("sck-source-pixels.bgra"));
    let mut pixel_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&evidence_path)
        .expect("exclusive source sample evidence file");
    let mut entries = Vec::with_capacity(observations.len());
    let mut byte_offset = 0usize;
    let timestamp_json = |timestamp: ClockTimestamp| {
        serde_json::json!({
            "value": timestamp.value,
            "timescale": timestamp.timescale,
            "flags": timestamp.flags,
            "epoch": timestamp.epoch,
        })
    };
    for (sample, mapping) in observations.iter().zip(&mappings) {
        pixel_file
            .write_all(&sample.bytes)
            .expect("write bounded source pixel bytes");
        entries.push(serde_json::json!({
            "sourcePts": timestamp_json(mapping.source_pts),
            "hostPts": timestamp_json(mapping.host_pts),
            "captureRelativeMs": mapping.capture_relative_ms,
            "width": sample.width,
            "height": sample.height,
            "pixelFormat": format!("{:08x}", sample.pixel_format),
            "planar": sample.planar,
            "colorAttachments": {
                "primaries": sample.color.primaries.as_deref(),
                "transferFunction": sample.color.transfer_function.as_deref(),
                "ycbcrMatrix": sample.color.ycbcr_matrix.as_deref(),
                "cgColorSpace": sample.color.cg_color_space.as_deref(),
            },
            "bytesPerRow": sample.bytes_per_row,
            "dataSize": sample.data_size,
            "byteOffset": byte_offset,
            "byteLength": sample.bytes.len(),
        }));
        byte_offset = byte_offset
            .checked_add(sample.bytes.len())
            .expect("bounded observation byte offset");
    }
    pixel_file.sync_all().expect("sync source sample evidence");
    let metadata_path = fixture.own_file(fixture.root.join("sck-source-samples.json"));
    let first_profile = observations
        .first()
        .and_then(|sample| sample.color.cg_color_space_icc.as_deref());
    let shared_profile = first_profile.filter(|profile| {
        observations
            .iter()
            .all(|sample| sample.color.cg_color_space_icc.as_deref() == Some(*profile))
    });
    let profile_status = if shared_profile.is_some() {
        "shared-identical-icc-profile"
    } else if observations
        .iter()
        .any(|sample| sample.color.cg_color_space_icc.is_some())
    {
        "varying-or-incomplete-icc-profile"
    } else {
        "missing-or-unsupported-icc-profile"
    };
    let metadata = serde_json::json!({
        "schema": "apparent.test-sck-source-samples/3",
        "admission": timestamp_json(admission.timestamp()),
        "deliberateSetupDelayMs": setup_delay.as_millis(),
        "sourceSpanMs": source_span_ms,
        "pixelBytesFile": evidence_path.file_name().unwrap().to_string_lossy(),
        "sourceColorSpaceProfileStatus": profile_status,
        "sourceColorSpaceICC": shared_profile.map(|profile| serde_json::json!({
            "encoding": "base64",
            "byteLength": profile.len(),
            "data": base64_profile(profile),
        })),
        "samples": entries,
        "encodedOrigin": "unproven-until-AVAssetReader-correspondence",
    });
    let metadata_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&metadata_path)
        .expect("exclusive source sample metadata file");
    let metadata_bytes = serde_json::to_vec(&metadata).expect("serialize source sample metadata");
    assert!(
        metadata_bytes.len() <= 128 * 1024,
        "source metadata exceeds file-only observer bound"
    );
    let mut metadata_file = metadata_file;
    metadata_file
        .write_all(&metadata_bytes)
        .expect("write bounded source sample metadata");
    fixture.bind_file(&evidence_path);
    fixture.bind_file(&metadata_path);
    eprintln!(
        "SCK_SOURCE_CORRESPONDENCE_INPUT samples={} source_span_ms={} sample_bytes={} metadata={} pixels={} encoded_origin=unproven",
        observations.len(),
        source_span_ms,
        byte_offset,
        metadata_path.display(),
        evidence_path.display()
    );
    stream
}

fn finish_sck_output(mut stream: CaptureStream) {
    let stopped = stream.stop_checked();
    assert_eq!(stopped.stop_capture, StopOperation::Succeeded);
    assert_eq!(stopped.remove_recording_output, StopOperation::Succeeded);
    let deadline = Instant::now() + Duration::from_secs(10);
    while stream.recording_finalization() == RecordingFinalization::Pending {
        assert!(Instant::now() < deadline, "SCK stop did not finish output");
        thread::sleep(Duration::from_millis(20));
    }
    let outcome = stream.checked_stop_outcome();
    assert!(
        outcome.is_finalized(),
        "SCK checked stop did not prove output finalization: {outcome:?}"
    );
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
