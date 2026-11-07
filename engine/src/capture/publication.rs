//! capture/2 provider adapter for the existing Core native publication port.
use super::retained::{self, RetainedMaterialSource};
use super::{
    Journal as CaptureJournal, Shared,
    materials_media::{JobRegistry as MediaJobRegistry, StoredJobState},
    persist, read_journal, sha,
};
use apparent_capture_engine::{
    Runtime, SecureStore,
    publication::{
        MaterialsKey, PublicationBackend, PublicationEvidence, PublicationRecoveryBinding,
        SealedFile, TrustedPublicationContext, TrustedSource,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

const PLAN_LIMIT: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    pub publication_id: String,
    pub plan_sha256: String,
    pub staging_root: String,
    pub plan_file: SealedFile,
    pub materials: MaterialsKey,
    pub lease_id: String,
    pub bundle_id: String,
    pub bundle_path: String,
    pub manifest_sha256: String,
    pub state: String,
    #[serde(default)]
    pub commit_decided: bool,
    #[serde(default)]
    pub evidence: Option<Receipt>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Receipt {
    capture_id: String,
    materials: MaterialsKey,
    publication_id: String,
    plan_sha256: String,
    bundle_id: String,
    bundle_path: String,
    artifact_version: u32,
    artifact_provenance: String,
    manifest_sha256: String,
    inventory_sha256: String,
}

struct Opened {
    backend: PublicationBackend,
    context: TrustedPublicationContext,
    plan_bytes: Vec<u8>,
    _retained: RetainedMaterialSource,
}

pub(super) fn worker_panicked(journal: &mut CaptureJournal) {
    if let Some(publication) = &mut journal.publication {
        if publication.state == "saved" {
            return;
        }
        // A durable commit decision can already have produced the destination.
        // Keep that outcome unresolved until authenticated Core inspection.
        if publication.commit_decided {
            publication.state = "publishing".into();
            journal.error = None;
        } else {
            publication.state = "failed".into();
            journal.error = Some("internal".into());
        }
    }
}

pub(super) fn status_fields(status: &mut Value, publication: &Journal) {
    status["state"] = Value::String(publication.state.clone());
    if matches!(
        publication.state.as_str(),
        "publishing" | "failed" | "cancelled"
    ) {
        status["publicationId"] = Value::String(publication.publication_id.clone());
        status["planSha256"] = Value::String(publication.plan_sha256.clone());
    }
    if publication.state == "saved" {
        if let Some(receipt) = &publication.evidence {
            status["publication"] = serde_json::to_value(receipt).unwrap_or(Value::Null);
        }
    }
}

/// Return the durable observation while a publication worker owns the slow
/// snapshot/commit path. This keeps same-connection inspect responsive.
pub(super) fn observe(request: &Value, root: &Path) -> Result<Value, &'static str> {
    let capture = request["captureId"].as_str().ok_or("malformed_request")?;
    let owner = request["ownerToken"].as_str().ok_or("malformed_request")?;
    let journal = read_journal(root, capture, owner)?;
    let publication = journal.publication.as_ref().ok_or("not_found")?;
    if publication.publication_id != request["publicationId"].as_str().unwrap_or_default()
        || publication.plan_sha256 != request["planSha256"].as_str().unwrap_or_default()
    {
        return Err("identity_conflict");
    }
    Ok(status_value(&journal))
}

pub(super) fn handle(
    request: &Value,
    runtime: &Runtime,
    root: &Path,
    all: &mut std::collections::HashMap<String, Arc<Shared>>,
) -> Result<Value, &'static str> {
    let method = request["method"].as_str().ok_or("malformed_request")?;
    let capture = request["captureId"].as_str().ok_or("malformed_request")?;
    let owner = request["ownerToken"].as_str().ok_or("malformed_request")?;

    // Authenticate the durable owner before publication state, plan, or receipts are read.
    let authenticated = read_journal(root, capture, owner)?;
    let mut journal = {
        let _owner = runtime
            .acquire_capture_owner(capture)
            .map_err(|_| "internal")?
            .ok_or("busy")?;
        read_journal(root, capture, owner)?
    };
    if journal.owner_token != authenticated.owner_token {
        return Err("permission_denied");
    }

    let mut opened = None;
    if method == "publish" {
        if let Some(prior) = journal
            .publication
            .as_ref()
            .filter(|publication| publication.state == "saved")
        {
            if request["publicationId"].as_str() != Some(prior.publication_id.as_str())
                || request["planSha256"].as_str() != Some(prior.plan_sha256.as_str())
                || request["materials"]
                    != serde_json::to_value(&prior.materials).map_err(|_| "internal")?
                || request["leaseId"].as_str() != Some(prior.lease_id.as_str())
                || request["stagingRoot"].as_str() != Some(prior.staging_root.as_str())
                || serde_json::from_value::<SealedFile>(request["planFile"].clone())
                    .ok()
                    .as_ref()
                    != Some(&prior.plan_file)
            {
                return Err("identity_conflict");
            }
            return Ok(status_value(&journal));
        }
        if let Some(prior) = journal
            .publication
            .as_ref()
            .filter(|publication| publication.commit_decided)
        {
            if !same_request_binding(request, prior)? {
                return Err("identity_conflict");
            }
            if let Some(status) = recover_saved(runtime, root, capture, owner, all, prior)? {
                return Ok(status);
            }
        }
        if journal.state != "materials-ready" || journal.cancel_requested {
            return Err(
                if journal.state == "cancelled" || journal.cancel_requested {
                    "cancelled"
                } else {
                    "not_ready"
                },
            );
        }
        let incoming = request_binding(request, &journal)?;
        match journal.publication.as_ref() {
            Some(prior) if !same_binding(prior, &incoming) => return Err("identity_conflict"),
            Some(_) => (),
            None => {
                opened = Some(open_publication(runtime, root, &journal, &incoming)?);
                let _owner = runtime
                    .acquire_capture_owner(capture)
                    .map_err(|_| "internal")?
                    .ok_or("busy")?;
                journal = read_journal(root, capture, owner)?;
                if journal.state != "materials-ready" || journal.cancel_requested {
                    return Err("cancelled");
                }
                opened
                    .as_ref()
                    .ok_or("internal")?
                    ._retained
                    .validate_current_journal(
                        &serde_json::to_value(&journal).map_err(|_| "internal")?,
                    )?;
                journal.publication = Some(incoming);
                persist_owned(runtime, root, &mut journal, all)?;
            }
        }
    } else {
        let publication = journal.publication.as_ref().ok_or("not_found")?;
        if publication.publication_id != request["publicationId"].as_str().unwrap_or_default()
            || publication.plan_sha256 != request["planSha256"].as_str().unwrap_or_default()
        {
            return Err("identity_conflict");
        }
    }

    let publication = journal.publication.clone().ok_or("not_found")?;
    // Also reconcile journals written by older panic handling, which could mark
    // an already committed publication failed. The immutable binding and Core
    // evidence, never that state label, establish Saved.
    if publication.commit_decided
        && publication.state != "saved"
        && let Some(status) = recover_saved(runtime, root, capture, owner, all, &publication)?
    {
        return Ok(status);
    }
    if publication.state == "saved"
        || publication.state == "cancelled"
        || publication.state == "failed"
    {
        return Ok(status_value(&journal));
    }

    let opened = match opened {
        Some(opened) => opened,
        None => open_publication(runtime, root, &journal, &publication)?,
    };
    let _preparation = runtime
        .acquire_capture_owner(capture)
        .map_err(|_| "internal")?
        .ok_or("busy")?;
    journal = read_journal(root, capture, owner)?;
    let current = journal.publication.as_ref().ok_or("not_found")?;
    if current.state == "cancelled" || journal.cancel_requested {
        return Err("cancelled");
    }
    opened
        ._retained
        .validate_current_journal(&serde_json::to_value(&journal).map_err(|_| "internal")?)?;
    drop(_preparation);
    let prepared = opened
        .backend
        .prepare(
            &opened.context,
            &opened.plan_bytes,
            &publication.plan_sha256,
        )
        .map_err(|_| "invalid_publication")?;
    let _ = prepared;
    #[cfg(debug_assertions)]
    if let Ok(delay) = std::env::var("APPARENT_CAPTURE_TEST_PUBLICATION_COMMIT_DECISION_DELAY_MS")
        && let Ok(delay) = delay.parse::<u64>()
    {
        std::thread::sleep(std::time::Duration::from_millis(delay.min(10_000)));
    }
    let _decision = runtime
        .acquire_capture_owner(capture)
        .map_err(|_| "internal")?
        .ok_or("busy")?;
    journal = read_journal(root, capture, owner)?;
    let current = journal.publication.as_ref().ok_or("not_found")?;
    if current.publication_id != publication.publication_id
        || current.plan_sha256 != publication.plan_sha256
    {
        return Err("identity_conflict");
    }
    if current.state == "cancelled" || journal.cancel_requested {
        return Err("cancelled");
    }
    if !current.commit_decided {
        journal
            .publication
            .as_mut()
            .ok_or("not_found")?
            .commit_decided = true;
        persist_owned(runtime, root, &mut journal, all)?;
    }
    drop(_decision);
    // Inspection resumes a durably requested publish after a worker restart. It
    // never reports Saved from `prepared`; only the actual Core commit can do so.
    let evidence = opened
        .backend
        .commit(
            &opened.context,
            &opened.plan_bytes,
            &publication.plan_sha256,
        )
        .map_err(|_| "invalid_publication")?;
    #[cfg(debug_assertions)]
    if let Ok(delay) = std::env::var("APPARENT_CAPTURE_TEST_PUBLICATION_AFTER_COMMIT_DELAY_MS")
        && let Ok(delay) = delay.parse::<u64>()
    {
        std::thread::sleep(std::time::Duration::from_millis(delay.min(10_000)));
    }
    let _owner = runtime
        .acquire_capture_owner(capture)
        .map_err(|_| "internal")?
        .ok_or("busy")?;
    journal = read_journal(root, capture, owner)?;
    finish_saved(runtime, root, &mut journal, all, evidence)?;
    Ok(status_value(&journal))
}

