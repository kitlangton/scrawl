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

On 2026-11-04, a temporary offline Rust harness using the pinned `age 0.11.5` and `ed25519-dalek 3.0.0` produced/consumed synthetic 80,000-byte age payloads through multipart reassembly in both directions. It also produced an Ed25519 signature that matched the TypeScript signature byte-for-byte. The temporary harness and keys were kept outside the repository. This establishes dependency-level wire interoperability only; production `pack`/`unpack` and a checked-in Rust-generated fixture remain required before claiming full envelope compatibility.
