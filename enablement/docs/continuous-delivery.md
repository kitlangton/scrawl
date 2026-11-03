# Website and Connect continuous delivery

GitHub Actions do not run automatically on PRs or feature-branch pushes. Feature
pushes run inexpensive selector, push-hook and release-admission contracts locally;
they do not rebuild the application.
Use `python3 enablement/verification.py --base <base> --run` for explicit affected
local checks. Dirty-tree, hidden-index and exact-HEAD push guards remain enforced.

`Linux verification` runs on main pushes. Running verification finishes; only
pending work is replaced. `Mac verification` follows successful Linux runs and
checks their exact SHA, so every automatic Mac candidate retains matching Linux
proof even while main advances. Mac's running work also finishes and its pending
candidates coalesce; slow Mac capacity never blocks Linux. Manual main dispatches
remain diagnostic and cannot substitute for automatic release receipts.
Both use `verification-platform.yml` and retain full Rust, Engine/client, browser,
Electron and applicable native checks against the entire selected tree. Cancelled
pending runs cannot hide changes. Full verification is main-only; feature branches
use focused checks, independent review, current-base integration and required
native/platform proof before merge. Do not wait for full branch CI as a merge gate.
Linux's terminal job is `verify / Release verification`; Mac's terminal receipt
includes the exact tested SHA, since a workflow-run control SHA may be newer.
General Mac verification uses `macos-15-intel` so it does not depend on the
standard Apple Silicon runner pool. The required Xcode/macOS SDK remains selected
explicitly. Preview publication stays on `macos-latest` for arm64 packaging; Intel
CI is not Apple Silicon runtime proof. An image change must still pass the same
checks before Mac publication.

Linux and web delivery trigger only after successful `Linux verification`.
Their admission job reads trusted-main code and requires that exact commit's
successful Linux/shared receipt. Mac preview publication requires both the
Linux/shared and Mac receipts for the same commit. A queued/failed Mac run cannot
hold Linux publication; Linux success cannot authorize a Mac release.
`enablement/release_gate.py --platform linux|macos` validates the exact caller
workflow, upstream run/attempt, repository, main push (or Linux-verified Mac
follower), SHA and every required job.
PR/manual checks, skipped jobs, failed/cancelled runs, legacy combined receipts
and another platform's receipt cannot substitute for the required proof.

Verified main ancestors may publish while newer main commits are still testing.
Every checkout and artifact uses the admitted SHA. Existing publication locks
finish running work and coalesce pending work. Under those locks, Linux previews,
Pages, Worker and the Mac appcast compare source ancestry against the published
revision; equal/older candidates never replace newer deployments. Unknown or
divergent identities fail closed. Mac also retains its monotonic Sparkle build
number check. All writers must use these workflow locks; out-of-band deployment
is not coordinated by Actions. Activation variables, schema, consent and
native/product acceptance requirements remain unchanged.

Manual/tag Linux releases require the same exact Linux/shared receipt; manual Mac
previews require both receipts. Their optional `candidate_sha` selects an exact
verified main ancestor; empty retains the dispatch commit. The selected SHA flows
through checkout, tags, admission and feed ordering. Linux `verify_only` still permits isolated branch
packaging without publication. Failed required verification leaves that platform's
last published version available; fix or revert main and verify the next candidate.
CodeQL managed setup is disabled by Simon's explicit instruction; it is not a
release gate.

`.github/workflows/web-delivery.yml` retains the existing independent website and
hosted production environments and activation variables. This policy does not
enable a disabled publisher, provision resources or apply migrations.

## Local commit signing

Activate the versioned hooks in each clone with `git config --local core.hooksPath
.githooks`. This common-repository setting also applies to linked worktrees;
cloning or pulling does not activate hooks automatically. Keep the existing
pre-commit directory-width check and pre-push verification enabled.

`.githooks/reference-transaction` requires `git verify-commit` to accept every
new commit introduced by a local branch or detached HEAD update. It runs before
refs move, so unsigned initial commits, ordinary commits, amendments, merges,
and batched updates fail even with `--no-gpg-sign`, `commit.gpgsign=false` or
`--no-verify`. It imposes no author/committer identity restriction. Commits
already reachable from local refs/HEAD are excluded: existing history, fetched
commits, branch switches, resets, remote-tracking refs, tags and deletions are
not revalidated. This is local creation policy, not a historical or push audit.

Set `commit.gpgsign=true`, choose `user.signingkey` and `gpg.format`, and configure
verification trust. SSH signing requires `gpg.ssh.allowedSignersFile` containing
the trusted public signing keys; GPG uses the local keyring. A signature that
cannot be verified fails closed. GitHub's Verified badge separately requires
registering the signing public key with GitHub; local verification does not
prove registration. No private keys belong in this repository.

