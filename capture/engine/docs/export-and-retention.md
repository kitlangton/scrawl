# Video export, retention and shrinking

Read this before changing `src/export/`, `capture/engine/bundle/retention.rs`, `capture/engine/native/Export.swift` or `capture/engine/native/Media.swift`.

## Video export (`src/export/`, `capture/engine/native/Export.swift`)

`apparent export <bundle>` writes one H.264/HEVC + AAC `.mp4`; the `videoExport` setting (default on) gates **Export Last Capture as Video…** on Mac and Linux. Mac saves to `~/Movies/Apparent` with progress beside the menu bar icon; Linux asks for a destination in the desktop's Save dialog and shows progress in the tray menu. Rust decides everything; on Mac the helper renders it from a JSON plan (AVAssetReader → Core Image crop/camera → Core Graphics ink → AVAssetWriter): `crop.rs` (`--crop auto|full|x,y,w,h` — auto is `autocrop` `Scope::Whole` — even-sided output size, audio window), `ink.rs` (strokes via `sketch::freehand_outline`, laser, pins, ripples; sizes mirror `render::annotate_keyframe_in_place`), `camera.rs` (on by default, `--no-zoom` opts out: pure events → camera rects, critically damped springs, never zooms during fast cursor travel; derived from ScreenDemon's click zoom). Everything in the plan is on the video's clock (bundle time − `videoOffsetMs`); Mac `audio.wav` starts `videoOffsetMs` in, an iPhone movie uses its own audio. Output is constant frame rate (screen movies only have frames on change). Test with `cargo test export::`.

On Linux the same Rust crop, ink and camera plans drive an FFmpeg decoder/encoder adapter (`capture/engine/export/linux.rs`); install `ffmpeg` and `ffprobe` with H.264/AAC support (HEVC also needs `libx265`). The CLI exports an existing bundle even when the separate GStreamer keyframe decoder is unavailable. The source video stays intact. Export writes a uniquely named partial beside the destination, checks streams and duration, then renames it; failures remove that partial. It refuses to replace source video, saved audio or any existing bundle file; choose a fresh name inside the bundle or an output path outside it. The CLI remains available when the menu is disabled: `apparent export <bundle> [--out path] [--no-zoom]`.

Fresh successful Hyprland screen/area movies record `videoInk: "captured"` in the bundle because their source pixels already contain the live overlay. Export, keyframes and mark zooms keep those pixels without replaying strokes, pins or laser. KDE Wayland records `"capturedVideo"`: movies and video zooms preserve captured ink, while still images reconstruct marks after KWin hides the live overlay on release. Semantic marks, crop geometry and click effects remain available. Recovery and reprocessing preserve these source facts. Missing `videoInk` means unknown provenance and retains legacy replay; `"excluded"` also permits replay. Other capture backends do not infer these facts from the machine reading the bundle.

### Linux tray export

The menu action is available by default. To hide it, set `"videoExport": false` in `~/.config/apparent/config.json`, preserving other keys. This setting is top-level, not inside `experiments`. The tray refreshes the flag while the app runs. Right-click Apparent's tray icon and choose **Export Last Capture as Video…**. A working `org.freedesktop.portal.FileChooser` SaveFile backend (such as `xdg-desktop-portal-gtk` or KDE's backend) must be available. Choose a local filename ending in `.mp4`; the dialog suggests the capture's name. Cancelling the dialog creates no output and restores the export action.

The menu shows the choosing/encoding status without hovering and disables duplicate exports until the worker finishes. Linux retains the last result in the menu; a capture failure takes precedence when export is idle. On success a notification names the saved file and Apparent reveals it in the file manager. On failure the notification gives the full error and the menu permits retry. Desktop shells can shorten long menu labels. This cancels destination selection, not an encoder already running. The menu keeps its existing optional-recorded-audio policy and does not change CLI audio requirements or the saved menu preference.

The Linux flow was exercised in an isolated Omarchy/Hyprland guest with the real tray and GTK SaveFile dialog: real capture, picker cancellation, export with busy status, missing-source failure and successful retry. Both saved H.264/AAC movies passed full decode. This is not an AppImage installation, physical microphone/transcription, X11 or new Mac runtime test.

