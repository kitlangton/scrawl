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

### Record, Agent and Engine output boundary

Accepted clarification, 2026-11-05. This supersedes the Desktop/Agent wording
above for capture-to-handoff output ownership; it records a target, not completed
adoption. Record is an independent TypeScript value stream, not a Desktop shell
subsystem. There is no separate Processing value stream.

- **Record:** capture what the user means and make it ready to hand off. Owns
  the capture-to-handoff workflow and default output policy: keyframe selection,
  transcript/notes assembly and bundle assembly. Exposes a headless TypeScript
  API so Agent can request the standard outcome without Electron.
- **Agent:** understand recordings and demonstrate work. Owns inspection,
  targeted extraction and proof-video workflows/recipes. Consumes Record's
  headless API for standard capture-to-handoff, and published Engine capabilities
  for specific native operations. Starting a capture from Agent does not transfer
  ownership of default output policy to Agent.
- **Engine:** Rust/native capture and media execution, including transcription
  inference, decoding, visual measurements, rendering and safe finalization.
  Record and Agent choose product policy; Engine enforces native safety,
  resource lifetime and truthful execution/completion facts.

Dependency direction for the standard outcome is Agent → Record → Engine;
Agent may also consume Engine directly for specific native operations. Record
does not depend on Agent. Connect consumes completed output for authorized
delivery/viewing without owning its production policy. Existing implementations
remain authoritative until each replacement and its consumers are verified.

### Remaining capture feature ownership

Recorded 2026-11-05 under Simon's authorization to resolve the remaining gaps.
These target assignments supersede older generic Desktop/Engine product-policy
assignments for the features below. They do not claim implementation parity,
schedule another thread's work, switch routing or authorize deletion/activation.
Record and Agent product behavior stays TypeScript; Engine stays Rust/native.
No additional Processing, Storage, Phone or Settings value stream is required.

