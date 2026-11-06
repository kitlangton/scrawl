# Record package

`@apparent/record` is an independently installable TypeScript workflow and presentation package. Its local recording controller consumes an injected `EngineClient`; its reusable React pill consumes an injected typed `PillHost`. `createWorkflowPillHost` preserves existing workflow callers, while `createRecordClientPillHost` binds the same pill to revisioned `RecordClient` snapshots and commands. Neither creates a recording or ships a fake/default production capture adapter.

`EngineSdkAdapter` composes an already constructed, packed `@apparent/engine-sdk` client. `createRecordHostRuntime` from `@apparent/record/host` composes it with `RecordWorkflow`, `SqliteCaptureMappingStore` and `PrivateBundleAllocator` using explicit private roots (Node 22.13+). The runtime reconciles persisted request IDs before admitting starts; active or ambiguous captures remain visible and block admission. The host subpath does not import React or Electron. It does not choose or launch an executable, select runtime locations, or establish capture readiness. The reusable pill is adopted through the existing legacy host; it does not install a replacement Electron main/preload route.

`@apparent/record/host/materials` exports `readCaptureMaterials`. Inject the
host-authenticated retained manifest, explicit expected material key/lease/bundle
identity, and a separately trusted immutable root. It reads the detached payload
and each declared collected fact (`context`, `interactions`, `candidates`,
`transcript`, `acceptedStrokes`) with finite per-file and aggregate byte limits,
checks sealed identity and SHA-256 before/after bounded `O_NOFOLLOW` reads, and
delegates schema and fact validation to the packed SDK 0.3.2 public capture API.
The result retains the original `payloadBytes` for exact revision binding. Pass
them with `materials.manifest` and `assemblyFacts` to canonical bundle assembly;
never re-encode the parsed payload. Unavailable collection states remain in the
validated materials document.
Video/audio stay sealed references. A successful read proves only that local
bytes matched the supplied seals at read time. It is not native authentication,
availability, media validation or Saved. Correctness across pathname races still
depends on the producer's retained lease, immutable root and same-user/native
exclusion; this is not a portable filesystem authority guarantee. The reader
does not write, publish, release or cancel anything. Importing this subpath
requires the host to install the optional `@apparent/engine-sdk` peer explicitly.

`@apparent/record/processing` exports `decodeSelectedFrames` and
`assembleProcessingOutput`; `@apparent/record/processing/media-consumer` exports
the durable media/1 consumer. A host admits the
Engine artifact through existing delivery contracts and injects a packed SDK
client launched from its explicit absolute worker path and private runtime
directory. Record does not discover or activate a production route. The consumer
has a real Linux native PNG proof using an isolated synthetic completed H.264
bundle and Engine's retained media artifact from source
`463f0053908a9c6030d3c5665aacd10089884aa`; this does not claim capture-v1
readiness, macOS media, or current-main artifact admission. Record retains the
same client and job ID after decode rejection/timeout and requires SDK inspect or
cancel reconciliation before another dispatch or cleanup. Assembly preserves
supplied transcript, selected frames, typed notes/pins, target/device,
session/context changes, interaction labels, marks and processing warnings.
Window titles remain manifest-only. The existing text formatter does not
select frames, write files or perform clipboard/link handoff; those adoption
steps remain outstanding.

`@apparent/record/processing/bundle` exports `assembleCanonicalBundle`. It
accepts detached materials manifest/payload bytes, supplied fact bytes, explicit
persisted `createdAtMs` and canonical target admission facts, Record's selected
frame/mark policy results, and exact sealed artifact bytes. By default it calls
`validateMaterialsDocument` and `validateFactFile` from the public
`@apparent/engine-sdk/capture` subpath; hosts may inject equivalent public SDK
validators. It returns UTF-8 `apparent.json` and `summary.md` bytes, a
deterministic versioned inventory with hashes/lengths and unavailable/empty
collection states, plus the validated original materials/fact documents. It
does not write or publish files, make a Saved claim, copy native materials,
create a final destination, or perform clipboard/network/cleanup effects.

