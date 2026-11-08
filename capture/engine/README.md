# Staged capture Engine

This independently compiled Rust package implements admission, durable receipts and
bundle maintenance exclusion. **No Desktop, timed recording, Agent or retention
call path consumes it yet.** It does not record media, publish clipboard content,
route captures, or certify a complete Apparent bundle. Package tests exercise
real filesystem and process boundaries, not native capture or media behavior.

## Source and retained review

This package-only port uses the complete 16-file admission/leases package from
`c571a1bb912d835a9096fdc9c9d8b14d554b7259` (identical to source-approved
`057f7a246f8655e1e96fd72c95b8f34d3adbeaad`), plus only the two-file sticky-mode
portability repair from `fc658cb327dd9bc3dc872738d5fa6a95491b641c`.
The repaired trust boundary's source approval and parent-supplied portability
review 49 are retained for unchanged bytes, not claimed as new reviews.
Historical Linux/macOS package proof covers these 22 tests, including two worker
entrypoints. Later model/recording leaves and session-provider work are excluded.
This port adds no root dependency, production caller, CI or hook-policy change.

## Caller contract

- Provision one absolute, current-user-owned `0700` runtime directory on a local
  filesystem supporting hard links, advisory locks, atomic rename and fsync.
  Every product must use the SAME persistent directory. Do not place it inside
  a bundle, a disposable runtime/temp tree, or a renamed deployment directory.
  Do not delete/rotate it during upgrade, restart or routine cleanup.
