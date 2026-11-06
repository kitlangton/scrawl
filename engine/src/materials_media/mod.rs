//! Internal retained-material native job port.
//!
//! Call only after authenticating the original capture owner and while holding
//! the exact retained-material read lease. Retained execution keeps that lease
//! borrowed through decode and receipt sealing. Its caller MUST run this work
//! inside the existing supervised media job child; timeout and cancellation
//! must kill and reap that child before dropping the lease. This module adds no
//! wire method and its receipt is native-origin evidence for the publisher.

pub mod contact_sheet;
mod registry;

pub use registry::{
    Binding, CancelObservation, CancelRequestOutcome, Checkpoint, FileIdentity, JobClaim,
    JobIntent, JobOutcome, JobReference, JobRegistry, MaterialsKey, NativeReceipt, OutputIdentity,
    SealedFile, SheetSeal, StoredJob, StoredJobState, verify_completed,
};

use crate::materials_media::contact_sheet::{Layout, RgbaImage, compose};
use apparent_capture_engine::{BundleLease, SecureStore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::error::Error as StdError;
use std::fs::File;
use std::io::Read;
#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;

type Result<T> = std::result::Result<T, Box<dyn StdError + Send + Sync>>;

macro_rules! ensure {
    ($condition:expr, $message:literal) => {
        if !$condition {
            return Err(std::io::Error::other($message).into());
        }
    };
    ($condition:expr) => {
        if !$condition {
            return Err(std::io::Error::other("native media2 validation failed").into());
        }
    };
}

/// Execute one accepted internal job. The caller owns authentication, original
/// materials lease admission, process supervision and cancellation/deadline
/// source. Replay and collision checks happen before any output write.
#[cfg(all(test, target_os = "linux", feature = "media"))]
pub(crate) fn execute_linux(
    registry: &JobRegistry,
    binding: Binding,
    original_request_bytes: &[u8],
    source: &File,
    materials_payload: &[u8],
    source_identity: &apparent_capture_engine::materials_media::FileIdentity,
    materials_lease: &BundleLease,
    output_root: Arc<SecureStore>,
    sink: &mut dyn apparent_capture_engine::materials_media::DerivativeOutputSink,
    layout: Option<Layout>,
    checkpoint: impl FnMut() -> Checkpoint,
) -> Result<JobOutcome> {
    let claim = match registry.begin(binding.clone(), output_root.clone())? {
        JobOutcome::Owned(claim) => claim,
        existing => return Ok(existing),
    };
    execute_pinned(
        binding,
        original_request_bytes,
        source,
        materials_payload,
        source_identity,
        claim,
        Some(materials_lease),
        None,
        output_root,
        sink,
        layout,
        derive_retained_linux,
        checkpoint,
    )
}

/// Run the retained job after the caller has authenticated the capture owner.
/// Call `read_authenticated_journal` before registry lookup; then pin this source
/// inside the existing supervised worker so its full video hash consumes that
/// worker's deadline/resource budget. Keep `source` alive until reaping.
#[cfg(all(target_os = "linux", feature = "media"))]
pub(crate) fn execute_retained_linux(
    registry: &JobRegistry,
    runtime: &apparent_capture_engine::Runtime,
    capture_journal_directory: &std::path::Path,
    capture_id: &str,
    owner_token: &str,
    materials: &crate::retained::MaterialKey,
    lease_id: &str,
    original_request_bytes: &[u8],
    output_root: Arc<SecureStore>,
    sink: &mut dyn apparent_capture_engine::materials_media::DerivativeOutputSink,
    checkpoint: impl FnMut() -> Checkpoint,
) -> Result<JobOutcome> {
    execute_retained(
        registry,
        runtime,
        capture_journal_directory,
        capture_id,
        owner_token,
        materials,
        lease_id,
        original_request_bytes,
        output_root,
        sink,
        checkpoint,
        derive_retained_linux,
    )
}

#[cfg(all(target_os = "macos", feature = "media"))]
pub(crate) fn execute_retained_macos(
    registry: &JobRegistry,
    runtime: &apparent_capture_engine::Runtime,
    capture_journal_directory: &std::path::Path,
    capture_id: &str,
    owner_token: &str,
    materials: &crate::retained::MaterialKey,
    lease_id: &str,
    original_request_bytes: &[u8],
    output_root: Arc<SecureStore>,
    sink: &mut dyn apparent_capture_engine::materials_media::DerivativeOutputSink,
    checkpoint: impl FnMut() -> Checkpoint,
) -> Result<JobOutcome> {
    execute_retained(
        registry,
        runtime,
        capture_journal_directory,
        capture_id,
        owner_token,
        materials,
        lease_id,
        original_request_bytes,
        output_root,
        sink,
        checkpoint,
        derive_retained_macos,
    )
}

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
fn execute_retained(
    registry: &JobRegistry,
    runtime: &apparent_capture_engine::Runtime,
    capture_journal_directory: &std::path::Path,
    capture_id: &str,
    owner_token: &str,
    materials: &crate::retained::MaterialKey,
    lease_id: &str,
    original_request_bytes: &[u8],
    output_root: Arc<SecureStore>,
    sink: &mut dyn apparent_capture_engine::materials_media::DerivativeOutputSink,
    mut checkpoint: impl FnMut() -> Checkpoint,
    derive: impl FnMut(
        &Binding,
        &File,
        &[u8],
        &apparent_capture_engine::materials_media::FileIdentity,
        &mut dyn apparent_capture_engine::materials_media::DerivativeOutputSink,
        &mut dyn FnMut() -> anyhow::Result<()>,
    ) -> anyhow::Result<(Vec<u8>, Vec<u8>)>,
) -> Result<JobOutcome> {
    ensure!(
        original_request_bytes.len() <= 256 * 1024,
        "request exceeds native bound"
    );
    let request: NativeRequest = serde_json::from_slice(original_request_bytes)?;
    ensure!(
        serde_json::to_vec(&request)? == original_request_bytes,
        "request is not canonical JSON"
    );
    ensure!(
        request.schema == "apparent.retained-frame-request/1",
        "unsupported retained media request"
    );
    ensure!(
        request.materials.capture_id == capture_id
            && request.materials.capture_id == materials.capture_id
            && request.materials.materials_id == materials.materials_id
            && request.materials.revision == materials.revision
            && request.lease_id == lease_id,
        "request differs from authenticated lookup"
    );
    let output_metadata = output_root.directory_metadata()?;
    let intent = JobIntent {
        schema: "apparent.media2-job-intent/1".into(),
        capture_id: request.materials.capture_id.clone(),
        materials: MaterialsKey {
            capture_id: request.materials.capture_id.clone(),
            materials_id: request.materials.materials_id.clone(),
            revision: request.materials.revision.clone(),
        },
        lease_id: request.lease_id.clone(),
        job_id: request.job_id.clone(),
        request_sha256: digest_hex(&Sha256::digest(original_request_bytes)),
        requested_bundle_ms: request.requested_bundle_ms.clone(),
        limits: registry::Limits {
            max_frames: request.limits.max_frames,
            max_edge: request.limits.max_edge,
            max_total_pixels: request.limits.max_total_pixels,
            deadline_ms: request.limits.deadline_ms,
        },
        output_root: request.output_directory.clone(),
        output_identity: OutputIdentity {
            device: output_metadata.dev().to_string(),
            inode: output_metadata.ino().to_string(),
        },
        layout: request.contact_sheet.clone(),
        admitted_at_ms: 0,
    };
    // Authentication and read_authenticated_journal happen before this method.
    // Intent admission is durable before the full source hash begins.
    let mut claim = match registry.admit(intent, output_root.clone())? {
        JobOutcome::Owned(claim) => claim,
        existing => return Ok(existing),
    };
    match checkpoint() {
        Checkpoint::Continue => {}
        terminal => {
            claim.fail(terminal)?;
            return Err(std::io::Error::other(match terminal {
                Checkpoint::Cancel => "cancelled",
                Checkpoint::Deadline => "deadline_exceeded",
                Checkpoint::Failed => "failed",
                Checkpoint::OwnerLost => "owner_lost",
                Checkpoint::Continue => unreachable!(),
            })
            .into());
        }
    }
    // Pinning re-authenticates and hashes the complete movie here, inside the
    // supervised worker so hard deadline/resource enforcement includes it.
    let source = crate::retained::pin_retained_materials(
        runtime,
        capture_journal_directory,
        capture_id,
        owner_token,
        materials,
        lease_id,
    )
    .map_err(std::io::Error::other)?;
    source.root.check()?;
    let root_metadata = source.root.directory_metadata()?;
    ensure!(
        root_metadata.dev() == source.root_device && root_metadata.ino() == source.root_inode,
        "retained root identity changed"
    );
    ensure!(
        request.materials.capture_id == source.materials.capture_id
            && source.capture_id == source.materials.capture_id
            && request.materials.materials_id == source.materials.materials_id
            && request.materials.revision == source.materials.revision
            && request.lease_id == source.lease_id,
        "request differs from authenticated retained source"
    );
    let layout = request.contact_sheet.clone();
    let binding = Binding {
        schema: "apparent.media2-job/1".into(),
        capture_id: source.capture_id.clone(),
        materials: MaterialsKey {
            capture_id: source.materials.capture_id.clone(),
            materials_id: source.materials.materials_id.clone(),
            revision: source.materials.revision.clone(),
        },
        materials_root: source
            .root_path
            .to_str()
            .ok_or_else(|| std::io::Error::other("retained root is not UTF-8"))?
            .into(),
        materials_root_identity: OutputIdentity {
            device: source.root_device.to_string(),
            inode: source.root_inode.to_string(),
        },
        lease_id: source.lease_id.clone(),
        source_video: SealedFile {
            relative_path: source.video.relative_path.clone(),
            identity: FileIdentity {
                device: source.video.identity.device.clone(),
                inode: source.video.identity.inode.clone(),
                size: source.video.identity.size,
                modified_ns: source.video.identity.modified_ns.clone(),
            },
            sha256: source.video.sha256.clone(),
        },
        video_offset_ms: source.payload["clock"]["videoOffsetMs"]
            .as_u64()
            .ok_or_else(|| std::io::Error::other("retained video offset missing"))?,
        job_id: request.job_id,
        receipt_id: derived_receipt_id(original_request_bytes),
        request_sha256: digest_hex(&Sha256::digest(original_request_bytes)),
        requested_bundle_ms: request.requested_bundle_ms,
        limits: registry::Limits {
            max_frames: request.limits.max_frames,
            max_edge: request.limits.max_edge,
            max_total_pixels: request.limits.max_total_pixels,
            deadline_ms: request.limits.deadline_ms,
        },
        output_root: request.output_directory,
        output_identity: OutputIdentity {
            device: output_metadata.dev().to_string(),
            inode: output_metadata.ino().to_string(),
        },
        layout: request.contact_sheet,
    };
    claim.bind_source(binding.clone())?;
    let identity = core_file_identity(&source.video.identity);
    execute_pinned(
        binding,
        original_request_bytes,
        &source.video_file,
        &source.payload_bytes,
        &identity,
        claim,
        None,
        Some(&source),
        output_root,
        sink,
        layout,
        derive,
        &mut checkpoint,
    )
}

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
fn core_file_identity(
    identity: &crate::retained::MaterialFileIdentity,
) -> apparent_capture_engine::materials_media::FileIdentity {
    apparent_capture_engine::materials_media::FileIdentity {
        device: identity.device.clone(),
        inode: identity.inode.clone(),
        size: identity.size,
        modified_ns: identity.modified_ns.clone(),
    }
}

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
fn derived_receipt_id(request: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"apparent.media2-receipt-id/1\0");
    digest.update(request);
    digest.finalize()[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
fn check_material_source(
    lease: Option<&BundleLease>,
    source: Option<&crate::retained::RetainedMaterialSource>,
) -> Result<()> {
    if let Some(lease) = lease {
        lease.check()?;
    }
    if let Some(source) = source {
        source.root.check()?;
        let metadata = source.root.directory_metadata()?;
        ensure!(
            metadata.dev() == source.root_device && metadata.ino() == source.root_inode,
            "retained root identity changed"
        );
    }
    Ok(())
}

#[cfg(all(target_os = "linux", feature = "media"))]
fn derive_retained_linux(
    binding: &Binding,
    source: &File,
    materials_payload: &[u8],
    source_identity: &apparent_capture_engine::materials_media::FileIdentity,
    sink: &mut dyn apparent_capture_engine::materials_media::DerivativeOutputSink,
    checkpoint: &mut dyn FnMut() -> anyhow::Result<()>,
) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let key = core_materials_key(binding);
    apparent_capture_engine::materials_media::derive_linux(
        source,
        &key,
        materials_payload,
        source_identity,
        &binding.materials_root,
        &binding.lease_id,
        &binding.job_id,
        &binding.receipt_id,
        &binding.output_root,
        &binding.requested_bundle_ms,
        binding.limits.max_edge,
        sink,
        checkpoint,
    )
}

#[cfg(all(target_os = "macos", feature = "media"))]
fn derive_retained_macos(
    binding: &Binding,
    source: &File,
    materials_payload: &[u8],
    source_identity: &apparent_capture_engine::materials_media::FileIdentity,
    sink: &mut dyn apparent_capture_engine::materials_media::DerivativeOutputSink,
    checkpoint: &mut dyn FnMut() -> anyhow::Result<()>,
) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    use apparent_capture_engine::materials_media::{DecodedFrame, RetainedPreparation};

    checkpoint()?;
    let key = core_materials_key(binding);
    let mut preparation = RetainedPreparation::new_pinned(
        source,
        &key,
        materials_payload,
        source_identity,
        &binding.materials_root,
        &binding.lease_id,
        &binding.job_id,
        &binding.receipt_id,
        &binding.output_root,
        &binding.requested_bundle_ms,
    )?;
    checkpoint()?;
    let effective_times = preparation.effective_video_times().collect::<Vec<_>>();
    let mut preparation_error = None;
    let decode_result = crate::macos::decode(
        source,
        &effective_times,
        binding.limits.max_edge,
        binding.limits.max_total_pixels,
        |decoded| match preparation.push(
            DecodedFrame {
                actual_video_ms: decoded.actual_ms,
                clamped_to_end: decoded.clamped_to_end,
                width: decoded.width,
                height: decoded.height,
                rgba: decoded.rgba,
            },
            sink,
            checkpoint,
        ) {
            Ok(()) => Ok(()),
            Err(error) => {
                let code = match error.to_string().as_str() {
                    "cancelled" => "cancelled",
                    "deadline_exceeded" => "deadline_exceeded",
                    "owner_lost" => "owner_lost",
                    message
                        if message.contains("limit")
                            || message.contains("budget")
                            || message.contains("dimensions")
                            || message.contains("pixel data") =>
                    {
                        "limits_exceeded"
                    }
                    _ => "decode_failed",
                };
                preparation_error = Some(error);
                Err(code)
            }
        },
    );
    if let Some(error) = preparation_error {
        return Err(error.into());
    }
    decode_result.map_err(std::io::Error::other)?;
    preparation.finish(sink, checkpoint)
}

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
fn core_materials_key(binding: &Binding) -> apparent_capture_engine::materials_media::MaterialsKey {
    apparent_capture_engine::materials_media::MaterialsKey {
        capture_id: binding.materials.capture_id.clone(),
        materials_id: binding.materials.materials_id.clone(),
        revision: binding.materials.revision.clone(),
    }
}

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
fn execute_pinned(
    binding: Binding,
    original_request_bytes: &[u8],
    source: &File,
    materials_payload: &[u8],
    source_identity: &apparent_capture_engine::materials_media::FileIdentity,
    mut claim: JobClaim,
    materials_lease: Option<&BundleLease>,
    retained_source: Option<&crate::retained::RetainedMaterialSource>,
    output_root: Arc<SecureStore>,
    sink: &mut dyn apparent_capture_engine::materials_media::DerivativeOutputSink,
    layout: Option<Layout>,
    mut derive: impl FnMut(
        &Binding,
        &File,
        &[u8],
        &apparent_capture_engine::materials_media::FileIdentity,
        &mut dyn apparent_capture_engine::materials_media::DerivativeOutputSink,
        &mut dyn FnMut() -> anyhow::Result<()>,
    ) -> anyhow::Result<(Vec<u8>, Vec<u8>)>,
    mut checkpoint: impl FnMut() -> Checkpoint,
) -> Result<JobOutcome> {
    check_material_source(materials_lease, retained_source)?;
    validate_original_request(
        original_request_bytes,
        &binding,
        layout.as_ref(),
        materials_payload,
    )?;
    ensure!(
        layout == binding.layout,
        "layout differs from durable native binding"
    );
    if source_identity.device != binding.source_video.identity.device
        || source_identity.inode != binding.source_video.identity.inode
        || source_identity.size != binding.source_video.identity.size
        || source_identity.modified_ns != binding.source_video.identity.modified_ns
    {
        return Err(
            std::io::Error::other("source identity differs from durable job binding").into(),
        );
    }
    validate_source_binding(&binding, materials_payload, source_identity)?;
    let (cancel_job, cancel_reference) = claim.cancel_probe();
    let mut check = || {
        check_material_source(materials_lease, retained_source)
            .map_err(|error| anyhow::Error::msg(error.to_string()))?;
        if registry::read_cancel_intent(&cancel_job, &cancel_reference)
            .map_err(|error| anyhow::Error::msg(error.to_string()))?
            .is_some()
        {
            return Err(std::io::Error::other("cancelled").into());
        }
        match checkpoint() {
            Checkpoint::Continue => Ok(()),
            Checkpoint::Cancel => Err(std::io::Error::other("cancelled").into()),
            Checkpoint::Deadline => Err(std::io::Error::other("deadline_exceeded").into()),
            Checkpoint::Failed => Err(std::io::Error::other("failed").into()),
            Checkpoint::OwnerLost => Err(std::io::Error::other("owner_lost").into()),
        }
    };
    let result: Result<JobOutcome> = (|| {
        let mut frame_sink = FrameSink {
            output: sink,
            legacy_receipt: None,
        };
        let (legacy_payload, _) = derive(
            &binding,
            source,
            materials_payload,
            source_identity,
            &mut frame_sink,
            &mut check,
        )?;
        check_material_source(materials_lease, retained_source)?;
        let files = validate_legacy_frame_payload(&legacy_payload, &binding, materials_payload)?;
        let legacy_receipt = frame_sink
            .legacy_receipt
            .take()
            .ok_or_else(|| std::io::Error::other("Core derivative receipt was not produced"))?;
        drop(frame_sink);
        let legacy_value: Value = serde_json::from_slice(&legacy_payload)?;
        let frame_values = legacy_value["frames"]
            .as_array()
            .ok_or_else(|| std::io::Error::other("Core frame payload missing"))?;
        let mut total_pixels = 0u64;
        for frame in frame_values {
            let width = frame["width"]
                .as_u64()
                .ok_or_else(|| std::io::Error::other("frame width missing"))?;
            let height = frame["height"]
                .as_u64()
                .ok_or_else(|| std::io::Error::other("frame height missing"))?;
            total_pixels = total_pixels
                .checked_add(width * height)
                .ok_or_else(|| std::io::Error::other("pixel budget overflow"))?;
        }
        ensure!(
            total_pixels <= binding.limits.max_total_pixels,
            "frame output pixel limit exceeded"
        );
        if let Some(layout) = layout.as_ref() {
            total_pixels = total_pixels
                .checked_add(u64::from(layout.width) * u64::from(layout.height))
                .ok_or_else(|| std::io::Error::other("pixel budget overflow"))?;
            ensure!(
                total_pixels <= binding.limits.max_total_pixels,
                "combined output pixel limit exceeded"
            );
        }
        let sheet = if let Some(layout) = layout.as_ref() {
            let layout_hash = layout.sha256().map_err(std::io::Error::other)?;
            ensure!(
                binding
                    .layout
                    .as_ref()
                    .and_then(|value| value.sha256().ok())
                    .as_deref()
                    == Some(layout_hash.as_str()),
                "sheet layout hash differs from job binding"
            );
            let frame_images = decode_output_frames(&output_root, &files)?;
            let composed =
                compose(&frame_images, &layout, &layout_hash).map_err(std::io::Error::other)?;
            let bytes = encode_png(&composed.image)?;
            let file = sink.write_new_synced("contact-sheet.png", &bytes)?;
            Some(SheetSeal {
                layout_hash,
                file: local_seal(file),
            })
        } else {
            ensure!(
                binding.layout.is_none(),
                "layout hash supplied without layout"
            );
            None
        };
        check()?;
        let mut payload_frames = Vec::with_capacity(frame_values.len());
        for (i, frame) in frame_values.iter().enumerate() {
            let file = &files[i];
            let width = frame["width"]
                .as_u64()
                .ok_or_else(|| std::io::Error::other("frame width missing"))?;
            let height = frame["height"]
                .as_u64()
                .ok_or_else(|| std::io::Error::other("frame height missing"))?;
            let bytes = read_sealed_file(&output_root, file)?;
            payload_frames.push(serde_json::json!({
                "requestIndex": i + 1,
                "requestedBundleMs": frame["requestedBundleMs"],
                "effectiveVideoMs": frame["effectiveVideoMs"],
                "actualVideoMs": frame["actualVideoMs"],
                "clampedToStart": frame["clampedToStart"],
                "clampedToEnd": frame["clampedToEnd"],
                "width": width,
                "height": height,
                "encoding": "png",
                "byteLength": bytes.len(),
                "file": file,
            }));
        }
        let sheet_value = if let Some(sheet) = &sheet {
            let bytes = read_sealed_file(&output_root, &sheet.file)?;
            let layout = layout
                .as_ref()
                .ok_or_else(|| std::io::Error::other("sheet layout missing"))?;
            Some(serde_json::json!({
                "encoding":"png", "byteLength":bytes.len(), "file":sheet.file,
                "width":layout.width, "height":layout.height, "layoutSha256":sheet.layout_hash, "layout":layout
            }))
        } else {
            None
        };
        ensure!(
            total_pixels <= binding.limits.max_total_pixels,
            "combined output pixel limit exceeded"
        );
        let payload = serde_json::json!({
            "schema":"apparent.media-derivative-payload/2",
            "materials":binding.materials,
            "leaseId":binding.lease_id,
            "jobId":binding.job_id,
            "receiptId":binding.receipt_id,
            "root":binding.output_root,
            "requestSha256":binding.request_sha256,
            "frames":payload_frames,
            "contactSheet":sheet_value,
        });
        let payload_bytes = serde_json::to_vec(&payload)?;
        // Core's checked legacy document builder is reused for decoding/frame
        // seals; its intermediate detached /1 bytes stay in memory only.
        ensure!(
            legacy_receipt == legacy_payload,
            "Core intermediate receipt differs from derived bytes"
        );
        let payload_file = local_seal(sink.write_new_synced("receipt.json", &payload_bytes)?);
        let envelope_value = serde_json::json!({
            "schema":"apparent.media-derivative/2",
            "materials":binding.materials,
            "leaseId":binding.lease_id,
            "jobId":binding.job_id,
            "receiptId":binding.receipt_id,
            "root":binding.output_root,
            "requestSha256":binding.request_sha256,
            "receiptFile":payload_file,
            "payload":payload,
        });
        let envelope = serde_json::to_vec(&envelope_value)?;
        let receipt = NativeReceipt::new(&binding, &files, payload_file, &envelope, sheet)?;
        claim.complete(receipt.clone(), &envelope)?;
        Ok(JobOutcome::Complete(receipt))
    })();
    if let Err(error) = &result {
        let reason = if error.to_string().contains("cancelled") {
            Checkpoint::Cancel
        } else if error.to_string().contains("deadline_exceeded") {
            Checkpoint::Deadline
        } else {
            Checkpoint::Failed
        };
        claim.fail(reason)?;
    }
    result
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct NativeRequest {
    pub(crate) schema: String,
    pub(crate) materials: MaterialsKey,
    pub(crate) lease_id: String,
    pub(crate) job_id: String,
    pub(crate) output_directory: String,
    pub(crate) requested_bundle_ms: Vec<u64>,
    pub(crate) limits: Limits,
    pub(crate) contact_sheet: Option<Layout>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Limits {
    pub(crate) max_frames: u64,
    pub(crate) max_edge: u32,
    pub(crate) max_total_pixels: u64,
    pub(crate) deadline_ms: u64,
}

fn validate_original_request(
    bytes: &[u8],
    binding: &Binding,
    layout: Option<&Layout>,
    materials_payload: &[u8],
) -> Result<()> {
    let request: NativeRequest = serde_json::from_slice(bytes)?;
    ensure!(
        serde_json::to_vec(&request)? == bytes,
        "original request is not canonical JSON"
    );
    ensure!(
        digest_hex(&Sha256::digest(bytes)) == binding.request_sha256,
        "original request digest mismatch"
    );
    ensure!(
        request.schema == "apparent.retained-frame-request/1",
        "unsupported retained frame request"
    );
    let materials: Value = serde_json::from_slice(materials_payload)?;
    let duration = materials["clock"]["durationMs"]
        .as_u64()
        .ok_or_else(|| std::io::Error::other("retained capture duration missing"))?;
    ensure!(
        request
            .requested_bundle_ms
            .iter()
            .all(|time| *time <= duration),
        "requested frame exceeds retained capture clock"
    );
    ensure!(
        request.materials.capture_id == binding.materials.capture_id
            && request.materials.materials_id == binding.materials.materials_id
            && request.materials.revision == binding.materials.revision
            && request.lease_id == binding.lease_id
            && request.job_id == binding.job_id
            && request.output_directory == binding.output_root
            && request.requested_bundle_ms == binding.requested_bundle_ms,
        "original request differs from native job binding"
    );
    ensure!(
        request.limits.max_frames == binding.limits.max_frames
            && request.limits.max_edge == binding.limits.max_edge
            && request.limits.max_total_pixels == binding.limits.max_total_pixels
            && request.limits.deadline_ms == binding.limits.deadline_ms,
        "original request limits differ from native job binding"
    );
    ensure!(
        request
            .contact_sheet
            .as_ref()
            .map(Layout::sha256)
            .transpose()
            .map_err(std::io::Error::other)?
            == layout
                .map(Layout::sha256)
                .transpose()
                .map_err(std::io::Error::other)?,
        "original request layout differs from execution layout"
    );
    ensure!(
        layout == binding.layout.as_ref(),
        "original request layout differs from durable native binding"
    );
    Ok(())
}

fn validate_source_binding(
    binding: &Binding,
    materials_payload: &[u8],
    source_identity: &apparent_capture_engine::materials_media::FileIdentity,
) -> Result<()> {
    let materials: Value = serde_json::from_slice(materials_payload)?;
    ensure!(
        materials["schema"] == "apparent.capture-materials-payload/1",
        "unsupported retained materials payload"
    );
    ensure!(
        materials["identity"]["captureId"] == binding.materials.capture_id
            && materials["identity"]["materialsId"] == binding.materials.materials_id
            && materials["root"] == binding.materials_root
            && materials["leaseId"] == binding.lease_id,
        "retained source key/root/lease differs from native job binding"
    );
    ensure!(
        materials["clock"]["videoOffsetMs"].as_u64() == Some(binding.video_offset_ms),
        "retained video offset differs from native job binding"
    );
    let video_path = materials["video"]["file"]
        .as_str()
        .ok_or_else(|| std::io::Error::other("retained video path missing"))?;
    ensure!(
        video_path == binding.source_video.relative_path,
        "retained source path differs from native job binding"
    );
    let file = materials["files"]
        .as_array()
        .and_then(|files| files.iter().find(|file| file["relativePath"] == video_path))
        .ok_or_else(|| std::io::Error::other("retained video seal missing"))?;
    ensure!(
        file["identity"]["device"] == source_identity.device
            && file["identity"]["inode"] == source_identity.inode
            && file["identity"]["size"].as_u64() == Some(source_identity.size)
            && file["identity"]["modifiedNs"] == source_identity.modified_ns
            && file["sha256"] == binding.source_video.sha256,
        "retained video identity/digest differs from durable job binding"
    );
    Ok(())
}

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
struct FrameSink<'a> {
    output: &'a mut dyn apparent_capture_engine::materials_media::DerivativeOutputSink,
    legacy_receipt: Option<Vec<u8>>,
}

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
impl apparent_capture_engine::materials_media::DerivativeOutputSink for FrameSink<'_> {
    fn write_new_synced(
        &mut self,
        relative_path: &str,
        bytes: &[u8],
    ) -> anyhow::Result<apparent_capture_engine::materials_media::SealedFile> {
        if relative_path != "receipt.json" {
            return self.output.write_new_synced(relative_path, bytes);
        }
        ensure!(
            self.legacy_receipt.is_none(),
            "Core wrote multiple intermediate receipts"
        );
        self.legacy_receipt = Some(bytes.to_vec());
        Ok(apparent_capture_engine::materials_media::SealedFile {
            relative_path: relative_path.into(),
            identity: apparent_capture_engine::materials_media::FileIdentity {
                device: "0".into(),
                inode: "0".into(),
                size: bytes.len() as u64,
                modified_ns: "0".into(),
            },
            sha256: digest_hex(&Sha256::digest(bytes)),
        })
    }
}

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
fn validate_legacy_frame_payload(
    bytes: &[u8],
    binding: &Binding,
    materials_payload: &[u8],
) -> Result<Vec<SealedFile>> {
    let value: Value = serde_json::from_slice(bytes)?;
    ensure!(
        value["schema"] == "apparent.media-derivative-payload/1",
        "invalid Core intermediate derivative schema"
    );
    ensure!(
        value["materials"]["captureId"] == binding.materials.capture_id,
        "derivative capture binding mismatch"
    );
    ensure!(
        value["materials"]["materialsId"] == binding.materials.materials_id,
        "derivative materials binding mismatch"
    );
    ensure!(
        value["materials"]["revision"] == binding.materials.revision,
        "derivative revision mismatch"
    );
    ensure!(
        value["jobId"] == binding.job_id
            && value["receiptId"] == binding.receipt_id
            && value["root"] == binding.output_root,
        "derivative job binding mismatch"
    );
    let materials: Value = serde_json::from_slice(materials_payload)?;
    let offset = materials["clock"]["videoOffsetMs"]
        .as_u64()
        .ok_or_else(|| std::io::Error::other("invalid materials video offset"))?;
    let frames = value["frames"]
        .as_array()
        .ok_or_else(|| std::io::Error::other("invalid derivative frames"))?;
    ensure!(
        frames.len() == binding.requested_bundle_ms.len(),
        "derivative frame count mismatch"
    );
    let mut total_pixels = 0u64;
    let mut files = Vec::with_capacity(frames.len());
    for (i, frame) in frames.iter().enumerate() {
        let requested = binding.requested_bundle_ms[i];
        ensure!(
            frame["requestIndex"].as_u64() == Some(i as u64 + 1),
            "derivative frame order mismatch"
        );
        ensure!(
            frame["requestedBundleMs"].as_u64() == Some(requested),
            "derivative requested time mismatch"
        );
        ensure!(
            frame["effectiveVideoMs"].as_u64() == Some(requested.saturating_sub(offset)),
            "derivative video clock mismatch"
        );
        ensure!(
            frame["clampedToStart"].as_bool() == Some(requested < offset),
            "derivative start clamp mismatch"
        );
        ensure!(
            frame["clampedToEnd"].as_bool().is_some(),
            "derivative end clamp missing"
        );
        ensure!(
            !(frame["clampedToStart"] == true && frame["clampedToEnd"] == true),
            "invalid double clamp"
        );
        let width = frame["width"]
            .as_u64()
            .ok_or_else(|| std::io::Error::other("invalid frame width"))?;
        let height = frame["height"]
            .as_u64()
            .ok_or_else(|| std::io::Error::other("invalid frame height"))?;
        ensure!(
            width > 0
                && height > 0
                && width <= u64::from(binding.limits.max_edge)
                && height <= u64::from(binding.limits.max_edge),
            "frame dimensions exceed bound"
        );
        total_pixels = total_pixels
            .checked_add(width * height)
            .ok_or_else(|| std::io::Error::other("pixel count overflow"))?;
        ensure!(
            total_pixels <= binding.limits.max_total_pixels,
            "total pixel limit exceeded"
        );
        let expected_path = format!("frames/frame-{:03}.png", i + 1);
        ensure!(
            frame["file"]["relativePath"] == expected_path,
            "frame output path mismatch"
        );
        files.push(serde_json::from_value(frame["file"].clone())?);
    }
    Ok(files)
}

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
fn decode_output_frames(output: &Arc<SecureStore>, files: &[SealedFile]) -> Result<Vec<RgbaImage>> {
    files
        .iter()
        .map(|seal| {
            let bytes = read_sealed_file(output, seal)?;
            let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
            decoder.set_limits(png::Limits {
                bytes: 256 * 1024 * 1024,
            });
            let mut reader = decoder.read_info()?;
            ensure!(reader.info().width <= 4096 && reader.info().height <= 4096);
            let mut pixels = vec![
                0;
                reader
                    .output_buffer_size()
                    .ok_or_else(|| std::io::Error::other("invalid PNG size"))?
            ];
            let info = reader.next_frame(&mut pixels)?;
            pixels.truncate(info.buffer_size());
            Ok(RgbaImage {
                width: info.width,
                height: info.height,
                pixels,
            })
        })
        .collect()
}

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
fn read_sealed_file(output: &Arc<SecureStore>, seal: &SealedFile) -> Result<Vec<u8>> {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::MetadataExt;
    let (store, leaf) = if let Some(leaf) = seal.relative_path.strip_prefix("frames/") {
        ensure!(
            !leaf.is_empty() && !leaf.contains('/'),
            "invalid frame seal path"
        );
        (output.child("frames")?, leaf.to_owned())
    } else {
        ensure!(
            seal.relative_path == "contact-sheet.png" || seal.relative_path == "receipt.json",
            "invalid output seal path"
        );
        (output.clone(), seal.relative_path.clone())
    };
    let mut file = store.file(&leaf, false)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.dev().to_string() == seal.identity.device
            && metadata.ino().to_string() == seal.identity.inode
            && metadata.len() == seal.identity.size
            && (i128::from(metadata.mtime()) * 1_000_000_000 + i128::from(metadata.mtime_nsec()))
                .to_string()
                == seal.identity.modified_ns,
        "sealed output identity changed"
    );
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    ensure!(
        digest_hex(&Sha256::digest(&bytes)) == seal.sha256,
        "sealed output digest changed"
    );
    Ok(bytes)
}

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
fn local_seal(file: apparent_capture_engine::materials_media::SealedFile) -> SealedFile {
    SealedFile {
        relative_path: file.relative_path,
        identity: FileIdentity {
            device: file.identity.device,
            inode: file.identity.inode,
            size: file.identity.size,
            modified_ns: file.identity.modified_ns,
        },
        sha256: file.sha256,
    }
}

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
fn encode_png(image: &RgbaImage) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&image.pixels)?;
        writer.finish()?;
    }
    Ok(bytes)
}

