# Record host allocators

## Privileged native host adapter

Trusted composition injects the public `NativeHostPort` from `@apparent/native-host`
into Record's adapter, then supplies that adapter to the existing capability provider:

```ts
import { createNativeHostPort } from '@apparent/native-host';
import { createRecordNativePort } from '@apparent/record/host/native';
import { createRecordCapabilityProvider } from '@apparent/record/capabilities';

const native = createRecordNativePort(createNativeHostPort(), {
  requiredPermissionIds: ['screen-recording', 'microphone'],
});
const client = createRecordCapabilityProvider({ runtime, native, providerEpoch, penConfig });
```

The adapter has a local structural seam, so normal headless Record installation has
no `@apparent/app-contracts` dependency. Composition can pass the producer port
directly; `test/native-host/adapter.types.ts` checks strict assignability against
the public producer contract.

Supply required permission IDs from trusted Record composition. The adapter preserves
permission status, usability, relaunch requirement and supported actions independently;
missing required facts are unavailable. Snapshot scope is the running Desktop process's
cached GUI observation, not a fresh TCC probe or Engine readiness. Shortcut configured
and effective labels remain separate. Edit text accepts `Ctrl+Shift+R` style tokens;
native validation returns the only keycode candidate eligible for save. Cancel drops the
draft without writing. Save/unknown results require a later `readSnapshot` for readback.
Paste is never changed. Safe relaunch, reset/defaults, desktop config opening, pen apply,
picker/stroke, copy/open, local-only admission, and installed routing remain unavailable
through this binding. It does not install the producer into Preview or an existing GUI.
Provider detach disposes only this adapter's draft and observers; it does not close
the injected host port or cancel an already admitted mutation.

For explicit local handoff composition, `createRecordLocalClipboardPort` accepts the
published `NativeClipboardPort`, the same host's `NativeHostPort`, an exact
capture/artifact resolver, and the producer's `MAX_UTF8_TEXT_BYTES`. The resolver must
provide authenticated Saved and processing facts; the adapter checks their identity and
bundle-path consistency, formats the existing full handoff text, then performs one
fresh-state clipboard write. The native receipt must confirm the exact UTF-8 SHA-256 and
byte length before Record returns `applied`. It never closes the injected ports, retries,
truncates, or infers Saved/publication itself. This adapter is exported for explicit
composition and is not wired into the capability provider or installed route.

`AbsentDestinationAllocator` is an explicit capture/2 host primitive. Construct
it with distinct absolute private control and destination roots, then call
`allocate(recordRequestId)`. The returned `AbsentDestinationBinding` contains a
stable bundle ID and absolute final path. SQLite stores the full request ID and
root binding privately; filenames use only its SHA256 digest. A repeated request
returns the original binding, including after the destination is populated. Stored bundle IDs and paths must match that deterministic binding on every replay.
Request IDs must be nonempty, well-formed UTF-8 strings of at most 4096 bytes,
without NUL. The control root identifies the persistent reservation registry;
retain it across restarts rather than replacing it with a new registry.

The control root is private and is created as needed. An existing destination
root may use ordinary owner-owned permissions such as `0755`; it must not be
group/world writable or resolve through an alias, and allocation never changes
its mode. A missing destination root is created with the existing private
`0700` mode. Allocation writes only private control metadata. The final bundle
path must be absent for a first reservation and is
never created or opened by this allocator. The absence check does not reserve
a filesystem entry; Engine must still enforce absent-only publication. Filesystem
checks happen outside the short SQLite write transaction, whose final read
resolves a concurrent reservation winner. Its result is an identity reservation,
not proof of ownership, readiness, admission, publication or Saved state; only
Engine inspection/publication can establish those facts. A changed root binding
is refused. This API is exported but is not routed through the existing v1
runtime; callers must opt in explicitly. The v1 `PrivateBundleAllocator` and its
consumers remain unchanged. A caller can roll back adoption by stopping calls to
this allocator; durable reservations are retained and must not be treated as
published bundles.