Smooth activity zoom is enabled in both CLI and menu exports. The default crop is `full`: show the whole recorded screen, window, region or phone for the first 500 ms, then ease toward activity. Even activity at time zero cannot skip this opening. Explicit `--crop auto` or a rectangle still bounds the entire camera path, including the opening. `--no-zoom` disables camera motion but keeps the chosen crop, ink, click ripples and audio; with the default crop it gives fixed full-frame output. The old opt-in `--zoom` flag has been replaced. Original bundle recordings remain unprocessed evidence; export makes the share copy. Zoom needs saved interaction telemetry: with no clicks, drags or marks, the camera stays still. Use the app's recording shortcut for demos; timed `record --duration` does not collect those events. These default changes are explicitly requested, not gated behind a new experiment.

## CLI quiet-stretch speedup

CLI exports automatically speed up confirmed long quiet holds at 4×. Use
`--no-speedup` to keep recorded timing. Menu export timing is unchanged.
Speedup requires FFmpeg on PATH on both macOS and Linux (H.264/AAC encoders;
HEVC needs libx265); the Mac base renderer continues to use AVFoundation.
The source movie, audio and bundle metadata are never changed.

Detection decodes full source frames with FFmpeg freezedetect at -60 dB,
intersects stable spans with audio below -55 dB, and requires at least a
ten-second accelerated interior after retaining five seconds at each boundary.
Silent exports use visual stability without an audio requirement. Interactions,
cursor movement, ink and transcript segments protect nearby intervals.
This conservatively retains short pauses, reading time and audible speech;
quiet background audio can prevent compression. Pixel stability cannot determine
whether a long result is important to a viewer; use --no-speedup for timing proof.

The completed render is retimed as one movie, keeping burned-in annotations,
camera motion and audio together. Source intervals/rates are printed on stderr
and stored in the MP4 comment metadata when acceleration occurs. Accelerated
spans show a small 4× indicator. No new caption
track is generated. Detection or encoding failure fails the export, with no
completed movie published.

## Explicit export audio

Plain `apparent export <bundle>` preserves usable recorded audio. Saved iPhone bundles
prefer their movie's overlapping native audio track; otherwise export uses the saved
WAV, trimmed by `videoOffsetMs`. A readable silent WAV is still recorded audio: export
does not infer speech or microphone health from its samples. A corrupt WAV is an error,
not permission to mute it. Missing or non-overlapping recorded audio fails nonzero
with `[AUDIO_MISSING]`, the resolved bundle path and both recovery choices, before
creating the output directory or starting the encoder:

```sh
apparent export './capture.apparent' --no-audio --out './silent.mp4'
apparent export './capture.apparent' --narration './cues.json' --out './narrated.mp4'
```

These switches conflict. `--no-audio` deliberately removes even an existing track.
`--narration` replaces it only in the exported movie, never in the source bundle.
Neither mode prompts, captures a microphone, uploads text, or falls back to another
audio policy. Existing menu exports retain their optional-recorded-audio behavior;
the menu has no narration/silence chooser. Existing render plans keep the same schema.

### Local narration

Available by default through explicit `--narration`; ordinary exports keep their
existing audio policy. To disable narration, set `"experiments": {"exportNarration": false}`
in `~/.config/apparent/config.json`, preserving other keys. The setting is read on each
export; restart is unnecessary. Model/worker provisioning remains explicit and separate.
This is independent of the Mac/Linux `videoExport` menu preference.

A UTF-8 JSON script uses exported-video milliseconds, not bundle timestamps:

```json
{"version":1,"cues":[{"startMs":500,"endMs":5500,"text":"Here is the first change."}]}
```

Unknown fields and versions fail. Cues must be ordered, non-overlapping and wholly
inside the video; `endMs` must exceed `startMs`. Limits: one-hour video, 1 MiB script,
1–256 cues, 1–1000 nonblank plain-text characters per cue, 32000 UTF-8 text bytes
in total. No SSML, voice switching, execution or template expansion is supported.
Speech starts at `startMs`; the rest of each slot and gaps remain silent. A cue that
does not fit fails with `[NARRATION_TIMING]`: shorten the text or extend its slot.
Speech is never clipped, sped up or silently moved. Each cue has a 120-second worker
deadline. Temporary script/audio files live in a private directory outside the bundle
and are removed on ordinary success/failure; a killed parent can leave private scratch.

### Provision once, export offline

From this source checkout, explicitly run:

