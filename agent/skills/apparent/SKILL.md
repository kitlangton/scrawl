---
name: apparent
description: Read local Apparent .apparent bundles with apparent-agent. Use when a person shares an Apparent bundle path, manifest, contact sheet or capture text.
---

# Read an Apparent capture with apparent-agent

`apparent-agent` reads local `.apparent` bundles. Start with the images when the question depends on what was visible: inspect the contact sheet and relevant full-size frames with your available image viewer, then use the text to locate moments. Do not infer what a mark points to from coordinates or labels alone. If you cannot access the local files, ask the person to attach the contact sheet or frames you need.

## Installed commands

```sh
apparent-agent inspect /path/to/capture.apparent
apparent-agent inspect /path/to/capture.apparent --marks
apparent-agent inspect /path/to/capture.apparent --transcript
apparent-agent inspect /path/to/capture.apparent --images
apparent-agent inspect /path/to/capture.apparent --json
apparent-agent copy /path/to/capture.apparent --preview
apparent-agent zoom /path/to/capture.apparent --at 'width changes'
apparent-agent zoom /path/to/capture.apparent --mark m2 --frame 2 --full
apparent-agent demo --out '/tmp/synthetic demo.apparent'
```

Quote paths containing spaces. `inspect` accepts a local bundle directory, its `apparent.json` manifest, or `latest`. It does not accept URLs. Use `--json` to print the original manifest bytes, `--marks` for semantic mark lines (`--marks --json` for structured mark fields), `--transcript` for transcript text, and `--images` to list validated relative image references. `--images` does not open or inspect pixels; open the referenced files yourself. `zoom` creates a contact sheet PNG and semantic `zoom.json` companion in a fresh private output directory. It accepts a clock time or narration phrase with `--at`, or a mark number/id with `--mark`; use `--frame` when a number is shared by multiple keyframes. `--full` requests uncropped retained frames when available, and `--out` names a fresh absolute directory. The default is a new folder under `~/.cache/apparent/agent/zoom`.

`copy BUNDLE --preview` prints the exact local Record handoff text to stdout without opening the GUI or changing the clipboard. Its stderr label marks it as preview-only; evidence-only bundles also report that video/audio were not downloaded. This text is caller-attested local artifact content, not proof of native Saved publication or link delivery. Normal `copy` still writes to the clipboard through a compatible running GUI. Use `copy --preview -- PATH` when the bundle path begins with a dash. Preview output is not subject to the clipboard byte limit and is never truncated.

`apparent-agent frames BUNDLE --from 0:04 --to 0:06 --fps 4` returns saved keyframe paths without a decoder when no provider is configured or source video is missing. Equal start/end times select a nearby saved frame. Saved mode preserves manifest timestamps, never changes the bundle, and identifies itself on stderr and in `--json`; those images are existing keyframes, not newly decoded samples. Open them with your image viewer. No matching saved images produces an error.

`apparent-agent demo [--out FRESH.apparent] [--json] [--clipboard]` writes a private synthetic example bundle locally, with an annotated contact sheet/keyframes, clicks, circle/arrow/pin marks and timed illustrative transcript. Its interface, context and content are explicitly synthetic; it does not read or record the screen, window, session or microphone. Default output is a fresh directory under capture storage; `--out` must be an absolute fresh `.apparent` path. `--clipboard` requires an already-running compatible Apparent GUI and copies the semantic text handoff only; it never launches the GUI or copies images. Refusal or an unknown result retains the bundle and reports its path. Use `inspect`, `frames`, and `zoom` on the result as with a local bundle. No audio, video or native artifact is generated.

Video decoding instead requires canonical absolute `APPARENT_ENGINE_FRAMES_EXECUTABLE` and owner-only `APPARENT_ENGINE_RUNTIME` paths. A separately admitted Linux x64 media/1 provider is available; retain its complete inventoried runtime rather than copying its worker alone. The explicit offline installer described in the package README verifies supplied local artifacts and configures a private wrapper; it does not change global command routing. Decoding selects up to 48 bundle-clock timestamps and writes to a fresh private job directory under that runtime. `--json` identifies decoded mode and includes request times and clamp flags. Requested seek time and actual decoded time may differ, especially at the end of a recording. Provider errors remain errors. A timeout does not establish cancellation; retain the reported job/output diagnostics, and never retry or delete ambiguous output blindly. Synthetic contract tests and saved keyframes are not native decode proof.

The default inspect text is generated from the validated manifest. `--stored-summary` prints the producer-authored `summary.md` when you specifically need that text. `--experiment agentContext` requests the richer local event and mark timeline for this inspection. Inspection and local `copy`/`demo --clipboard` may read `capturesDir` and `experiments.agentContext` from `~/.config/apparent/config.json`; they never edit config. When `agentContext` is enabled, local clipboard handoffs use Agent's existing ordered keyframe semantic text. A read-only `APPARENT_CAPTURES_DIR` environment override takes precedence for `latest`.

