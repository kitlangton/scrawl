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

Platform sets `OWNER_TARGET` to `linux-x64` or `macos-x64`; local builds may omit
it to select the matching supported x64 host. Builds run only on the matching
x64 host; no cross-compilation is claimed. Set `APPARENT_CARGO_OFFLINE=1` to require an already cached locked Rust
dependency graph. CI should leave it unset so a cold runner may fetch the
locked graph. `build` runs the locked staged capture Engine and provider Cargo
tests, builds the release provider, verifies the output inventory, and exercises
the exact staged executable through the SDK's real Rust-process tests. The SDK
remains a Rust-free consumer package;
only this `language: "native"` producer invokes Cargo.

`delivery.json` beside this README is the native producer registration contract.
Platform must add its separate `engine-native` catalog slot before it executes;
the existing `engine/delivery.json` continues to register the TypeScript SDK.

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
It replaces an existing nonempty `dist/` only after validating its exact known
artifact inventory; unexpected contents are preserved and rejected. The new
artifact must match the current candidate. Platform's external source fingerprint and build
attestation remain authoritative; the owner manifest checksum is an inventory
check, not provenance authorization.

## Producer input closure

The delivery registration accepts directory roots, so the bounded complete
owner roots are `engine/` and `.cargo/`. `.cargo/config.toml`
sets the Linux CPU baseline and macOS deployment target inherited by Cargo;
it must participate in native artifact fingerprints. `engine/` includes the producer
package, Rust manifests/lockfile/source/tests and the SDK files used by
real-process verification. It also fingerprints the existing `engine/delivery.json`
and the SDK package graph as part of that directory; neither is modified or
registered as a native producer. Root `Cargo.toml` and `Cargo.lock` are outside
these roots and are not build inputs: the engine manifest's path dependency
resolves directly to `engine/core`.

The common delivery workflow/core, runtime-budget action and verifier inputs
are included by the delivery system's mandatory input closure.

Any additional transitive Rust or Node build/verification input must be added to
the Platform fingerprint before artifact reuse or admission. SDK registration
and shared catalog wiring remain Platform-owned.
