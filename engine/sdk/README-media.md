# Engine media SDK seam (media protocol 1)

`@apparent/engine-sdk/media` is a Rust-free, additive interface for local media
decoding. It is deliberately a different channel and executable from capture
protocol v1. `capture-v1` peers must reject `channel: "media"` as an unknown
channel/version; they must not treat these calls as capture methods. The SDK
does not advertise a native worker or native availability until a worker exists.

## Consumer example

```ts
import { createMediaJobId, launchMediaWorker, MediaClient } from "@apparent/engine-sdk/media";

const transport = launchMediaWorker(verifiedFramesBinary, privateRuntimeDirectory);
const media = new MediaClient(transport);
await media.handshake();
const jobId = createMediaJobId();
const result = await media.decode({
  jobId,
  source: {
    bundleId: "capture-123.apparent",
    bundlePath: "/captures/capture-123.apparent",
    videoPath: "/captures/capture-123.apparent/recording.mov",
    videoOffsetMs: 250,
    identity: { device: "2049", inode: "40012", size: 8000000, modifiedNs: "1783212345000000000" },
  },
  requestedBundleMs: [1250, 1000, 1250],
  outputDirectory: "/private/runtime/media-job-7/output",
  limits: { maxFrames: 48, maxEdge: 1280, maxTotalPixels: 64000000, deadlineMs: 30000 },
});
// frames[0..2] retain requestIndex and requested bundle timestamps. In this
// example effectiveVideoMs are 1000, 750, 1000 respectively.
await media.close();
```

`launchMediaWorker` executes only the caller-verified absolute binary, with
`--media-runtime-directory DIR --media-channel media --media-protocol-version 1`;
it does not search PATH, build Rust, or fall back to source. The caller provisions
one private persistent runtime directory and uses the same directory for all
worker instances that coordinate bundle leases. Standard output is protocol
only; standard error is drained and not exposed. JSONL has a 64 KiB maximum line
and up to 32 pending RPCs. There is no media payload in JSON.

## Request, result and mapping

Every envelope has the literal `channel: "media"`, `protocolVersion: 1`,
32-hex `clientId` and fresh per-call 32-hex `rpcId`. Methods are `handshake`,
`decode` and `cancel`. A decode also has a fresh 32-hex `jobId`, explicit
`bundleId`, absolute bundle and video paths, nonnegative `videoOffsetMs`, and
caller-supplied video file identity (`device`, `inode`, `size`, nanosecond
modification time). The native peer pins and checks that identity before and
after the read; it rejects unfinished/corrupt bundles, path swaps, missing or
mismatched media, and output path collisions. The identity metadata is a
consistency key, not authorization to access arbitrary paths.

`requestedBundleMs` is an ordered array of 1–48 request slots. Values
are safe nonnegative integers and may repeat or be out of order. The SDK
preserves both. For each request, `effectiveVideoMs = requestedBundleMs -
videoOffsetMs`; pre-offset timestamps are rejected, not saturated to zero.
The native peer may clamp a post-duration request to the last decodable frame,
and reports `clampedToEnd`, `effectiveVideoMs` and optional decoder-reported
`actualVideoMs` separately. There is no promise of exact frame position when a
decoder seeks approximately. Results carry 1-based `requestIndex` plus the
original requested timestamp; no nearest-time remapping or deduplication is
allowed. A decoder must return one result per request or fail the whole job.

The caller owns selection of timestamps, phrase/mark windows, ordering, zoom
crop and output policy. It supplies finite positive limits: 1–48 frames, maximum
edge 1–2048 px, aggregate returned pixels 1–64,000,000 and deadline 1–60,000 ms.
The worker enforces independent hard ceilings and bounded decoder concurrency,
memory, subprocesses and scratch space; a caller limit can only narrow them.
Zero never means unlimited. The SDK validates request structure and returned
inventory bounds but does not claim the PNGs exist or are durable; only the
native provider's verified filesystem publication can do that.

Successful decode returns only PNG references: request index/times, absolute
PNG path, dimensions, byte length and SHA-256. No image bytes cross the JSON
transport. The destination must be a fresh private directory owned by the job.
The native provider refuses overwrite/symlink/path escape, validates each PNG,
syncs each file and directory, then atomically publishes a complete job
manifest. References are emitted only after that barrier. No partial success is
returned. Failed, cancelled, timed-out or disconnected work has no completed
inventory. Provider-owned cleanup may remove only files recorded in its own
private job manifest; never recursively clear caller-selected paths.

## Lifecycle, exclusion and failures