pub(super) fn cancel(shared: &Arc<Shared>, root: &Path) -> Result<Value, &'static str> {
    let prior = shared.journal.lock().map_err(|_| "internal")?.clone();
    let _owner = shared
        .runtime
        .acquire_capture_owner(&prior.capture_id)
        .map_err(|_| "internal")?
        .ok_or("busy")?;
    let mut journal = read_journal(root, &prior.capture_id, &prior.owner_token)?;
    if let Some(publication) = journal.publication.clone() {
        if publication.state == "saved" || publication.commit_decided {
            return Ok(status_value(&journal));
        }
        if publication.state != "cancelled" && publication.state != "failed" {
            let pub_journal = journal.publication.as_mut().ok_or("internal")?;
            pub_journal.state = "cancelled".into();
        }
    }
    journal.state = "cancelled".into();
    journal.error = None;
    persist_journal_to_shared(shared, &journal)?;
    Ok(status_value(&journal))
}

fn request_binding(request: &Value, journal: &CaptureJournal) -> Result<Journal, &'static str> {
    let materials = journal.materials.as_ref().ok_or("not_ready")?;
    let key: MaterialsKey =
        serde_json::from_value(materials["key"].clone()).map_err(|_| "invalid_materials")?;
    let plan_file: SealedFile =
        serde_json::from_value(request["planFile"].clone()).map_err(|_| "malformed_request")?;
    let staging_root = request["stagingRoot"].as_str().ok_or("malformed_request")?;
    let plan_sha256 = request["planSha256"].as_str().ok_or("malformed_request")?;
    if key.capture_id != journal.capture_id
        || key.materials_id != journal.materials_id.as_deref().unwrap_or_default()
        || key.revision != journal.payload_digest.as_deref().unwrap_or_default()
        || request["materials"] != materials["key"]
        || request["leaseId"].as_str() != journal.lease_id.as_deref()
        || materials["leaseId"].as_str() != journal.lease_id.as_deref()
        || materials["root"].as_str() != Some(journal.root.as_str())
        || plan_file.sha256 != plan_sha256
        || plan_file.relative_path != "publication-plan.json"
        || !valid_id(request["publicationId"].as_str().unwrap_or_default())
        || !valid_hash(plan_sha256)
        || !absolute_normalized(staging_root)
    {
        return Err("invalid_publication");
    }
    let plan_fd = open_publication_seal(Path::new(staging_root), &plan_file)
        .map_err(|_| "source_mismatch")?;
    let plan_bytes = retained::read_bounded(&plan_fd, PLAN_LIMIT).map_err(|_| "source_mismatch")?;
    if sha(&plan_bytes) != plan_sha256 {
        return Err("source_mismatch");
    }
    let plan: Value = serde_json::from_slice(&plan_bytes).map_err(|_| "invalid_publication")?;
    if plan["publicationId"].as_str() != request["publicationId"].as_str()
        || plan["materials"] != materials["key"]
        || plan["stagingRoot"].as_str() != Some(staging_root)
        || plan["bundleId"].as_str() != journal.request["bundleId"].as_str()
        || plan["destination"]["path"].as_str() != journal.request["bundlePath"].as_str()
        || plan["destination"]["mode"].as_str() != Some("absent")
    {
        return Err("identity_conflict");
    }
    let manifest_sha256 = plan["entries"]
        .as_array()
        .and_then(|entries| {
            entries.iter().find(|entry| {
                entry["relativePath"] == "apparent.json" && entry["role"] == "manifest"
            })
        })
        .and_then(|entry| entry["sha256"].as_str())
        .ok_or("invalid_publication")?
        .to_owned();
    if !valid_hash(&manifest_sha256) {
        return Err("invalid_publication");
    }
    Ok(Journal {
        publication_id: request["publicationId"].as_str().unwrap_or_default().into(),
        plan_sha256: plan_sha256.into(),
        staging_root: staging_root.into(),
        plan_file,
        materials: key,
        lease_id: journal.lease_id.clone().ok_or("invalid_materials")?,
        bundle_id: journal.request["bundleId"]
            .as_str()
            .ok_or("invalid_materials")?
            .into(),
        bundle_path: journal.request["bundlePath"]
            .as_str()
            .ok_or("invalid_materials")?
            .into(),
        manifest_sha256,
        // `publishing` is the existing capture/2 public observation. Keep the
        // durable binding retryable until Core returns committed evidence.
        state: "publishing".into(),
        commit_decided: false,
        evidence: None,
    })
}

fn same_binding(left: &Journal, right: &Journal) -> bool {
    left.publication_id == right.publication_id
        && left.plan_sha256 == right.plan_sha256
        && left.staging_root == right.staging_root
        && left.plan_file == right.plan_file
        && left.materials == right.materials
        && left.lease_id == right.lease_id
        && left.bundle_id == right.bundle_id
        && left.bundle_path == right.bundle_path
}

fn same_request_binding(request: &Value, prior: &Journal) -> Result<bool, &'static str> {
    Ok(
        request["publicationId"].as_str() == Some(prior.publication_id.as_str())
            && request["planSha256"].as_str() == Some(prior.plan_sha256.as_str())
            && request["materials"]
                == serde_json::to_value(&prior.materials).map_err(|_| "internal")?
            && request["leaseId"].as_str() == Some(prior.lease_id.as_str())
            && request["stagingRoot"].as_str() == Some(prior.staging_root.as_str())
            && serde_json::from_value::<SealedFile>(request["planFile"].clone())
                .ok()
                .as_ref()
                == Some(&prior.plan_file),
    )
}

/// Try Core's committed-only recovery before opening any caller source. This is
/// used only after the provider's authenticated durable record says the explicit
/// commit decision was reached; a missing Core commit falls through to the
/// ordinary source-validated preparation path.
fn recover_saved(
    runtime: &Runtime,
    root: &Path,
    capture: &str,
    owner: &str,
    all: &mut std::collections::HashMap<String, Arc<Shared>>,
    publication: &Journal,
) -> Result<Option<Value>, &'static str> {
    let destination_parent_path = Path::new(&publication.bundle_path)
        .parent()
        .ok_or("invalid_publication")?;
    let destination_parent = PublicationBackend::open_destination_parent(destination_parent_path)
        .map_err(|_| "source_mismatch")?;
    let control =
        Arc::new(SecureStore::ensure(&root.join("publication-core")).map_err(|_| "internal")?);
    let original = PublicationRecoveryBinding {
        publication_id: publication.publication_id.clone(),
        plan_sha256: publication.plan_sha256.clone(),
        materials: publication.materials.clone(),
        lease_id: publication.lease_id.clone(),
        bundle_id: publication.bundle_id.clone(),
        destination: publication.bundle_path.clone(),
        manifest_sha256: publication.manifest_sha256.clone(),
    };
    let backend = PublicationBackend::new(control);
    let Some(evidence) = backend
        .inspect_committed(&original, destination_parent)
        .map_err(|_| "invalid_publication")?
    else {
        return Ok(None);
    };

    let _owner = runtime
        .acquire_capture_owner(capture)
        .map_err(|_| "internal")?
        .ok_or("busy")?;
    let mut journal = read_journal(root, capture, owner)?;
    let current = journal.publication.as_ref().ok_or("not_found")?;
    if !current.commit_decided || !same_binding(current, publication) {
        return Err("identity_conflict");
    }
    if current.state == "saved" {
        return Ok(Some(status_value(&journal)));
    }
    finish_saved(runtime, root, &mut journal, all, evidence)?;
    Ok(Some(status_value(&journal)))
}

