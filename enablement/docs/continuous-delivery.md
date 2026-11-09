# Verification and publication

## Preview baseline restoration

Development resumes from Preview 0.12.2, tag `preview-mac-12-2` at
`cbaa8f74c65e0d5951138501977f684b03790e09`, including its matching Linux AppImages
(whose filenames retain Cargo version 0.3.13). Pre-restoration main is preserved
at `bf7671b1d72b1a71168627407fe51da32fda5e1a` on
`migration/main-preserved-2026-11-08`. The restoration is a forward commit, not a
history rewrite. Later migration work must be deliberately ported; merging an
already-ancestral branch does not automatically reapply restored-away changes.

The application, native adapters, dependencies and bundled resources match that
published preview. The exceptions are repository development guidance, this
verification documentation, the verifier's description, workflow triggers, the
new repository-checks workflow and its publication-trigger regression contract.
Reused native evidence belongs to the preview and its unchanged inputs; it does
not certify a newly built installer. Before publication, recheck platform,
packaging, signing and runtime inputs under the existing release gates.

Restoring source does not update installed apps, captures, user config, remote
services, published installers or update feeds. Publication is a separate task:
Linux release and website/Connect delivery have no automatic or scheduled trigger.
Review compatibility with the currently deployed service before publishing this
older client/server source.

## Continuous delivery

`Continuous delivery` runs automatically on PRs targeting main and main pushes.
It is the only automatic workflow: one run per PR open/reopen/commit update
against main, and one per push to main. It validates the theme, selector,
push-hook, publication-admission contracts and workflow syntax with read-only
permissions. Public web verification runs as a reusable job when Public/shared
web inputs change. The updater harness runs as a reusable job on main pushes
when its inputs change. Changes to the CI orchestrator select both consumers;
unavailable diff history selects both rather than skipping checks. Git diffs
include deleted and renamed source paths. There are no PR-edit or verification
lifecycle triggers. Superseded PR/main runs coalesce within their own event/ref.
Neither reusable consumer publishes on an automatic run.

Every non-deletion push runs cheap contracts plus the affected owners' local
suites, selected by `enablement/local-tests/run.ts` over the full merge-base delta.
Native-prerequisite, timing, CI routing and local-selector regressions run in the
cheap contracts in CI and locally. Failed selected commands reject the push.
Use `python3 enablement/verification.py --base <base> --run` for explicit affected
local checks. Dirty-tree, hidden-index and exact-HEAD push guards remain enforced.

Preview selection without executing tests:
`node --experimental-strip-types enablement/local-tests/run.ts --base origin/main --head HEAD`.
The hook supplies the merge-base; both rename sides and deletions are included.
Unknown/shared/root-build inputs, missing history and empty deltas select all
owners. Root documentation alone runs contracts. Tooling changes additionally
run directory-width tests; Hermes changes run its inspection suite. Public runs
its standalone verifier. Connect web runs its full package suite and Public
composition; client changes also run root integration; service changes include
client, browser, Public and root consumers. Desktop frontend changes run npm tests,
synthetic Electron smokes (Xvfb on Linux) and conservative root verification.
Capture script changes also load every tracked Python contract file explicitly
under Desktop/Agent scripts, including nested packaging and hyphenated Linux tests;
these run before expensive suites except the CLI integration, which follows the
root rebuild and receives its exact executable through `APPARENT_BIN`. New enablement test filenames are discovered
automatically. Other capture changes retain the broad Rust/Engine verifier. Shared UI
also selects Desktop, service and root integration. Package manifests/lockfiles
select the entire owner, with locked installs and ordinary build prerequisites.
Missing tools/packages fail rather than silently skip. Use Node 22.23+ and Bun
1.3.10 for CI routing and selected web/Public packages. Run hook-backed pushes once through
`t3-build` on Clanker.

This is owner-level selection, not inferred per-file test coverage. It runs the
canonical automated local suites; platform-only/ignored native scenarios, release
and publication checks still require their existing explicit procedures. A Linux
hook cannot certify macOS behavior. The advisory owner-lane report below remains
separate; it does not control the enforced hook selection.