Local hooks are bypassable by changing `core.hooksPath`, changing the hook or
using another clone. For activation across worktrees before the source change
lands, a pinned copy of the tested hook set can live under the Git common
directory with `core.hooksPath` pointing there. Restore the previous hooks path
to roll that installation back; switch to `.githooks` after all active worktrees
contain the new hook. Server-side policy is separate.

## Checks and artifact identity

The credential-free verification job runs:

- `python3 -m unittest discover -s scripts/cd -p 'test_*.py'`;
- the complete hosted TypeScript build and real local workerd/D1/R2 suite;
- the website's frozen Bun install, public build, allowlisted `dist-apparent`
  packaging, and six-file public website suite;
- Connect's browser scenarios against a synthetic credentialed cross-origin API.

Node 22.23.0 and Bun 1.3.10 are pinned. Browser installation uses the version of
`playwright-core` in the website lockfile. Production is built with
`VITE_HOSTED_API_ORIGIN=https://hosted.tryapparent.com`. The production artifact
is saved **before** a separate loopback-configured browser build. The loopback
package is never uploaded for deployment. Artifact download is scoped to the
same workflow run and named with its admitted SHA. The website artifact includes
compiled Pages Functions, provider config and a complete checksum manifest outside
the served tree; publication uses `--no-bundle` and verifies the downloaded bytes.
The source-built homepage is never replaced by a committed deployment snapshot.
Website artifacts are retained for 90 days, with Pages deployment IDs and receipts
supporting rollback without rebuilding. Production runs are serialized
without canceling a deployment halfway through. Publishers admit verified main
ancestors but refuse to replace an equal or newer deployed revision. Pages treats
such a superseded revision as a successful skip, without uploading or issuing a
deployment receipt; remote lookup, upload and readback errors still fail the job. A relevant
later push supplies the replacement; otherwise dispatch the workflow on `main` after reviewing it.

## Website activation: Cloudflare Pages

The active website job now publishes only the sealed source-built `dist-apparent` to the isolated
`apparent-capture` Pages project, using `public/site/scripts/cd/publish_pages.py` and
`public/site/wrangler.pages.jsonc`. It does not execute `public/site/wrangler.jsonc`, deploy the
hosted Worker, or connect to Clanker. See
[`../../public/site/docs/pages-deploy.md`](../../public/site/docs/pages-deploy.md) for the current build,
accepted-homepage reconciliation, video limits, exact hostname cutover and rollback.
Set `WEBSITE_PAGES_API_TOKEN` in `website-production`; the existing
`WEBSITE_CD_ENABLED=true` gate enables only Pages. Remove obsolete SSH settings
when present; they are no longer read. The hosted activation gate is unchanged.

### Retired Clanker capture-site origin

After canonical Pages acceptance, the capture site's Caddy serving block,
release symlink and historical release directories were removed. Do not restore
the old DNS A record, configure website SSH credentials, recreate that release
tree or run historical overlay activators. The retired publisher, activators
and their obsolete protocol tests have been deleted.
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

The CI workflow has separate setup and focused Cargo steps before this verifier;
those steps are not covered by this verifier entry-point guard. Feature pushes
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
(JS and synthetic Electron smoke). Use the existing web workflow for complete
composed-site and cross-origin acceptance, and platform workflow for real Rust
bridge/native checks. Entrypoints require the same toolchains, browser and system
dependencies as those workflows; they do not install OS dependencies or grant
native acceptance. Run heavy local commands through the existing build runner.

Client edits also select Capture: root tests intentionally reuse client scenarios
with the real AppConfig provider. Renderer edits select Capture for Rust bridge
integration. Shared UI feeds Desktop and service recipient UI. Connect web feeds
Public composition and consumes five explicit Public assets. Service API changes
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

## Verification receipts and timing

`verification.py --run|--pr --receipt-dir <external-directory>` and
`owner_lanes.py --lane <name> --run --receipt-dir <external-directory>` record
observation-only JSON through `enablement/verification/receipt.ts` (Node 22).
The wrapper can also run a bounded existing command:
`node enablement/verification/receipt.ts --suite <name> --output <external-directory> -- <command> ...`.
It preserves the command's working directory and nonzero exit status.

Receipts include the actual checkout SHA, a fingerprint of all tracked and
nonignored working inputs, platform/kernel, tool-version digests, verifier digest,
start/end timestamps, duration, outcome, and an Actions run/attempt evidence link.
They include neither command arguments, output nor environment values. Generated,
ignored and external/native inputs are explicitly outside this observation's
attestation; **these receipts cannot skip verification or authorize publication**.
An initial `in_progress` record precedes command execution. Signals are forwarded
to the command group and yield `interrupted`; hard termination leaves incomplete
proof. Source mutations are recorded and never labelled reusable.

