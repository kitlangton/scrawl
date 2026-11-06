# Engine rebuild acceptance

This inventory retains the full Engine assignment across prerequisite merges.
Ownership follows [the accepted boundaries](../shared/docs/value-stream-boundaries.md):
Record owns standard output and lifecycle policy; Agent owns command/proof policy;
Engine supplies native execution, facts, resource lifetime and safe publication.
Existing root implementations remain authoritative until replacements and callers
are verified. A source package or producer build does not establish adoption.

| Scoped capability | Current standalone state | Required acceptance / consumer return condition |
| --- | --- | --- |
| Installable runtime and discovery | The capture-v1 producer remains unavailable. The packaged Linux media/1 runtime and discovery were delivered at PR722 (`463f0053`), with exact-artifact handoff to Record and Agent. SDK0.3.2 retains capture/2 preflight and adds optional stroke RGB. The capture/2 producer and native integration merged in PR788 (`4ad3cb923`); Platform registered its separate Linux producer in PR794 (`5ed32afd4`). Exact archive admission and consumer adoption remain separate gates. | Preserve published packages/contracts. Platform retains the admitted-artifact return; Engine retains consumer capability handoffs and broader platform acceptance. |
| Real capture and readiness | Capture-v1 remains unavailable. PR788 moved Recorder/Source and Store into Core for real X11 display/preset-area capture, first-frame readiness, cold authenticated controls, retained sealed materials and crash/EOF reconciliation. Linux synthetic guest proof passed; broader platform/product adoption remains open. | Real first-frame readiness, authoritative permission/source facts, authenticated start/inspect/stop/cancel, durable valid native artifact, EOF/crash reconciliation and consumer integration on macOS/Linux. Never substitute a reference-peer receipt. |
| Native input, picker, drawing and capture exclusion | No standalone published capability. Platform owns the existing permission/shortcut Host transport; Record owns workflow and local-only/pen policy. | Publish actual input/geometry/exclusion facts and enforced native operations, with explicit unavailable methods. Coordinate existing Host mechanics rather than duplicate them. A stored preference or admission lock is not shortcut reservation, accepted drawing or local-only proof. |
| Frame decoding and media measurements | SDK 0.2 `media/1` and Linux worker are implemented; macOS decoder is absent. | Installable Linux runtime/discovery plus real conformance; separate macOS implementation/proof. Preserve source identity, ordering/clocks, leases, bounded jobs, cancellation and durable PNG references. Visual measurements needed by Record policy remain unpublished. |
| Export/render/encode/synthesis | Authoritative behavior remains in `capture/engine/export/` and native adapters; no standalone SDK export operation. | Published native execution for caller-selected crop/zoom/ink/audio/encoding plans, destination/source protection, bounded cancellation and validated durable media on claimed platforms. Record/Agent keep their respective export recipes and audio choices. |
| Transcription, reprocessing and recovery | Legacy native inference/recovery exists; no standalone SDK seam. | Provider/model readiness and bounded inference/media/recovery operations, stable time-aligned results and safe publication/reconciliation. Record assembles standard bundles; Agent calls Record's headless API for standard regeneration and Engine for specific native operations. |
| Retention, pinning and active-use protection | Staged native read/maintenance leases exist. Legacy retention does not participate; there is no published SDK maintenance/pinning API. | Bounded, identity-checked filesystem/media operations enforcing live-reader/capture/delivery exclusions and durable pin/publication rules. Record supplies eligibility/scheduling policy; preserve existing pending-transfer protections and verify participating readers/writers before claiming coordination. |

## Extraction and donor removal

Move existing native execution into top-level `engine/`, with minimal adapters;
do not retain a permanent Engine dependency on `capture/`. The core crate and tests
have moved, and capture/2 now consumes its recorder and storage primitives.
The current slice removes the media worker's duplicate decoder. No native
reuse percentage is claimed. Published SDK and Record output ownership remain.

