# Record Engine SDK adapter

`EngineSdkAdapter` injects an already constructed `@apparent/engine-sdk` client
through the narrow `EngineSdkPort`. It never locates or launches an executable,
chooses a runtime directory, builds Rust, or closes the host's SDK client.
Disposing the adapter detaches listeners and cancels its inspection timer only;
host shutdown owns `close()` and its reported outcome.

Before the first SDK start dispatch, the adapter asks the host's `CaptureMappingStore`
to atomically persist the Record request ID, SDK `captureId` and `ownerToken`, and
the host supplied bundle ID and absolute path. The store's `putIfAbsent` must return
the immutable winner across concurrent writers/processes. Replays use that same
identity and bundle; the SDK creates a fresh RPC ID on every call. Tokens remain
inside the store/adapter and are excluded from receipts, events and adapter errors.
If a mapping write has an ambiguous outcome, the store must reconcile reads by
Record request ID before another identity can be allocated. The host bundle
allocator should resolve the same request ID idempotently.
The owner-local `@apparent/record/host` export provides a SQLite mapping store
and private idempotent bundle reservation allocator. Hosts supply their explicit
private roots; these utilities do not choose runtime locations or write user
configuration. The allocator reserves an empty destination and does not create
a bundle artifact or establish capture readiness.

Readiness is true only for a native handshake reporting ready screen capture.
The SDK's default test-double rejection and `ReferenceEngine` unavailable
handshake remain intact. `test-synthetic` saved artifacts are rejected. Engine
`finalizing` maps to Record `processing`; `stopping` has no Record phase, so the
adapter polls briefly for the next observed phase and otherwise reports an
ambiguous operation error. Inspection snapshots are serialized per capture,
monotonic, and polled only while listeners are attached. V1 has no provider
events. Transport failures remain ambiguous and never become terminal receipts.

For the exact package consumer check, pack `engine/sdk` and install that tarball
into a clean temporary consumer, then set `APPARENT_ENGINE_SDK_CONSUMER` to that
consumer directory while running Record tests. Adapter unit tests may use
explicit scripted provider facts; those fixtures are not readiness or native
capture evidence.

Public receipts/events and controls use the caller's original Record request ID,
including arbitrary strings. The SDK's generated 32-hex capture ID is private to
the mapping/adapter, just like its owner token. The controller can therefore
reconcile its original identity after lost Start replies. Submitted starts with
unknown outcomes remain pollable; only actual inspection replies establish a
phase. Poll cycles cannot overlap or accumulate a second cycle while one runs.
Each observer receives its own event copy and cannot alter the returned receipt
or another observer's facts. Disposal suppresses pending replies and never closes
or cancels the host Engine.
