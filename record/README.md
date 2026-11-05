# Record package

`@apparent/record` is an independently installable TypeScript workflow and presentation package. Its local recording controller consumes an injected `EngineClient`; its reusable React pill consumes an injected typed `PillHost`. Neither creates a recording or ships a fake/default production capture adapter.

The controller accepts `start` only from idle or a terminal state after a fresh readiness query. A capture moves through `starting` → `recording` → `stopping` → `processing` → `ready`; a pending or active capture can be cancelled, and a failed stop can be retried with its original ID. Session IDs scope every event, and out-of-order phase regressions are ignored.

The interface follows the existing GUI capture control contract: a caller-owned request ID for start, real-frame readiness distinguishing `starting` from `recording`, and ID-scoped stop/cancel/inspect receipts. Stop transport failures retain the same capture ID for retry. The Engine retains capture safety, permission checks, native media execution, finalization and bundle truth. `EngineClient.readiness()` is a future capability for querying actual native admission readiness; the existing CLI does not expose that as a separate query yet, so an adapter cannot infer it from an idle capture state.

## Development

Run `npm ci`, `npm run typecheck`, `npm test`, and `npm run build` in this directory. These commands use TypeScript and Node only; they do not use Cargo or import Desktop code.

## Adoption limits

The repository's `apparent capture` CLI already implements GUI start/stop/cancel/inspect control over a same-user Unix socket on macOS/Linux. A separate readiness query is not currently exposed. This package does not yet provide that process transport, Electron main/preload wiring, or an adopted React surface. Electron needs a host-owned adapter that invokes the existing Rust GUI control path, obtains native readiness from an accepted Engine capability, and maps JSON receipts/events into `EngineClient`. The Engine's native readiness and capture lifecycle remain authoritative. Until that adapter and platform seam proof land, this package is a tested workflow prerequisite and makes no production recording-availability claim.

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
