//! Trusted-caller publication snapshot and no-replace commit port.
#[path = "profile.rs"]
mod profile;

use super::validate_movie;
use crate::{
    locking::Lock,
    secure::{Store, invalid},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{BufReader, Read, Seek, SeekFrom, Write},
    os::unix::fs::{FileExt, MetadataExt},
    path::{Component, Path},
    sync::Arc,
};

const PLAN_LIMIT: usize = 8 * 1024 * 1024;
const ENTRY_LIMIT: usize = 4096;
const ARTIFACT_LIMIT: u64 = 64 * 1024 * 1024 * 1024;
// Bound the entire snapshot before creating its journal or sibling stage.
const SNAPSHOT_LIMIT: u64 = 64 * 1024 * 1024 * 1024;
const TREE_LIMIT: usize = 4096;
const STAGE_PREFIX: &str = ".apparent-pub-";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MaterialsKey {
    #[serde(rename = "captureId")]
    pub capture_id: String,
    #[serde(rename = "materialsId")]
    pub materials_id: String,
    pub revision: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileIdentity {
    pub device: String,
    pub inode: String,
    pub size: u64,
    #[serde(rename = "modifiedNs")]
    pub modified_ns: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SealedFile {
    #[serde(rename = "relativePath")]
    pub relative_path: String,
    pub identity: FileIdentity,
    pub sha256: String,
}

/// A source descriptor resolved by the authenticated provider from its retained
/// materials/derivative registry or the exact Record plan. Core rechecks the
/// descriptor identity and bytes; this value does not create authority by itself.
pub struct TrustedSource {
    /// `material:<path>`, `assembled:<path>`, or `derivative:<receipt-id>:<path>`.
    pub source_key: String,
    pub file: File,
    pub seal: SealedFile,
}

/// Context the provider may construct only after authenticating owner and capture,
/// verifying retained material key/revision/root/lease, and holding read/source
/// ownership through `prepare` or `commit`. Core never authenticates user IDs.
pub struct TrustedPublicationContext {
    pub publication_id: String,
    pub materials: MaterialsKey,
    pub lease_id: String,
    pub bundle_id: String,
    pub staging_root: String,
    pub destination: String,
    /// Exact sealed file list from the provider-verified original materials payload.
    pub material_files: Vec<SealedFile>,
    /// Pinned file descriptors selected from plan provenance by the provider.
    pub sources: Vec<TrustedSource>,
    /// Trusted, pinned destination parent; final bundle leaf must remain absent.
    pub destination_parent: Arc<Store>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Plan {
    schema: String,
    publication_id: String,
    materials: MaterialsKey,
    bundle_id: String,
    staging_root: String,
    destination: Destination,
    artifact_version: u32,
    entries: Vec<Entry>,
    manifest_path: String,
    summary_path: String,
    profile: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Destination {
    path: String,
    mode: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Entry {
    relative_path: String,
    sha256: String,
    byte_length: u64,
    role: String,
    input: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Journal {
    schema: String,
    publication_id: String,
    materials: MaterialsKey,
    lease_id: String,
    bundle_id: String,
    plan_sha256: String,
    destination: String,
    stage_leaf: String,
    stage_device: String,
    stage_inode: String,
    manifest_sha256: String,
    inventory_sha256: String,
    state: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedPublication {
    pub publication_id: String,
    pub plan_sha256: String,
    pub stage_leaf: String,
}

/// This is evidence from Core's validated native commit port, not a wire receipt
/// or by itself a capture/2 `Saved` response. The caller correlates it to its
/// authenticated owner operation and serializes cancel around `commit`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PublicationEvidence {
    pub publication_id: String,
    pub materials: MaterialsKey,
    pub lease_id: String,
    pub bundle_id: String,
    pub destination: String,
    pub plan_sha256: String,
    pub manifest_sha256: String,
    pub inventory_sha256: String,
    pub destination_device: String,
    pub destination_inode: String,
    pub reconciled: bool,
}

/// Core-owned private journal namespace. Methods are serialized per publication ID.
pub struct PublicationBackend {
    control: Arc<Store>,
}
impl PublicationBackend {
    pub fn new(control: Arc<Store>) -> Self {
        Self { control }
    }

    /// Pin an output parent with the same owner/no-symlink/non-writable policy
    /// used by finalizer output paths. The returned Store keeps that inode pinned.
    pub fn open_destination_parent(path: &Path) -> Result<Arc<Store>> {
        Ok(Arc::new(Store::open_trusted_parent(path)?))
    }

    /// Validate exact plan bytes and sealed source ancestry, then snapshot only
    /// declared artifacts into a private sibling on the destination filesystem.
    /// No caller staging controls or undeclared files are ever copied.
    pub fn prepare(
        &self,
        context: &TrustedPublicationContext,
        plan_bytes: &[u8],
        plan_sha256: &str,
    ) -> Result<PreparedPublication> {
        let plan = checked_plan(context, plan_bytes, plan_sha256)?;
        let (namespace, _lock) = self.locked(&context.publication_id)?;
        let publication = namespace.child(&context.publication_id)?;
        let journal_exists = publication.entry_exists("journal.json")?;
        let existing_journal = if journal_exists {
            verify_exact(&publication, "plan.json", plan_bytes)?;
            Some(read_journal(&publication)?)
        } else {
            persist_exact(&publication, "plan.json", plan_bytes)?;
            None
        };
        let stage_leaf = if let Some(journal) = &existing_journal {
            journal.stage_leaf.clone()
        } else {
            format!(
                "{STAGE_PREFIX}{}-{}-stage",
                context.publication_id,
                crate::identity::random_hex(16)?
            )
        };
        let expected_manifest = find_entry(&plan, "apparent.json")?.sha256.clone();
        let expected_inventory = inventory_digest(&plan)?;
        let mut journal = if let Some(journal) = existing_journal {
            check_journal(&journal, &plan, context, plan_sha256)?;
            if committed_destination(&publication, &journal, &plan, context, true)?.is_some() {
                return Ok(PreparedPublication {
                    publication_id: plan.publication_id,
                    plan_sha256: plan_sha256.into(),
                    stage_leaf,
                });
            }
            ensure!(
                journal.state != "committed",
                "committed publication destination is missing; refusing to recreate it"
            );
            journal
        } else {
            let journal = Journal {
                schema: "apparent.native-publication/1".into(),
                publication_id: plan.publication_id.clone(),
                materials: plan.materials.clone(),
                lease_id: context.lease_id.clone(),
                bundle_id: plan.bundle_id.clone(),
                plan_sha256: plan_sha256.into(),
                destination: plan.destination.path.clone(),
                stage_leaf: stage_leaf.clone(),
                stage_device: String::new(),
                stage_inode: String::new(),
                manifest_sha256: expected_manifest.clone(),
                inventory_sha256: expected_inventory.clone(),
                state: "building".into(),
            };
            persist_journal(&publication, &journal)?;
            journal
        };
        let stage = context.destination_parent.child(&stage_leaf)?;
        if !journal.stage_inode.is_empty() {
            check_stage_identity(&stage, &journal)?;
        } else {
            let meta = stage.directory_metadata()?;
            journal.stage_device = meta.dev().to_string();
            journal.stage_inode = meta.ino().to_string();
            persist_journal(&publication, &journal)?;
        }
        let recovering_partial = journal.state == "building";
        validate_stage_contents(&stage, &plan.entries, recovering_partial)?;
        for entry in &plan.entries {
            if !stage_has(entry, &stage, recovering_partial)? {
                let source = source_for(entry, context)?;
                snapshot_entry(&stage, entry, source)?;
            }
        }
        validate_stage_contents(&stage, &plan.entries, false)?;
        validate_bundle(&stage, &plan, context)?;
        sync_tree(&stage)?;
        let stage_meta = stage.directory_metadata()?;
        journal.stage_device = stage_meta.dev().to_string();
        journal.stage_inode = stage_meta.ino().to_string();
        journal.state = "prepared".into();
        persist_journal(&publication, &journal)?;
        Ok(PreparedPublication {
            publication_id: plan.publication_id,
            plan_sha256: plan_sha256.into(),
            stage_leaf,
        })
    }

    /// Commit is the linearization point. The trusted caller must serialize its
    /// cancel decision with this call: cancel may win before entry; after a
    /// successful no-replace rename, commit wins. Errors after rename are
    /// unresolved until `inspect` proves the recorded inode and digests.
    pub fn commit(
        &self,
        context: &TrustedPublicationContext,
        plan_bytes: &[u8],
        plan_sha256: &str,
    ) -> Result<PublicationEvidence> {
        let plan = checked_plan(context, plan_bytes, plan_sha256)?;
        let (namespace, _lock) = self.locked(&context.publication_id)?;
        let publication = namespace.child(&context.publication_id)?;
        verify_exact(&publication, "plan.json", plan_bytes)?;
        let journal = read_journal(&publication)?;
        check_journal(&journal, &plan, context, plan_sha256)?;
        if let Some(evidence) = committed_destination(&publication, &journal, &plan, context, true)?
        {
            return Ok(evidence);
        }
        ensure!(
            journal.state == "prepared",
            "publication has not reached durable prepared state"
        );
        let stage = context.destination_parent.child(&journal.stage_leaf)?;
        check_stage_identity(&stage, &journal)?;
        validate_bundle(&stage, &plan, context)?;
        let destination_leaf = destination_leaf(&plan.destination.path)?;
        context.destination_parent.rename_directory_noreplace(&journal.stage_leaf, &destination_leaf, &stage.directory_file()?)
            .context("atomic no-replace publication rename failed; inspect the same binding to reconcile")?;
        let evidence = committed_destination(&publication, &journal, &plan, context, false)?
            .context("renamed publication did not reconcile to the recorded stage inode")?;
        Ok(evidence)
    }

    /// Read-only reconciliation. A destination is accepted only when its pinned
    /// inode equals the journalled stage inode and its complete inventory/profile
    /// still validates. A foreign populated destination is always a conflict.
    pub fn inspect(
        &self,
        context: &TrustedPublicationContext,
        plan_bytes: &[u8],
        plan_sha256: &str,
    ) -> Result<Option<PublicationEvidence>> {
        let plan = checked_plan(context, plan_bytes, plan_sha256)?;
        let (namespace, _lock) = self.locked(&context.publication_id)?;
        let publication = namespace.child(&context.publication_id)?;
        if !publication.entry_exists("journal.json")? {
            return Ok(None);
        }
        verify_exact(&publication, "plan.json", plan_bytes)?;
        let journal = read_journal(&publication)?;
        check_journal(&journal, &plan, context, plan_sha256)?;
        committed_destination(&publication, &journal, &plan, context, true)
    }

    fn locked(&self, publication_id: &str) -> Result<(Arc<Store>, Lock)> {
        validate_id(publication_id)?;
        let namespace = self.control.child("publication-v1")?;
        let lock = Lock::open(&namespace, &format!("publication-{publication_id}.lock"))?;
        lock.exclusive(&namespace)?;
        Ok((namespace, lock))
    }
}

fn checked_plan(context: &TrustedPublicationContext, bytes: &[u8], digest: &str) -> Result<Plan> {
    ensure!(
        bytes.len() <= PLAN_LIMIT && !bytes.is_empty(),
        "publication plan exceeds bound"
    );
    ensure!(
        hash(bytes) == digest && is_hash(digest),
        "publication plan digest mismatch"
    );
    validate_id(&context.publication_id)?;
    validate_id(&context.materials.capture_id)?;
    validate_id(&context.materials.materials_id)?;
    ensure!(
        is_hash(&context.materials.revision),
        "invalid materials revision"
    );
    validate_id(&context.lease_id)?;
    validate_bundle_id(&context.bundle_id)?;
    let plan: Plan = serde_json::from_slice(bytes).context("invalid exact publication plan")?;
    ensure!(
        plan.schema == "apparent.bundle-publication/1"
            && plan.artifact_version == 1
            && plan.profile == "apparent-bundle/1",
        "unsupported publication profile"
    );
    ensure!(
        plan.publication_id == context.publication_id
            && plan.materials == context.materials
            && plan.bundle_id == context.bundle_id,
        "publication binding differs from trusted capture materials"
    );
    ensure!(
        plan.staging_root == context.staging_root
            && plan.destination.mode == "absent"
            && plan.destination.path == context.destination,
        "plan roots/destination differ from trusted request"
    );
    ensure!(
        plan.manifest_path == "apparent.json" && plan.summary_path == "summary.md",
        "unsupported manifest or summary path"
    );
    ensure!(
        Path::new(&plan.destination.path).is_absolute()
            && Path::new(&plan.destination.path)
                .components()
                .all(|c| matches!(c, Component::RootDir | Component::Normal(_))),
        "destination path is not normalized absolute"
    );
    ensure!(
        Path::new(&plan.destination.path).to_string_lossy() == plan.destination.path,
        "destination path is not normalized"
    );
    ensure!(
        Path::new(&plan.staging_root).is_absolute()
            && Path::new(&plan.staging_root).to_string_lossy() == plan.staging_root,
        "staging root is not normalized absolute"
    );
    let parent = Path::new(&plan.destination.path)
        .parent()
        .ok_or_else(invalid)?;
    ensure!(
        parent == context.destination_parent.path(),
        "destination parent differs from pinned parent"
    );
    ensure!(
        plan.entries.len() >= 3 && plan.entries.len() <= ENTRY_LIMIT,
        "publication inventory size is invalid"
    );
    let total_bytes = plan.entries.iter().try_fold(0u64, |total, entry| {
        total
            .checked_add(entry.byte_length)
            .context("publication aggregate size overflow")
    })?;
    ensure!(
        total_bytes <= SNAPSHOT_LIMIT,
        "publication aggregate snapshot exceeds byte limit"
    );
    let mut paths = BTreeSet::new();
    let mut sources = BTreeSet::new();
    let mut material_paths = BTreeSet::new();
    for seal in &context.material_files {
        validate_sealed(seal)?;
        ensure!(
            material_paths.insert(seal.relative_path.as_str()),
            "duplicate path in trusted materials inventory"
        );
    }
    for source in &context.sources {
        ensure!(
            sources.insert(source.source_key.as_str()),
            "duplicate pinned publication source key"
        );
    }
    for entry in &plan.entries {
        validate_relative(&entry.relative_path)?;
        ensure!(
            paths.insert(entry.relative_path.as_str()),
            "duplicate publication path"
        );
        ensure!(
            entry.byte_length > 0 && entry.byte_length <= ARTIFACT_LIMIT && is_hash(&entry.sha256),
            "invalid publication entry size or digest"
        );
        ensure!(
            [
                "manifest",
                "summary",
                "companion",
                "frame",
                "contact-sheet",
                "video",
                "audio",
                "transcript",
                "context",
                "interactions",
                "marks"
            ]
            .contains(&entry.role.as_str()),
            "unknown publication role"
        );
        match entry.role.as_str() {
            "manifest" => ensure!(
                entry.relative_path == "apparent.json",
                "manifest role must name apparent.json"
            ),
            "summary" => ensure!(
                entry.relative_path == "summary.md",
                "summary role must name summary.md"
            ),
            "frame" | "contact-sheet" => ensure!(
                Path::new(&entry.relative_path)
                    .extension()
                    .and_then(|v| v.to_str())
                    == Some("png"),
                "image role must use PNG artifact"
            ),
            "video" => ensure!(
                matches!(
                    Path::new(&entry.relative_path)
                        .extension()
                        .and_then(|v| v.to_str()),
                    Some("mp4" | "mov")
                ),
                "video role must use MP4/MOV artifact"
            ),
            "audio" => ensure!(
                Path::new(&entry.relative_path)
                    .extension()
                    .and_then(|v| v.to_str())
                    == Some("wav"),
                "audio role must use WAV artifact"
            ),
            _ => {}
        }
        let source = source_for(entry, context)?;
        ensure!(
            source.seal.sha256 == entry.sha256 && source.seal.identity.size == entry.byte_length,
            "entry differs from sealed source"
        );
        check_identity(&source.file, &source.seal)?;
    }
    for path in &paths {
        let mut parent = Path::new(path).parent();
        while let Some(prefix) = parent {
            if prefix.as_os_str().is_empty() {
                break;
            }
            ensure!(
                !paths.contains(prefix.to_str().unwrap_or_default()),
                "publication file path is also an ancestor directory"
            );
            parent = prefix.parent();
        }
    }
    ensure!(
        find_entry(&plan, "apparent.json")?.role == "manifest"
            && find_entry(&plan, "summary.md")?.role == "summary",
        "profile lacks manifest or summary"
    );
    ensure!(
        context.material_files.len() <= ENTRY_LIMIT,
        "materials file inventory exceeds bound"
    );
    Ok(plan)
}

fn source_for<'a>(
    entry: &Entry,
    context: &'a TrustedPublicationContext,
) -> Result<&'a TrustedSource> {
    let input = entry
        .input
        .as_object()
        .context("publication input must be an object")?;
    let kind = input
        .get("kind")
        .and_then(Value::as_str)
        .context("publication input kind missing")?;
    let (key, expected) = match kind {
        "material" => {
            ensure!(input.len() == 2, "unknown material input fields");
            let path = input
                .get("relativePath")
                .and_then(Value::as_str)
                .context("material source path missing")?;
            validate_relative(path)?;
            let seal = context
                .material_files
                .iter()
                .find(|seal| seal.relative_path == path)
                .context("material source is absent from verified retained material inventory")?;
            (format!("material:{path}"), seal.clone())
        }
        "assembled" => {
            ensure!(input.len() == 2, "unknown assembled input fields");
            let seal: SealedFile = serde_json::from_value(
                input
                    .get("file")
                    .cloned()
                    .context("assembled seal missing")?,
            )?;
            validate_sealed(&seal)?;
            (format!("assembled:{}", seal.relative_path), seal)
        }
        "derivative" => {
            ensure!(input.len() == 3, "unknown derivative input fields");
            let index = input
                .get("fileIndex")
                .and_then(Value::as_u64)
                .context("derivative file index missing")?;
            ensure!(index > 0, "derivative file index must be one-based");
            let receipt = input
                .get("receipt")
                .and_then(Value::as_object)
                .context("derivative receipt missing")?;
            let receipt_id = receipt
                .get("receiptId")
                .and_then(Value::as_str)
                .context("derivative receipt identity missing")?;
            let payload = receipt
                .get("payload")
                .and_then(Value::as_object)
                .context("derivative payload missing")?;
            let frames = payload
                .get("frames")
                .and_then(Value::as_array)
                .context("derivative frame list missing")?;
            let frame = frames
                .get((index - 1) as usize)
                .and_then(Value::as_object)
                .context("derivative index outside receipt")?;
            ensure!(
                frame.get("requestIndex").and_then(Value::as_u64) == Some(index),
                "derivative index does not match request"
            );
            let seal: SealedFile = serde_json::from_value(
                frame
                    .get("file")
                    .cloned()
                    .context("derivative file seal missing")?,
            )?;
            validate_sealed(&seal)?;
            (
                format!("derivative:{receipt_id}:{}", seal.relative_path),
                seal,
            )
        }
        _ => bail!("unknown publication input kind"),
    };
    let source = context
        .sources
        .iter()
        .find(|source| source.source_key == key)
        .context("trusted provider did not supply the pinned plan source")?;
    ensure!(
        source.seal == expected,
        "provider source seal differs from exact plan/material seal"
    );
    Ok(source)
}

fn snapshot_entry(stage: &Arc<Store>, entry: &Entry, source: &TrustedSource) -> Result<()> {
    let (parent, leaf) = ensure_parent(stage, &entry.relative_path)?;
    let mut output = parent
        .file(&leaf, true)
        .context("could not exclusively create snapshot artifact")?;
    let before = check_identity(&source.file, &source.seal)?;
    let mut hasher = Sha256::new();
    let mut offset = 0u64;
    let mut buffer = vec![0u8; 1024 * 1024];
    while offset < entry.byte_length {
        let amount = usize::try_from((entry.byte_length - offset).min(buffer.len() as u64))?;
        let read = source.file.read_at(&mut buffer[..amount], offset)?;
        ensure!(read == amount, "source file ended while snapshotting");
        output.write_all(&buffer[..read])?;
        hasher.update(&buffer[..read]);
        offset += read as u64;
    }
    output.sync_all()?;
    let after = source.file.metadata()?;
    ensure!(
        same_identity(&before, &after) && hex(&hasher.finalize()) == entry.sha256,
        "source changed or hash mismatched during snapshot"
    );
    ensure!(
        output.metadata()?.len() == entry.byte_length,
        "snapshot length mismatch"
    );
    parent.directory_file()?.sync_all()?;
    Ok(())
}

fn validate_stage_contents(
    stage: &Arc<Store>,
    entries: &[Entry],
    recover_partial: bool,
) -> Result<()> {
    stage.check()?;
    let allowed: BTreeSet<_> = entries.iter().map(|e| e.relative_path.as_str()).collect();
    let mut seen = BTreeSet::new();
    let mut node_count = 0usize;
    let mut pending = vec![(stage.path().to_path_buf(), String::new())];
    while let Some((dir, prefix)) = pending.pop() {
        for item in fs::read_dir(&dir)? {
            let item = item?;
            let file_type = item.file_type()?;
            let rel = if prefix.is_empty() {
                item.file_name().to_string_lossy().into_owned()
            } else {
                format!("{prefix}/{}", item.file_name().to_string_lossy())
            };
            node_count += 1;
            ensure!(node_count <= TREE_LIMIT, "snapshot tree exceeds bound");
            if file_type.is_symlink() {
                bail!("snapshot contains symlink");
            }
            if file_type.is_dir() {
                ensure!(
                    entries
                        .iter()
                        .any(|entry| entry.relative_path.starts_with(&format!("{rel}/"))),
                    "snapshot contains an unlisted directory"
                );
                let relative_dir = if prefix.is_empty() {
                    item.file_name().to_string_lossy().into_owned()
                } else {
                    format!("{prefix}/{}", item.file_name().to_string_lossy())
                };
                let _pinned = open_directory_path(stage, &relative_dir)?;
                pending.push((item.path(), rel));
            } else {
                ensure!(
                    file_type.is_file() && allowed.contains(rel.as_str()),
                    "snapshot contains unlisted/non-file entry"
                );
                let meta = item.metadata()?;
                ensure!(
                    meta.uid() == unsafe { libc::geteuid() }
                        && meta.mode() & 0o077 == 0
                        && meta.nlink() == 1,
                    "snapshot artifact is not privately owned"
                );
                seen.insert(rel);
            }
        }
    }
    for path in seen {
        let entry = entries
            .iter()
            .find(|entry| entry.relative_path == path)
            .unwrap();
        let file = stage_file(stage, &entry.relative_path)?;
        let valid = file.metadata()?.len() == entry.byte_length
            && hash_file(&file, entry.byte_length)? == entry.sha256;
        drop(file);
        if !valid {
            ensure!(
                recover_partial,
                "existing prepared snapshot conflicts with plan"
            );
            let (parent, leaf) = open_parent(stage, &entry.relative_path)?;
            parent.remove_owned_file(&leaf)?;
        }
    }
    stage.check()?;
    Ok(())
}

fn stage_has(entry: &Entry, stage: &Arc<Store>, recover_partial: bool) -> Result<bool> {
    let (parent, leaf) = match open_parent(stage, &entry.relative_path) {
        Ok(parent) => parent,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    match parent.file(&leaf, false) {
        Ok(file) => {
            let valid = file.metadata()?.len() == entry.byte_length
                && hash_file(&file, entry.byte_length)? == entry.sha256;
            drop(file);
            if valid {
                Ok(true)
            } else {
                ensure!(
                    recover_partial,
                    "existing snapshot artifact conflicts with plan"
                );
                parent.remove_owned_file(&leaf)?;
                Ok(false)
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn validate_bundle(
    root: &Arc<Store>,
    plan: &Plan,
    _context: &TrustedPublicationContext,
) -> Result<()> {
    let entries: BTreeMap<_, _> = plan
        .entries
        .iter()
        .map(|entry| (entry.relative_path.as_str(), entry))
        .collect();
    ensure!(
        entries.len() == plan.entries.len(),
        "duplicate bundle paths"
    );
    for entry in &plan.entries {
        let file = stage_file(root, &entry.relative_path)?;
        let meta = file.metadata()?;
        ensure!(
            meta.len() == entry.byte_length && hash_file(&file, entry.byte_length)? == entry.sha256,
            "artifact differs from declared inventory"
        );
        match Path::new(&entry.relative_path)
            .extension()
            .and_then(|e| e.to_str())
        {
            Some("png") => validate_png(&file)
                .with_context(|| format!("invalid PNG {}", entry.relative_path))?,
            Some("wav") => validate_wav(&file)
                .with_context(|| format!("invalid WAV {}", entry.relative_path))?,
            Some("mp4" | "mov") => validate_movie(&file)
                .with_context(|| format!("invalid movie {}", entry.relative_path))?,
            _ => {}
        }
    }
    let manifest_bytes = read_file(&stage_file(root, "apparent.json")?, 64 * 1024 * 1024)?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)?;
    profile::validate_manifest(&manifest, &plan.bundle_id, &entries)?;
    Ok(())
}

fn committed_destination(
    publication: &Arc<Store>,
    journal: &Journal,
    plan: &Plan,
    context: &TrustedPublicationContext,
    reconciled: bool,
) -> Result<Option<PublicationEvidence>> {
    let leaf = destination_leaf(&plan.destination.path)?;
    if !context.destination_parent.entry_exists(&leaf)? {
        return Ok(None);
    }
    let destination = context
        .destination_parent
        .open_child_directory(&leaf)
        .context("destination exists but is not an owned publication directory")?;
    let meta = destination.metadata()?;
    ensure!(
        meta.dev().to_string() == journal.stage_device
            && meta.ino().to_string() == journal.stage_inode,
        "destination is foreign or does not match the transaction's recorded stage inode"
    );
    let bundle = Arc::new(
        Store::open(Path::new(&plan.destination.path))
            .context("published bundle directory identity is unsafe")?,
    );
    let opened = bundle.directory_metadata()?;
    ensure!(
        opened.dev() == meta.dev() && opened.ino() == meta.ino(),
        "destination path changed after pinned directory open"
    );
    validate_bundle(&bundle, plan, context)?;
    sync_store_tree(&bundle)?;
    context.destination_parent.directory_file()?.sync_all()?;
    let mut committed = journal.clone();
    committed.state = "committed".into();
    persist_journal(publication, &committed)?;
    let manifest = find_entry(plan, "apparent.json")?;
    Ok(Some(PublicationEvidence {
        publication_id: plan.publication_id.clone(),
        materials: plan.materials.clone(),
        lease_id: context.lease_id.clone(),
        bundle_id: plan.bundle_id.clone(),
        destination: plan.destination.path.clone(),
        plan_sha256: journal.plan_sha256.clone(),
        manifest_sha256: manifest.sha256.clone(),
        inventory_sha256: journal.inventory_sha256.clone(),
        destination_device: meta.dev().to_string(),
        destination_inode: meta.ino().to_string(),
        reconciled,
    }))
}

fn check_stage_identity(stage: &Arc<Store>, journal: &Journal) -> Result<()> {
    let meta = stage.directory_metadata()?;
    ensure!(
        meta.dev().to_string() == journal.stage_device
            && meta.ino().to_string() == journal.stage_inode,
        "staging inode differs from durable transaction binding"
    );
    Ok(())
}
fn check_journal(
    journal: &Journal,
    plan: &Plan,
    context: &TrustedPublicationContext,
    digest: &str,
) -> Result<()> {
    ensure!(
        journal.schema == "apparent.native-publication/1"
            && journal.publication_id == plan.publication_id
            && journal.materials == plan.materials
            && journal.lease_id == context.lease_id
            && journal.bundle_id == plan.bundle_id
            && journal.plan_sha256 == digest
            && journal.destination == plan.destination.path
            && journal.manifest_sha256 == find_entry(plan, "apparent.json")?.sha256
            && journal.inventory_sha256 == inventory_digest(plan)?,
        "original publication binding conflicts with durable journal"
    );
    Ok(())
}
fn persist_exact(store: &Arc<Store>, key: &str, bytes: &[u8]) -> Result<()> {
    if store.entry_exists(key)? {
        let mut old = Vec::new();
        store
            .file(key, false)?
            .take((PLAN_LIMIT + 1) as u64)
            .read_to_end(&mut old)?;
        ensure!(
            old == bytes,
            "durable original plan bytes conflict with replay"
        );
    } else {
        store.write_new(key, bytes)?;
    }
    Ok(())
}
fn verify_exact(store: &Arc<Store>, key: &str, expected: &[u8]) -> Result<()> {
    let mut bytes = Vec::new();
    store
        .file(key, false)?
        .take((PLAN_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes == expected,
        "exact durable publication plan bytes differ from original binding"
    );
    Ok(())
}
fn persist_journal(store: &Arc<Store>, journal: &Journal) -> Result<()> {
    store.write("journal.json", &serde_json::to_vec(journal)?)?;
    Ok(())
}
fn read_journal(store: &Arc<Store>) -> Result<Journal> {
    let bytes = store
        .read("journal.json")?
        .context("publication journal is absent")?;
    let journal: Journal =
        serde_json::from_slice(&bytes).context("publication journal is malformed")?;
    ensure!(
        journal.schema == "apparent.native-publication/1"
            && ["building", "prepared", "committed"].contains(&journal.state.as_str()),
        "unknown publication journal state"
    );
    validate_id(&journal.publication_id)?;
    let expected_prefix = format!("{STAGE_PREFIX}{}-", journal.publication_id);
    ensure!(
        journal.stage_leaf.starts_with(&expected_prefix) && journal.stage_leaf.ends_with("-stage"),
        "journal stage leaf differs from publication identity"
    );
    ensure!(
        journal
            .stage_leaf
            .strip_prefix(&expected_prefix)
            .and_then(|middle| middle.strip_suffix("-stage"))
            .is_some_and(|random| random.len() == 32
                && random
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())),
        "journal stage nonce is malformed"
    );
    ensure!(
        is_hash(&journal.plan_sha256)
            && is_hash(&journal.manifest_sha256)
            && is_hash(&journal.inventory_sha256),
        "journal digest is malformed"
    );
    if journal.state != "building" {
        ensure!(
            journal.stage_device.parse::<u64>().is_ok()
                && journal.stage_inode.parse::<u64>().is_ok(),
            "journal stage identity is malformed"
        );
    }
    Ok(journal)
}
fn inventory_digest(plan: &Plan) -> Result<String> {
    let mut inventory: Vec<_> = plan
        .entries
        .iter()
        .map(|e| (&e.relative_path, &e.sha256, e.byte_length, &e.role))
        .collect();
    inventory.sort_by(|a, b| a.0.cmp(b.0));
    Ok(hash(&serde_json::to_vec(&inventory)?))
}
fn find_entry<'a>(plan: &'a Plan, path: &str) -> Result<&'a Entry> {
    plan.entries
        .iter()
        .find(|entry| entry.relative_path == path)
        .context("required bundle inventory entry missing")
}
fn destination_leaf(path: &str) -> Result<String> {
    Ok(Path::new(path)
        .file_name()
        .context("destination leaf missing")?
        .to_str()
        .context("destination leaf is not UTF-8")?
        .to_owned())
}
fn validate_id(id: &str) -> Result<()> {
    ensure!(
        id.len() == 32
            && id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "invalid publication identity"
    );
    Ok(())
}
fn validate_bundle_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 255
            && !id.contains('/')
            && !id.contains('\\')
            && id != "."
            && id != "..",
        "invalid bundle identity"
    );
    Ok(())
}
fn is_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn hash(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn validate_relative(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty() && path.len() <= 1024 && !path.contains('\\') && !path.starts_with('/'),
        "unsafe relative artifact path"
    );
    let parts: Vec<_> = path.split('/').collect();
    ensure!(
        parts.len() <= 16
            && parts.iter().all(|part| !part.is_empty()
                && !matches!(*part, "." | "..")
                && !part.starts_with(".record-publication")
                && *part != "publication-plan.json"),
        "unsafe/controlled artifact path"
    );
    Ok(())
}
fn validate_sealed(seal: &SealedFile) -> Result<()> {
    validate_relative(&seal.relative_path)?;
    ensure!(
        is_hash(&seal.sha256) && seal.identity.size > 0 && seal.identity.size <= ARTIFACT_LIMIT,
        "invalid sealed file identity"
    );
    Ok(())
}
fn check_identity(file: &File, seal: &SealedFile) -> Result<std::fs::Metadata> {
    let meta = file.metadata()?;
    let modified_ns =
        (meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128).to_string();
    ensure!(
        meta.is_file()
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0
            && meta.nlink() == 1
            && meta.dev().to_string() == seal.identity.device
            && meta.ino().to_string() == seal.identity.inode
            && meta.len() == seal.identity.size
            && modified_ns == seal.identity.modified_ns,
        "source descriptor identity differs from its seal"
    );
    Ok(meta)
}
fn same_identity(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
}
fn hash_file(file: &File, len: u64) -> Result<String> {
    let mut file = file.try_clone()?;
    file.seek(SeekFrom::Start(0))?;
    let mut hasher = Sha256::new();
    let mut take = (&mut file).take(len + 1);
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = take.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        hasher.update(&buffer[..n]);
    }
    ensure!(
        total == len,
        "file length differs from expected digest range"
    );
    Ok(hex(&hasher.finalize()))
}
fn stage_file(root: &Arc<Store>, path: &str) -> Result<File> {
    let (parent, leaf) = open_parent(root, path)?;
    Ok(parent.file(&leaf, false)?)
}
fn ensure_parent(root: &Arc<Store>, path: &str) -> Result<(Arc<Store>, String)> {
    let parts: Vec<_> = path.split('/').collect();
    let mut current = root.clone();
    for part in &parts[..parts.len() - 1] {
        current = current.child(part)?;
    }
    Ok((current, parts.last().unwrap().to_string()))
}
fn open_parent(root: &Arc<Store>, path: &str) -> Result<(Arc<Store>, String)> {
    let parts: Vec<_> = path.split('/').collect();
    let mut current = root.clone();
    for part in &parts[..parts.len() - 1] {
        current = current
            .child(part)
            .with_context(|| format!("artifact parent unavailable: {part}"))?;
    }
    Ok((current, parts.last().unwrap().to_string()))
}
fn read_file(file: &File, limit: u64) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    file.try_clone()?.take(limit + 1).read_to_end(&mut out)?;
    ensure!(
        out.len() as u64 <= limit,
        "document exceeds validation bound"
    );
    Ok(out)
}
fn sync_tree(root: &Arc<Store>) -> Result<()> {
    sync_store_tree(root)
}
fn sync_store_tree(root: &Arc<Store>) -> Result<()> {
    root.check()?;
    for item in fs::read_dir(root.path())? {
        let item = item?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("non-UTF8 artifact path"))?;
        let kind = item.file_type()?;
        ensure!(!kind.is_symlink(), "symlink entered publication tree");
        if kind.is_dir() {
            let child = root.child(&name)?;
            sync_store_tree(&child)?;
        } else {
            root.file(&name, false)?.sync_all()?;
        }
    }
    root.directory_file()?.sync_all()?;
    root.check()?;
    Ok(())
}
fn open_directory_path(root: &Arc<Store>, relative: &str) -> Result<Arc<Store>> {
    let mut current = root.clone();
    for part in relative.split('/') {
        current = current.child(part)?;
    }
    Ok(current)
}
fn validate_png(file: &File) -> Result<()> {
    ensure!(
        file.metadata()?.len() <= 512 * 1024 * 1024,
        "PNG exceeds validation bound"
    );
    let mut input = file.try_clone()?;
    input.seek(SeekFrom::Start(0))?;
    let mut decoder = png::Decoder::new(BufReader::new(input));
    decoder.set_limits(png::Limits {
        bytes: 512 * 1024 * 1024,
    });
    let mut reader = decoder.read_info()?;
    let info = reader.info();
    ensure!(
        info.width > 0
            && info.height > 0
            && u64::from(info.width) * u64::from(info.height) <= 128 * 1024 * 1024,
        "invalid PNG dimensions"
    );
    let size = reader
        .output_buffer_size()
        .context("PNG decoded size overflow")?;
    ensure!(size <= 512 * 1024 * 1024, "PNG decoded size exceeds bound");
    let mut pixels = vec![0; size];
    reader.next_frame(&mut pixels)?;
    reader.finish()?;
    Ok(())
}
fn validate_wav(file: &File) -> Result<()> {
    use std::os::unix::fs::FileExt;
    #[derive(Clone, Copy)]
    struct Format {
        tag: u16,
        channels: u16,
        rate: u32,
        byte_rate: u32,
        align: u16,
        bits: u16,
    }
    let mut header = [0u8; 12];
    file.read_exact_at(&mut header, 0)?;
    ensure!(
        &header[..4] == b"RIFF" && &header[8..] == b"WAVE",
        "invalid WAV header"
    );
    ensure!(
        u64::from(u32::from_le_bytes(header[4..8].try_into()?)) + 8 == file.metadata()?.len(),
        "truncated WAV size"
    );
    let mut at = 12u64;
    let len = file.metadata()?.len();
    let mut format = None;
    let mut data_chunks = Vec::new();
    while at + 8 <= len {
        let mut chunk = [0u8; 8];
        file.read_exact_at(&mut chunk, at)?;
        let size = u64::from(u32::from_le_bytes(chunk[4..8].try_into()?));
        let end = at
            .checked_add(8)
            .and_then(|v| v.checked_add(size))
            .context("WAV chunk overflow")?;
        ensure!(end <= len, "truncated WAV chunk");
        if &chunk[..4] == b"fmt " {
            ensure!(size >= 16, "short WAV format chunk");
            let mut bytes = [0u8; 16];
            file.read_exact_at(&mut bytes, at + 8)?;
            format = Some(Format {
                tag: u16::from_le_bytes(bytes[0..2].try_into()?),
                channels: u16::from_le_bytes(bytes[2..4].try_into()?),
                rate: u32::from_le_bytes(bytes[4..8].try_into()?),
                byte_rate: u32::from_le_bytes(bytes[8..12].try_into()?),
                align: u16::from_le_bytes(bytes[12..14].try_into()?),
                bits: u16::from_le_bytes(bytes[14..16].try_into()?),
            });
        }
        if &chunk[..4] == b"data" {
            data_chunks.push((at + 8, size));
        }
        at = end + (size & 1);
    }
    ensure!(
        at == len && !data_chunks.is_empty(),
        "incomplete WAV structure"
    );
    let format = format.context("WAV format chunk missing")?;
    ensure!(
        format.channels > 0 && format.rate > 0 && format.align > 0,
        "invalid WAV channel/rate/alignment"
    );
    ensure!(matches!(format.tag, 1 | 3), "unsupported WAV sample format");
    ensure!(
        if format.tag == 1 {
            matches!(format.bits, 8 | 16 | 24 | 32)
        } else {
            format.bits == 32
        },
        "unsupported WAV sample depth"
    );
    let sample_bytes = u32::from(format.bits / 8);
    let expected_align = u32::from(format.channels)
        .checked_mul(sample_bytes)
        .context("WAV alignment overflow")?;
    ensure!(
        u32::from(format.align) == expected_align
            && format.byte_rate
                == format
                    .rate
                    .checked_mul(expected_align)
                    .context("WAV byte rate overflow")?,
        "inconsistent WAV format rates"
    );
    for (start, size) in data_chunks {
        ensure!(
            size % u64::from(format.align) == 0,
            "WAV data is not sample aligned"
        );
        if format.tag == 3 {
            let mut offset = 0u64;
            let mut samples = [0u8; 4 * 4096];
            while offset < size {
                let count = ((size - offset).min(samples.len() as u64) as usize) & !3;
                if count == 0 {
                    break;
                }
                file.read_exact_at(&mut samples[..count], start + offset)?;
                for bytes in samples[..count].chunks_exact(4) {
                    ensure!(
                        f32::from_le_bytes(bytes.try_into()?).is_finite(),
                        "nonfinite WAV sample"
                    );
                }
                offset += count as u64;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
    fn temp() -> PathBuf {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "apparent-pub-backend-{}",
            crate::identity::new_id().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        root
    }
    fn fixture(
        root: &Path,
    ) -> (
        Arc<Store>,
        Arc<Store>,
        TrustedPublicationContext,
        Vec<u8>,
        String,
    ) {
        let output = root.join("bundles");
        fs::create_dir(&output).unwrap();
        fs::set_permissions(&output, fs::Permissions::from_mode(0o700)).unwrap();
        let output_store = Arc::new(Store::open(&output).unwrap());
        let control_path = root.join("control");
        fs::create_dir(&control_path).unwrap();
        fs::set_permissions(&control_path, fs::Permissions::from_mode(0o700)).unwrap();
        let control = Arc::new(Store::open(&control_path).unwrap());
        let source_path = root.join("source");
        fs::create_dir(&source_path).unwrap();
        fs::set_permissions(&source_path, fs::Permissions::from_mode(0o700)).unwrap();
        let source_store = Arc::new(Store::open(&source_path).unwrap());
        let frame_bytes = tiny_png();
        source_store.write_new("frame.png", &frame_bytes).unwrap();
        source_store.write_new("sheet.png", &frame_bytes).unwrap();
        source_store.write_new("apparent.json", br#"{"version":1,"id":"bundle-a","createdAtMs":1,"durationMs":1,"target":{"mode":"display","displayId":1,"frame":{"x":0,"y":0,"width":1,"height":1,"scale":1}},"transcript":{"source":"none","text":"","segments":[]},"strokes":[],"marks":[],"events":[],"keyframes":[{"index":1,"t":0,"reason":"initial","file":"frame.png","event":"start"}],"contactSheet":"sheet.png"}"#).unwrap();
        source_store
            .write_new("summary.md", b"# Capture\n")
            .unwrap();
        let paths = ["apparent.json", "summary.md", "frame.png", "sheet.png"];
        let mut entries = Vec::new();
        let mut sources = Vec::new();
        for path in paths {
            let file = source_store.file(path, false).unwrap();
            let meta = file.metadata().unwrap();
            let bytes = read_file(&file, ARTIFACT_LIMIT).unwrap();
            let seal = SealedFile {
                relative_path: path.into(),
                identity: FileIdentity {
                    device: meta.dev().to_string(),
                    inode: meta.ino().to_string(),
                    size: meta.len(),
                    modified_ns: (meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128)
                        .to_string(),
                },
                sha256: hash(&bytes),
            };
            entries.push(serde_json::json!({"relativePath":path,"sha256":seal.sha256,"byteLength":seal.identity.size,"role":match path {"apparent.json"=>"manifest","summary.md"=>"summary","frame.png"=>"frame",_=>"contact-sheet"},"input":{"kind":"assembled","file":seal}}));
            sources.push(TrustedSource {
                source_key: format!("assembled:{path}"),
                file,
                seal,
            });
        }
        let plan = serde_json::json!({"schema":"apparent.bundle-publication/1","publicationId":"11111111111111111111111111111111","materials":{"captureId":"22222222222222222222222222222222","materialsId":"33333333333333333333333333333333","revision":"4444444444444444444444444444444444444444444444444444444444444444"},"bundleId":"bundle-a","stagingRoot":"/private/record/op","destination":{"path":output.join("bundle-a.apparent").to_string_lossy(),"mode":"absent"},"artifactVersion":1,"entries":entries,"manifestPath":"apparent.json","summaryPath":"summary.md","profile":"apparent-bundle/1"});
        let bytes = serde_json::to_vec(&plan).unwrap();
        let digest = hash(&bytes);
        fs::write(root.join("plan.bin"), &bytes).unwrap();
        let context = TrustedPublicationContext {
            publication_id: "11111111111111111111111111111111".into(),
            materials: MaterialsKey {
                capture_id: "22222222222222222222222222222222".into(),
                materials_id: "33333333333333333333333333333333".into(),
                revision: "4444444444444444444444444444444444444444444444444444444444444444".into(),
            },
            lease_id: "55555555555555555555555555555555".into(),
            bundle_id: "bundle-a".into(),
            staging_root: "/private/record/op".into(),
            destination: output
                .join("bundle-a.apparent")
                .to_string_lossy()
                .into_owned(),
            material_files: vec![],
            sources,
            destination_parent: output_store.clone(),
        };
        (control, output_store, context, bytes, digest)
    }
    fn tiny_png() -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[0, 0, 0, 255]).unwrap();
        drop(writer);
        out
    }
    #[test]
    fn aggregate_snapshot_budget_rejects_before_any_publication_side_effect() {
        let root = temp();
        let (control, _, context, bytes, _) = fixture(&root);
        let backend = PublicationBackend::new(control);
        for (length, expected) in [
            (ARTIFACT_LIMIT, "aggregate snapshot"),
            (u64::MAX, "aggregate size overflow"),
        ] {
            let mut plan: Value = serde_json::from_slice(&bytes).unwrap();
            plan["entries"][0]["byteLength"] = length.into();
            plan["entries"][1]["byteLength"] = length.into();
            let changed = serde_json::to_vec(&plan).unwrap();
            let error = backend
                .prepare(&context, &changed, &hash(&changed))
                .err()
                .unwrap();
            assert!(error.to_string().contains(expected), "{error:#}");
            assert!(fs::read_dir(root.join("bundles")).unwrap().next().is_none());
            assert!(fs::read_dir(root.join("control")).unwrap().next().is_none());
        }
        fs::remove_dir_all(root).unwrap();
    }
    fn wav_bytes(tag: u16, bits: u16, samples: &[u8]) -> Vec<u8> {
        let channels = 1u16;
        let rate = 16_000u32;
        let align = channels * (bits / 8);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
        bytes.extend_from_slice(&align.to_le_bytes());
        bytes.extend_from_slice(&bits.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        bytes.extend_from_slice(samples);
        let riff = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&riff.to_le_bytes());
        bytes
    }
    fn reopen(
        root: &Path,
    ) -> (
        Arc<Store>,
        Arc<Store>,
        TrustedPublicationContext,
        Vec<u8>,
        String,
    ) {
        let control = Arc::new(Store::open(&root.join("control")).unwrap());
        let output = Arc::new(Store::open(&root.join("bundles")).unwrap());
        let source = Arc::new(Store::open(&root.join("source")).unwrap());
        let plan_bytes = fs::read(root.join("plan.bin")).unwrap();
        let plan: Plan = serde_json::from_slice(&plan_bytes).unwrap();
        let sources = plan
            .entries
            .iter()
            .map(|entry| {
                let seal: SealedFile = serde_json::from_value(entry.input["file"].clone()).unwrap();
                TrustedSource {
                    source_key: format!("assembled:{}", seal.relative_path),
                    file: source.file(&seal.relative_path, false).unwrap(),
                    seal,
                }
            })
            .collect();
        let context = TrustedPublicationContext {
            publication_id: plan.publication_id.clone(),
            materials: plan.materials.clone(),
            lease_id: "55555555555555555555555555555555".into(),
            bundle_id: plan.bundle_id.clone(),
            staging_root: plan.staging_root.clone(),
            destination: plan.destination.path.clone(),
            material_files: vec![],
            sources,
            destination_parent: output.clone(),
        };
        let digest = hash(&plan_bytes);
        (control, output, context, plan_bytes, digest)
    }
    #[test]
    fn snapshot_commit_and_lost_reply_reconcile_only_recorded_inode() {
        let root = temp();
        let (control, _output, context, plan, digest) = fixture(&root);
        let backend = PublicationBackend::new(control);
        backend.prepare(&context, &plan, &digest).unwrap();
        let first = backend.commit(&context, &plan, &digest).unwrap();
        let retry = backend.inspect(&context, &plan, &digest).unwrap().unwrap();
        assert_eq!(first.destination_inode, retry.destination_inode);
        assert!(retry.reconciled);
        assert!(
            !Path::new(&context.destination)
                .join("publication-plan.json")
                .exists()
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn prepared_stage_stays_absent_until_caller_commit_decision() {
        let root = temp();
        let (control, _, context, plan, digest) = fixture(&root);
        PublicationBackend::new(control)
            .prepare(&context, &plan, &digest)
            .unwrap();
        assert!(!Path::new(&context.destination).exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn replacement_of_prepared_stage_inode_is_rejected() {
        let root = temp();
        let (control, output, context, plan, digest) = fixture(&root);
        let backend = PublicationBackend::new(control);
        let prepared = backend.prepare(&context, &plan, &digest).unwrap();
        let stage_path = output.path().join(&prepared.stage_leaf);
        fs::remove_dir_all(&stage_path).unwrap();
        fs::create_dir(&stage_path).unwrap();
        fs::set_permissions(&stage_path, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(backend.commit(&context, &plan, &digest).is_err());
        assert!(!Path::new(&context.destination).exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn subprocess_reconciles_rename_after_lost_commit_reply() {
        if let Ok(root) = std::env::var("APPARENT_PUBLICATION_RECONCILE_ROOT") {
            let root = PathBuf::from(root);
            let (control, _, context, plan, digest) = reopen(&root);
            let evidence = PublicationBackend::new(control)
                .inspect(&context, &plan, &digest)
                .unwrap()
                .unwrap();
            assert!(evidence.reconciled);
            return;
        }
        let root = temp();
        let (control, output, context, plan, digest) = fixture(&root);
        let backend = PublicationBackend::new(control);
        let prepared = backend.prepare(&context, &plan, &digest).unwrap();
        let stage = output.child(&prepared.stage_leaf).unwrap();
        let destination = destination_leaf(&context.destination).unwrap();
        output
            .rename_directory_noreplace(
                &prepared.stage_leaf,
                &destination,
                &stage.directory_file().unwrap(),
            )
            .unwrap();
        drop(backend);
        drop(context);
        drop(output);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(
                "publication::backend::tests::subprocess_reconciles_rename_after_lost_commit_reply",
            )
            .arg("--nocapture")
            .env("APPARENT_PUBLICATION_RECONCILE_ROOT", &root)
            .status()
            .unwrap();
        assert!(status.success());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn existing_foreign_destination_is_never_saved_or_replaced() {
        let root = temp();
        let (control, _output, context, plan, digest) = fixture(&root);
        let backend = PublicationBackend::new(control);
        backend.prepare(&context, &plan, &digest).unwrap();
        fs::create_dir(&context.destination).unwrap();
        fs::write(Path::new(&context.destination).join("foreign"), b"keep").unwrap();
        assert!(backend.commit(&context, &plan, &digest).is_err());
        assert_eq!(
            fs::read(Path::new(&context.destination).join("foreign")).unwrap(),
            b"keep"
        );
        assert!(backend.inspect(&context, &plan, &digest).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn plan_replay_conflict_and_unlisted_material_control_are_rejected() {
        let root = temp();
        let (control, _output, context, plan, digest) = fixture(&root);
        let backend = PublicationBackend::new(control);
        backend.prepare(&context, &plan, &digest).unwrap();
        let mut changed = plan.clone();
        changed.push(b' ');
        let changed_digest = hash(&changed);
        assert!(
            backend
                .prepare(&context, &changed, &changed_digest)
                .is_err()
        );
        assert!(validate_relative(".record-publication").is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn changed_source_identity_or_bytes_fail_before_snapshot() {
        let root = temp();
        let (control, _, context, plan, digest) = fixture(&root);
        fs::write(root.join("source/apparent.json"), b"changed source bytes").unwrap();
        let error = PublicationBackend::new(control)
            .prepare(&context, &plan, &digest)
            .unwrap_err();
        assert!(error.to_string().contains("seal") || format!("{error:#}").contains("identity"));
        assert!(!Path::new(&context.destination).exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn symlink_replacement_of_stage_is_rejected_without_following_target() {
        let root = temp();
        let (control, output, context, plan, digest) = fixture(&root);
        let backend = PublicationBackend::new(control);
        let prepared = backend.prepare(&context, &plan, &digest).unwrap();
        let stage = output.path().join(&prepared.stage_leaf);
        fs::remove_dir_all(&stage).unwrap();
        std::os::unix::fs::symlink(root.join("source"), &stage).unwrap();
        assert!(backend.commit(&context, &plan, &digest).is_err());
        assert!(!Path::new(&context.destination).exists());
        assert!(root.join("source/apparent.json").is_file());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn changed_declared_inventory_cannot_rebind_prepared_journal() {
        let root = temp();
        let (control, _, context, plan, digest) = fixture(&root);
        let backend = PublicationBackend::new(control);
        backend.prepare(&context, &plan, &digest).unwrap();
        let original: Value = serde_json::from_slice(&plan).unwrap();
        let mut changed = original;
        changed["entries"][0]["sha256"] = Value::String("a".repeat(64));
        let bytes = serde_json::to_vec(&changed).unwrap();
        let changed_digest = hash(&bytes);
        assert!(backend.prepare(&context, &bytes, &changed_digest).is_err());
        assert!(backend.inspect(&context, &plan, &digest).unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn wav_validation_checks_pcm_structure_and_finite_float_samples() {
        let root = temp();
        let pcm_path = root.join("valid.wav");
        fs::write(&pcm_path, wav_bytes(1, 16, &[0, 0, 255, 127])).unwrap();
        validate_wav(&File::open(&pcm_path).unwrap()).unwrap();
        let float_path = root.join("float.wav");
        fs::write(&float_path, wav_bytes(3, 32, &f32::INFINITY.to_le_bytes())).unwrap();
        assert!(validate_wav(&File::open(&float_path).unwrap()).is_err());
        let broken_path = root.join("broken.wav");
        let mut broken = wav_bytes(1, 16, &[0, 0]);
        broken.truncate(broken.len() - 1);
        fs::write(&broken_path, broken).unwrap();
        assert!(validate_wav(&File::open(&broken_path).unwrap()).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn interrupted_snapshot_restarts_from_its_exact_building_binding() {
        let root = temp();
        let (control, output, context, plan_bytes, digest) = fixture(&root);
        let backend = PublicationBackend::new(control);
        {
            let (namespace, lock) = backend.locked(&context.publication_id).unwrap();
            let publication = namespace.child(&context.publication_id).unwrap();
            persist_exact(&publication, "plan.json", &plan_bytes).unwrap();
            let stage_leaf = format!(
                "{STAGE_PREFIX}{}-{}-stage",
                context.publication_id,
                "a".repeat(32)
            );
            let stage = output.child(&stage_leaf).unwrap();
            let mut partial = stage.file("summary.md", true).unwrap();
            partial.write_all(b"partial crash tail").unwrap();
            partial.sync_all().unwrap();
            let metadata = stage.directory_metadata().unwrap();
            let plan: Plan = serde_json::from_slice(&plan_bytes).unwrap();
            persist_journal(
                &publication,
                &Journal {
                    schema: "apparent.native-publication/1".into(),
                    publication_id: plan.publication_id.clone(),
                    materials: plan.materials.clone(),
                    lease_id: context.lease_id.clone(),
                    bundle_id: plan.bundle_id.clone(),
                    plan_sha256: digest.clone(),
                    destination: plan.destination.path.clone(),
                    stage_leaf,
                    stage_device: metadata.dev().to_string(),
                    stage_inode: metadata.ino().to_string(),
                    manifest_sha256: find_entry(&plan, "apparent.json").unwrap().sha256.clone(),
                    inventory_sha256: inventory_digest(&plan).unwrap(),
                    state: "building".into(),
                },
            )
            .unwrap();
            drop(lock);
        }
        backend.prepare(&context, &plan_bytes, &digest).unwrap();
        assert!(backend.commit(&context, &plan_bytes, &digest).is_ok());
        fs::remove_dir_all(root).unwrap();
    }
}