The platform workflow uploads renderer and changed-tree observations for 14 days,
including failures, and puts command duration in the job summary. Missing uploads
or unfinished records are not passes. For queue time, use Actions run/job timestamps
alongside receipt execution times; command duration does not include runner wait,
setup, or the entire platform workflow. No speedup is claimed from instrumentation.

## Bounded Connect client reuse pilot

The independent Connect client build/test step uses
`enablement/verification/client-suite.ts`. Reuse is eligible only on identified
GitHub-hosted **Linux** images; Mac and local runs still execute the suite. Root
Rust checks, real-provider consumer tests and native/platform proof always remain
required. This pilot does not cache any GUI/native acceptance or release artifacts.

The reviewed closure includes every tracked non-Public input, conservatively
invalidating consumers, shared code, workflows and tooling. The standalone client
has no Public source dependency. Client sources/fixtures/manifests/locks, root
Cargo manifests/locks and `.cargo` configuration are additionally pinned by a
reviewed signature. A changed package/configuration automatically runs in full;
updating that signature requires reviewing its new dependency and runtime-input
closure, not copying a new hash to make a test pass. New untracked work, symlinks,
unknown runner images, global Cargo configuration, missing tool probes and
application environment overrides disable reuse. Image, kernel, architecture,
Rust/Cargo/curl/compiler/libc/Node identities and relevant build/locale environment
also participate in compatibility. Values are hashed rather than published.

A successful baseline plus **two matching full shadow executions** are required
before the first reuse, independently per compatible input/environment identity.
After four reuses the next candidate runs the suite as an uncached result audit
(Cargo dependency/build caches remain usable). Audit/shadow failure disables reuse
for that verifier namespace, including after subsequent full passes. Reviewed
verifier changes create a new namespace and restart shadow qualification.

Actions caches only the small proof/tombstone JSON, with a unique run/attempt key
and a verifier/platform restore prefix. Automatic main workflows alone save it;
manual diagnostics cannot displace it. Failed audits are saved too. A missing,
malformed, incompatible, failed or incomplete proof always executes; malformed
proof also disables reuse. Whole-tree input identity, not the last-push delta,
prevents cancelled intermediate commits from hiding changes. Reuse reports both
the currently checked candidate and the original successful test SHA/evidence.
The `connect-client-proof-<runner>` artifact records decisions and duration for
14 days. No speedup is assumed until these live artifacts demonstrate it.

For a local full execution through the build runner:
`node enablement/verification/client-suite.ts --state <external-directory>/proof.json --offline --shadow`.
Local runs deliberately cannot qualify CI reuse. Never copy local proof into an
Actions cache or treat a dependency cache as a successful test receipt.

## Commit integration checkpoint

With `core.hooksPath=.githooks`, `post-commit` refreshes `origin/main` with a
five-second, noninteractive fetch on feature branches. When HEAD does not contain
that main commit, it prints `INTEGRATION CHECKPOINT`, the exact fetched SHA,
incoming paths/areas and paths changed on both branches. Shared/build/verification
changes explicitly prompt consumer assessment. Paths are advisory: disjoint files
can still share contracts. Assess incoming dependencies and integrate immediately
for conflicts or relevant changes; otherwise finish the current bounded slice.
Before merging, fetch current main, integrate, rerun affected checks and complete
independent review. This checkpoint never automatically rebases or relaxes that
pre-merge requirement. Preserve remaining uncommitted work explicitly: the hook
never stashes, stages, rewrites, pushes or changes files.

Detached checkouts, main, and in-progress rebase/merge/cherry-pick/revert/bisect
operations skip the checkpoint. A failed or timed-out fetch reports unknown
freshness without failing the successful commit. No background worker is started;
other worktrees are untouched. Existing worktrees acquire the hook when they
integrate this change; frozen evidence checkouts need not be updated. Local hook
checks do not replace current-main integration and review before merge.

Linux Main's signing-history migration uses the v2 release namespace. The one-time
`bridge_commit` dispatch shares the Linux publication lock and aliases verified
canonical v2 assets; it never admits an unverified branch build. The signed bridge
tag retains legacy ancestry outside Main, with an identical canonical source tree.
Keep that permanent v1 release for late upgraders and never resume v1 publication.
Preparation, exact evidence and rollback constraints are in
[`Linux signing-history migration`](../../capture/desktop/docs/linux/linux.md#signing-history-migration-november-2026).