| Donor | Move/reuse destination and unavoidable boundary | Verification / removal condition |
| --- | --- | --- |
| `capture/engine/{src,tests,Cargo.toml,Cargo.lock}` | Existing crate moves to `engine/core`; no new execution or storage behavior in this slice. | All build/test/package references point to the moved crate; source/tests preserved and old crate files removed. |
| `capture/desktop/linux/capture/media.rs` recorder section | Moved existing GStreamer `Source`, encoder selection, recorder pipeline, crop, RGBA extraction, first-video-frame timing and EOS finish into `engine/core/src/linux/recorder.rs` (`linux-media` feature). | Desktop retains selection, portal negotiation, sampler scheduling, window-mask policy and logs/warnings. Engine owns the moved `Stream` guard inside `Source`, preserving the donor's FD/session ownership through finish/drop. Recorder inputs include dimensions/path/clock origin; it returns candidate bytes/timing/result and applies current masks to a copy of unmasked cached pixels on every sampling call. Focused feature build/tests, root caller coverage and isolated Linux guest capture are required; no legacy recorder implementation remains to remove. This does not provide standalone capture-v1. |
| `capture/desktop/linux/capture/media.rs` reader/probe/audio section | Reuses `VideoReader`, seeks, RGBA stride handling, `shrink_to_edge`, probe facts and first-track 16 kHz PCM extraction in `engine/core/src/linux/media.rs` (`linux-media`). Engine exposes neutral frame/probe values; Desktop converts to `RgbaFrame`/helper `Probe` and retains the WAV encoder/writer. | Focused native fixture proof must show decoded frame order/dimensions/timestamps and PCM leading-silence alignment; existing helper/media/probe consumer tests and root build cover the adapters. Desktop retains window masking, warnings, sampler, iPhone shrink behavior and output policy. No SDK media/1 or capture/2 adoption is implied. |
| Linux `runtime/recording/recorder.rs::capture_size` arithmetic | Reuses the existing encoder sizing arithmetic in neutral `engine/core::capture_dimensions`. Linux Desktop delegates through its existing target adapter; capture/2 uses it to resample odd physical rectangles into valid even encoded dimensions, preserving the confirmed physical rectangle separately. | Native 1366×768 source / 683×384 preset-area repro failed before the adapter and passes with 682×384 encoded output. Existing non-Linux callers retain their donor until their native adoption; remove that remaining body when those callers consume Core. |
| `capture/engine/bundle/bundle_processing/publication/movie.rs` and synthetic fixtures | Physically moved the existing descriptor-based ISO-BMFF validator to `engine/core/src/publication/movie.rs`; both legacy validation callers use the Core API. Fixtures and all test includes now point to Core. No new container parser or decoder. | Core MP4/MOV/truncation tests, root publication tests and root build cover the move. Mac runs exercise the same Core parser. Caller leases, full-profile validation, authenticated staging, durable no-replace commit and recovery remain separate provider gates; this API alone cannot establish Saved. |
| `capture/desktop/linux/capture/portal.rs` | Move stream mechanics into Engine with explicit selection/consent inputs; retain product policy with its owner. | Actual selected-output identity and portal consent proof; convert callers before deleting donor code. |
| `engine/native/{Media,Export,LiveTranscribe}.swift` plus their maintained tests | Moved byte-for-byte into the Engine owner; `build.rs` and Mac source preparation consume these paths. Media, export and live transcription remain legacy helper capabilities; helper basename ordering, flags, resource packaging and routing are preserved. | Reuse existing behavior and tests; verify relocated Swift closure on Mac and root build. Remaining `capture/engine/native/{Mirror,PlaybackVolume,ScreenProbe}.swift` stay in place. |

New durable protocol identity/reconnect adapters may be necessary; new capture,
media or filesystem execution requires a concrete gap in the existing donor.
This extraction does not close publication, macOS, export, inference or retention.

The Linux media/1 worker now uses Core's `PinnedVideoReader` for descriptor-bound
GStreamer decode, seek, timestamp and RGBA extraction, with Core enforcing the
source edge/byte bounds before copying. The worker retains a small adapter for its
published output pixel budget and byte-compatible box shrink, while its lease,
identity, job and publication policy remain local. Legacy path-based Desktop
decode/probe/PCM callers continue through `VideoReader`. Engine's optional `media`
feature selects Core's existing `linux-media` feature on Linux; neither the Core
default closure nor the Rust-free SDK closure changes. Remaining
donors include Desktop's warning/sampler/window-mask policy, WAV file encoding, iPhone
re-encoding and portal/source negotiation. No Rust reuse percentage is asserted.

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

The preserved capture/2 integration is being ported selectively onto these
moved donors. Its separate GStreamer capture and validation pipelines have been
removed: it uses `Recorder` with a pinned output-file adapter and the existing
reader for full-file readback. The adapter is required because the SDK materials
contract binds an exclusively created file identity rather than a writable path.
Cold controls use Core's anchored per-capture owner lock and metadata transaction;
a durable v2 cancel bit defaults false when reading earlier journals. This is
cooperative journal control, not a second capture registry. Core storage
extensions and durable protocol glue are preserved from the earlier reviewed
checkpoint, not substituted for Record output policy. The closed Linux package
and SDK peer passed on private Xvfb and in the isolated guest; the guest retained
its source/worker identity and materials and completed cleanup. Independent
source review and a real fdsink teardown regression establish borrowed-FD
ownership. Current-base integration and producer delivery admission remain gates;
no installed readiness is claimed.

## Recorded stroke color prerequisite

The bounded color addition reuses the existing `shared/model.rs::Stroke`, clip/
rebase, `capture/engine/export/ink.rs` plans and software/native export renderers.
`rgb?: [u8;3]` records exact bytes per stroke; absent preserves the unnamed legacy
renderer defaults and serialized legacy stroke/plan bytes. The legacy telemetry
setter/getter select one atomic native value and snapshot it at stroke begin (pins
at acceptance); no Engine palette mapping or config writer is introduced. Platform
owns process-scoped application/readback and overlays; Record owns canonical
assembly of optional RGB and settings. Whiteboard UI adaptation is not inferred
from this screen-stroke boundary. These native donors still need physical moves
under `engine/`; this compatible prerequisite does not introduce a parallel renderer
or claim a standalone drawing/export operation or installed adoption.

Focused proof covers exact RGB serialization/rebase/clipping, packed SDK 0.3.2
imports/types, software PNG pixels, real Linux H.264 export, and the complete
Swift helper build plus file-only native Mac stroke/pin/laser rendering. Explicit
RGB uses sRGB on Mac; device RGB otherwise converts through the display profile.
Absent-color Mac raw output was compared byte-for-byte with the original renderer.
No screen/microphone capture or installed routing was exercised for this color
slice; standalone capture/2 accepted-stroke collection remains unavailable.
