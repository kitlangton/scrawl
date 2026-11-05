# Record package

`@apparent/record` is an independently installable TypeScript workflow package. It owns explicit local recording states and consumes an injected `EngineClient`. `EngineSdkAdapter` is an injected host composition adapter for the packed `@apparent/engine-sdk`; it does not choose an executable, runtime directory, bundle allocator, or durable private mapping store.

The controller accepts `start` only from idle or a terminal state after a fresh readiness query. A capture moves through `starting` → `recording` → `stopping` → `processing` → `ready`; a pending or active capture can be cancelled, and a failed stop can be retried with its original ID. Session IDs scope every event, and out-of-order phase regressions are ignored.

The interface follows the existing GUI capture control contract: a caller-owned request ID for start, real-frame readiness distinguishing `starting` from `recording`, and ID-scoped stop/cancel/inspect receipts. Stop transport failures retain the same capture ID for retry. The Engine retains capture safety, permission checks, native media execution, finalization and bundle truth. `EngineClient.readiness()` is a future capability for querying actual native admission readiness; the existing CLI does not expose that as a separate query yet, so an adapter cannot infer it from an idle capture state.

## Development

Run `npm ci`, `npm run typecheck`, `npm test`, and `npm run build` in this directory. These commands use TypeScript and Node only; they do not use Cargo or import Desktop code.

## Adoption limits

The `EngineSdkAdapter` consumes an already constructed SDK client. The host still owns explicit executable/runtime selection, private durable mapping storage, bundle allocation, Electron main/preload wiring, and any adopted React surface. The current SDK contract has no native executable, so it cannot provide production readiness or native capture evidence. The adapter is not an installed route.

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
