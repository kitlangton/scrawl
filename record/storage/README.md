# Record Storage

`@apparent/record-storage` owns capture-location preference interpretation,
retention choices and the request/readback controllers used by Desktop's React
Storage surface. Its injected ports expose bounded preference results and usage,
not files, config revisions, credentials or arbitrary native commands.

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
the existing Electron CommonJS location-policy adapter from the same source.

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

Desktop still owns Electron IPC, native dialog/revision sequencing and presentation.
Linux and native fallback retain their existing Settings controller/preset menu;
capture-time default resolution and cleanup policy interpretation also remain
native compatibility debt. This package's headless checks prove its policy and
controller loop, not Linux adoption, native GUI acceptance or whole-Settings build
independence. Routing and fallback/removal gates remain in the frontend README.
