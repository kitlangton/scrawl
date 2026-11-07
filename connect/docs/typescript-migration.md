# Connect TypeScript migration inventory (#529)

## Accepted rebuild and current checkpoint

Connect has Delivery and Viewing modules under one accountable owner during the
rebuild. Delivery owns account authority, consent, transfers, access and retrieval;
Viewing owns the browser recipient/account experience. TypeScript owns product
behavior, CLI, tests and tooling; React/shared controls supply UI. Worker/D1/R2
remain the hosted runtime. Cross-module consumers use published contracts.

The first replacement checkpoint moves the adopted account/crypto package to
`connect/delivery/client` and registers its independent value-stream artifact.
Desktop still consumes the same emitted controller. The service, viewer and
independent browser delivery are TypeScript; the canonical combined Worker at
`app.tryapparent.com/connect` has verified live sign-in and shared-link management.
The remaining Rust state/queue/CLI and their production consumers remain migration
work; this does not establish whole-Connect independence. Subsequent
slices must adopt those callers, preserve one writer and retire replaced donors.

Current acceptance inventory (library/fixture capability is not installed adoption):

| Workflow | Implemented checkpoint | Remaining acceptance |
|---|---|---|
| Browser and hosted API | Canonical combined TypeScript Worker, live sign-in and shared-link management | Preserve live routing/auth/privacy contracts during later changes |
| Account and Settings | TypeScript account/device policy and receipt-fenced private session; device/revoke HTTP, approved credential preparation, login-marker and private identity publication plus revocation-state preparation | Rust still supplies installed HTTP/private facts and durable effects; login/logout/config adoption remains |
| CLI and recipients | Grammar, local and remote account-status reads, share-management transport, state-backed retrieval/publication and callable Node host | Installed argv/host/resource/writer admission and platform proof |
| Renewal | Compatible journals, exchange, completed recovery and coordinator library | Live sole-writer adoption, installed recovery and removal of temporary Rust bridge |
| Queue and sender | Planner, fenced admission/retirement/retry/worker and encrypted spool/HTTP capabilities | Host settings and stable snapshots, host recovery integration and uncommitted-spool settlement, worker/caller adoption |
| Native resources | Protected x64 factory and qualified Linux/Mac helper inputs | Supported-target installed assembly, including ARM, remains an adoption gate |
| Removal and release | TypeScript owner CI and qualified library artifacts | All live callers, packaging/build inputs and Rust donors must be migrated and audited |

The TypeScript recipient boundary now decodes bounded raw JSON without duplicate
keys, validates signed envelopes, decrypts bounded age streams through final EOF,
and checks inner manifests, content hashes and bundle references. This is an
in-memory integrity capability. The authenticated recipient retrieval API now
uses bounded bearer-only HTTP, resolves the current read identity and approved
upload sender from the service peer registry, pins context, verifies downloaded
bytes and rechecks sender authorization after decryption. It has explicit legacy
plaintext opt-in and supports grouped evidence-only reads. It is not a credential
store or filesystem publisher. The migration-only Rust producer
interop lane exercises the same boundary; installed retrieval still uses the
Rust client until transport, durable state and safe publication are adopted.

The recipient publication library now privately stages verified bytes and uses
the admitted native resource adapter for ACL checks and atomic no-replace
publication. Linux and Mac consumer tests cover the full filesystem path and a
synthetic encrypted CLI fetch; uncertain outcomes retain their data without retry
or deletion. This is an injected library capability, not installed adoption:
installed credential loading/renewal, provider assembly and the production CLI route still
use the authoritative legacy path until their separate acceptance gates pass.

Stored credential wire decoding, private-descriptor reading and queue-locked
credential snapshots are TypeScript capabilities with Linux/Mac native consumer
proof. Directory admission, inode/ACL checks, cancellation, revocation and local
login-incarnation fences preserve the legacy lock protocol; stored facts do not
authorize transport or clear `authRequired`. Completed legacy renewal responses
can now be read under the same queue lock when the full journal base matches;
recovery uses the original issue time and leaves all durable files unchanged.
Active renewal and durable journal publication, service authorization policy,
installed caller adoption and writer cutover remain parent integration work.
Rust remains the sole credential and queue JSON writer.

The state-backed recipient command now joins these read capabilities with the
legacy `<login-id>.identity` format, authenticated retrieval and native private
publication. It checks local credential lifetime, lazily reads keys only for
encrypted manifests and rechecks the local snapshot before publication. Explicit
legacy plaintext policy remains separate. Expired renewable access reports
`renewal_required` unless a matching completed response supplies a usable token.
A 401 permits one reload/retry only if writer state supplies a different token in
the same login. No refresh request or durable write occurs; active renewal and
installed routing remain open.
The package API has Linux/Mac native joined-consumer proof, not installed CLI
adoption or writer-cutover proof.