The same workflow also calls the full Linux and Mac platform suites on main
pushes and on non-draft PRs (opened, reopened, updated or marked ready for review).
Main pushes preserve the active run and keep only the newest pending run in the
same concurrency group, including Public checks. PR updates still cancel their
superseded active run. Manual publication retains its separate noncancelling lock.
Full Linux and Mac verification and Mac Hermes inspection share the cheap
contracts, Linux Hermes inspection and selector prerequisites. Once those finish,
both full platforms can start in parallel; neither waits for the other's results.
Failed, cancelled or skipped contracts or Linux Hermes inspection prevent their
allocation. A failed or unavailable selector retains full platform coverage rather
than silently skipping it; cancelled workflows do not allocate these jobs. All
applicable platform/native acceptance and publication gates remain required.
Drafts retain
inexpensive Linux checks; converting a PR back to draft cancels its
superseded full run. Each update tests GitHub's proposed merge commit, with
read-only permissions and no release receipt. These jobs appear directly on the
PR, so their actual completion is visible without a status reporter or manual
dispatch. Existing separate manual runs do not substitute for this integrated
candidate. Publication remains manual and retains its existing admission policy.
This is a cheap admission barrier, not a global concurrency limit or priority queue.
Restoring a full `linux` prerequisite on the Mac jobs reinstates the former serial
platform scheduling. Manual platform diagnostics remain independent.

`Linux verification` and `Mac verification` remain manual diagnostic entrypoints,
each coalescing superseded runs within its own platform. Both call the same reusable
`verification-platform.yml`, preserving the full Rust, Engine/client, browser,
Electron and applicable native checks. Each checks the entire tree, not just the
latest push delta: changes from a cancelled earlier run cannot escape verification.
Explicit `workflow_dispatch` on a feature branch runs the same full test jobs.
Feature dispatch does not emit the main-only `Release verification` receipt and
cannot authorize publication. Neither workflow has an automatic push trigger.
Each caller's `verify / Release verification` job is its platform receipt.
General Mac verification uses `macos-15-intel` so it does not depend on the
standard Apple Silicon runner pool. The required Xcode/macOS SDK remains selected
explicitly. Preview publication stays on `macos-latest` for arm64 packaging; Intel
CI is not Apple Silicon runtime proof. An image change must still pass the same
checks before Mac publication.

Linux and Connect delivery are manual-only. Their admission job reads trusted-main
code and requires that exact commit's
successful Linux/shared receipt. Mac preview publication requires both the
Linux/shared and Mac receipts for the same commit. A queued/failed Mac run cannot
hold Linux publication; Linux success cannot authorize a Mac release.
`enablement/release_gate.py --platform linux|macos` validates the exact caller
workflow, upstream run/attempt, repository, main branch, SHA and every required job.
Successful exact-main `workflow_dispatch` receipts are admitted alongside historical
push receipts; this lets the manual-only verification workflow authorize delivery.
PR/feature-branch checks, skipped jobs, failed/cancelled runs, legacy combined receipts
and another platform's receipt cannot substitute for the required proof.

Automatic stale candidates are skipped before builds. Every downstream checkout
and website artifact uses the admitted SHA, including when `workflow_run`'s default
SHA points at a newer main commit. Publication locks are separate from cancellable
verification; a new push never cancels publication. Existing service activation,
schema, consent and native/product acceptance requirements are unchanged.

Manual/tag Linux releases require the same exact Linux/shared receipt; manual Mac
previews require both receipts. Linux `verify_only` still permits isolated branch
packaging without publication. Failed required verification leaves that platform's
last published version available; fix or revert main and verify the next candidate.
CodeQL managed setup is disabled by Simon's explicit instruction; it is not a
release gate.

`.github/workflows/stream-public.yml` checks Public on relevant PRs/main pushes
and publishes only after its own successful web/shared verification during a
manual main dispatch. It does not require Linux/Rust, Desktop or Connect proof.
The same-run sealed Public-only artifact, current-main guard, deployment ordering
and canonical readback protect publication. This is the bounded Public exception
to full-platform admission; native and Connect gates remain unchanged.

`.github/workflows/web-delivery.yml` retains Connect's existing verification and
hosted deployment gate, with no Pages publisher. The existing `website-production`
and `hosted-production` environments and activation variables are retained. This
policy does not enable a disabled publisher, provision resources or apply migrations.

## Checks and artifact identity

