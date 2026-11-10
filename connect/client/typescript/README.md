# Connect crypto compatibility primitives

This package is the bounded TypeScript prerequisite for migrating Connect crypto callers. It preserves the current Rust `encryption.rs` wire rules:

- age v1 binary X25519 streams, concatenated after transport part splitting;
- archive bytes as a big-endian u32 descriptor length, ordered JSON descriptor, and exact file bytes;
- `apparent-encrypted-v1\0signed-envelope\0` followed by the signed JSON in Rust struct field order;
- strict Ed25519 verification (canonical encodings, weak public keys and small-order `R` rejected).

It does not implement identity persistence, filesystem permissions, envelope/asset policy validation, service authorization, or production transfer lifecycle. Those remain migration work. In particular, this package's synthetic tests do not replace exercising the production Rust `pack`/`unpack` functions against TypeScript output.

## Local commands

Requires Node.js 22.18+ for the TypeScript test entrypoint.

```sh
npm ci
npm test
npm run typecheck
npm run build
npm run pack
```

These commands operate on TypeScript and npm dependencies; they do not invoke Cargo. `package-lock.json` pins the direct and transitive npm graph.

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

## Current restored-main boundary

This prerequisite is tested against restored main's production Rust age0.11.5
pack/unpack implementation. Its v1 context has exactly origin, accountId,
captureId and senderGrantId; evidence/media selection is not an additional
context field. The old optional group field is deliberately absent.

These are framing and cryptographic primitives, not an adopted transfer client.
Signature verification alone does not validate version, trusted context, sender
scope/grant/key or the chunk manifest. Archive framing alone does not validate
asset paths, unknown fields, hashes, count or total limits; custom part sizes are
for dependency tests and need not be valid Rust transport. Production TS callers
must add those owner validation/publication gates before replacing Rust.
Current native auth, consent, queue and config writers remain authoritative.

Build with `npm run build`. The packed package exports compiled JavaScript and
declarations from `dist`; ordinary packed consumers need plain Node22.18+, not
TypeScript loaders, Cargo or rustc. The explicit Rust interoperability lane
requires the compiled package first; it does not belong in ordinary npm install
or build hooks. No activation or writer cutover is included.