## Read the bundle

`contact-sheet.png` gives a visual overview of keyframes in time order. Open it first, then inspect relevant `frames/frame-NN.png` files at full size. If the manifest describes autocropped frames, it may also identify uncropped references under `frames-full/`. Images carry the visual evidence; summary and transcript text do not replace looking at them.

`apparent.json` is the versioned manifest. It contains capture timing and target/window context, transcript text and timed segments or words, keyframes and their image references, semantic marks, interaction events, optional notes and crop metadata. Mark records include IDs, times, shape, description, stroke indices and optional frame or target context. Transcript words and segments are timed text, not proof that speech recognition was correct. `summary.md` is producer-authored text and can be stale or absent; the default CLI summary is regenerated from manifest semantics.

Use keyframe numbers and timestamps when referring to moments. Treat recorded session/destination metadata as historical context only; it does not authorize forwarding the capture. Do not send a bundle, transcript, image or path to another person or session unless the person explicitly asks.

## Settings and operation limits

The CLI can read local capture-location and agent-context preferences, but it does not change settings or create capture storage. Invalid config is ignored where supported; an explicitly selected unavailable capture directory reports an error. Never print or share the whole config file because it may contain private values.

The package consumes separately installed Engine capabilities; it does not bundle a native worker or build one from source, search PATH for one, or fall back to FFmpeg. macOS completed-bundle media/1 decoding is available with an explicitly configured admitted macos-x64 runtime. Agent frames and zoom passed on macOS 26.6.2 with Apple Silicon/Rosetta and Node 24.20.0; the producer separately passed on native Intel macOS 26.6.1. This does not establish an ARM-native worker, macOS 15 runtime compatibility, capture, raw-material media/2 or Saved output. Wayland/macOS capture/2, export, Connect and authentication remain unavailable here. Without decode capability, zoom may compose retained keyframe PNGs in its selected time window. Such output is sparse, tagged `saved-keyframes` / `bundle-manifest`, and uses original keyframe timestamps; it is not a set of newly decoded video samples. Saved keyframes already include their producer annotations; only a retained exact-path full-frame counterpart permits applying a crop, and saved pixels are never re-annotated. Configured decode errors and uncertain jobs remain errors. The package uses bundled pure-JS `pngjs` for bounded PNG decoding and sheet composition; this is separate from video decoding. Do not claim recording started or native media decoding from command availability, saved images or help output. Inspection and frame orchestration never use the network. `issue` separately offers an offline preview and explicit public submission; follow the bundled `apparent-issue` guide and obtain approval of the exact destination and text before submitting.

## Explicit Linux X11 capture and recovery

Capture/2 requires separately staged, admitted Linux x64 capture artifacts, with the complete runtime kept together. Set `APPARENT_ENGINE_CAPTURE_EXECUTABLE` to its absolute launcher path and `APPARENT_ENGINE_RUNTIME` to an existing owner-only persistent directory. Use a separate private `APPARENT_AGENT_CONTROL_ROOT` when isolating an operation. These paths do not install or activate a provider globally.

```sh
apparent-agent capture --protocol capture2 sources --json
apparent-agent record --protocol capture2 --source '<exact-discovered-id>' --duration 5 --out '/absolute/fresh.apparent' # or omit --out for configured capture storage
apparent-agent capture --protocol capture2 inspect '<reported-request-id>'
apparent-agent capture --protocol capture2 stop '<reported-request-id>'
apparent-agent capture --protocol capture2 cancel '<reported-request-id>'
```

Discovery is read-only and returns currently advertised display and half-area sources; it does not start capture. `record` rechecks the exact source and asks Record to allocate an absent destination in configured capture storage when `--out` is omitted, or reserves the exact requested absent destination when supplied. Its parent directory must already exist. Capture is explicit, silent and local-only; context, interaction events, transcript and strokes are unavailable. Retain the reported public request ID and the same private runtime/control roots for cold recovery. Materials-ready or processing returns exit 3 and pending: it is not Saved and does not produce the requested final bundle. A confirmed cancellation is not a saved recording either. Do not fabricate a completed bundle from retained materials or feed those raw materials to media/1, which accepts completed bundles. Standard processing and publication remain producer gates.

The original V1 recording adapter retains its separate private control-token journal. For that protocol, use `apparent-agent capture inspect <capture-id>` with the same runtime; `stop` and `cancel` are also available. Do not copy, print, upload or delete private authority records until recovery is resolved. Never switch protocols or retry a new capture after an ambiguous outcome. A stop acceptance is not proof that a bundle was saved.