fn digest_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    result
}

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
fn leaf(path: &str) -> Result<String> {
    let mut parts = path.split('/');
    let parent = parts.next();
    let leaf = parts.next();
    ensure!(parent == Some("frames") && leaf.is_some() && parts.next().is_none());
    Ok(leaf.unwrap().to_owned())
}

#[cfg(all(test, target_os = "linux", feature = "media"))]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    struct StoreSink(Arc<SecureStore>, Arc<std::sync::atomic::AtomicUsize>);

    impl apparent_capture_engine::materials_media::DerivativeOutputSink for StoreSink {
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
            self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut file = store.file(&leaf, false)?;
            let metadata = file.metadata()?;
            let mut content = Vec::new();
            file.read_to_end(&mut content)?;
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
                sha256: digest_hex(&Sha256::digest(content)),
            })
        }
    }

    #[test]
    fn frame_paths_are_confined_to_the_frames_store() {
        assert_eq!(
            super::leaf("frames/frame-001.png").unwrap(),
            "frame-001.png"
        );
        assert!(super::leaf("../receipt.json").is_err());
        assert!(super::leaf("frames/../receipt.json").is_err());
        assert!(super::leaf("receipt.json").is_err());
    }

    #[test]
    fn retained_receipt_id_is_stable_for_canonical_replay_input() {
        let request = br#"{"jobId":"f","limits":{"deadlineMs":1}}"#;
        assert_eq!(derived_receipt_id(request), derived_receipt_id(request));
        assert_ne!(
            derived_receipt_id(request),
            derived_receipt_id(b"different")
        );
    }

    #[test]
    fn executes_real_h264_to_v2_frames_and_separate_contact_sheet() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let nonce = format!(
            "{:032x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root_path = std::env::temp_dir().join(format!("engine-materials-media-{nonce}"));
        std::fs::create_dir(&root_path).unwrap();
        std::fs::set_permissions(&root_path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let source_path = root_path.join("retained");
        std::fs::create_dir(&source_path).unwrap();
        std::fs::set_permissions(&source_path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let movie_path = source_path.join("recording.mp4");
        // A decode-only producer must not require an encoder plugin at test time.
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("core/tests/fixtures/tiny.mp4"),
            &movie_path,
        )
        .unwrap();
        let source = File::open(&movie_path).unwrap();
        let source_meta = source.metadata().unwrap();
        let source_bytes = std::fs::read(&movie_path).unwrap();
        let source_sha = digest_hex(&Sha256::digest(&source_bytes));
        let source_identity = apparent_capture_engine::materials_media::FileIdentity {
            device: source_meta.dev().to_string(),
            inode: source_meta.ino().to_string(),
            size: source_meta.len(),
            modified_ns: (i128::from(source_meta.mtime()) * 1_000_000_000
                + i128::from(source_meta.mtime_nsec()))
            .to_string(),
        };
        let root = source_path.canonicalize().unwrap().display().to_string();
        let lease_id = "d".repeat(32);
        let material_payload = serde_json::to_vec(&serde_json::json!({
            "schema":"apparent.capture-materials-payload/1",
            "identity":{"captureId":"a".repeat(32),"materialsId":"b".repeat(32)},
            "bundleId":"synthetic", "root":root, "leaseId":lease_id,
            "clock":{"durationMs":2000,"videoOffsetMs":100},
            "video":{"file":"recording.mp4"},
            "files":[{"relativePath":"recording.mp4","identity":{
                "device":source_identity.device,"inode":source_identity.inode,"size":source_identity.size,"modifiedNs":source_identity.modified_ns
            },"sha256":source_sha}],
            "safety":{"safeForProcessing":true}
        })).unwrap();
        let key = MaterialsKey {
            capture_id: "a".repeat(32),
            materials_id: "b".repeat(32),
            revision: digest_hex(&Sha256::digest(&material_payload)),
        };
        let layout = Layout {
            schema: "apparent.contact-sheet-layout/1".into(),
            width: 170,
            height: 45,
            background_rgb: [12, 34, 56],
            fit: "stretch".into(),
            sampling: "legacy-bilinear".into(),
            clipping: "reject".into(),
            tiles: vec![
                contact_sheet::TileRect {
                    request_index: 2,
                    x: 0,
                    y: 0,
                    width: 80,
                    height: 45,
                },
                contact_sheet::TileRect {
                    request_index: 2,
                    x: 90,
                    y: 0,
                    width: 80,
                    height: 45,
                },
            ],
        };
        let output_path = root_path.join("derived");
        let output = Arc::new(SecureStore::create(&output_path).unwrap());
        let output_path = output_path.canonicalize().unwrap().display().to_string();
        let times = vec![0, 150, 150, 2_000];
        let limits = Limits {
            max_frames: 4,
            max_edge: 8,
            max_total_pixels: 64_000,
            deadline_ms: 10_000,
        };
        let request = NativeRequest {
            schema: "apparent.retained-frame-request/1".into(),
            materials: key.clone(),
            lease_id: lease_id.clone(),
            job_id: "f".repeat(32),
            output_directory: output_path.clone(),
            requested_bundle_ms: times.clone(),
            limits: Limits {
                max_frames: limits.max_frames,
                max_edge: limits.max_edge,
                max_total_pixels: limits.max_total_pixels,
                deadline_ms: limits.deadline_ms,
            },
            contact_sheet: Some(layout.clone()),
        };
        let request_bytes = serde_json::to_vec(&request).unwrap();
        let output_meta = output.directory_metadata().unwrap();
        let binding = Binding {
            schema: "apparent.media2-job/1".into(),
            capture_id: key.capture_id.clone(),
            materials: key,
            materials_root: root.clone(),
            materials_root_identity: {
                let metadata = SecureStore::open(&source_path)
                    .unwrap()
                    .directory_metadata()
                    .unwrap();
                OutputIdentity {
                    device: metadata.dev().to_string(),
                    inode: metadata.ino().to_string(),
                }
            },
            lease_id: lease_id.clone(),
            source_video: SealedFile {
                relative_path: "recording.mp4".into(),
                identity: FileIdentity {
                    device: source_identity.device.clone(),
                    inode: source_identity.inode.clone(),
                    size: source_identity.size,
                    modified_ns: source_identity.modified_ns.clone(),
                },
                sha256: source_sha,
            },
            video_offset_ms: 100,
            job_id: request.job_id.clone(),
            receipt_id: "e".repeat(32),
            request_sha256: digest_hex(&Sha256::digest(&request_bytes)),
            requested_bundle_ms: times.clone(),
            limits: registry::Limits {
                max_frames: limits.max_frames,
                max_edge: limits.max_edge,
                max_total_pixels: limits.max_total_pixels,
                deadline_ms: limits.deadline_ms,
            },
            output_root: output_path.clone(),
            output_identity: OutputIdentity {
                device: output_meta.dev().to_string(),
                inode: output_meta.ino().to_string(),
            },
            layout: Some(layout.clone()),
        };
        let mut changed_limits = request.clone();
        changed_limits.limits.deadline_ms += 1;
        let changed_bytes = serde_json::to_vec(&changed_limits).unwrap();
        assert!(
            validate_original_request(&changed_bytes, &binding, Some(&layout), &material_payload)
                .is_err()
        );
        let pretty = serde_json::to_string_pretty(&changed_limits).unwrap();
        assert!(
            validate_original_request(
                pretty.as_bytes(),
                &binding,
                Some(&layout),
                &material_payload
            )
            .is_err()
        );
        let mut forged_source = binding.clone();
        forged_source.source_video.sha256 = "0".repeat(64);
        assert!(
            validate_source_binding(&forged_source, &material_payload, &source_identity).is_err()
        );
        let runtime_path = root_path.join("media-runtime");
        std::fs::create_dir(&runtime_path).unwrap();
        std::fs::set_permissions(&runtime_path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = apparent_capture_engine::Runtime::open(&runtime_path).unwrap();
        let lease = runtime
            .material_lease(
                &binding.materials.materials_id,
                apparent_capture_engine::LeaseKind::Read,
            )
            .unwrap()
            .unwrap();
        let registry_root = Arc::new(SecureStore::create(&root_path.join("registry")).unwrap());
        let registry = JobRegistry::open(registry_root).unwrap();
        let mut sink = StoreSink(
            output.clone(),
            Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        );
        let outcome = execute_linux(
            &registry,
            binding.clone(),
            &request_bytes,
            &source,
            &material_payload,
            &source_identity,
            &lease,
            output.clone(),
            &mut sink,
            Some(layout.clone()),
            || Checkpoint::Continue,
        )
        .unwrap();
        let JobOutcome::Complete(native_receipt) = outcome else {
            panic!("unexpected job result")
        };
        let (verified, envelope_bytes) =
            registry.verify_completed(&binding, output.clone()).unwrap();
        assert_eq!(verified.request_sha256, binding.request_sha256);
        assert_eq!(native_receipt.frames.len(), times.len());
        let envelope: Value = serde_json::from_slice(&envelope_bytes).unwrap();
        let payload = &envelope["payload"];
        assert_eq!(payload["schema"], "apparent.media-derivative-payload/2");
        assert_eq!(payload["requestSha256"], binding.request_sha256);
        assert_eq!(payload["frames"].as_array().unwrap().len(), 4);
        assert_eq!(payload["frames"][0]["effectiveVideoMs"], 0);
        assert_eq!(payload["frames"][0]["clampedToStart"], true);
        assert_eq!(
            payload["frames"][1]["file"]["relativePath"],
            "frames/frame-002.png"
        );
        assert_eq!(
            payload["frames"][2]["file"]["relativePath"],
            "frames/frame-003.png"
        );
        assert_eq!(
            payload["frames"][1]["file"]["sha256"],
            payload["frames"][2]["file"]["sha256"]
        );
        assert_eq!(payload["frames"][1]["clampedToEnd"], false);
        assert_eq!(payload["frames"][3]["clampedToEnd"], true);
        for frame in payload["frames"].as_array().unwrap() {
            let bytes = read_sealed_file(
                &output,
                &serde_json::from_value(frame["file"].clone()).unwrap(),
            )
            .unwrap();
            assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
            let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
            let reader = decoder.read_info().unwrap();
            assert_eq!((reader.info().width, reader.info().height), (8, 8));
            assert_eq!(frame["encoding"], "png");
        }
        let sheet = &payload["contactSheet"];
        assert_eq!(sheet["file"]["relativePath"], "contact-sheet.png");
        assert_eq!(sheet["layoutSha256"], layout.sha256().unwrap());
        let sheet_bytes = read_sealed_file(
            &output,
            &serde_json::from_value(sheet["file"].clone()).unwrap(),
        )
        .unwrap();
        let mut decoder = png::Decoder::new(std::io::Cursor::new(sheet_bytes));
        decoder.set_transformations(png::Transformations::IDENTITY);
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (170, 45));
        assert_eq!(
            &pixels[((20 * 170 + 85) * 4)..((20 * 170 + 85) * 4 + 4)],
            &[12, 34, 56, 255]
        );
        assert_eq!(pixels[((20 * 170 + 5) * 4 + 3) as usize], 255);
        assert_eq!(sheet["encoding"], "png");

        // A native cancel after a durable frame write is terminal and keeps the
        // material maintenance lease excluded until this worker returns.
        let cancel_path = root_path.join("cancel-derived");
        let cancel_output = Arc::new(SecureStore::create(&cancel_path).unwrap());
        let cancel_output_path = cancel_path.canonicalize().unwrap().display().to_string();
        let cancel_request = NativeRequest {
            schema: request.schema.clone(),
            materials: request.materials.clone(),
            lease_id: request.lease_id.clone(),
            job_id: "9".repeat(32),
            output_directory: cancel_output_path.clone(),
            requested_bundle_ms: vec![300, 300],
            limits: Limits {
                max_frames: 2,
                max_edge: 8,
                max_total_pixels: 64_000,
                deadline_ms: 10_000,
            },
            contact_sheet: None,
        };
        let cancel_request_bytes = serde_json::to_vec(&cancel_request).unwrap();
        let cancel_meta = cancel_output.directory_metadata().unwrap();
        let mut cancel_binding = binding.clone();
        cancel_binding.job_id = cancel_request.job_id.clone();
        cancel_binding.receipt_id = "8".repeat(32);
        cancel_binding.request_sha256 = digest_hex(&Sha256::digest(&cancel_request_bytes));
        cancel_binding.requested_bundle_ms = cancel_request.requested_bundle_ms.clone();
        cancel_binding.limits = registry::Limits {
            max_frames: 2,
            max_edge: 8,
            max_total_pixels: 64_000,
            deadline_ms: 10_000,
        };
        cancel_binding.output_root = cancel_output_path;
        cancel_binding.output_identity = OutputIdentity {
            device: cancel_meta.dev().to_string(),
            inode: cancel_meta.ino().to_string(),
        };
        cancel_binding.layout = None;
        let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut cancel_sink = StoreSink(cancel_output.clone(), writes.clone());
        let cancelled = execute_linux(
            &registry,
            cancel_binding.clone(),
            &cancel_request_bytes,
            &source,
            &material_payload,
            &source_identity,
            &lease,
            cancel_output.clone(),
            &mut cancel_sink,
            None,
            || {
                if writes.load(std::sync::atomic::Ordering::SeqCst) > 0 {
                    Checkpoint::Cancel
                } else {
                    Checkpoint::Continue
                }
            },
        );
        match cancelled {
            Err(error) => assert!(error.to_string().contains("cancelled")),
            Ok(_) => panic!("cancelled job unexpectedly completed"),
        }
        assert!(matches!(
            registry
                .begin(cancel_binding, cancel_output.clone())
                .unwrap(),
            JobOutcome::ExistingFailed("cancelled")
        ));
        assert!(
            runtime
                .material_lease(
                    &binding.materials.materials_id,
                    apparent_capture_engine::LeaseKind::Maintenance
                )
                .unwrap()
                .is_none()
        );

        // Frame output fits by itself, while the requested sheet pushes the
        // combined inventory over the caller's explicit output pixel budget.
        let budget_path = root_path.join("budget-derived");
        let budget_output = Arc::new(SecureStore::create(&budget_path).unwrap());
        let budget_output_path = budget_path.canonicalize().unwrap().display().to_string();
        let mut budget_request = request.clone();
        budget_request.job_id = "7".repeat(32);
        budget_request.output_directory = budget_output_path.clone();
        budget_request.limits.max_total_pixels = 7_800;
        let budget_request_bytes = serde_json::to_vec(&budget_request).unwrap();
        let budget_metadata = budget_output.directory_metadata().unwrap();
        let mut budget_binding = binding.clone();
        budget_binding.job_id = budget_request.job_id.clone();
        budget_binding.receipt_id = "6".repeat(32);
        budget_binding.request_sha256 = digest_hex(&Sha256::digest(&budget_request_bytes));
        budget_binding.limits.max_total_pixels = 7_800;
        budget_binding.output_root = budget_output_path;
        budget_binding.output_identity = OutputIdentity {
            device: budget_metadata.dev().to_string(),
            inode: budget_metadata.ino().to_string(),
        };
        let mut budget_sink = StoreSink(
            budget_output.clone(),
            Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        );
        let budget_result = execute_linux(
            &registry,
            budget_binding.clone(),
            &budget_request_bytes,
            &source,
            &material_payload,
            &source_identity,
            &lease,
            budget_output.clone(),
            &mut budget_sink,
            Some(layout.clone()),
            || Checkpoint::Continue,
        );
        match budget_result {
            Err(error) => assert!(error.to_string().contains("combined output pixel limit")),
            Ok(_) => panic!("over-budget sheet unexpectedly completed"),
        }
        assert!(budget_output.file("contact-sheet.png", false).is_err());
        assert!(budget_output.file("receipt.json", false).is_err());
        assert!(matches!(
            registry
                .begin(budget_binding, budget_output.clone())
                .unwrap(),
            JobOutcome::ExistingFailed("failed")
        ));
        drop(lease);
        assert!(
            runtime
                .material_lease(
                    &binding.materials.materials_id,
                    apparent_capture_engine::LeaseKind::Maintenance
                )
                .unwrap()
                .is_some()
        );
        drop(runtime);
        drop(registry);
        drop(sink);
        drop(output);
        drop(cancel_sink);
        drop(cancel_output);
        drop(budget_sink);
        drop(budget_output);
        std::fs::remove_dir_all(root_path).unwrap();
    }
}
