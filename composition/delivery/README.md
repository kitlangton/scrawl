# Value stream delivery core

Each source owner registers `delivery.json` beside its package. Schema 1 names the package, tracked input roots, verification scripts, build script, supported target and staging class. Engine SDK, native core, media and capture share `stream-engine.yml`; their component IDs, supported targets and artifact names stay separate. Each `.github/workflows/stream-<owner>.yml` owns its path-filtered PR and main jobs, permissions, fixed owner/target matrix and runtime audit. Small composite actions in `.github/actions/delivery-contracts` and `delivery-component` share repeatable steps without owning workflow jobs. Owner jobs keep the established `deliver / stream (<owner>, <target>)`, `deliver / Delivery contracts`, and `deliver / Workflow runtime budget` receipt names. PR component jobs build and verify fresh with synthetic settings and read-only permissions; main producer jobs may admit verified unchanged artifacts and then seal, attest and upload. Public's workflow is the only Pages publisher and has no Connect completion trigger or artifact dependency. Connect's TypeScript account/crypto client has a separate `connect-client` workflow; its independent static hosting package has a separate `connect-browser` workflow, while the legacy browser/Worker producer retains its explicit manual/reusable invocation contract. The client artifact does not prove the remaining Rust transfer lifecycle migrated. Missing owners are reported pending; an existing source without a registration is reported as blocked.

The isolated TypeScript architecture checker in `composition/architecture` rejects cross-owner source imports, undeclared private package paths, reversed dependencies and alias escapes. `Delivery contracts` tests the checker and scans common shared/composition code; component jobs scan their own owner before artifact admission. An unrelated product violation fails its own lane. A successful shared contract job is required before any prior component can be admitted, and checker inputs are part of every component fingerprint. Before merging owner changes, install the checker's locked dependencies and run its typecheck, tests and repository check as documented there. This guard covers static dependency boundaries, not arbitrary runtime behavior or native effects.

`engine-native` consumes `engine/delivery/native/delivery.json` independently of the SDK slot. That native lane installs Rust and produces the exact executable/manifest inventory through the Engine-owned producer. Its inputs include `engine/` and inherited `.cargo/` settings. The separate `engine-media-native` producer also fingerprints `capture/engine/` because its Linux media worker still uses that legacy implementation. Packaging does not enable installed routing.

`platform-resources` consumes `shared/resources/delivery/delivery.json` through
`stream-platform-resources.yml`, with fixed Linux x64 and Intel Mac targets. Its
native helper remains owned by the producer and staged before the public
TypeScript adapter tests. The registration must fingerprint both
`shared/resources/` and inherited `.cargo/`; the synthetic contract rejects an
incomplete closure. This wiring does not enable or publish the helper by itself.

`engine-media-native` consumes `engine/delivery/media/delivery.json` as a separate
native `linux-x64` and `macos-x64` slot owned architecturally by Engine. Its Linux fresh build installs
the GStreamer development libraries, base/good/libav plugins, and FFmpeg fixture
tool on the Linux runner; this setup is skipped for successful artifact reuse and every
other owner. PR and main use the same installer with isolated signed Ubuntu
archive/security sources over HTTPS, 20-second acquisition timeouts, one retry,
and process-group deadlines of 60 seconds for metadata and 180 seconds for
installation, inside the five-minute setup step. This avoids the stalled Azure
mirror without extending the job/workflow budget or weakening apt authentication.
The producer uses the installed runtime and refuses plugin-source
overrides in CI. The catalog fingerprints `engine/`, `capture/engine/` and
`.cargo/`; packaged-media verification remains inside the owner producer.
The Mac lane runs on native `macos-26-intel`, qualifies Swift 6.2+/SDK 26+,
and skips Linux apt setup. Fresh Mac builds ensure fixture-only FFmpeg is
available, using a bounded Homebrew bottle install when absent. Its producer builds thin x86_64 Rust and Swift helpers
and runs packaged public SDK/H264 tests. Receipts and discovery retain the same
owner ID with a distinct target fingerprint. The compilation minimum of macOS 15
is not runtime proof on macOS 15; ARM proof is not Intel artifact admission.

`connect-client` consumes `connect/delivery.json` and builds only
`connect/delivery/client/`. Its path-filtered entrypoint does not schedule the
Connect browser/Worker producer, which retains separate manual/reusable
invocation, transfer lifecycle and artifact identities.

Pull requests targeting main run contracts and fresh checks/builds only for each
changed registered owner/target. The owner entrypoints carry the same path filters
for PRs and main pushes and execute their fixed target matrices directly. PR jobs are read-only, use synthetic analytics settings, and do not
reuse, seal, attest or publish artifacts. Main keeps its existing producer and
deployment lanes.

Run with Node 22's type stripping, without installing dependencies:

```sh
node --experimental-strip-types composition/delivery/cli.ts plan
node --experimental-strip-types composition/delivery/cli.ts admit record linux-x64
node --experimental-strip-types composition/delivery/cli.ts build record linux-x64 /controlled/payload
node --experimental-strip-types composition/delivery/cli.ts seal record linux-x64 /controlled/payload /controlled/sealed "$INPUT_DIGEST"
```

