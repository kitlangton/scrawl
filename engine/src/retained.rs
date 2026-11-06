//! Authenticated, leased access to capture/2 retained material files.
//!
//! This module is included by the native capture and media binaries. Callers
//! must supply the original capture owner secret and material identity before
//! they can receive any material paths or descriptors.

use apparent_capture_engine::{BundleLease, LeaseKind, Runtime, SecureStore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read},
    os::fd::AsRawFd,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};

const JOURNAL_LIMIT: u64 = 16 * 1024 * 1024;
const MATERIALS_LIMIT: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct MaterialKey {
    pub(crate) capture_id: String,
    pub(crate) materials_id: String,
    pub(crate) revision: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct MaterialFileIdentity {
    pub(crate) device: String,
    pub(crate) inode: String,
    pub(crate) size: u64,
    pub(crate) modified_ns: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct MaterialFileSeal {
    pub(crate) relative_path: String,
    pub(crate) identity: MaterialFileIdentity,
    pub(crate) sha256: String,
}

pub(crate) struct AuthenticatedJournal {
    pub(crate) bytes: Vec<u8>,
    pub(crate) document: Value,
}

/// Read and authenticate the owner record before any caller can inspect its
/// material identity or discover files beneath the retained-material root.
pub(crate) fn read_authenticated_journal(
    directory: &Path,
    capture_id: &str,
    owner_token: &str,
) -> Result<AuthenticatedJournal, &'static str> {
    if !valid_id(capture_id) {
        return Err("malformed_request");
    }
    let store = SecureStore::open(directory).map_err(|_| "source_mismatch")?;
    let file = match store.file(&format!("{capture_id}.json"), false) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err("not_found"),
        Err(_) => return Err("source_mismatch"),
    };
    let bytes = read_bounded(&file, JOURNAL_LIMIT).map_err(|_| "source_mismatch")?;
    let document: Value = serde_json::from_slice(&bytes).map_err(|_| "source_mismatch")?;
    if document["capture_id"].as_str() != Some(capture_id)
        || document["owner_token"].as_str().is_none_or(|stored| {
            stored.len() != 64
                || !apparent_capture_engine::identity_equal_secret(stored, owner_token)
        })
    {
        return Err("permission_denied");
    }
    Ok(AuthenticatedJournal { bytes, document })
}

#[allow(dead_code)] // Shared source contract; media/2 consumes the pinned bytes and handles.
pub(crate) struct RetainedMaterialSource {
    pub(crate) capture_id: String,
    pub(crate) materials: MaterialKey,
    pub(crate) lease_id: String,
    pub(crate) root_path: PathBuf,
    pub(crate) root_device: u64,
    pub(crate) root_inode: u64,
    pub(crate) root: SecureStore,
    pub(crate) root_file: File,
    pub(crate) materials_manifest: Value,
    /// Canonical JSON bytes for the manifest held by the authenticated capture journal.
    pub(crate) manifest_bytes: Vec<u8>,
    pub(crate) payload: Value,
    /// Exact bytes read from the sealed `materials.json` payload file.
    pub(crate) payload_bytes: Vec<u8>,
    pub(crate) payload_file: File,
    pub(crate) video: MaterialFileSeal,
    pub(crate) video_file: File,
    pub(crate) files: Vec<MaterialFileSeal>,
    pub(crate) journal_bytes: Vec<u8>,
    binding: JournalBinding,
    _read_lease: BundleLease,
}

impl RetainedMaterialSource {
    pub(crate) fn validate_current_journal(&self, document: &Value) -> Result<(), &'static str> {
        let expected = MaterialKey {
            capture_id: self.capture_id.clone(),
            materials_id: self.materials.materials_id.clone(),
            revision: self.materials.revision.clone(),
        };
        let current = binding_from_document(
            document,
            self.root_path
                .parent()
                .and_then(Path::parent)
                .ok_or("source_mismatch")?,
            &expected,
            &self.lease_id,
        )?;
        if !same_binding(&self.binding, &current) {
            return Err("source_mismatch");
        }
        Ok(())
    }

    pub(crate) fn open_material_file(&self, relative_path: &str) -> Result<File, &'static str> {
        let seal = self
            .files
            .iter()
            .find(|file| file.relative_path == relative_path)
            .ok_or("invalid_materials")?;
        self.root.check().map_err(|_| "source_mismatch")?;
        let file = if *seal == self.video {
            self.video_file.try_clone().map_err(|_| "source_mismatch")?
        } else {
            open_sealed(&self.root_path, &seal.relative_path)?
        };
        self.root.check().map_err(|_| "source_mismatch")?;
        validate_open_identity(&file, &seal.identity)?;
        if hash_file(&file).map_err(|_| "source_mismatch")? != seal.sha256 {
            return Err("source_mismatch");
        }
        Ok(file)
    }
}

