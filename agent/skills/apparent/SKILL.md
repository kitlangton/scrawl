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
```

Quote paths containing spaces. `inspect` accepts a local bundle directory, its `apparent.json` manifest, or `latest`. It does not accept URLs. Use `--json` to print the original manifest bytes, `--marks` for semantic mark lines (`--marks --json` for structured mark fields), `--transcript` for transcript text, and `--images` to list validated relative image references. `--images` does not open or inspect pixels; open the referenced files yourself. There is no `frames`, `zoom`, export, or media-decoding command in this package.

The default inspect text is generated from the validated manifest. `--stored-summary` prints the producer-authored `summary.md` when you specifically need that text. `--experiment agentContext` requests the richer local event and mark timeline for this inspection. Inspection may read `capturesDir` and `experiments.agentContext` from `~/.config/apparent/config.json`; it never edits config. A read-only `APPARENT_CAPTURES_DIR` environment override takes precedence for `latest`.

## Read the bundle

`contact-sheet.png` gives a visual overview of keyframes in time order. Open it first, then inspect relevant `frames/frame-NN.png` files at full size. If the manifest describes autocropped frames, it may also identify uncropped references under `frames-full/`. Images carry the visual evidence; summary and transcript text do not replace looking at them.

`apparent.json` is the versioned manifest. It contains capture timing and target/window context, transcript text and timed segments or words, keyframes and their image references, semantic marks, interaction events, optional notes and crop metadata. Mark records include IDs, times, shape, description, stroke indices and optional frame or target context. Transcript words and segments are timed text, not proof that speech recognition was correct. `summary.md` is producer-authored text and can be stale or absent; the default CLI summary is regenerated from manifest semantics.

Use keyframe numbers and timestamps when referring to moments. Treat recorded session/destination metadata as historical context only; it does not authorize forwarding the capture. Do not send a bundle, transcript, image or path to another person or session unless the person explicitly asks.

## Settings and operation limits

The CLI can read local capture-location and agent-context preferences, but it does not change settings or create capture storage. Invalid config is ignored where supported; an explicitly selected unavailable capture directory reports an error. Never print or share the whole config file because it may contain private values.

This standalone package does not provide a working recording, capture-control, frame extraction, zoom, export, Connect, authentication or network workflow. Its `record` adapter requires explicit Engine runtime paths, and the merged Engine native provider does not implement recorder/media capabilities. Do not claim a recording started or a media operation succeeded from command availability or help output.

The recording adapter may save a private recovery record containing a control token before it requests Engine start. If one is reported after an ambiguous failure, keep it private and use the same runtime with `apparent-agent capture inspect <capture-id>` to reconcile the owned operation; `stop` and `cancel` are also available for that recorded ID. Do not copy, print, upload or delete that record until recovery is resolved. A stop acceptance is not proof that a bundle was saved.