The headless source install pins SDK 0.3.2 from the committed
`vendor/engine-sdk-0.3.2.tgz` for development and declares `^0.3.2` as an
optional peer. A host importing the bundle subpath must install that peer. The
verified tarball SHA-256 is
`3b539f7d978d692a5d7fca9f358047bc197d415f6764c33707cff5a710119d01`.

Assembly requires at least one selected frame, one contact sheet, and exact
sealed media references. Each artifact declares the exact materials key; media
hashes and identities must match the validated material inventory. Generated
documents have no filesystem identity. Image seals remain caller-attested,
not authenticated derivative receipts: inventory provenance distinguishes them
from material references and generated documents. A publisher must still
validate native derivative authority and actual filesystem contents. It refuses stale capture/material/fact identities,
missing or duplicate paths, escaping paths, mismatched bytes, and unordered
frame indices/times. Validated source/invocation facts retain capture identity
in the returned material/fact documents; canonical window snapshots/titles stay
in `apparent.json`, while the existing summary formatter excludes titles.
Context/session changes and all interaction variants are retained. Session
destination fields such as `via` must be supplied as explicit Record admission
metadata; SDK session IDs remain fact references and are not guessed into a
routing destination. Frame reasons
and marks need explicit canonical mappings; Record presentation marks alone do
not contain enough geometry to make a v1 mark. Transcript
segments without `endMs` require an explicit Record estimate policy; this
policy estimates end at the next segment start or capture end and sets
`estimated: true`. Accepted stroke surface transforms remain authoritative:
non-empty strokes need an explicit Record-owned canonical v1 stroke mapping
that preserves the declared surface dimensions and observed normalized points.
Optional sRGB bytes come from each SDK-validated sealed stroke; geometry
mappings cannot supply color. Absent RGB remains absent, distinct from
`[0,0,0]`. Projection into a different surface is unsupported. Otherwise
assembly returns an exact unsupported error. This API is fixture-ready only;
it does not decode raw materials, choose frames/marks, produce contact sheets,
or establish native provenance/full-profile acceptance.

The publication subpath has an optional SDK peer: hosts using it supply the
published SDK. Other Record consumers do not need to install that peer. Source
checks use the pinned `record/vendor/engine-sdk-0.3.2.tgz` package (SHA256
`3b539f7d978d692a5d7fca9f358047bc197d415f6764c33707cff5a710119d01`),
without importing or building an adjacent Engine checkout. Publication remains
separate from the package root.

`@apparent/record/publication` exports `createRecordPublicationRecovery`, a
headless Record-owned SQLite binding and explicit reconnect reconciliation
wrapper for an already constructed `@apparent/engine-sdk/capture` client. The
host supplies the original capture authority and validated immutable materials,
plan bytes and derivative receipts. Record durably binds capture/publication IDs,
exact plan SHA256, materials key and lease ID, bundle ID/path and `apparent.json` SHA256 before
calling SDK publish. Hosts inject `preflightCapturePublication` from the same SDK
as the client; Record runs it before reserving the dispatch and checks native
publication availability before creating a binding. Hosts first validate the
materials with `validateMaterialsDocument(manifest, originalPayloadBytes)`. The
preflight validates inventory and byte bindings, not native authority or filesystem
contents. Record stores control metadata privately and never creates or
replaces the final bundle path. After an ambiguous reply, `reconcile` restores
its expected receipt from that original binding and calls `inspectPublication`;
changed bindings fail closed and a repeated command does not allocate new IDs.
The API does not assemble canonical bundles, release retained materials, or
claim native capture/publication availability. SDK0.3's testing peer remains a
test double and cannot produce native Saved.

`prepareRecordPublication` in the same subpath writes a private plan namespace
from canonical assembly output. It accepts exact source materials bytes and
caller-supplied frame/contact-sheet bytes, records actual filesystem identities
for generated files, and runs the packed SDK preflight before returning a
`CapturePublishInput`. Material references remain references; caller-attested
images stay `assembled` inputs and are not native derivative receipts. This
operation never calls `recovery.publish`, creates the final destination, or
claims Saved. Replays require byte-identical files in the same publication
namespace. The assembly reports its admitted `bundlePath`; staging requires
that exact destination so summary paths stay correct. Staging and destination
roots must not overlap; existing destination ancestors must not be aliases.
Inputs are snapshotted, and coalesced callers receive
independent results. Unmarked populated namespaces are refused without adding
ownership metadata; conflicting or partial files are retained for reconciliation and
never removed implicitly. The returned input remains fixture/packed-consumer
support: it does not establish native authority, filesystem/profile acceptance,
or production capture availability.

