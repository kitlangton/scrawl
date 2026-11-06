# Native media worker

`apparent-engine-frames` implements the published [`media/1` SDK contract](sdk/README-media.md)
for headless Linux GStreamer or macOS AVFoundation frame decoding. Build the
Linux worker with
`cargo build --manifest-path engine/Cargo.toml --features media --bin apparent-engine-frames --offline`.
The optional media feature currently requires Rust 1.96 through its GStreamer
dependency closure, plus GStreamer development/runtime libraries. The existing
capture producer does not enable this feature. Decoder plugins determine which
movies can actually decode; a registered factory alone is not proof. There is no
production FFmpeg fallback. The Mac worker reuses `engine/native/Media.swift`
through a private pinned-descriptor bridge; without that adjacent executable,
its handshake reports `decodeFrames: false`. The delivery catalog remains
Linux-only until the native x64 producer and discovery are qualified. Capture-v1
remains unavailable and unchanged.
The decoder re-exec preserves launcher-supplied `LD_LIBRARY_PATH` and GStreamer
plugin/system-plugin paths (including `_1_0` variants), so it uses the same native
runtime closure. A future packaged producer must inventory and verify that
closure; executable identity alone does not prove plugin identity. The decoder
uses no persistent registry cache and inherits no display/session-bus settings.

Launch with the SDK's `launchMediaWorker` using an absolute verified executable
and an existing caller-provisioned mode-0700 runtime directory. Stdout is media/1
JSONL only. No capture permission, portal, screen source or live-capture admission
is used. Each connection admits one active decode, retains up to 32 immutable job
bindings/statuses, and bounds duplicate RPC tracking to the last 4,096 calls.
Different clients cannot inspect or cancel each other's connection-local jobs.

The provider opens source paths component by component without symlinks, pins
the manifest and movie descriptors, verifies the requested movie identity and
manifest ID/video/offset, and refuses unfinished/unsafe capture markers. Input
files must be singly linked, owned by the current user and not writable by other
users. Source directories and ancestors must be current-user/root owned and not
group/world writable (root-owned sticky ancestors are allowed). It rechecks the
manifest, movie and paths before publishing. This is a supported pinned-source
boundary: deletion or replacement cannot redirect the decoder to another file;
detected change fails the job. It does not freeze in-place writers or make legacy
retention cooperate with the new lease protocol.

Both supervisor and decoder hold `Runtime.lease(Read)` through their work. That
excludes maintenance using the same runtime, separately from the occupied
live-capture slot. Legacy retention still uses its old process-local claim;
neither this lease nor a successful decode implies legacy retention exclusion.
Only final, stable bundles should be supplied.

The output must be absent or a caller-provisioned empty mode-0700 directory,
with an existing mode-0700 parent outside the source bundle. The worker opens
it without symlinks, checks emptiness through its pinned directory descriptor,
and exclusively creates `.engine-media-job` before reserving files. Concurrent
providers cannot both claim it; a stale claim or any existing entry is refused.
The claim contains only the job ID and is removed on normal completion/cleanup.
The worker reserves
each exact `frame-NNN.png` with an exclusive mode-0600 create, retaining its inode.
The decoder writes only these reserved files. The supervisor validates every
PNG, dimensions, hash, ordered timestamp mapping and retained inode, syncs files,
then atomically publishes and syncs `manifest.json`. Only then are references
returned. Failure/cancel/EOF cleanup unlinks only reserved files whose identities
still match; it preserves the empty output directory and any foreign replacement.
No recursive caller-path cleanup occurs.

Native decoding runs in an owned subprocess with 65 CPU seconds, a 256 MiB
per-file limit and 128 descriptors. Linux enforces a 1.5 GiB address-space limit.
Darwin cannot lower its reserved address-space map to that limit; it instead
checks the job and private Swift helper's combined physical footprint every
10 ms against 1536 MiB, failing closed if usage cannot be read. That is a sampled
physical-memory budget, not an instantaneous virtual-address ceiling. The caller's at-most
60-second deadline and cancellation kill and reap it before terminal state and
read-lease release. Source frames are bounded to 16,384 per edge and 256 MiB RGBA;
only one frame is copied/scaled/encoded at a time. Output dimensions, aggregate
pixels, frame count and job history have independent protocol ceilings. Linux
parent death also kills the decoder; a private inherited lifetime pipe is a
fallback. Forced death can leave private reserved files without a completed
manifest; callers must confirm process death before disposing of their directory.

## Focused verification

Use `cargo test --manifest-path engine/Cargo.toml --features media --offline`
for capture transport regression and actual occupied-capture/read-lease/EOF
subprocess checks. The latter generates a small synthetic MJPEG movie with
FFmpeg as a test fixture only.

After rebuilding the executable, run:

```sh
APPARENT_ENGINE_FRAMES_BIN="$PWD/engine/target/debug/apparent-engine-frames" \
  node --experimental-strip-types --test engine/tests/media-peer.test.ts
```

These checks use the public SDK against the real worker and verify actual PNG
bytes and manifest publication, ordered duplicate/nonzero-offset/end-clamped
requests, identity/marker/corrupt-source errors, collisions/symlinks, pixel limits,
cancellation/deadlines, timeout inventory recovery, source replacement and strict
protocol rejection. Host MJPEG proof is separate from H.264 codec proof,
disposable Linux guest proof and macOS proof. Keep those verdicts distinct;
no platform parity is implied.

The Linux implementation checkpoint was exercised in a disposable Omarchy guest
against a synthetic H.264/YUV420P MP4 through the public SDK. The guest lacked
`gst-libav`: decoding failed before provisioning, passed with the exact matching
signed plugin extracted into a private job directory, failed again without that
path, and passed after restoration. Package signature/dependencies and actual
PNG hashes/dimensions/order/clamp fields were checked; the installed-package
inventory stayed unchanged, artifacts were retained, and the guest/expiry ended.
The codec fixture is a runtime prerequisite, not a production fallback or proof
that an eventual distribution includes its decoder closure. Host MJPEG lifecycle,
lease/admission, timeout, cancellation and forced-death tests cover separate
boundaries. macOS decode proof and companion packaging remain outstanding.

The native delivery producer currently publishes only `apparent-engine`, not
this companion. Packaging/discovery and real consumer adoption require a separate
explicit artifact inventory change. There is no source/Cargo fallback in TS
consumers, no installed activation and no claim of completed native delivery.


## Mac native evidence and remaining delivery gate

The frozen `b5fc23c34` candidate passed both eight-case public SDK peer suites
against synthetic H.264/MP4 and MJPEG/MOV files on macOS 26.6.2 arm64 (Swift 6.4,
SDK 27, Rust 1.99.1), plus the packaged-launcher H.264 check. The tests cover
ordered/duplicate selection, offset mapping, final-sample pixels and actual
edited-video timestamps, pinned source replacement, output admission,
cancellation/deadlines and forced supervisor/helper death. End clamping maps
container sample PTS through the track edit before requesting the final image;
requested offset-mapped seek time remains separate from decoder readback.
The combined native run also passed all 45 Core publication-feature tests and
two footprint-guard tests. Earlier failed candidates remain separate evidence.

This is native arm64 synthetic-file proof, not a native x64 producer result,
macOS 15 runtime qualification, installed activation, capture permission proof,
media/2 or capture publication availability. Production x64 packaging, admitted
archives and Platform discovery remain separate gates.
