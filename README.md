# Scrawl

Show an agent a problem instead of describing it.

Press a shortcut (`⌥⇧S`) and Scrawl starts recording the screen. Talk, click around, draw on it, and press `⌥⇧S` again to finish. Scrawl records the screen, your voice, and your marks, then packs them into a compact bundle: a contact sheet of annotated keyframes (`contact-sheet.png`), individual full-resolution keyframes (`frames/frame-XX.png`), a time-aligned transcript and interaction timeline (`summary.md`), and a structured manifest (`scrawl.json`).

## Install

Scrawl needs an Apple silicon Mac on macOS 15 or later (live transcription and Liquid Glass need
macOS 26). Download the DMG from [scrawl.kitlangton.com](https://scrawl.kitlangton.com), or use
Homebrew:

```sh
brew install --cask kitlangton/tap/scrawl
```

The app updates itself (Sparkle), so Homebrew leaves updates to it.

On Windows 10 (2004) or later (x64), download the zip from the same page or the
[latest GitHub release](https://github.com/kitlangton/scrawl/releases/latest), unzip it and run
`scrawl.exe`. The Windows build is a preview and isn't signed yet, so Windows may say "Windows
protected your PC": click **More info**, then **Run anyway**. It doesn't update itself yet; see
[Windows](#windows).

## Agents

Install the agent skill for Claude Code, OpenCode, Codex and other agents:

```sh
bunx skills add kitlangton/scrawl
```

The `scrawl` command is the binary inside Scrawl.app. The Homebrew cask links it onto your PATH, or click
**Install** beside Command-line tool in Settings › Agents, which links it into `/opt/homebrew/bin`
or `/usr/local/bin` when you can write there, else `~/.local/bin`. The link always runs the app's
current copy, so it updates with the app.

With the skill, an agent that gets a Scrawl paste reads the contact sheet and transcript, opens the
keyframes the narration points at, and uses the command to look closer: `scrawl zoom <bundle> --at
0:12` (or `--at "a phrase you said"`, or `--mark 2` for the mark the text shows as ✎②; `scrawl
inspect --marks` lists them) makes a dense sheet of frames around a moment, `scrawl
frames` pulls a range, `scrawl send` queues a capture into an OpenCode session, and `scrawl export`
makes a shareable `.mp4`. `scrawl skill` prints the skill without installing anything.

## Usage

```sh
# Launch the GPUI menu-bar and review app (press ⌥⇧S to record)
cargo run -- app

# Generate a realistic multi-frame demo bundle and print its agent summary
cargo run -- demo --out /tmp/demo.scrawl

# Open a bundle (or `latest`) in the native GPUI review window
cargo run -- preview /tmp/demo.scrawl

# Print the agent summary or JSON manifest for a bundle
cargo run -- inspect /tmp/demo.scrawl
cargo run -- inspect /tmp/demo.scrawl --json

# Extract frames in a time window for an agent to look closer
cargo run -- frames /tmp/demo.scrawl --from 0:02 --to 0:07

# Copy contact-sheet.png + summary.md to the clipboard or send to OpenCode V2
cargo run -- send /tmp/demo.scrawl --clipboard
cargo run -- send /tmp/demo.scrawl --session ses_...

# Which session a capture of a dev site would go to (without sending)
cargo run -- route --url http://localhost:5173/

# Capture a cabled iPhone for 8 seconds from the terminal (mirror window + native video)
cargo run -- record --iphone --duration 8
cargo run -- devices

# Build a bundle from a ScrawlKit in-app recording (newest pair in ~/Downloads by default)
cargo run -- import
cargo run -- import ~/Downloads/20261013T164000.mp4 --timeline ~/Downloads/20261013T164000.json

# Rebuild an existing bundle with the current transcriber and keyframe rules
cargo run -- reprocess latest --out /tmp/again.scrawl

# Build a capture whose build never ran (the app died first), in place; the app does this at launch
cargo run -- recover ~/Library/Application\ Support/scrawl/captures/scrawl-….scrawl

# Tune auto-crop on a Screen capture, live
cargo run -- autocrop latest --serve --open

# Export a bundle as a shareable .mp4 (crop to the activity, ink burned in, audio; --zoom for auto-zoom)
cargo run -- export latest --zoom --out /tmp/demo.mp4

# Clean up old captures now (the app does it daily), or keep one forever
cargo run -- clean --dry-run
cargo run -- pin latest
```

## Welcome and permissions

On first launch Scrawl opens a welcome window with four screens: a welcome, how ⌥⇧S works, one
screen with every permission Scrawl needs, and a first capture. Each permission is a row with a
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

Scrawl never asks for Input Monitoring. Clicks, drags and scrolls in other apps come from an AppKit
global mouse monitor (`NSEvent.addGlobalMonitorForEvents`), which needs no permission; ⇧ for ink
and whether ⌥⇧ is still held after ⌥⇧S come from polled modifier flags, which need none either; key presses
come from an active event tap, which Accessibility allows. macOS applies an Accessibility grant to
a running app at once (System Settings shows no "Quit & Reopen" for it), and Scrawl makes the tap
within half a second of it.

Screen Recording and Accessibility are lists you drag an app into, so their button skips Apple's
prompt: it opens the pane in System Settings, and a card flies from the button to just below the
Settings window (or, with no room on screen there, over the window's empty lower area), with its
Scrawl row lined up under the list. Drag that row into the list. The card follows the window,
steps aside when another app is in front, and shows "Scrawl is allowed" once a fresh process sees
the grant (a drop alone proves nothing). The row drags the `Scrawl.app` that contains the helper,
the bundle macOS grants. With Reduce Motion on, the card fades in place. Microphone and Camera
can't be dragged in, so they use the macOS prompt.

Camera is asked for in the same window, with just its row, when first needed. Speech recognition
isn't asked for: SpeechAnalyzer transcribes without it. The ⌥⇧S and ⌥⇧V hotkeys themselves need
no permission (Carbon `RegisterEventHotKey`).

Until the required permissions are on, ⌥⇧S starts nothing (no picker or overlay that
could sit over a macOS prompt): once the keys are released it brings the permissions window
forward.

A Screen Recording grant may not reach a running app, and macOS offers **Quit & Reopen** or
**Later** as soon as it's granted. Either way nobody reopens Scrawl by hand. If you pick **Quit &
Reopen**, macOS restarts Scrawl and the tour opens on the permissions screen again, Screen
Recording checked. If you pick **Later** (or nothing), Scrawl checks whether it can capture anyway
(a one-frame ScreenCaptureKit screenshot, tried only once a fresh helper reports the grant), and if
it can't, the row shows "Reopening Scrawl" for a second and Scrawl relaunches itself into the same
screen. It only does that once in two minutes (`relaunchedAt` in the config); if a fresh copy
still can't capture, the row's button becomes **Reopen Scrawl**. The relaunch waits for this copy
to exit and opens the app without `-n`, so if macOS's Quit & Reopen got there first there's still
one Scrawl. `scrawl sck-probe [--watch SECONDS]` and `scrawl-helper sck-probe` try the same
capture from the command line.

Progress is `"onboarding"` in `~/.config/scrawl/config.json`, including the screen the tour is on,
so it picks up there whatever quits Scrawl. Once seen, the tour only comes back from **Welcome
tour** in Settings › About, but a required permission that was granted and is gone again reopens the
permissions screen at launch. `SCRAWL_NO_ONBOARDING=1` skips it.

To replay it from scratch:

```sh
tccutil reset All dev.kitlangton.scrawl
jq 'del(.onboarding)' ~/.config/scrawl/config.json > /tmp/c.json && mv /tmp/c.json ~/.config/scrawl/config.json
# then quit and reopen Scrawl
```

Preview every screen without the app, including the permissions screen with none, some and all
granted and each on-demand sheet: `target/debug/scrawl-helper onboarding --render /tmp/onboarding
--icon assets/app-icon.png`.

## Settings

**Settings…** in the menu bar (⌘, while the menu is open) opens a small window with six panes:

- **General**: Launch at login, Send dev-site captures to OpenCode, and the two shortcuts, Record
  (`⌥⇧S`) and Paste last capture (`⌥⇧V`, which can be turned off).
- **Recording**: the microphone, with its input level, the transcriber (Apple Speech with its
  language, Hex while it runs, with its model and language, or Command when it's configured) and,
  under Experimental, Video export.
- **Storage**: how long recordings are kept, where new captures go (Change…/Reset on macOS, `"capturesDir"` in
  `~/.config/scrawl/config.json` everywhere; captures already made stay put, and an unusable folder falls back to the built-in one), how much space they take, and Show in Finder.
- **Permissions**: the onboarding checklist, so a permission can be turned on later.
- **Agents**: Install or Uninstall the `scrawl` command (see [Agents](#agents)), and the skill's
  install command with Copy. A `scrawl` that belongs to another program is left alone.
- **About**: the version, Check for Updates…, links, and the welcome tour.

To change a shortcut, click its keys and press the new combination; `Esc` cancels. It needs ⌘, ⌥
or ⌃ (a function key can stand alone), and Scrawl turns down ⌘ with one key, combinations macOS
uses, the other Scrawl shortcut, and anything another app has already taken. The arrow beside the
heading puts the default back. While the recorder listens, Scrawl's own shortcuts are paused. They
are saved as `"shortcuts"` in `~/.config/scrawl/config.json`. The menu keeps Record Screen, Choose
What to Record…, Open Latest Capture, Export Last Capture as Video… (when on), Settings…, Check
for Updates… and Quit.

**Microphone** lists the connected inputs, with System default first, which follows whatever macOS
uses as it changes. The list updates as devices are plugged in or out. A chosen device is saved by
its CoreAudio UID as `"microphone": {"id", "name"}`; if it isn't connected when you record, Scrawl
records from the default instead and says so under the popup (and once in the log). The level
meter runs only while the Recording pane is on screen and Microphone is allowed. Cabled-iPhone
narration uses the same microphone. From the terminal, `scrawl mic list` shows the inputs and the
one recordings will use, `scrawl mic set <id|default>` chooses, and `scrawl mic watch` prints the
list again on every change.

Preview every pane, dark and light, with the recorder listening and a combination turned down:
`target/debug/scrawl-helper settings --render /tmp/settings --icon assets/app-icon.png`.

## Capture Controls

- **Press `⌥⇧S`**: record the display under the pointer as soon as you let go, however long you
  held it, auto-cropped (see below). Press it again to finish; a press within 0.4 s of the start is
  ignored, so a double press doesn't leave an empty capture. `⌃⌥⇧S` also works (only while `⌥⇧S` is the
  shortcut; see [Settings](#settings) to change it). As it starts, the island opens onto the modes (Area · Window ·
  **Screen** · iPhone when one is cabled) for about 3.5 s, or until you click, scroll or draw;
  hovering keeps them. Click Window or Area to retarget (the first seconds are dropped and the
  picker takes over: click a window or drag an area), iPhone to record the phone instead, or just
  carry on.
- **Choose What to Record…** in the menu bar: the picker (Area, Window, Screen, iPhone) opens on
  the display under the pointer. Click a mode, click a window or drag an area; `Esc` cancels.
- `⌥⇧V`: Paste both `contact-sheet.png` and `summary.md` into the active app or terminal (`⌃⌥⇧V` also works while it's the default)
- **Normal pointer**: Clicks and scrolls pass straight through to the app underneath
- **Hold `⇧` (Shift) + move**: Draw a persistent red marker stroke (`#ff3b30`) on screen that fades out after release
- **`⇧`-click**: Drop a numbered pin where you click (a press that moves under 12 pt and lets go within 1 s; longer is ink). It shows as a brief red pulse, not a lasting dot, and lands on the keyframes. A click after you've drawn in the same `⇧` hold is part of the drawing and drops no pin
- **`⌘Z`**: Undo the last stroke
- **`Esc`**: Cancel the active selection or capture

While picking a target, the island shows the modes **Area**, **Window**, **Screen**, and **iPhone**
(when one is cabled). It opens on the mode you used last:

- `1`–`4` jump to a mode; `Tab`/`→` and `⇧Tab`/`←` step through them; `Space` toggles Area and Window
- Area: drag a region. Window: click a window. Screen and iPhone: `⏎`, `S`, `⌥⇧S`, **Record**, or
  clicking the selected mode again starts recording (`⏎`/`⌥⇧S` record the full screen from Area and
  Window too)

## Sending captures of a dev site to its session

A capture that ends with a browser in front showing a project's dev site goes straight to that
project's OpenCode session, the same way iPhone recordings do. It is still copied to the clipboard.
The island says "Sending to SCRAWL…", then "Sent to SCRAWL", or "Copied · couldn't reach SCRAWL"
when OpenCode didn't take it (the app retries it, as for iPhone recordings). Window captures use the
captured window's app instead of the one in front. Brave, Chrome, Arc, Edge, Vivaldi, Chromium, and
Safari work; macOS asks once whether Scrawl may control the browser, so it can read the tab's
address. If you decline, captures stay on the clipboard.

Scrawl picks the session in this order:

1. **The page registered itself.** Add one dev-only line to the page:

   ```html
   <script src="http://127.0.0.1:47777/sdk.js" data-session="ses_…"></script>
   ```

   or `data-directory="/abs/path/to/project"` to use that project's most recently active session.
   The script comes from Scrawl itself. It registers the tab, sends a heartbeat every 30 s while the
   tab is visible, and unregisters when the tab closes (a registration lasts 2 minutes after its
   last heartbeat). With Vite, inject it only in dev:

   ```ts
   // vite.config.ts
   plugins: [{
     name: "scrawl",
     apply: "serve",
     transformIndexHtml: () => [{
       tag: "script",
       attrs: { src: "http://127.0.0.1:47777/sdk.js", "data-directory": process.cwd() },
       injectTo: "head",
     }],
   }]
   ```

   The script does nothing unless the page is on `localhost`, `*.localhost`, `127.x`, or `[::1]`.
   Scrawl accepts a registration without a token only from a loopback origin calling from this Mac,
   and only for that origin. A page anywhere else (a `*.ts.net` dev host, say) must also set
   `data-token` to the ingest token (`ingestToken` in `~/.config/scrawl/config.json`).
2. **Zero setup for localhost.** If the tab is on a loopback host and no page registered, Scrawl
   finds the process listening on that port (`lsof`), its working directory and that directory's
   git root, and sends to the most recently active open root session in the repository (any folder
   in it). Outside a git repository only a session in that exact folder counts.
3. Otherwise the capture is copied to the clipboard only, as before.

The destination is recorded in `scrawl.json` (`destination: {session, title, via: "sdk" |
"localhost", url, directory}`) and on the second line of `summary.md`
(`For session: SCRAWL (ses_…) · page http://localhost:5173/`). `scrawl send` without `--session`
sends to it again, and refuses a different `--session` unless you pass `--reroute`.

To see where a capture of a URL would go, without sending anything:

```sh
scrawl route --url http://localhost:5173/
scrawl route --browser com.brave.Browser   # the browser's front tab
```

Turn it off with **Send dev-site captures to OpenCode** in Settings › General, or `"autoSend": false` in
`~/.config/scrawl/config.json`. `SCRAWL_NO_OPENCODE=1` also turns it off.

A capture sent to a session that's busy joins the turn it's working on, the way a message typed
while it works does. To have it wait until the turn ends instead, set `"delivery": "queue"`.

## Auto-crop (Screen captures)

Instead of picking an area up front, record the whole screen: tap `⌥⇧S`, or choose **Screen**.
Area and Window captures aren't cropped.
Scrawl works out where you were gesturing and crops each keyframe to it. While a Screen capture
records, Scrawl logs:

- **`windows`** in `scrawl.json`: the on-screen windows, front to back (id, app, bundle ID, title,
  bounds in capture points, layer, frontmost), including the menu bar and the Dock's strip. A
  snapshot is taken at the start, when a click, ink stroke or shortcut starts, when the frontmost
  app changes, at most every 500 ms while the cursor moves and every 2 s otherwise, and stored only
  when the list changed. Each snapshot costs about 0.5 ms. Titles stay in `scrawl.json`; they never
  go into `summary.md` or the clipboard.
- **`cursor`**: the cursor at up to 15 Hz while it moves, as `[t, x, y]` (normalized), so dwell
  counts.

When the capture finishes, each keyframe gets the slice of the timeline halfway to its neighbours,
and `src/autocrop.rs` weighs what happened in it: ink, clicks, drags and scrolls first, then where
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
- `autocrop` in `scrawl.json`: each keyframe's crop in points, its kind and reason, and the
  changed regions that fed it.

`scrawl zoom` crops its cells like the nearest keyframe (`--full` for the whole screen).
`scrawl reprocess <bundle> --out <dir>` re-runs it on an existing Screen capture from its video.
To turn it off, or change a setting, use `~/.config/scrawl/config.json`:

```json
{ "autocrop": false }
{ "autocrop": { "padding": 24, "minW": 480 } }
```

### Tuning

`scrawl autocrop <bundle|latest> --serve --open` opens a playground that runs the same Rust crop as
you move its sliders: each keyframe with its windows, activity, cursor heat, outliers and crop, the
cropped frame, and the cropped contact sheet. **Copy settings** gives the JSON for the config.
Without `--serve` it writes `autocrop/index.html` with the presets only.

The settings were tuned on simulated captures with hand-labelled ideal crops: one maximized
terminal, two windows side by side, an arrow from one window to another, a tiny badge in a huge
window, a menu-bar menu, talk-only keyframes, a dragged window, overlapping windows, thirty seconds
hopping between three windows, a stray cursor, a full-screen app, and a scrolling page.
`scripts/autocrop-corpus.mjs` renders them with Playwright (from `site/node_modules`), `scrawl
fixture` builds each into a real bundle through the capture pipeline, `scrawl autocrop-eval` scores
the crops (coverage of the ink and clicks, overlap with the ideal, context, legibility in the sheet,
stability; `--sweep` searches the settings), and `scripts/autocrop-gallery.mjs` renders a gallery to
judge them by eye:

```sh
bun scripts/autocrop-corpus.mjs /tmp/corpus
for s in /tmp/corpus/*/scenario.json; do
  scrawl fixture "$s" --out "/tmp/bundles/$(basename "$(dirname "$s")").scrawl"
done
scrawl autocrop-eval /tmp/bundles --json /tmp/scores.json   # add --sweep to search
bun scripts/autocrop-gallery.mjs /tmp/gallery /tmp/bundles --scores /tmp/scores.json
```

`autocrop-eval --export src/autocrop-corpus.json` refreshes the corpus the unit tests score.

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

Phone bundles are ordinary bundles: `scrawl.json` adds `"source": "iphone"` and a `device` object
(name, capture `mirror` or `scrawlkit`, id, native size, screen points and scale, app and version
for SDK recordings), and the target's `mode` is `device`. Keyframes fit a 1600 px long edge
(736×1600 for a 1320×2868 phone); `scrawl frames` and `scrawl zoom` extract more frames from the
video when you need them.

### ScrawlKit (iOS)

`ScrawlKit` (`Sources/ScrawlKit`) is a Swift package (iOS 18+) for recording inside your own iOS app:
video of the app with narration, touches and freehand red strokes, markers, and app events on the
same clock, sent straight to Scrawl on your Mac.

Add this repo as a Swift package dependency (product `ScrawlKit`) and attach `.scrawl()` to your root
view:

```swift
import ScrawlKit

WindowGroup { ContentView().scrawl() }   // three-finger long press starts and stops
Scrawl.event("Done tapped")              // optional: what the app did, on the timeline
```

The host app needs `NSMicrophoneUsageDescription` in its Info.plist for narration (both capture
methods use the microphone). Set `Scrawl.shared.narration = false` to record without it.

`Scrawl.shared.captureMethod` picks how the video is made:

- **`.appFrames`** (default): ScrawlKit draws the app's own windows into an H.264 `.mp4` at 30 fps
  and 2× (880×1912 on a 440×956-point phone), with the microphone recorded alongside. There is no
  screen-recording prompt; the microphone asks once and iOS remembers the answer. If it's denied, the
  recording is video only and the timeline says `"audio": false`. The strip and ink are left out of
  the video (Scrawl on the Mac draws touches and strokes from the timeline), but so is
  anything the app doesn't draw itself: system alerts and permission prompts, and some layers (video
  players, camera previews, some Metal or backdrop effects) may be blank or look different. The
  system keyboard is drawn by another process, so its keys can't be captured: a plain panel labelled
  "Keyboard" stands in for it, following it as it slides (taps on the keyboard never reach the app,
  so they aren't on the timeline either). Sending the app to the background finishes and sends the
  recording.
- **`.replayKit`**: ReplayKit's in-app screen recording, the exact pixels on screen with the strip
  and the keyboard in them. iOS asks "Record screen and microphone?" unless you allowed it in the
  last ~8 minutes, and again after every reinstall.

The timeline records which was used (`"capture": "appFrames"` or `"replayKit"`). With either, the
context records the keyboard: `keyboard: visible` if it was up at the start, and
`keyboard → visible` / `keyboard → hidden` when it comes and goes.

On paired devices, the three-finger trigger turns off iOS's three-finger text-editing gestures in
the app (UIKit's `editingInteractionConfiguration` answers `.none` for the app's windows), because
the same press would otherwise open the Undo · Cut · Copy · Redo bar when a text field is focused.
The cost is that three-finger undo, redo, cut, copy, and paste don't work in that app on a paired
phone. Set `Scrawl.shared.suppressesSystemEditingGestures = false` to keep them. Unpaired devices
are unaffected.

While recording, a slim strip sits on the right edge (drag its dot to move it up or down or over to
the left edge; it remembers where you left it). From top to bottom: the recording dot (hold it to see
the elapsed time), Draw (tap to turn drawing red strokes on or off), Discard (with a few seconds to
Undo), and Send. Doing the three-finger trigger again also sends. The strip moves like liquid (the
"Liquid morph" variant of `site/public/prototype/strip`): it stretches out of a droplet, pours into
the button you tap to become a status circle, buds the "Sent to …", "Discarded · Undo" or "Mac didn't
answer · Share" bubble off on a neck, and stretches as you throw it. On iOS 26 it's Liquid Glass and
the neck is the glass itself blending; Reduce Motion cross-fades instead.

Drawn strokes are smooth and fade on their own:

- **Sampling.** Every coalesced touch is a sample, at the touch's own timestamp, so a busy main
  thread doesn't make strokes jagged. A one-euro filter (one cutoff from the 2D speed) takes out
  jitter as they come in, and a centered smoothing takes out the rest behind the tip without lag.
- **Look.** The line is a centripetal Catmull-Rom curve, 4.6 pt wide when drawn slowly and down to
  55% when fast (from speed, never pressure, like the Mac's ink), with tapered ends.
- **Drawing.** Finished stretches go into layers that never change, so each touch redraws only the
  live tip, which runs ahead through iOS's predicted touches. Predicted points are dropped on the
  next touch and never recorded.
- **Fading.** A stroke fades out (300 ms, ease-out) 1.5 s after the pen last lifted. Putting the pen
  down again before then holds every stroke on screen, so a circle and then an arrow fade together.
- **Timeline.** It gets the smoothed, unpredicted points, plus `fadeAt` (when the stroke began
  fading) on each stroke that faded during the recording. On keyframes the Mac draws a stroke from
  when it began until 1.2 s after it ended, or with the rest of its mark.

Touches aren't drawn on screen in either capture method. Each touch goes on the timeline (position,
phase, and the label of the element under it) and Scrawl on the Mac draws taps and swipes from it.

ScrawlKit stays completely inert on devices that haven't been configured (no gesture recognizers,
overlay window, network calls, files, or background `URLSession`). To pair your iPhone with this Mac, print a setup link (and
terminal QR code) and open it on the phone:

```sh
scrawl link --scheme updown [--session ses_…]
# updown://scrawl?url=http%3A%2F%2F100.64.0.12%3A47777&url=http%3A%2F%2F192.168.1.20%3A47777&token=…
```

`scrawl link` reads (or generates with mode `0600`) `ingestToken` in
`~/.config/scrawl/config.json` and defaults to this Mac's Tailscale and LAN IPs on port `47777`.
Opening `<scheme>://scrawl?reset=1` on the phone clears its Keychain config (`dev.kitlangton.scrawl`).
For cabled USB fallback via `devicectl` (`Documents/Scrawl`), list the app bundle IDs in
`~/.config/scrawl/config.json` under `"usbApps": ["com.kitlangton.UpDown"]`.

The Mac answers an upload once it has built the bundle and queued it into the OpenCode session (or
after up to 45 s), so the strip says what happened:

| On the Mac | Reply | The strip |
| --- | --- | --- |
| Queued into the session | `delivered: true, state: "sent"` | Sent to UP DOWN |
| Bundle saved, OpenCode didn't take it (retried from the Mac) | `delivered: false, state: "queued"` | On your Mac · Sends to UP DOWN when it can |
| Still building or sending when the wait ran out | `delivered: false, state: "processing"` | On your Mac · Sending to UP DOWN… |
| Bundle saved, no session for this app (or routing off) | `delivered: false, state: "saved"` | Saved on your Mac |
| Bundle couldn't be built | HTTP 500 | Mac didn't answer · Share |

A queued bundle gets `.deliver.json` and the Mac retries it every 15 s, backing off to 30 min, for
a day (then it's renamed `.deliver-failed.json`; both are logged to `~/Library/Logs/Scrawl/scrawl.log`).
The phone doesn't upload it again. Over USB the Mac writes the same reply to `<stamp>.delivery`
before `<stamp>.sent`. Replies without `delivered` (Macs from before this) read as sent.

You can also AirDrop the `.mp4` and `.json` pair to the Mac and run `scrawl import`:

- Touches become taps and swipes, drawn on keyframes as ripples and finger trails and listed in the
  review timeline. Each tap or swipe is a keyframe candidate, like a click.
- Markers become flags.
- App events (`Scrawl.event("Done tapped")`) appear in `summary.md` under the keyframe that was on
  screen when they happened (`  · 0:12 app: Done tapped`).

#### Destination and recovery

The original `timeline.json` stores the paired destination in its top-level `session` field.
`context.session`, when the app supplies it, describes the session being viewed and does not choose
the recipient. For example, recording OpenCode Settings while viewing another project's session
can still send to the paired OpenCode iOS coordinator. Automatic Mac ingest resolves the top-level
destination first; without one, it uses the app/session fallback in `resolve_opencode_session`.

When recovering a failed delivery, read that original top-level field and check whether the capture
already arrived before sending again. `scrawl send --session` explicitly overrides the recipient;
do not fill it from `context.session`. If there is no recorded destination and ownership is unclear,
ask before forwarding. Keep private-project endpoints and recipients unchanged.

#### Sending to an HTTPS endpoint

A setup link can point at a hosted endpoint instead of a Mac, for phones that aren't on your LAN or
tailnet (for example `ledger://scrawl?url=https%3A%2F%2Fproblems.example.com%2Fapi%2Fscrawl%2Fledger&token=pbcap_…`).
It speaks the same protocol as the Mac: `GET {url}/health` and `POST {url}/ingest` with a bearer
token, where the body is a 4-byte big-endian timeline length, the timeline JSON, then the MP4. When
any configured URL is `https`, recordings go through a durable outbox instead of the LAN path
(plain `http` URLs keep the direct POST plus USB fallback):

- **Saved before sending.** On Send, the framed payload and a small metadata file (target,
  `recordedAt`, attempts, last error) are written to `Application Support/ScrawlOutbox`. They're
  protected until the phone's first unlock, so uploads can run while it's locked, and excluded from
  backup. The recording never goes to `Documents/Scrawl` (it's staged in `tmp`) and isn't added to
  `Scrawl.shared.recordings`. An entry is deleted only after a 2xx.
- **Background upload.** Uploads are file uploads on a background `URLSession`
  (`dev.kitlangton.scrawl.outbox.<bundle ID>`), so they survive the app being suspended or killed
  and never load the movie into memory. The session is recreated whenever the app is configured, and
  it picks up uploads still in flight from an earlier launch.
- **Retries.** A 401 or 403 (the link was revoked), a 413 (too large) and any other 4xx except 408
  and 429 (the server rejected these exact bytes) are permanent: the entry stays on disk, marked
  failed, with Share. Anything else (offline, timeouts, 408, 429, 5xx) is retried
  with backoff (15 s, doubling up to 30 min), and immediately when the network comes back, the app
  comes to the foreground, or it launches. An entry never uploads twice at once. The server dedupes
  on person and `recordedAt`, so resending after a lost reply is safe. Opening a new setup link
  retries the entries that failed with the old one.
- **Strip.** While the upload runs on a working network (`NWPathMonitor` says `.satisfied`), the
  strip stays up as a small status circle with a progress ring, saying "Sending…" however long it
  takes. Touches on it reach the app. With no bytes going out for 30 seconds it says "Still
  sending…", never "Queued". When the upload lands it turns into "Sent to …" with a success haptic.
  "Queued · sends when online" (with a success haptic, since the recording is safe on the phone) is
  only for no network or a failed attempt waiting to retry; it leaves after a few seconds, and if a
  retry starts while it's up it goes back to "Sending…". If the upload lands after the strip has
  gone (a new recording took over, it was queued, or the app was in the background), the strip comes
  back briefly with "Sent to …", as soon as the app is in front. Permanent failures say "Scrawl link
  expired · Ask Kit for a new one", "Too large to send" or "Couldn't send", with Share, which shares
  copies of the recording and leaves the entry queued.
- **Revoked links.** The `/health` check at the start of a recording also goes to the HTTPS
  endpoint. A 401 there shows "Scrawl link expired" beside the strip, but the recording carries on
  and is kept in the outbox, marked failed, until the link is renewed.

The endpoint rejects bodies over 95 MB, so HTTPS recordings are smaller and capped:

| Capture | Video | Audio | Per second | Cap | At the cap |
| --- | --- | --- | --- | --- | --- |
| `.appFrames` to a Mac | 6 Mbps | 96 kbps | ≈ 0.76 MB | none | — |
| `.appFrames` to HTTPS | 3 Mbps | 96 kbps | ≈ 0.39 MB | 3:00 | ≈ 70 MB |
| `.replayKit` to HTTPS | not settable (est. 8–12 Mbps) | | ≈ 1–1.5 MB | 0:45 | ≈ 45–68 MB |

3 Mbps at 880×1912 and 30 fps is about 0.06 bits per pixel. That's plenty for app UI, which is mostly
static between scrolls and transitions, and the 25 MB of headroom covers the encoder overshooting its
average. Ten seconds before the cap, the strip shows "Length limit · Sending in 10 s" with a warning
haptic, then sends. A payload that's still over the limit is never uploaded: it's marked "Too large
to send" and kept for Share.

Apps with an app delegate can optionally forward background session events, so iOS can relaunch a
killed app to finish reporting uploads. Without this, finished uploads are picked up at the next
launch:

```swift
func application(_: UIApplication, handleEventsForBackgroundURLSession identifier: String,
                 completionHandler: @escaping () -> Void) {
  Scrawl.handleBackgroundURLSessionEvents(identifier: identifier, completionHandler: completionHandler)
}
```

#### App context

The app can say which screen it's on and add its own key/values, so the agent doesn't have to guess
from pixels. ScrawlKit sends only what the app passes in, plus its bundle ID, name, version, and build.

```swift
SettingsView().scrawlScreen("Settings › Experimental")   // while visible; restores the previous on disappear
SessionView().scrawlContext("session", session.id)       // follows the value; nil leaves it to enclosing views
Scrawl.screen("Onboarding")                              // or set either one directly (nil clears)
Scrawl.context("server", "studio")
```

The most recently appeared screen that is still visible wins, so pushed screens, sheets, and tabs
work. The timeline gets `shortVersion`, `context` (values at the start), `contextChanges`
(`[{"t", "key", "value"}]`, `null` when cleared), and `screens` (`[{"t", "name"}]`, the first at `0`).
The bundle keeps them as `context`, `contextChanges`, and `screens` spans (`{t0, t1, name}`), and
`summary.md` (and the prompt queued into OpenCode) shows them:

```
[scrawl 0:25 · iPhone · OpenCode 1.4 (354) · /…/20261014-101500.scrawl]
Open contact-sheet.png and frames/ in that folder before changing anything: the marks and words only make sense on the images, which can show more than the text does.
Screen: Sessions · server: studio · session: ses_3f…
- #1 (0:01) “So here.”
- #3 (0:08) ⟨Settings › Experimental⟩ [tapped “Back”] “Go back.”
  · 0:09 server → mini
```

### Headless ingest server (`scrawl serve`)

On an always-on Mac (such as a Mac Studio reachable over Tailscale), run `scrawl serve` to accept
authenticated ScrawlKit uploads, build `.scrawl` bundles, and route them straight into local
OpenCode sessions without running the GUI app:

```sh
scrawl serve [--port 47777] [--usb]
```

To run `scrawl serve` under `launchd`, copy `scripts/scrawl-serve.plist` into `~/Library/LaunchAgents`
and load it:

```sh
cp scripts/scrawl-serve.plist ~/Library/LaunchAgents/dev.kitlangton.scrawl.serve.plist
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.kitlangton.scrawl.serve.plist
```

## Export as video

`scrawl export` turns a bundle into one shareable `.mp4`: H.264 (`--hevc` for HEVC) with AAC audio in
sync, the red ink, pins, laser and click ripples burned in at the times they happened, cropped to
where the activity was. It runs in `scrawl-helper` on AVFoundation (hardware decode and encode, Core
Image for the crop, Core Graphics for the ink), so nothing is decoded into Rust and ffmpeg isn't
needed.

```sh
scrawl export latest                              # ~/Movies/Scrawl/<bundle>.mp4
scrawl export latest --out demo.mp4 --zoom        # with Screen Studio-style auto-zoom
scrawl export latest --crop full --size 1280      # whole capture, 1280 px long edge
scrawl export latest --crop 0.1,0.2,0.6,0.5       # x,y,w,h as fractions, or pixels of the video
```

- **Crop** (`--crop auto|full|x,y,w,h`): `auto`, the default, is auto-crop's whole-recording crop for
  Screen captures and everything for window, area and iPhone captures. The output keeps the crop's
  aspect with even sides, is capped at `--size` (1920) on its long edge, and is never scaled up.
- **Zoom** (`--zoom`): the camera eases to ~1.5–1.8× toward clicks, drags, marks and laser points,
  holds while activity stays nearby (2.5 s after the last), follows the cursor once it leaves a
  central safe zone, and eases back out. It never starts a zoom while the cursor is travelling fast.
  The path is a pure function of the bundle (`src/export/camera.rs`, critically damped springs,
  no bounce); the helper only renders each frame's rectangle.
- **Audio**: `audio.wav` starts where the video does (`videoOffsetMs`); a cabled iPhone's movie
  uses its own audio track. `--no-ink` and `--no-clicks` leave the annotations out, `--fps` sets the
  rate (default 60 for a 60 fps recording).
- **Menu**: with **Video export** on in Settings › Recording (`"videoExport": true` in
  `~/.config/scrawl/config.json`, default off), the menu shows **Export Last Capture as Video…**. It saves to `~/Movies/Scrawl/<name>.mp4`, shows progress
  beside the menu bar icon, and reveals the file in Finder. The command works without the flag.

## Keeping captures small

Captures would otherwise pile up (mostly video). The app cleans up shortly after launch and then
hourly, at low priority, and logs what it did:

- **Video, audio and full-size frames** (`recording.mp4`/`.mov`, `audio.wav`, `frames-full/`),
  plus the `zoom/` sheets made from them, go to the Trash after 7 days, as one `<bundle> media`
  folder. The prompt (`summary.md`, `scrawl.json`, `contact-sheet.png`, `frames/`) stays, so
  pasting and reading still work; `scrawl zoom` says the video was cleaned up.
- **Whole bundles** go to the Trash after 30 days.
- **Keep recordings** in Settings › Storage picks Video for 7 Days (bundles 30), Video for 30 Days
  (bundles 90), or Forever. It's stored as `"retention": {"mediaDays": 7, "bundleDays": 30}` in
  `~/.config/scrawl/config.json`; `null` keeps forever. The same pane shows how much the captures
  take and opens the folder in Finder.
- Never touched: a bundle under a day old or changed in the last day (a copy just arrived from
  another Mac, a zoom), one whose media is still copying (`.media-pending` there, `.media-sending`
  here; a marker over a day old is ignored), one being recorded or re-encoded, and one with a
  `.pinned` file (`scrawl pin`, `scrawl pin --off`).
- **iPhone video** from the cable is ~18 Mbps at 1320×2868. A minute after the capture (after any
  copy to another Mac finishes, and never during a recording) `scrawl-helper shrink` re-encodes it
  to HEVC at a 1600 px long edge, audio copied as is and every frame at its original time, and
  swaps it in place: a 4.5-minute capture goes from 750 MB to 72 MB in about 20 s. One pass a day
  shrinks any native iPhone video that was missed.

`scrawl clean --dry-run` prints what a pass would do now.

## Transcription

Narration is transcribed on device by Apple's SpeechAnalyzer (macOS 26), or on macOS 15 by the
older on-device `SFSpeechRecognizer`. On macOS 15 onboarding asks for Speech Recognition, and
Dictation's on-device model has to be on the Mac (System Settings › Keyboard › Dictation); Scrawl
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

Apple's SpeechAnalyzer is the default; Scrawl prepares its language model automatically. If you use Hex
(2.2 or later, running), Scrawl can transcribe with it instead and with whichever model Hex has
ready: Settings › Recording › **Transcription** → **Hex**, then pick a model under **Hex model**.
Or in
`~/.config/scrawl/config.json`:

```json
{ "transcriber": "hex", "hexModel": "whisper_large_v3_turbo", "hexLanguage": "nl" }
```

`"transcriber"` is `"apple"` (default), `"hex"` or `"command"` (below); `hexModel` defaults to the
first ready one of `parakeet_unified_en`, `parakeet_v3`, `parakeet_v2`, `whisper_large_v3_turbo`.
Scrawl never downloads or prepares a Hex model: install it in Hex first.

**Language** in Settings › Recording picks the language Apple Speech transcribes; **Automatic**
(the default) follows the Mac's language. In config it's `"appleLocale"`, e.g. `"zh-TW"` for
Traditional Chinese (Taiwan). Scrawl downloads that language's model in the background, and a
language this Mac doesn't support falls back to Automatic (Settings names the one it uses).

**Hex language** in Settings › Recording (or `"hexLanguage"` in config) sets the language
code sent to Hex. It defaults to `en` to preserve existing behavior; use `nl` for Dutch or
`auto` for automatic detection when the model supports it. Press Return or Apply to save;
a blank field resets to `en`. Codes are not restricted to a list in Scrawl: Hex validates
model/language support. If Hex rejects a language/model combination, Scrawl uses the existing
Apple Speech fallback. This setting does not inherit Hex's dictation language. Whisper uses the selected
language to guide recognition; models such as Parakeet v3 determine language from audio
and ignore the hint.

- Hex is optional. When it isn't running, **Hex** is greyed out as "Hex (not running)" and Scrawl
  uses Apple, whatever the saved preference. If Hex fails on a recording, that recording falls back
  to Apple Speech (see `~/Library/Logs/Scrawl`).
- Hex returns text without word times, so Scrawl sends each stretch of speech separately and
  spreads its words across it by length. The transcript says `"source": "hex:<model>"` and
  `"estimated": true`; because those times are approximate, spoken cues ("now", "here") don't cut
  keyframes from a Hex transcript. Marks, clicks, laser and scene changes still do.
- `scrawl hex status` shows whether Hex is connected, its ready models and what the next recording
  will use. `scrawl reprocess <bundle> --out … --transcriber hex|apple|command` (also
  `scrawl import`) picks an engine for one run.

### Your own model (command)

Any other local model (whisper.cpp, MLX Whisper, faster-whisper, Parakeet, a Core ML model…) can
transcribe through a small wrapper program. Scrawl runs it on the recording's audio and reads a
JSON transcript from its stdout; it knows nothing about the model, adds no network requests and
doesn't sandbox the program. It runs on macOS only for now; on Windows recordings keep the local
engine. In `~/.config/scrawl/config.json` (Settings shows **Command** once it is set; editing it
stays in the file):

```json
{
  "transcriber": "command",
  "transcribeCommand": ["/Users/me/bin/whisper-scrawl", "--model", "large-v3-turbo", "{audio}"],
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
- The program gets the whole recording as one 16 kHz mono 16-bit WAV, `SCRAWL_TRANSCRIBE_VERSION=1`
  and `SCRAWL_AUDIO=<path>`, stdin `/dev/null`, and its own process group. Its stderr is discarded.
  It must not change the audio, daemonize or leave processes behind, and must cope with two runs at
  once. On timeout or more than 8 MiB of output Scrawl kills the group.
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
  cues only cut keyframes from measured words. Without `words`, Scrawl spreads the text over the
  segments (or the whole recording) and marks those times estimated, as it does for Hex.
- Spans may overlap or be zero-length; Scrawl sorts them by `t0`. Ends up to 500 ms past the audio
  are clamped. A negative, fractional or reversed span, or one further past the end, makes Scrawl
  drop that array and keep the text, spreading it as above when `words` are dropped.
- The transcript says `"source": "command:<file name of argv[0]>"`. Anything else going wrong (a
  malformed `transcribeCommand` or timeout, the program missing, a non-zero exit, a timeout, invalid
  JSON, another version) means Apple Speech transcribes that recording. Each problem is one line in
  `~/Library/Logs/Scrawl` (a malformed config only once), never with the program's output,
  arguments or what was said.

### Transcribing while recording (experiment)

```json
{ "experiments": { "liveTranscribe": true } }
```

With this in `~/.config/scrawl/config.json`, a Mac recording transcribed by Apple Speech streams the
microphone to `scrawl-helper transcribe-stream` as it records, so finishing only waits for the last
words (tens of milliseconds) instead of transcribing the whole recording. The streamed samples are
exactly the ones `audio.wav` holds, so word times are on the same clock. If the stream fails, the
speech model isn't installed yet, or it takes too long, the recording is transcribed afterwards as
usual; the log says which (`Live transcription: …`). Hex and iPhone recordings are unaffected.
`scrawl reprocess <bundle> --out … --bench 1 --live-speed 1` replays a bundle's audio through it.

## Helper

`scrawl-helper` is a small Swift executable built by `build.rs` from `helper/` and shipped inside
`Scrawl.app` next to `scrawl`. It owns the Swift-only or unbound Apple frameworks: SpeechAnalyzer,
CoreMediaIO/AVFoundation iPhone capture with the mirror window, AVFoundation movie decoding, and the
SwiftUI welcome window (`onboarding`).
Scrawl runs it as a subprocess (JSON on stdout; `stop`/`cancel` on stdin for the mirror). Run it
directly to test a piece in isolation:

```sh
target/debug/scrawl-helper devices
target/debug/scrawl-helper permissions
target/debug/scrawl-helper transcribe audio.wav
target/debug/scrawl-helper frames recording.mov --out-dir /tmp/f --at 1000,2000 --max-edge 1600
target/debug/scrawl-helper shrink recording.mov --out /tmp/small.mov --max-edge 1600
target/debug/scrawl-helper export plan.json   # the plan `scrawl export` writes (src/export)
target/debug/scrawl-helper permission-panel --permission screen-recording --preview   # the drag card, no grant polling
```

`permission-panel --permission screen-recording|accessibility [--from X,Y,W,H]`
(`--from` is the button's frame in global top-left points) prints JSON lines: `docked`, `dropped`,
then `granted` or `cancelled` with a `reason` (`back`, `settings-closed`, `no-settings-window`,
`parent`, `timeout`), and exits. On stdin it takes `cancel`, `back` and `granted`.
`permission-panel --check P` prints `{"granted":…}` from a fresh process. `permission-panel --plan --settings
X,Y,W,H --visible X,Y,W,H` prints where the card would dock (`below`, `inside`, `right`, `left` or
`clamped`); `scripts/check-permission-panel-dock.sh` checks the cases.

## Windows

Scrawl runs on Windows 10 (2004) and later as the same app: the same capsule, picker, overlay,
review window, welcome tour and Settings, from the same GPUI code. Only the parts the operating
system owns differ:

| | macOS | Windows |
|---|---|---|
| Shortcut | `⌥⇧S`, `⌥⇧V` | `Alt+Shift+S`, `Alt+Shift+V` |
| Menu | Menu bar item | Notification area icon (left or right click) |
| Screen and window capture | ScreenCaptureKit | Windows.Graphics.Capture, H.264 through Media Foundation |
| Clicks, keys, Shift ink | Mouse monitor, Accessibility key tap, modifier state | Low-level hooks and key state |
| Ink overlay | Metal panel | Click-through GPUI window, left out of recordings |
| Transcription | SpeechAnalyzer, or Hex | Parakeet on device (`transcribe.cpp`), Hex's model or Scrawl's own copy |
| Movie decoding | `scrawl-helper` (AVFoundation) | Media Foundation, in process |
| Paste | `⌘V` | `Ctrl+V` |
| Settings and welcome tour | SwiftUI (`scrawl-helper`) | GPUI windows in process, same panes and protocol |
| Recording capsule | Native AppKit + Metal pill (or GPUI) | GPUI capsule |
| Launch at login | `SMAppService` | The user's `Run` key (Settings › Apps › Startup) |
| Captures | `~/Library/Application Support/scrawl/captures` | `%LOCALAPPDATA%\scrawl\captures` |
| Log | `~/Library/Logs/Scrawl/scrawl.log` | `%LOCALAPPDATA%\Scrawl\Logs\scrawl.log` |

- **Permissions:** none, apart from the microphone switch in Settings › Privacy & security, which
  the welcome tour and Scrawl's Settings › Permissions check. Screen capture, input hooks and
  pasting need no grant on Windows.
- **Shortcuts:** both are recorded in Settings › General like on macOS, as `Ctrl`, `Alt`, `Win` and
  `Shift` combinations. Scrawl turns down `Ctrl` with one key (apps use those), `Win` with a key
  (Windows uses those), and `Alt+Tab`, `Alt+F4` and `Alt+Space`.
- **Transcription:** Parakeet runs on device with measured word timings, so spoken cues ("look
  here") cut keyframes as on macOS. If [Hex](https://github.com/anomalyco/hex) has installed a
  model (`%APPDATA%\hex\models`, or `%APPDATA%\voice-control\models` from older Hex), Scrawl
  uses it. Otherwise the first launch downloads Hex's Parakeet Unified English (730 MB, checked
  against its SHA-256) into `%LOCALAPPDATA%\Scrawl\models` in the background; Settings › Recording
  shows the progress, and captures made before it finishes get their transcript once it lands.
- **Command-line tool:** **Install** in Settings › Agents writes a `scrawl.cmd` shim into
  `%LOCALAPPDATA%\Microsoft\WindowsApps`, which is on every user's PATH, so agents and terminals
  can run `scrawl`.
- **agentContext:** what clicks and marks land on comes from UI Automation, as it does from the
  Accessibility tree on macOS.
- **Not on Windows:** the cabled iPhone (it needs AVFoundation's screen capture devices), video
  export, Sparkle updates (**Check for Updates…** opens the releases page) and the dev-site tab URL
  (`/sdk.js` registrations still route).
- **Releases:** a `v*` tag builds `Scrawl-<version>-windows-x64.zip` on GitHub Actions
  (`.github/workflows/release-windows.yml`) and attaches it to the release. It isn't code-signed
  yet; signing and automatic updates are planned.

Build with the MSVC toolchain, CMake and the Windows SDK (Visual Studio Build Tools with the C++
workload). Release builds also need GPUI's shader compiler: set `GPUI_FXC_PATH` to the SDK's
`fxc.exe` (for example `C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\fxc.exe`).

```powershell
cargo build --release
.\target\release\scrawl.exe app
```

## Testing

```sh
taskpolicy -c utility cargo test -j 3 --offline -- --test-threads=2
./scripts/install-app.sh
```

On Windows, `cargo test`; then run `scrawl.exe app` and record with `Alt+Shift+S`.
`SCRAWL_SHOW_OWN_WINDOWS=1` keeps Scrawl's own windows (capsule, overlay, picker) in screenshots,
which normally leave them out.

- **iPhone:** connect a phone, press `⌥⇧S`, click the iPhone chip, talk, draw over the mirror, press M,
  Finish. Inspect the bundle's `summary.md`, `scrawl.json` (`source`, `device`), contact sheet, and
  frames.
- **ScrawlKit:** `scrawl import` on a recording and its timeline (or hand-write a timeline for any
  phone video), then check the ripples and trails on frames and the app-event lines in the summary.
  `swift test` runs the outbox tests on the Mac against a `URLProtocol` stub of the ingest endpoint
  (persisting, retry classification, resends, deletion only on 2xx, and what the strip says through
  slow uploads, no network, and retries).
- **Transcription:** `scrawl-helper transcribe <wav or movie>` prints words with timings;
  `scrawl reprocess <bundle> --out …` reruns transcription and keyframe selection on a real bundle.

## License

MIT. See [LICENSE](LICENSE).

### THIRD_PARTY

`helper/PermissionPanel.swift` adapts code and tuning from two MIT-licensed projects:

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
