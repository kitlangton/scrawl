# Retained-material derivative/2 contract

Public SDK export: `@apparent/engine-sdk/media2` (SDK 0.4.0). This slice publishes
types and offline validators for Record's additive staging adapter. It supplies
no transport, native job authority, launch route or decoder availability claim.
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
