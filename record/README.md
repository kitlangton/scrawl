# Record package

`@apparent/record` is an independently installable TypeScript workflow and presentation package. Its local recording controller consumes an injected `EngineClient`; its reusable React pill consumes an injected typed `PillHost`. `createWorkflowPillHost` preserves existing workflow callers, while `createRecordClientPillHost` binds the same pill to revisioned `RecordClient` snapshots and commands. Neither creates a recording or ships a fake/default production capture adapter.

`EngineSdkAdapter` composes an already constructed, packed `@apparent/engine-sdk` client. `createRecordHostRuntime` from `@apparent/record/host` composes it with `RecordWorkflow`, `SqliteCaptureMappingStore` and `PrivateBundleAllocator` using explicit private roots (Node 22.13+). The runtime reconciles persisted request IDs before admitting starts; active or ambiguous captures remain visible and block admission. The host subpath does not import React or Electron. It does not choose or launch an executable, select runtime locations, or establish capture readiness. The Record workflow and pill-host adapters are implemented and packaged; `createRecordClientPillHost` and `RecordClientPillHostOptions` are public exports, while installed native-provider routing and platform adoption remain separate gates.

`@apparent/record/host/materials` exports `readCaptureMaterials`. Inject the
host-authenticated retained manifest, explicit expected material key/lease/bundle
identity, and a separately trusted immutable root. It reads the detached payload
and each declared collected fact (`context`, `interactions`, `candidates`,
`transcript`, `acceptedStrokes`) with finite per-file and aggregate byte limits,
checks sealed identity and SHA-256 before/after bounded `O_NOFOLLOW` reads, and
delegates schema and fact validation to the packed SDK 0.4.0 public capture API.
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
readiness or current-main artifact admission. A separate completed-bundle media/1 consumer passed on macOS 26.6.2 using the admitted x64 worker under Rosetta on Apple silicon; that is not Intel CPU, macOS 15, or native ARM proof. Record retains the
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

The headless source install pins SDK 0.4.2 from the committed
`vendor/engine-sdk-0.4.2.tgz` for development and declares `^0.4.2` as an
optional peer. A host importing the bundle subpath must install that peer. The
verified tarball SHA-256 is
`62b1b2e103a585dbf10b848c9538ddcf888b3643b85d7eb89db8c1a61647066c`.

The SDK `/observations` subpath exposes `readFrameVisualObservations` and its
public input/result/source types. Reader callers supply the original validated
materials, completed derivative/2 document, 1-based request index, exact sealed
PNG bytes, and explicit byte budget. The typed result is either unavailable
(profile absent or unsupported) or measured against the decoded derivative
RGBA8 plane. These offline consistency measurements do not authenticate native
issuance or establish selection, candidate collection, Saved, or publication.

Assembly requires at least one selected frame, one contact sheet, and exact
sealed media references. Hosts may supply video/audio as `materialReferences`
instead of reading media bytes into JavaScript; Record derives each media
inventory hash, length, and identity only from the SDK-validated original
materials payload. References must name the declared video/audio files, and
cannot overlap byte-backed artifacts. Existing byte-backed media callers keep
their behavior. Each artifact declares the exact materials key; media hashes
and identities must match the validated material inventory. Generated
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

`@apparent/record/processing/media2-consumer` exports `RecordMedia2Job` over the published SDK 0.4.2 client. It durably binds the original retained-material request, layout and hash before dispatch; ambiguous jobs inspect the same identity, and cancellation remains explicit.

The retained packed Linux Xvfb qualification used SDK 0.4.1 and the admitted `0570f775` native Media/2 worker with retained Capture/2 materials, then read its sealed frames/contact sheet into canonical assembly and a prepared staging plan. SDK 0.4.2's capture, media/1 and media/2 core client modules are byte-identical, so that interface evidence is reused; this dependency upgrade did not run a new native Record observation/selection qualification. Selection times/layout in that proof were explicit caller input; validated derivatives were processing inputs, never Saved. That earlier producer had no publication capability. The later authenticated publication continuation is tracked below; the staged-plan proof alone does not authenticate a published bundle or establish copy.

