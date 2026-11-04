# Native test infrastructure

## Mac verification setup

`mac-setup.ts` runs the existing FFmpeg and Metal setup commands concurrently
after Xcode selection. Both must succeed before compiler/cache/build steps.
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

Mac/frontend driver extraction is deferred while Settings and other native
acceptance owners modify it. Storage/retention UI assertions remain Desktop
scenarios; a filename containing “native” does not make a test reusable.

[Desktop verification policy](../../capture/desktop/docs/native-testing.md)
continues to govern: shared UI and real Rust bridge primarily on Linux, affected
native seams on their OS, unchanged-input evidence reuse, and current-package
Mac publication smoke plus exact-main verification/signing. Relocation alone
requires path/contract checks, not a fresh native workflow or paid resources.
