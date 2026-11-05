# Config host producer

This native producer builds `shared/config-native` for Linux x64 or macOS x64
from a clean, frozen checkout with locked Cargo inputs. The delivered directory
contains only `apparent-config-host` and `manifest.json`. The manifest records
the full source SHA, target/triple, protocol version, executable SHA-256, mode
0755 and the explicit `--config <caller-config-path> --owner <compiled-owner-id>`
arguments. It is unsigned and unnotarized and carries no resources.

The helper receives the caller's absolute config path and compiled owner ID on
its command line. Runtime use does not search `PATH`, invoke Cargo or npm, or
resolve another provider. Build verification runs the native crate tests and
the public client integration suite against the exact staged binary. This
artifact is not installed or activated by this producer; routing, signing,
notarization and packaged-app proof remain separate acceptance gates.

Run `npm ci`, `npm run typecheck` and `npm run contract` for producer contracts.
CI's `npm run verify` installs the locked shared contract and client dependencies,
then checks the TypeScript client and its transport tests as well. It invokes no
Rust; Cargo is confined to the explicit native `build` step.
From a clean committed checkout, run `OWNER_TARGET=linux-x64 npm run build`
on Linux x64 or `OWNER_TARGET=macos-x64 npm run build` on macOS x64. Set
`APPARENT_CARGO_OFFLINE=1` only when its complete locked graph is cached.
Cross-compilation is refused. The independent `platform-config` CI lane builds
each native target; consumers reuse admitted bytes and do not compile this crate.