`handshake` confirms the media marker/version and implementation. Each decode
is a separate job lifecycle: accepted → decoding → (complete | failed |
cancelled). Cancellation addresses `jobId` in a fresh RPC; `cancelling` is only
an acknowledgment of the request, not proof of cancellation. The provider owns
worker termination, descriptor release, lease release and scratch cleanup.
Callers can generate and retain a job ID with `createMediaJobId()` before
starting decode, then issue `cancel(jobId)` while its decode RPC is pending.
Deadline or client RPC timeout does not send cancel and cannot manufacture a
complete/partial result. To retry, the caller creates a new job ID and a fresh
empty output directory after observing/cleaning up the prior job according to
provider status. RPC correlation is independent of job identity; late replies
cannot resolve another call.

Provider work acquires the existing published `Runtime.lease(Read)` bundle
lease and retains it through source validation, actual decode, PNG publication
and manifest sync. It does not create another lease/store mechanism. The lease
is released only after provider-owned file descriptors and decoder work stop.
This read/maintenance exclusion is separate from capture job lifecycle and
global live-capture admission: media decode can run while capture admission is
occupied and must never contend for the live-capture lock. It requests no screen
permission, portal, capture start or single-live-capture slot.

Stable error codes include `source_mismatch`, `source_incomplete`,
`invalid_output`, `limits_exceeded`, `decode_failed`, `cancelled`,
`deadline_exceeded`, `busy`, `unsupported_channel`, `unsupported_version` and
`internal`. Messages are bounded and generic; they must not disclose private
paths or media contents. Unknown fields and protocol markers fail closed.

## Reference peer and compatibility

`@apparent/engine-sdk/media/testing` exposes an explicit test-only
`ReferenceMediaPeer`; use `new MediaClient(peer, { allowTestDouble: true })`.
Its references are synthetic and are not PNG files or native proof. The
production default rejects a `test-double` handshake. It supports success,
partial decode failure, cancellation, timeout, source mismatch and output
collision scenarios. This fixture is for consumer state/error handling; native
conformance must separately validate real output bytes, durability, leases,
cancellation, resource ceilings and macOS/Linux decoding.

This package release is additive `0.2.0`. Capture v1 types, methods, exports and
wire remain unchanged. Install and test the package without Cargo using the
existing SDK `npm ci`, `npm run typecheck`, `npm test`, `npm pack` commands. A
clean consumer installs the tarball and imports both `@apparent/engine-sdk`
and `@apparent/engine-sdk/media` with declarations. The testing subpath is
explicit and separate.

## Source behavior reconciliation

Legacy `extract_frames_range` validates a range/FPS and its direct FFmpeg path
seeks on its input timestamps, while its native fallback maps bundle times by
subtracting the video offset. The new seam makes the intended bundle clock
explicit and consistently subtracts `videoOffsetMs`; it does not preserve the
inconsistent direct-branch interpretation. Legacy `zoom::sample_times` chooses
up to 48 times, but selection is Agent policy and remains with the caller.
Legacy `movie_frames_at_with` uses saturating subtraction and nearest-request
remapping, which loses duplicate/order identity. The new seam rejects negative
effective times and carries each request index/time through decode, including
duplicates; post-end clamping is explicit rather than presented as exact seek.

## Rust peer conformance fixture

Use the exported TypeScript types and `validateMediaRequest` /
`validateMediaResponse` as consumer fixtures, not private transport hooks. A
conforming peer must cover these cases against the real protocol implementation:

1. Unsupported `capture` channel and media protocol version are rejected before
   method dispatch; capture v1 remains strict and unchanged.
2. Source identity mismatch, unfinished/corrupt input, path replacement,
   existing output, output symlink/path escape and invalid PNG fail closed.
3. Offset mapping with a nonzero offset; negative pre-offset rejection;
   post-duration clamp reporting; unsorted timestamps and duplicates preserve
   request indices and all entries.
4. Partial decoder error produces no success inventory; cancellation racing a
   completed result has one authoritative terminal result; deadline and late
   RPC response cannot become success.
5. RPC IDs are fresh per call even for a retried job. Bundle read lease lasts
   through decode/publication, maintenance exclusion works, capture admission
   can be occupied concurrently, and decode resource/deadline limits are hard.
6. Returned paths refer to validated PNG bytes inside the fresh private output
   directory; file+directory durability precedes atomic manifest publication
   and response. Test synthetic refs never pass as native evidence.

The separate `apparent-engine-frames` Rust executable and its native decoder,
provider manifest/inventory, platform distribution and real decode proofs are
not implemented by this SDK contract checkpoint.
