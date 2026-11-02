# Apparent

Show an agent a problem instead of describing it.

Want your agent to install the desktop app? Copy this prompt into your agent chat:

```markdown
Install the Apparent desktop app from [Apparent releases](https://github.com/tryapparent/apparent/releases), then launch it.
```

Press a shortcut (`⌥⇧S`) and Apparent starts recording the screen. Talk, click around, draw on it, and press `⌥⇧S` again to finish. Apparent records the screen, your voice, and your marks, then packs them into a compact bundle: a contact sheet of annotated keyframes (`contact-sheet.png`), individual full-resolution keyframes (`frames/frame-XX.png`), a time-aligned transcript and interaction timeline (`summary.md`), and a structured manifest (`apparent.json`).

## Install

No branded installers, Homebrew cask, or signed update have been published yet. Build from source
in this checkout; see [macOS packaging](capture/desktop/docs/release.md) or [Linux](capture/desktop/docs/linux/linux.md). Apparent supports
macOS and Linux, not Windows. The Mac build needs an Apple silicon Mac on macOS 15 or later (live
transcription and Liquid Glass need macOS 26). Linux builds target Ubuntu 24.04+ on x86_64 or ARM64.

When a Linux AppImage is published, make it executable and launch it with:

```sh
chmod +x Apparent-*-linux-*.AppImage
./Apparent-<version>-linux-x86_64.AppImage app
```

Press **Alt+Shift+S** to record. On Wayland, clicks and Shift ink need access to input devices:
run `sudo usermod -aG input "$USER"`, then **log out and back in**. Apparent checks access in the
welcome tour and Settings › Permissions. If the AppImage reports a missing `libfuse.so.2`,
install `libfuse2t64` on Ubuntu 24.04 (`libfuse2` on older distributions), or use
`./Apparent-<version>-linux-x86_64.AppImage --appimage-extract-and-run app`.
The first launch downloads the speech model (731 MB); captures get their transcript when it
finishes. See [Linux](capture/desktop/docs/linux/linux.md) for desktop-specific limits.

## Agents

Install the capture-reading skill for Claude Code, OpenCode, Codex and other agents:

```sh
bunx skills add https://github.com/tryapparent/apparent/tree/main/capture/agent/skills/apparent
```

The `apparent` command is the binary inside Apparent.app. Click
**Install** beside Command-line tool in Settings › Agents, which links it into `/opt/homebrew/bin`
or `/usr/local/bin` when you can write there, else `~/.local/bin`. The link always runs the app's
current copy, so it updates with the app.

With the skill, an agent that gets a Apparent paste reads the contact sheet and transcript, opens the
keyframes the narration points at, and uses the command to look closer: `apparent zoom <bundle> --at
0:12` (or `--at "a phrase you said"`, or `--mark 2` for the mark the text shows as ✎②; `apparent
inspect --marks` lists them) makes a dense sheet of frames around a moment, `apparent
frames` pulls a range, `apparent copy` puts a capture on the clipboard, and `apparent export`
makes a shareable `.mp4`. `apparent skill` prints the skill without installing anything.

The repository also ships [`make-it-apparent`](capture/agent/skills/make-it-apparent/SKILL.md), a separate
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

## Apparent Connect (candidate)

[Connect](connect/docs/hosted-client.md) lets a recording computer upload complete captures
and an agent fetch them with separate read approval. Uploads are off by default;
website sign-in alone grants neither permission. Update Apparent to use
`apparent connect login` and `apparent connect status`. Older apps with this feature
use `apparent cloud`, retained as a compatibility alias. Service and native
integration verification are still required; this is not a release announcement.

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

## Welcome and permissions

On first launch Apparent opens a welcome window with four screens: a welcome, how ⌥⇧S works, one
screen with every permission Apparent needs, and a first capture. Each permission is a row with a
button that shows the macOS prompt (or, once asked, opens System Settings) and a check that appears
by itself when macOS grants it (the window re-reads the grants every 0.5 s). Screen Recording is
first and its button is filled in; after that the next row's button is. Grant them in any order;
Continue unlocks once all three are on, and the screen is skipped when they already are.

| Permission | Used for | When it's asked for |
|---|---|---|
| Screen & System Audio Recording | Recording the screen (ScreenCaptureKit) | onboarding, required |
| Microphone | Your narration | onboarding, required |
| Accessibility ("Device Control and Data Access" on macOS 27) | The key event tap (⌘Z, Esc, ⌘-shortcuts in the window log), and `⌥⇧V` and the Paste chip, which post ⌘V into the app in front | onboarding, required |
| Camera | The cabled iPhone mirror (macOS treats it as a camera) | the first time you pick iPhone |

