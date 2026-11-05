# Native Engine producer

This owner-local package builds and stages the standalone `apparent-engine`
Rust provider for the independent native delivery lane. It does not install or
route the binary into an application. The current provider speaks protocol 1,
reports `screenCapture: false` and `readiness: "unavailable"`, and durably
fails an admitted start without creating media. The artifact records those
limits. It is an unsigned, unnotarized executable, not a Mac installer.

Node 22.6+ and the Rust toolchain are required. Platform CI installs Rust. From
this directory:

```sh
npm ci
npm run verify
OWNER_TARGET=linux-x64 npm run build
```

`OWNER_TARGET` is required by the producer job and accepts only `linux-x64` or
`macos-x64`. Builds run only on the matching x64 host; no cross-compilation is
claimed. Set `APPARENT_CARGO_OFFLINE=1` to require an already cached locked Rust
dependency graph. CI should leave it unset so a cold runner may fetch the
locked graph. `build` uses the locked root engine graph in release mode, verifies
the output inventory, and exercises the exact staged executable through the
SDK's real Rust-process tests. The SDK remains a Rust-free consumer package;
only this `language: "native"` producer invokes Cargo.

The package artifact path is `dist/`. It contains exactly:

```text
dist/
├── apparent-engine
└── manifest.json
```

The manifest hashes the produced executable bytes and records the full source
commit, owner target, Rust triple, OS and architecture, protocol version,
executable mode and explicit launch arguments. Capture availability and required
runtime files are represented truthfully. Runtime launch uses an explicit
executable path and a caller-provisioned persistent private runtime directory:

```sh
/absolute/path/apparent-engine --runtime-directory /absolute/private/runtime
```

No `PATH` lookup, Cargo/npm invocation, source/build-tree dependency, helper,
model, secret or absolute checkout path is part of the runtime artifact. The
runtime directory must follow the Engine contract in `../../README.md`.

The build requires clean tracked source, freezes the full `HEAD` SHA before
compilation, checks it again after compilation and validates the staged bytes.
It refuses a pre-existing nonempty `dist/` so stale output cannot be mistaken
for the current candidate. Platform's external source fingerprint and build
attestation remain authoritative; the owner manifest checksum is an inventory
check, not provenance authorization.

## Producer input closure

The native delivery registration should fingerprint these repository inputs:

- `engine/delivery/native/` including package metadata, lockfile, scripts,
  tests and this contract.
- `engine/Cargo.toml`, `engine/Cargo.lock` and `engine/src/`.
- `capture/engine/Cargo.toml`, `capture/engine/Cargo.lock` and
  `capture/engine/src/`.
- `engine/tests/native-peer.test.ts`.
- `engine/sdk/package.json`, `engine/sdk/package-lock.json`,
  `engine/sdk/tsconfig.json`, `engine/sdk/src/` and `engine/sdk/test/`, which
  supply the real-process protocol verification consumer.
- Root `Cargo.toml` and `Cargo.lock` are not build inputs for this package; the
  engine manifest's path dependency resolves directly to `capture/engine`.
- The common delivery workflow/core, runtime-budget action and verifier inputs
  are included by the delivery system's mandatory input closure.

Any additional transitive Rust or Node build/verification input must be added to
the Platform fingerprint before artifact reuse or admission. No SDK registration
or shared registration slot is owned here.
