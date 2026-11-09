# Linux

## Tray appearance

The StatusNotifierItem tray uses the same boxed purple “a” as the Mac menu bar:
Lilac on Carbon in Dark mode and darker purple on Paper in Light mode. It reads
the XDG Settings portal's color-scheme at app startup and watches it while
running, including when Settings has never been opened. A change refreshes the
tray's pixmaps without restarting Apparent. If the portal is unavailable, the
initial fallback is Dark; a failed read retains the last known appearance.
No preference uses Light, matching Settings. The opaque box keeps the glyph
contrasting even when a panel is themed independently of that preference.
The app/window icon is unchanged. GNOME still requires a tray host such as
AppIndicator; without one, relaunching Apparent opens Settings as before.

Native acceptance needs the actual running icon on light and dark panels,
Light → Dark → Light without restarting, and an open tray menu in each mode.
Check an independently themed panel as well; pixel/asset tests alone do not
prove tray-host rendering.

For a reusable disposable-guest capture/save/cancel check, see
[Native testing](../native-testing.md). Its runner receipts distinguish environment,
test-driver and product failures from unrun gates; host contract tests are not
native acceptance.

The default-off native launch experiment and explicit AppImage/desktop URL-handler
registration are documented in [native launch](../native-launch.md). Opening a URL never
installs or changes the selected desktop handler.

Read this before changing `capture/engine/runtime/platform/linux.rs`, `capture/desktop/ui/linux.rs`, `src/linux/`, `capture/desktop/ink_view.rs`, `capture/engine/runtime/audio/transcription/speech.rs`, or anything behind a `platform::` call in `capture/engine/runtime/mod.rs` and `capture/desktop/mod.rs`. The shared GPUI UI uses platform adapters for the macOS and Linux shells. Windows code is retained but unsupported.

Optional [playback lowering](linux-playback.md) is a default-off, manually
configured experiment on one pinned PipeWire/WirePlumber stack. It does not
change user volume or routing and has no live effectiveness indicator.

## X11 everywhere, Wayland around it

GPUI is built with its `x11` feature only, and `linux::prepare_process` (the first thing `main` does) hides `WAYLAND_DISPLAY` from it, so on Wayland every Apparent window is an XWayland window (`APPARENT_NATIVE_WAYLAND=1` opts out, for experiments). That keeps one window model on both: Apparent's floating windows (pill, picker, ink overlay) are override-redirect X windows it places itself in global pixels (`linux::x11::float`, `place`), with an empty input shape to let clicks through (`set_click_through`). `float` waits for the window manager to finish mapping and then unmanaging the window before it takes it over; otherwise GNOME on Wayland keeps drawing it at its old frame's offset.

