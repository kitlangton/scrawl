# Apparent Agent CLI

This owner-local TypeScript CLI inspects local `.apparent` bundles and selects video frames through the published Engine media SDK. Inspection has no Desktop, Cargo, network, or Connect dependency. Recording and frame decoding require separately installed native Engine providers. See [CLI parity inventory](docs/cli-parity.md) for command ownership, status and dependencies.

```sh
npm ci
npm run build
node dist/cli.js inspect /path/to/capture.apparent
node dist/cli.js inspect /path/to/capture.apparent/apparent.json --json
node dist/cli.js inspect /path/to/capture.apparent --marks
node dist/cli.js frames /path/to/capture.apparent --from 0:04 --to 0:06 --fps 4
node dist/cli.js zoom /path/to/capture.apparent --at 'width changes' --frames 12
node dist/cli.js demo
node dist/cli.js demo --out '/tmp/my synthetic demo.apparent'
node dist/cli.js zoom latest --mark m2 --frame 2 --full
node dist/cli.js skill
node dist/cli.js record --duration 30 --out /absolute/path/demo.apparent
node dist/cli.js record --protocol capture2 --duration 30 --out /absolute/path/demo.apparent --source <discovered-source-id>
node dist/cli.js capture --protocol capture2 inspect <request-id>
node dist/cli.js issue --title 'Short title' --body 'Exact Markdown description'
node dist/cli.js issue --title 'Short title' --body-file './description.md' --submit
```

`issue` prints the exact repository, title and body as JSON without contacting GitHub by default. `--submit` is the only network path and uses the installed `gh` login, pinned to `github.com` and `tryapparent/apparent`; it reads the new issue back and checks its identity and text before returning success. Search existing issues and show the full preview to the human for approval before adding `--submit`. It collects no captures, attachments, config or diagnostics. Submission is not idempotent: inspect the known issue URL/repository before retrying after an ambiguous failure. The standalone approval skill is bundled at `skills/apparent-issue/SKILL.md`.

`apparent-agent skill` prints the exact standalone capture-reading guide bundled with the package. The guide documents only this package's local inspection behavior; it does not route or replace the legacy `apparent skill` command.

`apparent-agent demo [--out FRESH.apparent] [--json]` creates a private, local illustrative bundle with annotated PNG keyframes, contact sheet, semantic marks, clicks, timed synthetic narration and summary. It reads no screen, window, session, microphone or config secrets. By default it writes a fresh randomized `.apparent` directory under the configured captures directory (read-only `capturesDir`, or the OS default); `--out` requires an absolute fresh path. Existing destinations and symlink parents fail without overwrite. JSON output contains the summary and absolute image paths. `--clipboard` is recognized but fails before writing because no native clipboard route is published. The imagery and every event/text are synthetic; this command does not create a recording, audio/video file, or native artifact.

Synthetic demo generation owns its fresh private output directory until the manifest is published. Use a parent directory controlled by your user or root; other same-user processes must not mutate that directory during generation. File identity checks preserve detected foreign replacements, but this local workflow is not an isolation boundary against another process with the same user identity.

`inspect` generates semantic text from the validated manifest, including context, notes, timed transcript, marks/events and audio-sample, transcript, image-quality and crop warnings. It regenerates output even when a producer `summary.md` is missing or stale; `--stored-summary` explicitly prints that producer-authored text. `--experiment agentContext` selects the richer event/mark timeline; `experiments.agentContext` in the read-only config can enable it by default, and the explicit flag overrides a configured `false`. The default is off. `--stored-summary` conflicts with this formatter because stored producer text cannot apply it. `--json` writes the raw `apparent.json` bytes unchanged. `--marks` prints semantic mark lines (`--marks --json` returns their structured fields), `--transcript` prints transcript text, and `--images` lists safe relative contact sheet/keyframe references. Explicit bundle directories and manifest paths are supported.

`latest` reads the configured `APPARENT_CAPTURES_DIR`, then the read-only `capturesDir` in `~/.config/apparent/config.json`, then the OS default. It skips malformed, incomplete, and unsupported bundles. Manifest and explicit stored-summary reads are strict UTF-8, capped at 64 MiB, and reject symlink files. Invalid relative config values are skipped; a valid explicitly selected but unavailable capture directory produces a read error instead of creating storage or silently selecting another recording. Linux defaults honor XDG_DATA_HOME. `--marks` retains ids, timestamps, frame references, stroke counts, labels, and numbering metadata. `--images` lists safe relative references, including uncropped frames when present; it does not inspect image contents. Raw JSON preserves the manifest bytes.