The publication subpath has an optional SDK peer: hosts using it supply the
published SDK. Other Record consumers do not need to install that peer. Source
checks use the pinned `record/vendor/engine-sdk-0.4.2.tgz` package (SHA256
`62b1b2e103a585dbf10b848c9538ddcf888b3643b85d7eb89db8c1a61647066c`),
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
New reserved publications retain the exact preflighted dispatch metadata without
`ownerToken`. Reconciliation can claim a still-undispatched reservation once through
the existing dispatch CAS; an ambiguous dispatched publication remains inspect-only.
Record limits each recovery descriptor to 2 MiB of serialized JSON and all retained
recovery descriptors in one control store to 16 MiB, including terminal rows.
These are Record operational limits, smaller than SDK document maxima. Admission
checks the aggregate under the SQLite write lock; capacity exhaustion refuses a
new reservation before dispatch and never evicts history or changes the bundle.
This bounds recovery metadata, not the entire SQLite database or filesystem.
The API does not assemble canonical bundles, release retained materials, or
claim native capture/publication availability. SDK0.4's testing peer remains a
test double and cannot produce native Saved.

SDK 0.4.0's packed `/media2` offline validators support additive derivative/2
staging. Supply the original prepared request/layout and detached receipt bytes in
`derivatives2`, then map every final frame/contact-sheet inventory path in
`derivative2Artifacts` to a receipt ID and frame request slot or contact-sheet
selector. The stager checks the document against original materials, lease, job,
request/layout, and exact assembled role/hash/length/file identity before it
creates staging state. Plans retain native receipt seals and selectors while
publication paths remain final bundle paths. Replays bind document bytes and
selector maps into their fingerprint and marker. Fixtures establish only offline
consistency; the native publisher must authenticate registry origin. SDK 0.4.0
provides no media/2 transport or native job authority, so this consumer does not
claim native processing, authenticated Saved, or copy acceptance.

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

`createRecordPublicationOutput` is the durable Saved-to-handoff composition for
hosts that also need public request facts and exact local handoff resolution.
It stores the immutable public request ID, video-proof preference, and
assembler-authored processing value in the existing publication binding before
dispatch. Only a native Saved receipt matching that binding produces a `saved`
fact or resolves the same request/artifact pair after restart; delivery remains
`unknown`. Older bindings without this output context stay unavailable. This
does not add clipboard effects or infer native origin from local materials.

`@apparent/record/host/capture2-coordinator` exports
`createRecordCaptureCoordinator` for hosts composing the existing capture/2
runtime, retained media/2 job, materials and sealed-artifact readers, canonical
assembler, and publication output. The host supplies trusted roots and one
explicit processing-selection callback; its decisions and stable identities are
stored in the existing private publication SQLite database before media dispatch.
After restart it inspects the original media job and publication binding, then
exposes the same authenticated Saved facts and exact handoff resolver. It does
not choose default frames/layout, close caller-owned clients, release materials,
copy to the clipboard, or infer local-only delivery. Synthetic method-port and
packed SDK transport coverage prove the offline composition only; native capture/media/publication
and installed-provider adoption remain separate evidence gates. Automatic failures
are observable through `processingIssues()` and `observeProcessingIssues`, and
appear as sanitized processing outcomes in `captureFacts`. Failed and unknown
jobs retain their original journals and are never redispatched by inspection.
Issue observations are reconstructed during startup; Saved facts still have only
the publication store as authority. Await `dispose()` before deleting owned private
control data; it drains coordinator work without cancelling or closing SDK clients.
Hosts first close their own UI/provider command admission and drain commands to injected native ports.
They may then call the coordinator’s `shutdown()`: it closes coordinator
admission, drains already admitted calls, and returns native/durable ownership
items separately from local drain completion. Pending or unknown items are not
idle proof; this coordinator does not produce next-shortcut admission facts.
The report covers `scope: "record-coordinator"`; it cannot clear other users of a shared SDK/runtime.
Await `shutdown()` and then `dispose()` before closing caller-owned SDK clients, and
require any other SDK owners to clear their own work. Background capture observation
is drained even when the final mapping read fails. A reserved publication discovered
after shutdown admission closes remains undispatched for recovery on the next owner.

`createStandardRecordCaptureCoordinator` from the same export supplies Record's
bounded visual-only processing policy instead of a caller frame/layout callback:

```ts
const flow = await createStandardRecordCaptureCoordinator({
  ...hostOptions, // same admitted capture/media clients, stores, source/consent and trusted root ports
  standardProcessing: { requestVideoProof: true },
});
// flow.createProvider(...), flow.captureFacts and flow.resolveHandoff use the existing Saved authority.
```

It persists original sampling jobs before dispatch, reads their exact sealed PNGs
through SDK 0.4.2 observations, then freezes one final selection in the same
publication database before deriving the publication images and sheet. Restart
reuses those jobs and decisions. Missing observation profiles are observable
unavailable outcomes; uncertain jobs inspect only their original request. The
standard path requires interaction, transcript and accepted-stroke collections to
be unavailable; collected rich facts need the explicit policy path rather than
being silently discarded. Context and collection availability remain in the bundle.
Source selection, permission/consent, trusted runtime roots and client ownership
remain host inputs. `createdAtMs` defaults to the original invocation's
`observedAtMs`; trusted composition can supply it explicitly, along with optional
destination metadata. No native candidate collection is claimed.