- Generate and retain the `new_id()` request identity and separate
  `new_owner_token()` **before the first attempt**. Retrying changes neither.
  Request IDs remain 128-bit random lowercase hex; mutation authority remains a
  separate random 256-bit token. Neither bundle IDs nor PIDs authorize controls.
  Reuse the bundle's persisted identity: `apparent-<milliseconds>-<hex-pid>`
  (including its counter suffix), caller-selected output stems, and legacy
  random hex IDs are supported. A bundle identity is 1–255 UTF-8 bytes, excluding
  control characters, `/`, `\`, `.` and `..`. Unicode is not normalized or case
  folded. Retain the exact manifest ID through reads, recovery and path renames.
- `Runtime::admit(request, token, bundle)` returns `Acquired(Session)` only after
  durable admission and kernel-held global recording, per-request owner and
  shared bundle locks. Begin ALL media side effects after this return, never
  before it. `Existing` means inspect the durable receipt; never start again.
  Busy admissions persist a `Rejected` tombstone. A later intentional new
  capture needs a fresh request ID, not retry of that tombstone.
- Keep `Session` alive through adapter startup, recording, postprocessing and
  finalizer completion. `recording()` marks actual first-frame readiness;
  `processing()` follows recording. Do not report readiness just because a
  thread/process started. Startup failure calls `finish(Err(()))`.
- `request_control(request, token, Stop|Cancel)` records authenticated intent.
  The owner polls `Session::control()` and stops its own adapter safely. Cancel
  is sticky, can supersede Stop, and prevents Saved even when a proof arrives.
  Controls never signal a PID or claim the media has already stopped. Caller
  timeout does NOT cancel; no clock/TTL can steal a live owner's locks.
- Only after the real finalizer validates all required media/artifacts and
  completes its durability barrier, call `FinalizerSuccess::validate` with the
  exact request, bundle, absolute final directory and manifest bytes. It checks
  the manifest identity and exact bytes, rejects unfinished/unsafe-video markers
  and pins directory identity. `Session::finish(Ok(proof))` rechecks that proof
  and session binding before Saved. It is NOT a general bundle/media validator;
  its JSON fixture in tests is deliberately synthetic, not production media.
- Output trust is stricter than no-follow: the absolute output path must have
  no symlinks and every ancestor must belong to root or the effective user and
  disallow group/other writes. Only root-owned sticky ancestors (such as `/tmp`)
  are exempt from the write-mode rule; their owned children remain protected.
  The final directory must belong to the effective user and disallow group/other
  writes, without exception. The proof pins that directory and checks both its
  descriptor and freshly traversed path at construction and again before Saved.
  The manifest must be an owned, single-link, non-group/other-writable regular
  file. ACLs, privileged attackers and malicious same-user mutation are outside
  this mode-bit/cooperative boundary; callers must provision output roots without
  ACL grants to other writers. Validation never repairs unsafe modes or owners.
- Manifests have a separate **64 MiB** byte budget, not the runtime receipt's
  64 KiB budget. `capture/engine/runtime/mod.rs::write_bundle_artifacts` serializes the entire
  `ApparentBundle` with pretty JSON, including unbounded event/stroke-point,
  transcript and context histories (`shared/model.rs`); 64 KiB is not realistic.
  The Engine rejects supplied bytes above 64 MiB before parsing/copying, reads
  at most 64 MiB + 1 from the pinned manifest and requires exact bytes. This is
  a bounded integration budget, not a promise every possible capture fits. A
  production caller must surface over-budget finalization as failure or revise
  the budget/representation with measured evidence before wiring; never truncate.
- A dropped/killed owner never manufactures success. `inspect` (or an authorized
  retry) recovers an abandoned active receipt to Failed, or Cancelled if intent
  was already persisted. Recovery is based on the kernel owner lock, not a PID.
  Terminal receipts survive normal exit/SIGKILL and never restart or change on
  later controls. A crash after successful finalization but before terminal
  publication remains Failed, not speculative Saved.
- Acquire `lease(bundle_id, Read|Write|Finalize)` for every relevant reader,
  writer and finalizer. All three are SHARED maintenance exclusions, not writer
  serialization. `Maintenance` is EXCLUSIVE and returns `None` on contention;
  keep it through rename/delete/shrink/recovery mutation. Request IDs do not
  identify bundle locks. Reuse the persisted bundle ID across pathname changes;
  never generate a new ID to clean up an already-known bundle.

All locks use maintained Rust 1.89 `File` locking, with narrow Unix no-follow
filesystem operations. They are close-on-exec. A raw fork inherits lock-holding
file descriptions; leaking one conservatively keeps admission/cleanup blocked
until the LAST inherited descriptor closes. Do not fork-and-continue a recorder;
use exec or explicitly close child copies. Process tests cover both cases.

Lock names and hard-link identity anchors are permanent. The Engine never
unlinks lock files. Unsafe modes, symlinks, replaced lock inodes, malformed or
unknown-version receipts and replaced runtime roots fail closed. An interrupted
lock/anchor bootstrap also fails closed rather than guessing a repair. Repair
requires separately proving every consumer stopped and retaining receipts;
there is intentionally no automatic stale-lock deletion API. Private `0600`
record files contain authority; public receipts and errors do not expose it.
This is cooperative same-user coordination, NOT a sandbox against malicious
code already able to rewrite both runtime state and its identity anchors.

Legacy 32-lowerhex bundle IDs retain their original `bundle-<id>.lock` anchors.
Other bundle IDs use `bundle-v1/<hex-chunks>/identity.lock` under the runtime
root: exact UTF-8 bytes encoded as lowercase hex, in 120-character chunks.
This is reversible and injective, including prefix IDs and Unicode equivalents;
no digest collision, path interpolation or per-operation random mapping exists.
Namespace directories are permanent owned `0700` directories, opened relative
to pinned descriptors with no-follow, fsynced and rechecked with their parents.
Never rename/delete these directories independently of the runtime lock anchors.

## Integration work intentionally left to the parent

Wire the common runtime into Desktop/shortcut `Recording::start` paths,
`main.rs` timed recording and `capture/desktop/workflow/agent_capture.rs` before any media starts.
Replace neither a process-local Registry nor the socket listener lock with an
unsupported global claim. Poll authenticated intent in the owning adapter,
keep the guard in its real finalization lifetime, and supply that finalizer's
validated bytes. Wire bundle leases into `capture/engine/bundle/retention.rs`, `capture/engine/bundle/recover.rs`,
export/reprocess/read paths before removing process-local BUSY/RECORDING guards.
Audit all call paths together; adding this package alone provides no whole-app
exclusion or production adoption.

## Verification and package boundary

Run from the repository root:

```
cargo test --manifest-path capture/engine/Cargo.toml --offline
cargo build --manifest-path capture/engine/Cargo.toml --offline
cargo fmt --manifest-path capture/engine/Cargo.toml -- --check
cargo tree --manifest-path capture/engine/Cargo.toml --offline
```

The package pins cached libc/serde/serde_json dependencies and has its own small,
unmodified Cargo-generated lockfile. Root Cargo.toml/Cargo.lock and CI are
unchanged: this is a staged package, not a final workspace/lock policy decision.
There are no Agent, GPUI, recorder or product dependencies. The target branch's
repository hooks remain unchanged; no integration-branch line-cap or lockfile
policy is imported. Historical macOS package build/process proof is retained
for source-identical code, not a fresh run on this port. Native capture/media
acceptance and both products' real integration remain separate gates.

Tests that construct finalizer proofs need a synthetic `TMPDIR` with trusted
ancestors under the output contract above. A group-writable cache ancestor is
correctly rejected even when the individual fixture is private; do not chmod a
real cache or capture root just to make a test pass.