`plan` remains a local diagnostic and reports registrations; owner workflows contain checked-in target matrices, so a run cannot schedule unrelated owners. Package source must be covered by a declared input root. `admit` runs on a hosted runner with its `ImageOS` and `ImageVersion` identity set. It looks only at push runs on main from the exact owner workflow path, with the historical owner stream job, `Delivery contracts` and runtime audit all terminal-success, an ancestor source SHA and a matching unexpired artifact. API/auth/identity uncertainty causes a rebuild. Fingerprints use Git tree mode, path and blob IDs for all owner input roots, registration, `LICENSE`, the delivery core, owner workflow and shared component action, and runtime budget checker/action, and include Node, runner image, OS/architecture and exact target identities.

On reuse the workflow downloads the admitted artifact by its numeric ID. `reuse ID TARGET ZIP OUTPUT_DIR INPUT_DIGEST PRIOR_SHA ARTIFACT_ID PRIOR_RUN_ID PRIOR_ATTEMPT` checks the ZIP inventory, receipt identity and archive hash/size without extracting application files. It republishes the exact component bytes with a current-run receipt retaining the prior artifact/source identity. Failed retrieval or validation removes partial output and falls through to a fresh owner build; a failed fresh build still blocks admission. Both fresh and reused components receive provenance attestations and a current-run artifact.

`build` runs the registered npm verification scripts and build. npm owners are packed without lifecycle scripts rerunning, installed into a fresh consumer with scripts disabled, and checked through every public export. That all-exports verifier explicitly provisions declared optional host peers (such as React for UI entrypoints) inside its disposable consumer only; they are not added to the staged library artifact or made mandatory for headless installations. Owner checks still verify their separate headless and UI profiles. App-shell output copies the built app, shell and fixtures; a future `directory` owner can copy a declared package-contained artifact directory, including a native-language producer whose owner script owns its toolchain invocation. The parent workflow owns OS setup and uploads `stream-ID-TARGET-DIGEST` with `component.tar.gz` and `receipt.json`. `seal` only archives a controlled prebuilt staging directory and records source, digest, archive hash/size and Actions run identity; it does not build or publish. Main-only canonical workflow identity is required. Symlinks are allowed only when relative and contained by the staging root. App artifacts are development staging output; this protocol does not publish stable releases or create an updater.

## Public web artifacts

`public/delivery.json` registers Public's `dist-apparent` directory as a `website`
for one `web` target on Ubuntu. The reusable builder uses Bun's frozen lockfile and
Public's `verify:installed` entrypoint, including owner checks and a private
synthetic-enabled analytics proof. It never builds Connect. Optional `files` add
explicit content/config inputs without replacing required package-directory
coverage; optional `environment` names build variables hashed into reuse identity.
Missing optional config files are also tracked by their absence and invalidate
reuse when added. Bun/version, Node, runner and verifier inputs remain pinned.

The owner workflow reuses unchanged verified artifacts using its existing admission
and receipt protocol. Website archives use ordinary USTAR regular files and
directories; receipts include source and environment digests for independent
consumer validation. Public's consumer checks canonical-main producer identity,
the exact Public job, Delivery contracts and runtime audit, source/config equality
and ZIP/tar hashes before extracting only public bytes. An unrelated failed stream
does not invalidate a successful Public job with successful shared contracts/audit.

The Public owner workflow’s `deploy (public, web)` job needs only its dedicated Public
producer. It validates the same-run component, Public resource graph, metadata,
browser/theme/movie behavior and byte preservation, then seals and publishes only
Public files. It has no Connect artifact, browser, API-origin or completion-trigger
dependency. Connect pages, API/auth/viewer resources and path routing belong to its
independent Worker. Existing Pages activation, environment, source ordering, readback
and publication lock remain in force. The Connect owner has verified the live route/auth/privacy cutover. Retain
its app routes when publishing or rolling back the Public-only snapshot.


PR readiness is established from the affected owner workflows directly. Each owner
retains its own checks, builds, permissions and publication gates.

App Shell's Linux Electron prerequisite setup uses its locked Playwright Chromium
package list through the same isolated signed HTTPS Ubuntu installer. The complete
installer process group has a 240-second deadline; acquisition timeouts and one
retry bound mirror stalls without changing tests or workflow budgets. Architecture
checker installs retain `npm ci` and integrity checks, with a lockfile-keyed npm
download cache and offline preference; cached packages do not replace fresh checks.

Capture/2 is independently discoverable as `engine-capture-native` from
`engine/delivery/capture/delivery.json`, Linux x64 only. Its `engine/` and `.cargo/`
inputs include the standalone Cargo manifests/lock, Core and native launcher;
mandatory delivery/architecture/workflow inputs are added by admission. The CI
capture dependency profile adds X11 and H264 encoder/parser plugins to the bounded
Ubuntu installer. Existing `engine-native` and `engine-media-native` retain their
own routes. Select the absolute manifest launch entry and its 40-second shutdown
grace; registration does not activate installed routing or certify unsupported
Wayland, macOS, audio, drawing or publication capabilities.
