# App Shell foundation

This development-only shell composes independently built stream artifacts into
one Electron application. Its default build is a usable empty shell; synthetic
contributions are generated only in disposable test directories. It does not
replace the installed Apparent route, capture, onboarding, updater or production
Settings. No experiment/config flag is added. Record, Connect and Engine are
not adopted by this slice.

## Ownership and contract

`composition/app` owns startup, navigation, assembly and host packaging.
`shared/contracts` owns the versioned manifest and React contribution types;
`shared/ui` supplies canonical controls and appearance tokens. Streams depend on
contracts, never on the shell implementation. The shell imports only compiled
stream artifacts. There is no dynamic plugin discovery or arbitrary-path loading.

Each stream exports a default `{id, Screen, Settings}` contribution. `Settings`
is the owner's component, including validation, persistence and effects. The
shell simply presents it. Tests generate small contributions with independent
synthetic preferences; this is persistence proof, not a production config API
or a security boundary between trusted streams. Future owners choose their
supported persistence/credential adapter; no renderer receives Node or generic
filesystem IPC. The default empty shell ships a narrow preload facade for fixed
Onboarding progress read/CAS calls, but has no registered handler unless a trusted
main binding is supplied at build time. It exposes no generic IPC, filesystem or
config-path operation. The route is bound to the exact main frame and local entry
URL, then revoked on navigation or window close.

`@apparent/app-contracts/host` defines the API 1 owner host seam. A compiled-in
`ContributionFactory` declares its own dependency type; composition binds the
expected owner once, verifies factory and module identity, and serializes
activation/disposal. Main binds each config namespace before exposing a narrow
owner adapter.
`ProgressStore` matches the Onboarding `read`/`compareAndSwap` shape. Its codec
belongs to the owner and handles initial and legacy values. The app adapter only
delegates to a transactional backend; it has no filesystem writer or JS
read-modify-write fallback. `composition/app/scripts/build.ts` accepts one trusted
compiled main binding, its published Onboarding assets, and the exact staged config
helper plus digest. It copies and hashes those inputs into the shell inventory;
runtime code derives the config file from Electron's user data directory and fixes
the helper path inside the application. No renderer input selects an owner, executable, config path or key.
The existing foundation profile override remains a trusted launcher input. A build without these inputs
keeps the empty shell usable and registers no progress handler.

After explicit owner bootstrap and the legacy-writer compatibility gate succeed,
a composition assembly module can bind the published public package exports:

```ts
import { createConfigBackend } from '@apparent/config-host';
import { onboardingProgressCodec } from '@apparent/onboarding';
import { bindOwnerProgress } from './host-services.js';

export async function createOnboardingProgressStore({ configPath, configHostPath }) {
  const backend = await createConfigBackend({ executable: configHostPath,
    executableSha256: ONBOARDING_CONFIG_HOST_SHA256, configPath, ownerId: 'onboarding' });
  return { store: bindOwnerProgress(backend, 'onboarding', onboardingProgressCodec),
    close: () => backend.close() };
}
```

The binding module is compiled into `owner-binding.mjs`; main supplies only its
fixed config path and the helper copied into the app. Do not retire the legacy
writer or adopt installed user state automatically: quiesce the old writer and
make cutover explicit. The native integration test uses a separate synthetic
config, retires its unused legacy writer, adopts through
`adoptOnboardingProgress`, and verifies that unrelated keys and legacy bytes
remain. This seam does not switch the installed route or finish Record injection,
coaching or full factory activation. Optional renderer injection is adapted inside
composition; the shared no-props `StreamModule` ABI remains unchanged.

The assembly caller supplies the published package resolution root and matching
assets at build time:

```ts
await buildShell(shellOutput, {
  module: compiledCompositionBinding,
  configHost: { path: attestedLinuxHelper, sha256: attestedHelperSha256 },
  ownerAssets: publishedOnboardingAssets,
  resolvePaths: [publishedPackagesNodeModules],
});
```

The helper path/hash are admission inputs to this build call, not runtime app
settings. Onboarding retirement/adoption decisions belong in the supplied
composition binding and must be explicit for the target config.

Coaching views use bounded declarative content, placement, and owner-allowlisted
identifiers validated against a compiled-in view list.
The foundation host reports the coaching provider unavailable. Production
adoption still needs an owner/native provider, boundary validation, and affected
platform checks; this contract adds no native effect, IPC route, or renderer
filesystem access. The compiled-in factory registry and activation call sites
also remain to be connected by the production composition owner.

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
Both paths validate the archive against the checksums shipped in the pinned
Electron npm package. If its official release download returns HTTP 5xx, the
same archive may be fetched through the official GitHub release API; its digest
must match that pin. The upstream installer still selects and extracts the
runtime. Packaging uses the verified ZIP without a second checksum download.

