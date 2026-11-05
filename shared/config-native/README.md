# Authoritative config transactions

This is the existing config transaction mechanism extracted from the legacy
Connect client, with that client's public API re-exported unchanged. Both old
writers and the standalone helper use the same `config.json.lock` inode and
hold its interprocess lock through read, validation, publication and directory
sync. No stream-specific schema, onboarding policy or OS setting effect lives
here. Rust is limited to native locking and durable publication.

The helper takes a fixed absolute config path and owner ID on its command line:
`apparent-config-host --config /absolute/config.json --owner onboarding`.
Requests cannot change either binding. Line-delimited protocol 1 accepts only
`read` or `compareAndSwap` with a correlation ID. CAS takes an expected numeric
revision and a JSON value; it returns `saved`, `conflict` with current state, or
`unknown` when success cannot be confirmed. Reads and conflicts do not rewrite
the file. Never retry an uncertain write without reading persisted state.

Host envelopes live under `streamState[owner]` as `{revision,value}`. An absent
entry reads as revision 0 and null. Revisions are monotonically increasing safe
JavaScript integers. Unrelated keys, secrets and other owner entries remain
opaque and preserved. A malformed existing config or selected owner envelope
fails closed. Owner codecs define schema and legacy interpretation outside this
resource. Existing legacy configuration keys retain their authoritative writers.

The native producer in `composition/config/native` supplies precompiled
Linux/macOS x64 artifacts and exercises the exact staged executable. Normal
TypeScript owner development consumes the typed client in `composition/config`
without Cargo. The helper is unsigned/unnotarized; it does not install a shell
bridge, activate a route, implement permission/shortcut effects or establish
Record/Engine readiness. First platform artifact admission and installed native
adoption remain separate gates.
