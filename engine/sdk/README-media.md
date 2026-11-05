# Engine media SDK seam (media protocol 1)

`@apparent/engine-sdk/media` is a Rust-free, additive interface for local media
decoding. It is deliberately a different channel and executable from capture
protocol v1. `capture-v1` peers must reject `channel: "media"` as an unknown
channel/version; they must not treat these calls as capture methods. The SDK
does not advertise a native worker or native availability until a worker exists.

## Consumer example

```ts
import {
  createMediaJobId,
  launchMediaWorker,
  MediaClient,
} from "@apparent/engine-sdk/media";

const transport = launchMediaWorker(
  verifiedFramesBinary,
  privateRuntimeDirectory,
);
const media = new MediaClient(transport);
await media.handshake();
const jobId = createMediaJobId();
const input = {
  jobId,
  source: {
    bundleId: "capture-123.apparent",
    bundlePath: "/captures/capture-123.apparent",
    videoPath: "/captures/capture-123.apparent/recording.mov",
    videoOffsetMs: 250,
    identity: {
      device: "2049",
      inode: "40012",
      size: 8000000,
      modifiedNs: "1783212345000000000",
    },
  },
  requestedBundleMs: [1250, 0, 1000, 1250],
  outputDirectory: "/private/runtime/media-job-7/output",
  limits: {
    maxFrames: 48,
    maxEdge: 1280,
    maxTotalPixels: 64000000,
    deadlineMs: 30000,
  },
};
try {
  const result = await media.decode(input);
  // Request slots preserve order, duplicates and original bundle timestamps.
  // The pre-offset request maps to video time zero with clampedToStart: true.
  void result;
} catch {
  // A decode RPC timeout does not stop the worker. Inspect the original job.
  const status = await media.inspect(input);
  if (status.state === "decoding" || status.state === "accepted") {
    await media.cancel(jobId);
    const afterCancel = await media.inspect(input);
    // This may still be "cancelling". Do not retry or remove output until
    // bounded observation finds a terminal result, or owned shutdown confirms exit.
    void afterCancel;
  }
} finally {
  const closeOutcome = await media.close();
  // A requested signal is not proof that the worker process has exited.
  void closeOutcome;
}
```

`launchMediaWorker` executes only the caller-verified absolute binary, with
`--media-runtime-directory DIR --media-channel media --media-protocol-version 1`;
it does not search PATH, build Rust, or fall back to source. The caller provisions
one private persistent runtime directory and uses the same directory for all
worker instances that coordinate bundle leases. Standard output is protocol
only; standard error is drained and not exposed. JSONL accepts LF or CRLF and
has a 64 KiB maximum line, up to 32 pending RPCs, and a 1 MiB write-buffer
ceiling. Invalid UTF-8 and unknown fields fail closed. There is no media payload
in JSON. `close()` is idempotent and reports `already-exited`, `eof-exited`,
`term-requested-after-timeout` or `kill-requested-after-timeout`; a signal
request is not proof of process reaping.

## Request, result and mapping