`build:shell` compiles the shell. `build` creates the empty default application
and `assemble` only consumes its shell and any explicitly supplied stream
artifacts; assembly invokes no compiler and does not inspect producer source.
The repository does not ship demo stream sources or a production stream producer.

Assembly requires a **fresh destination** and refuses to overwrite an existing
app. To assemble an update from owner-built artifacts, reuse the existing shell,
pass the artifact directories and choose another destination:

```sh
cd composition/app
node scripts/assemble.ts dist/shell dist/app-next /path/to/stream-a /path/to/stream-b
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
freshness, typechecks and exercises generated synthetic artifact acceptance,
independent rebuild, unchanged artifact preservation, empty-shell assembly and
assembly with Cargo/rustc/TypeScript/esbuild/npm commands denied.
`xvfb-run -a npm run test:electron` on Linux launches the exact packaged empty
shell plus a separately packaged app with generated test contributions, exercises
both screens and Settings sections, verifies preferences across restart and
checks renderer isolation/network refusal.
These are synthetic shell checks, not native recording, clipboard or Mac proof.

Linux execution requires working Chromium sandbox support. On restricted test
hosts, `APPARENT_FOUNDATION_SANDBOX_HELPER=/opt/google/chrome/chrome-sandbox`
may name an existing root-owned, setuid, non-writable-by-others helper. The test
links that helper into its disposable runtime only; it changes no system files
and never disables sandboxing. CI provisions the helper from the pinned Electron
package with root ownership and mode `4755` in the disposable runner's temporary
directory, then removes it after testing. It does not rely on the runner's Chrome
helper ownership/mode or alter system Chrome.

Pull requests run the `app-shell delivery` owner checks on Linux and Mac; main
pushes also produce the registered development-app receipts. The shell lane
packages its prebuilt output and tests that exact bundle on Linux and Mac.
See [delivery contracts](../delivery/README.md) for registration and admission.
Artifacts remain development output; stable publication and an updater are not
provided. Existing native verification and publication gates remain conservative.

For production adoption, a stream must supply its real artifact/contract and
owner checks, prove affected OS seams, then switch a complete workflow. Preserve
the current installed route until replacement acceptance; remove each old route
only after its remaining consumers and packaging inputs are verified. Until then,
rollback is to close this separate development application; existing Apparent
continues to run normally.

The bridge unit tests cover deterministic promotion/revocation, registration
rollback/successors and destroyed-window cleanup. The explicit native Electron
lane is `APPARENT_ONBOARDING_CONFIG_HOST=<verified-helper> npm run test:electron:onboarding`
(use Xvfb and the existing sandbox helper on Linux). It consumes the pinned Linux
or macOS helper artifact, exercises real packaged main/preload/renderer calls,
and requires privileged host cleanup to finish before application exit. No Rust
compilation is added to the normal TypeScript check loop.

## Record host composition checkpoint

`buildShell(output, undefined, { module, resolvePaths? })` accepts a trusted
composition module exporting `createRecordHostServices({ profilePath })`. It
returns the public `RecordClient`, authoritative `canClose()` and `close()`.
The binding is bundled and hashed into the existing shell inventory. The renderer
cannot select a package, executable, profile, config owner or native filesystem
path. This Record-only binding is separate from the existing progress binding;
it neither changes nor activates Onboarding.

Main exposes fixed, bounded Record read/execute/observation channels only to the
exact main frame of its owned window. `connectRecordClient` adapts the preload
facade to Record's immediate-observation API. Unknown command replies are never
retried. Disposing renderer observation does not detach the provider, cancel a
capture or create shutdown clearance. A close request stays blocked while the
host reports unresolved ownership or cleanup fails. A production binding must
serialize its admission/shutdown boundary and include in-flight commands and
incomplete reconciliation when deciding clearance; a snapshot of renderer state
is insufficient.

The pinned Record tarball and provenance beside it identify the independently
verified public producer used for type and fixture integration. `npm run
test:electron:record` exercises the packaged main/preload/renderer path with that
public fixture provider and a private synthetic profile. Ordinary App Shell
Electron CI includes this check on Linux and Mac. This proves transport and
window lifecycle mechanics, not a production capture/provider or installed route.

Remaining Platform acceptance: bind the real Record/native runtime and trusted
artifact discovery; verify the pill-to-capture/processing/authenticated-Saved/copy
composition; qualify native window and clipboard boundaries and legacy-writer
compatibility; then complete delivery/adoption under existing explicit gates.
This Record-only slice does not include Onboarding factory, journey, adoption or
activation work.

## Privileged Record provider assembly

`createRecordNativeBindings` in `src/record/native-bindings.ts` composes the
published NativeHost, NativePen, NativeClipboard and Record-owned config adapters.
Trusted composition supplies the admitted config helper path/hash, the same
config path used by the running native GUI, required permission IDs from Record
policy, and a resolver for an exact authenticated Saved capture/artifact pair.
It fixes the clipboard limit to the active public producer constant. It never
starts the GUI, chooses permission policy, formats handoff text, creates a second
config writer or treats a clipboard dispatch as success. Closing this binding
only closes its own config backend.

`createRecordProviderHost` in `src/record/provider-host.ts` constructs the real
Record capability provider around those ports and the caller's published Record
runtime. Its `RecordHostServices` result plugs directly into the trusted
`createRecordHostServices` build binding. The caller transfers exclusive runtime
command/observation ownership on success and supplies an idempotent cleanup for
its owned Engine resources. Authenticated output facts remain supplied by Record;
omitting them leaves output unavailable.

The host counts queued and in-flight commands and atomically closes admission
before cleanup. Both the native ownership report (complete reconciliation,
readable storage, no unresolved IDs) and Record's authoritative capture and
shortcut-reservation facts must allow shutdown. It never derives clearance from
a renderer snapshot. Cleanup advances by named stages; uncertain provider
disposal blocks further release, and failed Engine cleanup cannot claim closure.
Observation-only client detach does not dispose the runtime or cancel capture.
Configured output-fact producers are reread before close admission; pending,
unavailable or active facts cannot be cleared by the provider's initial snapshot.
The packaged provider test uses the real public Record provider and synthetic
native/Engine producers to verify reconciliation refusal, renderer refresh and
ordered one-time cleanup through the shipped Electron bridge on both CI targets.
These assembly APIs do not select an installed executable, authenticate a Saved
artifact, enable an installed route or clear native product acceptance gates.

The pinned public Record package additionally supplies
`createRecordPublicationOutput` from `@apparent/record/publication`; its admitted
SDK peer is packaged beside it. Trusted assembly injects `output.captureFacts`
and uses `output.resolveHandoff` for the native clipboard adapter. That port owns
the existing durable publication authority; composition adds no Saved-facts store
or substitute resolver. It does not provide shortcut-reservation authority:
`nextShortcutAdmission: unavailable` still blocks host shutdown until the trusted
assembly can establish authoritative clearance. Automatic processing orchestration
and installed legacy-transition clearance remain separate prerequisites.
The trusted Record build may additionally provide `nativeArtifacts`, containing
the Linux x64 Capture/2, Media/2 and config-helper payload directories, each with
its externally admitted manifest SHA-256 and original build source SHA. Use the
canonical delivery consumer to establish provenance and receipt admission first;
the build does not turn an arbitrary local manifest into an admitted artifact.
It snapshots and validates all three complete inventories before writing their
bytes beneath `native-record/{capture,media,config}`. Undeclared files, aliases,
changed bytes, unsupported protocols/targets and altered modes are refused.
The shell and assembly receipts seal every native file and its executable mode;
packaging rechecks both. No owner source or native compiler runs during assembly.

`createRecordHostServices` receives `{ profilePath, nativeRoot }` from privileged
main. `nativeRoot` is fixed inside this packaged application, never selected by
the renderer. The binding must use absolute entries beneath it and one persistent
private Capture/2 runtime for both Capture/2 and Media/2; it retains SDK/process
shutdown ownership. A build without native artifacts retains the previous
development-shell behavior. This route supports Linux x64 only because the
Capture/2 and Media/2 producers are unavailable on Mac. It does not start those
producers automatically, select capture sources, retire legacy writers, activate
the installed route or clear the native pill/exclusion gates.

Stream manifests may additionally seal `style.css` and at most nine font files
under `fonts/` (TTF or WOFF2). These precompiled assets are copied and checked with
the stream entry during assembly and packaged verification; undeclared files,
symlinks and changed bytes are refused. The renderer loads the declared local
stylesheet before mounting the stream, and CSP permits only local fonts. This
does not grant renderer filesystem access or compile an owner's source during
assembly.

`npm run build:record -- <new-output-directory>` builds the thin Record pill
contribution from the pinned public UI package, its compiled stylesheet and font.
Assemble that artifact with a Record-bound shell using the existing assembly
command. The shell injects only its fixed Record renderer port into this module;
the module delegates all pill actions and presentation to Record's public API.
Changing shell surfaces disconnects only the renderer's observations. Record
Settings are not supplied by this contribution. The default shell remains the
development foundation; this does not enable installed routing or claim native
pill placement, focus, stacking, exclusion or capture-to-Saved behavior.
