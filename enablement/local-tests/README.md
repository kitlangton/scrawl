# Local verification reuse

The local runner reuses successful Connect service serial Node tests on Linux x64/glibc by
default when their qualified inputs match. The audited closure is pinned in
`service.ts` to the service, shared UI and shared assets at `a918e744d`. Changing
that closure runs the original `npm test`. Expanding qualification requires
reviewing the command's transitive reads, subprocesses and environment again.
The service's 102 tests use synthetic identities and in-memory Miniflare, including
loopback OAuth endpoints. They do not consume Git ancestry or the native app.

Every selected command still runs through the existing owner plan. `npm ci` and
the service build run fresh, producing the worker and recipient assets before
receipt lookup. Only the serial Node test phase can be reused. Its full sorted
test inventory is identical to the canonical package script. Global Node module
search is disabled so missing local dependencies cannot resolve from the user's
home. Miniflare's optional live geolocation fetch is disabled in this qualified
phase, which never exercises `Request.cf`. Transport/lifecycle IDs are removed
from that subprocess environment. Every remaining variable is hashed, including
worker allocation, PATH, locale and temporary-directory settings. Environment
values are never written into receipts. Runtime-loader and dependency-path
overrides use the original fresh command.

Qualification also requires a clean checked-out exact HEAD, no hidden index
flags and integration with the current `origin/main` checkpoint. The runner
hashes the entire committed tree and actual source bytes, verifies every tracked
file against its committed blob and executable bit independently of Git's stat
cache, and binds all installed service
dependencies, generated build output, modes and internal symlink targets. The
Node executable, kernel/boot, OS configuration and transitive ELF loader libraries
of Node and installed glibc native dependencies are included. npm's inactive musl
variants are content-bound, with their incompatible loaders excluded. External source links,
special files, ancestor `node_modules`, unknown history and unreadable inputs
fall back to execution. Identical `npm ci` reinstalls can match by content. Inodes
and change timestamps are checked separately before and after each lookup/run to
reject installation races and input changes during execution.

Evidence lives in an automatically discovered, private worktree-specific
`/tmp/apparent-dev-local-reuse-<worktree-hash>-<random>/` job. No environment or CLI
argument can supply a receipt. One latest success expires after six hours and is
sealed with a random private local key. Ownership, modes, file types, links and
the seal are verified. An exclusive lock prevents cooperating writers from
sharing an in-flight receipt. Contention and interrupted locks run fresh. A
failed command removes the prior receipt before execution and never publishes
success. Atomic replacement follows successful execution and unchanged inputs.
Cleanup or storage failure loses only the optimization. Removing this job resets
the evidence. This trusts the local OS account that can already edit the hook
and its code. It does not authenticate evidence against a malicious same-account
writer capable of replacing both code and keys.

The hook's cheap contracts and its before/after clean-tree, hidden-index and
exact-HEAD checks stay fresh. Owner selection, unknown-input coverage, Rust,
Electron/browser tests, product native acceptance, hosted CI and release admission
retain their existing procedures. Local receipts cannot admit any hosted or
published artifact. Roll back reuse by removing the `serviceTests` call from the
runner. No service data or persistent configuration is migrated.

Run the regression suite with
`node --experimental-strip-types --test --test-concurrency=1 enablement/local-tests/*.test.ts`
through the normal build admission. It includes a real installed Git hook with a
synthetic test phase, receipt corruption/expiry/concurrency, failed commands,
environment and file/dependency invalidation, and mutation during verification.
Real package timing evidence is recorded separately from these synthetic tests.