Public's credential-free job runs `cd public/site && npm run verify`: its source
build, allowlisted resource graph, browser and shared appearance/analytics checks,
publication contracts, compiled movie worker in workerd, and Public-only Pages
preparation. It seals `.build/website` with the run SHA and every file's checksum.
The deploy job downloads only that workflow run's SHA-named artifact and validates
it before uploading. PR/push jobs have no production credentials and cannot deploy.

Connect's credential-free job retains its publication contracts, full hosted
TypeScript/workerd/D1/R2 suite, independent browser build, composed website checks
and credentialed cross-origin synthetic browser scenarios. Its production gate
still requires full Linux/shared verification for the same commit. Its local
composition is an integration fixture, not a Public publication prerequisite.

Node 22.23.0 and Bun 1.3.10 remain pinned. Browsers use the website lockfile's
`playwright-core`. Public and Connect production publication use separate locks
that never cancel an upload. Public rejects a candidate that is no longer current
main or would overwrite an equal/newer deployed website. Superseded candidates
skip without upload or a new receipt; transport, auth and integrity failures remain
failures. Dispatch current main to publish a replacement when appropriate.

## Website activation: Cloudflare Pages

The Public deploy job publishes only the sealed `dist-apparent` to the isolated
`apparent-capture` Pages project, using `public/site/scripts/cd/publish-pages.ts` and
`public/site/wrangler.pages.jsonc`. It does not execute `public/site/wrangler.jsonc`, deploy the
hosted Worker, or connect to Clanker. See
[`../public/site/docs/pages-deploy.md`](../public/site/docs/pages-deploy.md) for the current build,
source-built homepage preservation, video limits, exact hostname cutover and rollback.
Set `WEBSITE_PAGES_API_TOKEN` in `website-production`; the existing
`WEBSITE_CD_ENABLED=true` gate enables only Pages. Remove obsolete SSH settings
when present; they are no longer read. The hosted activation gate is unchanged.

### Retired Clanker capture-site origin

After canonical Pages acceptance, the capture site's Caddy serving block,
release symlink and historical release directories were removed. Do not restore
the old DNS A record, configure website SSH credentials, recreate that release
tree or run historical overlay activators. Production `publish_site.py` already
rejects this root; its temporary-filesystem tests are not a deployment procedure.
Use only the Pages rollback in `public/site/docs/pages-deploy.md`.

The proxy is shared. Its container, volumes, unrelated sites and separate
document-product poller/deploy-control services remain in place. They must not
be removed as capture-site cleanup. The website publication lock remains as an
inert coordination inode; there is no active capture website behind it.

## Hosted activation (separate operator gate)

Complete [`../connect/service/README.md`](../connect/service/README.md)'s deployment prerequisites
first: approved Free-plan configuration and CPU proof, existing private Standard
R2 bucket, initialized D1/ledger, live Google/cookie/device/cron proof and security
review. A green local workerd suite is not those proofs. Do not activate this job
against an uninitialized database or change to Paid implicitly.

Configure `hosted-production`, limited to `main`, with:

| Kind | Name | Meaning |
| --- | --- | --- |
| Variable | `CLOUDFLARE_ACCOUNT_ID` | Approved account ID |
| Variable | `HOSTED_D1_ID` | Existing initialized database UUID |
| Variable | `HOSTED_R2_BUCKET` | Existing private Standard bucket name |
| Variable | `HOSTED_GOOGLE_CLIENT_ID` | Approved Google OAuth client ID (public) |
| Secret | `CLOUDFLARE_API_TOKEN` | Least-privilege token for this Worker/deployment and binding reads |

The existing Worker's `GOOGLE_CLIENT_SECRET` and `TOKEN_ENCRYPTION_KEY` stay in
Cloudflare. They are never supplied to Actions or rotated by CD. Restrict the token
as far as Cloudflare permits and review its actual access before activation.
`hosted_config.py` validates the required public identifiers and writes an ignored
`connect/service/wrangler.cd.json` next to the existing entrypoint. The fixed production
origins retain hosted API/auth/callback and legacy transition CORS; the UI stays
on app. Observability, workers.dev and preview URLs remain off. No CPU override,
resource creation command, database migration or ledger initializer is run.

