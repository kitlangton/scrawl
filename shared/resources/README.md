# Privileged native resource primitives

`@apparent/native-resources` supplies TypeScript adapters for two narrow OS
capabilities. Product policy, schemas, credentials, consent, retries and recovery
remain with the caller. The helper does not open caller-selected paths, create
lock files, delete state or interpret bundle contents. This is a privileged main
process API; never expose it or its descriptors to a renderer.

## Prebuilt provider and independent development

Assembly must admit a verified native producer artifact and its source closure,
workflow provenance and platform before constructing `NativeResources` with the
absolute executable path, SHA-256, target and protocol 1. Content/target checks in
`validateResourceArtifact` supplement that admission; a self-authored manifest is
not provenance. The executable and containing directory must remain protected
against replacement for the operation's lifetime. Per-call hash, ownership, mode
and no-follow checks do not provide a sandbox against another process with the
same user's authority. There is no PATH search, install-time compilation or
source-build fallback.

The normal package loop requires Node 22.18+ and npm, without Rust:

```sh
npm ci
npm run typecheck
npm test
npm run build
npm run pack
```

Default tests use faithful protocol peers, not native-proof claims. The separate
producer under `delivery/` compiles Rust, seals two files (helper + manifest), and
runs the real TypeScript/native integration against those exact staged bytes.
Its targets are Linux x64, macOS x64 and macOS arm64. Native artifacts are
unsigned/unnotarized until an installer separately qualifies/signs its assembly.
Connect's installed Rust state/CLI implementation remains authoritative; adding
this primitive does not adopt a new writer or retire any donor.

## Descriptor ownership

Both calls consume their supplied idle `FileHandle` on every outcome, including
validation failures and cancellation. The caller must hold no duplicated or
in-flight use of that open-file description. The child inherits fd 3, and the
parent closes its copy before returning. With `flock`, duplicated descriptors
share the lock: retaining a parent duplicate would prevent helper death from
releasing it. Tests cover ownership transfer, explicit release, EOF and SIGKILL.

The helper checks effective-UID ownership and resource metadata. Callers still
must open without following symlinks, validate pathname-to-inode identity after
waiting, and impose their required lock order. The helper never opens, unlinks,
recreates or repairs a lock sidecar.

## Lock lease

`lock(handle, {timeoutMs, signal})` accepts an owned private, writable, single-link
regular file opened read/write. Acquisition is nonblocking and deadline-bounded
(0–60,000 ms); cancellation before acquisition returns `cancelled`. Results are
`acquired` with a lease, `busy`, `cancelled`, `refused`, or `failed`. Provider or
protocol failures throw a category-only `NativeResourceError`.

A lease explicitly transitions `held → releasing → released`, or to `lost` on
unexpected helper termination/protocol failure. Check `lease.assertHeld()` and
observe its abort signal before protected effects. No process lease can roll
back work already in progress. `release()` waits for native unlock and process
exit and is idempotent after success. Caller cancellation also releases a held
lease. Losing the helper invalidates authority; do not continue or silently
reacquire within the same transaction.

Wire invocation is `apparent-resource-host lock --timeout-ms N` with fd 3. Before
acquisition, stdin EOF or `c`/`r` cancels; while held, EOF or `c`/`r` releases.
Unexpected input fails closed. Stdout has at most two exact protocol-1 JSON lines:
`acquired`, then `released`/`failed`, or one terminal acquisition result. No
request data, paths, errno messages or private contents appear in the protocol.

## Atomic directory publication

`publish(parentHandle, sourceBasename, destinationBasename, options)` uses a
caller-opened directory owned by the current UID or root. Group/other write
access requires the sticky bit, preserving the legacy trusted shared-parent
contract; the staged source remains owned and private. Source and
candidate destination must share this parent; names are well-formed single UTF-8 POSIX components, excluding slash, NUL,
dot/dot-dot and equal names. Spaces and Unicode remain valid. Inputs are bounded
to 4,096 UTF-8 bytes; the kernel enforces its actual filesystem name limit.
The source must be an owned private real directory. The caller stages/validates
all contents and fsyncs files and directories first; this helper does not walk
or certify content, symlinks inside the staged tree, formats or user intent.

The native operation pins the source directory, syncs it, performs an exclusive
rename (`renameat2/RENAME_NOREPLACE` on Linux, `renameatx_np/RENAME_EXCL` on macOS),
then syncs the parent and checks destination identity/source absence. Unsupported
filesystems fail closed with no replacement fallback.

Results are `published`, `destination_exists`, `refused`, or
`publication_uncertain`. `published` establishes namespace identity and the
required directory syncs, subject to the caller's content staging. Existing
output is never replaced; refusal preserves the staged source. Cancellation
before dispatch is refused. Once dispatched, lost response, timeout, cancellation,
post-rename sync failure or ambiguous identity is uncertain. **Never retry,
remove the destination, or delete the stage after uncertainty.** The caller must
reconcile its immutable stage/destination facts. Successful rename cannot be
undone by cancellation.

Wire invocation is `apparent-resource-host publish --source NAME --destination
NAME` with fd 3 and one exact protocol-1 result line. No recursive deletion or
product recovery behavior is implemented here.

`checkAcl(handle)` consumes a file/directory descriptor and reports `accepted` or
`refused`. It checks effective native ACL facts, not private-file or directory-tree
authority. On macOS it refuses allow entries and accepts deny-only ACLs, adapting
the legacy Connect check. On Linux the caller still must enforce the mode/ACL mask.
Callers must independently validate ownership, private modes, inode identity and
every ancestor. No contents or ACL principals are returned. The operation uses
`apparent-resource-host acl` with fd 3; older helpers refuse this unknown operation
and cannot satisfy callers requiring it. No credential or queue writer is added.

Lock descriptors and publication parent/source descriptors also refuse Darwin ACL
grants immediately before native effects. Ancestor validation remains caller-owned.