| User outcome / feature | Accountable owner | Capability boundary and consumers |
| --- | --- | --- |
| Keep local recordings available without filling the disk | Record | Owns capture location, local retention preferences, eligibility/scheduling, usage presentation and recovery experience. Engine provides bounded filesystem/media operations, durable recovery/finalization and exclusion protecting live captures/readers. Record proposes cleanup; Engine may refuse unsafe operations. Agent consumes the same local lifecycle policy rather than adding another cleanup scheduler. |
| Preserve recordings while they are being delivered | Record locally; Connect remotely | Connect owns transfer lifecycle and remote retention/deletion. It holds an explicit artifact-use claim during transfer; Engine enforces the claim against local cleanup. Record does not inspect Connect queue internals. Local expiry and remote revocation/deletion are separate actions; neither implies the other. Preserve current pending-transfer protections until an accepted replacement exists. |
| Capture an attached iPhone | Deferred; no current maintenance obligation | Simon explicitly deferred iPhone support. Phone discovery, mirroring, recording and phone-specific shrinking need not be maintained or adopted by Record, Engine, Agent or Onboarding in the current rebuild. They are not acceptance gates. Existing code is retained pending separately scoped removal; preserve existing recording data/readers. Resumption requires an explicit scope decision. |
| Draw, type notes or use a whiteboard while explaining | Record | Owns pen/notes/whiteboard workflow, preferences and semantic inclusion in default output. Engine/native adapters supply actual input, overlay/window mechanics and capture-safe geometry/exclusion. Onboarding consumes factual Record outcomes for teaching; it does not implement drawing or manufacture accepted-stroke facts. |
| Configure recording, transcription and export | Record | Owns recording preferences, provider/model selection intent, default output/export recipes and user-facing fallback choices. Engine reports supported providers/models, readiness and execution failures and performs inference/rendering. Provider secrets remain behind their supported credential adapter. Agent owns preferences specific to its proof recipes. Existing Engine policy remains authoritative until the complete replacement is verified. |
| Inspect frames, zoom into evidence, reprocess or retranscribe | Agent | Owns command behavior, targeted inspection and recovery messages. Uses Record's headless API for standard bundle regeneration/default output; uses Engine for specific decode/crop/transcription operations. Distinguish derived inspection artifacts from an explicit rewrite of the canonical bundle. Record retains default output policy. |
| Export a recording or produce a proof video | Record for standard recording export; Agent for proof workflows | Record owns the ordinary export recipe and human export workflow. Agent owns proof-specific sequencing, narration and evidence recipes. Engine executes rendering, synthesis and encoding with safe destination/finalization rules. Shared native execution does not transfer recipe ownership or require a second renderer. |
| Share, retry, revoke or retrieve a recording | Connect | Owns consent, account/auth, credentials, encryption, durable transfer/retry, remote access and recipient viewing. Record offers an authorized artifact and presents delivery facts; Agent consumes Connect's public client for retrieval/sharing. Local save, copied link and ready-to-watch remain distinct facts. Connect is excluded from the current Agent slice, not unowned. |
| Install, update and launch the assembled application reliably | Platform / Composition | Owns app-level startup/login preference, About/build identity, update channel, update/relaunch/recovery UX, artifact assembly and installer/updater integration in TypeScript. Native adapters own OS registration, signing/update installation mechanics and truthful installed state. Relaunch respects active Engine sessions. This is app infrastructure within the existing Platform owner, not another value stream or permission to publish. |
| Configure a stream in one Settings window | Each stream; Composition hosts | Each stream owns its settings schema, validation, persistence and effects. Composition owns navigation, system-theme presentation and supported host/config adapters. One authoritative transactional mutation contract preserves unrelated keys/secrets. There is no central Settings policy owner or second config writer. |
| Learn capture and repair setup | Onboarding for teaching; Record for recording readiness | Onboarding owns journey/progress, practice association and coaching. It consumes Record's general capability API; Record consumes Engine/native permission facts and allowed repair actions. Composition supplies bounded window/activation/persistence effects. OS grants and successful registration must be observed, never inferred from saved preferences. |
| Install Agent tooling and skills | Agent | Owns standalone CLI/skill installation, compatibility and actual installation checks, plus agent-demo/proof preferences. Composition hosts its Settings contribution; Onboarding may explain it without owning the installer. |
| Diagnose failures with consent | Each owner; Platform composes app consent | Each owner owns its errors and recovery diagnostics. Platform owns the app-wide technical-reporting preference and common transport adapter; streams respect it and preserve redaction. No capture/transcript upload follows from diagnostic consent. Enablement owns test infrastructure, not product recovery policy. |
| Legacy dev-page registration and smart routing | Agent for any retained developer integration | Existing registration/SDK remnants are compatibility/dead-code assessment inputs, not an active delivery feature. Record preserves explicit local handoff; Connect owns authorized remote delivery. Do not restore automatic routing or send to a session. Verify consumers/build inputs and separately scope removal before deleting legacy code. |

Implementation coverage remains distinct from this ownership map. Record's
output/lifecycle adoption, Agent CLI/media parity, Connect client integration
and Platform's installed maintenance route remain acceptance work for their
respective owners. A package skeleton, fixture or successful build does not prove
native recording, permission, clipboard, device or packaged-app behavior.
Provider changes require affected consumer/OS checks; retain existing formats,
sole writers, active-use protections and installed fallbacks until adoption is
verified. This clarification alone creates no new implementation thread.

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
- Legacy Engine runtime, bundle, export and native modules remain compiled into
  the root application from `capture/engine/`. `engine/core` is the standalone
  Store/admission crate consumed by the native Engine provider; it does not
  replace the legacy recorder. See its [core contract](../../engine/core/README.md)
  and the [legacy adoption limits](../../capture/engine/README.md).
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

## Public adoption checkpoint

Public now owns TypeScript source/tooling and independently builds/packages its
website and Pages Function. Its publisher is TypeScript; the shared color guard
has one TypeScript implementation with a legacy Python forwarding entrypoint for
other owners. `npm run check:independence` in `public/site` proves clean-source
install/build/tests/package/dev with Cargo, Rust and Python invocations denied.
See [Public development and delivery](../../public/site/docs/pages-deploy.md#independent-public-development).
Production composition retains authenticated immutable Connect artifact admission
and its existing activation/publication gates. This checkpoint does not certify
Connect, Desktop, Agent or the new App Shell as migrated.
