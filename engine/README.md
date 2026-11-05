# Engine v1 contract

`engine/sdk` defines a TypeScript client and a JSON Lines contract for a future
native Engine executable. This is a contract checkpoint only: there is no native
peer, capture implementation, durable store, media validation, crash recovery,
permission proof, readiness implementation, or EOF finalization implementation
in this slice. `ReferenceEngine` is test-only and never reports native readiness
or produces an artifact. A native `saved` response requires an absolute path,
artifact version 1 and `artifactProvenance: "native"`. The test-only peer can
create an explicitly marked `test-synthetic` fixture after a test-double
handshake; the client rejects that fixture unless it has observed that handshake.

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

The future Record adapter maps `readiness()` to the Engine handshake's actual
native readiness, `start(requestId)` to the durable `captureId` (or maintains a
private mapping if the caller's ID is not 32-hex), and ID-scoped stop/cancel/
inspect to the matching RPCs with its stored owner token. Record's phase names
`processing` and `saved` do not directly match Engine's `finalizing` and `saved`;
the adapter must map only observed facts and keep polling authoritative. Its
listener contract must be driven by observed snapshots, not fabricated native
events. This SDK currently has no Record adapter, event API or native readiness
query implementation.

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
