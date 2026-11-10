# Record completion

This is the headless post-publication workflow. It does not record, finalize,
format bundles, manage settings, choose a Connect destination or send/paste.
`src/policy.ts` owns saved/failed/copied/manual-handoff decisions. No Electron,
DOM, native source or other owner implementation is imported.

Run `npm ci`, `npm run typecheck`, `npm run build`, `npm test` in this directory
with Node 22.18+ and npm. Only TypeScript and Node types are development
dependencies. Tests separate pure decisions from the real Node worker protocol.
The ordinary owner loop needs neither Rust nor a compiled native provider.
This does not exempt composition, native provider changes or main/release checks.

## Production boundary

The unchanged native finalizer publishes its exact result. Desktop completes
the Agent receipt and notifies Connect before the presentation-generation gate;
the admission-pinned `capture_local_practice` flag still prevents practice upload
even after coaching closes. Connect keeps its own consent/transfer/retry policy.

For a current result Desktop starts `worker.ts` using the already packaged
Electron runtime with `ELECTRON_RUN_AS_NODE=1`, without a window or renderer.
One take uses one bounded subprocess and at most two exchanges. No new daemon,
socket listener, installer or configuration writer is introduced. The worker
chooses the clipboard capability; Desktop rechecks the current generation on
the GUI thread immediately before executing it against the retained native path.
It acknowledges `copied`, `failed` or `revoked`. Only an actual successful write
can produce `copied`. The blocked automatic trial requests text only and remains
manual; there is no paste/send capability in this protocol.

The native adapter also supplies the saved bundle's immutable `imageQualityWarning`.
Record preserves that exact warning in Linux completion notifications: an unsettled
ink sample is not evidence that decoding failed. Quality and manual-handoff notices
remain separate, with at most two bounded notices per completion. Desktop only
executes those notices after its current-generation check.

The result drives the existing native/installed Electron pill through
`completed_note`. Its Inspect control retains the saved bundle after copy failure.
The observations are `{capture:{generation,captureId,requestId}, outcome,
clipboard,current}`; generation is a decimal string (no JavaScript integer loss).
`outcome` is `saved` with `bundlePath` or `failed` with `message`; `clipboard` is
`notRequested`, `copied` or `failed`. These private facts never enter the renderer.
Onboarding consumes only the current capture's facts. Clipboard receipt is not
manual-paste confirmation; bundle-open receipts remain with the shell.

Dismissal, new capture or revocation prevents later capability/status effects;
the finalizer's own receipt remains complete. The reducer accepts duplicate
events without replaying writes. Disconnect/reconnect restores observations
without retrying an ambiguous clipboard write. Production does not retry a lost
worker. Native code retains the exact result, finishes bounded child cleanup,
and presents `Saved · handoff unavailable` with bundle access, or `Capture failed`.
It never reports copied based on child exit or renderer readiness.

## Composition and verification

Desktop's existing `scripts/build-pill.ts` typechecks these sources and emits
the worker into ignored `electron/record-completion/`. Existing settings staging
copies that directory with the locked runtime. No Cargo build input embeds the
policy. Native CI prerequisite preparation also runs that generator and requires
all three worker outputs before Cargo, including in jobs without a renderer build.
A policy-only owner build is independent; an installed composition still
requires Desktop packaging and native acceptance. No CI savings are claimed.

`npm run test:pill` in the Desktop frontend checks the generated worker with the
locked runtime, a real isolated Electron clipboard and the pill/reload consumer.
The explicit native provider test is
`cargo test record_completion_electron_rust_roundtrip -- --ignored --test-threads=1` after building
frontend assets. Synthetic facts/clipboard fixtures are not recording acceptance.
In an isolated desktop, `cargo test record_completion_native_clipboard_survives_worker_exit -- --ignored --test-threads=1`
checks a real bundle clipboard write followed by worker termination. The native
receipt survives that failed exchange; onboarding still receives `Some(true)`.
Verify exact installed Mac/Linux candidates, actual capture/save/failure, focus,
clipboard and bundle text/images separately under the Desktop native-test policy.

Default routing uses Record for both pill presentations. Missing/broken worker
assets use only the factual native recovery above, not a second copy-policy
implementation. Rollback requires reverting this dependency-complete slice;
old completion policy is not selectable at runtime. Native generation admission,
receipt/resource safety and exceptional saved-data access remain required while
the capture coordinator owns those resources.