Every envelope has the literal `channel: "media"`, `protocolVersion: 1`,
32-hex `clientId` and fresh per-call 32-hex `rpcId`. Methods are `handshake`,
`decode`, `inspect` and `cancel`. A decode requires a caller-retained 32-hex
`jobId`, explicit
`bundleId`, absolute bundle and video paths, nonnegative `videoOffsetMs`, and
caller-supplied video file identity (`device`, `inode`, `size`, nanosecond
modification time). The identity metadata is a consistency key, not
authorization to access arbitrary paths. Native implementation status and
platform acceptance are recorded in the [Engine native worker documentation](https://github.com/tryapparent/apparent/blob/main/engine/README-media.md).
Descriptor pinning, before/after identity checks, bundle validation and path
swap rejection are native conformance gates, independent of SDK validation.

`requestedBundleMs` is an ordered array of 1–48 request slots. Values
are safe nonnegative integers and may repeat or be out of order. The SDK
preserves both. For each request, `effectiveVideoMs = max(0, requestedBundleMs -
videoOffsetMs)`; pre-offset timestamps retain their requested time and set
`clampedToStart`. This preserves legacy saturating subtraction.
`effectiveVideoMs` always remains this offset-mapped requested seek, including
when it exceeds the movie duration. A native peer may decode the last available
frame for a post-duration request and set `clampedToEnd: true`; it must not replace
`effectiveVideoMs` with that clamped position. `actualVideoMs`, when supplied,
reports the decoder position separately. For example, a bundle request at
5250 ms with a 250 ms offset has `effectiveVideoMs: 5000`; an end-clamped decode
may report `actualVideoMs: 1999` and `clampedToEnd: true` for a two-second movie.
There is no promise of exact frame position when a decoder seeks approximately.
Results carry 1-based `requestIndex` plus the
original requested timestamp; no nearest-time remapping or deduplication is
allowed. A decoder must return one result per request or fail the whole job.

The caller owns selection of timestamps, phrase/mark windows, ordering, zoom
crop and output policy. It supplies finite positive limits: 1–48 frames, maximum
edge 1–4096 px, aggregate returned pixels 1–64,000,000 and deadline 1–60,000 ms.
The worker enforces independent hard ceilings and bounded decoder concurrency,
memory, subprocesses and scratch space; a caller limit can only narrow them.
Zero never means unlimited. The SDK validates request structure and returned
inventory bounds but does not claim the PNGs exist or are durable; only a
verified native provider filesystem publication can establish that.

Successful decode returns only PNG references: request index/times, absolute
PNG path, dimensions, byte length and SHA-256. No image bytes cross the JSON
transport. The destination must be a fresh private directory owned by the job:
either an absent path or a caller-provisioned empty mode-0700 directory under an
existing private mode-0700 parent. The native provider claims it exclusively
before writing; existing entries or a stale job claim are refused. Callers do not
precreate PNGs or a completed manifest.
The native provider refuses overwrite/symlink/path escape, validates each PNG,
syncs each file and directory, then atomically publishes a complete job
manifest. References are emitted only after that barrier. No partial success is
returned. Failed and cancelled jobs have no completed inventory. An RPC timeout
or disconnect does not determine the job outcome; an intact worker may later
return a complete inventory through `inspect`. Provider-owned cleanup may remove only files recorded in its own
private job manifest; never recursively clear caller-selected paths.

## Lifecycle, exclusion and failures

`handshake` confirms the media marker/version and implementation. Each decode
is a separate job lifecycle: accepted → decoding → (complete | failed |
cancelled). `inspect` and `cancel` address a job in the same private worker
connection. Statuses are local to that worker connection; the SDK has no
cross-process job database. `cancelling` is only an acknowledgment of the
request, not proof of cancellation. The provider owns worker termination, descriptor release, lease
release and scratch cleanup.
`cancel(jobId)` returns only the observed job state; inspect the original input
for a stable terminal error or complete frame inventory.
Callers retain the required job ID and full decode input. `inspect(input)` reports
status within the same worker connection and recovers a validated complete
inventory after a decode RPC timeout. Timeout does not send cancel. A
cancellation acknowledgement is not cleanup; only terminal `cancelled`,
`failed` or `complete` state (or confirmed process death) is terminal. Do not
assume status after worker loss. Same-ID decode replay returns `busy` and never
launches a second decode, including after completion; use `inspect` to retrieve
terminal state. Changed bindings return `identity_conflict`. Client and peer
history is capped at 32 jobs. Inspect retained jobs before reaching the cap;
after all jobs are terminal, close and recreate the client/worker to clear the
bounded history. Use a fresh job ID and private output directory for a retry.
Completed PNG manifest durability remains a native-provider property.

Native provider work must hold the published `Runtime.lease(Read)` lease in its private runtime through source validation, decode, PNG publication and manifest sync. It coordinates only maintenance using that same private runtime; legacy retention uses a process-local claim and is not excluded by this lease. Native conformance must prove retention-participant adoption or a supported pinned-source boundary, and pin the source descriptor and recheck identity before/after reading. Release the lease only after decoder work and descriptors stop.
This read/maintenance exclusion is separate from capture job lifecycle and
global live-capture admission: media decode can run while capture admission is
occupied and must never contend for the live-capture lock. It requests no screen
permission, portal, capture start or single-live-capture slot.

Stable error codes include `source_mismatch`, `source_incomplete`,
`invalid_output`, `limits_exceeded`, `decode_failed`, `cancelled`,
`deadline_exceeded`, `busy`, `identity_conflict`, `job_not_found`,
`unsupported_channel`, `unsupported_version` and `internal`. Messages are
bounded and generic; they must not disclose private
paths or media contents. Unknown fields and protocol markers fail closed. The
public validators replace provider-supplied messages with bounded generic
messages keyed by stable error code.

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
remapping, which loses duplicate/order identity. The new seam preserves saturating subtraction with explicit `clampedToStart` and carries each request index/time through decode, including duplicates; post-end clamping is separate from offset mapping. Legacy Linux decoding can retain 2560px display detail, so callers can request 2560 in small chunks within the aggregate pixel limit.

## Rust peer conformance fixture

Use the exported TypeScript types and `validateMediaRequest` /
`validateMediaResponse` as consumer fixtures, not private transport hooks.
A completed inspect response requires the retained `DecodeRequest` as the fourth
validator argument; `MediaClient.inspect(input)` supplies that context itself.
A conforming peer must cover these cases against the real protocol implementation:

1. Unsupported `capture` channel and media protocol version are rejected before
   method dispatch; capture v1 remains strict and unchanged.
2. Source identity mismatch, unfinished/corrupt input, path replacement,
   existing output, output symlink/path escape and invalid PNG fail closed.
3. Offset mapping with a nonzero offset; pre-offset saturation with explicit start-clamp reporting;
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

The SDK itself supplies no native decoder, provider inventory or platform
distribution. See the Engine-owned native worker documentation above for its
implementation, measured proof and remaining delivery/platform gates.
