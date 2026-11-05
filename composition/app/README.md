# App Shell foundation

This development-only shell composes two synthetic stream artifacts into one
Electron application. It does not replace the installed Apparent route, capture,
onboarding, updater or production Settings. No experiment/config flag is added.
Record, Connect and Engine are not adopted by this slice.

## Ownership and contract

`composition/app` owns startup, navigation, assembly and host packaging.
`shared/contracts` owns the versioned manifest and React contribution types;
`shared/ui` supplies canonical controls and appearance tokens. Streams depend on
contracts, never on the shell implementation. The shell imports only compiled
stream artifacts. There is no dynamic plugin discovery or arbitrary-path loading.

Each stream exports a default `{id, Screen, Settings}` contribution. `Settings`
is the owner's component, including validation, persistence and effects. The
shell simply presents it. Alpha stores its boolean and Beta stores its selection
under separate synthetic localStorage keys; changing one does not rewrite the
other. This is synthetic persistence proof, not a production config API or a
security boundary between trusted streams. Future owners choose their supported
persistence/credential adapter; no renderer receives Node or generic filesystem IPC.
There is intentionally no preload bridge until a real capability requires one.

Artifacts contain exactly `index.js` and `manifest.json`. The manifest pins the
stream identity/version, shell API, React version and entry checksum. Producers
bundle their own dependencies except the explicitly shared React runtime.
The shell artifact includes a single shared React runtime. Assembly validates
compatibility, inventories and hashes, rejects duplicate identities/symlinks,
revalidates copied bytes and writes an `assembly.json` receipt.
Checksums detect corruption; they do not authenticate externally supplied code.
Only trusted local build outputs are accepted in this foundation. Production
artifact admission/signing remains a separately required adoption boundary.

## Build, assemble and run

Node 22.18+ and npm are required. No Rust toolchain or existing application build
outputs are needed. Build the contracts package before installing its file consumer:

```sh
npm --prefix shared/contracts ci
npm --prefix composition/app ci
npm --prefix shared/contracts run typecheck
npm --prefix shared/contracts test
npm --prefix composition/app run check
npm --prefix composition/app run build
npm --prefix composition/app start
```

Electron's npm package does not automatically install its runtime. `start`
prepares it explicitly; CI runs `npm run prepare:electron` before provisioning
the matching sandbox helper. Packaging independently obtains the pinned runtime.

`build:shell`, `build:alpha` and `build:beta` are independently callable producer
commands. `assemble` only consumes their artifacts; it invokes no compiler and
does not inspect producer source. `build` is a first-run convenience composition.
Alpha/Beta are fixtures using a reference producer, not real product owners or
required build tools for future streams.

Assembly requires a **fresh destination** and refuses to overwrite an existing
app. To assemble an update after rebuilding only Alpha, reuse the existing shell
and Beta artifacts and choose another directory:

```sh
cd composition/app
npm run build:alpha
node scripts/assemble.ts dist/shell dist/app-next dist/artifacts/alpha dist/artifacts/beta
node scripts/package.ts dist/app-next dist/packages-next
```

`npm run package` bundles `dist/app` with the published Electron runtime for the
current Linux/macOS host and x64/arm64 architecture; it does not compile native
source. The development bundle has a distinct name, bundle ID and user profile.
It is unsigned and has no installer/updater/publication integration. Host-native
packaging is supported by the command; Linux packaged execution is tested here.
Mac packaged launch/signing proof is required before adopting that route on Mac.

## Verification and adoption

`npm run check` rebuilds/tests the contracts, checks installed local dependency
freshness, typechecks and exercises artifact acceptance, independent rebuild,
unchanged artifact preservation and assembly with Cargo/rustc/TypeScript/esbuild/npm
commands denied. `xvfb-run -a npm run test:electron` on Linux launches the real
packaged executable, exercises both screens and Settings sections, verifies owner
preferences across restart and checks renderer isolation/network refusal.
These are synthetic shell checks, not native recording, clipboard or Mac proof.

Linux execution requires working Chromium sandbox support. On restricted test
hosts, `APPARENT_FOUNDATION_SANDBOX_HELPER=/opt/google/chrome/chrome-sandbox`
may name an existing root-owned, setuid, non-writable-by-others helper. The test
links that helper into its disposable runtime only; it changes no system files
and never disables sandboxing. CI provisions the helper from the pinned Electron
package with root ownership and mode `4755` in the disposable runner's temporary
directory, then removes it after testing. It does not rely on the runner's Chrome
helper ownership/mode or alter system Chrome.

Feature pushes select the foundation check for shell, contracts, shared UI or
delivery-workflow inputs; dependencies must already be installed. Main runs
the independent `Value-stream delivery` workflow, without the legacy verify
selector. Registered owners build and verify separately; unchanged owner inputs
can reuse a successful prior component receipt from that workflow. The shell
lane packages its prebuilt output and tests that exact bundle on Linux and Mac.
See [delivery contracts](../delivery/README.md) for registration and admission.
Artifacts remain development output; stable publication and an updater are not
provided. Existing native verification and publication gates remain conservative.

For production adoption, a stream must supply its real artifact/contract and
owner checks, prove affected OS seams, then switch a complete workflow. Preserve
the current installed route until replacement acceptance; remove each old route
only after its remaining consumers and packaging inputs are verified. Until then,
rollback is to close this separate development application; existing Apparent
continues to run normally.
