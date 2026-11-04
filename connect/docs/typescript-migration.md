# Connect TypeScript migration inventory (#529)

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
`client/typescript/src/settings-controller.cts` through the generated Desktop
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