After separate approval, set repository variable `HOSTED_CD_ENABLED=true` and
dispatch `main`. Wrangler uses strict deployment and tags the version with the
checked SHA. Readback requires one 100%-traffic version with that exact tag and
an anonymous `/v1/session` response with the canonical credentialed CORS headers.
This read-only probe does not prove live Google sign-in, authenticated cookies,
uploads or native integration. Inspect those separately before calling it live
acceptance. A failed readback fails the job; it does **not** silently change
traffic again or roll back a database. Disable the gate and use Cloudflare's
reviewed Worker version rollback after investigating; record the previous version
before the first activation. Website and Worker publication are independent, not
a distributed transaction. Protocol changes must retain backwards compatibility
or use a separately reviewed release sequence.

## Local verification

Run the commands in the checks section from the repository root (the Node commands
from their package directories). For a no-network deployment wiring check, generate
configuration using **synthetic** identifiers and run
`(cd connect/service && node_modules/.bin/wrangler deploy --config wrangler.cd.json --dry-run)`.
This proves configuration and packaging only, not permissions or deployment. Never
put real tokens in test fixtures, the generated file, artifacts, or logs.

## Rust verification environment

Before explicit local verification (or a direct main push) starts subprocesses
for Rust checks, the verifier
checks the caller's Unix `TMPDIR` (or `/tmp` when unset). Run the same read-only
check before direct expensive commands:

```sh
python3 enablement/verification.py --base unknown --preflight
```

Supply a short absolute, user-owned mode-0700 temporary root. Ancestors must be
root/current-user owned and not group/world writable, except root-owned sticky
ancestors supported by the Engine. Symlinks are rejected without following them.
The check diagnoses rather than replaces `TMPDIR`, repairs permissions or deletes
anything. For an isolated run on Clanker, create a private per-job
`/tmp/apparent-dev-<job>-<run>/` directory, explicitly pass it as child `TMPDIR`,
and keep its context alive until all foreground checks exit.
Retain ownership/cleanup receipts outside that tree for long-running work.

The pathname budget models ordinary `agent_capture` socket fixtures: Linux
`apparent-owner-<32 hex>/capture.sock`, macOS `ac-<32 hex>/s` (owner fixtures use
`/tmp` there). It counts filesystem bytes, including separators, with space for
NUL. This is a deterministic prerequisite, not a writability/bind guarantee,
fixture completeness check or native proof. Ignored Electron/native scenarios
must check their own longer HOME-derived socket paths. Keep hostile/sticky and
symlink production tests enabled; a friendly preflight environment does not
replace those assertions.

The platform workflow checks a private temporary root before its first Cargo
command. Its Rust/native job also performs the locked frontend install, explicit
Electron download/resolution and executable/input readiness check before compiling,
with a five-minute setup deadline. Download archives are cached by OS, architecture
and frontend lockfile; installed modules and test results are never restored.
Cold downloads remain required on a cache miss. These prerequisites do not replace
any Electron/Rust or native GUI assertion. Feature pushes
and PR CI run its regression tests as part of the cheap contracts; they do not
require a Rust test environment when no Rust suite is selected. Before any native
allocation, inspect the exact candidate and real scenario entry point offline:
verify fixture files/schema, selectors against current controls, required guest
capabilities and the existing canonical repository/issue budget ledger plus
allocation/cleanup receipts. Missing receipts are not zero spend. Follow the
native owner's existing preflight and shared queue; this command grants no
allocation authority and establishes no GUI acceptance.

## Advisory owner lanes

`python3 enablement/owner_lanes.py --base origin/main --head HEAD` prints a
versioned JSON proposal over the complete merge-base delta. `--json <path>` saves
it; `--summary <path>` appends an input/reason summary. Main CI retains this report
as `owner-plan-<runner>` and in the selector job summary. Its `HEAD^` comparison is
advisory only: cancelled-run coverage still comes from unchanged empty-tree full
verification. `executed: false` means the report supplies no successful-test proof.

Explicit manual entrypoints use `--lane <name> --run` (and `--online` for locked
Cargo dependencies). Lanes are `capture` (whole root build/test), `engine-staged`
(staged Engine package, not production Engine), `connect-client` (independent
Cargo build/test), `connect-web` (existing package build/browser fixtures),
`connect-service` (build and real synthetic workerd tests), `public-site`
(standalone build/package and website/publication contracts), and `desktop-ui`
(JS and synthetic Electron smoke). Use the Connect workflow for complete
composed-site and cross-origin acceptance, and platform workflow for real Rust
bridge/native checks. Entrypoints require the same toolchains, browser and system
dependencies as those workflows; they do not install OS dependencies or grant
native acceptance. Run heavy local commands through the existing build runner.

