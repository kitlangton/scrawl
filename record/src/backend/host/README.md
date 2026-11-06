# Record host allocators

`AbsentDestinationAllocator` is an explicit capture/2 host primitive. Construct
it with distinct absolute private control and destination roots, then call
`allocate(recordRequestId)`. The returned `AbsentDestinationBinding` contains a
stable bundle ID and absolute final path. SQLite stores the full request ID and
root binding privately; filenames use only its SHA256 digest. A repeated request
returns the original binding, including after the destination is populated. Stored bundle IDs and paths must match that deterministic binding on every replay.
Request IDs must be nonempty, well-formed UTF-8 strings of at most 4096 bytes,
without NUL. The control root identifies the persistent reservation registry;
retain it across restarts rather than replacing it with a new registry.

Allocation creates the two private roots as needed and writes only control
metadata. The final bundle path must be absent for a first reservation and is
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
the SDK 0.3.1 public types. Start returns a lifecycle receipt, not a Saved
claim.
