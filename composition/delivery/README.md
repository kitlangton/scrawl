# Value stream delivery core

Each source owner registers `delivery.json` beside its package. Schema 1 names the package, tracked input roots, verification scripts, build script, supported target and staging class. Engine SDK, native core, media and capture share `stream-engine.yml`; their component IDs, supported targets and artifact names stay separate. Each `.github/workflows/stream-<owner>.yml` owns its path-filtered PR and main jobs, permissions, fixed owner/target matrix and runtime audit. Small composite actions in `.github/actions/delivery-contracts` and `delivery-component` share repeatable steps without owning workflow jobs. Owner jobs keep the established `deliver / stream (<owner>, <target>)`, `deliver / Delivery contracts`, and `deliver / Workflow runtime budget` receipt names. PR component jobs build and verify fresh with synthetic settings and read-only permissions; main producer jobs may admit verified unchanged artifacts and then seal, attest and upload. Public's workflow is the only Pages publisher and has no Connect completion trigger or artifact dependency. Connect's TypeScript account/crypto client has a separate `connect-client` workflow; its independent static hosting package has a separate `connect-browser` workflow, while the legacy browser/Worker producer retains its explicit manual/reusable invocation contract. The client artifact does not prove the remaining Rust transfer lifecycle migrated. Missing owners are reported pending; an existing source without a registration is reported as blocked.

The isolated TypeScript architecture checker in `composition/architecture` rejects cross-owner source imports, undeclared private package paths, reversed dependencies and alias escapes. `Delivery contracts` tests the checker and scans common shared/composition code; component jobs scan their own owner before artifact admission. An unrelated product violation fails its own lane. A successful shared contract job is required before any prior component can be admitted, and checker inputs are part of every component fingerprint. Before merging owner changes, install the checker's locked dependencies and run its typecheck, tests and repository check as documented there. This guard covers static dependency boundaries, not arbitrary runtime behavior or native effects.

`engine-native` consumes `engine/delivery/native/delivery.json` independently of the SDK slot. That native lane installs Rust and produces the exact executable/manifest inventory through the Engine-owned producer. Its inputs include `engine/` and inherited `.cargo/` settings. The separate `engine-media-native` producer also fingerprints `capture/engine/` because its Linux media worker still uses that legacy implementation. Packaging does not enable installed routing.

`platform-resources` consumes `shared/resources/delivery/delivery.json` through
`stream-platform.yml`, with fixed Linux x64 and Intel Mac targets. Its
native helper remains owned by the producer and staged before the public
TypeScript adapter tests. The registration must fingerprint both
`shared/resources/` and inherited `.cargo/`; the synthetic contract rejects an
incomplete closure. This wiring does not enable or publish the helper by itself.

