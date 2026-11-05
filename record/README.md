# Record package

`@apparent/record` is an independently installable TypeScript workflow and presentation package. Its local recording controller consumes an injected `EngineClient`; its reusable React pill consumes an injected typed `PillHost`. Neither creates a recording or ships a fake/default production capture adapter.

`EngineSdkAdapter` composes an already constructed, packed `@apparent/engine-sdk` client. `createRecordHostRuntime` from `@apparent/record/host` composes it with `RecordWorkflow`, `SqliteCaptureMappingStore` and `PrivateBundleAllocator` using explicit private roots (Node 22.13+). The runtime reconciles persisted request IDs before admitting starts; active or ambiguous captures remain visible and block admission. The host subpath does not import React or Electron. It does not choose or launch an executable, select runtime locations, or establish capture readiness. The reusable pill is adopted through the existing legacy host; it does not install a replacement Electron main/preload route.

`@apparent/record/processing` exports `decodeSelectedFrames` and
`assembleProcessingOutput`. The decode function consumes an injected public
`@apparent/engine-sdk/media` client and Record-selected bundle-clock timestamps;
the Engine owns decoding, safe output publication and truthful execution facts.
The caller supplies an existing verified media runtime, a fresh private output
directory, source identity and selected frames. Assembly formats those facts as
semantic handoff text with absolute frame/video/bundle paths; it does not select
frames, transcribe, create images or write a bundle. Linux worker conformance is
documented by Engine, but packaged runtime discovery and Record adoption remain
pending the Engine owner's committed artifact and verified discovery/launch/
platform handoff. No native proof is claimed here.

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
