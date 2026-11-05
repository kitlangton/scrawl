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
The owner-local `@apparent/record/host` export provides a SQLite mapping store,
private idempotent bundle reservation allocator and `createRecordHostRuntime`.
The runtime composes those with an injected SDK port and the Record workflow;
it does not launch the SDK or select paths. Hosts supply their explicit private
roots; the runtime does not write user configuration. The allocator reserves a private destination containing only its
request marker; it does not create a bundle artifact or establish readiness.
The explicit allocator root owns its deterministic reservation namespace. An
empty owned reservation can be recovered after interrupted creation; unrelated
contents and mismatching markers are preserved and refused. Markers are synced
before atomic publication, so a crash cannot expose a partially written marker.
The database and existing SQLite sidecars must be private, owned, regular files
with a single link; aliases are refused before opening SQLite.

On creation, the runtime enumerates durable Record request IDs and inspects each
through its original private SDK mapping before admitting a new start. Active
captures and ambiguous inspection results remain named in `startup.unresolved`
and block fresh starts until `reconcileStartup()` observes terminal facts.
Scanning or blocked startup never removes stop/cancel/reconcile authority from
a capture already owned by this runtime. Commands are admitted one at a time;
a competing start is refused before reserving its request ID.
Terminal history remains reserved against request ID reuse. An unavailable
native handshake on an empty store leaves startup clear but workflow readiness
false, so the workflow cannot dispatch capture. The runtime does not resume an
active capture into the controller or stop one during disposal. Hosts must
inspect `unresolvedOwnership()` before their own shutdown decision; disposal
only detaches observation and never closes the caller-owned SDK.
Ownership reports distinguish `complete`, `pending`, `interrupted`, and `failed`
reconciliation. An empty ID list is not a clearance when reconciliation is
incomplete. Scans retain command reservations and publish discovered IDs before
inspection; disposal preserves those IDs conservatively as unknown.

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