`createCompletedCapturePublisher` composes the existing APIs for a host that has
an observed SDK `materials-ready` status and the original materials manifest and
payload bytes. `publishCompletedCapture` checks the observed capture/materials/lease
binding against explicit Record admission before assembly. It assembles the bundle,
checks existing durable publication ownership before staging, then calls the private
stager and recovery publisher. `reconcile` accepts the caller's persisted capture,
publication, materials, lease, bundle, and destination binding and delegates to
recovery without rerunning assembly or staging. Returned values are SDK recovery
statuses; the owner token is never returned. This is a composition seam only:
selected frames/images remain caller-attested, and it does not authenticate
native provenance, supply capture/2 lifecycle, release materials, or prove
production publication.

Binary inputs are fingerprinted by digest and length. Supplied derivative receipts
are forwarded to the existing stager and SDK validation without asserting provenance.
Once a durable binding exists, a new composition refuses before staging; use
`reconcile` with the original binding. A reserved plan that never reached dispatch
can be resumed through the existing recovery publisher with its original prepared
input. Concurrent first-time publishers still rely on the journal's atomic dispatch
admission; losing staging files are private and preserved, never implicitly deleted.

```ts
import { preflightCapturePublication } from "@apparent/engine-sdk/capture";
import { createRecordPublicationRecovery } from "@apparent/record/publication";
const recovery = createRecordPublicationRecovery(client, privateControlRoot, preflightCapturePublication);
await recovery.publish(validatedPublicationInput);
await recovery.reconcile(captureId, ownerToken);
```

The controller accepts `start` only from idle or a terminal state after a fresh readiness query. A capture moves through `starting` → `recording` → `stopping` → `processing` → `ready`; a pending or active capture can be cancelled, and a failed stop can be retried with its original ID. Session IDs scope every event, and out-of-order phase regressions are ignored.

The interface follows the existing GUI capture control contract: a caller-owned request ID for start, real-frame readiness distinguishing `starting` from `recording`, and ID-scoped stop/cancel/inspect receipts. Stop transport failures retain the same capture ID for retry. The Engine retains capture safety, permission checks, native media execution, finalization and bundle truth. `EngineClient.readiness()` is a future capability for querying actual native admission readiness; the existing CLI does not expose that as a separate query yet, so an adapter cannot infer it from an idle capture state.

## Development

Run `npm ci`, `npm run typecheck`, `npm test`, and `npm run build` in this directory. These commands use TypeScript and Node only; they do not use Cargo or import Desktop code.

## Adoption limits

The repository's `apparent capture` CLI already implements GUI start/stop/cancel/inspect control over a same-user Unix socket on macOS/Linux; a separate readiness query is not currently exposed. The native delivery package now builds and stages an explicit `apparent-engine` executable wrapper, but its provider reports `screenCapture: false` and `readiness: "unavailable"`, and cannot perform real capture. Thus the wrapper is not production readiness or native capture evidence. A production host still needs to select and launch the executable with a private runtime directory, wire an accepted readiness/capture capability into `EngineSdkPort`, and complete affected platform seam proof before adopting this runtime as a route. The runtime is not an installed route, and this package makes no production recording-availability claim.

## Asynchronous ownership

Start reserves its request synchronously in `checking`, before querying readiness.
Once submitted, that ID cannot be reused within the controller. Only one mutation
may be in flight; competing commands are refused by returning the current state.
During a pending start RPC, Cancel is refused until the bounded adapter call
settles; an admitted `starting` capture can then be cancelled. Adapters must bound
all calls. A timeout never proves that a native capture ended.

Ambiguous Start/Cancel replies retain the ID in `uncertain`; `reconcile` inspects
that same capture before allowing another start. Stop transport failure retains
`stop-failed` and supports inspection or an explicit same-ID retry. Matching Engine
events supersede pending replies, including failures; phase regressions are
ignored. Disposal detaches observation and suppresses pending state updates; it
is not a capture-stop operation. The host must resolve active capture ownership
before disposing a controller or quitting.

