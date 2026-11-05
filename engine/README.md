# Engine v1 contract

Full rebuild scope, adoption gaps and consumer return conditions are tracked in
[`IMPLEMENTATION.md`](IMPLEMENTATION.md). The latest merged slice is not the
completion boundary for Engine.

The additive Rust-free local decode SDK contract is documented in
[`sdk/README-media.md`](sdk/README-media.md). It defines a separate `media/1`
channel. The Linux native worker and its current acceptance limits are documented
in [`README-media.md`](README-media.md); it is separate from capture-v1.

`engine/sdk` defines the TypeScript client and JSON Lines contract. The standalone
`apparent-engine` executable now implements the bounded protocol transport and
uses `capture/engine` for durable, authenticated admission and receipts. Its
screen adapter is not implemented yet: handshake reports `readiness: "unavailable"`
and `screenCapture: false`, and an admitted start is durably failed without
creating media. This is not native capture acceptance. There is still no real
first-frame readiness, permission proof, complete bundle/media validation,
crash-during-capture recovery, or EOF finalization proof. `ReferenceEngine` is
test-only and never reports native readiness or produces a native artifact.

Build the executable with `cargo build --manifest-path engine/Cargo.toml
--offline`. The caller provisions one absolute private runtime directory and
passes it using `--runtime-directory`; no fallback path is selected. A native
`saved` response still requires an absolute path, artifact version 1 and
`artifactProvenance: "native"`. The test-only peer can create an explicitly
marked `test-synthetic` fixture after a test-double handshake; production clients
reject test-double handshakes by default. Tests must explicitly construct
`new EngineClient(peer, { allowTestDouble: true })`; never set this option in an
installed application adapter.

The executable is not wired into installed routing. The existing root runtime
remains authoritative. A future route can use `launchEngine` only after the
standalone source provides first-real-frame readiness, a validated durable
bundle and the required OS proof. Keep `ScreenCaptureSession` and its current
callers/build inputs until that replacement is accepted; then remove the donor
only after every consumer is verified.

## Shutdown budget

The existing Linux GStreamer finalizer has an eight-second EOS deadline. The
SDK's compatible `launchEngine(path, runtimeDirectory, shutdownTimeoutMs)` option
defaults to 3 seconds, so that default would send SIGTERM before the current
native finish deadline and SIGKILL at 6 seconds. A caller integrating that
finalizer should explicitly use `12_000` ms: EOF then gets 12 seconds before
SIGTERM, with later bounded escalation, leaving four seconds beyond the current
EOS deadline. This is a justified configuration, not a native shutdown proof;
measure real finalization and keep timeout outcomes distinct from saved receipts.

## Identity and authority

There are three independent identities. `captureId` is a random 32-character
lowercase hex durable operation key, globally unique within the private runtime
directory's Engine operation namespace. It survives client/process reconnects.
`ownerToken` is a private 64-character lowercase hex capability generated once
with the operation and persisted by the caller in private storage. It authorizes
replay and controls; never place it in logs, bundle metadata, errors or user
visible output. `rpcId` is a fresh 32-character lowercase hex correlation key for
every wire call, including retries and controls. Responses and embedded receipts
must echo that call's `rpcId` and the original `captureId`. A late reply is
discarded after its RPC times out and cannot resolve a later call.

`clientId` identifies one SDK process instance only. It is not an operation key,
authority or part of durable uniqueness. A reconnect can use another client ID
provided it supplies the same capture ID and owner token and uses the same
persistent runtime directory. Engine operation identity binds the capture ID to
owner authority and the original `bundleId`, absolute `bundlePath` and `options`.
On replay the Engine must authenticate the owner before exposing any receipt,
then compare every bound field. A different owner fails `permission_denied`; a
same-owner mismatch fails `identity_conflict`. No error message may reveal the
token or private path.

## Wire contract

The transport is UTF-8 JSON Lines over stdin/stdout. Each request has protocol
version 1, a per-process client ID, fresh RPC ID and one method. A line is limited
to 64 KiB before its newline. CRLF is accepted; embedded CR, invalid UTF-8,
malformed JSON and oversized lines terminate the transport. Stdout is protocol
only. Stderr is drained but its private diagnostics are not returned or logged by
the SDK. There may be at most 64 concurrent pending RPCs. Unknown/late RPC IDs
are ignored; a malformed envelope or unsupported version fails all pending
calls. RPC timeouts are bounded to 1–60,000 ms and never cancel capture.