`engine-media-native` consumes `engine/delivery/media/delivery.json` as a separate
native `linux-x64` and `macos-x64` slot owned architecturally by Engine. Its Linux qualification installs
the GStreamer development libraries, base/good/libav plugins, and FFmpeg fixture
tool on the Linux runner before admission, including reuse hits; other owners do not
run this setup. PR and main use the same installer with isolated signed Ubuntu
archive/security sources over HTTPS, 20-second acquisition timeouts, one retry,
and process-group deadlines of 60 seconds for metadata and 180 seconds for
installation, inside the five-minute setup step. This avoids the stalled Azure
mirror without extending the job/workflow budget or weakening apt authentication.
The producer uses the installed runtime and refuses plugin-source
overrides in CI. The catalog fingerprints `engine/`, `capture/engine/` and
`.cargo/`; packaged-media verification remains inside the owner producer.
The Mac lane runs on native `macos-26-intel`, qualifies Swift 6.2+/SDK 26+,
and skips Linux apt setup. Mac qualification ensures fixture-only FFmpeg is
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
node --experimental-strip-types composition/delivery/cli.ts consume engine linux-x64 "$PRODUCER_COMMIT" /controlled/empty-consumer-dir
```

`consume ID TARGET PRODUCER_SHA [EMPTY_DESTINATION]` admits only that exact
canonical main producer commit through the existing admission rules for owner
job, contracts, runtime audit, exact artifact name/expiry and receipt identity.
The producer receipt's input digest must match that exact artifact name and its
tracked source digest must match the requested commit. It verifies the GitHub build provenance attestation for the component
archive and checks its exact producer run/attempt before caching bytes. It never
selects a different commit or starts a producer. Without a destination it prints
JSON containing the immutable component archive and receipt paths; with a
destination it stages those two files into an empty directory. Consumers remain
responsible for their owner checks and any paired component contract. No package
or native dependency is installed automatically.

The private cache is `${XDG_CACHE_HOME:-~/.cache}/apparent/delivery`, with
owner-only directories/files. Its content key includes repository, owner,
target, source commit, input digest and authenticated archive hash. Cold handoff
downloads once; warm and concurrent calls reuse the content after checking its
receipt/hash, current producer admission, tar inventory and online attestation.
Corrupt cache entries are discarded and fetched again; unavailable or failed
GitHub/attestation checks fail closed. The consumer validates archive paths and
rejects links/special entries without extracting them.

`plan` remains a local diagnostic and reports registrations; owner workflows contain checked-in target matrices, so a run cannot schedule unrelated owners. Package source must be covered by a declared input root. `admit` runs on a hosted runner with its `ImageOS` and `ImageVersion` identity set. It looks only at push runs on main from the exact owner workflow path, with the historical owner stream job, `Delivery contracts` and runtime audit all terminal-success, an ancestor source SHA and a matching unexpired artifact. API/auth/identity uncertainty causes a rebuild. Fingerprints use Git tree mode, path and blob IDs for all owner input roots, registration, `LICENSE`, the delivery core, owner workflow and shared component action, and runtime budget checker/action, and include Node, runner image, OS/architecture and exact target identities.

On reuse the workflow downloads the admitted artifact by its numeric ID. `reuse ID TARGET ZIP OUTPUT_DIR INPUT_DIGEST PRIOR_SHA ARTIFACT_ID PRIOR_RUN_ID PRIOR_ATTEMPT` checks the ZIP inventory, receipt identity and archive hash/size without extracting application files. It republishes the exact component bytes with a current-run receipt retaining the prior artifact/source identity. Failed retrieval or validation removes partial output and falls through to a fresh owner build; a failed fresh build still blocks admission. Both fresh and reused components receive provenance attestations and a current-run artifact.

`build` runs the registered npm verification scripts and build. npm owners are packed without lifecycle scripts rerunning, installed into a fresh consumer with scripts disabled, and checked through every public export. That all-exports verifier explicitly provisions declared optional host peers (such as React for UI entrypoints) inside its disposable consumer only; they are not added to the staged library artifact or made mandatory for headless installations. Owner checks still verify their separate headless and UI profiles. App-shell output copies the built app, shell and fixtures; a future `directory` owner can copy a declared package-contained artifact directory, including a native-language producer whose owner script owns its toolchain invocation. The parent workflow owns OS setup and uploads `stream-ID-TARGET-DIGEST` with `component.tar.gz` and `receipt.json`. `seal` only archives a controlled prebuilt staging directory and records source, digest, archive hash/size and Actions run identity; it does not build or publish. Main-only canonical workflow identity is required. Symlinks are allowed only when relative and contained by the staging root. App artifacts are development staging output; this protocol does not publish stable releases or create an updater.

## PR verification reuse

The Engine SDK Linux lane, native Engine/media Linux/Intel Mac lanes and App Shell
Linux/Intel Mac lanes, Config Linux/Intel Mac lanes and Onboarding Linux/Intel Mac
lanes retain seven-day PR candidates after their fresh checks and build. Main jobs first try existing verified-main reuse, then a same-repository
merged PR candidate. Admission authenticates the merged PR, exact workflow/run and
successful attempt jobs (including contracts and runtime audit), then checks the
synthetic merge commit's PR head parent and authenticates its base as an ancestor
of the landed main commit's first parent. It fetches the tested commit when absent
and recomputes its complete registered component source closure against main.
Unrelated main changes may differ; every tracked component input must match. Component/environment/Node/runner-image digests and
archive hash/size/run-attempt checks still apply. Forks, missing metadata/commits, divergent bases, changed
component inputs, expired artifacts and failed retrieval/validation fall back to fresh work.
The run API's head SHA identifies the PR head, not its tested synthetic merge.

Candidates use separate `pr-ID-TARGET-MERGE_SHA-DIGEST` artifact names;
retention is not publication authority. Main reissues an exact-main receipt with
original-run provenance, then attests/uploads through the existing path. Main job
summaries distinguish reuse from fresh verification. No workflows, skipped jobs or
cancellation rules are added. Other owner lanes retain their existing
behavior; production configuration and platform/native acceptance remain required.
These Engine exceptions supersede the general statement that PR jobs do not archive
artifacts. Component matching includes transitive registered sources, locks/vendor archives,
shared delivery/architecture code, workflow/actions, license and verification
inputs. App Shell also tracks the external process-cleanup helper and legacy
workflow its tests read; changes select its platform jobs. Missing proof falls
back to fresh verification. Receipt validation independently checks source identity.

Native Engine and media admission install Rust before fingerprinting on PRs and main,
including reuse hits. Its digest additionally covers actual rustc/Cargo/C compiler
versions, macOS SDK/Clang where applicable, and compiler/linker environment
overrides. Empty/missing identities and unknown Rust compiler wrappers fail closed.
The archived native manifest retains its original build source SHA. The outer
receipt records the admitted main SHA in `sourceSha` and the unchanged payload
build SHA in `buildSourceSha`, plus original-run provenance; promotion never
rewrites native bytes or their embedded manifest. Prior fingerprints lacking this
identity rebuild once. Native runtime, GUI routes and deployment authority do not
change; existing native producer checks run fresh on both supported PR targets.

App Shell candidates are archived only after the exact packaged Electron app
passes the existing platform tests. Promotion retains the complete package and
its archive hash; it does not repackage or change installed routing, bridge
contracts, Electron versions or package layout. Its fingerprint additionally
covers Electron/npm overrides and Node options/environment; locked Electron and
packager inputs remain in the source closure. The original package source is
retained in `buildSourceSha`. The shared TypeScript allowlist selects retention,
sealing and admission consistently for enabled owner/target pairs. Both existing
platform PR lanes keep their fresh checks; main authenticates only its own owner
workflow and target. No new Desktop GUI/native behavior is introduced.

Media qualifies its dependencies before admission even on a reuse hit: the
existing bounded signed Ubuntu installer on Linux, and existing Swift/SDK/FFmpeg
qualification on Intel Mac. The media digest adds the installed Linux package
versions, GStreamer version/link flags and glibc, or Mac Swift and OS version,
plus FFmpeg and relevant media loader/pkg-config overrides. Version or override
changes invalidate reuse; missing identity fails closed. Qualification setup is
retained, while compilation and packaged media tests are reused from the exact
successful PR archive. Both platform PR producers continue their real staged
H264/SDK checks. Embedded build manifests and all archived runtime bytes remain
unchanged; main retains original build provenance. Capture is not enabled for PR
reuse by this media slice.

Config uses the same actual native compiler/SDK/flag identity and conservative
component-input admission on both supported targets. Supported native candidates
install Rust before admission; unsupported native lanes keep fresh-only setup.
The Config helper, its embedded source SHA and protocol manifest are promoted
byte-for-byte with original build provenance. Its workflow retains fresh staged
helper/client integration tests on PRs. No config writer, permission, activation,
installed routing or native-helper implementation changes in this reuse slice.

Onboarding promotes the exact packed TypeScript library only after its existing
fresh checks/build and disposable public-export consumer verification. Its
committed vendor tarballs and lockfile remain the complete package dependency
inputs. Node/npm/esbuild overrides join its fingerprint; Electron-only overrides
remain App Shell-specific. Shared package-build identity also captures esbuild
overrides for App Shell. Onboarding gains no Rust dependency or native setup, and
this delivery change does not activate or alter any installed onboarding route.

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


Platform config, resources and App Shell share `stream-platform.yml`. One contracts
job selects changed components from the full PR merge-base diff or main push
before/after diff, including deleted files and both rename sides. Missing history
selects all; shared delivery/architecture inputs select all; unrelated inputs
select none. Registrations supply each selected component's target matrix.
Main concurrency locks belong to each selected component/target, so an unrelated
push cannot replace another component's pending producer.
Native producers and the TypeScript shell remain isolated jobs with unchanged
verification, packaging and artifact IDs. The audit requires successful contracts
and selected execution before a receipt can authorize reuse. A failure in any
selected component therefore blocks the shared audit and candidate reuse for that
run; component jobs still run independently with fail-fast disabled.
The new canonical workflow path participates in fingerprints and admission;
old workflow receipts are not promoted across the migration and the first new
producer run builds fresh. Broad manual platform verification remains separate.