Install the packed package locally with `npm install -g ./apparent-agent-0.1.0.tgz`; it provides `apparent-agent`, leaving the existing `apparent` command unchanged. Build requires Node 22.13+ and TypeScript dev dependencies. The package bundles the authenticated compiled `@apparent/record@0.1.0` and `@apparent/engine-sdk@0.3.2` tarballs. Agent install, build, typecheck, tests and packing compile only Agent source and work from an Agent-only checkout; the SDK's public `/testing` subpaths remain fixtures only. This proves TypeScript source/build isolation, not native recording or media availability, which still require their separately installed Engine provider. Each vendored tarball's receipt and provenance identity are recorded beside it under `vendor/`.

The original `record --duration --out` form retains its V1 provider route. Capture/2 is selected only with `record --protocol capture2`; it requires `APPARENT_ENGINE_CAPTURE_EXECUTABLE`, `APPARENT_ENGINE_RUNTIME`, an exact `--source` ID from current discovery, and a normalized absolute `--out`. Use `capture --protocol capture2 sources [--json]` for read-only public SDK discovery; it prints exact IDs/kind/geometry or validated descriptors and never creates Record authority or starts capture. It sends `{audio: "none", localOnly: true}` and never retries through V1. Record owns the durable request mapping, opaque bundle identity and exact destination reservation. Agent retains only versioned public request identity/path metadata under its private control root and prints the public request ID to stderr before start, so `apparent-agent capture --protocol capture2 inspect|stop|cancel <request-id>` can reconnect with the same identity. The requested duration begins after observed recording readiness; startup has a 30-second deadline. SIGINT/SIGTERM request cancellation before recording or stop the observed recording; they do not prove cleanup. The published Capture2Host phases end at `materials-ready`, `processing`, `cancelled`, `failed`, or `unknown`; it exposes no `Saved` phase. Agent therefore always returns a pending/unavailable outcome for materials-ready and processing, even if the receipt contains publication-shaped metadata. No output path or zero exit is inferred from that metadata. A standard Saved terminal outcome remains a producer requirement before Agent can report completion. A separately staged Linux x64 native provider is available for explicit X11 display capture; it is not installed or activated by default, advertises only X11 display sources and half-area selection, and supports silent local capture only. This qualifies the explicit Capture/2 consumer on Linux X11; it does not establish Wayland, macOS, context facts, strokes, transcript, Saved processing, publication, or default CLI routing. V1 recovery remains `apparent-agent capture inspect|stop|cancel <capture-id>` and keeps its existing token journal; capture/2 uses the same command family with explicit protocol selection and no Agent V1 authority journal.

The legacy Rust implementation can be removed only after all supported installed/package-manager consumers use this CLI and its inspection compatibility is accepted. V1 recording still requires a production Engine executable and complete native recorder/media artifact API. Explicit Capture/2 can use a separately staged Linux X11 provider when configured; it retains native materials but does not produce a Saved bundle or complete recording parity.

`frames BUNDLE --from TIME --to TIME [--fps 4] [--json]` selects up to 48 timestamps (1–240 fps). With an installed provider it requests actual decoded frames through `@apparent/engine-sdk/media`; output goes to a fresh job-owned directory under `APPARENT_ENGINE_RUNTIME`. Without provider paths, or for a point range (`from == to`), it can return already-saved manifest keyframes when their timestamps fall within an inclusive range plus half-frame margin. Saved mode only accepts regular PNG-signature files under `frames/`; it preserves manifest timestamps and never describes them as decoded video frames. JSON tags `mode` and `source`; plain output remains paths and stderr identifies saved mode. If no saved images match while the provider is unavailable, it reports that video extraction is unavailable rather than implying success. A configured provider's decode, failed-job or uncertain-job errors remain authoritative; they do not fall back to saved images. Both modes leave the bundle and config unchanged; provider decode writes its owned job output under Engine runtime, while saved mode is read-only. The package does not search PATH, build Engine, decode video itself, open images, or send/network anything.

`zoom [BUNDLE|latest] (--at TIME_OR_PHRASE | --mark NUMBER_OR_ID) [--frame N] [--span S] [--frames N] [--full] [--out DIR] [--json]` creates a real PNG contact sheet plus `zoom.json` in a new private output directory. `--at` accepts `0:12`, seconds such as `12.5`, or narration: it uses the first matching timed-word phrase, then the first transcript-segment fallback. Moment windows lean ¼ of the span before the point; mark windows are centered and include the entire mark plus 500 ms where possible. Sampling includes both endpoints (up to 48 cells). Mark numbers follow `inspect --marks`; use `--frame` to resolve a number shared by keyframes. The default output is a fresh directory under `~/.cache/apparent/agent/zoom`; `--out` must name a fresh absolute directory. The sheet uses row-major image cells without captions; companion JSON keeps absolute source paths, cell timestamps, crop/full-frame source mapping, relevant marks/interactions, and timed narration. Pictures stay visual and text stays semantic. For decoded video frames, timed stroke ink, pins and laser trails use each stroke's optional sRGB `rgb: [r, g, b]` byte triplet; absent `rgb` retains the established style colors. Malformed present values fail before frame decoding or output creation.