Clients generate fresh RPC IDs for every call. The native provider rejects
duplicate IDs within its most recent 4,096 calls, retaining a bounded replay
window so long-running polling cannot grow process memory indefinitely. Evicted
RPC IDs are no longer checked; durable capture identity and owner authorization
still protect start retries independently of this transport-level window.
Bundle IDs reject all Unicode control characters consistently across Rust and
TypeScript, as well as path separators and the special names `.` and `..`.

V1 rejects unknown fields at every object level. A peer must not silently ignore
an extension. Any added field or method requires a new negotiated protocol
version; v1 clients reject that version rather than guessing. Errors have a
stable code and bounded generic message. Handshake reports protocol agreement,
implementation and capabilities separately from runtime readiness.

Methods are `handshake`; `start` with `captureId`, owner token, `bundleId`, an
absolute normalized platform path and `{ "screen": true }`; then `inspect`,
`stop` or `cancel` with the same capture ID and token. The current start options
select screen capture only. Stop before `recording` returns `not_ready`; retry
with a new RPC ID. Cancel is sticky and terminal once accepted. Stop and cancel
are idempotent against terminal operations. `saved` requires an artifact
provenance marker, artifact version and absolute bundle path; only `native`
provenance is meaningful for a production Engine. `test-synthetic` is confined
to explicit test-double workflows. `failed` and
`rejected` require a structured error; other states cannot carry an error or
artifact fields.

Allowed lifecycle transitions are `starting → recording → stopping → finalizing
→ saved`, `starting|recording|stopping|finalizing → cancelled`, and active state
→ `failed`. `rejected` is an admission outcome before a live capture exists.
Terminal states are stable. `inspect` is the v1 observation mechanism. V1 does
not define events or subscriptions: consumers needing ongoing updates must poll
`inspect` with bounded intervals and treat each response as the authoritative
snapshot. A future host adapter may publish those observed snapshots to local
listeners, but may not invent phases or treat transport failure as a terminal
receipt.

## Process and durability boundary

`launchEngine(executablePath, runtimeDirectory)` requires explicit absolute
paths and passes `--runtime-directory <path>` to the executable. The same private
directory must be used for every Engine process that needs to recover an
operation. The SDK does not choose a directory, search `PATH`, build Rust, or
fall back to source/build paths.

`close()` ends SDK stdin and waits a bounded interval for the Engine to observe
EOF and exit. The Engine contract requires its native owner to stop capture,
finalize or record a truthful failed/cancelled outcome, and persist it before
normal EOF exit; this implementation does not provide or prove that behavior.
If EOF does not exit, the SDK requests SIGTERM and then SIGKILL on bounded
deadlines and returns an explicit outcome describing whether exit was observed
or kill was merely requested. Process death is not proof of a durable stop or a
saved bundle. Ordinary RPC timeout never signals or kills the Engine.

## SDK build and package consumption

The package is Rust-free and requires Node 22.6 or later. From `engine/sdk`, run
`npm ci`, `npm run typecheck`, `npm run build`, `npm test`, and `npm pack`. The
pack lifecycle builds the TypeScript output via `scripts.prepack`; only generated
`dist/` files are shipped. A clean consumer installs the resulting tarball and
imports `@apparent/engine-sdk` plus the explicitly test-only
`@apparent/engine-sdk/testing` subpath. No Cargo toolchain is involved.

Record's owner-local [SDK adapter](../record/src/backend/engine/README.md) maps
`readiness()` to the handshake's actual native readiness and persists a private
mapping from its request ID to Engine's capture ID and owner token. ID-scoped
controls use that same mapping. Record's `processing` phase corresponds to
Engine's `finalizing` phase; its listeners are driven by authoritative inspection
snapshots. The adapter does not establish native readiness, select or launch a
runtime, or switch installed routing. The SDK itself provides no Record product
adapter or provider event API.

`ReferenceEngine.handshake()` deliberately reports `unavailable`; its scenario
controls can exercise later protocol phases without asserting readiness.
Record's workflow tests should inject readiness through their own controlled
consumer fixture when they need to test the ready path. Do not change the
reference peer's handshake to imply native admission.

## Acceptance boundary

The SDK tests exercise validators, client correlation, reference-peer authority
and a real child-process JSON Lines transport. They do not establish native
conformance. Native acceptance still requires a real Rust-free-consumable Engine
executable, durable operation storage and replay, authentication/conflict checks,
EOF stop/finalization, crash recovery, real first-frame readiness, valid bundle
inspection and focused macOS/Linux permission, privacy and capture-boundary
proof. Until those pass, the existing runtime remains authoritative.