The adapter must translate provider facts: `failed`/`rejected` are terminal only
when the Engine confirms that this request owns no live capture. Transport errors
must reject the call rather than manufacture terminal receipts. The current
consumer interface is not a newly published Engine protocol or native proof.

State subscribers are observers, not lifecycle participants. A subscriber that
throws is detached; remaining subscribers still receive the state. Its exception
cannot abort Engine event processing or strand command ownership. Hosts should
handle presentation errors within their own observer and may resubscribe after
repair. No capture is cancelled as a side effect of an observer failure.

## Pill presentation adoption

`src/ui` owns the existing Desktop recording pill presentation and CSS. The `@apparent/record/ui` export exposes `RecordingPill`, `PillView`, `PocketView` and the injected `PillHost` API; it has no window globals or app mount assumptions. The Record build places the CSS and Host Grotesk font beside the emitted component. The Desktop renderer imports the built package through its file dependency. Its thin `src/pill` legacy adapter injects the existing privileged Electron preload API, mounts the component, and applies the transparent window reset.

The existing Electron `electron/pill` host and Rust `ui/electron_pill` lifecycle/transport remain authoritative. Installed routing stays on its existing fallback-aware path; this adoption does not enable or switch a native route, add config, or claim capture capability. Rollback restores the previous frontend package dependency and pill component mount. Remove that adapter and its legacy consumers only when a separately accepted host migration retires the current runtime, and remove the old native pill only after the recorded native acceptance gates in `capture/desktop/ui/frontend/README.md` pass.

The public UI artifact bundles shared controls and icons; only React/React DOM
remain host peers. Import `@apparent/record/ui/style.css` with `RecordingPill`.
Its stylesheet is precompiled and ships its font: packed consumers need neither
shared source nor Tailwind tooling. Shared UI is a development/build input only.
`npm run test:package` installs a tarball in an isolated temporary consumer and
exercises root, capabilities, fixtures and server-rendered UI imports. This
explicit artifact check may install React peers; hooks do not install dependencies.
Source builds require the repository's `shared/ui` and shared font inputs, but
never Rust. Replacing an injected PillHost remounts its presentation session,
clearing old revisions, pending actions and subscriptions before new interaction.

`@apparent/record/ui` also exports `createWorkflowPillHost` and
`disposeWorkflowPillHost` as an injection seam for a constructed `RecordWorkflow`.
The adapter presents workflow snapshots to the existing `PillHost` contract with
host-local monotonic revisions.
Each read and observer receives an isolated view; host clock failures use an empty
clock and cannot detach authoritative updates. Optional selecting-phase `status`
text makes readiness/checking/starting states visible without target controls.
It exposes readiness refresh, start, stop, cancel, reconcile and same-ID stop retry
only in states where the workflow admits them;
it does not provide paste, link, notes, selection, whiteboard, window or clipboard
behavior. Disposal unsubscribes the presentation adapter and leaves workflow and
capture ownership intact. This seam is not an installed route. Activation still
requires a real native readiness/capture provider, composition of the supplied
durable mapping store and allocator, executable/runtime composition, and affected macOS/Linux capture,
permission, packaging and ownership proof. Until those gates pass, the installed
fallback route remains authoritative.

## Full Record scope inventory

