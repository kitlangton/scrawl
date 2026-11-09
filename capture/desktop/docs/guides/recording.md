# Recording guide

[← Apparent](../../../../README.md)

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
- **Agents**: Install or Uninstall the `apparent` command (see [Agents](../../../agent/docs/usage.md#agents)), and the skill's
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

## iPhone

Plug an iPhone in with a cable (unlocked, trusted) and the picker shows **iPhone** next to Area and Screen. Choosing it opens a floating mirror of the
phone on the right of the screen and starts recording:

- The phone's own video is recorded natively (1320×2868 at up to 60 fps on an iPhone Pro Max) with
  your narration from the Mac's microphone, into the bundle as `recording.mov`. Keyframes are
  decoded from that video, never from a screenshot of the mirror. A minute after the capture the
  video is re-encoded to HEVC at the keyframes' 1600 px (see [Keeping captures small](../../../engine/docs/guides/processing.md#keeping-captures-small)).
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
- **Window recording:** temporarily disabled on both platforms. Use Screen or Area.
  On KDE Wayland, desktop ink hides when Shift is released so clicks reach the app; the bundle keeps it.
- **Not on Linux:** the cabled iPhone, `agentContext` and Sparkle updates.
- **Video export on Linux:** requires `ffmpeg` and `ffprobe`; see the [export guide](../../../engine/docs/guides/processing.md#export-as-video).

See [capture/desktop/docs/linux/linux.md](../linux/linux.md) for how it works and how to build it.
