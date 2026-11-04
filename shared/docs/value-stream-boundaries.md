# Value-stream language and build boundaries

Accepted architecture direction, 2026-11-04. This decision defines the target;
it does not implement migration, switch installed routing or relax release gates.

## Outcome

A normal product change stays within its outcome owner. Owners that do not need
native execution use TypeScript throughout their maintained implementation and
can install, develop, build, test and package without a Rust toolchain, Cargo
invocation, source build or cache hit. Moving Rust behind a wrapper or into
`shared` does not satisfy this contract. Shipping dependencies may contain
precompiled native implementations, but must not require consumer-side Rust
compilation; native dependencies need an explicit supported distribution path.

## Ownership

| Owner | Useful outcome and target language boundary |
| --- | --- |
| Connect | Recording delivery and recipient access: account/auth journeys, credentials, consent, encryption, transfer queues, retries, sharing/revocation, viewer, service and remote CLI behavior in TypeScript. Zero repo-owned Rust, including clients, tests and build tooling. |
| Public | Discovery and marketing: website, content and owner tooling in TypeScript. Zero repo-owned Rust or Rust build dependency. |
| Desktop | Fast human capture controls and handoff: Electron/React/TypeScript owns Desktop workflow, Desktop-owned settings policy and persistence, and shell presentation. Desktop composes stream-owned settings; Connect owns the meaning, validation, persistence and effects of Connect settings even in Desktop Settings. Native adapters provide OS effects; Engine provides capture execution. Existing Rust/native presentation is migration debt. |
| Agent | Agent requests, inspection and proof orchestration: TypeScript owns workflow and semantic output; published Engine capabilities provide native recording/media operations. Existing Rust CLI behavior is migration debt, not a reason to duplicate recording. |
| Engine | Reliable capture/media execution: Rust and platform-native adapters own live sessions, audio/frame buffers, timestamps, native input/context collection, expensive media kernels and narrow resource/durability/safety enforcement. Product workflow does not belong here merely because it follows capture. |
| Shared | Stable reusable UI, schemas and utilities, not displaced stream policy. TypeScript consumers must not acquire Rust through shared packages or generated bindings. Capture-internal native sharing stays under `capture`. |
| Enablement / Composition | Verification and thin assembly, respectively. TypeScript tooling invokes owner entrypoints; neither becomes a product-policy owner or forces all streams through the root Rust build. Native assembly remains where required. |

These ownership boundaries do not require one process, service or repository per
stream. Desktop remains a capture tool; the recipient workspace belongs to Connect.
Capture-native support remains macOS/Linux only.

## Contracts and authority

Engine publishes commands, structured events and artifact references; consumers
do not import implementation modules. Keep raw audio/video in native memory or
files rather than routinely transporting it through the product event loop.
TypeScript may select keyframe/output policy and export recipes while native
workers calculate visual measurements, perform inference and render media.

The Engine contract owns actual first-frame readiness, stopping/draining,
resource lifetime, safe artifact finalization and enforcement of capture privacy
constraints. Product intent cannot override those facts. Connect owns whether
and how an authorized artifact is uploaded/shared; Engine must not import its
queue, credentials or sharing policy. A saved local artifact is not upload
consent, and a copied link is not proof of playback readiness.

Each provider owns its versioned schema and compatibility; the consumer owns its
adapter. Contracts must specify identity, authorization, errors, readiness,
timeouts, cancellation, retries, idempotency and recovery. Use existing bridges
where sufficient; this document chooses no new transport or worker topology.
Schema generation and package installation for TypeScript consumers must not
invoke Rust. Providers test real native execution; consumers use faithful
fixtures and separately exercise the real published boundary at integration.

Product persistence moves with its policy, with one authoritative writer and
preserved file formats, unrelated config keys, secrets, locking and cancellation
semantics. OS permissions and registration remain OS facts. Shared config needs
one supported mutation contract across languages before ownership changes; do
not introduce independent read-modify-write implementations. Retain native
credential storage/crypto primitives behind narrow supported adapters where
needed, without moving Connect product behavior into a Rust dependency.