fn open_publication(
    runtime: &Runtime,
    root: &Path,
    capture: &CaptureJournal,
    publication: &Journal,
) -> Result<Opened, &'static str> {
    let mid = capture.materials_id.as_deref().ok_or("not_ready")?;
    let retained = retained::pin_retained_materials(
        runtime,
        root,
        &capture.capture_id,
        &capture.owner_token,
        &retained::MaterialKey {
            capture_id: publication.materials.capture_id.clone(),
            materials_id: publication.materials.materials_id.clone(),
            revision: publication.materials.revision.clone(),
        },
        &publication.lease_id,
    );
    let retained = match retained {
        Ok(source) => source,
        Err(error) => {
            // A source pin can race an authenticated cancel after our earlier
            // snapshot. Abort with that durable outcome; never resume or accept
            // a source whose pin failed. Other source/authority errors survive.
            let current = read_journal(root, &capture.capture_id, &capture.owner_token)?;
            if current.state == "cancelled" || current.cancel_requested {
                return Err("cancelled");
            }
            return Err(error);
        }
    };
    validate_manifest_binding(
        capture,
        &retained.materials_manifest,
        mid,
        &publication.materials,
    )?;
    validate_payload_binding(capture, &retained.payload, publication)?;
    let material_root = retained.root_path.clone();
    let material_files = retained.files.iter().map(core_seal).collect::<Vec<_>>();
    let payload = &retained.payload;
    let stage_store =
        SecureStore::open(Path::new(&publication.staging_root)).map_err(|_| "source_mismatch")?;
    let plan_file =
        open_publication_seal(Path::new(&publication.staging_root), &publication.plan_file)
            .map_err(|_| "source_mismatch")?;
    let plan_bytes =
        retained::read_bounded(&plan_file, PLAN_LIMIT).map_err(|_| "source_mismatch")?;
    if sha(&plan_bytes) != publication.plan_sha256 {
        return Err("source_mismatch");
    }
    let plan: Value = serde_json::from_slice(&plan_bytes).map_err(|_| "invalid_publication")?;
    validate_plan_binding(&plan, publication)?;
    let mut media_registry = None;
    let mut sources = Vec::new();
    let mut seen = HashSet::new();
    for entry in plan["entries"].as_array().ok_or("invalid_publication")? {
        let (source_key, source_root, seal) = if entry["input"]["kind"] == "derivative2" {
            if media_registry.is_none() {
                let runtime_root = root.parent().ok_or("internal")?;
                let store =
                    Arc::new(SecureStore::open(runtime_root).map_err(|_| "source_mismatch")?);
                media_registry =
                    Some(MediaJobRegistry::open(store).map_err(|_| "source_mismatch")?);
            }
            resolve_derivative2_source(
                entry,
                media_registry.as_ref().ok_or("internal")?,
                &retained,
            )?
        } else {
            source_binding(entry, &payload, &material_root, &publication.staging_root)?
        };
        if seen.insert(source_key.clone()) {
            let file = if source_key.starts_with("material:") {
                retained.open_material_file(&seal.relative_path)?
            } else {
                open_publication_seal(&source_root, &seal).map_err(|_| "source_mismatch")?
            };
            retained::validate_open_identity(&file, &retained_identity(&seal))
                .map_err(|_| "source_mismatch")?;
            sources.push(TrustedSource {
                source_key,
                file,
                seal,
            });
        }
    }
    stage_store
        .directory_file()
        .map_err(|_| "source_mismatch")?;
    retained.root.check().map_err(|_| "source_mismatch")?;
    let destination = plan["destination"]["path"]
        .as_str()
        .ok_or("invalid_publication")?;
    let destination_parent_path = Path::new(destination)
        .parent()
        .ok_or("invalid_publication")?;
    let destination_parent = PublicationBackend::open_destination_parent(destination_parent_path)
        .map_err(|_| "source_mismatch")?;
    let control =
        Arc::new(SecureStore::ensure(&root.join("publication-core")).map_err(|_| "internal")?);
    let backend = PublicationBackend::new(control);
    Ok(Opened {
        backend,
        context: TrustedPublicationContext {
            publication_id: publication.publication_id.clone(),
            materials: publication.materials.clone(),
            lease_id: publication.lease_id.clone(),
            bundle_id: publication.bundle_id.clone(),
            staging_root: publication.staging_root.clone(),
            destination: publication.bundle_path.clone(),
            material_files,
            sources,
            destination_parent,
        },
        plan_bytes,
        _retained: retained,
    })
}

fn retained_identity(seal: &SealedFile) -> retained::MaterialFileIdentity {
    retained::MaterialFileIdentity {
        device: seal.identity.device.clone(),
        inode: seal.identity.inode.clone(),
        size: seal.identity.size,
        modified_ns: seal.identity.modified_ns.clone(),
    }
}

fn core_seal(seal: &retained::MaterialFileSeal) -> SealedFile {
    SealedFile {
        relative_path: seal.relative_path.clone(),
        identity: apparent_capture_engine::publication::FileIdentity {
            device: seal.identity.device.clone(),
            inode: seal.identity.inode.clone(),
            size: seal.identity.size,
            modified_ns: seal.identity.modified_ns.clone(),
        },
        sha256: seal.sha256.clone(),
    }
}

fn open_publication_seal(root: &Path, seal: &SealedFile) -> Result<std::fs::File, &'static str> {
    let file = retained::open_sealed(root, &seal.relative_path)?;
    retained::validate_open_identity(&file, &retained_identity(seal))?;
    Ok(file)
}

fn source_binding(
    entry: &Value,
    payload: &Value,
    material_root: &Path,
    staging_root: &str,
) -> Result<(String, PathBuf, SealedFile), &'static str> {
    let input = &entry["input"];
    match input["kind"].as_str().ok_or("invalid_publication")? {
        "material" => {
            let path = input["relativePath"]
                .as_str()
                .ok_or("invalid_publication")?;
            let seal_value = payload["files"]
                .as_array()
                .and_then(|files| files.iter().find(|f| f["relativePath"] == path))
                .ok_or("invalid_materials")?;
            let seal: SealedFile =
                serde_json::from_value(seal_value.clone()).map_err(|_| "invalid_materials")?;
            Ok((format!("material:{path}"), material_root.to_owned(), seal))
        }
        "assembled" => {
            let seal: SealedFile =
                serde_json::from_value(input["file"].clone()).map_err(|_| "invalid_publication")?;
            let path = seal.relative_path.clone();
            Ok((
                format!("assembled:{path}"),
                PathBuf::from(staging_root),
                seal,
            ))
        }
        // The derivative envelope is caller-authored and has no committed
        // Engine media-job authority. Do not classify it as native output.
        "derivative" => Err("unavailable"),
        _ => Err("invalid_publication"),
    }
}

