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
apparent-agent zoom /path/to/capture.apparent --at 'width changes'
apparent-agent zoom /path/to/capture.apparent --mark m2 --frame 2 --full
```

Quote paths containing spaces. `inspect` accepts a local bundle directory, its `apparent.json` manifest, or `latest`. It does not accept URLs. Use `--json` to print the original manifest bytes, `--marks` for semantic mark lines (`--marks --json` for structured mark fields), `--transcript` for transcript text, and `--images` to list validated relative image references. `--images` does not open or inspect pixels; open the referenced files yourself. `zoom` creates a contact sheet PNG and semantic `zoom.json` companion in a fresh private output directory. It accepts a clock time or narration phrase with `--at`, or a mark number/id with `--mark`; use `--frame` when a number is shared by multiple keyframes. `--full` requests uncropped retained frames when available, and `--out` names a fresh absolute directory. The default is a new folder under `~/.cache/apparent/agent/zoom`.

`apparent-agent frames BUNDLE --from 0:04 --to 0:06 --fps 4` returns saved keyframe paths without a decoder when no provider is configured or source video is missing. Equal start/end times select a nearby saved frame. Saved mode preserves manifest timestamps, never changes the bundle, and identifies itself on stderr and in `--json`; those images are existing keyframes, not newly decoded samples. Open them with your image viewer. No matching saved images produces an error.

Video decoding instead requires canonical absolute `APPARENT_ENGINE_FRAMES_EXECUTABLE` and owner-only `APPARENT_ENGINE_RUNTIME` paths. It selects up to 48 bundle-clock timestamps and writes to a fresh private job directory under that runtime. `--json` identifies decoded mode and includes request times and clamp flags. The SDK currently has no native decoder: do not claim real decoding or PNG proof from synthetic contract tests. Provider errors remain errors. A timeout does not establish cancellation; retain the reported job/output diagnostics, and never retry or delete ambiguous output blindly.

The default inspect text is generated from the validated manifest. `--stored-summary` prints the producer-authored `summary.md` when you specifically need that text. `--experiment agentContext` requests the richer local event and mark timeline for this inspection. Inspection may read `capturesDir` and `experiments.agentContext` from `~/.config/apparent/config.json`; it never edits config. A read-only `APPARENT_CAPTURES_DIR` environment override takes precedence for `latest`.

## Read the bundle

`contact-sheet.png` gives a visual overview of keyframes in time order. Open it first, then inspect relevant `frames/frame-NN.png` files at full size. If the manifest describes autocropped frames, it may also identify uncropped references under `frames-full/`. Images carry the visual evidence; summary and transcript text do not replace looking at them.

`apparent.json` is the versioned manifest. It contains capture timing and target/window context, transcript text and timed segments or words, keyframes and their image references, semantic marks, interaction events, optional notes and crop metadata. Mark records include IDs, times, shape, description, stroke indices and optional frame or target context. Transcript words and segments are timed text, not proof that speech recognition was correct. `summary.md` is producer-authored text and can be stale or absent; the default CLI summary is regenerated from manifest semantics.

Use keyframe numbers and timestamps when referring to moments. Treat recorded session/destination metadata as historical context only; it does not authorize forwarding the capture. Do not send a bundle, transcript, image or path to another person or session unless the person explicitly asks.

## Settings and operation limits

The CLI can read local capture-location and agent-context preferences, but it does not change settings or create capture storage. Invalid config is ignored where supported; an explicitly selected unavailable capture directory reports an error. Never print or share the whole config file because it may contain private values.

This standalone package does not provide a native recording, capture-control or frame decoder, export, Connect or authentication workflow. Its `record` and video `frames`/`zoom` paths require explicit Engine provider paths, and the merged Engine foundation does not implement native decode. Without decode capability, zoom may compose retained keyframe PNGs in its selected time window. Such output is sparse, tagged `saved-keyframes` / `bundle-manifest`, and uses original keyframe timestamps; it is not a set of newly decoded video samples. Configured decode errors and uncertain jobs remain errors. The package uses bundled pure-JS `pngjs` only to read/composite/encode PNG images. Do not claim recording started or native media decoding from command availability, saved images or help output. Inspection and frame orchestration never use the network. `issue` separately offers an offline preview and explicit public submission; follow the bundled `apparent-issue` guide and obtain approval of the exact destination and text before submitting.

The recording adapter may save a private recovery record containing a control token before it requests Engine start. If one is reported after an ambiguous failure, keep it private and use the same runtime with `apparent-agent capture inspect <capture-id>` to reconcile the owned operation; `stop` and `cancel` are also available for that recorded ID. Do not copy, print, upload or delete that record until recovery is resolved. A stop acceptance is not proof that a bundle was saved.