GPUI 0.2.2's X11 window panics on `HasWindowHandle`, so `x11::window_id` finds a GPUI window's X id itself (the newest `_NET_WM_PID` window it hasn't seen, cached per `WindowId`).

Global coordinates are root-window pixels; X11 has one scale for the screen (`x11::ui_scale`), so `MAX_CAPTURE_SCALE` is 1.

## What each session can do

Verified in an Ubuntu 24.04 arm64 VM with GNOME 46 (Wayland and X11) and Plasma 5.27.12 (Wayland). Plasma 6 and GNOME 48+ portal shortcuts still need desktop testing.

| | X11 | GNOME on Wayland | KDE Plasma on Wayland |
|---|---|---|---|
| Shortcuts | `XGrabKey` | GNOME custom keybindings running `apparent shortcut record\|paste` (GNOME 48+: the GlobalShortcuts portal) | Plasma 5: native KGlobalAccel; Plasma 6: GlobalShortcuts portal, with KGlobalAccel fallback |
| Recording a display or area | `ximagesrc` | `org.gnome.Mutter.ScreenCast.RecordArea`, no dialog | ScreenCast portal, asks once (restore token in `~/.local/share/apparent/portal.json`) |
| Recording a window | Temporarily disabled | Temporarily disabled | Temporarily disabled |
| Clicks, keys, ⇧ ink | the X server (pointer and key state, polled) | the input devices (`/dev/input`, needs the `input` group) | same |
| Pointer position | the X server | XWayland while it's over X11 windows; an absolute pointing device (tablet, touchscreen, VM or remote desktop) from the input devices; the overlay takes the pointer while ⇧ is held | same |
| App in front | `_NET_ACTIVE_WINDOW` | AT-SPI when the app exposes it (`linux::atspi`; X11 apps use XWayland) | AT-SPI when available; Kate did not expose context in the test VM |
| Paste | XTEST `Ctrl+V` | RemoteDesktop portal, asks once | same |

### Clicks and ink on Wayland

`apparent record --duration N` is a separate timed recorder, not a shortcut-driven app session. Even with input access it does not collect click or Shift ink events; its manifest and summary explicitly mark that interaction-telemetry gap. Use `apparent shortcut record` with the app to start and finish when those events matter. The permission and pointer-position limits below still apply to the shortcut path.

`apparent export <bundle> --out movie.mp4` renders a saved Linux capture as an MP4 with ink, pins, click ripples and the saved WAV when those tracks exist. Install `ffmpeg` and `ffprobe` with H.264/AAC encoders; HEVC export needs `libx265`. Export uses FFmpeg decoding even if the capture's GStreamer full-quality keyframe decoder fell back to candidate images. By default the tray also offers **Export Last Capture as Video…**, a native SaveFile destination dialog, visible busy/progress and error states, and retry after cancellation or failure. It needs a FileChooser portal backend such as GTK or KDE. Top-level `"videoExport": false` hides the action; see [Linux tray export](../../../engine/docs/export-and-retention.md#linux-tray-export). A timed CLI capture has no click/ink telemetry to burn into the movie; use the app shortcut for an annotated recording. See `capture/engine/docs/export-and-retention.md`.

Wayland tells no app where the pointer is over another app's window. Apparent reads buttons and keys from the input devices. In onboarding or Settings › Permissions, **Copy command** copies `sudo usermod -aG input "$USER"`. Run it in a terminal, then log out and back in. Apparent checks whether it can open actual keyboard/mouse devices, rather than trusting group membership; it retries devices when access changes. This is the **Input devices** row (the Accessibility row on macOS).

While ⇧ is held, the ink overlay takes the pointer (`linux::overlay::take_pointer`), so XWayland reports where it is and the ink follows it; a click while ⇧ is held lands on the overlay, not the app. On KDE Wayland, Apparent hides the recording overlay between strokes because KWin 5 otherwise blocks clicks into native apps even with an empty X input shape. Ink remains in the bundle, but disappears from the desktop when ⇧ is released.

On Hyprland 0.56.2 with Lua configuration, the released overlay stays visible for its existing fade. A process-owned runtime rule matches only its exact Fade title and prevents focus; Draw uses a different title. Each Shift transition unmaps locally before asynchronous compositor reconciliation, so a late reply cannot remap an older input state. Reloading config hides and re-establishes the rule; unsupported syntax or policy failure retains the hide-between-strokes fallback. Finishing unmaps before disabling the rule. The one reused rule is disabled on ordinary cleanup; a crash can leave an inert rule matching only that process's unique title until config reload. This is a native compositor repair; the current Electron surface has no production overlay interaction to migrate independently.

A click's position is right over X11 apps and with an absolute pointing device. With a mouse or touchpad over a Wayland app, XWayland still has the pointer where it last saw it, so the click's position is stale: the keyframe is cut at the right moment, but its ripple can be in the wrong place. A GNOME Shell extension that reports the pointer would close this gap (see the PR's open questions).

Without the `input` group, a capture on Wayland has no clicks, keys or ⇧ ink outside Apparent's own windows, and records only the screen and voice.

### Picker and portal limits

Screen and Area work on the tested desktops. Window recording is temporarily disabled on every backend pending original-window lifetime safety (#230); the picker omits it and `record --window` returns an error without starting a recorder or substituting a screen. Choose Screen or Area explicitly. Existing Window bundles remain readable. A portal window source exists internally, but the UI does not offer it. A press of the record shortcut records the display under the pointer once its keys come up, however long they're held; the tray menu's **Choose What to Record…** opens the picker, which stays open until you choose a target or press Esc. The Mac-only iPhone mode is hidden. Screen sharing can ask again if a desktop does not honor the saved restore token. Finishing while the dialog is open cancels the pending session; a failed or empty capture reports an error rather than producing demo imagery, and `latest` skips its unfinished directory.

`apparent capture start --wait --timeout 30` uses the running GUI's normal interaction-aware
screen path and returns its own request ID. Portal consent still applies; `capture state` reports
`starting` until a real frame arrives. An agent can cancel its owned pending ID but cannot
stop-and-save it until `recording`. `capture stop <id> --wait --timeout 60` returns the exact saved
bundle path for that ID; `capture inspect <id>` recovers its process-lifetime receipt after timeout.
See `capture/engine/docs/capture.md` for the same-user GUI control protocol on both supported platforms.

Plasma 5 can report successful GlobalShortcuts binding with an empty result. Apparent validates both returned bindings and uses KGlobalAccel directly on Plasma 5, so the shortcut does not also type into the focused app.

### Hyprland

Tried on Omarchy 4 (Hyprland 0.56, Lua config), Wayland. `linux::hyprland` binds the shortcuts at runtime with `hyprctl eval hl.bind(…)` (a hyprlang config takes `keyword bindd` instead, untested), each running `apparent shortcut record|paste`. Hyprland consumes the chord, so it doesn't also reach the app in front, and nothing needs the `input` group. The binds are described `Apparent: record` and `Apparent: paste`. A chord some other bind holds is left alone and reported as taken; a bind with Apparent's description (a permanent one in the user's config, which a config reload doesn't drop) is left in place:

```lua
hl.bind("ALT + SHIFT + S", hl.dsp.exec_cmd("apparent shortcut record"), { description = "Apparent: record" })
```

A config reload drops runtime binds, so Apparent binds them again on the event socket's `configreloaded`. The GlobalShortcuts portal can't stand in: `xdg-desktop-portal` refuses an app without an app id, and `xdg-desktop-portal-hyprland` only fires shortcuts the user bound by hand with `bind = …, global, <app id>:<name>`.

Clicks on the pill and the picker need four things, found in Hyprland's source (`ViewHitTester.cpp`, `XSurface.cpp`) and Omarchy's `windows.lua`, because the symptom is always the same: the click lands on the app below.

- Omarchy gives `no_focus` to floating XWayland windows with an empty class and title, which Hyprland's hit test skips. GPUI sets no class on these windows, so `x11::accept_pointer` gives them one.
- For an override-redirect window of `_NET_WM_WINDOW_TYPE_NOTIFICATION` (GPUI's pop-up type), tooltip, menu, splash or utility, the hit test answers "the window in focus" instead of the window. `accept_pointer` makes them toolbar windows, which get the pointer and aren't focused when they map.
- Hyprland takes an override-redirect window's position when it maps it and ignores later moves, so `x11::place` maps the window again when it moves on Hyprland.
- The full-screen ink overlay is a notification window above the pill, so it shares the KDE treatment: mapped only while ⇧ is held (`hide_between_strokes`).

Hyprland also assigns override-redirect windows to a workspace. The recording capsule
is pinned across workspaces after mapping (and again after a move remaps it), without
focusing it or pinning Settings, the picker, whiteboard or ink overlay. The adapter
sets an X11-ID-specific title on the capsule and matches that title plus its process
ID in `clients -j`; it never guesses from the active window. A missing identity
or failed pin is retried briefly and logged, not replaced with a broad app rule.
Verified with Hyprland in the isolated Omarchy VM. Lua uses an explicit set-pin
action; legacy hyprlang dispatches pin only after observing the capsule unpinned.

XWayland sees the pointer only over X11 windows and keeps the last place it saw it over the rest, which picked the wrong display for a recording and started an area selection in the wrong place. On Hyprland `linux::cursor_position` asks Hyprland (`j/cursorpos`) and maps the point to the same place on the same output in XWayland's layout (`hyprland::cursor_in_x_space`): XWayland can put a monitor at y 0 where Hyprland has it at y 1080, so outputs are matched by name and the point is moved by the difference.

The clipboard goes through `wl-copy`, as on KDE Wayland.

On Linux, the recording controls offer **Whiteboard** by default.
It opens a separate native window over the ongoing display capture: draw without holding ⇧,
use Undo or Clear, and close/reopen it without losing the current session's strokes. Resizing
reveals more of the fixed paper instead of scaling marks. On Hyprland the window is floated
after it maps so it can move and resize without tiling the browser; the recording remains
one continuous screen-and-voice session. The board appears in the captured video like any
other window. The older hyprlang dispatcher cannot safely clear a fullscreen board by
address (it acts on the focused window), so Apparent leaves that board untouched and
logs an error instead of changing another window; the tested Omarchy Lua path clears it.
At finish, a nonempty board also gets its own last contact-sheet tile: the final strokes
on white paper, cropped to their bounds for readability even if the window was closed or
small when recording stopped. The numbered tile and stroke count appear in `summary.md`;
the screen keyframes still show where the board was beside the browser. Clear removes
the drawing from this final tile. Finish closes the whiteboard window after taking its
final snapshot; the drawing stays in the completed bundle. An ordinary clipboard copy
is text only; use Paste to attach the contact sheet and text to an agent composer.

The recording capsule shows five smooth level bars by default, fed by the captured
microphone PCM. They indicate input activity only, not who spoke or whether
transcription can understand the voice.
The recording cue and Finish remain separate; microphone and intelligibility warnings
retain their own status text rather than borrowing the bars.

`autoPasteDraft` is off by default and currently fails closed: the finished capture leaves the text summary on the clipboard and the sheet in the saved bundle, without automatically pasting into a window. A Hyprland window does not identify a T3 thread or prove an attachment was accepted. Attach the sheet in the intended unsent composer and use Ctrl+V for the summary. Ctrl+V alone cannot reliably attach both image and text across apps; use the paper tile's Paste control or attach `contact-sheet.png` manually if the image is missing. The paper stays visible for review. Apparent cannot tell whether the app accepted either paste: inspect the unsent draft before sending. Ctrl+V and Super+V shortcuts are unchanged. See `capture/engine/docs/keyframes-and-text.md`.

On the two-output Omarchy VM (Virtual-1 at 1.25x and HEADLESS-1 at 1x), the picker and capsule appeared on HEADLESS-1 and a dragged Region there produced a bundle with that output's pixels, not Virtual-1. The during-drag outline and dimension chip were visible. QMP pointer input reached both outputs; during active Region recordings the capsule followed crossings in both directions without retargeting the capture. Separate clicks on Cancel and Finish on the scaled first output invoked the intended actions: Cancel left no completed bundle, and Finish saved a Region movie. The idle ink overlay was unmapped during those clicks. On X1, separate active recordings selected a Region on a temporary 1x headless output, then moved the capsule to the real 1.25x output. Synthetic clicks on visibly targeted Cancel and Finish invoked their respective actions; Cancel left no completed bundle, while Finish saved a 610×370 H.264 movie and a contact sheet showing the selected output's content. The earlier X1 Cancel-to-Finish misdispatch did not recur on this integrated candidate, though these checks cannot establish that every input route is fixed. With `movableRecordingControls` enabled on the final candidate, the capsule was dragged on the headless output; it stayed at the dropped position while the held pointer crossed to the scaled output, after release, and after another crossing. Finish on the moved capsule saved a second Region bundle with visible content. RandR's output-name atom keeps the chosen display ID stable if monitor order changes. The portal's restore choice is kept per output and, on multi-output Hyprland, its stream mapping ID must name the selected output. A wrong or unidentifiable stream fails instead of quietly capturing another display; an output added or removed while the sharing dialog is open, or a chosen output that moved or changed geometry, also requires a new selection. Other Wayland portal backends and mirrored output layouts have not been checked for the same stream-identity guarantee. Pointer position comes from Hyprland when available, falling back to XWayland; click attribution over native Wayland windows still needs a separate check.

In a further disposable mixed-scale Omarchy VM run, a Region selected on the 1.25x Virtual-1 output saved a 550×400 H.264 movie with changing terminal content from that output. The contact sheet and keyframe contained the same content (at candidate-frame quality because the VM lacks a full-quality GStreamer decoder). After the pointer moved onto the 1x output, Finish still dispatched. This establishes readable selected-output pixels for the scaled-output path; it does not establish full-quality VM keyframe decoding or other portal backends.

## Apparent's own windows are in the recording

Neither X11 nor GNOME's or KDE's screencasts let an app leave its windows out, so the pill and live ink are in `recording.mp4`. Fresh verified Hyprland screen/area movies identify their captured ink with `videoInk: "captured"`; output preserves those pixels without duplicate replay. KDE Wayland uses `"capturedVideo"` to stop movie replay while still images reconstruct marks after the live overlay hides on release. Other backends and historical bundles retain legacy replay, which can redraw and prolong captured ink. The candidate frames used to pick keyframes paint over the pill (`linux::media::own_window_masks`), so its ticking clock never counts as activity.

This is a capture-source limitation, not a missing mask in the keyframe selector. On
X11, `ximagesrc` records the already-composited root image; GNOME's `RecordArea`
and the ScreenCast portal return already-composited monitor streams. The portal's
`SelectSources` can select a monitor or a window, but cannot subtract a set of
windows from a monitor stream. The GStreamer `tee` sends the same source to the
MP4 encoder and candidate-frame sink. `own_window_masks` changes only the
candidate copies used to detect activity; full-quality keyframes are decoded
again from the MP4, so they can contain the controls too. Hiding the picker,
capsule or ink from that stream while keeping the user-opened whiteboard and
click-through/⇧ ink behavior requires compositor-side exclusion or an equivalent
source that supplies the uncovered pixels. Painting rectangles over the movie
would destroy the content beneath the controls, not exclude the controls.

Hyprland's `noscreenshare` rule draws **black rectangles** in place of matching
windows, including when another window is above them; it is not a transparent
exclusion mechanism and must not be applied to Apparent windows as a purported
fix. A separately scoped fallback could record an explicitly selected window
instead of the monitor where the backend supports it, but it would omit other
windows such as the user's whiteboard and is not equivalent to Screen/Area
capture. Keep the existing controls and mark this as unsupported until a
specific compositor/backend can produce a movie that retains the uncovered
screen and whiteboard; verify the actual MP4 and decoded full-quality frames,
not just candidate-frame selection, before changing the documented support status.

Sources: [ScreenCast portal interface](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html)
(`SelectSources` source types and options),
[Hyprland `noscreenshare`](https://wiki.hypr.land/0.50.0/Configuring/Window-Rules/)
(black rectangles).

## Movies, transcription, the rest

### Local languages

Apparent’s downloaded `parakeet-unified-en-0.6b-Q8_0.gguf` supports English
only. Both the pause worker and whole-WAV recovery currently request `en`.
`appleLocale` does not change Linux recognition; it controls Apple Speech on
macOS only. Reused Hex model files may advertise different capabilities, but
Apparent currently still requests English; their presence is not verified
non-English support.

For another language, explicitly configure a **local** `transcribeCommand`
with that recognizer’s model and language arguments, then select **Command**
in Recording Settings. The command must implement Apparent’s versioned JSON
transcript contract; ordinary text output is not sufficient. Apparent does not
supply a language argument to the command or install its model. Inspect
`transcriber` and `transcribeCommand` in `~/.config/apparent/config.json`.
Change them while idle: finalization reads the engine and command again, so an
edit during recording can affect that recording. See the
[command contract](../../../engine/docs/keyframes-and-text.md#transcription-captureengineruntimeaudiotranscriptiontranscribers-sharedhexrs-captureengineruntimeaudiotranscriptioncommand_transcriberrs).
If Command or Hex fails, the existing fallback is local Parakeet, whose default
model is English-only. This is unsupported-language guidance, not a claim of
verified non-English Linux transcription or automatic model/provider selection.

- **Movies:** GStreamer (`linux::media`): the source above, `videorate` to 30 fps, the first encoder installed of `x264enc`, `openh264enc` and VA-API's, `mp4mux`. Candidate frames come off a `tee` through an `appsink`; `Recorder::finish` drains it, but a stalled candidate branch must not make Apparent delete an MP4 whose file sink has already reported EOS. An unfinished encoder or mux still fails after the eight-second deadline. Compositors send frames only when the screen changes, so the last frame is reused until a new one comes. A standalone build without a usable encoder refuses to start a screen capture and names the host package to install (Arch/Omarchy: `sudo pacman -S gst-plugins-ugly`; Debian/Ubuntu: `sudo apt install gstreamer1.0-plugins-ugly`; Fedora: `sudo dnf install gstreamer1-plugins-ugly`). The preflight encodes a tiny synthetic movie once per process, so a registered but unusable hardware encoder cannot pass. A broken AppImage instead asks for a fresh AppImage: host packages cannot repair its bundled plugins. Apparent never installs packages automatically. Failed movies after recording begins still retain candidates.
- **H.264 decoder:** Apparent checks its own GStreamer registry at launch and in Settings › Recording. If no usable H.264 decoder factory exists, it warns with the host distribution's installation command (Arch/Omarchy: `sudo pacman -S gst-libav`; Debian/Ubuntu: `sudo apt install gstreamer1.0-libav`; Fedora: `sudo dnf install gstreamer1-plugin-libav`). Apparent never runs the command. A capture whose MP4 decode fails keeps the **whole** smaller candidate frames in `frames/` and `contact-sheet.png` rather than cropping around an idle cursor; these are already the full candidate images, so there is no duplicate `frames-full/`. It writes an `Image quality:` explanation in `summary.md`; the Linux GUI also notifies. Successful full-quality decode still uses normal autocrop and preserves `frames-full/` when cropped. A decoder factory does not guarantee that a particular stream can be decoded, so the packaging check actually decodes a small H.264 MP4 inside the AppImage.
- **Transcription:** Parakeet through `transcribe.cpp` (`speech.rs`). Local recordings transcribe completed speech during recording by default after 700 ms of quiet, with a three-second minimum chunk and 200 ms margins. Stop processes the remaining saved audio; any pause-worker failure falls back to whole-WAV transcription. Hex and command selections do not use the pause worker. See [pause transcription](../../../engine/docs/keyframes-and-text.md#linux-pause-transcription-srcpause_transcribers-srcspeechrs). Apparent downloads the model once at launch through `speech_model::ensure` (731 MB with `curl` or `wget`, pinned Hugging Face revision, checked against its SHA-256, into `~/.local/share/apparent/models`) unless Hex already has one; Settings › Recording shows its progress. A capture made before it lands says so in `summary.md` and is marked `.transcript-pending`, and is transcribed again in place when the download finishes.
- **Tray:** a StatusNotifierItem (`ksni`); left click starts a recording, the menu is the macOS menu bar menu. GNOME shows it with the AppIndicator extension (on by default on Ubuntu). Without a tray, Apparent says so in a notification, and launching it again opens Settings (`linux::ipc`, a socket at `$XDG_RUNTIME_DIR/apparent.sock`).
- **Clipboard:** text and PNG through the X clipboard (`linux::clipboard`) on X11 and GNOME; KDE Wayland uses `wl-copy` (bundled in the AppImage) because KWin does not reliably bridge background XWayland selections to native apps.
- **Launch at login:** `~/.config/autostart/apparent.desktop`; an explicit `launchAtLogin: true` in config refreshes a stale `Exec=` path when the AppImage moves, but respects a desktop-disabled entry. **Trash:** the freedesktop trash. **Captures:** `~/.local/share/apparent/captures`; **log:** `~/.local/share/apparent/Logs/apparent.log`.
- **Command-line tool:** Settings › Agents links `~/.local/bin/apparent` to the AppImage (`$APPIMAGE`).

### Updates and the running version

Settings › About shows the current version, a **Check for Updates…** button, and a Linux channel
picker. `stable` is the default; `main` follows the verified feed from main builds. The same
settings can be managed without opening the GUI:

```
apparent update status
apparent update channel main
apparent update channel stable
apparent update check
```

`status` prints JSON. It reads the selected channel from `updateChannel` in
`~/.config/apparent/config.json` (or `$APPARENT_CONFIG`), and reports `running.version` plus its
`revision` if the build has one. At app launch, Apparent writes `running` with its version, PID,
and Linux process start ticks to that config. **Do not infer a live version from the JSON file
alone after a crash**: `apparent update status` checks that PID and start time; `running: null`
means no matching process is alive. `installed` describes the downloaded AppImage marker,
which may be newer than the still-running process; `pendingRestart` indicates that difference.
`check` returns an error outside an owned AppImage rather than silently claiming success.
The update is installed for the *next* launch, not the current process. A corrupt config is
left untouched rather than replaced when changing the channel or recording a launch.

### Warm Parakeet recognizer

Local Parakeet recordings use lazy cross-recording model/session reuse by default.
Set the top-level `"warmRecognizer": false` in `~/.config/apparent/config.json`
to disable reuse at the next recording start; remove it or set it to `true` to
restore reuse. The retired `experiments.warmRecognizer` flag has no effect.
A single owner worker retains at most one recognizer,
and releases it after 30 minutes idle following the actual end of the last pause
worker, never during capture or recognition. Cancellation waits for that worker's
real completion before starting the idle clock, without blocking capture teardown.
Model selection is pinned at first recognition per capture; selection/file changes
invalidate reuse between captures, and native errors discard the unhealthy session.
Disabling the flag or choosing Hex/command clears it at the next recording start
once active work ends. No model RAM is reserved at app launch, and manual Paste
is unchanged. Retaining the recognizer costs additional idle RAM (about 1 GiB in
the two-capture Linux VM comparison). See
[warm recognizer](../../../engine/docs/keyframes-and-text.md#warm-recognizer-srcwarm_recognizerrs)
for lifecycle and same-process replay limits.

## Not on Linux

The cabled iPhone, OS element lookups for `agentContext` (`ax_other.rs`), Sparkle (Linux instead updates AppImages itself) and the dev-site tab URL (`/sdk.js` registrations still route). With `agentContext` on, an unnamed Linux click says `clicked (element unavailable)` and a mark receives no OS-derived element label. AT-SPI cannot securely join an accessible top-level to the X11 window where the action occurred; matching PID, title and bounds do not rule out a stale window. Registered dev pages can still supply their own context. See `capture/engine/docs/keyframes-and-text.md`.

## Packaging

`capture/desktop/scripts/linux/appimage/appimage.sh [binary]` builds `target/appimage/Apparent-<arch>.AppImage` with linuxdeploy and its GStreamer plugin, so GStreamer, `pipewiresrc`, `ximagesrc` and the H.264 encoders come with it. The binary baseline is **Ubuntu 24.04+**. Install `libfuse2t64` to run the AppImage, or run `./Apparent-<arch>.AppImage --appimage-extract-and-run app` without FUSE.

### From an owned Scrawl AppImage

The rename deliberately did **not** migrate Scrawl's settings, captures, credentials, model cache or OS permissions. Apparent uses different config/data paths and bundle identities; do not copy `~/.config/scrawl/config.json` or rename `.scrawl` bundles to `.apparent`. Older captures remain available by their old paths and the retained `scrawl` CLI. The old updater only accepts `Scrawl-…AppImage` and its own feed; it cannot install an Apparent image.

Once an **Apparent** Linux AppImage is published, obtain its versioned asset and `.sha256` from the same checked release, compare the downloaded image with the sidecar, install `libfuse2t64` (the normal launcher and CLI require FUSE), finish any Scrawl recording and quit Scrawl. From a desktop session, run once:

```sh
python3 capture/desktop/scripts/linux/maintenance/migrate-scrawl.py ~/Downloads/Apparent-<version>-linux-<arch>.AppImage \
  --shortcut-file ~/.config/hypr/bindings.lua
```

Omit `--shortcut-file` if there is no permanent Scrawl binding or the binding is in the standard `~/.config/hypr/bindings.lua` (auto-detected). The command expects an owned Scrawl AppImage behind `~/.local/bin/scrawl` and an owned desktop entry at `~/.local/share/applications/dev.kitlangton.scrawl.desktop`. It checks that **both** executables are type-2 AppImages reporting their expected product and asset version. It refuses an active Scrawl process, an existing `apparent` command target, malformed Apparent config, unknown shortcut syntax, or paths redirected outside the user's home. It tests normal AppImage startup before changing targets, installs a separate copy in `~/Applications`, creates `~/.local/bin/apparent` and the new desktop launcher, carries an **enabled** Scrawl autostart into Apparent unless Apparent was explicitly disabled, and changes only recognized permanent `scrawl shortcut record|paste` bindings. It then removes the owned old desktop/autostart entries, **not** the Scrawl image, CLI, captures or config. It preserves all other Apparent config keys, sets `launchAtLogin` to reflect the resulting autostart, and selects Main only when `updateChannel` is absent. A disabled Scrawl startup remains disabled. It backs up replaced launch files/config in `~/.local/share/apparent/migration-backups/<id>/` and rolls back touched targets if a step fails. It never stops a process, changes a system package or edits a package-manager-owned command. If the new GUI fails later, `~/.local/bin/scrawl app` still launches the old installation, and its prior desktop/autostart entries remain in the backup folder for manual recovery. Restart the desktop session to activate permanent Hyprland binding edits; inspect `hyprctl binds -j` afterwards. If your bindings differ from the documented Lua form, edit them deliberately before retrying; the migration refuses to guess.

Check the resulting `apparent update status`, desktop launcher `Exec`, `~/.local/bin/apparent` target, autostart entry and live shortcut separately. The command's CLI smoke test is **not a GUI test**: launch Apparent from the desktop and verify Settings and recording in an isolated GUI before using this on X1. A local AppImage with `updateChannel: main` is not proof of working updates: the selected channel needs a published release with both architecture assets and matching checksums. At the time of this migration change, the latest published Linux Main release still contains **Scrawl** assets, which the Apparent updater correctly rejects; issue #83 owns publication of a current Apparent preview. Issue #85 owns the repeatable Settings update-feedback click test, not this migration's files.
The build installs `gstreamer1.0-libav` and `capture/desktop/scripts/linux/appimage/check-appimage-decoder.sh` creates a tiny H.264 MP4 and asks the packaged executable to decode a full-resolution frame. Separately, `capture/desktop/scripts/linux/appimage/check-appimage-encoder.sh` asks the packaged executable to encode synthetic video and verifies its MP4 with FFmpeg; `appimage.sh` fails packaging if this fails. Both checks use the packaged runtime rather than the runner's host plugins.

`.github/workflows/release-linux.yml` builds x86_64 and aarch64 on Ubuntu 24.04 runners after each push/merge to `main`, with a nightly Main retry and manual dispatch retained. A dispatch on `main` with `tag=v<version>` checks out that tag and uploads versioned AppImages to its GitHub release; a dispatch on `main` with `publish_main=true` and no tag publishes a Main preview. A dispatch with `verify_only=true`, empty tag and `publish_main=false` verifies without publishing. Tags, other branches and PRs do not trigger this release workflow. The site links directly to the versioned assets. Build/deploy the site only after the appcast version is updated and both release assets exist. Flatpak is out of scope.

CPU portability (#82): `transcribe-cpp` uses `dynamic-backends`, which forces `GGML_NATIVE=OFF`. On x86_64 it builds separate `libggml-cpu-<tier>.so` modules (including separate Alder Lake and Zen 4 tiers); on aarch64 it builds a portable CPU module. `.cargo/config.toml` sets the Rust Linux baselines to `x86-64` and `generic`, respectively. `build.rs` stages the runtime and module directories next to binaries/tests with `$ORIGIN` RPATH. AppImage packaging explicitly includes the modules that dependency scanning cannot discover, all beside `libtranscribe` in `usr/lib`; `init_backends_default` resolves that library with `dladdr` and selects a CPU tier there once per process.

The Linux release workflow runs `capture/desktop/scripts/linux/appimage/check-cpu-tiers.py` before stripping symbols. It checks the core libraries and all x86 tiers for EVEX/AVX-512, VEX AVX-VNNI and AMX against their declared features, including baseline-only backend scoring. The release job checks the actual CMake cache has native compilation off, then runs Parakeet on two seconds of audio with only the extracted AppImage's libraries available. The x86 job repeats it under Intel SDE `-icl`: AVX512-VNNI present, AVX-VNNI absent, as on the Ryzen 8700G in #82. It also generates a demo bundle with the shipped executable under SDE, covering Rust post-processing without a desktop or model installation.

Linux previews can be published manually from checked AppImage artifacts as `linux-preview-N`, with `--prerelease --latest=false` and a combined `SHA256SUMS`. No tags trigger release workflows automatically, and neither the site nor the Sparkle appcast selects these previews.

### AppImage update channels

Settings › About › Channel selects Stable releases (default) or Latest main. Only an AppImage owned by the user and stored under their home directory auto-updates; a debug build, symlink or system-owned install stays untouched. At launch and hourly, Apparent checks its selected release channel. It downloads the matching architecture's AppImage, checks its SHA-256 against that release's checksum, atomically swaps the executable, and keeps the previous image beside it. The running process stays on the old image; the new one starts at the next launch. Settings and the tray's Check for Updates… trigger the same check. Errors are logged without replacing the installed image.
In Settings, a manual check immediately changes the button to “Checking…” and disables repeat clicks. When it finishes, a result shows the pixel frog and one line: “Update available” (including an image staged for the next launch), “No update available”, or “Update check failed”. The result does not show raw diagnostics; failures go to the log. Available offers Update now (relaunch the staged image) and Update later (dismiss); Tab focuses a choice, Enter or Space activates it, and Escape defers. Without a focused choice Enter chooses now. Other results have an OK button; Enter, Escape or OK dismisses them. If the new image exits before GPUI initializes, or initialization times out, Apparent restores the previous image and version marker at the installed path and skips that failed release until a newer one is published. A later crash after initialization is not treated as a failed update. The tray and automatic checks remain silent except for the existing installed-update notification and log.

Stable reads the paginated Releases API on `tryapparent/apparent` and selects the highest published non-prerelease `v<version>` containing this architecture’s Apparent AppImage and checksum. It excludes standalone components, Mac previews, Scrawl assets, drafts and incomplete architecture uploads. No compatible desktop release means “No update available”; network, malformed metadata, incomplete pagination and checksum failures remain errors. Discovery reads at most ten pages of 100 releases, with a 4 MiB metadata-response limit. Main reads published `linux-main-v2-<sha>` prereleases, excluding pre-signing compatibility bridges and metadata tags, from that repository's paginated Releases API, using the same page and response bounds as Stable, not raw source or a mutable feed branch. The workflow pins checkout and artifact identity to the event SHA. Both architectures must pass the full Rust suite, packaging, model/codec and CPU checks; all four asset digests must match the private draft before publication. Pushes share one non-canceling workflow lane: the running build can finish and publish, while redundant pending runs coalesce to the latest pending event. A checked ancestor of current `main` may publish even after the branch advances. Diverged history and an older rerun behind an already published eligible Main build cannot promote. Published SHA releases are immutable; a failed build leaves the last good release available. Publication is manual-only after the preview-baseline restoration; the historical automatic retry schedule is disabled. Manual `publish_main=true` with an empty `tag` uses the same checks.

Main therefore means the newest *successfully verified and published* build, not an immediate download after every merge. Build/test/package time comes first; the app then checks at launch or within its hourly interval (or immediately through Settings). Commit identity, not a semantic-version bump, distinguishes Main updates. Before staging over a known Main revision, the updater asks GitHub to confirm forward ancestry. An older feed leaves the newer installed/staged image intact; divergence, an unavailable comparison or an invalid response fails closed. Fresh CI packages use their embedded SHA when no installed marker exists. Legacy images without a revision can migrate once. Explicit Stable/Main channel switches retain their existing semantics, including potentially lower versions. Choosing Main accepts unreviewed code.

Older updaters only recognize `linux-main-<sha>`. For recovery, the manual release workflow accepts `bridge_commit`: a signed commit whose first parent is the last legacy bridge (`1a55d5e`) and second parent is the already verified canonical build, with exactly the same tree as that build. `signing-bridge.ts prepare <source>` creates and pushes this separate tag; `publish` runs only in the serialized release workflow. It requires exact-source Linux verification, an existing public v2 release, coverage of every published legacy revision and byte-identical assets. It never merges legacy history into Main or overwrites public assets. The corrected updater authenticates the bridge’s parents and tree before comparing from its compiled canonical revision, so a staged bridge can continue on v2 without permitting a downgrade.

Automatic checks only stage an image; they never restart the app. A private file lock beside the AppImage serializes GUI and CLI update transactions before release selection through download and atomic swap. A competing check fails safely and can be retried; it cannot finish an older download over a newer staged revision. The same lock spans explicit relaunch readiness and rollback, with no handoff gap; launched app processes do not keep it. Recovery checks the exact attempted image/backup identities and marker contents before restoring or marking a failed revision. The lock inode stays in place after release.

If the relaunched app does not acknowledge readiness within 15 seconds, the helper requests cooperative termination and allows two more seconds for it to exit. If the process remains live, the helper reports failure by its exit status and releases its lock without rollback or blacklisting. It never escalates to SIGKILL: missing readiness does not prove that a partly initialized app has no capture to finish. The installed image, its marker and the previous backup remain intact; a live app must be allowed to finish or quit before another restart. An exited failed child still follows the identity-checked rollback path.

Settings › Update now is refused while capture is admitted, selecting, recording or processing (including preview/lab work). Desktop's cross-platform `capture_busy()` also counts outstanding media finalizers independently of the capsule: dismissing processing UI does not make an unfinished bundle safe to interrupt. Each finalizer releases its count on receipt, even if its UI result is stale; one completion cannot clear another worker's busy state. The staged update remains available: finish or cancel the capture and wait for finalization, then choose Update now again. This decision runs on the capture UI thread before starting any relaunch helper; becoming idle never causes an automatic restart.

Repeatable packaged Settings click-through, controlled guest-local release responses,
isolation requirements and private video evidence: [Linux updater GUI acceptance](linux-updater-gui-test.md).
This checks the GUI and release contract; it does not establish GitHub publication.

## Building and testing

Build dependencies (Ubuntu/Debian; CI installs the same):

```sh
./capture/desktop/scripts/linux/appimage/dependencies.sh
cargo build -j 2 --offline
./target/debug/apparent app
```

GPUI needs Vulkan (`mesa-vulkan-drivers`; llvmpipe works in a VM). Tests: `cargo test`. A desktop test runs in a Tart Ubuntu VM on the Mac mini (`macvm`, see `~/code/home/tools/macvm`): GNOME on Wayland by default, X11 with `WaylandEnable=false` in `/etc/gdm3/custom.conf`. Never test on Kit's desktop session.