```sh
node capture/agent/scripts/provision-narrator.ts --check
# A missing worker requires an explicit source build:
node capture/agent/scripts/provision-narrator.ts --build-from-source
```

Setup needs Node 22.18+ and Python 3.10+. Missing workers additionally need
Rust 1.92+, a native linker and network access; current releases do not publish a
standalone narrator executable. Existing workers are reused without Cargo.
See the [narrator setup contract](../../agent/narrator/README.md) for readiness
states, interrupted-download recovery and explicit rebuilds.
The pinned worker uses embedded English G2P data, not espeak. It downloads assets once and
builds a separate locked worker; root `Cargo.toml` and `Cargo.lock` are unchanged.
The worker and assets live at `~/.local/share/apparent/narrator` on both platforms.
`--cache` is a provisioning/testing override only; exports use the default directory.
The worker must be provisioned separately on each OS/architecture. Export itself
never downloads anything and never calls a cloud TTS service.

Pinned assets (both are verified by size and SHA-256 before reuse or publication):

| Asset | Source | Bytes | SHA-256 |
|---|---|---:|---|
| `model.onnx` | thewh1teagle/kokoro-onnx GitHub release `model-files-v1.1`, `kokoro-v1.0.int8.onnx` | 114119327 | `ae315a79b623f244700e4afb9246c46a26066782e049ba174bf3ba433970ee9c` |
| `voices/bm_fable.bin` | onnx-community/Kokoro-82M-v1.0-ONNX, revision `1939ad2a8e416c0acfeecc08a694d14ef25f2231` | 522240 | `f889083196807b4adb15e9204252165f503b8d33d3982e681c52443c49d798f1` |

Downloads require HTTPS (including redirects), bounded bytes and a 15-minute deadline.
Private exclusive partial files are verified before publication; symlinked cache
components are refused. Damaged cached assets fail closed: remove the named damaged
file explicitly and rerun provisioning. No unverified model is used as a fallback.
The model/voice originate from Kokoro-82M (Apache-2.0); `loqui-kokoro =0.4.0` is
the pinned inference adapter. Consult those upstream licenses when redistributing
assets; upstream NOTICE files also apply to the embedded G2P data. The pinned ONNX
Runtime is statically linked; no separately installed runtime is needed.

The fixed voice is user-approved British male `bm_fable`, speed 1, CPU, three inference
threads. The worker emits mono 24 kHz PCM16 and applies an RBJ 9600 Hz notch (Q 2.5)
to mitigate the measured narrow whine. Regression tests measure rejection at 9600 Hz
and preservation of the speech band; this is not a general denoiser or a guarantee
that every model artifact is removed. Export muxes AAC and logs model, voice, speed,
filter and script path/cue count on stderr, without logging the script text. No
transcript or synthetic audio is written back into the bundle.

Runtime errors use `[NARRATION_DISABLED]`, `[NARRATION_SCRIPT]`, `[NARRATION_SETUP]`,
`[NARRATION_SYNTHESIS]` or `[NARRATION_TIMING]`; all are nonzero with no completed
export. Provisioning failures are separate `narrator provisioning failed:` diagnostics.
An empty generated WAV is a synthesis failure, not evidence that the user did not
speak; silent recorded samples remain a valid recorded track.

## Retention and shrinking (`capture/engine/bundle/retention.rs`, `Media.shrink` in `capture/engine/native/Media.swift`)

The app's `retention` worker (utility QoS) runs a pass 90 s after launch and hourly (`PASS_EVERY`): heavy media (`handoff::HEAVY_MEDIA`, plus `zoom/` from `DERIVED_MEDIA`) to the Trash after `mediaDays` (default 7, writes `.media-cleaned`), whole bundles after `bundleDays` (30); `"retention"` in `~/.config/apparent/config.json`, Settings › Storage. It skips bundles under a day old or whose folder changed (ctime) in the last day, live `.media-pending`/`.media-sending` markers, `.pinned`, `.deliver.json`, and bundles `retention::claim`ed. Cabled-iPhone `recording.mov` (> 6 Mbps) is re-encoded to HEVC at 1600 px a minute after `finalize_live_capture`, written beside the bundle and renamed over the original; sample times are kept, so `apparent.json` is unchanged. Anything reading `video` must handle it missing (`retention::missing_media_reason`). Try passes on a temp folder with `apparent clean --captures DIR --days-ahead N [--dry-run]`.
