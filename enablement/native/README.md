# Native test infrastructure

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
