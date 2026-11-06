# T3 worktree preparation

`node --experimental-strip-types enablement/worktree-setup/prepare.ts` prepares the shared `composition/architecture` check package. Add `--owner <name>` to prepare only that additional owner closure: `agent`, `app-shell`, `record`, `onboarding`, `resources`, `connect-client`, `public-site`, or `browser`. Repeat the option to prepare more than one. There is no install-all mode.

The command checks Node >=22.18.0, Python >=3.9.0 and npm >=9.0.0; Bun owners additionally require the repository-pinned Bun 1.3.10. It checks the already-installed state first, runs only the selected locked package installs when needed, then reuses `enablement/verification.py`'s read-only dependency preflight and existing package freshness checks. npm uses `ci --prefer-offline --no-audit --no-fund`; Bun uses `install --frozen-lockfile --prefer-offline`. A cache miss can use the ordinary configured network path. Lockfiles are never written by this tool. Package-manager errors, absent or unsupported tools, stale copied package inputs and failed imports exit nonzero with the original diagnostic visible. Rerunning after a rebase rechecks current lock and source state; valid installs are left untouched.

The shared baseline is deliberately small. Owner selection is explicit because a new checkout has no Git diff from which to infer future task scope. npm lifecycle scripts remain enabled: selected owners may compile TypeScript during installation, and App Shell prepares shared/contracts before installing its consumer. The command installs no Rust toolchains or crates and does not open a GUI, publish, activate, or rent a machine. It never copies another checkout's `target/`, `node_modules/` or absolute build paths. npm and Bun reuse their integrity-checked download caches; this is not a compiled-artifact cache. Bun owners always run their frozen install to validate the installed graph, since existing Bun preflights do not establish lock freshness.

Automatic and explicit preparation serialize on a worktree-local `.apparent-worktree-prepare.lock` directory. A competing invocation waits up to two minutes, then reports the exact lock and recovery prerequisite. Errors release the current invocation's lock. After a crash, verify that no preparation is running before removing its empty lock directory; the tool never deletes a competing lock.

## T3 project setup integration

For project `mcp:c63c1e72-12e4-40e0-9f78-10c3728e66ad` (`/home/simon/Projects/scrawl`), append the following script to the latest `scripts` array, preserving existing entries. The guard allows a worktree deliberately based on an older commit to start with an explicit unprepared warning; update that branch before relying on this setup command.

```json
{
  "id": "apparent-prepare",
  "name": "Prepare Apparent worktree",
  "command": "if test -f enablement/worktree-setup/prepare.ts; then node --experimental-strip-types enablement/worktree-setup/prepare.ts; else echo 'Apparent setup unavailable on this older commit; dependencies are not prepared.' >&2; fi",
  "icon": "configure",
  "runOnWorktreeCreate": true,
  "async": false
}
```

`async: false` makes launch paths through T3's `ThreadLaunchService` wait for setup completion; a failed setup prevents that launched agent from starting. This does not cover every delegation path: a real `delegate_task(isolation: "worktree")` smoke on `01c1eb046` started with the architecture dependencies absent despite the enabled project script. Include the explicit preparation command in delegated implementation briefs before checks or builds; do not infer preparation from a started agent or a workspace-prepared activity label. In contrast, `t3_worktree_handoff` returns `setupScript.status: "started"` without awaiting completion. Inspect the setup terminal/result or run preparation explicitly before relying on dependencies. Script IDs must contain at most 24 lowercase letters, digits or hyphens.

T3 project scripts have no task-context input contract. The baseline therefore runs automatically, while owner packages require an explicit follow-up command from the task's known owner. Never infer owner from the fresh worktree's empty diff and never compensate with all packages. Setup terminals run with the project's normal command environment; this command assumes Node, Python 3, npm and, for Bun owners, Bun are on `PATH`. Missing tools and network errors are reported as failures. Setup does not install tools or modify T3 settings itself.

## Cache evidence and limits

The observed host cache sizes (npm about 5.4 GiB, Cargo registry 1.7 GiB, Bun 825 MiB, sccache 919 MiB with an 8 GiB configured cap) establish available stores only. Current environment inspection found no `CARGO_TARGET_DIR`, no `RUSTC_WRAPPER`, no tracked root `.cargo/config.toml`, and no `/home/simon/.t3-upstream/worktrees/scrawl/.cargo/config.toml`; therefore sccache was not shown to be active for this checkout. This slice does not touch Cargo configuration or claim compiled cache hits. Shared Cargo registry downloads already avoid repeat network fetches, while copying a Cargo target between worktrees is unsafe because build scripts can embed donor absolute paths.

The initial validation records only real package-install/import counts and elapsed time from a cold synthetic checkout and its warm rerun. It does not attribute cache use from directory size or Cargo `Fresh` output. No compiled dependency cache phase is proposed: the documented cross-worktree Rust/CMake path and build-script path sensitivity make a safe measured win larger than this bounded setup slice.