/// Authenticate derivative/2 provenance from the original capture-bound job
/// registry. Caller-supplied envelope bytes and seals are correlation data only.
fn resolve_derivative2_source(
    entry: &Value,
    registry: &MediaJobRegistry,
    retained: &RetainedMaterialSource,
) -> Result<(String, PathBuf, SealedFile), &'static str> {
    let input = entry["input"].as_object().ok_or("invalid_publication")?;
    if input.len() != 3 || input.get("kind").and_then(Value::as_str) != Some("derivative2") {
        return Err("invalid_publication");
    }
    let supplied = input.get("receipt").ok_or("invalid_publication")?;
    let envelope = supplied.as_object().ok_or("invalid_publication")?;
    let materials = &envelope["materials"];
    let capture_id = envelope["materials"]["captureId"]
        .as_str()
        .ok_or("invalid_publication")?;
    let materials_id = envelope["materials"]["materialsId"]
        .as_str()
        .ok_or("invalid_publication")?;
    let revision = envelope["materials"]["revision"]
        .as_str()
        .ok_or("invalid_publication")?;
    let lease_id = envelope["leaseId"].as_str().ok_or("invalid_publication")?;
    let job_id = envelope["jobId"].as_str().ok_or("invalid_publication")?;
    let receipt_id = envelope["receiptId"]
        .as_str()
        .ok_or("invalid_publication")?;
    let request_sha256 = envelope["requestSha256"]
        .as_str()
        .ok_or("invalid_publication")?;
    if envelope.len() != 9
        || envelope["schema"].as_str() != Some("apparent.media-derivative/2")
        || !valid_id(capture_id)
        || !valid_id(materials_id)
        || !valid_hash(revision)
        || !valid_id(lease_id)
        || !valid_id(job_id)
        || !valid_id(receipt_id)
        || !valid_hash(request_sha256)
        || capture_id != retained.capture_id
        || materials_id != retained.materials.materials_id
        || revision != retained.materials.revision
        || *materials != serde_json::to_value(&retained.materials).map_err(|_| "internal")?
        || lease_id != retained.lease_id
    {
        return Err("identity_conflict");
    }

    // `open_publication` is called only after the capture owner was
    // authenticated and the retained source read lease was acquired.
    let stored = registry
        .lookup_authenticated(capture_id, job_id, request_sha256)
        .map_err(|_| "invalid_publication")?;
    if stored.state != StoredJobState::Complete {
        return Err("unavailable");
    }
    let binding = stored.binding.as_ref().ok_or("invalid_publication")?;
    let native_receipt = stored.receipt.as_ref().ok_or("invalid_publication")?;
    let native_envelope = stored.envelope.as_ref().ok_or("invalid_publication")?;
    let root_path = PathBuf::from(&binding.output_root);
    let retained_root = retained.root_path.to_str().ok_or("source_mismatch")?;
    if stored.intent.capture_id != retained.capture_id
        || stored.intent.materials.capture_id != retained.materials.capture_id
        || stored.intent.materials.materials_id != retained.materials.materials_id
        || stored.intent.materials.revision != retained.materials.revision
        || stored.intent.lease_id != retained.lease_id
        || stored.intent.job_id != job_id
        || stored.intent.request_sha256 != request_sha256
        || binding.schema != "apparent.media2-job/1"
        || binding.capture_id != retained.capture_id
        || binding.materials.capture_id != capture_id
        || binding.materials.materials_id != materials_id
        || binding.materials.revision != revision
        || binding.lease_id != retained.lease_id
        || binding.job_id != job_id
        || binding.request_sha256 != request_sha256
        || binding.materials_root != retained_root
        || binding.materials_root_identity.device != retained.root_device.to_string()
        || binding.materials_root_identity.inode != retained.root_inode.to_string()
        || serde_json::to_value(&binding.source_video).map_err(|_| "internal")?
            != serde_json::to_value(core_seal(&retained.video)).map_err(|_| "internal")?
        || !absolute_normalized(&binding.output_root)
        || envelope["root"].as_str() != Some(binding.output_root.as_str())
        || envelope["receiptFile"]
            != serde_json::to_value(&native_receipt.payload_file).map_err(|_| "internal")?
        || native_receipt.capture_id != capture_id
        || native_receipt.materials.capture_id != capture_id
        || native_receipt.materials.materials_id != materials_id
        || native_receipt.materials.revision != revision
        || native_receipt.lease_id != retained.lease_id
        || native_receipt.job_id != job_id
        || native_receipt.receipt_id != receipt_id
        || native_receipt.request_sha256 != request_sha256
        || sha(native_envelope) != native_receipt.envelope_sha256
    {
        return Err("identity_conflict");
    }

    let output = Arc::new(SecureStore::open(&root_path).map_err(|_| "source_mismatch")?);
    let (verified_receipt, verified_envelope) = registry
        .verify_completed(binding, output.clone())
        .map_err(|_| "invalid_publication")?;
    if serde_json::to_vec(supplied).map_err(|_| "invalid_publication")? != verified_envelope
        || verified_envelope != *native_envelope
        || serde_json::to_value(&verified_receipt).map_err(|_| "internal")?
            != serde_json::to_value(native_receipt).map_err(|_| "internal")?
    {
        return Err("identity_conflict");
    }

    let payload = envelope["payload"]
        .as_object()
        .ok_or("invalid_publication")?;
    if payload.len() != 9
        || payload["schema"].as_str() != Some("apparent.media-derivative-payload/2")
        || payload["materials"] != *materials
        || payload["leaseId"].as_str() != Some(retained.lease_id.as_str())
        || payload["jobId"].as_str() != Some(job_id)
        || payload["receiptId"].as_str() != Some(receipt_id)
        || payload["root"].as_str() != Some(binding.output_root.as_str())
        || payload["requestSha256"].as_str() != Some(request_sha256)
        || payload["frames"].as_array().is_none()
    {
        return Err("identity_conflict");
    }

    let selector = input
        .get("selector")
        .and_then(Value::as_object)
        .ok_or("invalid_publication")?;
    let (seal, expected_role) = match selector.get("kind").and_then(Value::as_str) {
        Some("frame") if selector.len() == 2 => {
            let index = selector
                .get("requestIndex")
                .and_then(Value::as_u64)
                .filter(|index| *index > 0)
                .ok_or("invalid_publication")?;
            let Some(native_seal) = native_receipt.frames.get((index - 1) as usize) else {
                return Err("invalid_publication");
            };
            let frames = payload["frames"].as_array().ok_or("invalid_publication")?;
            let Some(frame) = frames.get((index - 1) as usize) else {
                return Err("invalid_publication");
            };
            if frame["requestIndex"].as_u64() != Some(index)
                || frame["file"] != serde_json::to_value(native_seal).map_err(|_| "internal")?
            {
                return Err("identity_conflict");
            }
            (native_seal.clone(), "frame")
        }
        Some("contact-sheet") if selector.len() == 1 => {
            let native_sheet = native_receipt.sheet.as_ref().ok_or("invalid_publication")?;
            let sheet = payload["contactSheet"]
                .as_object()
                .ok_or("invalid_publication")?;
            if sheet["file"] != serde_json::to_value(&native_sheet.file).map_err(|_| "internal")?
                || sheet["layoutSha256"].as_str() != Some(native_sheet.layout_hash.as_str())
                || Some(&sheet["layout"])
                    != binding
                        .layout
                        .as_ref()
                        .map(serde_json::to_value)
                        .transpose()
                        .map_err(|_| "internal")?
                        .as_ref()
            {
                return Err("identity_conflict");
            }
            (native_sheet.file.clone(), "contact-sheet")
        }
        _ => return Err("invalid_publication"),
    };
    if entry["role"].as_str() != Some(expected_role)
        || entry["sha256"].as_str() != Some(seal.sha256.as_str())
        || entry["byteLength"].as_u64() != Some(seal.identity.size)
    {
        return Err("identity_conflict");
    }
    let core_seal: SealedFile =
        serde_json::from_value(serde_json::to_value(seal).map_err(|_| "internal")?)
            .map_err(|_| "invalid_publication")?;
    let source_key = format!("derivative:{receipt_id}:{}", core_seal.relative_path);
    Ok((source_key, root_path, core_seal))
}

fn validate_manifest_binding(
    capture: &CaptureJournal,
    manifest: &Value,
    mid: &str,
    key: &MaterialsKey,
) -> Result<(), &'static str> {
    if manifest["schema"].as_str() != Some("apparent.capture-materials/1")
        || manifest["key"]["captureId"].as_str() != Some(capture.capture_id.as_str())
        || manifest["key"]["materialsId"].as_str() != Some(mid)
        || manifest["key"]["revision"].as_str()
            != Some(capture.payload_digest.as_deref().unwrap_or_default())
        || manifest["key"] != serde_json::to_value(key).map_err(|_| "invalid_materials")?
        || manifest["leaseId"].as_str() != capture.lease_id.as_deref()
        || manifest["root"].as_str() != Some(capture.root.as_str())
    {
        return Err("invalid_materials");
    }
    Ok(())
}

fn validate_payload_binding(
    capture: &CaptureJournal,
    payload: &Value,
    publication: &Journal,
) -> Result<(), &'static str> {
    if payload["schema"].as_str() != Some("apparent.capture-materials-payload/1")
        || payload["identity"]["captureId"].as_str() != Some(capture.capture_id.as_str())
        || payload["identity"]["materialsId"].as_str()
            != Some(publication.materials.materials_id.as_str())
        || payload["root"].as_str() != Some(capture.root.as_str())
        || payload["leaseId"].as_str() != Some(publication.lease_id.as_str())
        || payload["bundleId"].as_str() != Some(publication.bundle_id.as_str())
        || payload["safety"]["safeForProcessing"] != true
        || payload["files"].as_array().is_none()
    {
        return Err("invalid_materials");
    }
    let movie = payload["files"]
        .as_array()
        .and_then(|files| {
            files
                .iter()
                .find(|file| file["relativePath"] == "recording.mp4")
        })
        .ok_or("invalid_materials")?;
    if Some(&movie["identity"]) != capture.release_movie.as_ref()
        || payload["video"]["file"].as_str() != Some("recording.mp4")
    {
        return Err("invalid_materials");
    }
    Ok(())
}

fn validate_plan_binding(plan: &Value, publication: &Journal) -> Result<(), &'static str> {
    if plan["schema"].as_str() != Some("apparent.bundle-publication/1")
        || plan["publicationId"].as_str() != Some(publication.publication_id.as_str())
        || plan["materials"]
            != serde_json::to_value(&publication.materials).map_err(|_| "invalid_publication")?
        || plan["bundleId"].as_str() != Some(publication.bundle_id.as_str())
        || plan["stagingRoot"].as_str() != Some(publication.staging_root.as_str())
        || plan["destination"]["path"].as_str() != Some(publication.bundle_path.as_str())
        || plan["destination"]["mode"].as_str() != Some("absent")
        || plan["entries"].as_array().is_none()
    {
        return Err("invalid_publication");
    }
    let manifest = plan["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["relativePath"] == "apparent.json")
        .ok_or("invalid_publication")?;
    if manifest["sha256"].as_str() != Some(publication.manifest_sha256.as_str()) {
        return Err("identity_conflict");
    }
    Ok(())
}

