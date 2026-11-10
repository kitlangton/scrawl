# Record Storage

`@apparent/record-storage` owns capture-location preference interpretation,
retention choices and the request/readback controllers used by Desktop's React
Storage surface. Renderer-controller ports expose bounded preference results and
usage, not files, revisions, credentials or arbitrary native commands. Its host
`LocationWorkflow` consumes revision-bearing native facts/probes, an OS chooser
port and allowlisted set/reset execution; it never exposes revisions to rendering.

```sh
npm ci
npm run build
npm run typecheck
npm test
```

These owner commands require Node >=22.18 and the locked TypeScript compiler.
They need no Rust, Electron, React or native build output. Public consumers load
compiled ESM and declarations through the package export; `npm pack` rebuilds
the fixed `dist` inventory. The Desktop build uses its own installed compiler to
build these same exports before typechecking/bundling its consumer, and generates
the Electron CommonJS location-policy and workflow adapters from the same source.

Reads supersede older reads. Mutations serialize admission and require a trusted
readback; failed or uncertain writes require Refresh before another mutation.
An uncertain reply never causes a write replay. Usage replies are retired when
preferences change and rejected if their effective folder differs. Disposing a
controller retires notifications and follow-up reads. It does not prove that an
open native chooser was cancelled or that a dispatched write was prevented.

The native config transaction remains the single durable writer, preserving
unknown fields/secrets, locking, CAS, cancellation before publication and uncertain
publication semantics. Native ports retain chooser execution, filesystem safety,
bounded scanning and active-capture exclusions. Startup-folder timing is unchanged:
existing workers retain their startup folder until the native app restarts.

Record owns location facts/probe resolution, chooser-result validation and
revision-bearing save/reset intent. Desktop retains Electron IPC, OS dialog
execution, shared mutation admission, window lifetime and presentation. The host
must hold that gate for `mutate` and supply its current lifetime check; the
workflow suppresses stale results and never replays writes. A thrown write or
post-publication resolution failure returns `outcome_unknown` for refresh.
`RetentionWorkflow` keeps executor revisions private, coalesces same-version
reads, validates save/reset intent and retires stale preference/usage replies.
Desktop supplies the shared version/lifetime check and holds the native-operation
gate for mutations. Invalidation retires pending replies independently; failed or
uncertain writes require fresh authoritative readback, including when a read
completed while the write was pending. Both renderer and native transport guards
consume Record's same bounded retention-policy validator.
Linux and native fallback retain their existing Settings controller/preset menu,
but all routes persist recognized retention leaves through the same native
Storage executor, preserving unknown nested keys. The fallback obtains a fresh
owner revision before saving; a concurrent change is refused without replay.
Engine no longer supplies a whole-object retention writer;
capture-time default resolution and cleanup policy interpretation also remain
native compatibility debt. This package's headless checks prove its policy and
controller loop, not Linux adoption, native GUI acceptance or whole-Settings build
independence. Routing and fallback/removal gates remain in the frontend README.
