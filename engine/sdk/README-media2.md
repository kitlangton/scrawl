# Retained-material derivative/2 contract

SDK0.4.2 adds the Rust-free offline export
`@apparent/engine-sdk/observations: readFrameVisualObservations(input)`.
Supply the original `document`, validated `materials`, ordered 1-based
`requestIndex`, sealed `pngBytes` and explicit `maxFileBytes` budget. It validates
the original derivative request/layout/material/lease binding and full PNG seal,
then returns raw measurements plus that source identity, clock and collection
references. It performs no I/O, decode, selection, dispatch or material release.
Offline consistency is not native issuance: retain authenticated Media2Client
completion and native registry authority for publication.

Optional media/2 frame PNG profile `apOB` is ancillary, private and unsafe to copy
after image transformations. A native producer measures the exact final decoded
RGBA8 plane immediately before lossless PNG encoding; no later resize or colour
rewrite is permitted before sealing. Contact sheets and media/1 are unchanged.
The profile is covered by the existing PNG hash, byte length and file identity.
Old jobs/outputs return `state: 'unavailable'` with reason
`observation-profile-absent`; recognized future versions return unsupported.
Malformed, contradictory or duplicate profiles throw, including bad CRC, dimensions,
length, grid shape and nonfinite/out-of-range cells. PNG chunk walking is bounded
to 16,384 chunks and the caller's file budget; no pixel decompression occurs here.
The existing Node>=22.6 floor is retained: the CRC primitive is
[`node:zlib.crc32`, available since Node22.2](https://nodejs.org/download/release/v22.23.0/docs/api/zlib.html#zlibcrc32data-value).

Profile bytes are ASCII `APOBS001`, then little-endian width/height u32, region
columns/rows u16, 2,304 thumbnail f32 cells, then columns*rows region f32 cells.
The maximum is 74,772 bytes. Dimensions must match both PNG IHDR and the selected
receipt; regions are exactly min(width,128) by min(height,128). All cells are
row-major native float32 in [0,1], exposed unchanged as JS numbers. PNG lengths
and CRC retain standard big-endian encoding.
The single `apOB` chunk follows the complete IDAT sequence and precedes IEND;
the reader rejects an observation block before image data or interleaved IDAT.

The 64×36 thumbnail samples cell centres using native f32 encoded-RGB
`(0.2126R+0.7152G+0.0722B)/255`, ignoring alpha, without gamma linearization.
The region grid preserves the donor's integer luma `((54R+183G+19B)>>8)`, sampled
cells as `f32(sum)/255.0/f32(max(count,1))` in that native f32 operation order,
with step `max(floor(max(width,height)/512),1)`. Thumbnail products, left-associated
sum and final division likewise use f32 arithmetic; no f64 recomputation is stored.
These observations describe the requested **decoded derivative dimensions**:
resizing does not promise numeric or selector parity with unscaled donor input.
Record chooses an adequate common decode size and compatible planes, owns all
scene/region comparisons, thresholds, grouping, novelty and final frame policy,
and may use `Math.fround` for donor comparison fidelity. Pixel measurements do
not infer interactions, marks or strokes. Capture candidates remain unavailable
until a real collector exists. The SDK export alone establishes no native
profile availability, standard selector readiness or supported-platform proof.

Public SDK export: `@apparent/engine-sdk/media2`. SDK0.4.0 froze the types and
offline validators for Record's additive staging adapter. SDK0.4.1 adds an
explicitly negotiated client and process transport. This supplies no native job
authority, qualified runtime artifact or decoder availability by itself.
Capture/2, media/1 and derivative/1 retain their existing shapes. Do not dispatch
derivative2 plans to a producer until its concrete media/2/publication handoff
establishes support; capture/2's publication bit alone does not establish it.

`prepareMedia2Request(input, validatedMaterials)` returns an owned request snapshot,
exact UTF-8 request bytes and SHA256. Its required `contactSheet` is either a layout
or null. Use the original request on reconnect. The digest includes original
materials revision, lease, job, output destination, ordered duplicate selections,
all limits (including the native deadline) and layout. Reuse a job ID only with
that identical tuple; changed limits/layout/path after timeout are conflicts.
Capture owner credentials are supplied separately and never serialized into the
public digest. Native authentication must precede job lookup or receipt recovery.

Canonical request bytes are `JSON.stringify` of the fixed field order produced
by `prepareMedia2Request`, with no whitespace or newline, encoded as UTF-8.
Nested materials, limits, layout and tiles have the explicit field orders in
`request.ts`; there are no floating-point numbers. Layout bytes use the same rule
through `encodeContactSheetLayout`, independently hashed. Neither detached digest
is stored inside the document whose bytes it hashes. Native implementations must
verify the supplied exact bytes and parsed fields, not hash an arbitrary map order.

Request times are integer capture/bundle milliseconds in `[0, durationMs]`.
Frame slots are ordered and 1-based; duplicate times retain separate paths/slots.
`effectiveVideoMs = max(0, requestedBundleMs - videoOffsetMs)` remains the requested
seek even at end clamp. `actualVideoMs` is required decoder readback, separate
from that seek; decoder edit/PTS offsets need not have an inferred numeric ordering
against it. `clampedToStart` denotes a request before the video offset;
`clampedToEnd` reports native last-frame clamping. They cannot both be true.
Record chooses canonical times/reasons and must not apply the offset twice.

The sheet is a separate image, never a video-frame slot. Canvas dimensions and
tile rectangles are integer physical output pixels with origin at top-left;
rectangles are half-open `[x,x+width) × [y,y+height)`. Tiles execute in array order,
can reuse a frame requestIndex, must lie fully inside the canvas, and may not
overlap. `fit: "stretch"` scales the complete referenced decoded frame to the
provided rectangle with no crop or implicit aspect policy; Record computes the
desired geometry. `clipping: "reject"` rejects out-of-bounds rectangles.
`backgroundRgb` is exact 8-bit RGB; uncovered pixels are opaque background.
`sampling: "legacy-bilinear"` reuses `capture/engine/render.rs::blit_scaled`
with radius zero: pixel-center bilinear interpolation with clamped source taps,
and its four quarter-pixel samples when either inverse scale exceeds 1.35.
No captions, badges, semantic text, rounded corners or palette mapping are added.
Outputs are PNGs with 8-bit RGBA pixels; the sheet is opaque.

At most 48 frame requests and 48 sheet tiles, frame maxEdge and each sheet edge
at most 4096, native deadline at most 60,000 ms. `maxTotalPixels` is the **combined
output inventory**: sum of all emitted frame width×height plus sheet width×height,
at most 64,000,000. Source decode allocations and supervised-process memory/CPU
limits remain additional native gates; this output count is not a memory quota.

`validateDerivative2Document({envelope,bytes,request,layoutBytes}, materials)`
checks detached receipt bytes, original request hash, lease, key/revision, job,
receipt, output root, ordered frames and separate sheet/layout. Every image carries
dimensions, PNG encoding, byteLength and the full file seal (relative path,
device/inode/size/modifiedNs, SHA256); paths are relative to receipt.root and unique
across all outputs and receiptFile. `layoutBytes` is null iff no sheet was requested;
otherwise it is the exact canonical original layout bytes, whose digest and parsed
layout are in the authenticated receipt. The payload hash is detached in
envelope.receiptFile; payload and envelope identities must agree exactly.

Offline consistency is not native origin. A supported native producer must own a
durable job record binding that full request and reserved output directory/file
identities, retain the original material read lease throughout decode/composition,
and seal/sync all files and receipt before completion. A caller-written manifest,
matching hashes or same-user ownership do not establish Engine provenance.
OutputDirectory is a caller-selected fresh absolute destination; native reserves
and pins it before work and never adopts an occupied replacement. Same-user OS
ownership is not a sandbox claim. Native publication independently checks the
Engine-issued record, exact receipt and selected output identity. Copying output
bytes into Record staging never changes that native source binding.

Job lifecycle is explicit: running → complete, failed or cancelled. Replaying the
same job returns its durable state/receipt; terminal state is immutable. An RPC
timeout/connection loss does not cancel, rebind, release or delete any output.
Native deadline expiry is a terminal deadline failure, not a refreshed timer on
replay. Inspect/cancel must authenticate the original authority and job/request
binding, remain responsive while execution runs, and reap the supervised worker.
Cancel before completion wins only through its durable decision; completion that
already won is returned unchanged. Release cannot remove active read-leased
materials; completed receipt/output retention must last through publication and
explicit safe cleanup. No maintenance/automatic cleanup authority is added here.

Publication input is `{kind:"derivative2",receipt,selector}` where selector is
`{kind:"frame",requestIndex}` or `{kind:"contact-sheet"}`. The complete receipt
includes receiptId and exact receiptFile.sha256; SDK preflight requires matching
detached bytes plus original request/layout in `CapturePublishInput.derivatives2`.
Selectors must agree with the inventory role, hash and byteLength. Publication
relativePath is the **final bundle path**, while the receipt file path remains
relative to the native derivative root. Record can stage copies for assembly but
must emit these selectors, not reclassify native outputs as assembled authority.
`preflightCapturePublication` validates this before Record reserves dispatch; its
existing original publication expectation and ambiguous-dispatch rules remain.

## Negotiated media/2 client

SDK0.4.1 adds `Media2Client` and `launchMedia2Worker(absoluteExecutable,
privateRuntimeDirectory)`. It uses the existing media process transport and
shutdown/resource-lifetime handling, with protocol version2, a bounded1MiB
metadata line and no fallback to media/1. Launch has the same six arguments,
ending `--media-protocol-version`, `2`. Native artifacts must independently
advertise and qualify this channel before consumers dispatch; old media/1
artifacts do not implement it.
For retained materials, use the same private Engine runtime root that owns the
capture/2 journal. Media/2 must not search other runtimes or infer authority from
caller file paths. Sharing that root does not acquire the live-capture slot.

SDK0.4.4 exports `capture2MaterialsRoot(privateRuntimeDirectory, captureId)` from
`@apparent/engine-sdk/capture`. This is the capture/2 provider's supported expected
path binding: `<runtime>/capture-v2/materials/<captureId>`. Use the exact private,
canonical runtime independently configured for the admitted native launch and the
original capture ID from the host's durable admission/mapping, never values taken
only from a manifest. Capture/2 providers using this layout retain it across
reconnect; future incompatible layouts need a separate versioned contract.

The helper is pure: it validates normalized absolute paths and lowercase32hex
capture IDs, creates nothing, and performs no filesystem discovery, authentication,
alias/ownership check or lease acquisition. It supplies Record's independently
host-trusted expected root; require the authenticated manifest and detached payload
to match it and preserve their capture/material/revision/lease and sealed-file
checks. A matching path alone proves neither native origin nor Saved or safe
release. Retain the original runtime and mapping through recovery. Only the
qualified Linux capture/2 provider is currently available; this helper does not
make Mac capture/2 or another provider available.

`preflightMedia2Dispatch({ownerToken,request,materials}, timeoutMs)` validates
local input before reserving dispatch. The client shares that exact preflight;
local rejection has `dispatched:false`. `derive`, `inspect` and `cancel` accept
that original input and return an observation bound to jobId/requestSha256.
Derive needs native retainedFrames/contactSheet/nativeJobAuthority capabilities;
cold inspect/cancel remain callable when decoding is unavailable. Explicit test
peer opt-in never permits a native derivative receipt. Transport uncertainty has
`dispatched:"unknown"`; it never cancels or chooses a new job identity.

Wire channel is `media`, protocolVersion2, clientId/rpcId lowercase32hex.
Handshake result has implementation and exact retainedFrames/contactSheet/
nativeJobAuthority boolean capabilities. Derive adds captureId/ownerToken/jobId/
requestSha256 and `requestJson` (the exact canonical UTF-8 request represented
as a JSON string). Inspect/cancel carry the same authority/job/hash, without
requestJson. Native authenticates before lookup. Responses carry exactly result
or a structured error code; arbitrary errors/metadata are not forwarded.
Job observations have jobId/requestSha256 and state running, cancelled, failed
with error, or complete with `{derivative:{envelope,receiptJson}}`. ReceiptJson
contains exact detached metadata bytes, never PNG/media bytes. Complete is
validated against the caller's restored original request/layout and returns a
Derivative2DocumentInput ready for publication preflight. It does not mean Saved.