fn finish_saved(
    runtime: &Runtime,
    root: &Path,
    journal: &mut CaptureJournal,
    all: &mut std::collections::HashMap<String, Arc<Shared>>,
    evidence: PublicationEvidence,
) -> Result<(), &'static str> {
    let p = journal.publication.as_mut().ok_or("internal")?;
    if evidence.publication_id != p.publication_id
        || evidence.plan_sha256 != p.plan_sha256
        || evidence.materials != p.materials
        || evidence.lease_id != p.lease_id
        || evidence.bundle_id != p.bundle_id
        || evidence.destination != p.bundle_path
        || evidence.manifest_sha256 != p.manifest_sha256
    {
        return Err("identity_conflict");
    }
    p.state = "saved".into();
    p.evidence = Some(Receipt {
        capture_id: journal.capture_id.clone(),
        materials: evidence.materials,
        publication_id: evidence.publication_id,
        plan_sha256: evidence.plan_sha256,
        bundle_id: evidence.bundle_id,
        bundle_path: evidence.destination,
        artifact_version: 1,
        artifact_provenance: "native".into(),
        manifest_sha256: evidence.manifest_sha256,
        inventory_sha256: evidence.inventory_sha256,
    });
    persist_record(runtime, root, journal, all)
}

fn persist_owned(
    runtime: &Runtime,
    root: &Path,
    journal: &mut CaptureJournal,
    all: &mut std::collections::HashMap<String, Arc<Shared>>,
) -> Result<(), &'static str> {
    persist_record(runtime, root, journal, all)
}

fn persist_record(
    runtime: &Runtime,
    root: &Path,
    journal: &CaptureJournal,
    all: &mut std::collections::HashMap<String, Arc<Shared>>,
) -> Result<(), &'static str> {
    let shared = all.get(&journal.capture_id).cloned().unwrap_or_else(|| {
        Arc::new(Shared {
            journal: Mutex::new(journal.clone()),
            directory: root.to_owned(),
            runtime: runtime.clone(),
            stop: std::sync::atomic::AtomicBool::new(false),
            cancel: std::sync::atomic::AtomicBool::new(false),
            epoch: std::time::Instant::now(),
        })
    });
    *shared.journal.lock().map_err(|_| "internal")? = journal.clone();
    persist(&shared).map_err(|_| "internal")?;
    let durable = read_journal(root, &journal.capture_id, &journal.owner_token)?;
    *shared.journal.lock().map_err(|_| "internal")? = durable;
    all.insert(journal.capture_id.clone(), shared);
    Ok(())
}

fn persist_journal_to_shared(
    shared: &Arc<Shared>,
    journal: &CaptureJournal,
) -> Result<(), &'static str> {
    *shared.journal.lock().map_err(|_| "internal")? = journal.clone();
    persist(shared).map_err(|_| "internal")?;
    *shared.journal.lock().map_err(|_| "internal")? =
        read_journal(&shared.directory, &journal.capture_id, &journal.owner_token)?;
    Ok(())
}

fn status_value(journal: &CaptureJournal) -> Value {
    let mut value = json!({"captureId":journal.capture_id,"state":journal.state});
    if let Some(materials) = &journal.materials {
        value["materials"] = materials.clone();
    }
    if let Some(publication) = &journal.publication {
        status_fields(&mut value, publication);
    }
    value
}

