# Engine rebuild acceptance

This inventory retains the full Engine assignment across prerequisite merges.
Ownership follows [the accepted boundaries](../shared/docs/value-stream-boundaries.md):
Record owns standard output and lifecycle policy; Agent owns command/proof policy;
Engine supplies native execution, facts, resource lifetime and safe publication.
Existing root implementations remain authoritative until replacements and callers
are verified. A source package or producer build does not establish adoption.

| Scoped capability | Current standalone state | Required acceptance / consumer return condition |
| --- | --- | --- |
| Installable runtime and discovery | The capture-v1 producer remains unavailable. The packaged Linux media/1 runtime and discovery were delivered at PR722 (`463f0053`), with exact-artifact handoff to Record and Agent. SDK0.3.1 adds capture/2 document/client preflight, without native capture/2 availability. | Preserve published packages/contracts. The repaired capture/2 candidate is unmerged; its native adoption and platform gates remain open. |
| Real capture and readiness | Capture-v1 admission, durable identity/receipts and SDK exist. Native handshake remains unavailable; starts fail without media. | Real first-frame readiness, authoritative permission/source facts, authenticated start/inspect/stop/cancel, durable valid native artifact, EOF/crash reconciliation and consumer integration on macOS/Linux. Never substitute a reference-peer receipt. |
| Native input, picker, drawing and capture exclusion | No standalone published capability. Platform owns the existing permission/shortcut Host transport; Record owns workflow and local-only/pen policy. | Publish actual input/geometry/exclusion facts and enforced native operations, with explicit unavailable methods. Coordinate existing Host mechanics rather than duplicate them. A stored preference or admission lock is not shortcut reservation, accepted drawing or local-only proof. |
| Frame decoding and media measurements | SDK 0.2 `media/1` and Linux worker are implemented; macOS decoder is absent. | Installable Linux runtime/discovery plus real conformance; separate macOS implementation/proof. Preserve source identity, ordering/clocks, leases, bounded jobs, cancellation and durable PNG references. Visual measurements needed by Record policy remain unpublished. |
| Export/render/encode/synthesis | Authoritative behavior remains in `capture/engine/export/` and native adapters; no standalone SDK export operation. | Published native execution for caller-selected crop/zoom/ink/audio/encoding plans, destination/source protection, bounded cancellation and validated durable media on claimed platforms. Record/Agent keep their respective export recipes and audio choices. |
| Transcription, reprocessing and recovery | Legacy native inference/recovery exists; no standalone SDK seam. | Provider/model readiness and bounded inference/media/recovery operations, stable time-aligned results and safe publication/reconciliation. Record assembles standard bundles; Agent calls Record's headless API for standard regeneration and Engine for specific native operations. |
| Retention, pinning and active-use protection | Staged native read/maintenance leases exist. Legacy retention does not participate; there is no published SDK maintenance/pinning API. | Bounded, identity-checked filesystem/media operations enforcing live-reader/capture/delivery exclusions and durable pin/publication rules. Record supplies eligibility/scheduling policy; preserve existing pending-transfer protections and verify participating readers/writers before claiming coordination. |

## Extraction and donor removal

Move existing native execution into top-level `engine/`, with minimal adapters;
do not retain a permanent Engine dependency on `capture/`. The immediate slice
moves the existing core crate and tests byte-for-byte (except documentation).
Unmerged capture/2 repairs and their evidence are preserved separately. No native
reuse percentage is claimed. Published SDK and Record output ownership remain.

| Donor | Move/reuse destination and unavoidable boundary | Verification / removal condition |
| --- | --- | --- |
| `capture/engine/{src,tests,Cargo.toml,Cargo.lock}` | Existing crate moves to `engine/core`; no new execution or storage behavior in this slice. | All build/test/package references point to the moved crate; source/tests preserved and old crate files removed. |
| `capture/desktop/linux/capture/media.rs` | Moved existing GStreamer `Source`, encoder selection, recorder pipeline, crop, RGBA extraction, first-video-frame timing and EOS finish into `engine/core/src/linux/recorder.rs` (`linux-media` feature). | Desktop retains selection, portal negotiation, sampler scheduling, window-mask policy, logs/warnings and reader/export/audio behavior. Engine owns the moved `Stream` guard inside `Source`, preserving the donor's FD/session ownership through finish/drop. Recorder inputs include dimensions/path/clock origin; it returns candidate bytes/timing/result and applies current masks to a copy of unmasked cached pixels on every sampling call. Focused feature build/tests, root caller coverage and isolated Linux guest capture are required; no legacy recorder implementation remains to remove. This does not provide standalone capture-v1. |
| `capture/desktop/linux/capture/portal.rs` | Move stream mechanics into Engine with explicit selection/consent inputs; retain product policy with its owner. | Actual selected-output identity and portal consent proof; convert callers before deleting donor code. |
| `capture/engine/native/{Media,Export,LiveTranscribe}.swift` | Adapt/package existing native execution under `engine/`; new code limited to demonstrated SDK/ownership gaps. | Preserve active reservations; affected macOS compilation/native proof and converted build/package inputs before donor removal. |

New durable protocol identity/reconnect adapters may be necessary; new capture,
media or filesystem execution requires a concrete gap in the existing donor.
This extraction does not close publication, macOS, export, inference or retention.

## Contracts and return routes

- The SDK remains independently installable/buildable/testable without Rust.
  Strict capture-v1 remains screen-only and `media/1` remains its published frame
  contract. New operations require explicit compatible negotiation/exports; do
  not silently add methods or weaken saved/readiness semantics.
- Engine retains direct owner-to-owner notification commitments to Record and
  Agent for each concrete usable capability. A handoff names committed SDK/export,
  native artifact and launch identity, discovery, platform evidence, unavailable
  methods and remaining gates. Generic task/CI completion does not clear a wait.
- Record retains consumer integration and the Onboarding-provider handoff. Agent
  retains CLI integration. Platform retains app assembly/installed discovery and
  its Host transport. Engine retains its native implementation and relevant
  post-merge results; missing external effects must name their actual producer.
- Adoption requires verified real consumers and applicable platform/artifact
  evidence. Track source-only, fixture-only and installed behavior separately.

## Preserved limits

iPhone discovery/mirroring/recording/shrinking is explicitly deferred and is not
an acceptance gate; preserve existing recordings/readers. Windows is unsupported.
Connect delivery/retrieval and remote lifecycle remain Connect-owned and excluded
from the current Agent rebuild. No new capture forwarding, installed activation,
legacy deletion, access or spending follows from this inventory. Existing consent,
named native-resource restrictions, budgets and publication gates still apply.
