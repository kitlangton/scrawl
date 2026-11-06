//! Retained-material derivative document preparation.
//!
//! This is deliberately not an authority or filesystem API. The caller must
//! authenticate the capture owner, hold the retained-material read lease, pin
//! the source descriptor, and supply sealed output identities from its native
//! sink. The provider remains responsible for durable job/receipt identity and
//! release exclusion.

use anyhow::{Result, ensure};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::os::unix::{fs::MetadataExt, io::AsRawFd};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialsKey {
    pub capture_id: String,
    pub materials_id: String,
    pub revision: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileIdentity {
    pub device: String,
    pub inode: String,
    pub size: u64,
    pub modified_ns: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SealedFile {
    pub relative_path: String,
    pub identity: FileIdentity,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct DecodedSlot {
    /// 1-based ordered request slot. Repeated requests remain separate slots.
    pub request_index: usize,
    pub requested_bundle_ms: u64,
    pub actual_video_ms: u64,
    pub clamped_to_end: bool,
    pub width: u32,
    pub height: u32,
    /// Created, synced and sealed by the caller-owned native sink.
    pub file: SealedFile,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload<'a> {
    schema: &'static str,
    materials: &'a MaterialsKey,
    job_id: &'a str,
    receipt_id: &'a str,
    root: &'a str,
    frames: Vec<Frame<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Frame<'a> {
    request_index: usize,
    requested_bundle_ms: u64,
    effective_video_ms: u64,
    actual_video_ms: u64,
    clamped_to_start: bool,
    clamped_to_end: bool,
    width: u32,
    height: u32,
    file: &'a SealedFile,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<'a> {
    schema: &'static str,
    materials: &'a MaterialsKey,
    job_id: &'a str,
    receipt_id: &'a str,
    root: &'a str,
    receipt_file: &'a SealedFile,
    payload: &'a Value,
}

/// Confirm the source descriptor still names the exact sealed video in the
/// already-validated detached materials payload. This check does not confer
/// owner or lease authority; the caller's active lease port does that.
pub fn check_pinned_material(
    video: &File,
    key: &MaterialsKey,
    payload_bytes: &[u8],
    expected: &FileIdentity,
    expected_root: &str,
    expected_lease_id: &str,
) -> Result<u64> {
    let revision = hex(&Sha256::digest(payload_bytes));
    ensure!(key.revision == revision, "materials revision mismatch");
    let payload: Value = serde_json::from_slice(payload_bytes)?;
    ensure!(payload["schema"] == "apparent.capture-materials-payload/1");
    ensure!(payload["identity"]["captureId"] == key.capture_id);
    ensure!(payload["identity"]["materialsId"] == key.materials_id);
    ensure!(payload["root"] == expected_root && payload["leaseId"] == expected_lease_id);
    ensure!(payload["safety"]["safeForProcessing"] == true);
    ensure!(normalized_absolute(expected_root) && id(expected_lease_id));
    let video_offset_ms = payload["clock"]["videoOffsetMs"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("invalid materials clock"))?;
    ensure!(video_offset_ms <= 9_007_199_254_740_991);
    let video_name = payload["video"]["file"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("invalid materials video"))?;
    let file_entry = payload["files"]
        .as_array()
        .and_then(|files| files.iter().find(|f| f["relativePath"] == video_name))
        .ok_or_else(|| anyhow::anyhow!("materials video seal missing"))?;
    let metadata = video.metadata()?;
    ensure!(metadata.is_file(), "pinned video is not a regular file");
    let actual = FileIdentity {
        device: metadata.dev().to_string(),
        inode: metadata.ino().to_string(),
        size: metadata.len(),
        modified_ns: (i128::from(metadata.mtime()) * 1_000_000_000
            + i128::from(metadata.mtime_nsec()))
        .to_string(),
    };
    ensure!(
        actual.device == expected.device
            && actual.inode == expected.inode
            && actual.size == expected.size
            && actual.modified_ns == expected.modified_ns,
        "pinned source identity changed"
    );
    ensure!(file_entry["identity"]["device"] == actual.device);
    ensure!(file_entry["identity"]["inode"] == actual.inode);
    ensure!(file_entry["identity"]["size"].as_u64() == Some(actual.size));
    ensure!(file_entry["identity"]["modifiedNs"] == actual.modified_ns);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut byte_offset = 0i64;
    loop {
        // SAFETY: pread reads into a live writable buffer from the caller's
        // pinned regular-file descriptor without changing its shared offset.
        let n = unsafe {
            libc::pread(
                video.as_raw_fd(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                byte_offset,
            )
        };
        ensure!(n >= 0, "cannot hash pinned video");
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n as usize]);
        byte_offset = byte_offset
            .checked_add(n as i64)
            .ok_or_else(|| anyhow::anyhow!("video too large"))?;
    }
    ensure!(
        hex(&hasher.finalize()) == file_entry["sha256"].as_str().unwrap_or_default(),
        "sealed video hash mismatch"
    );
    let after = video.metadata()?;
    ensure!(
        after.dev() == metadata.dev()
            && after.ino() == metadata.ino()
            && after.len() == metadata.len()
            && after.mtime() == metadata.mtime()
            && after.mtime_nsec() == metadata.mtime_nsec(),
        "pinned video changed while hashing"
    );
    Ok(video_offset_ms)
}

/// Serialize exact detached Media/2 payload and envelope bytes from already
/// decoded slots. PNG creation, durable output publication, job identity
/// reservation, and native receipt lifecycle are intentionally caller-owned.
pub fn payload_document(
    key: &MaterialsKey,
    job_id: &str,
    receipt_id: &str,
    root: &str,
    video_offset_ms: u64,
    requested_bundle_ms: &[u64],
    slots: &[DecodedSlot],
) -> Result<Vec<u8>> {
    ensure!(
        id(&key.capture_id) && id(&key.materials_id) && hash(&key.revision),
        "invalid materials key"
    );
    ensure!(id(job_id) && id(receipt_id), "invalid derivative identity");
    ensure!(
        std::path::Path::new(root).is_absolute()
            && std::path::Path::new(root).components().all(|c| matches!(
                c,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )),
        "derivative root must be normalized and absolute"
    );
    ensure!(!requested_bundle_ms.is_empty() && requested_bundle_ms.len() <= 48);
    ensure!(video_offset_ms <= 9_007_199_254_740_991);
    ensure!(
        requested_bundle_ms
            .iter()
            .all(|ms| *ms <= 9_007_199_254_740_991)
    );
    ensure!(slots.len() == requested_bundle_ms.len());
    let mut frames = Vec::with_capacity(slots.len());
    let mut pixels = 0u64;
    for (i, (slot, requested)) in slots.iter().zip(requested_bundle_ms).enumerate() {
        ensure!(
            slot.request_index == i + 1 && slot.requested_bundle_ms == *requested,
            "frame order/request mismatch"
        );
        ensure!(slot.width > 0 && slot.height > 0 && slot.width <= 4096 && slot.height <= 4096);
        ensure!(slot.actual_video_ms <= 9_007_199_254_740_991);
        ensure!(hash(&slot.file.sha256) && relative(&slot.file.relative_path));
        ensure!(valid_identity(&slot.file.identity));
        pixels = pixels
            .checked_add(u64::from(slot.width) * u64::from(slot.height))
            .ok_or_else(|| anyhow::anyhow!("pixel budget overflow"))?;
        ensure!(pixels <= 64_000_000, "pixel budget exceeded");
        let effective = requested.saturating_sub(video_offset_ms);
        frames.push(Frame {
            request_index: i + 1,
            requested_bundle_ms: *requested,
            effective_video_ms: effective,
            actual_video_ms: slot.actual_video_ms,
            clamped_to_start: *requested < video_offset_ms,
            clamped_to_end: slot.clamped_to_end,
            width: slot.width,
            height: slot.height,
            file: &slot.file,
        });
    }
    let payload = Payload {
        schema: "apparent.media-derivative-payload/1",
        materials: key,
        job_id,
        receipt_id,
        root,
        frames,
    };
    Ok(serde_json::to_vec(&payload)?)
}

/// Bind the detached payload bytes to their durable caller-sink identity.
pub fn envelope_document(
    key: &MaterialsKey,
    job_id: &str,
    receipt_id: &str,
    root: &str,
    payload_bytes: &[u8],
    receipt_file: &SealedFile,
) -> Result<Vec<u8>> {
    ensure!(
        id(&key.capture_id) && id(&key.materials_id) && hash(&key.revision),
        "invalid materials key"
    );
    ensure!(id(job_id) && id(receipt_id), "invalid derivative identity");
    ensure!(
        normalized_absolute(root),
        "derivative root must be normalized and absolute"
    );
    ensure!(relative(&receipt_file.relative_path) && hash(&receipt_file.sha256));
    ensure!(valid_identity(&receipt_file.identity));
    ensure!(receipt_file.identity.size == payload_bytes.len() as u64);
    ensure!(
        hex(&Sha256::digest(payload_bytes)) == receipt_file.sha256,
        "receipt digest mismatch"
    );
    let payload: Value = serde_json::from_slice(payload_bytes)?;
    ensure!(payload["schema"] == "apparent.media-derivative-payload/1");
    ensure!(payload["materials"]["captureId"] == key.capture_id);
    ensure!(payload["materials"]["materialsId"] == key.materials_id);
    ensure!(payload["materials"]["revision"] == key.revision);
    ensure!(
        payload["jobId"] == job_id && payload["receiptId"] == receipt_id && payload["root"] == root
    );
    let frame_paths: Vec<&str> = payload["frames"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("invalid frames"))?
        .iter()
        .filter_map(|f| f["file"]["relativePath"].as_str())
        .collect();
    ensure!(frame_paths.len() == payload["frames"].as_array().map_or(0, Vec::len));
    ensure!(frame_paths.iter().all(|p| relative(p)));
    ensure!(!frame_paths.contains(&receipt_file.relative_path.as_str()));
    for (i, path) in frame_paths.iter().enumerate() {
        ensure!(
            !frame_paths[..i].contains(path),
            "duplicate frame output path"
        );
    }
    let envelope = Envelope {
        schema: "apparent.media-derivative/1",
        materials: key,
        job_id,
        receipt_id,
        root,
        receipt_file,
        payload: &payload,
    };
    Ok(serde_json::to_vec(&envelope)?)
}

/// Output adapter owned by the authenticated provider. Implementations must
/// exclusively create each relative path beneath the job's pinned output root,
/// sync it before returning, and report its post-write identity and digest.
#[cfg(all(target_os = "linux", feature = "linux-media"))]
pub trait DerivativeOutputSink {
    fn write_new_synced(&mut self, relative_path: &str, bytes: &[u8]) -> Result<SealedFile>;
}

/// Decode retained frames from an already-open descriptor using Core's pinned
/// Linux reader, encode valid PNGs, and prepare SDK-compatible detached bytes.
/// Owner authentication, lease acquisition, durable job reservation and sink
/// safety are explicit caller responsibilities; this routine does not claim
/// those authorities or alter capture admission. `checkpoint` must reject a
/// cancelled or expired job. It runs before expensive phases and every write.
/// The reused reader may block for up to ten seconds inside one native read;
/// callers MUST run this port in their existing supervised native job process,
/// whose hard timeout/cancel reaps it. Never run it on a UI/RPC service thread.
#[cfg(all(target_os = "linux", feature = "linux-media"))]
pub fn derive_linux(
    video: &File,
    key: &MaterialsKey,
    payload_bytes: &[u8],
    expected_video_identity: &FileIdentity,
    expected_root: &str,
    expected_lease_id: &str,
    job_id: &str,
    receipt_id: &str,
    root: &str,
    requested_bundle_ms: &[u64],
    max_edge: u32,
    sink: &mut dyn DerivativeOutputSink,
    checkpoint: &mut dyn FnMut() -> Result<()>,
) -> Result<(Vec<u8>, Vec<u8>)> {
    checkpoint()?;
    let video_offset_ms = check_pinned_material(
        video,
        key,
        payload_bytes,
        expected_video_identity,
        expected_root,
        expected_lease_id,
    )?;
    ensure!((1..=4096).contains(&max_edge));
    ensure!(!requested_bundle_ms.is_empty() && requested_bundle_ms.len() <= 48);
    ensure!(
        requested_bundle_ms
            .iter()
            .all(|ms| *ms <= 9_007_199_254_740_991)
    );
    checkpoint()?;
    let reader = crate::linux::media::PinnedVideoReader::open(video)?;
    let duration_ms = reader.duration_ms();
    let mut slots = Vec::with_capacity(requested_bundle_ms.len());
    let mut total_pixels = 0u64;
    for (i, requested) in requested_bundle_ms.iter().copied().enumerate() {
        checkpoint()?;
        let effective = requested.saturating_sub(video_offset_ms);
        let mut frame = reader.frame(effective, 16_384, 256 * 1024 * 1024)?;
        checkpoint()?;
        let (width, height, rgba) =
            crate::linux::media::shrink_to_edge(frame.width, frame.height, frame.rgba, max_edge);
        frame.width = width;
        frame.height = height;
        frame.rgba = rgba;
        total_pixels = total_pixels
            .checked_add(u64::from(frame.width) * u64::from(frame.height))
            .ok_or_else(|| anyhow::anyhow!("frame pixel budget overflow"))?;
        ensure!(total_pixels <= 64_000_000, "frame pixel budget exceeded");
        checkpoint()?;
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, frame.width, frame.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(&frame.rgba)?;
            writer.finish()?;
        }
        let path = format!("frames/frame-{:03}.png", i + 1);
        checkpoint()?;
        let sealed = sink.write_new_synced(&path, &png_bytes)?;
        verify_output(&sealed, &path, &png_bytes)?;
        slots.push(DecodedSlot {
            request_index: i + 1,
            requested_bundle_ms: requested,
            actual_video_ms: frame.t_ms,
            clamped_to_end: effective >= duration_ms,
            width: frame.width,
            height: frame.height,
            file: sealed,
        });
    }
    let detached = payload_document(
        key,
        job_id,
        receipt_id,
        root,
        video_offset_ms,
        requested_bundle_ms,
        &slots,
    )?;
    checkpoint()?;
    let receipt = sink.write_new_synced("receipt.json", &detached)?;
    verify_output(&receipt, "receipt.json", &detached)?;
    let envelope = envelope_document(key, job_id, receipt_id, root, &detached, &receipt)?;
    Ok((detached, envelope))
}

#[cfg(all(target_os = "linux", feature = "linux-media"))]
fn verify_output(sealed: &SealedFile, expected_path: &str, bytes: &[u8]) -> Result<()> {
    ensure!(
        sealed.relative_path == expected_path,
        "output sink returned another path"
    );
    ensure!(
        sealed.identity.size == bytes.len() as u64,
        "output sink size mismatch"
    );
    ensure!(
        hex(&Sha256::digest(bytes)) == sealed.sha256,
        "output sink digest mismatch"
    );
    Ok(())
}

fn id(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn relative(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 4096
        && !s.starts_with('/')
        && !s.contains('\\')
        && s.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}
fn normalized_absolute(s: &str) -> bool {
    s.len() <= 4096
        && std::path::Path::new(s).is_absolute()
        && std::path::Path::new(s).components().all(|c| {
            matches!(
                c,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
}
fn valid_identity(identity: &FileIdentity) -> bool {
    !identity.device.is_empty()
        && identity.device.bytes().all(|b| b.is_ascii_digit())
        && !identity.inode.is_empty()
        && identity.inode.bytes().all(|b| b.is_ascii_digit())
        && identity.size <= 9_007_199_254_740_991
        && !identity.modified_ns.is_empty()
        && identity.modified_ns.bytes().all(|b| b.is_ascii_digit())
}
fn hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(H[(b >> 4) as usize] as char);
        out.push(H[(b & 15) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;

    #[cfg(all(target_os = "linux", feature = "linux-media"))]
    struct TempDir(std::path::PathBuf);

    #[cfg(all(target_os = "linux", feature = "linux-media"))]
    impl TempDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!("{label}-{}", crate::new_id().unwrap()));
            std::fs::create_dir(&path).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
    }

    #[cfg(all(target_os = "linux", feature = "linux-media"))]
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(all(target_os = "linux", feature = "linux-media"))]
    struct FsSink(std::path::PathBuf);

    #[cfg(all(target_os = "linux", feature = "linux-media"))]
    impl DerivativeOutputSink for FsSink {
        fn write_new_synced(&mut self, relative_path: &str, bytes: &[u8]) -> Result<SealedFile> {
            use std::io::Write;
            use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
            let path = self.0.join(relative_path);
            let parent = path.parent().unwrap();
            std::fs::create_dir_all(parent)?;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .read(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            let metadata = file.metadata()?;
            Ok(SealedFile {
                relative_path: relative_path.into(),
                identity: FileIdentity {
                    device: metadata.dev().to_string(),
                    inode: metadata.ino().to_string(),
                    size: metadata.len(),
                    modified_ns: (i128::from(metadata.mtime()) * 1_000_000_000
                        + i128::from(metadata.mtime_nsec()))
                    .to_string(),
                },
                sha256: hex(&Sha256::digest(bytes)),
            })
        }
    }

    fn seal(path: &str) -> SealedFile {
        SealedFile {
            relative_path: path.into(),
            identity: FileIdentity {
                device: "1".into(),
                inode: "2".into(),
                size: 3,
                modified_ns: "4".into(),
            },
            sha256: "a".repeat(64),
        }
    }

    #[test]
    fn detached_documents_bind_clock_order_duplicates_and_seals() {
        let key = MaterialsKey {
            capture_id: "a".repeat(32),
            materials_id: "b".repeat(32),
            revision: "c".repeat(64),
        };
        let requests = [50, 150, 150];
        let slots = requests
            .iter()
            .enumerate()
            .map(|(i, ms)| DecodedSlot {
                request_index: i + 1,
                requested_bundle_ms: *ms,
                actual_video_ms: [0, 40, 40][i],
                clamped_to_end: i == 2,
                width: 2,
                height: 1,
                file: seal(&format!("frames/frame-{:03}.png", i + 1)),
            })
            .collect::<Vec<_>>();
        let payload_bytes = payload_document(
            &key,
            &"d".repeat(32),
            &"e".repeat(32),
            "/out",
            100,
            &requests,
            &slots,
        )
        .unwrap();
        let mut receipt = seal("receipt.json");
        receipt.identity.size = payload_bytes.len() as u64;
        receipt.sha256 = hex(&Sha256::digest(&payload_bytes));
        let envelope_bytes = envelope_document(
            &key,
            &"d".repeat(32),
            &"e".repeat(32),
            "/out",
            &payload_bytes,
            &receipt,
        )
        .unwrap();
        let payload: Value = serde_json::from_slice(&payload_bytes).unwrap();
        assert_eq!(payload["frames"][0]["effectiveVideoMs"], 0);
        assert_eq!(payload["frames"][0]["clampedToStart"], true);
        assert_eq!(payload["frames"][1]["effectiveVideoMs"], 50);
        assert_eq!(payload["frames"][1]["actualVideoMs"], 40);
        assert_eq!(payload["frames"][2]["requestIndex"], 3);
        assert_eq!(
            payload["frames"][1]["requestedBundleMs"],
            payload["frames"][2]["requestedBundleMs"]
        );
        let envelope: Value = serde_json::from_slice(&envelope_bytes).unwrap();
        assert_eq!(envelope["payload"], payload);
        assert_eq!(envelope["receiptFile"]["sha256"], receipt.sha256);
    }

    #[test]
    fn rejects_slot_reordering_and_receipt_path_alias() {
        let key = MaterialsKey {
            capture_id: "a".repeat(32),
            materials_id: "b".repeat(32),
            revision: "c".repeat(64),
        };
        let requests = [10, 20];
        let mut slots = requests
            .iter()
            .enumerate()
            .map(|(i, ms)| DecodedSlot {
                request_index: i + 1,
                requested_bundle_ms: *ms,
                actual_video_ms: *ms,
                clamped_to_end: false,
                width: 1,
                height: 1,
                file: seal(&format!("frames/frame-{:03}.png", i + 1)),
            })
            .collect::<Vec<_>>();
        slots.swap(0, 1);
        assert!(
            payload_document(
                &key,
                &"d".repeat(32),
                &"e".repeat(32),
                "/out",
                0,
                &requests,
                &slots
            )
            .is_err()
        );
        slots.swap(0, 1);
        let bytes = payload_document(
            &key,
            &"d".repeat(32),
            &"e".repeat(32),
            "/out",
            0,
            &requests,
            &slots,
        )
        .unwrap();
        let mut receipt = seal("frames/frame-001.png");
        receipt.identity.size = bytes.len() as u64;
        receipt.sha256 = hex(&Sha256::digest(&bytes));
        assert!(
            envelope_document(
                &key,
                &"d".repeat(32),
                &"e".repeat(32),
                "/out",
                &bytes,
                &receipt
            )
            .is_err()
        );
    }

    #[test]
    fn pinned_material_checks_revision_identity_lease_and_video_digest() {
        let path = std::env::temp_dir().join(format!("materials-media-{}", std::process::id()));
        let mut video = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        video.write_all(b"synthetic retained movie bytes").unwrap();
        video.sync_all().unwrap();
        let metadata = video.metadata().unwrap();
        let expected = FileIdentity {
            device: metadata.dev().to_string(),
            inode: metadata.ino().to_string(),
            size: metadata.len(),
            modified_ns: (i128::from(metadata.mtime()) * 1_000_000_000
                + i128::from(metadata.mtime_nsec()))
            .to_string(),
        };
        let digest = hex(&Sha256::digest(b"synthetic retained movie bytes"));
        let make_payload = |video_hash: &str| {
            serde_json::to_vec(&serde_json::json!({
                "schema":"apparent.capture-materials-payload/1",
                "identity":{"captureId":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","materialsId":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
                "root":"/retained/capture","leaseId":"cccccccccccccccccccccccccccccccc",
                "clock":{"videoOffsetMs":37},"video":{"file":"recording.mp4"},
                "files":[{"relativePath":"recording.mp4","identity":{
                    "device":expected.device,"inode":expected.inode,"size":expected.size,"modifiedNs":expected.modified_ns
                },"sha256":video_hash}],"safety":{"safeForProcessing":true}
            })).unwrap()
        };
        let bytes = make_payload(&digest);
        let key = MaterialsKey {
            capture_id: "a".repeat(32),
            materials_id: "b".repeat(32),
            revision: hex(&Sha256::digest(&bytes)),
        };
        assert_eq!(
            check_pinned_material(
                &video,
                &key,
                &bytes,
                &expected,
                "/retained/capture",
                &"c".repeat(32)
            )
            .unwrap(),
            37
        );
        let wrong_hash_bytes = make_payload(&"d".repeat(64));
        let wrong_hash_key = MaterialsKey {
            revision: hex(&Sha256::digest(&wrong_hash_bytes)),
            ..key.clone()
        };
        assert!(
            check_pinned_material(
                &video,
                &wrong_hash_key,
                &wrong_hash_bytes,
                &expected,
                "/retained/capture",
                &"c".repeat(32)
            )
            .is_err()
        );
        let wrong_revision = MaterialsKey {
            revision: "e".repeat(64),
            ..key.clone()
        };
        assert!(
            check_pinned_material(
                &video,
                &wrong_revision,
                &bytes,
                &expected,
                "/retained/capture",
                &"c".repeat(32)
            )
            .is_err()
        );
        let mut wrong_identity = expected.clone();
        wrong_identity.inode = "0".into();
        assert!(
            check_pinned_material(
                &video,
                &key,
                &bytes,
                &wrong_identity,
                "/retained/capture",
                &"c".repeat(32)
            )
            .is_err()
        );
        assert!(
            check_pinned_material(
                &video,
                &key,
                &bytes,
                &expected,
                "/retained/capture",
                &"f".repeat(32)
            )
            .is_err()
        );
        drop(video);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_derivative_pixel_and_frame_count_budgets() {
        let key = MaterialsKey {
            capture_id: "a".repeat(32),
            materials_id: "b".repeat(32),
            revision: "c".repeat(64),
        };
        let times = vec![0; 48];
        let huge = times
            .iter()
            .enumerate()
            .map(|(i, ms)| DecodedSlot {
                request_index: i + 1,
                requested_bundle_ms: *ms,
                actual_video_ms: 0,
                clamped_to_end: false,
                width: 4096,
                height: 4096,
                file: seal(&format!("frames/frame-{:03}.png", i + 1)),
            })
            .collect::<Vec<_>>();
        assert!(
            payload_document(
                &key,
                &"d".repeat(32),
                &"e".repeat(32),
                "/out",
                0,
                &times,
                &huge
            )
            .is_err()
        );
        let too_many = vec![0; 49];
        assert!(
            payload_document(
                &key,
                &"d".repeat(32),
                &"e".repeat(32),
                "/out",
                0,
                &too_many,
                &[]
            )
            .is_err()
        );
    }

    #[cfg(all(target_os = "linux", feature = "linux-media"))]
    #[test]
    fn pinned_linux_decode_writes_real_pngs_and_ordered_detached_receipt() {
        use std::io::Read;
        let source_root = TempDir::new("materials-media-source");
        let output_root = TempDir::new("materials-media-output");
        let movie_path = source_root.0.join("recording.mp4");
        crate::linux::encode_synthetic(&movie_path).unwrap();
        let video = File::open(&movie_path).unwrap();
        let metadata = video.metadata().unwrap();
        let expected_identity = FileIdentity {
            device: metadata.dev().to_string(),
            inode: metadata.ino().to_string(),
            size: metadata.len(),
            modified_ns: (i128::from(metadata.mtime()) * 1_000_000_000
                + i128::from(metadata.mtime_nsec()))
            .to_string(),
        };
        let movie_bytes = std::fs::read(&movie_path).unwrap();
        let material_payload = serde_json::to_vec(&serde_json::json!({
            "schema":"apparent.capture-materials-payload/1",
            "identity":{"captureId":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","materialsId":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
            "bundleId":"synthetic","root":source_root.0,"leaseId":"cccccccccccccccccccccccccccccccc",
            "clock":{"videoOffsetMs":100},"video":{"file":"recording.mp4"},
            "files":[{"relativePath":"recording.mp4","identity":{
                "device":expected_identity.device,"inode":expected_identity.inode,"size":expected_identity.size,"modifiedNs":expected_identity.modified_ns
            },"sha256":hex(&Sha256::digest(&movie_bytes))}],
            "safety":{"safeForProcessing":true}
        })).unwrap();
        let key = MaterialsKey {
            capture_id: "a".repeat(32),
            materials_id: "b".repeat(32),
            revision: hex(&Sha256::digest(&material_payload)),
        };
        let output_path = output_root.0.canonicalize().unwrap();
        let output_root_str = output_path.to_str().unwrap();
        let lease_id = "c".repeat(32);
        let requests = [0, 300, 300, 2_000];
        let mut sink = FsSink(output_path.clone());
        let (detached, envelope_bytes) = derive_linux(
            &video,
            &key,
            &material_payload,
            &expected_identity,
            source_root.0.to_str().unwrap(),
            &lease_id,
            &"d".repeat(32),
            &"e".repeat(32),
            output_root_str,
            &requests,
            80,
            &mut sink,
            &mut || Ok(()),
        )
        .unwrap();
        // A cancellation/deadline checkpoint after the first sealed frame must
        // prevent another frame or detached receipt from becoming observable.
        let stopped_output = TempDir::new("materials-media-stopped");
        let writes = std::rc::Rc::new(std::cell::Cell::new(0usize));
        struct StoppingSink(FsSink, std::rc::Rc<std::cell::Cell<usize>>);
        impl DerivativeOutputSink for StoppingSink {
            fn write_new_synced(&mut self, path: &str, bytes: &[u8]) -> Result<SealedFile> {
                let file = self.0.write_new_synced(path, bytes)?;
                self.1.set(self.1.get() + 1);
                Ok(file)
            }
        }
        let mut stopped_sink = StoppingSink(FsSink(stopped_output.0.clone()), writes.clone());
        let error = derive_linux(
            &video,
            &key,
            &material_payload,
            &expected_identity,
            source_root.0.to_str().unwrap(),
            &lease_id,
            &"f".repeat(32),
            &"1".repeat(32),
            stopped_output.0.to_str().unwrap(),
            &requests,
            80,
            &mut stopped_sink,
            &mut || {
                ensure!(writes.get() == 0, "cancelled or deadline exceeded");
                Ok(())
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("cancelled or deadline exceeded"));
        assert_eq!(writes.get(), 1);
        assert!(!stopped_output.0.join("frames/frame-002.png").exists());
        assert!(!stopped_output.0.join("receipt.json").exists());
        let payload: Value = serde_json::from_slice(&detached).unwrap();
        let envelope: Value = serde_json::from_slice(&envelope_bytes).unwrap();
        assert_eq!(envelope["payload"], payload);
        assert_eq!(payload["frames"].as_array().unwrap().len(), 4);
        assert_eq!(payload["frames"][0]["width"], 80);
        assert_eq!(payload["frames"][0]["height"], 45);
        assert_eq!(payload["frames"][0]["requestedBundleMs"], 0);
        assert_eq!(payload["frames"][0]["effectiveVideoMs"], 0);
        assert_eq!(payload["frames"][0]["clampedToStart"], true);
        assert_eq!(payload["frames"][1]["requestedBundleMs"], 300);
        assert_eq!(payload["frames"][1]["effectiveVideoMs"], 200);
        assert_eq!(payload["frames"][2]["requestIndex"], 3);
        assert_eq!(payload["frames"][3]["clampedToEnd"], true);
        assert_ne!(payload["frames"][1]["actualVideoMs"], 2000);
        let frame1 = std::fs::read(output_path.join("frames/frame-001.png")).unwrap();
        let frame2 = std::fs::read(output_path.join("frames/frame-002.png")).unwrap();
        let frame3 = std::fs::read(output_path.join("frames/frame-003.png")).unwrap();
        assert_eq!(&frame1[..8], b"\x89PNG\r\n\x1a\n");
        assert_ne!(frame1, frame2);
        assert_eq!(frame2, frame3);
        let mut decoder = png::Decoder::new(std::io::Cursor::new(frame2));
        decoder.set_limits(png::Limits {
            bytes: 256 * 1024 * 1024,
        });
        let mut reader = decoder.read_info().unwrap();
        assert_eq!((reader.info().width, reader.info().height), (80, 45));
        let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut rgba).unwrap();
        assert!(
            rgba.chunks_exact(4)
                .any(|pixel| pixel[0] != pixel[1] || pixel[1] != pixel[2])
        );
        let mut file = File::open(output_path.join("receipt.json")).unwrap();
        let mut receipt_bytes = Vec::new();
        file.read_to_end(&mut receipt_bytes).unwrap();
        assert_eq!(receipt_bytes, detached);
        assert_eq!(
            envelope["receiptFile"]["sha256"],
            hex(&Sha256::digest(&detached))
        );
    }
}
