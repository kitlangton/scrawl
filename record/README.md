# Record package

`@apparent/record` is an independently installable TypeScript workflow package. It owns explicit local recording states and consumes an injected `EngineClient`; it does not create a recording itself and ships no fake or default production Engine adapter.

The controller accepts `start` only from idle or a terminal state after a fresh readiness query. A capture moves through `starting` → `recording` → `stopping` → `processing` → `ready`; a pending or active capture can be cancelled, and a failed stop can be retried with its original ID. Session IDs scope every event, and out-of-order phase regressions are ignored.

The interface follows the existing GUI capture control contract: a caller-owned request ID for start, real-frame readiness distinguishing `starting` from `recording`, and ID-scoped stop/cancel/inspect receipts. Stop transport failures retain the same capture ID for retry. The Engine retains capture safety, permission checks, native media execution, finalization and bundle truth. `EngineClient.readiness()` is a future capability for querying actual native admission readiness; the existing CLI does not expose that as a separate query yet, so an adapter cannot infer it from an idle capture state.

## Development

Run `npm ci`, `npm run typecheck`, `npm test`, and `npm run build` in this directory. These commands use TypeScript and Node only; they do not use Cargo or import Desktop code.

## Adoption limits

The repository's `apparent capture` CLI already implements GUI start/stop/cancel/inspect control over a same-user Unix socket on macOS/Linux. A separate readiness query is not currently exposed. This package does not yet provide that process transport, Electron main/preload wiring, or an adopted React surface. Electron needs a host-owned adapter that invokes the existing Rust GUI control path, obtains native readiness from an accepted Engine capability, and maps JSON receipts/events into `EngineClient`. The Engine's native readiness and capture lifecycle remain authoritative. Until that adapter and platform seam proof land, this package is a tested workflow prerequisite and makes no production recording-availability claim.