Client edits also select Capture: root tests intentionally reuse client scenarios
with the real AppConfig provider. Renderer edits select Capture for Rust bridge
integration. Shared UI feeds Desktop and service recipient UI. Connect web feeds
its retained composition fixtures and consumes five explicit Public assets. It is
not an input to Public publication. Service API changes
include root/browser consumers. Production Engine/Desktop/Agent remain root
modules with reverse dependencies; no lane claims their isolation. Shared,
unclassified, newly added, deleted, renamed or type-changed inputs and missing
history select every lane. Manifests and lockfiles follow their owner's consumers;
root build, packaging and verification inputs remain broad.

No report output controls execution, hooks, release admission or publishers.
Before enabling skipping in a separate issue, establish each suite's complete
input closure including toolchain/platform, native/build/packaging contracts and
verifier identity; compare against successful compatible proof covering the full
candidate, including cancelled-run changes. Missing or incompatible proof must
rerun. Validate shadow decisions against full execution and investigate omissions.
Dependency caches are not test receipts; no speedup is claimed here.

## Commit integration checkpoint

With `core.hooksPath=.githooks`, `post-commit` refreshes `origin/main` with a
five-second, noninteractive fetch on feature branches. When HEAD does not contain
that main commit, it prints `INTEGRATION REQUIRED` and the exact fetched SHA.
The owning agent should finish running operations, preserve any remaining
uncommitted work, rebase onto that SHA, inspect incoming changes and rerun affected
checks before starting the next work section. This is an advisory checkpoint,
not an enforced merge gate or an automatic rebase. A partial commit remains a
partial commit: the hook never stashes, stages, rewrites, pushes or changes files.

Detached checkouts, main, and in-progress rebase/merge/cherry-pick/revert/bisect
operations skip the checkpoint. A failed or timed-out fetch reports unknown
freshness without failing the successful commit. No background worker is started;
other worktrees are untouched. Existing worktrees acquire the hook when they
integrate this change; frozen evidence checkouts need not be updated. Local hook
checks do not replace current-main integration and review before merge.

## Verification timing diagnostics

Mac verification pins Rust 1.99.1 and checks the disposable Intel hosted runner's
compiler inventory before dependency-cache restore. Only the known unused 1.98.0
stable toolchain may be removed; unknown identities fail before mutation. Keep all
three workspace paths in the Rust cache even if client execution moves jobs:
Actions cache versions include paths as well as the visible key. Compiler,
environment and manifest invalidation remain Swatinem defaults. Mac dependency
caches may save after later native failures on main and feature branches under
Actions' normal branch isolation. Saving after failure is armed only once the
changed-tree compilation and tests succeed, avoiding immutable partial seeds
after compiler/setup failures. Caches never admit verification or publication.

The existing unoptimized-root observation (Mac run 42505355940) restored a 748 MiB
fallback cache in about 34 seconds and compiled root tests in 3m20s. Main already
has that profile; this change claims no profile or compatible-hit speedup.
Pinning prevents future compiler drift, and failure saves can avoid repeated cold
dependency compilation after a failed run. Those savings are projected, not a
measured walltime reduction; successful exact-key hits already skip cache saves.
No main-only save restriction is used, so isolated feature branches can seed caches.

After native prerequisites pass, the prepared Electron download-cache path is
exported to subsequent job steps. Clean packaging installs reuse that archive
while retaining the ordinary installer and its checksum/version checks.

Platform Rust jobs retain seven-day observation-only JSONL spans and Cargo HTML
compiler timings. Spans report elapsed active command time, never CPU time or
billed savings. Runner queue time is separate: use Actions job queued/started
timestamps; it is not included in command spans. No queue estimate is inferred
from workflow walltime. The exact candidate SHA accompanies spans.

The verifier times contracts, application build, test compilation and test
execution separately. With timing enabled, an identical Cargo test selection is
compiled with --no-run first; the original Cargo test command then executes in
full, including its normal incremental compiler check. Compiler HTML describes
compilation rather than harness execution. Packaging, bridge integration, native
assertions and native Storage socket/control readiness have named spans.
Readiness spans can overlap the enclosing native assertion span; do not sum them.