| Agreed behavior | State | Evidence / exact remaining producer |
|---|---|---|
| Capture lifecycle and durable request identity | V1 retained; capture/2 consumer implemented, not installed | The existing `RecordWorkflow`/`EngineSdkAdapter` V1 path and callers remain unchanged. `@apparent/record/host/capture2` injects the published SDK 0.3.2 `CaptureClient`, uses durable `AbsentDestinationAllocator` bindings and the SQLite mapping store, and offers `Capture2StartIntent.destinationPath` for exact requested output. Exact paths require a normalized absolute path and an existing non-aliased parent; Record does not create final paths or change parent permissions. The immutable start intent includes the path, mapping persistence precedes SDK dispatch, and restart reconciles the original SDK bundle identity without replacement starts. Generated destinations and v1 callers remain compatible. Other coverage includes readiness/discovery, exact advertised source, public request-ID controls, stop/cancel, unknown/restart inspection, and retained materials references. Start inputs are snapshotted before asynchronous work. Cold controls handshake without requiring new-capture readiness; inspection cannot block Stop/Cancel. Unknown outcomes retain the last observed phase and materials lease; this host never releases materials. Disposal prevents later dispatch and detaches without stopping or closing the SDK. The SQLite store atomically compares the complete inspected capture/2 registry before insertion, preventing two hosts sharing that store from both admitting different starts from a stale scan. The native producer enforces global admission across all clients and private roots. A persisted intent whose start was never dispatched remains unresolved until authoritative inspection; this consumer never retries start under a replacement identity. Scripted transport tests remain synthetic. The opt-in `npm run test:package:native:capture2` packed-consumer qualification runs the supplied exact Linux capture/2 artifact on owned synthetic Xvfb, checks display/center-half-area discovery, durable start/stop/materials, same-client host recreation, cancellation and decoded movie behavior; this does not restart the SDK process. This is Linux Xvfb consumer evidence only; it does not establish other OS capabilities, installed routing, or local-only admission enforcement. V1 callers remain unchanged. |
| Screen picker and typed target | Fixture contract only; production blocked | Version-2 picker outcomes require screen plus display/area/window. No real picker receipt producer exists. Platform owns the native host-effect boundary; its committed same-user `capture.sock` GUI actor extension and shared host-port/client binding must publish the receipt. Platform directly notifies the parent Record thread when the usable committed permission/shortcut port is ready, with exact API/injection and platform evidence. |
| Pen color and persistence | Provider policy implemented; Record orchestration/injection pending | #779 made the authoritative fixed-key `penColor` config source and artifacts available. #781 supplies a native config-lock/callback guard primitive only; the public GUI RGB apply/readback port is still absent. Record orchestration/injection remains blocked on that producer. Pen stays unknown/unavailable: neither CAS, a guarded callback primitive nor configured color proves an ink effect. |
| Drawing and accepted stroke | Fixture contract only; production blocked | Typed screen/whiteboard surface and stable stroke ID are required. No actual drawing/stroke receipt producer exists; current capture-v1 has no drawing capability. Engine's established parent-thread notification covers its usable native capture/artifact/capability delivery. |
| Permission grant, usability, relaunch and repair actions | Production Record adapter implemented; installed routing/Preview adoption unavailable | `@apparent/record/host/native` consumes injected `NativeHostPort`; trusted composition supplies required permission IDs. Status, usability, relaunch and allowed actions remain independent; missing required facts are unavailable. Facts are cached observations scoped to the running GUI process, not fresh TCC or Engine readiness. Platform #737's exact-main Linux/Mac seam evidence is reused; no new OS boundary or installed route is claimed. |
| Shortcut configuration, registration, edit/save/cancel | Production Record adapter implemented; installed routing/Preview adoption unavailable | Adapter preserves configured/effective labels and registration separately. Record owns edit sessions, local shortcut text decoding, validation candidate ownership, save/cancel and refusals; only the native returned candidate can be saved. Save effects require the accepted snapshot's exact epoch/revision and readback remains producer-owned. Paste, defaults/reset, and unverified/external registrations are not mutated or inferred. |
| Next-shortcut admission and local-only admission | Explicitly unavailable | Current capture-v1 has neither reservation enforcement nor local-only enforcement. Record refuses local-only starts and refuses arm when the producer reports unavailable; it never infers enforcement from intent. A future producer must publish reservation consumption/cancellation/expiry and effective per-capture delivery readback. Engine committed parent-thread notification when usable native capabilities land. |
| Transcript association | Implemented in the headless package | `associateTranscriptWithSelectedFrames` partitions supplied word/segment facts across existing selected frames using measured clause/anchor rules. Source-derived Rust parity tests cover Unicode whitespace, punctuation, gaps and boundary timing. This does not select images or supply missing native visual metrics. |
| Semantic processing assembly and frame decode | Consumer implemented; installed adoption partial | PR #704 (main `7c2afd5`) added the formatter and decoder seam; this slice adds durable job ownership and real PNG decode proof against the retained Linux `media/1` artifact. It does not implement frame selection, native final publication, default output selection or installed handoff adoption. |
| Capture/2 retained payload and fact bytes | Headless read-only consumer implemented; production producer availability unavailable | `@apparent/record/host/materials` reads an already host-authenticated retained manifest from an explicitly trusted immutable root, verifies bounded payload/fact contents with SDK 0.3.2 public validators and preserves unavailable observations. It feeds validated payload/fact bytes to existing assembly inputs. It does not authenticate native authority, decode video/audio, select frames, publish or claim Saved. A production capture/2 retained-material producer and its platform evidence are still unavailable; fixture and file-reader readiness do not establish native availability. |
| Capture material to Record bundle assembly and authenticated publication | Assembly/publication composition implemented; native authority and installed adoption unavailable | `createCompletedCapturePublisher` validates an observed materials-ready status against original SDK-validated materials and explicit admission, then composes `assembleCanonicalBundle`, `prepareRecordPublication`, and durable recovery. Reconnect uses the same caller-persisted capture/owner binding without rerunning assembly/staging. The new capture/2 host slice supplies lifecycle materials references but does not read material bytes or invoke this publisher. Selected frames/images remain caller-attested. Native media2 lifecycle, selection, publication authority, final filesystem acceptance, material release, and installed adoption remain open. |
| Keyframes, transcript, bundle contents, warnings and default output | Partially implemented; full adoption blocked | Record formats supplied semantic inputs, including target/session context, interaction labels/marks and processing warnings; default keyframe selection, native final bundle publication and installed handoff are still unadopted. Provider maps Engine v1 `saved` to Record `processing`; only authenticated Record output facts may publish `saved` or a successful save outcome. |
| Clipboard copy, link and artifact open | Local prompt policy implemented; native effects and receipts blocked | `formatLocalHandoff` preserves semantic context, warnings and the timed request, with access instructions first and no clipboard/publication effect. Copy/open actions may be dispatched through the native port, but dispatch is not success. Copy needs a clipboard receipt; link needs the existing link producer; artifact open needs the exact-artifact viewer acknowledgement or supported OS observation. No such production receipts are synthesized here. |
| Pill/runtime installed binding | Existing UI and runtime adopted through current fallback | Reusable pill and injected workflow host are packaged. This capability provider is owner-local and does not install an Electron main/preload route or switch routing. |
| Platform proof | Existing producer seam evidence reused; installed-route gates remain | Platform #737 merged at main `aa8f0cc7fdf462b1b6056b3303f6e5db6eda206a`; its frozen producer source and stated Linux isolated Omarchy and Mac installed seam evidence apply to the unchanged native transport. This Record slice adds no OS boundary or installed route. |
| Windows | Excluded | Apparent supports macOS and Linux only. |

