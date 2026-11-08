# Native test infrastructure

## Owned native-test processes

`owned-processes.ts` provides the packaged smoke driver's existing Bash
process-group supervisor, bounded operation wrapper (`within`), `stopBackend`
and `stopElectron`. The native Storage driver also imports `pidFileSupervisor`
and `stopOwned` from this module; its labelled deadlines use `withinLabelled`.
The capture-reference driver imports `stopChild` for its existing direct-child
TERM/2.5-second KILL escalation. That helper does not manage process groups. Its supervisor requires the pre-created,
non-symlink, owned `$TMPDIR/rust.pid` and publishes only its launched child PID.
`stopOwned` returns a per-child cleanup receipt and refuses to signal after its
leader exits. It treats an exited group probe's `EPERM` as failure; do not silently
substitute the packaged smoke's different, snapshot-validated cleanup contract.
Import it from a Node 22.18+ driver; the main verification
workflow uses Node 22. It has no Playwright, Desktop or application dependency.
Callers retain admission, isolated HOME/config, operation budgets, assertions,
evidence/report writing and fixture retention. Keep the supervisor's stdin open
until teardown; EOF stops its owned group if the driver disappears.

Packaged-smoke cleanup preserves its existing TERM/KILL sequence. Darwin `EPERM` is
accepted only after the supervisor exits and a successful bounded, validated
`ps` snapshot establishes no live group members. Live, unknown or malformed
results fail and the caller retains its private fixture. A timed-out Electron
close kills its owned child and still fails the check. `within` bounds waiting;
it does not cancel the operation. Callers must arrange owned cleanup.

Run `node --test enablement/native/owned-processes.test.ts` for the real harmless
child/descendant and strict cleanup contracts. The packaged smoke regressions
moved from Desktop; additional contracts exercise the Storage caller import, PID
file and receipt/refusal semantics. They establish no new Mac GUI acceptance.
The packaged smoke entrypoint exports only its Desktop admission/readback
contracts; lifecycle consumers import this canonical module. There is one
implementation, with no compatibility executable or copied helper.

## Mac verification setup

`mac-setup.ts` runs the existing FFmpeg and Metal setup commands concurrently
after Xcode selection. In main verification, its bounded worker overlaps Rust
compiler/cache setup; an atomic, invocation-bound success record gates Cargo.
Both direct commands must close successfully; the supervisor signals any
remaining owned descendants before publishing the record. It does not separately
confirm every descendant has exited. A missing, malformed, stale or failed record stops compilation.
The helper retains command output, reports elapsed times, and terminates its
owned process groups on failure, cancellation or the 120-second setup deadline.
`mac-setup.test.ts` exercises the orchestration with executable fixtures; actual
Mac setup timing and the complete main verification result remain separate gates.

## Native compiler outputs

`build_cache.rs` supports the root build script's macOS native output reuse.
Reuse stays inside the current Cargo `OUT_DIR`; it is separate from executable
artifact admission and Rust dependency caches. The key covers actual native
source/plist bytes, compiler recipe, target and environment, and selected Apple
toolchain identity, including streamed SDK content hashes. SDK hashing runs once
per build-script process; it does not trust a persistent timestamp-only SDK hash.
SDK links escaping their root disable reuse. Actual hashing overhead must be
included in the Darwin timing result. A receipt is reusable only while its output hashes match.
Phase warnings separate tool probes, tool-byte hashing and SDK-content hashing.
Cargo can replay these warnings for a fresh build script; replayed durations are
not new executions or additional measurements.
On each build-script execution, missing, corrupt or mismatched output rebuilds; failed identity discovery runs
the normal compiler path without admitting a cache entry. Root Git revision and
dirty labels continue to be calculated on every build-script invocation.

Root build identity is still coupled to the whole checkout: `build_version.rs`
watches Git refs and every tracked path so it can refresh `APPARENT_VERSION_LABEL`
for both unrelated edits and root source edits. The Cargo fixture in
`build-cache.test.ts` traces the consequence: a tracked unrelated edit relinks
the root crate while native outputs remain reusable, and an unrelated commit
changes the embedded label to the new checkout revision; removing all tracked
watches would instead leave the embedded label stale after a dirty Rust source edit.
This is a synthetic Cargo trace only: it makes no production rebuild optimization
or native proof claim. In the fixture, the unrelated edit/commit relinks the root
crate while the native compiler invocation count stays unchanged; actual Rust
source edits still relink and update the dirty label. Do not narrow this watch set
to guessed directories or remove it. After the accepted component consumers adopt
the root artifact's original source identity and separate assembly revision, use
that artifact's verified compiler dependency closure to propose exclusions, then
complete the required focused Mac build/artifact proof before changing production
watches. Existing Mac producer admission and platform verification continue to
use exact checkout SHA/tree and clean-source checks; the UI label is not a
substitute for those gates.