/// Authenticate the owner, acquire the read lease, recheck the durable source
/// binding/release state, then return pinned handles and the exact payload bytes.
/// This function never acquires the capture-owner permit: callers may already
/// hold it while authorizing their operation.
pub(crate) fn pin_retained_materials(
    runtime: &Runtime,
    directory: &Path,
    capture_id: &str,
    owner_token: &str,
    expected: &MaterialKey,
    expected_lease_id: &str,
) -> Result<RetainedMaterialSource, &'static str> {
    let first = read_authenticated_journal(directory, capture_id, owner_token)?;
    let first_binding =
        binding_from_document(&first.document, directory, expected, expected_lease_id)?;
    let read_lease = runtime
        .material_lease(&expected.materials_id, LeaseKind::Read)
        .map_err(|_| "internal")?
        .ok_or("lease_busy")?;
    read_lease.check().map_err(|_| "lease_busy")?;

    // Release writes its intent before taking the exclusive maintenance lease.
    // Re-read after the shared lease is acquired so an earlier release intent or
    // rebound journal can never be adopted; later release must wait for this guard.
    let current = recheck_after_lease(
        directory,
        capture_id,
        owner_token,
        expected,
        expected_lease_id,
        &first_binding,
        &read_lease,
    )?;
    let current_binding =
        binding_from_document(&current.document, directory, expected, expected_lease_id)?;

    let root = SecureStore::open(&current_binding.root_path).map_err(|_| "source_mismatch")?;
    let root_metadata = root.directory_metadata().map_err(|_| "source_mismatch")?;
    if root_metadata.dev() != current_binding.root_device
        || root_metadata.ino() != current_binding.root_inode
        || root_metadata.uid() != unsafe { libc::geteuid() }
        || root_metadata.mode() & 0o777 != 0o700
    {
        return Err("source_mismatch");
    }

    let manifest = current.document["materials"].clone();
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|_| "invalid_materials")?;
    let payload_seal: MaterialFileSeal =
        serde_json::from_value(manifest["payloadFile"].clone()).map_err(|_| "invalid_materials")?;
    validate_seal(&payload_seal)?;
    if payload_seal.relative_path != "materials.json"
        || payload_seal.sha256 != expected.revision
        || Some(&manifest["payloadFile"]["identity"]) != current.document.get("release_payload")
    {
        return Err("invalid_materials");
    }
    let payload_file = root
        .file("materials.json", false)
        .map_err(|_| "source_mismatch")?;
    validate_open_identity(&payload_file, &payload_seal.identity)?;
    let payload_bytes =
        read_bounded(&payload_file, MATERIALS_LIMIT).map_err(|_| "source_mismatch")?;
    if sha(&payload_bytes) != expected.revision
        || current.document["payload_digest"].as_str() != Some(expected.revision.as_str())
    {
        return Err("source_mismatch");
    }
    let payload: Value = serde_json::from_slice(&payload_bytes).map_err(|_| "invalid_materials")?;
    if payload["schema"].as_str() != Some("apparent.capture-materials-payload/1")
        || payload["identity"]["captureId"].as_str() != Some(capture_id)
        || payload["identity"]["materialsId"].as_str() != Some(expected.materials_id.as_str())
        || payload["root"].as_str()
            != Some(
                current_binding
                    .root_path
                    .to_str()
                    .ok_or("source_mismatch")?,
            )
        || payload["leaseId"].as_str() != Some(expected_lease_id)
        || payload["files"].as_array().is_none()
    {
        return Err("invalid_materials");
    }
    let files: Vec<MaterialFileSeal> = payload["files"]
        .as_array()
        .ok_or("invalid_materials")?
        .iter()
        .cloned()
        .map(|value| serde_json::from_value(value).map_err(|_| "invalid_materials"))
        .collect::<Result<_, _>>()?;
    if files.is_empty() || files.len() > 4096 {
        return Err("invalid_materials");
    }
    for file in &files {
        validate_seal(file)?;
    }
    let video = files
        .iter()
        .find(|file| file.relative_path == "recording.mp4")
        .cloned()
        .ok_or("invalid_materials")?;
    let release_movie: MaterialFileIdentity =
        serde_json::from_value(current.document["release_movie"].clone())
            .map_err(|_| "invalid_materials")?;
    if video.identity != release_movie || payload["video"]["file"].as_str() != Some("recording.mp4")
    {
        return Err("invalid_materials");
    }
    let video_file = root
        .file("recording.mp4", false)
        .map_err(|_| "source_mismatch")?;
    validate_open_identity(&video_file, &video.identity)?;
    if hash_file(&video_file).map_err(|_| "source_mismatch")? != video.sha256 {
        return Err("source_mismatch");
    }
    read_lease.check().map_err(|_| "lease_busy")?;
    let root_file = root.directory_file().map_err(|_| "source_mismatch")?;

    Ok(RetainedMaterialSource {
        capture_id: capture_id.to_owned(),
        materials: expected.clone(),
        lease_id: expected_lease_id.to_owned(),
        root_path: current_binding.root_path.clone(),
        root_device: current_binding.root_device,
        root_inode: current_binding.root_inode,
        root,
        root_file,
        materials_manifest: manifest,
        manifest_bytes,
        payload,
        payload_bytes,
        payload_file,
        video,
        video_file,
        files,
        journal_bytes: current.bytes,
        binding: current_binding,
        _read_lease: read_lease,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct JournalBinding {
    root_path: PathBuf,
    root_device: u64,
    root_inode: u64,
    manifest: Value,
    release_movie: Value,
    release_payload: Value,
    payload_digest: String,
    materials_id: String,
    lease_id: String,
}

fn binding_from_document(
    document: &Value,
    directory: &Path,
    expected: &MaterialKey,
    expected_lease_id: &str,
) -> Result<JournalBinding, &'static str> {
    let materials = &document["materials"];
    let manifest_key: MaterialKey =
        serde_json::from_value(materials["key"].clone()).map_err(|_| "invalid_materials")?;
    let capture_id = expected.capture_id.as_str();
    let root_path = PathBuf::from(document["root"].as_str().ok_or("invalid_materials")?);
    let expected_root = directory.join("materials").join(capture_id);
    let release_phase_absent = document["release_phase"].is_null();
    let state_ready = document["state"].as_str() == Some("materials-ready");
    let root_device = document["root_device"]
        .as_u64()
        .ok_or("invalid_materials")?;
    let root_inode = document["root_inode"].as_u64().ok_or("invalid_materials")?;
    let payload_digest = document["payload_digest"]
        .as_str()
        .ok_or("invalid_materials")?
        .to_owned();
    let materials_id = document["materials_id"]
        .as_str()
        .ok_or("invalid_materials")?
        .to_owned();
    let lease_id = document["lease_id"]
        .as_str()
        .ok_or("invalid_materials")?
        .to_owned();
    let release_movie = document["release_movie"].clone();
    let release_payload = document["release_payload"].clone();
    if !release_phase_absent
        || !state_ready
        || root_path != expected_root
        || document["capture_id"].as_str() != Some(capture_id)
        || materials_id != expected.materials_id
        || lease_id != expected_lease_id
        || payload_digest != expected.revision
        || manifest_key != *expected
        || materials["schema"].as_str() != Some("apparent.capture-materials/1")
        || materials["root"].as_str() != root_path.to_str()
        || materials["leaseId"].as_str() != Some(expected_lease_id)
        || materials["payloadFile"]["sha256"].as_str() != Some(expected.revision.as_str())
        || materials["payloadFile"]["relativePath"].as_str() != Some("materials.json")
        || materials["payloadFile"]["identity"] != release_payload
        || release_movie.is_null()
        || release_payload.is_null()
    {
        return Err("source_mismatch");
    }
    Ok(JournalBinding {
        root_path,
        root_device,
        root_inode,
        manifest: materials.clone(),
        release_movie,
        release_payload,
        payload_digest,
        materials_id,
        lease_id,
    })
}

fn same_binding(left: &JournalBinding, right: &JournalBinding) -> bool {
    left == right
}

fn recheck_after_lease(
    directory: &Path,
    capture_id: &str,
    owner_token: &str,
    expected: &MaterialKey,
    expected_lease_id: &str,
    first: &JournalBinding,
    lease: &BundleLease,
) -> Result<AuthenticatedJournal, &'static str> {
    lease.check().map_err(|_| "lease_busy")?;
    let current = read_authenticated_journal(directory, capture_id, owner_token)?;
    let binding = binding_from_document(&current.document, directory, expected, expected_lease_id)?;
    if !same_binding(first, &binding) {
        return Err("source_mismatch");
    }
    lease.check().map_err(|_| "lease_busy")?;
    Ok(current)
}

