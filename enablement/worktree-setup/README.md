# T3 worktree preparation

`node --experimental-strip-types enablement/worktree-setup/prepare.ts` prepares the shared `composition/architecture` check package. Add `--owner <name>` to prepare only that additional owner closure: `agent`, `app-shell`, `record`, `onboarding`, `resources`, `connect-client`, `public-site`, or `browser`. Repeat the option to prepare more than one. There is no install-all mode.

The command checks the already-installed state first, runs only the selected locked package installs when needed, then reuses `enablement/verification.py`'s read-only dependency preflight and existing package freshness checks. npm uses `ci --prefer-offline --no-audit --no-fund`; Bun uses `install --frozen-lockfile --prefer-offline`. A cache miss can use the ordinary configured network path. Lockfiles are never written by this tool. Package-manager errors, absent tools, stale copied package inputs and failed imports exit nonzero with the original diagnostic visible. Rerunning after a rebase rechecks current lock and source state; valid installs are left untouched.

The shared baseline is deliberately small. Owner selection is explicit because a new checkout has no Git diff from which to infer future task scope. The command installs no Rust toolchains or crates, does not compile, open a GUI, package, publish, activate, or rent a machine. It never copies another checkout's `target/`, `node_modules/` or absolute build paths. npm and Bun reuse their integrity-checked download caches; this is not a compiled-artifact cache.

## T3 project setup integration

For project `mcp:c63c1e72-12e4-40e0-9f78-10c3728e66ad` (`/home/simon/Projects/scrawl`), append the following script to the current empty `scripts` array. If the project already has scripts when applying the change, preserve each one and append this entry; do not replace the array.

```json
{
  "id": "apparent-worktree-prepare",
  "name": "Prepare Apparent worktree",
  "command": "node --experimental-strip-types enablement/worktree-setup/prepare.ts",
  "icon": "configure",
  "runOnWorktreeCreate": true,
  "async": false
}
```

`async: false` makes T3's top-level and delegated `t3_thread_launch` setup wait for completion; the installed `ThreadLaunchService` records a failed setup and does not advance that launched agent to its run. Do not infer success from a thread starting. In contrast, `t3_worktree_handoff` runs `ProjectSetupScriptRunner` and returns `setupScript.status: "started"`; it does not await the completion or translate a later failure into the handoff result. The caller must inspect the setup terminal/result or run the preparation command itself before relying on owner dependencies. A setup terminal is attached to the worktree thread, so the explicit owner command remains the portable completion check for handoff and recovery after rebase.

T3 project scripts have no task-context input contract. The baseline therefore runs automatically, while owner packages require an explicit follow-up command from the task's known owner. Never infer owner from the fresh worktree's empty diff and never compensate with all packages. Setup terminals run with the project's normal command environment; this command assumes Node, Python 3, npm and, for Bun owners, Bun are on `PATH`. Missing tools and network errors are reported as failures. Setup does not install tools or modify T3 settings itself.

## Cache evidence and limits

The observed host cache sizes (npm about 5.4 GiB, Cargo registry 1.7 GiB, Bun 825 MiB, sccache 919 MiB with an 8 GiB configured cap) establish available stores only. Current environment inspection found no `CARGO_TARGET_DIR`, no `RUSTC_WRAPPER`, no tracked root `.cargo/config.toml`, and no `/home/simon/.t3-upstream/worktrees/scrawl/.cargo/config.toml`; therefore sccache was not shown to be active for this checkout. This slice does not touch Cargo configuration or claim compiled cache hits. Shared Cargo registry downloads already avoid repeat network fetches, while copying a Cargo target between worktrees is unsafe because build scripts can embed donor absolute paths.

The initial validation records only real package-install/import counts and elapsed time from a cold synthetic checkout and its warm rerun. It does not attribute cache use from directory size or Cargo `Fresh` output. No compiled dependency cache phase is proposed: the documented cross-worktree Rust/CMake path and build-script path sensitivity make a safe measured win larger than this bounded setup slice.
