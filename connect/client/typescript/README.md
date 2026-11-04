# Connect TypeScript client components

This package supplies the adopted Electron account controller/presentation and
the crypto compatibility primitives for #529. It preserves the current Rust
`encryption.rs` wire rules:

- age v1 binary X25519 streams, concatenated after transport part splitting;
- archive bytes as a big-endian u32 descriptor length, ordered JSON descriptor, and exact file bytes;
- `apparent-encrypted-v1\0signed-envelope\0` followed by the signed JSON in Rust struct field order;
- strict Ed25519 verification (canonical encodings, weak public keys and small-order `R` rejected).

It does not implement identity persistence, filesystem permissions, envelope/asset policy validation, service authorization, or production transfer lifecycle. Those remain migration work. In particular, this package's synthetic tests do not replace exercising the production Rust `pack`/`unpack` functions against TypeScript output.

## Account Settings adoption

`settings-controller.cts` owns explicit consent, confirmation, asynchronous
request/receipt ownership, cancellation, retries, browser-action admission and
worker-activation intent. `settings-presentation.ts` owns the card presentation.
Desktop's existing `electron/connect.cjs` entry is generated from a thin TypeScript
composition entry plus these modules; prebuild/pretest compile them without Rust.
The installed Settings stager copies the same generated asset tree and existing
Electron runtime. No second runtime, service or config writer is introduced.

`settings-transport.cts` consumes the narrow backend RPC on the existing private
socket (`connect: backend`, `version: 1`; responses carry `account.version: 1`).
Native code supplies local snapshots, existing device auth/renewal and
durable mutations. Login pins the origin displayed at consent; activation needs
a successful matching operation and fresh committed upload gates. EOF, timeout
or feature disable before activation retains native durable cancellation. Failed
readback starts durable cleanup instead of leaving a reusable authorization.
These Rust auth/snapshot/storage dependencies remain explicit migration debt;
this adoption does not complete the account or Rust-free stream acceptance.

Fallback to `legacy-settings.cts` is allowed only for an unsupported **read-only
opening**, before any action. It preserves the prior transport for older native
versions; its mechanically retained unchecked types are bounded compatibility
debt. No ambiguous mutation is retried. Remove it and the native controller only
after all supported consumers and native fallback surfaces migrate. The existing
native Settings path remains available for surfaces not yet replaced; no two
controllers drive one window.

## Local commands

Requires Node.js 22.18+ for the TypeScript test entrypoint.

```sh
npm ci
npm test
npm run typecheck
npm run build
npm run pack
npm run dev
```

These commands operate on TypeScript and npm dependencies; they do not invoke Cargo.
`dev` watches the owner package output; rebuild/relaunch the existing Desktop
frontend to consume a changed controller there, against an unchanged native
backend. `prepack` builds the exported runtime/declarations. `package-lock.json`
pins the direct and transitive npm graph.

## Dependency provenance and bridge evidence

- `age-encryption@0.3.1` is the upstream FiloSottile TypeScript age implementation: <https://github.com/FiloSottile/typage>. Its README documents Node.js 20+, binary `Uint8Array` streams and age CLI interoperability.
- `@noble/ed25519@3.2.0` supplies Ed25519. Verification explicitly uses the library's strict (`zip215: false`) mode and rejects small-order public keys and `R`, matching the current Rust caller's strict verification intent.

The reproducible `production_typescript_crypto_interop` Rust test calls the
actual production `pack` and `unpack` functions. It generates private synthetic
identities and a 33 MiB payload, then invokes `test/production-interop.ts`.
TypeScript verifies Rust's signed bytes/signature, stored identities, multipart
stream and original bytes, and publishes a newly encrypted/signed stream for
Rust to unpack. Both sides reject wrong recipients and tampering; strict
Ed25519 negative vectors are checked against Rust's `verify_strict` result.
All generated identities and payloads stay in the private test fixture and are
removed by its existing cleanup. No static private keys or large binaries are
checked in. Run this migration-only compatibility lane from the repository root:

```sh
APPARENT_CONNECT_INTEROP_NODE="$(command -v node)" \
APPARENT_CONNECT_INTEROP_SCRIPT="$PWD/connect/client/typescript/test/production-interop.ts" \
cargo test --manifest-path connect/client/Cargo.toml --offline \
  production_typescript_crypto_interop -- --ignored --nocapture
```

On Clanker, wrap that Cargo command once with `t3-build`, per root instructions.
This explicit cross-language migration check needs Rust; ordinary package
install/build/test/pack commands above do not. The fixture does not certify
production TypeScript transport, filesystem publication, account enrollment or
grouped evidence/media consumers, which remain migration work.
