# Connect TypeScript client components

This package supplies the adopted Electron account controller/presentation and
the crypto compatibility primitives for #529. It preserves the current Rust
`encryption.rs` wire rules:

- age v1 binary X25519 streams, concatenated after transport part splitting;
- archive bytes as a big-endian u32 descriptor length, ordered JSON descriptor, and exact file bytes;
- `apparent-encrypted-v1\0signed-envelope\0` followed by the signed JSON in Rust struct field order;
- strict Ed25519 verification (canonical encodings, weak public keys and small-order `R` rejected).

`validateRecipientEnvelope` is the parsed-DTO recipient boundary: it bounds the
envelope, enforces the v1 context and ordered multipart chunk rules, pins every
context field to caller supplied expectations, and verifies the signature with
the caller's trusted sender signing key. Its typed error categories contain no
wire content. Signature integrity is not service authorization: callers must
resolve the sender grant/key from current trusted authorization state first.
Use `validateRecipientEnvelopeBytes` for raw wire JSON. It enforces the 128 KiB
limit, valid UTF-8 and bounded JSON complexity, and rejects duplicate decoded
keys (including escaped aliases) before structural or signature validation.
The parsed-object API remains for already decoded DTOs; `JSON.parse` alone
cannot establish those raw-input properties.

`openRecipientArchive` consumes a raw envelope, ordered ciphertext parts,
endpoint identity and independently pinned context/sender key. It checks part
hashes, a bounded X25519 age header, authenticated decryption through final EOF,
the exact inner context, v1/group manifests, every file hash and bundle references.
No plaintext is returned until all checks pass. Ciphertext is consumed a part at
a time; validated plaintext (at most 500,000,000 bytes) is retained in memory.
Cancellation and errors return allowlisted categories, without wire content.
This API performs no service authorization, credential/config writes or filesystem
publication. Callers must resolve and recheck current sender authorization and
publish through the private, no-overwrite staging boundary. Identity persistence,
filesystem safety and production transfer lifecycle remain migration work.
Existing Rust callers remain authoritative; this module is not installed in a
production recipient path and does not replace the active Rust client.

## Authenticated recipient retrieval

`retrieveRecipientCapture` reads a capture through the real bearer-only HTTP
transport. The caller supplies a current read credential and endpoint identity.
It checks the authenticated peer registry against that local identity, resolves
the signed sender only from approved upload peers, pins account/origin/capture/
group independently, checks wire hashes before decryption, and rechecks current
sender authorization before returning verified in-memory files. Grouped
evidence-only retrieval skips media bytes. Legacy plaintext requires explicit
`allowLegacyPlaintext: true`; no automatic downgrade or token renewal occurs.

Requests never follow redirects, send cookies or include private data in error
messages. JSON/file byte limits and per-request deadlines cover response bodies;
`waitSeconds` bounds only readiness polling (0–900 seconds). Transfer requests
have 60-second deadlines individually, so callers should supply an AbortSignal
for an overall workflow deadline. Server bases retain the native client's HTTPS
or IPv4/localhost HTTP profile and exact root or `/connect` signed identity.

This API does not read or write credential files, refresh tokens, mutate config,
run a worker, or publish directories. Its caller remains responsible for the
single durable writer and private atomic no-overwrite publication; evidence-only
publication must retain the existing `.evidence-only.json` receipt. Installed
CLI/fetch paths still use the Rust implementation until those seams are adopted.

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
When the native peer advertises `deviceFlow: 1`, `device-flow.cts` owns the
installed device-start reply validation, public approval projection, polling,
expiry/rejection, credential validation and commit intent. The main process alone
receives private device replies; they never enter renderer views. Native code
executes the two allowlisted HTTP calls and the existing single-writer credential/
config transaction. It retains the approved response and original issue time;
commit consumes it once after fresh cancellation/marker/queue checks.

`settings-state.cts` derives local account state from `stateFacts: 1` private
metadata: configured origin, independent upload gates, grant usability/current
access, renewal/reauthorization and bounded account initial. It never probes or
mutates state. Raw facts and unknown metadata are stripped before renderer views;
malformed facts fail closed rather than using a legacy snapshot.

Native code still supplies safe private-record reads, renewal and durable mutations. Login pins the origin displayed at consent; activation needs
a successful matching operation and fresh committed upload gates. EOF, timeout
or feature disable before activation retains native durable cancellation. Failed
readback starts durable cleanup instead of leaving a reusable authorization.
These Rust HTTP/renewal/private-record/storage dependencies and older-consumer auth remain explicit migration debt;
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

The optional local service integration proof uses the actual built Worker with
isolated Miniflare D1/R2, real synthetic device approval/peer registration, signed
encrypted upload, retrieval on root and mounted origins, and revocation. From the
repository root, install/build the service before running it:

```sh
npm --prefix connect/service ci
npm --prefix connect/service run build
npm --prefix connect/delivery/client run test:service
```

This separate proof uses service development dependencies. It is outside the
independent client stream's default `typecheck`/`test` CI; those commands test the
bounded HTTP and recipient behavior without building the service or Rust.

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
APPARENT_CONNECT_INTEROP_SCRIPT="$PWD/connect/delivery/client/test/production-interop.ts" \
cargo test --manifest-path connect/client/Cargo.toml --offline \
  production_typescript_crypto_interop -- --ignored --nocapture
```

On Clanker, wrap that Cargo command once with `t3-build`, per root instructions.
This explicit cross-language migration check needs Rust; ordinary package
install/build/test/pack commands above do not. The fixture does not certify
production TypeScript transport, filesystem publication, account enrollment or
grouped evidence/media consumers, which remain migration work. The envelope
validator is exercised on the production Rust multipart envelope in this lane,
alongside synthetic context, bounds, signature and mutation cases. Authenticated
retrieval now supplies bounded decoding and current authorization-derived
context/key facts in TypeScript. Retain the active Rust caller and native
publication gates until installed consumers and safe publication are independently
verified.

## Canonical API base compatibility

Client settings and recipient contexts accept a legacy root API URL or the exact `/connect` API base. API routes are appended to that base; HTTP Origin/CSRF allowlists remain origin-only. The signed envelope context stays pinned to the exact normalized transport base, account, capture, sender grant and group. The migration-only production crypto lane exercises both root and mounted contexts through actual Rust pack/unpack and TypeScript encryption/signatures. Ordinary TypeScript package commands do not invoke Rust.

The active native caller remains authoritative migration debt. This source change does not update installed apps: publication/adoption of a client containing the mounted-base fix is required before the canonical setup command is available to those installations. Keep the existing root API compatibility profile until those consumers migrate.
