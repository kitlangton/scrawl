# Owner config host

This privileged TypeScript client binds a single owner and an explicitly supplied
native artifact to the API in `@apparent/app-contracts/host`. It stages the verified
bytes in a fresh private directory and runs that copy, then removes it on close.
Replacing the admitted source path cannot change the executable being launched.
It never opens the
configuration file, invokes Cargo, searches PATH or installs a provider. The
native resource uses the same transaction implementation and sidecar lock as
existing application writers, preserving unrelated settings and secrets.

Composition supplies a verified artifact path/hash and config path, then binds
the stream's own codec with `bindOwnerProgress(backend, ownerId, codec)`. Only
that narrow facade may cross a renderer boundary. No generic filesystem or IPC
API is exposed. The new `streamState` envelope is host metadata; stream schema,
legacy interpretation and journey decisions remain owner-owned.

Requests are serialized, bounded and correlated. A lost write reply is `unknown`;
there is no automatic retry or guessed success. After transport failure, close
the backend, construct a new one and read persisted state before deciding what
to do next. Shutdown closes the child process; it does not cancel Engine capture.

Run `npm ci`, `npm run typecheck` and `npm test` without Rust. Native producer
verification additionally sets `APPARENT_CONFIG_HOST_BIN` to the exact staged
executable and runs `npm run test:native`. A passing transport fixture is not
native durability or installed-app evidence. This package does not install a
preload bridge, select an installed route, implement coaching windows or provide
Record/Engine capabilities.