fn valid_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn absolute_normalized(value: &str) -> bool {
    let path = Path::new(value);
    path.is_absolute()
        && path.to_str() == Some(value)
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::Read,
        os::unix::fs::{MetadataExt, PermissionsExt},
        time::{SystemTime, UNIX_EPOCH},
    };

    const FRAME: &[u8] = include_bytes!("../../core/tests/fixtures/frame.png");
    const MOVIE: &[u8] = include_bytes!("../../core/tests/fixtures/tiny.mp4");

    struct Fixture {
        base: PathBuf,
        runtime_path: PathBuf,
        capture_root: PathBuf,
        runtime: Runtime,
        request: Value,
        shared: Arc<Shared>,
        destination: PathBuf,
        owner: String,
        capture_id: String,
    }

    fn private_dir(path: &Path) {
        fs::create_dir_all(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn temp() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "capture-pub-provider-{}-{stamp}",
            std::process::id()
        ));
        private_dir(&path);
        path
    }

    fn sealed(store: &SecureStore, root: &Path, relative_path: &str) -> SealedFile {
        let path = root.join(relative_path);
        let parent = SecureStore::open(path.parent().unwrap()).unwrap();
        let file = parent
            .file(path.file_name().unwrap().to_str().unwrap(), false)
            .unwrap();
        let metadata = file.metadata().unwrap();
        let mut bytes = Vec::new();
        file.try_clone().unwrap().read_to_end(&mut bytes).unwrap();
        let _ = store;
        SealedFile {
            relative_path: relative_path.into(),
            identity: apparent_capture_engine::publication::FileIdentity {
                device: metadata.dev().to_string(),
                inode: metadata.ino().to_string(),
                size: metadata.len(),
                modified_ns: (metadata.mtime() as i128 * 1_000_000_000
                    + metadata.mtime_nsec() as i128)
                    .to_string(),
            },
            sha256: sha(&bytes),
        }
    }

    fn write(store: &SecureStore, name: &str, bytes: &[u8]) {
        store.write_new(name, bytes).unwrap();
    }

    #[cfg(feature = "media")]
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct RetainedRequest {
        schema: &'static str,
        materials: super::super::materials_media::MaterialsKey,
        lease_id: String,
        job_id: String,
        output_directory: String,
        requested_bundle_ms: Vec<u64>,
        limits: RetainedLimits,
        contact_sheet: Option<super::super::materials_media::contact_sheet::Layout>,
    }

    #[cfg(feature = "media")]
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct RetainedLimits {
        max_frames: u64,
        max_edge: u32,
        max_total_pixels: u64,
        deadline_ms: u64,
    }

    #[cfg(feature = "media")]
    struct RegistrySink(Arc<SecureStore>);

    #[cfg(feature = "media")]
    impl apparent_capture_engine::materials_media::DerivativeOutputSink for RegistrySink {
        fn write_new_synced(
            &mut self,
            relative_path: &str,
            bytes: &[u8],
        ) -> anyhow::Result<apparent_capture_engine::materials_media::SealedFile> {
            let (store, leaf) = if let Some(leaf) = relative_path.strip_prefix("frames/") {
                (self.0.child("frames")?, leaf.to_owned())
            } else {
                anyhow::ensure!(
                    relative_path == "receipt.json" || relative_path == "contact-sheet.png"
                );
                (self.0.clone(), relative_path.to_owned())
            };
            store.write_new(&leaf, bytes)?;
            let mut file = store.file(&leaf, false)?;
            let metadata = file.metadata()?;
            let mut contents = Vec::new();
            file.read_to_end(&mut contents)?;
            Ok(apparent_capture_engine::materials_media::SealedFile {
                relative_path: relative_path.into(),
                identity: apparent_capture_engine::materials_media::FileIdentity {
                    device: metadata.dev().to_string(),
                    inode: metadata.ino().to_string(),
                    size: metadata.len(),
                    modified_ns: (i128::from(metadata.mtime()) * 1_000_000_000
                        + i128::from(metadata.mtime_nsec()))
                    .to_string(),
                },
                sha256: sha(&contents),
            })
        }
    }

    #[cfg(feature = "media")]
    fn replace_plan(f: &Fixture, plan: &Value) -> Value {
        let staging_root = Path::new(f.request["stagingRoot"].as_str().unwrap());
        let bytes = serde_json::to_vec(plan).unwrap();
        SecureStore::open(staging_root)
            .unwrap()
            .replace_checked("publication-plan.json", &bytes)
            .unwrap();
        let plan_file = sealed(
            &SecureStore::open(staging_root).unwrap(),
            staging_root,
            "publication-plan.json",
        );
        let mut request = f.request.clone();
        request["planFile"] = serde_json::to_value(plan_file).unwrap();
        request["planSha256"] = Value::String(sha(&bytes));
        request
    }

    fn fixture() -> Fixture {
        let base = temp();
        let runtime_path = base.join("runtime");
        private_dir(&runtime_path);
        let runtime = Runtime::open(&runtime_path).unwrap();
        let capture_root = runtime_path.join("capture-v2");
        let captures = SecureStore::ensure(&capture_root).unwrap();
        let material_parent = capture_root.join("materials");
        SecureStore::ensure(&material_parent).unwrap();

        let capture_id = apparent_capture_engine::new_id().unwrap();
        let materials_id = apparent_capture_engine::new_id().unwrap();
        let lease_id = apparent_capture_engine::new_id().unwrap();
        let publication_id = apparent_capture_engine::new_id().unwrap();
        let owner = apparent_capture_engine::new_owner_token().unwrap();
        let bundle_id = "capture-provider-fixture.apparent";
        let material_root = material_parent.join(&capture_id);
        let materials = SecureStore::create(&material_root).unwrap();
        write(&materials, "recording.mp4", MOVIE);
        let movie = sealed(&materials, &material_root, "recording.mp4");
        let payload = json!({
            "schema":"apparent.capture-materials-payload/1",
            "identity":{"captureId":capture_id,"materialsId":materials_id},
            "bundleId":bundle_id,
            "root":material_root,
            "leaseId":lease_id,
            "clock":{"durationMs":1,"videoOffsetMs":0},
            "files":[movie],
            "video":{"file":"recording.mp4","width":16,"height":16,"durationMs":1},
            "safety":{"safeForProcessing":true}
        });
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        write(&materials, "materials.json", &payload_bytes);
        let payload_file = sealed(&materials, &material_root, "materials.json");
        let revision = sha(&payload_bytes);
        let key = MaterialsKey {
            capture_id: capture_id.clone(),
            materials_id: materials_id.clone(),
            revision: revision.clone(),
        };
        let manifest = json!({"schema":"apparent.capture-materials/1","key":key,"root":material_root,"leaseId":lease_id,"payloadFile":payload_file});
        let root_metadata = materials.directory_metadata().unwrap();
        let start_path = base.join("bundles").join(bundle_id);
        private_dir(start_path.parent().unwrap());
        let staging_root = base.join("record-stage").join(&publication_id);
        private_dir(staging_root.parent().unwrap());
        private_dir(&staging_root);
        let stage = SecureStore::open(&staging_root).unwrap();
        write(&stage, ".record-publication", b"private Record control");
        write(&stage, "summary.md", b"# Capture\n");
        write(&stage, "contact-sheet.png", FRAME);
        let summary = sealed(&stage, &staging_root, "summary.md");
        let contact = sealed(&stage, &staging_root, "contact-sheet.png");
        let manifest_value = json!({
            "version":1,"id":bundle_id,"createdAtMs":1,"durationMs":1,
            "target":{"mode":"display","displayId":1,"frame":{"x":0,"y":0,"width":16,"height":16,"scale":1}},
            "transcript":{"source":"none","text":"","segments":[]},
            "strokes":[],"marks":[],"events":[],
            "keyframes":[{"index":1,"t":0,"reason":"initial","file":"frames/frame-01.png","event":"start"}],
            "contactSheet":"contact-sheet.png","video":"recording.mp4"
        });
        let manifest_bytes = serde_json::to_vec(&manifest_value).unwrap();
        write(&stage, "apparent.json", &manifest_bytes);
        let bundle_manifest = sealed(&stage, &staging_root, "apparent.json");
        let plan = json!({
            "schema":"apparent.bundle-publication/1","publicationId":publication_id,"materials":key,
            "bundleId":bundle_id,"stagingRoot":staging_root,
            "destination":{"path":start_path,"mode":"absent"},"artifactVersion":1,
            "entries":[
                {"relativePath":"apparent.json","sha256":bundle_manifest.sha256,"byteLength":bundle_manifest.identity.size,"role":"manifest","input":{"kind":"assembled","file":bundle_manifest}},
                {"relativePath":"summary.md","sha256":summary.sha256,"byteLength":summary.identity.size,"role":"summary","input":{"kind":"assembled","file":summary}},
                {"relativePath":"frames/frame-01.png","sha256":contact.sha256,"byteLength":contact.identity.size,"role":"frame","input":{"kind":"assembled","file":contact}},
                {"relativePath":"contact-sheet.png","sha256":contact.sha256,"byteLength":contact.identity.size,"role":"contact-sheet","input":{"kind":"assembled","file":contact}},
                {"relativePath":"recording.mp4","sha256":movie.sha256,"byteLength":movie.identity.size,"role":"video","input":{"kind":"material","relativePath":"recording.mp4"}}
            ],"manifestPath":"apparent.json","summaryPath":"summary.md","profile":"apparent-bundle/1"
        });
        let plan_bytes = serde_json::to_vec(&plan).unwrap();
        write(&stage, "publication-plan.json", &plan_bytes);
        let plan_file = sealed(&stage, &staging_root, "publication-plan.json");
        let plan_sha256 = sha(&plan_bytes);
        let capture_journal = CaptureJournal {
            capture_id: capture_id.clone(),
            owner_token: owner.clone(),
            request: json!({"captureId":capture_id,"ownerToken":owner,"bundleId":bundle_id,"bundlePath":start_path}),
            capture_backend: None,
            state: "materials-ready".into(),
            error: None,
            materials: Some(manifest),
            root: material_root.to_string_lossy().into_owned(),
            materials_id: Some(materials_id),
            lease_id: Some(lease_id.clone()),
            root_device: Some(root_metadata.dev()),
            root_inode: Some(root_metadata.ino()),
            release_phase: None,
            release_movie: Some(serde_json::to_value(&movie.identity).unwrap()),
            release_payload: Some(serde_json::to_value(&payload_file.identity).unwrap()),
            partial_movie: None,
            partial_audio: None,
            release_audio: None,
            audio_clock: None,
            payload_digest: Some(revision),
            ink_registration: None,
            ink_cutoff_ns: None,
            fact_files: Default::default(),
            admitted_monotonic_ns: None,
            confirmed_at_ms: None,
            cancel_requested: false,
            publication: None,
        };
        captures
            .write_new(
                &format!("{capture_id}.json"),
                &serde_json::to_vec(&capture_journal).unwrap(),
            )
            .unwrap();
        let shared = Arc::new(Shared {
            journal: Mutex::new(capture_journal),
            directory: capture_root.clone(),
            runtime: runtime.clone(),
            stop: std::sync::atomic::AtomicBool::new(false),
            cancel: std::sync::atomic::AtomicBool::new(false),
            epoch: std::time::Instant::now(),
        });
        Fixture {
            base,
            runtime_path,
            capture_root,
            runtime,
            request: json!({"method":"publish","captureId":capture_id,"ownerToken":owner,"leaseId":lease_id,
                "publicationId":publication_id,"materials":key,"stagingRoot":staging_root,"planFile":plan_file,"planSha256":plan_sha256}),
            shared,
            destination: start_path,
            owner,
            capture_id,
        }
    }

    fn call(fixture: &Fixture, request: &Value) -> Result<Value, &'static str> {
        handle(
            request,
            &fixture.runtime,
            &fixture.capture_root,
            &mut Default::default(),
        )
    }

    #[test]
    fn authenticated_publish_snapshots_only_declared_inventory_and_replays_saved() {
        let f = fixture();
        let saved = call(&f, &f.request).unwrap();
        assert_eq!(saved["state"], "saved");
        assert_eq!(saved["publication"]["artifactProvenance"], "native");
        assert_eq!(
            saved["publication"]["bundlePath"],
            f.destination.to_string_lossy().as_ref()
        );
        assert!(f.destination.join("frames/frame-01.png").is_file());
        assert!(f.destination.join("recording.mp4").is_file());
        assert!(!f.destination.join(".record-publication").exists());
        assert!(!f.destination.join("publication-plan.json").exists());
        assert_eq!(
            cancel(&f.shared, &f.capture_root).unwrap()["state"],
            "saved"
        );
        let inspect = json!({"method":"inspectPublication","captureId":f.capture_id,"ownerToken":f.owner,
            "publicationId":f.request["publicationId"],"planSha256":f.request["planSha256"]});
        assert_eq!(call(&f, &inspect).unwrap(), saved);
        let mut unauthenticated = inspect.clone();
        unauthenticated["ownerToken"] = Value::String("f".repeat(64));
        unauthenticated["publicationId"] = Value::String("0".repeat(32));
        assert_eq!(call(&f, &unauthenticated).unwrap_err(), "permission_denied");
        let mut conflict = inspect;
        conflict["publicationId"] = Value::String("0".repeat(32));
        assert_eq!(call(&f, &conflict).unwrap_err(), "identity_conflict");
        fs::remove_dir_all(f.base).unwrap();
    }

    #[test]
    fn cancel_wins_before_commit_and_a_foreign_destination_never_becomes_saved() {
        let f = fixture();
        let cancelled = cancel(&f.shared, &f.capture_root).unwrap();
        assert_eq!(cancelled["state"], "cancelled");
        assert_eq!(call(&f, &f.request).unwrap_err(), "cancelled");
        fs::remove_dir_all(f.base).unwrap();

        let f = fixture();
        private_dir(&f.destination);
        fs::write(f.destination.join("foreign.txt"), b"foreign").unwrap();
        assert_eq!(call(&f, &f.request).unwrap_err(), "invalid_publication");
        assert_eq!(
            fs::read(f.destination.join("foreign.txt")).unwrap(),
            b"foreign"
        );
        fs::remove_dir_all(f.base).unwrap();
    }

    #[test]
    fn derivative_input_is_unavailable_without_native_job_authority() {
        let f = fixture();
        let staging_root = Path::new(f.request["stagingRoot"].as_str().unwrap());
        let plan_path = staging_root.join("publication-plan.json");
        let mut plan: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
        let entry = plan["entries"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["relativePath"] == "frames/frame-01.png")
            .unwrap();
        entry["input"] = json!({"kind":"derivative","receipt":{"schema":"apparent.media-derivative/1"},"fileIndex":1});
        let plan_bytes = serde_json::to_vec(&plan).unwrap();
        SecureStore::open(staging_root)
            .unwrap()
            .replace_checked("publication-plan.json", &plan_bytes)
            .unwrap();
        let plan_file = sealed(
            &SecureStore::open(staging_root).unwrap(),
            staging_root,
            "publication-plan.json",
        );
        let mut request = f.request.clone();
        request["planFile"] = serde_json::to_value(plan_file).unwrap();
        request["planSha256"] = Value::String(sha(&plan_bytes));
        assert_eq!(call(&f, &request).unwrap_err(), "unavailable");
        assert!(!f.destination.exists());
        fs::remove_dir_all(f.base).unwrap();
    }

    #[cfg(feature = "media")]
    #[test]
    fn derivative2_frame_and_sheet_require_authenticated_completed_registry_authority() {
        use super::super::materials_media::{Checkpoint, JobOutcome, JobRegistry, StoredJobState};

        let f = fixture();
        let materials: super::super::materials_media::MaterialsKey =
            serde_json::from_value(f.request["materials"].clone()).unwrap();
        let lease_id = f.request["leaseId"].as_str().unwrap().to_owned();
        let output_path = f.base.join("native-derivative-output");
        let output = Arc::new(SecureStore::create(&output_path).unwrap());
        let output_directory = output_path.canonicalize().unwrap().display().to_string();
        let job_id = apparent_capture_engine::new_id().unwrap();
        let layout = super::super::materials_media::contact_sheet::Layout {
            schema: "apparent.contact-sheet-layout/1".into(),
            width: 170,
            height: 45,
            background_rgb: [12, 34, 56],
            fit: "stretch".into(),
            sampling: "legacy-bilinear".into(),
            clipping: "reject".into(),
            tiles: vec![
                super::super::materials_media::contact_sheet::TileRect {
                    request_index: 2,
                    x: 0,
                    y: 0,
                    width: 80,
                    height: 45,
                },
                super::super::materials_media::contact_sheet::TileRect {
                    request_index: 4,
                    x: 90,
                    y: 0,
                    width: 80,
                    height: 45,
                },
            ],
        };
        let request = RetainedRequest {
            schema: "apparent.retained-frame-request/1",
            materials: materials.clone(),
            lease_id: lease_id.clone(),
            job_id: job_id.clone(),
            output_directory: output_directory.clone(),
            requested_bundle_ms: vec![0, 0, 1, 1],
            limits: RetainedLimits {
                max_frames: 4,
                max_edge: 80,
                max_total_pixels: 64_000,
                deadline_ms: 20_000,
            },
            contact_sheet: Some(layout),
        };
        let request_bytes = serde_json::to_vec(&request).unwrap();
        retained::read_authenticated_journal(&f.capture_root, &f.capture_id, &f.owner).unwrap();
        let registry =
            JobRegistry::open(Arc::new(SecureStore::open(&f.runtime_path).unwrap())).unwrap();
        let mut sink = RegistrySink(output.clone());
        let outcome = super::super::materials_media::execute_retained_linux(
            &registry,
            &f.runtime,
            &f.capture_root,
            &f.capture_id,
            &f.owner,
            &retained::MaterialKey {
                capture_id: materials.capture_id.clone(),
                materials_id: materials.materials_id.clone(),
                revision: materials.revision.clone(),
            },
            &lease_id,
            &request_bytes,
            output.clone(),
            &mut sink,
            || Checkpoint::Continue,
        )
        .unwrap();
        let JobOutcome::Complete(_) = outcome else {
            panic!("H.264 fixture did not produce a new native job completion");
        };
        let request_sha256 = sha(&request_bytes);
        let stored = registry
            .lookup_authenticated(&f.capture_id, &job_id, &request_sha256)
            .unwrap();
        assert_eq!(stored.state, StoredJobState::Complete);
        let envelope_bytes = stored.envelope.unwrap();
        let envelope: Value = serde_json::from_slice(&envelope_bytes).unwrap();
        let mut base_plan: Value = serde_json::from_slice(
            &fs::read(
                Path::new(f.request["stagingRoot"].as_str().unwrap()).join("publication-plan.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let frame = envelope["payload"]["frames"][0].clone();
        let sheet = envelope["payload"]["contactSheet"].clone();
        for (path, role, image, selector) in [
            (
                "frames/frame-01.png",
                "frame",
                frame,
                json!({"kind":"frame","requestIndex":1}),
            ),
            (
                "contact-sheet.png",
                "contact-sheet",
                sheet,
                json!({"kind":"contact-sheet"}),
            ),
        ] {
            let entry = base_plan["entries"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|entry| entry["relativePath"] == path)
                .unwrap();
            entry["sha256"] = image["file"]["sha256"].clone();
            entry["byteLength"] = image["byteLength"].clone();
            entry["role"] = Value::String(role.into());
            entry["input"] = json!({"kind":"derivative2","receipt":envelope,"selector":selector});
        }

        let mut forged = base_plan.clone();
        forged["entries"][2]["input"]["receipt"]["receiptId"] =
            Value::String(apparent_capture_engine::new_id().unwrap());
        forged["entries"][2]["input"]["receipt"]["payload"]["receiptId"] =
            forged["entries"][2]["input"]["receipt"]["receiptId"].clone();
        let request = replace_plan(&f, &forged);
        assert!(call(&f, &request).is_err());
        assert!(
            read_journal(&f.capture_root, &f.capture_id, &f.owner)
                .unwrap()
                .publication
                .is_none()
        );

        let mut wrong_material = base_plan.clone();
        wrong_material["entries"][2]["input"]["receipt"]["materials"]["revision"] =
            Value::String("0".repeat(64));
        wrong_material["entries"][2]["input"]["receipt"]["payload"]["materials"]["revision"] =
            Value::String("0".repeat(64));
        let request = replace_plan(&f, &wrong_material);
        assert!(call(&f, &request).is_err());

        let mut wrong_request = base_plan.clone();
        wrong_request["entries"][2]["input"]["receipt"]["requestSha256"] =
            Value::String("0".repeat(64));
        wrong_request["entries"][2]["input"]["receipt"]["payload"]["requestSha256"] =
            Value::String("0".repeat(64));
        let request = replace_plan(&f, &wrong_request);
        assert!(call(&f, &request).is_err());

        let mut wrong_root = base_plan.clone();
        wrong_root["entries"][2]["input"]["receipt"]["root"] =
            Value::String(f.base.join("foreign-root").to_string_lossy().into_owned());
        wrong_root["entries"][2]["input"]["receipt"]["payload"]["root"] =
            wrong_root["entries"][2]["input"]["receipt"]["root"].clone();
        let request = replace_plan(&f, &wrong_root);
        assert!(call(&f, &request).is_err());

        let mut cross_role = base_plan.clone();
        cross_role["entries"][2]["role"] = Value::String("contact-sheet".into());
        let request = replace_plan(&f, &cross_role);
        assert!(call(&f, &request).is_err());

        let mut wrong_index = base_plan.clone();
        wrong_index["entries"][2]["input"]["selector"]["requestIndex"] = json!(99);
        let request = replace_plan(&f, &wrong_index);
        assert!(call(&f, &request).is_err());

        let mut wrong_hash = base_plan.clone();
        wrong_hash["entries"][2]["sha256"] = Value::String("0".repeat(64));
        let request = replace_plan(&f, &wrong_hash);
        assert!(call(&f, &request).is_err());

        let mut wrong_length = base_plan.clone();
        wrong_length["entries"][2]["byteLength"] =
            json!(base_plan["entries"][2]["byteLength"].as_u64().unwrap() + 1);
        let request = replace_plan(&f, &wrong_length);
        assert!(call(&f, &request).is_err());
        assert!(
            read_journal(&f.capture_root, &f.capture_id, &f.owner)
                .unwrap()
                .publication
                .is_none()
        );
        assert!(!f.destination.exists());

        // Restore the exact completed registry envelope and publish both
        // independently selected outputs through the normal capture/2 path.
        let request = replace_plan(&f, &base_plan);
        let saved = call(&f, &request).unwrap();
        assert_eq!(saved["state"], "saved");
        assert_eq!(saved["publication"]["artifactProvenance"], "native");
        let native_frames = SecureStore::open(&output_path.join("frames")).unwrap();
        let selected_frame = native_frames.file("frame-001.png", false).unwrap();
        let mut selected_frame_bytes = Vec::new();
        selected_frame
            .try_clone()
            .unwrap()
            .read_to_end(&mut selected_frame_bytes)
            .unwrap();
        assert_eq!(
            fs::read(f.destination.join("frames/frame-01.png")).unwrap(),
            selected_frame_bytes
        );
        assert_eq!(
            fs::read(f.destination.join("contact-sheet.png")).unwrap(),
            fs::read(output_path.join("contact-sheet.png")).unwrap()
        );
        let inspect = json!({"method":"inspectPublication","captureId":f.capture_id,"ownerToken":f.owner,
            "publicationId":f.request["publicationId"],"planSha256":request["planSha256"]});
        assert_eq!(call(&f, &inspect).unwrap(), saved);
        fs::remove_dir_all(f.base).unwrap();
    }

    #[test]
    fn inspect_recovers_committed_publication_after_worker_panic_without_sources() {
        for legacy_failed_state in [false, true] {
            let f = fixture();
            let mut journal = read_journal(&f.capture_root, &f.capture_id, &f.owner).unwrap();
            let mut publication = request_binding(&f.request, &journal).unwrap();
            let opened =
                open_publication(&f.runtime, &f.capture_root, &journal, &publication).unwrap();
            opened
                .backend
                .prepare(
                    &opened.context,
                    &opened.plan_bytes,
                    &publication.plan_sha256,
                )
                .unwrap();
            publication.commit_decided = true;
            journal.publication = Some(publication.clone());
            persist_record(
                &f.runtime,
                &f.capture_root,
                &journal,
                &mut Default::default(),
            )
            .unwrap();
            let worker = std::thread::spawn(move || {
                opened
                    .backend
                    .commit(
                        &opened.context,
                        &opened.plan_bytes,
                        &publication.plan_sha256,
                    )
                    .unwrap();
                panic!("synthetic worker loss after Core commit");
            });
            assert!(worker.join().is_err());
            worker_panicked(&mut journal);
            assert_eq!(journal.publication.as_ref().unwrap().state, "publishing");
            assert!(journal.publication.as_ref().unwrap().commit_decided);
            if legacy_failed_state {
                journal.publication.as_mut().unwrap().state = "failed".into();
                journal.error = Some("internal".into());
            }
            persist_record(
                &f.runtime,
                &f.capture_root,
                &journal,
                &mut Default::default(),
            )
            .unwrap();
            fs::remove_dir_all(&journal.root).unwrap();
            fs::remove_dir_all(&journal.publication.as_ref().unwrap().staging_root).unwrap();
            let restarted = Runtime::open(&f.runtime_path).unwrap();
            let inspect = json!({"method":"inspectPublication","captureId":f.capture_id,"ownerToken":f.owner,
                "publicationId":f.request["publicationId"],"planSha256":f.request["planSha256"]});
            let recovered = handle(
                &inspect,
                &restarted,
                &f.capture_root,
                &mut Default::default(),
            )
            .unwrap();
            assert_eq!(recovered["state"], "saved");
            assert!(f.destination.join("apparent.json").is_file());
            assert_eq!(
                recovered["publication"]["planSha256"],
                f.request["planSha256"]
            );
            fs::remove_dir_all(f.base).unwrap();
        }
    }

    #[test]
    fn inspect_recovers_commit_after_worker_restart_and_lost_reply() {
        let f = fixture();
        let mut journal = read_journal(&f.capture_root, &f.capture_id, &f.owner).unwrap();
        let publication = request_binding(&f.request, &journal).unwrap();
        let opened = open_publication(&f.runtime, &f.capture_root, &journal, &publication).unwrap();
        opened
            .backend
            .prepare(
                &opened.context,
                &opened.plan_bytes,
                &publication.plan_sha256,
            )
            .unwrap();
        journal.publication = Some(publication);
        let _owner = f
            .runtime
            .acquire_capture_owner(&f.capture_id)
            .unwrap()
            .unwrap();
        persist_record(
            &f.runtime,
            &f.capture_root,
            &journal,
            &mut Default::default(),
        )
        .unwrap();
        drop(_owner);
        drop(opened);

        let restarted_runtime = Runtime::open(&f.runtime_path).unwrap();
        let inspect = json!({"method":"inspectPublication","captureId":f.capture_id,"ownerToken":f.owner,
            "publicationId":f.request["publicationId"],"planSha256":f.request["planSha256"]});
        let recovered = handle(
            &inspect,
            &restarted_runtime,
            &f.capture_root,
            &mut Default::default(),
        )
        .unwrap();
        assert_eq!(recovered["state"], "saved");
        assert!(f.destination.join("apparent.json").is_file());
        fs::remove_dir_all(f.base).unwrap();
    }

    #[test]
    fn cancellation_between_snapshot_and_source_pin_is_not_source_corruption() {
        let f = fixture();
        let stale = read_journal(&f.capture_root, &f.capture_id, &f.owner).unwrap();
        let incoming = request_binding(&f.request, &stale).unwrap();
        cancel(&f.shared, &f.capture_root).unwrap();
        assert_eq!(
            open_publication(&f.runtime, &f.capture_root, &stale, &incoming).err(),
            Some("cancelled")
        );
        assert!(!f.destination.exists());
        fs::remove_dir_all(f.base).unwrap();
    }

    #[test]
    fn owner_contention_returns_busy_without_binding_or_publishing() {
        let f = fixture();
        let owner = f
            .runtime
            .acquire_capture_owner(&f.capture_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            handle(
                &f.request,
                &f.runtime,
                &f.capture_root,
                &mut Default::default()
            )
            .err(),
            Some("busy")
        );
        assert!(
            read_journal(&f.capture_root, &f.capture_id, &f.owner)
                .unwrap()
                .publication
                .is_none()
        );
        assert!(!f.destination.exists());
        drop(owner);
        assert_eq!(
            cancel(&f.shared, &f.capture_root).unwrap()["state"],
            "cancelled"
        );
        assert_eq!(
            handle(
                &f.request,
                &f.runtime,
                &f.capture_root,
                &mut Default::default()
            )
            .err(),
            Some("cancelled")
        );
        assert!(!f.destination.exists());
        fs::remove_dir_all(f.base).unwrap();
    }

    #[test]
    fn concurrent_cancel_and_commit_have_one_durable_winner() {
        let f = fixture();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let publish_barrier = barrier.clone();
        let request = f.request.clone();
        let runtime = f.runtime.clone();
        let capture_root = f.capture_root.clone();
        let publish_handle = std::thread::spawn(move || {
            publish_barrier.wait();
            handle(&request, &runtime, &capture_root, &mut Default::default())
        });
        let cancel_barrier = barrier;
        let shared = f.shared.clone();
        let root = f.capture_root.clone();
        let cancel_handle = std::thread::spawn(move || {
            cancel_barrier.wait();
            cancel(&shared, &root)
        });
        let publish_result = publish_handle.join().unwrap();
        let cancel_result = cancel_handle.join().unwrap();
        // Either contender can encounter the other's short owner permit.
        // Busy is not a cancellation or publication receipt; reconcile the
        // original tuple after both contenders have released their permits.
        assert!(
            publish_result.is_ok() || matches!(publish_result, Err("cancelled" | "busy")),
            "publish returned {publish_result:?}"
        );
        assert!(
            cancel_result.is_ok() || cancel_result == Err("busy"),
            "cancel returned {cancel_result:?}"
        );
        let before = read_journal(&f.capture_root, &f.capture_id, &f.owner).unwrap();
        if before.publication.is_some() {
            let inspect = json!({"method":"inspectPublication","captureId":f.capture_id,"ownerToken":f.owner,
                "publicationId":f.request["publicationId"],"planSha256":f.request["planSha256"]});
            let status = handle(
                &inspect,
                &f.runtime,
                &f.capture_root,
                &mut Default::default(),
            )
            .unwrap();
            assert!(
                status["state"] == "saved" || status["state"] == "cancelled",
                "nonterminal winner: {status:?}"
            );
        }
        let durable = read_journal(&f.capture_root, &f.capture_id, &f.owner).unwrap();
        match durable.publication.map(|publication| publication.state) {
            Some(state) if state == "saved" => assert!(f.destination.is_dir()),
            Some(state) if state == "cancelled" => assert!(!f.destination.exists()),
            None => {
                assert_eq!(durable.state, "cancelled");
                assert!(!f.destination.exists());
            }
            other => panic!("invalid publication cancel/commit winner: {other:?}"),
        }
        fs::remove_dir_all(f.base).unwrap();
    }
}
