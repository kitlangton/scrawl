# Development desktop bundle

Run `node --experimental-strip-types enablement/desktop/build-dev-app.ts --help`. The default source root is the CLI's checkout, even when invoked from another directory. The command wraps the existing `composition/app/scripts/build-omarchy.ts` builder; it is not a second assembly implementation.

Build with explicit, privately held admitted input and a new output directory:

```sh
node --experimental-strip-types enablement/desktop/build-dev-app.ts \
  --inputs /absolute/path/admitted-native-inputs.json \
  --output /absolute/path/apparent-dev-bundle
```

The result is under `OUTPUT/apparent-omarchy/`, including its launcher and packaged app. Use `--plan` to check source discovery and print the build prerequisites without starting subprocesses. If this branch does not yet contain the builder, pass `--source-root /path/to/accepted/checkout`; that source is read-only to this tool. The builder enforces its clean committed source requirement and refuses an existing output directory. Do not pass an installed app or evidence directory as output.

Requirements are Linux x64, Node 22.18+ and the builder checkout's already installed dependencies. The input JSON is the existing builder's `RecordNativeArtifacts` descriptor (`composition/app/scripts/record-native.ts`): admitted Capture/Media/config payloads plus a pinned GUI AppImage with source and byte identities. Retain the producer admission receipts; a JSON descriptor alone is not provenance. This CLI does not download or rebuild native artifacts. Inputs without the GUI are rejected as incomplete; it never substitutes a synthetic shell. Build/package success is not native desktop acceptance. The command does not install, activate or launch the bundle.

On Clanker, run an actual build through the shared pool once (do not nest wrappers):

```sh
/home/simon/.local/bin/t3-build --jobs 3 -- \
  node --experimental-strip-types enablement/desktop/build-dev-app.ts \
  --inputs /absolute/path/admitted-native-inputs.json --output /absolute/path/fresh-bundle
```

`--help` and `--plan` do not build. Source must be clean and committed: this entry currently creates development bundles of commits, not dirty-tree/hot-reload builds. Use a separate fresh output for each version; build evidence and installed applications are never replaced.

Focused contracts: `node --test enablement/desktop/build-dev-app.test.ts`.
