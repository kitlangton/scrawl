# Working with captures

[← Apparent](../../../README.md)

## Agents

Install the [official CLI](installation.md) to run `apparent` on the recipient machine.
Read login/fetch needs no desktop launch, recording permissions, speech model or upload consent.

Optionally install the capture-reading skill for Claude Code, OpenCode, Codex and other agents:

```sh
bunx skills add https://github.com/tryapparent/apparent/tree/main/capture/agent/skills/apparent
```

On macOS, the `apparent` command is the binary inside Apparent.app. Click
**Install** beside Command-line tool in Settings › Agents, which links it into `/opt/homebrew/bin`
or `/usr/local/bin` when you can write there, else `~/.local/bin`. The link always runs the app's
current copy, so it updates with the app.

With the skill, an agent that gets a Apparent paste reads the contact sheet and transcript, opens the
keyframes the narration points at, and uses the command to look closer: `apparent zoom <bundle> --at
0:12` (or `--at "a phrase you said"`, or `--mark 2` for the mark the text shows as ✎②; `apparent
inspect --marks` lists them) makes a dense sheet of frames around a moment, `apparent
frames` pulls a range, `apparent copy` puts a capture on the clipboard, and `apparent export`
makes a shareable `.mp4`. `apparent skill` prints the skill without installing anything.

The repository also ships [`make-it-apparent`](../skills/make-it-apparent/SKILL.md), a separate
agent skill for a short, checked change demo, unresolved-problem demonstration or technical
explainer. It covers causal explanation, captions, privacy, playback review and honest evidence
limits. The capture-reading skill above is still for understanding feedback someone sends you;
the video skill makes the current topic understandable, even before implementation is finished. The default install command above
does not guarantee that this named workflow is installed. Install it explicitly in the agent's
environment before asking for a demo:

```sh
bunx skills add https://github.com/tryapparent/apparent/tree/main/capture/agent/skills/make-it-apparent
```

Live Apparent proof needs a working capture on an accessible machine; a conceptual explainer
can use created visuals without a desktop capture. Explicit requests for actual app footage still
require that footage. Export a separate polished video and review it before sharing. Apparent
export supports macOS and Linux (`ffmpeg` and `ffprobe` are required on Linux).
Installing either skill does not install the app or a media renderer.

## Usage

```sh
# Launch the GPUI menu-bar and review app (press ⌥⇧S to record)
cargo run -- app

# Generate a realistic multi-frame demo bundle and print its agent summary
cargo run -- demo --out /tmp/demo.apparent

# Open a bundle (or `latest`) in the native GPUI review window
cargo run -- preview /tmp/demo.apparent

# Print the agent summary or JSON manifest for a bundle
cargo run -- inspect /tmp/demo.apparent
cargo run -- inspect /tmp/demo.apparent --json

# Extract frames in a time window for an agent to look closer
cargo run -- frames /tmp/demo.apparent --from 0:02 --to 0:07

# Copy contact-sheet.png + summary.md to the clipboard
cargo run -- copy /tmp/demo.apparent

# Capture a cabled iPhone for 8 seconds from the terminal (mirror window + native video)
cargo run -- record --iphone --duration 8
cargo run -- devices

# Rebuild an existing bundle with the current transcriber and keyframe rules
cargo run -- reprocess latest --out /tmp/again.apparent

# Build a capture whose build never ran (the app died first), in place; the app does this at launch
cargo run -- recover ~/Library/Application\ Support/apparent/captures/apparent-….apparent

# Tune auto-crop on a Screen capture, live
cargo run -- autocrop latest --serve --open

# Export a polished .mp4 (smooth activity zoom, crop, ink and audio; --no-zoom opts out)
cargo run -- export latest --out /tmp/demo.mp4

# Clean up old captures now (the app does it daily), or keep one forever
cargo run -- clean --dry-run
cargo run -- pin latest
```
