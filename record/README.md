# Record package

`@apparent/record` is an independently installable TypeScript workflow and presentation package. Its local recording controller consumes an injected `EngineClient`; its reusable React pill consumes an injected typed `PillHost`. Neither creates a recording or ships a fake/default production capture adapter.

`EngineSdkAdapter` composes an already constructed, packed `@apparent/engine-sdk` client. `createRecordHostRuntime` from `@apparent/record/host` composes it with `RecordWorkflow`, `SqliteCaptureMappingStore` and `PrivateBundleAllocator` using explicit private roots (Node 22.13+). The runtime reconciles persisted request IDs before admitting starts; active or ambiguous captures remain visible and block admission. The host subpath does not import React or Electron. It does not choose or launch an executable, select runtime locations, or establish capture readiness. The reusable pill is adopted through the existing legacy host; it does not install a replacement Electron main/preload route.

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

The headless source install pins SDK 0.3.1 from the committed
`vendor/engine-sdk-0.3.1.tgz` for development and declares `^0.3.1` as an
optional peer. A host importing the bundle subpath must install that peer. The
verified tarball SHA-256 is
`1b605f06273f9c1a134b68b0ed233605b120813214914ca70ed0c8c5f0346f04`.

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
that preserves the declared surface dimensions and observed normalized points;
projection into a different surface is unsupported. Otherwise
assembly returns an exact unsupported error. This API is fixture-ready only;
it does not decode raw materials, choose frames/marks, produce contact sheets,
or establish native provenance/full-profile acceptance.

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
| Capture lifecycle and durable request identity | Implemented in owner package; not installed | `RecordWorkflow`, `EngineSdkAdapter`, SQLite mapping and private allocator; existing host-runtime tests exercise readiness, active capture, ambiguous replies, stop and restart reconciliation. The provider consumes this runtime. A usable capture SDK/native artifact is still absent from capture-v1. Engine has committed to notify the parent Record thread when a usable capture SDK/native artifact and authoritative capabilities land. |
| Screen picker and typed target | Fixture contract only; production blocked | Version-2 picker outcomes require screen plus display/area/window. No real picker receipt producer exists. Platform owns the native host-effect boundary; its committed same-user `capture.sock` GUI actor extension and shared host-port/client binding must publish the receipt. Platform directly notifies the parent Record thread when the usable committed permission/shortcut port is ready, with exact API/injection and platform evidence. |
| Pen color and persistence | Provider policy implemented; config binding blocked | Record accepts only coral/purple/blue/green and publishes a new effective color only after the injected existing top-level `penColor` writer returns persisted/applied readback. Its initial stored/effective read also comes from that port. The landed config CAS only writes owner-scoped `streamState`; it cannot safely stand in for this app setting. No Record TypeScript binding to the existing pen writer is present yet; until injected, pen is unknown/unavailable. |
| Drawing and accepted stroke | Fixture contract only; production blocked | Typed screen/whiteboard surface and stable stroke ID are required. No actual drawing/stroke receipt producer exists; current capture-v1 has no drawing capability. Engine's established parent-thread notification covers its usable native capture/artifact/capability delivery. |
| Permission grant, usability, relaunch and repair actions | Provider translation seam implemented; Platform producer blocked | Native requirement facts are copied from the injected native port; missing/failed binding yields unavailable. Platform owns the same-user `capture.sock` GUI actor extension and shared public host-port types/client; it has committed direct parent-Record-thread notification with the committed API/injection and platform evidence. |
| Shortcut configuration, registration, edit/save/cancel | Provider translation seam implemented; Platform producer blocked | Effective configured binding and OS registration remain distinct facts. The native producer owns them; this provider neither binds shortcuts nor duplicates the transport. Same Platform notification route as above. |
| Next-shortcut admission and local-only admission | Explicitly unavailable | Current capture-v1 has neither reservation enforcement nor local-only enforcement. Record refuses local-only starts and refuses arm when the producer reports unavailable; it never infers enforcement from intent. A future producer must publish reservation consumption/cancellation/expiry and effective per-capture delivery readback. Engine committed parent-thread notification when usable native capabilities land. |
| Transcript association | Implemented in the headless package | `associateTranscriptWithSelectedFrames` partitions supplied word/segment facts across existing selected frames using measured clause/anchor rules. Source-derived Rust parity tests cover Unicode whitespace, punctuation, gaps and boundary timing. This does not select images or supply missing native visual metrics. |
| Semantic processing assembly and frame decode | Consumer implemented; installed adoption partial | PR #704 (main `7c2afd5`) added the formatter and decoder seam; this slice adds durable job ownership and real PNG decode proof against the retained Linux `media/1` artifact. It does not implement frame selection, canonical bundle writing, default output selection or installed handoff adoption. |
| Capture material to Record bundle assembly and authenticated publication | Pure document/inventory assembly implemented; native publication unavailable | `assembleCanonicalBundle` maps public SDK-validated materials and facts plus explicit Record policy into canonical document bytes and an inventory. Image metadata remains caller-attested, not authenticated native derivatives. It performs no file writes or publication. Capture/2, raw-material decode and native publication remain unavailable; Engine retains its committed usable-capability return. |
| Keyframes, transcript, bundle contents, warnings and default output | Partially implemented; full adoption blocked | Record formats supplied semantic inputs, including target/session context, interaction labels/marks and processing warnings; default keyframe selection, canonical bundle write and installed handoff are still unadopted. Provider maps Engine v1 `saved` to Record `processing`; only authenticated Record output facts may publish `saved` or a successful save outcome. |
| Clipboard copy, link and artifact open | Local prompt policy implemented; native effects and receipts blocked | `formatLocalHandoff` preserves semantic context, warnings and the timed request, with access instructions first and no clipboard/publication effect. Copy/open actions may be dispatched through the native port, but dispatch is not success. Copy needs a clipboard receipt; link needs the existing link producer; artifact open needs the exact-artifact viewer acknowledgement or supported OS observation. No such production receipts are synthesized here. |
| Pill/runtime installed binding | Existing UI and runtime adopted through current fallback | Reusable pill and injected workflow host are packaged. This capability provider is owner-local and does not install an Electron main/preload route or switch routing. |
| Platform proof | Blocked for this provider route | No new OS boundary is claimed. Platform's committed evidence notification must name the native facts/action contract and affected macOS/Linux evidence; existing Desktop platform gates still apply before any route adoption. |
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