pub(crate) fn validate_open_identity(
    file: &File,
    expected: &MaterialFileIdentity,
) -> Result<(), &'static str> {
    let metadata = file.metadata().map_err(|_| "source_mismatch")?;
    let modified_ns =
        (metadata.mtime() as i128 * 1_000_000_000 + metadata.mtime_nsec() as i128).to_string();
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
        || metadata.dev().to_string() != expected.device
        || metadata.ino().to_string() != expected.inode
        || metadata.len() != expected.size
        || modified_ns != expected.modified_ns
    {
        return Err("source_mismatch");
    }
    Ok(())
}

pub(crate) fn open_sealed(root: &Path, relative_path: &str) -> Result<File, &'static str> {
    if !valid_relative(relative_path) {
        return Err("invalid_publication");
    }
    let mut path = root.to_owned();
    let components: Vec<_> = Path::new(relative_path).components().collect();
    for component in &components[..components.len().saturating_sub(1)] {
        let Component::Normal(part) = component else {
            return Err("invalid_publication");
        };
        path.push(part);
        SecureStore::open(&path).map_err(|_| "source_mismatch")?;
    }
    let Component::Normal(leaf) = components.last().ok_or("invalid_publication")? else {
        return Err("invalid_publication");
    };
    let parent = SecureStore::open(&path).map_err(|_| "source_mismatch")?;
    let file = parent
        .file(leaf.to_str().ok_or("invalid_publication")?, false)
        .map_err(|_| "source_mismatch")?;
    Ok(file)
}