With an explicitly configured native Engine media provider, zoom requests the exact ordered timestamps through `@apparent/engine-sdk/media`; decode errors and uncertain jobs remain errors. A missing source video is eligible for saved-frame fallback even when provider paths are configured; pending/evidence-only state and actual provider failures remain authoritative. Without configured provider paths, or when the provider reports no decode capability, it composes only retained PNG keyframes inside the requested window and labels the result `saved-keyframes` / `bundle-manifest`. Their timestamps are original manifest keyframe times, so saved mode is sparse and does not create dense video samples. The package includes pure-JS `pngjs` for PNG decoding and sheet encoding; this image-PNG work is separate from native video decoding. Linux x64 native decode can use the complete admitted media runtime through the explicit offline installer below. macOS native media decode remains unavailable. The `zoom` command does not install or activate a provider, and there is no source-build/FFmpeg fallback. Saved keyframes already contain Engine annotations and are not cropped or annotated a second time unless an exact matching retained full-frame image is available. Zoom renders actual timed stroke ink, pins, laser trails, touch trails and nearby click context for provider-decoded frames; mark shapes and flag intent remain semantic in companion JSON. Composition preflights PNG structure, rejects interlacing, bounds input files, dimensions, aggregate bytes/pixels, annotation work and cell count, checks descriptor/path identity, and rejects output collisions. No image is opened or sent automatically. The SDK test double is rejected by the production command.

`delivery.json` registers Agent with the independent value-stream producer. Its inputs track the Agent owner, including its vendored SDK artifact and lockfile; sibling Engine SDK source, Engine Rust/native implementation and Desktop are not producer inputs. The Linux producer builds/tests/packs and verifies a fresh public API import. This stages a reusable npm artifact; it does not publish to npm or switch the installed app CLI. The package root exports application helpers without running a CLI. Agent's Capture/2 launch supplies the admitted provider's 40-second shutdown bound for native finalization; close remains bounded and an unconfirmed exit is reported as uncertain with the request identity retained.

Agent-only TypeScript, JSON, Markdown and package-control changes run the installed owner suite without legacy Rust compilation. Engine SDK changes also run Agent tests but retain conservative native selection; unknown/native Agent file types, mixed shared changes and unknown history remain conservative. Hooks never install dependencies.
# Offline local installation with native Linux media

`apparent-agent-install-local` installs an already-built Agent npm tarball and
an already-built complete Engine media root into a fresh private Linux x64
prefix. It performs no download, authentication, build, global npm install or
OS routing change. The two SHA-256 values must come from a trusted upstream
admission or receipt; a native manifest hash proves integrity only, not who
produced it. The installer checks the package hash before npm, validates the
entire native inventory against its manifest before and after copying, and
requires the installed Agent 0.1.0 and a stable SDK 0.2.x or 0.3.x media export.
It accepts plain patch releases in those lines; new minor lines and prereleases are refused.
The media/1 handshake remains mandatory. SDK 0.3's
capture/2 interface does not enable native recording in this installer. Node 22.13+ and
an existing npm are required.

```sh
node /absolute/path/to/extracted-agent-package/dist/install-local.js \
  --agent /absolute/path/apparent-agent-0.1.0.tgz \
  --agent-sha256 c9e87015dee2ff2c35dcd72bc4627733e21b427e468318888ed29beafca4f3aa \
  --native-root /absolute/path/to/native-payload \
  --manifest-sha256 98ef787ed53c430e074484de806bd4de4b08893aba21fb1c34ab36f238a6da20 \
  --prefix /absolute/fresh/private-prefix
/absolute/fresh/private-prefix/apparent-agent --version
/absolute/fresh/private-prefix/apparent-agent frames /absolute/capture.apparent --from 0 --to 2
```

The example digests identify the locally retained 463f0053908 artifact pair;
they do not bypass the normal current-tree admission/reuse gate. The launcher
keeps both native entry and worker in the copied tree, supplies only the
explicit media executable and private runtime paths, and invokes the installed
Agent CLI. Install success requires a native media/1 handshake and confirmed
worker exit. This is not codec proof. Native media decode is available here
only for Linux x64; Mac media, capture, export and inference remain unavailable.
Connect is deferred. Standalone Node CLI inspection remains portable without
the native tree. A failure retains a clearly named private partial prefix and
never emits a success receipt. Paths are private to cooperating same-UID
processes; this installer does not promise same-UID isolation.
