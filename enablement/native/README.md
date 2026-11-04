# Native test infrastructure

## Owned native-test processes

`owned-processes.ts` provides the packaged smoke driver's existing Bash
process-group supervisor, bounded operation wrapper (`within`), `stopBackend`
and `stopElectron`. Import it from a Node 22.18+ driver; the main verification
workflow uses Node 22. It has no Playwright, Desktop or application dependency.
Callers retain admission, isolated HOME/config, operation budgets, assertions,
evidence/report writing and fixture retention. Keep the supervisor's stdin open
until teardown; EOF stops its owned group if the driver disappears.

Cleanup preserves the existing anchored TERM/KILL sequence. Darwin `EPERM` is
accepted only after the supervisor exits and a successful bounded, validated
`ps` snapshot establishes no live group members. Live, unknown or malformed
results fail and the caller retains its private fixture. A timed-out Electron
close kills its owned child and still fails the check. `within` bounds waiting;
it does not cancel the operation. Callers must arrange owned cleanup.

Run `node --test enablement/native/owned-processes.test.ts` for the real harmless
child/descendant and strict cleanup contracts. These moved from Desktop's host
regressions; they establish no new Mac GUI acceptance. Desktop's packaged smoke
entrypoint still exports the same helper objects for existing callers. Those
exports are a transition contract: remove only after supported imports/tests and
active frozen candidates have migrated, with an owner check. There is one
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
contracts. The three files were relocated unchanged; this is not a new VM
controller or a claim of additional platform support.

Desktop owns scenario behavior and assertions under
`capture/desktop/scripts/linux/{gui,updater}`. Follow the
[Desktop native recipes](../../capture/desktop/docs/native-testing.md) for
scenario admission, template identity, host capacity, consent and output
inspection. The runner supplies `SCRAWL_GUI_BINARY`, `SCRAWL_GUI_OUTPUT`, optional
stimulus and the candidate library path to a single Bash-invoked scenario.
Successful transport is not product acceptance.

Retention remains fail closed: only a successful trusted scenario plus matching
copied-file manifests permits disk deletion. Failed/interrupted runs retain cold
disk/firmware; a matching live copy is only a partial snapshot. Keep private
receipts and media private. No migration command launches guests, changes
installed tools, reserves capacity or redirects an active run.

## Temporary compatibility

The three former paths in `capture/desktop/scripts/linux/gui/` are relative
symlinks to this implementation, not copies. All three remain because updater
launches the former runner path and runner/tests locate the reaper by sibling
path. Existing frontend documentation and active owners can continue using them.
Remove these links together only after updater callers/tests and maintained
documentation move, and active owner reservations/runs are checked. Do not
rewrite an active candidate merely to retire these entrypoints.

Further Mac/frontend extraction checks exact active hunks with their owners;
an open PR or pending native acceptance alone does not reserve stable mechanics. Storage/retention UI assertions remain Desktop
scenarios; a filename containing “native” does not make a test reusable.

[Desktop verification policy](../../capture/desktop/docs/native-testing.md)
continues to govern: shared UI and real Rust bridge primarily on Linux, affected
native seams on their OS, unchanged-input evidence reuse, and current-package
Mac publication smoke plus exact-main verification/signing. Relocation alone
requires path/contract checks, not a fresh native workflow or paid resources.
