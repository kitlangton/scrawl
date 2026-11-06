# Record pen adapter

Trusted composition injects the public `NativePenPort`, the fixed-key
`RecordPenColorConfig` facade, and `NativeHostPort` into
`createRecordPenConfigPort`. It returns the existing `RecordPenConfigPort` for
the capability provider; it imports no Shared/native-host runtime code and owns
none of the injected ports. In particular it never closes the config child or
native host transport.

Reads report persisted preference and the effective GUI getter separately.
Only the four exact palette RGB tuples map to a named effective color; black,
null and other RGB values are unknown. Missing and stored-null config both map
to no persisted preference, while unsupported stored values fail closed and
are preserved.

Writes serialize locally, read and validate the current fixed `penColor`, then
perform one config CAS. Only a confirmed CAS save proceeds to a fresh native
host snapshot and one guarded RGB apply using that saved presence/value. The
native writer lock is the authority for preventing a concurrent config change
between CAS and apply. Conflicts, stale epochs/revisions, unavailable results,
and uncertain replies return unknown without retry or rollback. A saved result
requires the config value to remain current and an exact same-process getter
readback after native apply.

Routing remains composition-owned and is not installed or enabled here. To
roll back composition, stop injecting this adapter and retain the existing
unknown/unavailable Record behavior; remove this module only after its
composition consumer is removed. The adapter establishes no installed route,
Preview behavior, stroke/bundle claim, or native GUI proof.
