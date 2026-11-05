# Apparent Agent CLI

This owner-local TypeScript CLI inspects local `.apparent` bundles. Inspection has no Desktop, Cargo, network, or Connect dependency. The package includes the published Engine SDK v1 for an optional foreground screen-recording command; recording still requires a separately installed native Engine runtime.

```sh
npm install
npm run build
node dist/cli.js inspect /path/to/capture.apparent
node dist/cli.js inspect /path/to/capture.apparent/apparent.json --json
node dist/cli.js inspect /path/to/capture.apparent --marks
node dist/cli.js record --duration 30 --out /absolute/path/demo.apparent
```

`inspect` prints the stored `summary.md` when present; that text was authored by the capture producer and is not regenerated. `--json` writes the raw `apparent.json` bytes. `--marks` prints the stored semantic marks, `--transcript` prints transcript text, and `--images` lists safe relative contact sheet/keyframe references. Explicit bundle directories and manifest paths are supported.

`latest` reads the configured `APPARENT_CAPTURES_DIR`, then the read-only `capturesDir` in `~/.config/apparent/config.json`, then the OS default. It skips malformed, incomplete, and unsupported bundles. Manifest reads are strict UTF-8 and capped at 64 MiB. The summary mode prints producer-authored `summary.md`; if it is unavailable, the CLI gives an error with `--transcript` and `--json` recovery choices. `--marks` retains ids, timestamps, frame references, stroke counts, labels, and numbering metadata. `--images` lists safe relative references, including uncropped frames when present; it does not inspect image contents. Raw JSON preserves the manifest bytes.

Install the packed package locally with `npm install -g ./apparent-agent-0.1.0.tgz`; it provides `apparent-agent`, leaving the existing `apparent` command unchanged. Build requires Node 22.6+ and TypeScript dev dependencies. Recording reads explicit `APPARENT_ENGINE_EXECUTABLE` and `APPARENT_ENGINE_RUNTIME` absolute paths, requires a native screen-capable ready peer, then starts, polls and stops in the foreground. It ignores stdin, handles SIGINT/SIGTERM by stopping or cancelling the owned request, and waits up to 60 seconds for finalization; a timeout reports that the Engine operation may still be active. It never opts into the SDK test double. There is no recorder/media API for frames, zoom or export yet; Connect, authentication, network access, and standalone Engine runtime archives are out of scope. No native recording success is implied without that installed runtime.

The legacy Rust implementation can be removed only after all supported installed/package-manager consumers use this CLI and its inspection compatibility is accepted. Recording requires a production Engine executable and a complete native recorder/media artifact API; the currently published SDK v1 contract has no such provider.
