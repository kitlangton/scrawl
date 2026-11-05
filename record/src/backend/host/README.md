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

Example:

```ts
import { AbsentDestinationAllocator } from "@apparent/record/host";

const allocator = new AbsentDestinationAllocator(controlRoot, bundleRoot);
const binding = await allocator.allocate(recordRequestId);
// Pass binding to capture/2; do not create binding.bundlePath here.
```
