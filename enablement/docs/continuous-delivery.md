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

## Repository checks

`Repository checks` runs automatically on PRs targeting main and main pushes.
It validates the theme, selector, push-hook, publication-admission contracts and
workflow syntax with read-only permissions. Feature pushes run the inexpensive
contracts locally; they do not rebuild the application.
Use `python3 enablement/verification.py --base <base> --run` for explicit affected
local checks. Dirty-tree, hidden-index and exact-HEAD push guards remain enforced.

`Linux verification` and `Mac verification` are manual diagnostic entrypoints,
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
anything. For an isolated run, create a fresh directory with
`tempfile.TemporaryDirectory(prefix='.av-', dir=Path.home())`, explicitly pass it
as child `TMPDIR`, and keep its context alive until all foreground checks exit.
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

## PR verification status (bootstrap required)

The `verification-status` job in `checks.yml` is a credentialed, event-driven default-branch reporter.
It publishes the commit status **Verification complete**, independent of the cheap
Repository checks. It never dispatches native work. PR changes, main pushes and
manual verification lifecycle events reconcile affected open main PRs. Candidate
events coalesce per branch; main pushes retain global reconciliation. Candidate code is read as data, never executed with the reporter token.

Native changes require both manual exact-head workflows, their latest run and
latest attempt, the complete current or parallel job union, and successful Mac
native assertions. Feature-only skipped release receipts are excluded. Unknown
inventories, missing history, fork candidates and policy mismatches stay pending;
failed/cancelled runs and skipped/missing required jobs fail. Successful evidence
is retained across unrelated owner-documentation changes on main; other new base
inputs require integration. Owner documentation alone needs no platform run.
Public-only changes require manual Public verification at the exact head (PR merge
checks do not substitute); they require neither Mac nor Rust. This PR status never
admits publication or replaces native product acceptance outside CI.

Bootstrap in order: merge reviewed reporter code into main under existing gates;
open/update the authorized probe PR; inspect pending in `gh pr view --json
statusCheckRollup`; dispatch only approved manual verification, demonstrate real
failure and success transitions with Actions/status links; then add precisely
`Verification complete` to required checks while preserving all live protections.
No protection should be enabled on an unproven reporter. Initial read-only inspection
found no main branch protection and no rulesets. This workflow cannot receive its
own default-branch events before merge. Parent delivery ownership must retain the
live transition proof and protection activation steps. A direct hand-written green
status is not reporter proof. No polling, watchdog or expensive automatic run is
part of this mechanism.

Policy-changing PRs are evaluated as data using Bun 1.3.10 YAML parsing.
The supported current/parallel job union must be present; each required command
step must succeed in its owning job and candidate order. Mac assertions are
required only on Mac. The manual run's triggering actor must retain repository
write/maintain/admin access, checked again before status publication. Changed
workflow policy is parsed as data and must preserve the trusted required job
union and successful candidate command evidence. This completion status does
not authenticate source-code correctness: independent source/security review
remains part of delivery under the existing repository policy, without adding
a second human GitHub approver requirement. It never authorizes publication.

GitHub status writes have no compare-and-swap: final candidate/evidence re-reads
and serialized writers reduce the race window; later lifecycle events reconcile
concurrent external changes. Live bootstrap and required-check activation remain
unverified until performed after merge under existing gates.

The reporter writes pending before reading a known PR's evidence and isolates
per-PR API failures so later candidates still reconcile. A PR lifecycle event
can invalidate its known head even if PR enumeration fails. GitHub outages that
prevent status writes, or prevent discovery of otherwise unknown heads during
a global scan, cannot be made fail-closed by commit statuses alone. Reporter
workflow failures remain operational errors; no green is proof of API freshness
during such an outage. Public command steps, including verification and sealing,
must succeed in the owning verify job; tolerated failures are rejected.


The disposable Mac Storage fixture waits for real Storage/Retention read receipts
before its single Change click. Its TypeScript CI driver separates the owned
Go-to-Folder shortcut, read-only AX sheet readiness, and one path-entry/Return
operation. The initial picker and nested Go sheet are distinct readiness phases;
no mutation, keyboard shortcut or preference write is retried. Existing exact
PID/window/sheet/control checks, Choose/Cancel/Reset assertions, native consent,
process cleanup and test deadlines remain required. This is test orchestration,
not a change to installed Desktop routing or application persistence policy.

## Reporter API budgets and recovery

The trusted reporter uses the workflow's Actions credential throughout; another
user token's quota does not establish recovery. PR lifecycle events reconcile
only that PR. Manual verification lifecycle events look up matching open PRs by
head branch and exact SHA; automatic Public runs and first-attempt in-progress
notifications do no API work. Requested/rerun-start events authenticate the latest
run and attempt before leaving pending without evaluating complete job evidence.
Delayed start events reconcile current completed evidence instead of leaving a
stale pending result. Completed events evaluate the current
manual runs. Main pushes and blank manual dispatches retain a global scan, with
all known heads invalidated before evidence reads. Global and candidate work use
separate job-level coalescing domains, including a separate targeted-recovery
domain. Reporter events have unique workflow-level keys: rejected events cannot
evict eligible work before the job condition is evaluated. Non-main PR events
are filtered both by the trigger and by the reporter.

Every process preflights the same Actions credential. Targeted runs have a
60-request hard budget and need at least 70 remaining requests; global runs have
a 200-request budget and need at least 210. Content reads pinned to immutable
commit SHAs are reused within the run; mutable refs, permissions, run listings
and final evidence reads are refreshed. Status writes are spaced by one second.
Quota exhaustion, the five-request reserve, or budget exhaustion halts the whole
run immediately; no further PR, status or evidence requests are attempted.

There is no polling, automatic retry, credential substitution or manual status
publication. A rate-limit failure records the response's reset/Retry-After time
and request ID without dumping response bodies, process environments or tokens.
After that time, allow a natural relevant lifecycle event or perform one targeted
recovery from trusted main:

```sh
gh workflow run checks.yml --ref main -f pull_request=1094
```

The recovery process must pass its own Actions-token quota preflight before any
status writes. A successful application suite or user-token rate-limit request
does not satisfy that condition. If quota is still insufficient, stop and retain
the gate hold until the newly reported reset; do not rerun application/native
verification to repair reporter delivery. Status-write/discovery outages retain
the documented limitation above: the reporter cannot invalidate a previously
published status when GitHub refuses the write. Reporter repair must be proven
live before making `Verification complete` required.