Sampling uses the donor 220 ms interval and 640-pixel edge, bounded to 480 samples
in jobs of at most 48. Longer recordings use a coarser interval with a warning.
Selection uses the visual transition/settling, novelty, spacing and duration
budget policy. Final whole-frame images have a 24-million-pixel aggregate bound;
the integer aspect-preserving sheet has at most three columns and no rasterized
captions. Downscaling is warned and the original sealed video is retained. These
bounds and layout are not unrestricted legacy rendering parity. Shutdown accounts
for every sampling job and the final media/publication job without dispatching new
work. Synthetic packed-client recovery tests establish this composition; a native
standard-selector capture-to-copy run and rendered/installed adoption remain unrun.
The earlier same-recording explicit-selection proof is unchanged.

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

The existing Electron `electron/pill` host and Rust `ui/electron_pill` lifecycle/transport remain authoritative for installed routing. The public Record client can now bind the pill host to a constructed workflow and runtime, but that package-level integration does not install or switch the native-provider route, establish native capture readiness, or replace the existing fallback-aware route. Rollback restores the previous frontend package dependency and pill component mount. Remove the legacy adapter and its consumers only when the installed host migration is accepted, and remove the old native pill only after the recorded native acceptance gates in `capture/desktop/ui/frontend/README.md` pass.

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
behavior. The public `createRecordClientPillHost` composes the pill presentation
with the published Record client/runtime contract. Disposal unsubscribes the
presentation adapter and leaves workflow and capture ownership intact. The
package-level path is implemented, but not an installed native route. Remaining
adoption requires a real native readiness/capture provider, Platform's installed
host binding, runtime/executable composition, and affected macOS/Linux capture,
permission, packaging and ownership proof. Until those gates pass, the existing
fallback route remains authoritative.

## Full Record scope inventory