For caller-selected output, use `reserveExact(recordRequestId,
absoluteBundlePath)`. The path must be normalized and absolute, valid UTF-8,
contain no NUL, and fit within 4096 UTF-8 bytes. Its parent directory must
already exist and must not resolve through a symlink; an ordinary user-owned
directory with mode `0755` is supported and its mode is never changed. Output
ancestors are not created. The exact final file, directory or symlink must be
absent on first reservation. Known bindings replay after native publication has
populated that path. The final path must not overlap the private control root.
The opaque request-derived `bundleId` remains separate from the requested
basename. The reservation is metadata only; Engine remains authoritative for
capture admission and absent-only publication.

`Capture2StartIntent.destinationPath?: string` passes the requested destination
through Record. Record snapshots this field before its first await, durably
reserves it and persists the full capture intent and SDK `bundlePath` before
native start dispatch. Retries for that request must keep the same destination.
After restart, the host inspects the stored identity and never retries start
with a replacement destination. Native `saved` remains Record `processing`;
the reservation is not proof of publication or Saved.

Example:

```ts
import { AbsentDestinationAllocator } from "@apparent/record/host";

const allocator = new AbsentDestinationAllocator(controlRoot, bundleRoot);
const binding = await allocator.allocate(recordRequestId);
// Pass binding to capture/2; do not create binding.bundlePath here.
```

Exact capture/2 start uses only the public SDK and Record host contracts:

```ts
import { createCapture2Host } from "@apparent/record/host/capture2";
import { AbsentDestinationAllocator, SqliteCaptureMappingStore } from "@apparent/record/host";

const controlRoot = "/home/user/.local/share/apparent/record";
const destinations = new AbsentDestinationAllocator(controlRoot, "/home/user/.local/share/apparent/bundles");
const host = createCapture2Host({ client, mappings: new SqliteCaptureMappingStore(controlRoot), destinations });
const receipt = await host.start({ requestId, source, options, invocation, destinationPath: "/home/user/output/requested.apparent" });
```

The caller must supply an existing, non-aliased output parent. `client` is a
published `@apparent/engine-sdk/capture` `CaptureClient`; `source` must be the
exact descriptor from current discovery, and `options` and `invocation` follow
the SDK 0.4.0 public types. Start returns a lifecycle receipt, not a Saved
claim.

To inject this host into the existing public Record client, import
`createCapture2RecordRuntime` from `@apparent/record/host/capture2-runtime` and pass
the same returned runtime as both `runtime` and `capture2Start` to
`createRecordCapabilityProvider`. Its required `createStartIntent(requestId,
discovery)` callback must select a source from the supplied fresh discovery and
provide SDK options, invocation and exact destination. Record checks the
descriptor against that discovery result; the capture2 host rechecks it against
its current discovery generation before durable reservation. There is no default
screen inference, picker, shortcut or legacy Engine import. The public request ID
is persisted and returned unchanged; private SDK IDs remain inside the existing
capture2 mapping store.

The adapter reconciles the existing capture2 host before fresh admission, blocks
new starts for active or unknown ownership, and keeps stop/cancel/inspect controls
available for an owned request. A bounded nonoverlapping observer inspects active
requests to receive lifecycle transitions. Detaching Record clears only its
listener and timer; the caller still owns and may inspect or dispose the capture2
host and SDK. Native readiness comes from the capture2 handshake. Native
materials-ready and SDK saved receipts map to Record processing until a separate
authenticated Record output producer supplies a matched Saved fact. The host
retains material lease references; this adapter does not release them or fabricate
a bundle.

```ts
import { createRecordCapabilityProvider } from "@apparent/record/capabilities";
import { createCapture2RecordRuntime } from "@apparent/record/host/capture2-runtime";

const runtime = await createCapture2RecordRuntime({
  host,
  createStartIntent(requestId, discovery) {
    if (discovery.state !== "available" || !discovery.sources[0]) throw new Error("Choose a discovered source");
    return {
      source: discovery.sources[0],
      options: { audio: "none", localOnly: true },
      invocation: { invocationId, observedAtMs: Date.now(), kind: "explicit" },
      destinationPath: destinationFor(requestId),
    };
  },
});
const record = createRecordCapabilityProvider({ runtime, capture2Start: runtime, providerEpoch });
```

Native adapter conformance runs in `npm run test:contracts` against the declared
`@apparent/app-contracts/host` package export. Its development-only tarball is
pinned to the public package source at `a8194a4` (provenance in `vendor/`).
Record runtime declarations and consumers use the structural injection seam and
require no Shared package installation or repository source imports.