## Current implementation and contained debt

At the decision's starting revision (`d11ecd29`):

- `connect/web` and `connect/service` have JavaScript/TypeScript build entrypoints;
  their complete independence is not certified by this document.
- `connect/client` is the production Rust client. `connect/hosted.rs` composes it
  into the root Rust application. Its independent Cargo build avoids GPUI and
  Engine, but still violates Connect's target. See
  [client ownership and contracts](../../connect/docs/hosted-client.md).
- Desktop/Agent policy and CLI behavior still live partly in Rust, including
  root composition and shared config/handoff modules. Existing bridge routes
  are not proof those owners have independent development loops.
- Production Engine modules are compiled into the root application.
  `capture/engine/src` is a staged admission/receipt package with no production
  recording callers; it is not an adopted standalone capture engine. See its
  [current adoption limits](../../capture/engine/README.md).
- `enablement/verification.py` has a bounded Connect browser-file selector;
  unknown/shared inputs conservatively broaden verification. Other owner and
  build-hook routes must be audited before claiming Rust-free change selection.

Do not add new product capabilities or widen cross-owner imports in legacy Rust
policy. Necessary correctness/security repairs and their regressions remain
allowed. Migrate coherent behavior with its callers/tests when in scope;
existing working implementations remain authoritative until accepted replacement.
This decision does not require an unrelated feature to perform an entire migration.

## Proof of a Rust-free stream

Before declaring an owner migrated, verify all of the following:

1. Inventory tracked owner sources, clients, CLI surfaces, tests, manifests,
   scripts, generated-code inputs and transitive build/package consumers. No
   repo-owned Rust or concealed Rust product behavior remains in its closure.
2. From a clean environment without Cargo/rustc or preexisting Rust build
   outputs, run supported installation, build, owner tests and packaging.
   Verify development mode also needs no Rust compilation. Native capabilities,
   when consumed, use an explicit prebuilt distribution with no source fallback.
3. Exercise representative workflow and failure behavior, including retry,
   cancellation and restart where applicable. For Connect, preserve auth,
   encryption/wire compatibility, credential handling, consent, sharing and
   revocation; do not replace meaningful integration proof with mocks alone.
4. Trace an ordinary owner-only feature delta through hooks and CI selection.
   It must select owner checks and applicable contract checks without compiling
   Rust. Unknown paths still fail conservatively; earn narrower selection by
   proving dependency closure rather than adding blanket directory exclusions.
5. Provider/native contract changes run affected integration/platform checks.
   Full main verification and exact-commit publication gates remain required
   under [continuous delivery](../../enablement/docs/continuous-delivery.md).
   Independent development does not waive assembled-app acceptance.

Enablement owns future executable source/import and build-command guards;
stream owners own workflow and package proof. No new guard is implemented here.

## Migration order and removal

First establish owner-local commands and measured baseline evidence. Connect is
the first candidate: reuse its service/wire contracts, define a TypeScript client
entrypoint and migrate one complete behavior with real callers, compatibility
tests and packaging. Its encryption/credential and Rust CLI consumers are
explicit dependencies to resolve, not work to hide in a shared native helper.
Choose the first behavior from current demand and active ownership before editing.

For each slice, name the sole authoritative implementation, consumers, rollback
route, material platform checks and donor-removal condition. Delete replaced Rust
and obsolete tests once all supported consumers/build inputs are verified;
temporary compatibility adapters must be thin, named and bounded. Do not retain
two independent policy implementations. Preserve existing protocol/data readers
and private evidence. Coordinate shared config and capture contracts with their
owners; avoid an umbrella rewrite or speculative new services.

Later Desktop/Agent slices use the Engine boundary to remove Rust from ordinary
product changes. Retain native safety enforcement and proportionate macOS/Linux
proof. Completion means adopted behavior plus demonstrated build independence,
not folder relocation, a package skeleton or this decision record.