| Agreed behavior | State | Evidence / exact remaining producer |
|---|---|---|
| Capture lifecycle and durable request identity | Capture/2 consumer/runtime and bounded Linux producer implemented; V1 separately unavailable | The existing `RecordWorkflow`/`EngineSdkAdapter` V1 path and callers remain unchanged; V1 still has no supported native capture capability. `@apparent/record/host/capture2` injects the published SDK 0.4.1 `CaptureClient`, uses durable `AbsentDestinationAllocator` bindings and the SQLite mapping store, and supports exact requested destinations under the documented parent/alias constraints. The immutable start intent is persisted before native dispatch; restart reconciles the original identity and never retries under a replacement. `createCapture2RecordRuntime` bridges the host into Record's runtime seam, but is not installed routing. Linux X11 producer support is bounded to explicit-display/advertised-half-area start, control, retained materials and release. The opt-in packed-consumer test is authored to exercise source discovery, durable start/stop/materials, host recreation, cancellation and decoded output on owned synthetic Xvfb; running it establishes consumer evidence, not installed renderer/permission proof or cross-platform acceptance. The SDK `ReferenceCapturePeer` remains unavailable. Mac, Wayland and installed-route evidence remain absent; V1 is separate and unavailable. |
| Screen picker and typed target | Fixture contract only; production blocked | Version-2 picker outcomes require screen plus display/area/window. No real picker receipt producer exists. Platform owns the native host-effect boundary; its committed same-user `capture.sock` GUI actor extension and shared host-port/client binding must publish the receipt. Platform directly notifies the parent Record thread when the usable committed permission/shortcut port is ready, with exact API/injection and platform evidence. |
| Pen color and persistence | Config/effect producers and Record adapter implemented; installed injection/activation pending | Published `@apparent/engine-sdk` 0.4.0 capture APIs, `@apparent/native-host/pen`, the Record palette RGB mapping, and Record's `createRecordPenConfigPort` adapter are package-level seams. Platform #779 supplies the authoritative `penColor` config seam; #786 supplies the pen effect producer, and #799 implements Record read/CAS/guarded-apply/getter adaptation. These producer seams are source-verified, but installed Platform composition and activation are not established. A configured value or dispatch alone is not effective ink: Record publishes only confirmed readback; without the installed composition it remains unknown/unavailable. |
| Drawing and accepted stroke | Fixture contract only; production blocked | Typed screen/whiteboard surface and stable stroke ID are required. No actual drawing/stroke receipt producer exists; current capture-v1 has no drawing capability. Engine's established parent-thread notification covers its usable native capture/artifact/capability delivery. |
| Permission grant, usability, relaunch and repair actions | Production Record adapter implemented; installed routing/Preview adoption unavailable | `@apparent/record/host/native` consumes injected `NativeHostPort`; trusted composition supplies required permission IDs. Status, usability, relaunch and allowed actions remain independent; missing required facts are unavailable. Facts are cached observations scoped to the running GUI process, not fresh TCC or Engine readiness. Platform #737's exact-main Linux/Mac seam evidence is reused; no new OS boundary or installed route is claimed. |
| Shortcut configuration, registration, edit/save/cancel | Production Record adapter implemented; installed routing/Preview adoption unavailable | Adapter preserves configured/effective labels and registration separately. Record owns edit sessions, local shortcut text decoding, validation candidate ownership, save/cancel and refusals; only the native returned candidate can be saved. Save effects require the accepted snapshot's exact epoch/revision and readback remains producer-owned. Paste, defaults/reset, and unverified/external registrations are not mutated or inferred. |
| Scoped next-shortcut admission | Record refusal policy implemented; native reservation producer unavailable | Current installed capture-v1 does not expose reservation enforcement. Record refuses to arm when the producer reports unavailable. The producer must publish reservation consumption, cancellation or expiry before an explicit Start can proceed; the admitted capture/2 start/control path does not implement this next-shortcut reservation contract. |
| Local-only admission | Capture/2 request contract requires `localOnly: true`; whole Record pipeline enforcement unavailable | A local-only request bit and the separate `nativeRemoteDelivery` prohibition do not establish that the complete capture, processing, publication and handoff pipeline is local-only. Installed V1 lacks effective per-capture delivery readback, and the bounded capture/2 producer evidence is not whole-pipeline or installed-route proof. Record must keep unsupported local-only admission unavailable until a producer reports effective enforcement for the requested capture. |
| Transcript association | Implemented in the headless package | `associateTranscriptWithSelectedFrames` partitions supplied word/segment facts across existing selected frames using measured clause/anchor rules. Source-derived Rust parity tests cover Unicode whitespace, punctuation, gaps and boundary timing. This does not select images or supply missing native visual metrics. |
| Semantic processing assembly and frame decode | Media/1, retained Media/2 and bounded standard visual policy implemented; native standard-path qualification pending | Record consumes completed-bundle media/1 and retained-material media/2 through the published SDK 0.4.1. The admitted Linux paired capture/media artifacts from source `0570f775` passed the packed consumer processing path on owned Xvfb. Completed-bundle media/1 also passed on macOS 26.6.2 using an x64 worker through Rosetta, not an Intel-CPU or Mac capture/media2 claim. `RecordMedia2Job` retains original request/material/lease identities and inspects uncertain work without redispatch. Admitted SDK 0.4.2 and native Media/2 PNG observations supply bounded visual inputs. The standard coordinator owns sampling and visual-only selection; richer semantic selection and native standard-path acceptance remain separate gates. |
| Capture/2 retained payload and fact bytes | Reader implemented; admitted Linux X11 producer path consumed by Record on synthetic Xvfb | SDK 0.4.1 public validators and `@apparent/record/host/materials` provide the packed consumer. Engine #788/#794 source (`5ed32afd`) supplies the earlier bounded capture/2 producer; the newly admitted paired Capture/2 and Media/2 artifacts from source `0570f775` were consumed through the packed Record host and retained-material reader. Record then passed the original retained video to media/2 without copying it into JavaScript staging. This is a synthetic Xvfb consumer qualification, not installed Record routing or cross-platform proof. Mac, Wayland, audio, picker, drawing, shortcut admission, candidate selection and general-purpose frame policy remain unavailable or unproven. |
| Capture material to Record bundle assembly and authenticated publication | Automatic public coordinator with explicit and standard visual policies implemented; installed adoption pending | `createRecordCaptureCoordinator` composes the existing capture2 runtime, retained media job/readers, canonical assembler, stager, recovery and Saved output. Its immutable processing plan is stored in the existing private publication database before media dispatch. Engine publication source `d60dd82b` provides the admitted Linux native publisher. The original retained recording reached authenticated Saved and cold inspection of its original publication tuple, followed by one confirmed native copy through the public provider/PillHost API. That proof used explicit host composition; the new automatic coordinator has synthetic packed-client/identity/recovery checks, not a fresh native capture/copy run. Saved context and exact handoff resolution survive restart through `createRecordPublicationOutput`; raw materials-ready remains processing. |
| Keyframes, transcript, bundle contents, warnings and default output | Canonical assembly, durable output and bounded visual defaults implemented; native standard-path and installed adoption pending | Record preserves supplied semantic context, interaction/mark facts, warnings and sealed media references. The coordinator persists explicit decisions or standard visual sampling and final frame/layout decisions; missing measurements remain unavailable. Only an authenticated native publication receipt matching the original Record binding produces Saved output and its handoff context. The visual-only standard path consumes admitted Engine PNG observations. Collected semantic/mark selection and native standard-path acceptance remain open. |
| Clipboard copy, link and artifact open | Exact-text clipboard adapter, provider receipt mapping and pill action implemented; installed adoption and other receipts remain gated | Platform #833 supplies the privileged clipboard write/readback port. `createRecordLocalClipboardPort` from `@apparent/record/host/native` formats the full handoff resolved for the exact authenticated Saved capture/artifact tuple, preflights Unicode and the injected producer byte limit, reads fresh native admission and performs one write. Only an exact digest/UTF-8-length readback becomes a `copy` outcome with `evidence: "native-write-readback"`, artifact and command IDs; dispatch acceptance, refusal and unknown never become copy success. The public client pill offers Copy handoff for a saved artifact and presents unknown/refusal without retry. Native producer Linux X11/Mac seam evidence is separate from this injected consumer and installed host assembly. The resolver does not authenticate publication; the public automatic coordinator and durable resolver now compose that output path; installed host adoption remains a separate gate. Link needs its existing producer; artifact open still needs exact-artifact viewer acknowledgement or supported OS observation. |
| Pill/runtime installed binding | Runtime and public pill client composition implemented; native installed routing pending | Runtime #803 (`6f62dcb6`) and public `RecordClientPillHost` #807 are merged. The public pill API can compose the client/runtime with the provider; this is package-level integration, not installed renderer/native permissions or proof of full output. The existing fallback-aware route remains authoritative pending platform composition and affected native proof. |
| Independent packaging and builds | Record TypeScript package and public exports implemented; end-to-end installed adoption incomplete | `record/package.json` exposes the headless host, capture/2 runtime, native host adapter, processing, publication and UI package entrypoints. The Record package build and packed-consumer checks use TypeScript/Node and do not invoke Cargo; SDK 0.4.2 is a published optional peer and is explicitly installed by relevant packed consumers. These package contracts do not establish a production executable selection, installed route, release or complete Desktop/Engine delivery. |
| Platform proof | Bounded Linux capture→media2→Saved→native copy passed through public APIs; automatic and installed-route limits remain | The original synthetic Xvfb recording used admitted `0570f775` capture/media closures, the admitted Linux `d60dd82b` publisher and the source-identified Platform X11 clipboard GUI. Its full 62-frame H.264 decode, selected images/sheet and final seven-file inventory were inspected. Authenticated Saved and cold tuple reconciliation preceded one exact-text native write/readback; the public provider/PillHost API reported it. No rendered React/Electron click, installed routing, paste or viewer-open proof is implied. The automatic coordinator is separately tested with synthetic ports and the packed Media2Client. No capture/copy was repeated for that code slice. Wayland/Mac capture, Mac media2/publication consumer proof, audio and full-video control-surface exclusion remain outside this evidence. |
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
  clipboard: recordLocalClipboardPort, // privileged exact-text adapter over the published clipboard port
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

The local clipboard adapter is injected separately from permission/shortcut actions:

```ts
const clipboard = createRecordLocalClipboardPort({
  clipboard: createNativeClipboardPort(), // privileged Platform facade
  nativeHost: createNativeHostPort(),
  maximumTextBytes: MAX_UTF8_TEXT_BYTES, // exported by the active producer
  resolveHandoff: trustedSavedHandoffResolver,
});
const client = createRecordCapabilityProvider({ ...ports, clipboard });
const pill = createRecordClientPillHost(client, { createRequestId });
```

Only trusted composition receives native ports and the Saved resolver; the renderer
receives `RecordClient`. The resolver must return the exact capture/artifact pair
and matching `LocalHandoffInput` from authenticated publication and semantic facts.
The adapter checks that correlation and copies the existing complete formatter
output without truncation, splitting, normalization or an inspect-only substitute.
Invalid text and over-limit output refuse before any write. Native timeout, lost
reply or mismatched digest/length remains unknown and is never retried or rolled
back. Wayland's inability to confirm ownership remains unknown. The receipt
contains a digest/length, never ambient clipboard contents or a private bundle
path. A copy receipt does not establish paste, delivery or artifact-open success.
Disposing the pill unsubscribes its view; native transports remain caller-owned.