The provider is available from `@apparent/record/capabilities` as
`createRecordCapabilityProvider`. A host constructs the published Engine-backed
runtime and injects it, plus the narrow Record-local native/config seams:

```ts
const runtime = await createRecordHostRuntime(engineSdk, privateRoots);
const client = createRecordCapabilityProvider({
  runtime,
  providerEpoch: crypto.randomUUID(),
  native: platformRecordPort, // absent until Platform's committed binding lands
  captureFacts: authenticatedRecordOutputPort, // absent until Engine's assembly/publish channel lands
  penConfig: authoritativePenConfigPort, // existing top-level penColor writer/readback binding
});
```

Engine facts come only from runtime readiness and Engine receipts. Native
permission/shortcut facts and native actions come only from the Platform port;
pen preference comes only from the authoritative config read/write/apply
readback. Picker/stroke, delivery, shortcut admission, copy, and artifact-open
facts pass through only as actual producer receipts. Missing ports report
unavailable; command acceptance reports dispatch only. `detach()` drops
observation and disposes the owner runtime adapter without issuing capture stop
or cancel. The current runtime does not expose persisted terminal history as
complete capability snapshots, so this provider retains facts observed during
its lifetime and reports only unresolved startup ownership it can actually
inspect; it does not reconstruct omitted history or claim an artifact identity.
The Engine SDK v1 `saved` receipt is only material completion: Record keeps that
capture in `processing` until the authenticated output port confirms Record's
assembled artifact. No v1 material receipt can publish Record `saved`.
