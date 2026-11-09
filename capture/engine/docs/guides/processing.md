# Processing, transcription and export

[← Apparent](../../../../README.md)

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
  FileChooser portal backend. See [Linux tray export](../export-and-retention.md#linux-tray-export).
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

Off by default. When enabled, a Mac recording using Apple Speech streams the microphone to one
continuous recognizer, using the preferred SpeechTranscriber or the existing DictationTranscriber
fallback. Apple chooses when results become final and may defer them until EOF. Samples and word
times use the saved WAV's source clock; batch word-for-word or timing identity is not promised.
If live recognition fails, loses samples or exceeds its helper I/O deadline, Apparent transcribes
the complete saved WAV. Hex and imported recordings keep their existing paths.
`apparent reprocess <bundle> --out … --bench 1 --live-speed 1` replays a bundle's audio through it.
See the [live transcription contract](../keyframes-and-text.md#experiment-livetranscribe-captureengineruntimeaudiotranscriptionlive_transcribers-captureenginenativelivetranscribeswift)
for buffering, completion and deadline limits.
