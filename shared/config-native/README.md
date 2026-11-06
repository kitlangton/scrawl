# Authoritative config transactions

This is the existing config transaction mechanism extracted from the legacy
Connect client, with that client's public API re-exported unchanged. Both old
writers and the standalone helper use the same `config.json.lock` inode and
hold its interprocess lock through read, validation, publication and directory
sync. No stream-specific schema, onboarding policy or OS setting effect lives
here. Rust is limited to native locking and durable publication.

The helper takes a fixed absolute config path and owner ID on its command line:
`apparent-config-host --config /absolute/config.json --owner onboarding`.
Requests cannot change either binding. Line-delimited protocol 1 accepts
`read`, `compareAndSwap`, `readForAdoption` and `adoptLegacy` with a correlation
ID. Adoption reads the owner entry and only the top-level legacy key whose name
is exactly the bound owner ID. Record-only pen color operations address the
fixed top-level `penColor` key and are rejected unless the helper was started
with owner `record`. Their presence/value envelope distinguishes missing from
JSON null; exact CAS uses the same writer lock, returns a fresh value on
conflict, and limits values to 16 KiB of JSON. No operation accepts a
request-selected key. `adoptLegacy` requires the owner entry to remain absent
and legacy presence/value to match the supplied snapshot under the same writer
lock; it writes owner revision 1 and never removes the legacy key. Conflicts
return a fresh snapshot. Reads and conflicts do not rewrite the file. Never
retry an uncertain write without reading persisted state.

For `onboarding`, a successful `adoptLegacy` writes the explicit
`onboardingLegacyWriter: {schema: 1, state: "retired"}` ownership marker in
the same config publication as revision 1. The existing legacy progress writer
checks this marker while holding the same config writer lock, so its saves are
ordered before adoption or refused afterward. When an owner entry already
exists, privileged onboarding assembly may explicitly call
`retireOnboardingLegacyWriter`; it is owner-bound, durable across restart, and
does not block writes to other config keys. A malformed marker fails closed.
The marker is not inferred from an owner entry or other config presence.

The authoritative GUI process can use
`with_pen_color_current_in(path, expected, effect)` for the native apply seam.
It acquires that same config writer lock, validates and compares only the fixed
top-level `penColor` presence/value, and invokes the bounded callback while
still holding the lock. It returns `PenColorEffectResult::Conflict(current)`
without invoking the callback when the snapshot is stale, or
`PenColorEffectResult::Applied(result)` only when the callback succeeds. Callback
errors propagate and are never reported as applied; the operation does not
write config or roll back any partial native effect.

Bounded host requests must instead call
`with_pen_color_current_allowed_in(path, expected, allowed, effect)`. Its
admission predicate controls nonblocking lock acquisition and is rechecked
immediately before the callback. Expiry before that point never invokes the
effect. The callback must still bound any effect already in progress; cancellation
cannot roll back an OS mutation that has started.

The required lock order is config writer lock, then runtime/native resource
lock. Callbacks must not reenter config access, perform filesystem or network
work, or wait beyond caller-owned admission/deadline controls. This primitive
does not implement pen policy, map colors, or prove that an OS pen effect was
applied; the callback owner supplies and verifies that effect.

Host envelopes live under `streamState[owner]` as `{revision,value}`. An absent
entry reads as revision 0 and null. Revisions are monotonically increasing safe
JavaScript integers. Unrelated keys, secrets and other owner entries remain
opaque and preserved. A malformed existing config or selected owner envelope
fails closed. Owner codecs define schema and legacy interpretation outside this
resource. Existing legacy configuration keys retain their authoritative writers;
this primitive does not interpret or delete legacy values.

The native producer in `composition/config/native` supplies precompiled
Linux/macOS x64 artifacts and exercises the exact staged executable. Normal
TypeScript owner development consumes the typed client in `composition/config`
without Cargo. The helper is unsigned/unnotarized; it does not install a shell
bridge, activate a route, implement permission/shortcut effects or establish
Record/Engine readiness. First platform artifact admission and installed native
adoption remain separate gates.


The persistent sidecar is created with `O_CREAT|O_EXCL`, or opened without
`O_CREAT` when creation reports `EEXIST`. This avoids the concurrent Darwin
create-or-open failure observed before locking. Both paths retain no-follow,
private-file ownership/link checks, flock and the post-wait inode check. A
missing existing sidecar is an error, never a reason to create a replacement
inode beneath a holder. Tests inject ENOENT at the combined create/open boundary, cover failure after
observing an existing lock, and run 32 independent competing first-writer pairs.
The injected fault does not establish a Darwin kernel cause; the affected macOS
producer must run the real repeated-contention regression before readiness.

Older binaries do not enforce the onboarding retirement marker. Adoption hosts
must admit only a legacy GUI built with this fence, or stop that older writer;
config-helper success is not proof of the running GUI's implementation identity.
