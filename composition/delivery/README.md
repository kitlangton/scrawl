# Value stream delivery core

Each source owner registers `delivery.json` beside its package. Schema 1 names the package, tracked input roots, npm verification scripts, build script, supported target and staging class. Public registers a Bun-built `web` directory artifact, with a dedicated deployment job consuming that verified output. Connect retains its separate producer. Missing owners are reported pending; an existing source without a registration is reported as blocked.

The isolated TypeScript architecture checker in `composition/architecture` rejects cross-owner source imports, undeclared private package paths, reversed dependencies and alias escapes. `Delivery contracts` tests the checker and scans common shared/composition code; component jobs scan their own owner before artifact admission. An unrelated product violation fails its own lane. A successful shared contract job is required before any prior component can be admitted, and checker inputs are part of every component fingerprint. Before merging owner changes, install the checker's locked dependencies and run its typecheck, tests and repository check as documented there. This guard covers static dependency boundaries, not arbitrary runtime behavior or native effects.

`engine-native` consumes `engine/delivery/native/delivery.json` independently of the SDK slot. Only that native lane installs Rust and produces the exact executable/manifest inventory through the Engine-owned producer. Its inputs include `engine/`, `capture/engine/` and inherited `.cargo/` settings. The current executable reports capture unavailable; packaging does not enable installed routing or prove screen capture.

Run with Node 22's type stripping, without installing dependencies:

```sh
node --experimental-strip-types composition/delivery/cli.ts plan
node --experimental-strip-types composition/delivery/cli.ts admit record linux-x64
node --experimental-strip-types composition/delivery/cli.ts build record linux-x64 /controlled/payload
node --experimental-strip-types composition/delivery/cli.ts seal record linux-x64 /controlled/payload /controlled/sealed "$INPUT_DIGEST"
```

`plan` validates registration and emits the owner/target matrix to `GITHUB_OUTPUT`; pending lanes are not fake green results. Package source must be covered by a declared input root. `admit` runs on a hosted runner with its `ImageOS` and `ImageVersion` identity set. It looks only at push runs on main from this workflow, with the owner job, `Delivery contracts` and runtime audit all terminal-success, an ancestor source SHA and a matching unexpired artifact. API/auth/identity uncertainty causes a rebuild. Fingerprints use Git tree mode, path and blob IDs for all owner input roots, registration, `LICENSE`, the delivery core, this workflow and runtime budget checker/action, and include Node, runner image, OS/architecture and exact target identities.

On reuse the workflow downloads the admitted artifact by its numeric ID. `reuse ID TARGET ZIP OUTPUT_DIR INPUT_DIGEST PRIOR_SHA ARTIFACT_ID PRIOR_RUN_ID PRIOR_ATTEMPT` checks the ZIP inventory, receipt identity and archive hash/size without extracting application files. It republishes the exact component bytes with a current-run receipt retaining the prior artifact/source identity. Failed retrieval or validation fails the owner job; it cannot produce a green missing component. Both fresh and reused components receive provenance attestations and a current-run artifact.

`build` runs the registered npm verification scripts and build. npm owners are packed without lifecycle scripts rerunning, installed into a fresh consumer with scripts disabled, and imported through their public package root. App-shell output copies the built app, shell and fixtures; a future `directory` owner can copy a declared package-contained artifact directory, including a native-language producer whose owner script owns its toolchain invocation. The parent workflow owns OS setup and uploads `stream-ID-TARGET-DIGEST` with `component.tar.gz` and `receipt.json`. `seal` only archives a controlled prebuilt staging directory and records source, digest, archive hash/size and Actions run identity; it does not build or publish. Main-only canonical workflow identity is required. Symlinks are allowed only when relative and contained by the staging root. App artifacts are development staging output; this protocol does not publish stable releases or create an updater.

## Public web artifacts

`public/delivery.json` registers Public's `dist-apparent` directory as a `website`
for one `web` target on Ubuntu. The common builder uses Bun's frozen lockfile and
Public's `verify:installed` entrypoint, including owner checks and a private
synthetic-enabled analytics proof. It never builds Connect. Optional `files` add
explicit content/config inputs without replacing required package-directory
coverage; optional `environment` names build variables hashed into reuse identity.
Missing optional config files are also tracked by their absence and invalidate
reuse when added. Bun/version, Node, runner and verifier inputs remain pinned.

The common workflow reuses unchanged verified artifacts using its existing admission
and receipt protocol. Website archives use ordinary USTAR regular files and
directories; receipts include source and environment digests for independent
consumer validation. Public's consumer checks canonical-main producer identity,
the exact Public job, Delivery contracts and runtime audit, source/config equality
and ZIP/tar hashes before extracting only public bytes. An unrelated failed stream
does not invalidate a successful Public job with successful shared contracts/audit.

The common workflow’s `deploy (public, web)` job needs only the dedicated Public
producer. It downloads that producer’s same-run artifact and a compatible Connect
artifact, then checks their composed
resource/privacy/browser seams, seals the final candidate and publishes under its
existing controls. It never recompiles Public or Connect. Enabled synthetic Public
analytics fixtures stay private to the producer and are never uploaded.
