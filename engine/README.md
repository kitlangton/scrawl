# Engine protocol v1

This directory defines the first independently consumable Engine contract. It is a
specification and TypeScript SDK, not a recorder or a claim of native readiness.
The Rust-free package `@apparent/engine-sdk` may be installed, built, tested and
packed without Cargo. A production consumer supplies an explicit absolute path to
a separately built Engine executable; the SDK never searches `PATH`, source trees
or build directories.

## Agreement vs readiness

`protocolVersion: 1` means both peers agree on this wire contract. It does not
mean screen capture is available. A production handshake reports the executable's
real capabilities and readiness; a test reference peer reports
`implementation: "test-double"` and `screenCapture: false`. Consumers must not
present the latter as recording support.

## Wire protocol

Transport is UTF-8 JSON Lines over child stdin/stdout. One request and one response
per line; each line is at most 64 KiB. Stdout is protocol-only; diagnostics go to
stderr. Unknown fields are rejected. Requests have `protocolVersion: 1`, a
per-process `clientId`, unique `requestId`, and a method. The SDK generates the
request ID and a distinct random 256-bit `ownerToken` before `start`; retain both
before sending so retries after a timeout reuse the same identity and authority.
Keep the token private and out of bundle metadata and logs; controls require it.

Methods: `handshake`; `start` with caller-selected stable `bundlePath`; `inspect`
with `requestId` and `ownerToken`; `stop` or `cancel` with the same identity and
authority. Repeating start with identical identity is an inspect/replay, never a
second capture. A conflicting reuse is `identity_conflict`. Stop while starting
returns `not_ready`; callers may retry. Cancel is sticky and may supersede stop.
An RPC timeout is not cancellation. On transport loss the supervisor requests
bounded stop, waits for durable terminal receipt, then exits; if shutdown cannot
complete, process death releases kernel ownership and the next inspect reports
recovered `failed`/`cancelled`, never `saved` by inference.

Lifecycle states are `starting`, `recording`, `stopping`, `finalizing`, `saved`,
`cancelled`, `failed`, `rejected`. Only `saved` carries an absolute `bundlePath`
and artifact version. Readiness means the native adapter observed its first real
frame. The v1 artifact is an existing valid `.apparent` bundle; this API does not
define audio or window capture capability. Structured errors carry stable `code`
and bounded human-readable `message`, never authority tokens or private paths.

All messages are versioned and validated. Unsupported versions fail with
`unsupported_version`; malformed JSON/shape, oversized lines, unknown methods or
fields fail closed. A consumer must retain request identity and authority across
retry/reconnect and use the same persistent private runtime directory across
Engine processes. Terminal results are durable and idempotent.

## Acceptance scenarios

`sdk/test/scenarios.ts` is the shared bounded scenario set. Both the test reference
peer and the eventual Rust executable must pass it. Native acceptance additionally
requires real first-frame readiness, valid finalized media/bundle inspection,
permission/privacy checks on macOS and Linux, process crash/restart recovery, and
the platform-specific proof in Desktop's native testing guide. The reference peer
only checks consumer protocol behavior and explicitly cannot satisfy those gates.

Record can consume `engine/sdk` via a local package dependency and launch only an
explicit prebuilt executable path. The donor capture runtime remains authoritative
until a production adapter and each caller are accepted; remove it only after all
runtime callers, tests, build inputs and packaged app routes use Engine v1 and the
platform gates pass.

For local consumer work, declare `"@apparent/engine-sdk": "file:../../engine/sdk"`
from the consumer package (adjust the relative path), then `npm install` and
`npm run build`. `npm test` and `npm run typecheck` in `engine/sdk` require only
Node 22.6+ and its pinned TypeScript dev dependency. Use `launchEngine(absolutePath)`
with the verified installed binary, `new EngineClient(transport)`, then
`handshake()`. Call `prepareStart(bundleId, absoluteBundlePath)`, persist its
`requestId` and `ownerToken` privately, then pass that exact object to `start()`.
Retry by sending the same object; use `control("inspect"|
"stop"|"cancel", requestId, ownerToken)` for follow-up. Package build, test and
pack perform no Cargo operation. The test reference is separately exported from
`@apparent/engine-sdk/testing` and is intentionally incapable of producing Saved.