Apparent never asks for Input Monitoring. Clicks, drags and scrolls in other apps come from an AppKit
global mouse monitor (`NSEvent.addGlobalMonitorForEvents`), which needs no permission; ⇧ for ink
and whether ⌥⇧ is still held after ⌥⇧S come from polled modifier flags, which need none either; key presses
come from an active event tap, which Accessibility allows. macOS applies an Accessibility grant to
a running app at once (System Settings shows no "Quit & Reopen" for it), and Apparent makes the tap
within half a second of it.

Screen Recording and Accessibility are lists you drag an app into, so their button skips Apple's
prompt: it opens the pane in System Settings, and a card flies from the button to just below the
Settings window (or, with no room on screen there, over the window's empty lower area), with its
Apparent row lined up under the list. Drag that row into the list. The card follows the window,
steps aside when another app is in front, and shows "Apparent is allowed" once a fresh process sees
the grant (a drop alone proves nothing). The row drags the `Apparent.app` that contains the helper,
the bundle macOS grants. With Reduce Motion on, the card fades in place. Microphone and Camera
can't be dragged in, so they use the macOS prompt.

Camera is asked for in the same window, with just its row, when first needed. Speech recognition
isn't asked for: SpeechAnalyzer transcribes without it. The ⌥⇧S and ⌥⇧V hotkeys themselves need
no permission (Carbon `RegisterEventHotKey`).

Until the required permissions are on, ⌥⇧S starts nothing (no picker or overlay that
could sit over a macOS prompt): once the keys are released it brings the permissions window
forward.

A Screen Recording grant may not reach a running app, and macOS offers **Quit & Reopen** or
**Later** as soon as it's granted. Either way nobody reopens Apparent by hand. If you pick **Quit &
Reopen**, macOS restarts Apparent and the tour opens on the permissions screen again, Screen
Recording checked. If you pick **Later** (or nothing), Apparent checks whether it can capture anyway
(a one-frame ScreenCaptureKit screenshot, tried only once a fresh helper reports the grant), and if
it can't, the row shows "Reopening Apparent" for a second and Apparent relaunches itself into the same
screen. It only does that once in two minutes (`relaunchedAt` in the config); if a fresh copy
still can't capture, the row's button becomes **Reopen Apparent**. The relaunch waits for this copy
to exit and opens the app without `-n`, so if macOS's Quit & Reopen got there first there's still
one Apparent. `apparent sck-probe [--watch SECONDS]` and `apparent-helper sck-probe` try the same
capture from the command line.

Progress is `"onboarding"` in `~/.config/apparent/config.json`, including the screen the tour is on,
so it picks up there whatever quits Apparent. Once seen, the tour only comes back from **Welcome
tour** in Settings › About, but a required permission that was granted and is gone again reopens the
permissions screen at launch. `APPARENT_NO_ONBOARDING=1` skips it.

Do not reset real macOS permission grants to replay onboarding. Preview every screen without the app, including the permissions screen with none, some and all
granted and each on-demand sheet: `target/debug/apparent-helper onboarding --render /tmp/onboarding
--icon capture/desktop/assets/app-icon.png`.

## Settings

**Settings…** in the menu bar (⌘, while the menu is open) opens a small window with six panes:

- **General**: Launch at login and the two shortcuts, Record
  (`⌥⇧S`) and Paste last capture (`⌥⇧V`, which can be turned off).
- **Recording**: the microphone, with its input level, the transcriber (Apple Speech with its
  language, Hex while it runs, with its model and language, or Command when it's configured) and,
  Video export (on by default; its existing Settings section is still labelled Experimental).
- **Storage**: how long recordings are kept, where new captures go (Change…/Reset on macOS, `"capturesDir"` in
  `~/.config/apparent/config.json` everywhere; captures already made stay put, and an unusable folder falls back to the built-in one), how much space they take, and Show in Finder.
- **Permissions**: the onboarding checklist, so a permission can be turned on later.
- **Agents**: Install or Uninstall the `apparent` command (see [Agents](#agents)), and the skill's
  install command with Copy. A `apparent` that belongs to another program is left alone.
- **About**: the version, Check for Updates…, links, and the welcome tour.

To change a shortcut, click its keys and press the new combination; `Esc` cancels. It needs ⌘, ⌥
or ⌃ (a function key can stand alone), and Apparent turns down ⌘ with one key, combinations macOS
uses, the other Apparent shortcut, and anything another app has already taken. The arrow beside the
heading puts the default back. While the recorder listens, Apparent's own shortcuts are paused. They
are saved as `"shortcuts"` in `~/.config/apparent/config.json`. The menu keeps Record Screen, Choose
What to Record…, Open Latest Capture, Export Last Capture as Video… (when on), Settings…, Check
for Updates… and Quit.

**Microphone** lists the connected inputs, with System default first, which follows whatever macOS
uses as it changes. The list updates as devices are plugged in or out. A chosen device is saved by
its CoreAudio UID as `"microphone": {"id", "name"}`; if it isn't connected when you record, Apparent
records from the default instead and says so under the popup (and once in the log). The level
meter runs only while the Recording pane is on screen and Microphone is allowed. Cabled-iPhone
narration uses the same microphone. From the terminal, `apparent mic list` shows the inputs and the
one recordings will use, `apparent mic set <id|default>` chooses, and `apparent mic watch` prints the
list again on every change.

Preview every pane, dark and light, with the recorder listening and a combination turned down:
`target/debug/apparent-helper settings --render /tmp/settings --icon capture/desktop/assets/app-icon.png`.

## Capture Controls

- **Press `⌥⇧S`**: record the display under the pointer as soon as you let go, however long you
  held it, auto-cropped (see below). Press it again to finish; a press within 0.4 s of the start is
  ignored, so a double press doesn't leave an empty capture. `⌃⌥⇧S` also works (only while `⌥⇧S` is the
  shortcut; see [Settings](#settings) to change it). As it starts, the island opens onto the modes (Area ·
  **Screen** · iPhone when one is cabled) for about 3.5 s, or until you click, scroll or draw;
  hovering keeps them. Click Area to retarget (the first seconds are dropped and the
  picker takes over: drag an area), iPhone to record the phone instead, or just
  carry on.
- **Choose What to Record…** in the menu bar: the picker (Area, Screen, iPhone) opens on
  the display under the pointer. Click a mode or drag an area; `Esc` cancels.
- `⌥⇧V`: Paste both `contact-sheet.png` and `summary.md` into the active app or terminal (`⌃⌥⇧V` also works while it's the default)
- **Normal pointer**: Clicks and scrolls pass straight through to the app underneath
- **Hold `⇧` (Shift) + move**: Draw a persistent red marker stroke (`#ff3b30`) on screen that fades out after release
- **`⇧`-click**: Drop a numbered pin where you click (a press that moves under 12 pt and lets go within 1 s; longer is ink). It shows as a brief red pulse, not a lasting dot, and lands on the keyframes. A click after you've drawn in the same `⇧` hold is part of the drawing and drops no pin
- **`⌘Z`**: Undo the last stroke
- **`Esc`**: Cancel the active selection or capture

Window recording is temporarily disabled on macOS and Linux pending [source-lifetime safety](https://github.com/tryapparent/apparent/issues/264).
`apparent record --window <name>` returns an explicit error before recording, without falling
back to a screen. Existing Window bundles remain readable. Choose Screen or Area explicitly.

While picking a target, the island shows the modes **Area**, **Screen**, and **iPhone**
(when one is cabled). It opens on the mode you used last:

- `1`–`3` jump to a visible mode; `Tab`/`→` and `⇧Tab`/`←` step through them; `Space` toggles Area and Screen
- Area: drag a region. Screen and iPhone: `⏎`, `S`, `⌥⇧S`, **Record**, or
  clicking the selected mode again starts recording (`⏎`/`⌥⇧S` record the full screen from Area too).
  A previously saved Window picker preference opens Area instead, without starting a recording.

Captures are saved as `.apparent` bundles and copied to the clipboard. Apparent does not select an
OpenCode session or send captures automatically. Use `apparent copy <bundle>` to copy an older bundle.

## Auto-crop (Screen captures)

Instead of picking an area up front, record the whole screen: tap `⌥⇧S`, or choose **Screen**.
Area and Window captures aren't cropped.
Apparent works out where you were gesturing and crops each keyframe to it. While a Screen capture
records, Apparent logs:

- **`windows`** in `apparent.json`: the on-screen windows, front to back (id, app, bundle ID, title,
  bounds in capture points, layer, frontmost), including the menu bar and the Dock's strip. A
  snapshot is taken at the start, when a click, ink stroke or shortcut starts, when the frontmost
  app changes, at most every 500 ms while the cursor moves and every 2 s otherwise, and stored only
  when the list changed. Each snapshot costs about 0.5 ms. Titles stay in `apparent.json`; they never
  go into `summary.md` or the clipboard.
- **`cursor`**: the cursor at up to 15 Hz while it moves, as `[t, x, y]` (normalized), so dwell
  counts.

When the capture finishes, each keyframe gets the slice of the timeline halfway to its neighbours,
and `capture/engine/autocrop/autocrop.rs` weighs what happened in it: ink, clicks, drags and scrolls first, then where
the cursor rested and what changed on screen (a cursor wandering far from the ink is ignored). The
crop is then:

- **the window** holding the activity (two at most, for an arrow from one to the other), including
  a menu it opened and the touched part of the menu bar;
- or, when that window is most of the screen (maximized, full screen), **the activity inside it**
  with some context, its edges moved into nearby white space and past the ends of any lines of
  text they cross (up to the window's edge), so they don't cut words in half;
- or the previous crop, for a keyframe with only talk.

Neighbouring crops that mostly overlap become one, so the sheet doesn't jitter. A crop covering
most of the screen isn't worth it, and the keyframe stays whole.

The bundle then has:

- `frames/frame-XX.png` and `contact-sheet.png`: cropped from the full-resolution frames (Screen
  keyframes are decoded from the 2560 px wide recording), with the ink and keyframe numbers redrawn at the crop's scale;
- `frames-full/frame-XX.png` and `contact-sheet-full.png`: the uncropped versions;
- one line in `summary.md`, e.g. `Cropped to: Ghostty + Safari (full frames in frames-full/)`;
- `autocrop` in `apparent.json`: each keyframe's crop in points, its kind and reason, and the
  changed regions that fed it.

`apparent zoom` crops its cells like the nearest keyframe (`--full` for the whole screen).
`apparent reprocess <bundle> --out <dir>` re-runs it on an existing Screen capture from its video.
To turn it off, or change a setting, use `~/.config/apparent/config.json`:

```json
{ "autocrop": false }
{ "autocrop": { "padding": 24, "minW": 480 } }
```

### Tuning

`apparent autocrop <bundle|latest> --serve --open` opens a playground that runs the same Rust crop as
you move its sliders: each keyframe with its windows, activity, cursor heat, outliers and crop, the
cropped frame, and the cropped contact sheet. **Copy settings** gives the JSON for the config.
Without `--serve` it writes `autocrop/index.html` with the presets only.

The settings were tuned on simulated captures with hand-labelled ideal crops: one maximized
terminal, two windows side by side, an arrow from one window to another, a tiny badge in a huge
window, a menu-bar menu, talk-only keyframes, a dragged window, overlapping windows, thirty seconds
hopping between three windows, a stray cursor, a full-screen app, and a scrolling page.
`public/site/scripts/motion/autocrop-corpus.mjs` renders them with Playwright (from `public/site/node_modules`), `apparent
fixture` builds each into a real bundle through the capture pipeline, `apparent autocrop-eval` scores
the crops (coverage of the ink and clicks, overlap with the ideal, context, legibility in the sheet,
stability; `--sweep` searches the settings), and `public/site/scripts/motion/autocrop-gallery.mjs` renders a gallery to
judge them by eye:

```sh
bun public/site/scripts/motion/autocrop-corpus.mjs /tmp/corpus
for s in /tmp/corpus/*/scenario.json; do
  apparent fixture "$s" --out "/tmp/bundles/$(basename "$(dirname "$s")").apparent"
done
apparent autocrop-eval /tmp/bundles --json /tmp/scores.json   # add --sweep to search
bun public/site/scripts/motion/autocrop-gallery.mjs /tmp/gallery /tmp/bundles --scores /tmp/scores.json
```

`autocrop-eval --export capture/engine/autocrop/autocrop-corpus.json` refreshes the corpus the unit tests score.

## iPhone

Plug an iPhone in with a cable (unlocked, trusted) and the picker shows **iPhone** next to Area,
Window, and Screen. Choosing it opens a floating mirror of the
phone on the right of the screen and starts recording:

- The phone's own video is recorded natively (1320×2868 at up to 60 fps on an iPhone Pro Max) with
  your narration from the Mac's microphone, into the bundle as `recording.mov`. Keyframes are
  decoded from that video, never from a screenshot of the mirror. A minute after the capture the
  video is re-encoded to HEVC at the keyframes' 1600 px (see [Keeping captures small](#keeping-captures-small)).
- The laser, ink, pins, and clicks work over the mirror exactly as over a window. The mirror's
  on-screen rect is the capture target, so everything you draw is stored in phone-screen
  coordinates and lands in the right place on the phone keyframes. Move or resize the mirror
  freely; the mapping follows it.
- Press **M** with the mirror focused to flag a moment ("look here"); flags always get a keyframe
  and show as `⚑` in the summary.
- Finish from the island as usual. Unplugging the phone finishes the capture with what was recorded.

Phone bundles are ordinary bundles: `apparent.json` adds `"source": "iphone"` and a `device` object
(name, capture `mirror`, id, native size, screen points and scale), and the target's `mode` is `device`. Keyframes fit a 1600 px long edge
(736×1600 for a 1320×2868 phone); `apparent frames` and `apparent zoom` extract more frames from the
video when you need them.

## Export as video

`apparent export` turns a bundle into one shareable `.mp4`: H.264 (`--hevc` for HEVC) with AAC audio in
sync, the red ink, pins, laser and click ripples burned in at the times they happened. It opens on
the whole capture for half a second, then smoothly zooms toward activity by default. On macOS it
runs in `apparent-helper` on AVFoundation (hardware decode and encode, Core Image for the crop,
Core Graphics for the ink), without FFmpeg. Linux uses the same camera and annotation plan with
`ffmpeg` and `ffprobe` (H.264/AAC support required). Original bundle recordings stay unchanged.

```sh
apparent export latest                              # ~/Movies/Apparent/<bundle>.mp4
apparent export latest --out demo.mp4               # smooth activity zoom by default
apparent export latest --no-zoom --crop full --size 1280  # fixed whole capture, 1280 px long edge
apparent export latest --crop 0.1,0.2,0.6,0.5       # x,y,w,h as fractions, or pixels of the video
```

- **Crop** (`--crop full|auto|x,y,w,h`): `full`, the default, opens on the whole recorded screen,
  window, area or iPhone. Explicit `auto` uses the activity region for Screen captures; explicit
  rectangles are respected, including at the opening. The output keeps the crop's
  aspect with even sides, is capped at `--size` (1920) on its long edge, and is never scaled up.
- **Zoom** (on by default; `--no-zoom` opts out): after a 0.5 s whole-view opening, the camera eases to ~1.5–1.8× toward clicks, drags, marks and laser points,
  holds while activity stays nearby (2.5 s after the last), follows the cursor once it leaves a
  central safe zone, and eases back out. It never starts a zoom while the cursor is travelling fast.
  The path is a pure function of the bundle (`capture/engine/export/camera.rs`, critically damped springs,
  no bounce); the renderer only draws each frame's rectangle. Without saved interaction events
  the camera stays still: use the app's recording shortcut for demos, not timed `record --duration`.
- **Audio**: `audio.wav` starts where the video does (`videoOffsetMs`); a cabled iPhone's movie
  uses its own audio track. `--no-ink` and `--no-clicks` leave the annotations out, `--fps` sets the
  rate (default 60 for a 60 fps recording).
- **Menu**: with **Video export** on in Settings › Recording (`"videoExport": true` in
  `~/.config/apparent/config.json`, default on), the menu shows **Export Last Capture as Video…**.
  Mac saves to `~/Movies/Apparent/<name>.mp4`, shows progress beside the menu bar icon,
  and reveals the file in Finder. Linux opens the desktop Save dialog for a local `.mp4`
  destination and shows progress, cancellation and failures in the tray menu; it needs a
  FileChooser portal backend. See [Linux tray export](capture/engine/docs/export-and-retention.md#linux-tray-export).
  The command works without the flag.

## Keeping captures small

Captures would otherwise pile up (mostly video). The app cleans up shortly after launch and then
hourly, at low priority, and logs what it did:

- **Video, audio and full-size frames** (`recording.mp4`/`.mov`, `audio.wav`, `frames-full/`),
  plus the `zoom/` sheets made from them, go to the Trash after 7 days, as one `<bundle> media`
  folder. The prompt (`summary.md`, `apparent.json`, `contact-sheet.png`, `frames/`) stays, so
  pasting and reading still work; `apparent zoom` says the video was cleaned up.
- **Whole bundles** go to the Trash after 30 days.
- **Keep recordings** in Settings › Storage picks Video for 7 Days (bundles 30), Video for 30 Days
  (bundles 90), or Forever. It's stored as `"retention": {"mediaDays": 7, "bundleDays": 30}` in
  `~/.config/apparent/config.json`; `null` keeps forever. The same pane shows how much the captures
  take and opens the folder in Finder.
- Never touched: a bundle under a day old or changed in the last day (a copy just arrived from
  another Mac, a zoom), one whose media is still copying (`.media-pending` there, `.media-sending`
  here; a marker over a day old is ignored), one being recorded or re-encoded, and one with a
  `.pinned` file (`apparent pin`, `apparent pin --off`).
- **iPhone video** from the cable is ~18 Mbps at 1320×2868. A minute after the capture (after any
  copy to another Mac finishes, and never during a recording) `apparent-helper shrink` re-encodes it
  to HEVC at a 1600 px long edge, audio copied as is and every frame at its original time, and
  swaps it in place: a 4.5-minute capture goes from 750 MB to 72 MB in about 20 s. One pass a day
  shrinks any native iPhone video that was missed.

`apparent clean --dry-run` prints what a pass would do now.

## Transcription

Narration is transcribed on device by Apple's SpeechAnalyzer (macOS 26), or on macOS 15 by the
older on-device `SFSpeechRecognizer`. On macOS 15 onboarding asks for Speech Recognition, and
Dictation's on-device model has to be on the Mac (System Settings › Keyboard › Dictation); Apparent
never falls back to Apple's servers. Both report when every word was spoken. The word timings
drive two things:

- **Text per keyframe.** Each keyframe's text runs from the start of the clause in progress when
  its moment began (the action, the transition, or the spoken cue) until the next keyframe, so the
  transcript is split at sentence and clause boundaries, in order, with nothing repeated or lost.
- **Spoken cues.** Saying "now", "here", "look", or "watch" is a deliberate cue: if the screen
  changes around the word, that settled view becomes a keyframe; otherwise the frame as you said it.
  A cue only cuts a keyframe if its image differs from every keyframe already chosen, so talking
  over an unchanged screen still produces one frame. "This", "that", "these", "those", "see", and
  "there" count only when they coincide with a change, because they are usually filler. Speech
  pauses never cut keyframes.

### Apple or Hex

Apple's SpeechAnalyzer is the default; Apparent prepares its language model automatically. If you use Hex
(2.2 or later, running), Apparent can transcribe with it instead and with whichever model Hex has
ready: Settings › Recording › **Transcription** → **Hex**, then pick a model under **Hex model**.
Or in
`~/.config/apparent/config.json`:

```json
{ "transcriber": "hex", "hexModel": "whisper_large_v3_turbo", "hexLanguage": "nl" }
```

`"transcriber"` is `"apple"` (default), `"hex"` or `"command"` (below); `hexModel` defaults to the
first ready one of `parakeet_unified_en`, `parakeet_v3`, `parakeet_v2`, `whisper_large_v3_turbo`.
Apparent never downloads or prepares a Hex model: install it in Hex first.

**Language** in Settings › Recording picks the language Apple Speech transcribes; **Automatic**
(the default) follows the Mac's language. In config it's `"appleLocale"`, e.g. `"zh-TW"` for
Traditional Chinese (Taiwan). Apparent downloads that language's model in the background, and a
language this Mac doesn't support falls back to Automatic (Settings names the one it uses).

**Hex language** in Settings › Recording (or `"hexLanguage"` in config) sets the language
code sent to Hex. It defaults to `en` to preserve existing behavior; use `nl` for Dutch or
`auto` for automatic detection when the model supports it. Press Return or Apply to save;
a blank field resets to `en`. Codes are not restricted to a list in Apparent: Hex validates
model/language support. If Hex rejects a language/model combination, Apparent uses the existing
Apple Speech fallback. This setting does not inherit Hex's dictation language. Whisper uses the selected
language to guide recognition; models such as Parakeet v3 determine language from audio
and ignore the hint.

- Hex is optional. When it isn't running, **Hex** is greyed out as "Hex (not running)" and Apparent
  uses Apple, whatever the saved preference. If Hex fails on a recording, that recording falls back
  to Apple Speech (see `~/Library/Logs/Apparent`).
- Hex returns text without word times, so Apparent sends each stretch of speech separately and
  spreads its words across it by length. The transcript says `"source": "hex:<model>"` and
  `"estimated": true`; because those times are approximate, spoken cues ("now", "here") don't cut
  keyframes from a Hex transcript. Marks, clicks, laser and scene changes still do.
- `apparent hex status` shows whether Hex is connected, its ready models and what the next recording
  will use. `apparent reprocess <bundle> --out … --transcriber hex|apple|command` picks an engine for one run.

### Your own model (command)

Any other local model (whisper.cpp, MLX Whisper, faster-whisper, Parakeet, a Core ML model…) can
transcribe through a small wrapper program. Apparent runs it on the recording's audio and reads a
JSON transcript from its stdout; it knows nothing about the model, adds no network requests and
doesn't sandbox the program. It runs on macOS only. In `~/.config/apparent/config.json` (Settings
shows **Command** once it is set; editing it stays in the file):

```json
{
  "transcriber": "command",
  "transcribeCommand": ["/Users/me/bin/whisper-apparent", "--model", "large-v3-turbo", "{audio}"],
  "transcribeTimeoutSeconds": 600
}
```

- `transcribeCommand` is an argv array run directly: no shell, quoting, `~`, variables or globbing.
  `argv[0]` must be an absolute path. Every `{audio}` in the other arguments becomes the audio path;
  without one, the path is the last argument. Put your model's language in it too (`--language nl`):
  Settings' **Language** is Apple Speech's alone. These keys are read on their own, so a mistake in
  them never resets the rest of the config.
- `transcribeTimeoutSeconds` is optional, a positive number of seconds: by default the limit is
  `max(300, 60 + 10 × audio seconds)`, wall clock from the start, model loading included.
- The program gets the whole recording as one 16 kHz mono 16-bit WAV, `APPARENT_TRANSCRIBE_VERSION=1`
  and `APPARENT_AUDIO=<path>`, stdin `/dev/null`, and its own process group. Its stderr is discarded.
  It must not change the audio, daemonize or leave processes behind, and must cope with two runs at
  once. On timeout or more than 8 MiB of output Apparent kills the group.
- It exits 0 and prints exactly one JSON object:

  ```json
  {
    "version": 1,
    "model": "whisper-large-v3-turbo",
    "text": "Click here.",
    "segments": [{ "t0": 300, "t1": 980, "text": "Click here." }],
    "words": [{ "t0": 300, "t1": 610, "text": "Click" }, { "t0": 640, "t1": 980, "text": "here." }],
    "estimated": false
  }
  ```

  `version` (the integer 1) and `text` (empty means no speech) are required; the rest may be
  omitted or `null`, and unknown fields (or a `model` or `estimated` of another type) are ignored. Times are integer milliseconds from the first
  sample. `segments` and `words`, when given, cover the whole text. `estimated` is `false` only if
  the word times were measured (by the recognizer or an aligner); it defaults to `true`, and spoken
  cues only cut keyframes from measured words. Without `words`, Apparent spreads the text over the
  segments (or the whole recording) and marks those times estimated, as it does for Hex.
- Spans may overlap or be zero-length; Apparent sorts them by `t0`. Ends up to 500 ms past the audio
  are clamped. A negative, fractional or reversed span, or one further past the end, makes Apparent
  drop that array and keep the text, spreading it as above when `words` are dropped.
- The transcript says `"source": "command:<file name of argv[0]>"`. Anything else going wrong (a
  malformed `transcribeCommand` or timeout, the program missing, a non-zero exit, a timeout, invalid
  JSON, another version) means Apple Speech transcribes that recording. Each problem is one line in
  `~/Library/Logs/Apparent` (a malformed config only once), never with the program's output,
  arguments or what was said.

### Transcribing while recording (experiment)

```json
{ "experiments": { "liveTranscribe": true } }
```

With this in `~/.config/apparent/config.json`, a Mac recording transcribed by Apple Speech streams the
microphone to `apparent-helper transcribe-stream` as it records, so finishing only waits for the last
words (tens of milliseconds) instead of transcribing the whole recording. The streamed samples are
exactly the ones `audio.wav` holds, so word times are on the same clock. If the stream fails, the
speech model isn't installed yet, or it takes too long, the recording is transcribed afterwards as
usual; the log says which (`Live transcription: …`). Hex and iPhone recordings are unaffected.
`apparent reprocess <bundle> --out … --bench 1 --live-speed 1` replays a bundle's audio through it.

## Helper

`apparent-helper` is a small Swift executable built by `build.rs` from explicit owner-native source groups and shipped inside
`Apparent.app` next to `apparent`. It owns the Swift-only or unbound Apple frameworks: SpeechAnalyzer,
CoreMediaIO/AVFoundation iPhone capture with the mirror window, AVFoundation movie decoding, and the
SwiftUI welcome window (`onboarding`).
Apparent runs it as a subprocess (JSON on stdout; `stop`/`cancel` on stdin for the mirror). Run it
directly to test a piece in isolation:

```sh
target/debug/apparent-helper devices
target/debug/apparent-helper permissions
target/debug/apparent-helper transcribe audio.wav
target/debug/apparent-helper frames recording.mov --out-dir /tmp/f --at 1000,2000 --max-edge 1600
target/debug/apparent-helper shrink recording.mov --out /tmp/small.mov --max-edge 1600
target/debug/apparent-helper export plan.json   # the plan `apparent export` writes (capture/engine/export)
target/debug/apparent-helper permission-panel --permission screen-recording --preview   # the drag card, no grant polling
```

`permission-panel --permission screen-recording|accessibility [--from X,Y,W,H]`
(`--from` is the button's frame in global top-left points) prints JSON lines: `docked`, `dropped`,
then `granted` or `cancelled` with a `reason` (`back`, `settings-closed`, `no-settings-window`,
`parent`, `timeout`), and exits. On stdin it takes `cancel`, `back` and `granted`.
`permission-panel --check P` prints `{"granted":…}` from a fresh process. `permission-panel --plan --settings
X,Y,W,H --visible X,Y,W,H` prints where the card would dock (`below`, `inside`, `right`, `left` or
`clamped`); `capture/desktop/scripts/support/check-permission-panel-dock.sh` checks the cases.

## Linux

Apparent runs on Linux desktops (GNOME and KDE Plasma, on Wayland or X11) as the same app, from
the same GPUI code. It ships as an AppImage (`capture/desktop/scripts/linux/appimage/appimage.sh`; every pull request's CI
run has one) with GStreamer inside, so movies record without extra packages. On Wayland its
windows run through XWayland.

| | X11 | Wayland |
|---|---|---|
| Shortcut | `Alt+Shift+S`, `Alt+Shift+V` (`XGrabKey`) | GNOME keybindings, Plasma 5 KGlobalAccel, or the GlobalShortcuts portal |
| Menu | Tray icon (StatusNotifierItem) | The same (GNOME needs the AppIndicator extension, on by default on Ubuntu) |
| Screen capture | `ximagesrc` | GNOME's screencast API (no dialog), else the ScreenCast portal (asks once) |
| Clicks, keys, Shift ink | The X server | The input devices (join the `input` group) |
| Paste | `Ctrl+V` (`Ctrl+Shift+V` in terminals) through XTEST | The same, through the RemoteDesktop portal (asks once) |
| Transcription | Parakeet on device (`transcribe.cpp`): downloads at launch (731 MB); progress in Settings › Recording | The same |
| Captures | `~/.local/share/apparent/captures` | The same |

- **Wayland:** reading clicks, keys and Shift ink needs `sudo usermod -aG input $USER` and a new
  login (Settings › Permissions shows it). Click positions are exact over X11 apps and with
  tablets, touchscreens and VMs; with a mouse over a Wayland app they can be stale. Apparent's pill
  and ink show in the recording (no Linux desktop lets an app leave its windows out).
- **Window picker:** lists X11/XWayland windows. Use Screen or Area for native Wayland apps.
  On KDE Wayland, desktop ink hides when Shift is released so clicks reach the app; the bundle keeps it.
- **Not on Linux:** the cabled iPhone, video export, `agentContext` and Sparkle updates.

See [capture/desktop/docs/linux/linux.md](capture/desktop/docs/linux/linux.md) for how it works and how to build it.

## Testing

```sh
taskpolicy -c utility cargo test -j 3 --offline -- --test-threads=2
./capture/desktop/scripts/packaging/install-app.sh
```

On Linux, run `cargo test`; then run `apparent app` in a desktop session and record with `Alt+Shift+S`.
`APPARENT_SHOW_OWN_WINDOWS=1` keeps Apparent's own windows (capsule, overlay, picker) in screenshots,
which normally leave them out.

- **iPhone:** connect a phone, press `⌥⇧S`, click the iPhone chip, talk, draw over the mirror, press M,
  Finish. Inspect the bundle's `summary.md`, `apparent.json` (`source`, `device`), contact sheet, and
  frames.
- **Transcription:** `apparent-helper transcribe <wav or movie>` prints words with timings;
  `apparent reprocess <bundle> --out …` reruns transcription and keyframe selection on a real bundle.

## License

MIT. See [LICENSE](LICENSE).

### THIRD_PARTY

`capture/engine/native/` and `capture/desktop/native/PermissionPanel.swift` adapt code and tuning from two MIT-licensed projects:

- [AskForPermission](https://github.com/riko2chen/AskForPermission), Copyright (c) 2026 Riko: the
  flight along a quadratic Bézier, waiting for the System Settings window to settle, following it
  through the window server, and the race that resolves a drop.
- [PermissionFlow](https://github.com/jaywcjlove/PermissionFlow), Copyright (c) 2026 小弟调调: the
  Privacy pane deep links and macOS 27's "Device Control and Data Access" name for Accessibility.

Both are under the MIT License: permission is hereby granted, free of charge, to any person
obtaining a copy of this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to use, copy, modify,
merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to
whom the Software is furnished to do so, subject to the following conditions: The above copyright
notice and this permission notice shall be included in all copies or substantial portions of the
Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES
OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