pub(crate) fn read_bounded(file: &File, limit: u64) -> Result<Vec<u8>, &'static str> {
    let len = file.metadata().map_err(|_| "source_mismatch")?.len();
    if len == 0 || len > limit {
        return Err("invalid_publication");
    }
    let mut bytes = Vec::with_capacity(len as usize);
    file.try_clone()
        .map_err(|_| "source_mismatch")?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "source_mismatch")?;
    if bytes.len() as u64 != len {
        return Err("source_mismatch");
    }
    Ok(bytes)
}

fn validate_seal(seal: &MaterialFileSeal) -> Result<(), &'static str> {
    if !valid_relative(&seal.relative_path)
        || !valid_hash(&seal.sha256)
        || seal.identity.size == 0
        || !digits(&seal.identity.device)
        || !digits(&seal.identity.inode)
        || !digits(&seal.identity.modified_ns)
    {
        return Err("invalid_materials");
    }
    Ok(())
}

fn valid_relative(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value.contains('\0')
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && !value
            .split('/')
            .any(|component| matches!(component, "" | "." | ".."))
}

fn valid_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hash_file(file: &File) -> io::Result<String> {
    let mut digest = Sha256::new();
    let mut chunk = [0u8; 64 * 1024];
    let mut offset = 0i64;
    loop {
        let n = unsafe {
            libc::pread(
                file.as_raw_fd(),
                chunk.as_mut_ptr().cast(),
                chunk.len(),
                offset as libc::off_t,
            )
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n == 0 {
            break;
        }
        let n = n as usize;
        digest.update(&chunk[..n]);
        offset += n as i64;
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct Fixture {
        base: PathBuf,
        capture_root: PathBuf,
        material_root: PathBuf,
        runtime: Runtime,
        capture_id: String,
        materials_id: String,
        lease_id: String,
        owner_token: String,
        key: MaterialKey,
        payload_bytes: Vec<u8>,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    fn fixture() -> Fixture {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "apparent-retained-authority-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir(&base).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let runtime_path = base.join("runtime");
        fs::create_dir(&runtime_path).unwrap();
        fs::set_permissions(&runtime_path, fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = Runtime::open(&runtime_path).unwrap();
        let capture_root = base.join("capture-v2");
        let captures = SecureStore::create(&capture_root).unwrap();
        let material_parent = SecureStore::create(&capture_root.join("materials")).unwrap();
        let capture_id = apparent_capture_engine::new_id().unwrap();
        let materials_id = apparent_capture_engine::new_id().unwrap();
        let lease_id = apparent_capture_engine::new_id().unwrap();
        let owner_token = apparent_capture_engine::new_owner_token().unwrap();
        let material_root = capture_root.join("materials").join(&capture_id);
        let materials = SecureStore::create(&material_root).unwrap();

        let movie_bytes = b"sealed synthetic retained movie";
        materials.write_new("recording.mp4", movie_bytes).unwrap();
        let video = sealed(&materials, "recording.mp4");
        let payload = json!({
            "schema":"apparent.capture-materials-payload/1",
            "identity":{"captureId":capture_id,"materialsId":materials_id},
            "bundleId":"retained-fixture.apparent",
            "root":material_root,
            "leaseId":lease_id,
            "files":[video],
            "video":{"file":"recording.mp4","width":16,"height":16,"durationMs":1},
            "safety":{"safeForProcessing":true}
        });
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        materials
            .write_new("materials.json", &payload_bytes)
            .unwrap();
        let payload_file = sealed(&materials, "materials.json");
        let key = MaterialKey {
            capture_id: capture_id.clone(),
            materials_id: materials_id.clone(),
            revision: payload_file.sha256.clone(),
        };
        let manifest = json!({
            "schema":"apparent.capture-materials/1",
            "key":key,
            "root":material_root,
            "leaseId":lease_id,
            "payloadFile":payload_file,
        });
        let root_metadata = materials.directory_metadata().unwrap();
        let document = json!({
            "capture_id":capture_id,
            "owner_token":owner_token,
            "request":{"captureId":capture_id,"ownerToken":owner_token},
            "state":"materials-ready",
            "materials":manifest,
            "root":material_root,
            "materials_id":materials_id,
            "lease_id":lease_id,
            "root_device":root_metadata.dev(),
            "root_inode":root_metadata.ino(),
            "release_phase":null,
            "release_movie":video.identity,
            "release_payload":payload_file.identity,
            "payload_digest":key.revision,
        });
        captures
            .write_new(
                &format!("{capture_id}.json"),
                &serde_json::to_vec(&document).unwrap(),
            )
            .unwrap();
        drop(material_parent);
        Fixture {
            base,
            capture_root,
            material_root,
            runtime,
            capture_id,
            materials_id,
            lease_id,
            owner_token,
            key,
            payload_bytes,
        }
    }

    fn sealed(root: &SecureStore, relative_path: &str) -> MaterialFileSeal {
        let file = root.file(relative_path, false).unwrap();
        let metadata = file.metadata().unwrap();
        let mut bytes = Vec::new();
        file.try_clone().unwrap().read_to_end(&mut bytes).unwrap();
        MaterialFileSeal {
            relative_path: relative_path.into(),
            identity: MaterialFileIdentity {
                device: metadata.dev().to_string(),
                inode: metadata.ino().to_string(),
                size: metadata.len(),
                modified_ns: (i128::from(metadata.mtime()) * 1_000_000_000
                    + i128::from(metadata.mtime_nsec()))
                .to_string(),
            },
            sha256: sha(&bytes),
        }
    }

    fn write_document(fixture: &Fixture, document: &Value) {
        SecureStore::open(&fixture.capture_root)
            .unwrap()
            .replace_checked(
                &format!("{}.json", fixture.capture_id),
                &serde_json::to_vec(document).unwrap(),
            )
            .unwrap();
    }

    fn pin(fixture: &Fixture) -> Result<RetainedMaterialSource, &'static str> {
        pin_retained_materials(
            &fixture.runtime,
            &fixture.capture_root,
            &fixture.capture_id,
            &fixture.owner_token,
            &fixture.key,
            &fixture.lease_id,
        )
    }

    #[test]
    fn authenticated_guard_pins_exact_payload_and_excludes_maintenance_release() {
        let fixture = fixture();
        let source = pin(&fixture).unwrap();
        assert_eq!(source.capture_id, fixture.capture_id);
        assert_eq!(source.materials, fixture.key);
        assert_eq!(source.payload_bytes, fixture.payload_bytes);
        assert_eq!(
            source.payload_file.metadata().unwrap().ino().to_string(),
            source.materials_manifest["payloadFile"]["identity"]["inode"]
        );
        assert_eq!(
            source.video_file.metadata().unwrap().ino().to_string(),
            source.video.identity.inode
        );
        assert_eq!(
            source.root_file.metadata().unwrap().ino(),
            source.root_inode
        );
        assert_eq!(
            source.journal_bytes,
            read_authenticated_journal(
                &fixture.capture_root,
                &fixture.capture_id,
                &fixture.owner_token
            )
            .unwrap()
            .bytes
        );
        assert!(
            fixture
                .runtime
                .material_lease(&fixture.materials_id, LeaseKind::Maintenance)
                .unwrap()
                .is_none()
        );
        drop(source);
        assert!(
            fixture
                .runtime
                .material_lease(&fixture.materials_id, LeaseKind::Maintenance)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn owner_and_material_key_mismatches_are_rejected_before_source_adoption() {
        let fixture = fixture();
        fs::rename(&fixture.material_root, fixture.base.join("held-materials")).unwrap();
        assert_eq!(
            pin_retained_materials(
                &fixture.runtime,
                &fixture.capture_root,
                &fixture.capture_id,
                &"f".repeat(64),
                &fixture.key,
                &fixture.lease_id
            )
            .err()
            .unwrap(),
            "permission_denied"
        );
        fs::rename(fixture.base.join("held-materials"), &fixture.material_root).unwrap();

        let mut wrong_key = fixture.key.clone();
        wrong_key.materials_id = apparent_capture_engine::new_id().unwrap();
        assert_eq!(
            pin_retained_materials(
                &fixture.runtime,
                &fixture.capture_root,
                &fixture.capture_id,
                &fixture.owner_token,
                &wrong_key,
                &fixture.lease_id
            )
            .err()
            .unwrap(),
            "source_mismatch"
        );
        let mut wrong_revision = fixture.key.clone();
        wrong_revision.revision = "0".repeat(64);
        assert_eq!(
            pin_retained_materials(
                &fixture.runtime,
                &fixture.capture_root,
                &fixture.capture_id,
                &fixture.owner_token,
                &wrong_revision,
                &fixture.lease_id
            )
            .err()
            .unwrap(),
            "source_mismatch"
        );
        assert_eq!(
            pin_retained_materials(
                &fixture.runtime,
                &fixture.capture_root,
                &fixture.capture_id,
                &fixture.owner_token,
                &fixture.key,
                &apparent_capture_engine::new_id().unwrap()
            )
            .err()
            .unwrap(),
            "source_mismatch"
        );
    }

    #[test]
    fn payload_capture_identity_is_checked_even_with_a_consistent_seal() {
        let mut fixture = fixture();
        let mut payload: Value = serde_json::from_slice(&fixture.payload_bytes).unwrap();
        payload["identity"]["captureId"] = json!(apparent_capture_engine::new_id().unwrap());
        let bytes = serde_json::to_vec(&payload).unwrap();
        let root = SecureStore::open(&fixture.material_root).unwrap();
        fs::remove_file(fixture.material_root.join("materials.json")).unwrap();
        root.write_new("materials.json", &bytes).unwrap();
        let seal = sealed(&root, "materials.json");
        fixture.key.revision = seal.sha256.clone();
        let mut journal = read_authenticated_journal(
            &fixture.capture_root,
            &fixture.capture_id,
            &fixture.owner_token,
        )
        .unwrap()
        .document;
        journal["materials"]["key"] = json!(fixture.key);
        journal["materials"]["payloadFile"] = json!(seal);
        journal["payload_digest"] = json!(fixture.key.revision);
        journal["release_payload"] = json!(seal.identity);
        write_document(&fixture, &journal);
        assert_eq!(pin(&fixture).err().unwrap(), "invalid_materials");
    }

    #[test]
    fn sealed_paths_reject_normalized_aliases() {
        assert!(valid_relative("facts/context.json"));
        for path in [
            "facts/./context.json",
            "./recording.mp4",
            "facts/../recording.mp4",
            "facts//context.json",
        ] {
            assert!(!valid_relative(path), "{path}");
        }
    }

    #[test]
    fn pending_or_replayed_release_and_replaced_root_or_file_are_rejected() {
        let fixture = fixture();
        let journal = read_authenticated_journal(
            &fixture.capture_root,
            &fixture.capture_id,
            &fixture.owner_token,
        )
        .unwrap();
        let mut document = journal.document;
        document["release_phase"] = json!("pending");
        write_document(&fixture, &document);
        assert_eq!(pin(&fixture).err().unwrap(), "source_mismatch");
        document["release_phase"] = json!("acknowledged");
        write_document(&fixture, &document);
        assert_eq!(pin(&fixture).err().unwrap(), "source_mismatch");

        document["release_phase"] = Value::Null;
        write_document(&fixture, &document);
        fs::rename(&fixture.material_root, fixture.base.join("old-root")).unwrap();
        SecureStore::create(&fixture.material_root).unwrap();
        assert_eq!(pin(&fixture).err().unwrap(), "source_mismatch");

        fs::remove_dir_all(&fixture.material_root).unwrap();
        fs::rename(fixture.base.join("old-root"), &fixture.material_root).unwrap();
        let materials = SecureStore::open(&fixture.material_root).unwrap();
        fs::remove_file(fixture.material_root.join("recording.mp4")).unwrap();
        materials
            .write_new("recording.mp4", b"sealed synthetic retained movie")
            .unwrap();
        assert_eq!(pin(&fixture).err().unwrap(), "source_mismatch");
    }

    #[test]
    fn release_intent_committed_after_first_read_fails_the_post_lease_recheck() {
        let fixture = fixture();
        let first = read_authenticated_journal(
            &fixture.capture_root,
            &fixture.capture_id,
            &fixture.owner_token,
        )
        .unwrap();
        let first_binding = binding_from_document(
            &first.document,
            &fixture.capture_root,
            &fixture.key,
            &fixture.lease_id,
        )
        .unwrap();
        let mut pending = first.document;
        pending["release_phase"] = json!("pending");
        write_document(&fixture, &pending);
        let lease = fixture
            .runtime
            .material_lease(&fixture.materials_id, LeaseKind::Read)
            .unwrap()
            .unwrap();
        assert_eq!(
            recheck_after_lease(
                &fixture.capture_root,
                &fixture.capture_id,
                &fixture.owner_token,
                &fixture.key,
                &fixture.lease_id,
                &first_binding,
                &lease
            )
            .err()
            .unwrap(),
            "source_mismatch"
        );
        assert!(
            fixture
                .runtime
                .material_lease(&fixture.materials_id, LeaseKind::Maintenance)
                .unwrap()
                .is_none()
        );
        drop(lease);
        assert!(
            fixture
                .runtime
                .material_lease(&fixture.materials_id, LeaseKind::Maintenance)
                .unwrap()
                .is_some()
        );

        pending["release_phase"] = Value::Null;
        pending["lease_id"] = json!(apparent_capture_engine::new_id().unwrap());
        write_document(&fixture, &pending);
        let lease = fixture
            .runtime
            .material_lease(&fixture.materials_id, LeaseKind::Read)
            .unwrap()
            .unwrap();
        assert_eq!(
            recheck_after_lease(
                &fixture.capture_root,
                &fixture.capture_id,
                &fixture.owner_token,
                &fixture.key,
                &fixture.lease_id,
                &first_binding,
                &lease
            )
            .err()
            .unwrap(),
            "source_mismatch"
        );
    }
}