When the Swift helper receipt is absent, its unchanged compiler command overlaps
complete identity discovery in a scoped thread. Both finish before output
validation and receipt publication; failed identity discovery leaves no receipt.
Existing receipts retain identity-first validation, including missing or corrupt
outputs. The helper runs before shaders so the first SDK fingerprint can overlap
Swift compilation. Inputs must remain stable during the build, as with sequential
compilation. The `swift-helper` duration includes identity and compilation;
`swift compiler execution` reports the compiler command separately.

An absolute, existing `DEVELOPER_DIR` matching the selected developer directory
allows Cargo to omit the global Xcode selector symlink watch, which otherwise
recursively watches its destination. Selected Xcode metadata, SDK contents and
compiler inputs remain watched and fingerprinted. Without an explicit selection,
the global watch remains; an invalid or mismatched override disables reuse.

Run `node --test enablement/native/build-cache.test.ts` for the production
build-script regression with synthetic compilers. Its fixtures establish cache
decisions and truthful identity behavior, not Darwin compilation or performance.
Actual Mac compilation, reuse/invalidation and phase timings remain separate
evidence requirements. Different Cargo output directories do not share outputs,
and a cold native build still invokes the compilers.

`linux/gui-vm.py` owns disposable Omarchy VM lifecycle, candidate/fixture
staging, independent expiry and private evidence retention. Its sibling
`gui-vm-reaper.py` survives controller/worktree loss by being copied into each
owned job. Run `python3 enablement/native/linux/test-gui-vm.py` for launch-free
contracts. These are the existing maintained lifecycle and cleanup entrypoints,
not a second VM controller or a claim of additional platform support.

Desktop owns scenario behavior and assertions under
`capture/desktop/scripts/linux/{gui,updater}`. Follow the
[Desktop native recipes](../../capture/desktop/docs/native-testing.md) for
scenario admission, template identity, host capacity, consent and output
inspection. The runner supplies `SCRAWL_GUI_BINARY`, `SCRAWL_GUI_OUTPUT`, optional
stimulus and the candidate library path to a single Bash-invoked scenario.
Successful transport is not product acceptance.

`--retain-cold-state` explicitly opts one job into preserving its cold
`disk.qcow2` and matching final `OVMF_VARS.fd`. Default verified-success deletion
is unchanged. Normal cleanup and the copied expiry sibling share the same
private job-bound request and metadata-only FD ownership check; unknown access,
open owners or unsafe resources produce no positive cold receipt. QEMU still
stops, sockets are removed and normal cleanup stops the expiry timer. Failure or
expiry remains failure; retained state does not convert it into success.
`cold-state-{runner,expiry}.json` records owned cold file identities and artifact
retention status, not native acceptance, image hashes or standalone equivalence.
An A→B handoff must separately seal the actual cold pair under its bounded
offline admission, convert a backed overlay through the supported standalone
route with fsync/sector comparison, preserve its originals and freshly qualify
B's persisted device/inode/publication bindings. Never use a partial profile
snapshot, a live copy or a deliberately failed scenario as the preservation path.

Retention remains fail closed: only a successful trusted scenario plus matching
copied-file manifests permits disk deletion. Failed/interrupted runs retain cold
disk/firmware; a matching live copy is only a partial snapshot. Keep private
receipts and media private. No migration command launches guests, changes
installed tools, reserves capacity or redirects an active run.

## Compatibility

The former runner, reaper and test symlinks in
`capture/desktop/scripts/linux/gui/` were removed after the updater, maintained
frontend documentation and shared VM skill adopted canonical paths. The Settings
owner explicitly released the old paths: its current disposable controller uses
the canonical runner and its frozen Mac packet references none. No running
Python command named the old paths during the final audit. Historical receipts
and older checkouts retain their own source/provenance; they do not reserve new
main entrypoints. No VM, cold evidence or active resource was changed.