The next recipient adoption prerequisite adds a bounded private request to the
existing writer for read-grant renewal and terminal rejection. The TypeScript
transport prepares fresh authority for each GET, renews within 30 seconds,
retries only a rejected GET once and delegates `authRequired` marking after a
second 401. The writer pins server, generation, token and cancellation; it owns
all journal/credential commits and cannot select upload scope or config effects.
This is the temporary same-writer adapter permitted by step 2 below, not the
zero-Rust target or installed adoption. Its source-bound joined Linux/Mac proof uses
a synthetic service, the real writer executable and the native resource adapter;
installed proof remains required. Published resource/provider assembly
and the separate legacy writer launch/admission are also required before routing
production consumers. Rust still owns renewal product behavior in this bridge;
move it with the coordinated writer and remove the bridge at that cutover.


Read with [hosted-client.md](hosted-client.md), [hosted-api.md](hosted-api.md),
and [hosted-encryption.md](hosted-encryption.md). This is a source inventory,
not proof that the TypeScript replacement or routing has been adopted.

## Active route and consumers

The active production path is `composition/cli.rs` → `connect/hosted.rs` →
`apparent-connect-client` (`connect/client/src`). Desktop Settings calls the
same hosted facade through `capture/desktop/settings/connect.rs` and
`capture/desktop/settings/settings.rs`; sender completion/sharing and the
background worker also enter `connect/hosted.rs` from Desktop composition.
`connect/hosted.rs` is an adapter, not a second policy owner. Its public surface
includes `settings_backend`, `copy_link_handoff`, `prepare_public_link_handoff`,
sharing state/progress/size helpers, `account_view_in`, `command`, `fetch`,
`inspect_source`, and `start_worker`.

The adopted Electron account workflow additionally enters Connect's
`delivery/client/src/settings-controller.cts` through the generated Desktop
entry. Its backend RPC uses the existing Connect socket worker for Rust local
private-record facts/renewal/durable operations. `settings-state.cts` owns
local account-state meaning from `stateFacts: 1`; the old snapshot remains only
for older consumers. With `deviceFlow: 1`, the adopted
`device-flow.cts` also owns device approval validation, polling/expiry, grant
validation and commit intent; native code temporarily executes the two closed
HTTP routes and one-shot credential/config publication. The TypeScript controller owns consent,
confirmation, cancellation/receipt handling and activation intent. Native EOF
guards and the current client writer remain authoritative for durable effects.
Legacy opening fallback is read-only and occurs before any mutation; older/native
surfaces retain their previous controller. This is explicit coexistence debt,
not a second queue/config writer or completed account migration.

CLI contract currently lives in `ConnectCommand`/`SharesCommand` in
`connect/client/src/lib.rs`: `login`, `status [--account]`, `disable`, `logout`,
`retry`, `upload <bundle>`, and `shares list|revoke <id>`. `cloud` is a retained
alias in root composition and is a compatibility contract. `fetch` and
`inspect` are existing root CLI commands, not Connect subcommands. Preserve
stdout/JSON shapes, exit/error boundaries, flags, fetch publication, and the
existing `apparent connect`/`apparent cloud` names.

The independent service and browser already use TypeScript. Their owner-local
inputs are `connect/service/{package.json,package-lock.json,tsconfig.json,
wrangler.toml,src/**,test/**,scripts/**}` and
`connect/web/{package.json,bun.lock,tsconfig.json,vite.config.ts,src/**,
test/**,scripts/**}`. Service owns HTTP/API policy; web owns browser auth and
recipient presentation. Neither should become a hidden client-state writer.

## Persistent formats and serialization boundary

| Location / file | Current contract and writers |
|---|---|
| `~/.config/apparent/config.json` | Shared JSON config. Connect reads fresh values; root `shared/handoff/config_store.rs` serializes durable updates. Owned fields are `cloud.enabled`, `cloud.uploadConsent`, `cloud.server`, and `experiments.hostedSync`, `experiments.hostedSharing`, `experiments.hostedEncryption`, `experiments.connectSettings`. Preserve unknown root/nested keys and secrets. `cloud.enabled` and consent remain independent; neither a credential nor sign-in implies consent. |
| `<config-dir>/hosted/upload.credential`, `read.credential` | Private JSON credential records, one per scope; contain bearer/refresh credential, server, generation, grant and expiry/identity metadata. Read must remain secret-free at UI/CLI presentation boundaries. |
| `<config-dir>/hosted/<id>.job` | Private JSON queue record, one per random 32-lowercase-hex capture ID; includes bundle/source identity, status, encryption/public-copy intent, retry/error/size state, grant generation and share state. Atomic replace; retain compatible pending/terminal/canceled semantics. |
| `queue.lock`, `worker.lock` | Private single-link lock files. Rust uses `flock(LOCK_EX)`; lock order is **worker → queue → config**. `worker.lock` spans `process_once`, including uploads; `queue.lock` protects short state transactions and is released for network work so foreground cancellation can proceed. |
| `cancellation` | Atomic JSON string epoch. Requests/renewals compare it across waits so late completions cannot restore state after disable/disconnect. |
| `revocation-<random>.intent` | Atomic private JSON boolean (`logout` intent). Publish before queue wait; any unreadable/pending intent fails closed across restart. Settlement removes only intents covered by committed effects. Never manually delete to recover. |
| `<scope>.renewal.lock`, `<scope>.renewal`, `<scope>.response`, `<scope>.login` | Per-scope flock and atomic JSON renewal/login journal files. Preserve response recovery, generation checks, cancellation and restart behavior. Renewal acquires its scope lock before short queue transactions; never acquire the renewal lock while holding queue. |
| `<login-id>.identity`, `<id>.sealed/`, temporary staging and publication locks | Private endpoint identity/encryption artifacts and staged encrypted transfer. Preserve exact age v1 envelope/manifest, Ed25519 signed bytes, stream/part assembly, no-follow/private modes, checksum checks and atomic complete-bundle publication. |

