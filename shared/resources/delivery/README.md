# Native resource producer

This package is the only Rust-building route for `@apparent/native-resources`.
It requires a clean committed checkout, locked npm dependencies and a native
Rust toolchain for the selected host target. Set `OWNER_TARGET` to `linux-x64`,
`macos-x64` or `macos-arm64`; cross-compilation is refused as native proof.

```sh
npm ci
npm run typecheck
npm test
OWNER_TARGET=linux-x64 npm run build
```

On Clanker wrap build once with `t3-build`, honoring its admitted worker count;
set `APPARENT_CARGO_OFFLINE=1` when the locked dependencies are already cached.
Build runs Rust tests and release compilation, emits exactly `dist/apparent-resource-host`
and `dist/manifest.json`, then runs the public adapter's real native suite against
that staged executable. No root application or Connect Rust package is built.
Consumers install the independent TypeScript package and verified native artifact;
they never invoke this producer as a package install hook.

The output manifest seals protocol, platform, source SHA, executable mode and
hash. Workflow receipts and attestation must additionally establish trusted
source/input closure and producer identity. Platform owns workflow/admission
integration; this local producer alone does not make an artifact available to
installed consumers. Binary signing and installed assembly remain separate gates.