Retiring packaged-smoke lifecycle re-exports requires maintained callers to
migrate and the active/frozen owner check to clear. All harness imports and
executable VM commands use canonical paths.

Further Mac/frontend extraction checks exact active hunks with their owners;
an open PR or pending native acceptance alone does not reserve stable mechanics. Storage/retention UI assertions remain Desktop
scenarios; a filename containing “native” does not make a test reusable.

[Desktop verification policy](../../capture/desktop/docs/native-testing.md)
continues to govern: shared UI and real Rust bridge primarily on Linux, affected
native seams on their OS, unchanged-input evidence reuse, and current-package
Mac publication smoke plus exact-main verification/signing. Relocation alone
requires path/contract checks, not a fresh native workflow or paid resources.

## Desktop scenario boundary

The inspected frontend drivers remain beside Desktop: `native-smoke.cjs` asserts
UI tabs and the real runtime bridge; `native-retention.cjs` asserts user retention
policy and config writes (it is not evidence retention); `reference-native.cjs`
asserts selection and actual clipboard payloads; `mac-storage-native.cjs` asserts
Storage, reporting and native picker/recovery behavior. Their private fixtures,
screenshots, report schemas and failure-retention decisions describe those
scenarios. Generic supervision, waiting and child cleanup are imported here.
Named AX controls and their allowlists remain part of Desktop acceptance; there
is no independent cross-owner AX consumer to justify extracting their selector
logic. Product startup/readiness adapters likewise retain their real bridge
assertions with Desktop. This boundary does not wait for PR376 acceptance.

## Linux sealed fixture preflight

For a flat guest fixture that stages `input-seals.sha256`, run the same closure
check against the **actual** runner argv before budget initialization/allocation:

```sh
node --experimental-strip-types --experimental-vm-modules enablement/native/linux/fixture-closure.ts check-config CONFIG_JSON EXPECTED_CONFIG_SHA256
```

This is a Node22 parser-only check, not execution or admission. It rejects missing
sealed files, duplicate basenames, missing/unsealed static relative imports and
unsupported non-flat or relative dynamic imports. Type-only imports are erased;
computed archive imports still rely on the existing installer/archive seals.
No module is linked/evaluated and no archive/model/image is read. Actual content
hashes and installed runtime closure remain separate admission/guest checks.
The runner repeats this check before overlay/expiry/QEMU for manifest-bearing
fixtures. A frozen runner copy must retain the adjacent `fixture-closure.ts` and
pin its bytes too; a Python-only copy fails closed rather than booting unchecked.

Cold retention also prechecks whether the current UID's complete FD metadata
census is accessible before creating an overlay. Access denial reports only
numeric PID, stage and errno, and still refuses. This precheck is not future
absence proof: cleanup repeats inode ownership checks and preserves unknown
resources. It does not skip protected processes, grant host privilege or qualify
an earlier failed/partial cold disk. An unavailable complete census remains a
cold-retention method blocker under unprivileged access.

On Clanker, `--retain-cold-state --host-fd-observation` explicitly opts one new
job into a read-only sudo fallback for that access denial. It records the owned
job opt-in for both normal and copied expiry reapers. No flag means the original
unprivileged refusal. The fallback reuses the same bounded sequential UID/inode
census as literal code passed to isolated system Python; root imports no
user-writable module and receives no runtime path/code/UID override. Namespace,
ptrace/DAC capabilities, owned directory/resources and before/after metadata are
checked. It requires a <=5GiB/noSwap/one-CPU enclosing scope, uses noninteractive
sudo; the opted-in independent expiry service explicitly sets MemoryMax=5G,
MemorySwapMax=0 and CPUQuota=100% rather than inheriting the runner's scope. The
default expiry service properties are unchanged. The fixed reader uses
a 4s timeout plus 0.5s kill grace, and rejects denial, malformed output,
timeouts or any unknown instead of skipping protected FDs. This adds no installed
privilege grant, global setting, process stop or resource-content read. A match
still prevents retention. A complete empty result is only a sequential metadata
observation, not atomic/future exclusion, standalone equivalence or native proof.
Existing owned-QEMU drain, retention verification and final identity checks
remain required. Old failed resources are not retroactively qualified or reused.