All JSON writes use create-new private temporary file, fsync, rename, and parent
directory fsync. Readers reject unsafe symlinks, permissions, ownership, sizes,
malformed state, and unsafe lock replacement. Rust's queue/config protocol
cannot safely coexist with an independent Node read-modify-write writer. The
filesystem lock files are protocol, not advisory documentation.

## Safe staged ownership transition

1. Keep Rust as the sole mutator of config, queue, credential, renewal,
   cancellation and revocation state while TypeScript gains compatible
   read-only parsing/presentation and transport/crypto interoperability.
   No Node process may mutate shared JSON while the Rust implementation can run.
2. If an interim slice needs TypeScript product logic while Rust still owns
   durable mutation, expose a narrow request/response mutation contract through
   the existing Rust owner (or a single Rust command endpoint): typed
   operations, validated arguments, fresh read under lock, exact atomic commit,
   bounded response, explicit cancellation result. It must not hand TypeScript
   a file path for direct writes or make Rust a policy bypass. Keep one worker
   and one writer. Mark this adapter temporary debt.
3. Convert the complete conflicting state machine as one coordinated cutover:
   stop/disable Rust worker and CLI mutation entrypoints before enabling the
   TypeScript writer; port shared lock/atomic-write semantics (including
   `flock`, lock order, cancellation, immutable revocation intent and restart
   recovery); then verify both-process exclusion and stale completion races.
   Do not dual-write, mirror state, or run Rust and Node workers concurrently.
4. Retain backward-compatible readers for supported persisted credential,
   queue, renewal, sealed transfer and wire formats. Rollback remains safe only
   while the previous Rust reader can consume every state TypeScript writes;
   otherwise cutover must keep writes in the old-compatible schema or make
   rollback explicitly unavailable and blocked from release.

## Build, test, hook and package consumers

- Root `Cargo.toml` depends on `connect/client`; root `Cargo.lock` includes it.
  `connect/client/Cargo.toml` and `Cargo.lock` are a separate package. Removing
  the donor requires updating both root manifests/lock and any feature wiring.
- Root facade imports the crate in `connect/hosted.rs`; CLI and Desktop imports
  above are direct consumers. Root hosted tests include
  `connect/hosted/hosted_{tests,settings_tests,logging_tests}.rs`; the root
  package activates client `test-support` for shared real-provider scenarios.
  Client has unit/test-support modules and
  `connect/client/tests/evidence-service.ts`, which currently drives the Rust
  application binary and must be redirected to the new TypeScript entrypoint.
- `enablement/owner_lanes.py`: `connect-client` lane executes Cargo build and
  tests; service lane runs `npm ci`/`npm test`; web lane selects browser checks.
  `enablement/verification.py` currently treats Connect client/service and
  `connect/**` changes conservatively as root/shared inputs. `enablement/verification/client-suite.ts`
  has a dedicated Rust client suite. These selectors/commands and owner lanes
  must move to Connect TypeScript checks without making unknown/shared/native
  changes less conservative.
- `.githooks/pre-push`, post-commit selectors, CI workflows and root
  `build.rs`/Cargo packaging need a full reference audit before donor deletion.
  Agent install and Desktop packaging paths build/package the root app; remove
  any transitive Cargo/rustc requirement only after checking their actual
  commands and emitted artifacts. Service `test:evidence-cli` is a consumer of
  the client evidence script. Rust removal must not remove native Desktop/Engine
  Rust or alter capture packaging.

## Donor removal gate

Delete `connect/client/**`, `connect/hosted.rs` and `connect/hosted/**` only
after every CLI, Desktop Settings, capture handoff, share, worker, fetch and
test consumer uses the supported TypeScript composition; the old route is no
longer active; persisted state and encrypted artifacts remain readable; hooks,
CI, service tests and installers no longer invoke this package; and repository
search/build-input review finds no remaining imports, manifests, tests or
commands. Keep the compatibility facade only while named callers still depend
on it; remove each entry when its consumer migrates. This inventory does not
authorize enabling uploads, changing consent defaults, deployment, or removing
unrelated Rust/native code.
