# Value stream delivery core

Each source owner registers `delivery.json` beside its package. Schema 1 names the package, tracked input roots, npm verification scripts, build script, supported Linux/macOS target and staging class. Existing Connect and Public pipelines remain external. Onboarding and Agent show as pending until their source owners register; an existing source without a registration is reported as blocked.

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