Missing/endless spans mean incomplete observation, never successful acceptance.
Started commands retain their start record if interrupted; TERM/INT/HUP are
forwarded and exit status is preserved. Artifact upload is guarded by the timing
step outcome, runs after failures, and missing diagnostics only warn. Diagnostics
record no arguments, environment dump, transcript or application data. Existing
native evidence, cleanup, 60-minute Rust timeout and release gates are unchanged.

## Portable verification routing

Each platform caller runs portable contracts and the independent Connect web
artifact/Chromium fixtures in its existing Linux `select` job. Its success is
a prerequisite of both platform jobs. `--execution portable` runs the theme,
selector contracts and selected Connect web fixtures; `--execution platform`
retains push/credential/staging and temporary-filesystem contracts, root and staged Engine Rust suites.
The ordinary local verifier defaults to `full`, preserving both inventories,
full-delta selection and unknown-input fallback. No environment variable silently
changes this split.

Connect Rust/client security, filesystem, credential and staging tests remain on
both platforms, as do Desktop tests, actual Electron integration, packaging and
Mac native assertions when Mac verification is selected (see Portable Settings PR
verification below). Both platform jobs depend on the successful Linux `select`
prerequisite; portable failures prevent their allocation. This is once per
platform caller, not reuse of another workflow's results. Reverting the explicit
routing and workflow
placement restores the previous duplicate execution.

The supplied six-hour cohort reports Mac changed-tree median 10:16 (n=32) and
successful whole-run median 21:50 (n=19); these are mixed profiles/cache states.
Only the portable fraction plus browser setup is projected to leave Mac runner
time. No measured whole-run savings or queue reduction is established by local
routing tests. Linux selection now takes longer before native jobs become
eligible; candidate timings must assess that latency tradeoff separately.
The disposable Mac Storage fixture waits for real Storage/Retention read receipts
before its single Change click. Its TypeScript CI driver separates the owned
Go-to-Folder shortcut, read-only AX sheet readiness, and one path-entry/Return
operation. The initial picker and nested Go sheet are distinct readiness phases;
no mutation, keyboard shortcut or preference write is retried. Existing exact
PID/window/sheet/control checks, Choose/Cancel/Reset assertions, native consent,
process cleanup and test deadlines remain required. This is test orchestration,
not a change to installed Desktop routing or application persistence policy.

## PR verification evidence

The automatic verification status reporter and its `Verification complete` commit
status have been removed to eliminate repeated GitHub API reconciliation.
Linux, Mac and Public verification workflows remain available under their existing
manual triggers. Exact-head verification, independent review, native acceptance
and publication admission remain required; inspect their workflow evidence
directly. The retired status is no longer required by branch protection.

## Portable Settings PR verification

Ready PRs whose complete base-to-merge delta modifies only the five reviewed
Settings renderer cards (`storage.tsx`, `retention.tsx`,
`connect-settings.tsx`, `error-reporting.tsx`, `reference.tsx` in
`capture/desktop/ui/frontend/src/`) use the existing full Linux verification
without allocating the Mac platform workflow. Both full platforms share the cheap
contracts, Linux Hermes inspection and selector barrier, then can run in parallel.
Only successful contracts and Linux Hermes inspection are allocation guards;
failed/unavailable selection or a missing output conservatively requires Mac.
Mac Hermes inspection retains its ordinary admission. Linux still runs the real
Electron build and smoke suites. Main pushes, manual Mac verification and Mac publication
retain full Mac evidence. This is a PR iteration exemption, not native acceptance
or evidence that Linux proves Mac window behavior.

The exact allowlist lives in `enablement/ci/select.ts`. Empty/unknown deltas,
additions, deletions, type changes, renames and every other input require Mac verification.
Shell/preload/bridge contracts, styles, onboarding, notes, shared UI, dependencies,
native code, packaging and CI changes are outside this exemption. New files are
not implicitly admitted. Changed native behavior still needs its platform proof.
Restore unconditional ready-PR Mac admission in `checks.yml` to roll back;
do not broaden the allowlist without reviewing the new platform boundary.
