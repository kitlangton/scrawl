# Linux

Read this before changing `src/capture/linux.rs`, `src/ui/linux.rs`, `src/linux/`, `src/ink_view.rs`, `src/speech.rs`, or anything behind a `platform::` call in `src/capture.rs` and `src/ui.rs`. `docs/windows.md` explains the shared shape (one GPUI UI, per-OS shells); Linux is the third shell beside macOS and Windows.

## X11 everywhere, Wayland around it

GPUI is built with its `x11` feature only, and `linux::prepare_process` (the first thing `main` does) hides `WAYLAND_DISPLAY` from it, so on Wayland every Scrawl window is an XWayland window (`SCRAWL_NATIVE_WAYLAND=1` opts out, for experiments). That keeps one window model on both: Scrawl's floating windows (pill, picker, ink overlay) are override-redirect X windows it places itself in global pixels (`linux::x11::float`, `place`), with an empty input shape to let clicks through (`set_click_through`). `float` waits for the window manager to finish mapping and then unmanaging the window before it takes it over; otherwise GNOME on Wayland keeps drawing it at its old frame's offset.

GPUI 0.2.2's X11 window panics on `HasWindowHandle`, so `x11::window_id` finds a GPUI window's X id itself (the newest `_NET_WM_PID` window it hasn't seen, cached per `WindowId`).

Global coordinates are root-window pixels; X11 has one scale for the screen (`x11::ui_scale`), so `MAX_CAPTURE_SCALE` is 1 as on Windows.

## What each session can do

Verified in an Ubuntu 24.04 arm64 VM with GNOME 46 (Wayland and X11) and Plasma 5.27.12 (Wayland). Plasma 6 and GNOME 48+ portal shortcuts still need desktop testing.

| | X11 | GNOME on Wayland | KDE Plasma on Wayland |
|---|---|---|---|
| Shortcuts | `XGrabKey` | GNOME custom keybindings running `scrawl shortcut record\|paste` (GNOME 48+: the GlobalShortcuts portal) | Plasma 5: native KGlobalAccel; Plasma 6: GlobalShortcuts portal, with KGlobalAccel fallback |
| Recording a display or area | `ximagesrc` | `org.gnome.Mutter.ScreenCast.RecordArea`, no dialog | ScreenCast portal, asks once (restore token in `~/.local/share/scrawl/portal.json`) |
| Recording a window | `ximagesrc xid=` | X11 apps by id; native Wayland windows are not listed | X11 apps by id; native Wayland windows are not listed |
| Clicks, keys, ⇧ ink | the X server (pointer and key state, polled) | the input devices (`/dev/input`, needs the `input` group) | same |
| Pointer position | the X server | XWayland while it's over X11 windows; an absolute pointing device (tablet, touchscreen, VM or remote desktop) from the input devices; the overlay takes the pointer while ⇧ is held | same |
| App in front | `_NET_ACTIVE_WINDOW` | AT-SPI when the app exposes it (`linux::atspi`; X11 apps use XWayland) | AT-SPI when available; Kate did not expose context in the test VM |
| Paste | XTEST `Ctrl+V` | RemoteDesktop portal, asks once | same |

### Clicks and ink on Wayland

Wayland tells no app where the pointer is over another app's window. Scrawl reads buttons and keys from the input devices. In onboarding or Settings › Permissions, **Copy command** copies `sudo usermod -aG input "$USER"`. Run it in a terminal, then log out and back in. Scrawl checks whether it can open actual keyboard/mouse devices, rather than trusting group membership; it retries devices when access changes. This is the **Input devices** row (the Accessibility row on macOS).

While ⇧ is held, the ink overlay takes the pointer (`linux::overlay::take_pointer`), so XWayland reports where it is and the ink follows it; a click while ⇧ is held lands on the overlay, not the app. On KDE Wayland, Scrawl hides the recording overlay between strokes because KWin 5 otherwise blocks clicks into native apps even with an empty X input shape. Ink remains in the bundle, but disappears from the desktop when ⇧ is released.

A click's position is right over X11 apps and with an absolute pointing device. With a mouse or touchpad over a Wayland app, XWayland still has the pointer where it last saw it, so the click's position is stale: the keyframe is cut at the right moment, but its ripple can be in the wrong place. A GNOME Shell extension that reports the pointer would close this gap (see the PR's open questions).

Without the `input` group, a capture on Wayland has no clicks, keys or ⇧ ink outside Scrawl's own windows, and records only the screen and voice.

### Picker and portal limits

Screen and Area work on the tested desktops. The Window picker lists X11/XWayland windows only; use Screen or Area for native Wayland apps. A portal window source exists internally, but the UI does not currently offer it. A press of the record shortcut records the display under the pointer once its keys come up, however long they're held; the tray menu's **Choose What to Record…** opens the picker, which stays open until you choose a target or press Esc. The Mac-only iPhone mode is hidden. Screen sharing can ask again if a desktop does not honor the saved restore token. Finishing while the dialog is open cancels the pending session; a failed or empty capture reports an error rather than producing demo imagery, and `latest` skips its unfinished directory.

Plasma 5 can report successful GlobalShortcuts binding with an empty result. Scrawl validates both returned bindings and uses KGlobalAccel directly on Plasma 5, so the shortcut does not also type into the focused app.

## Scrawl's own windows are in the recording

Neither X11 nor GNOME's or KDE's screencasts let an app leave its windows out, so the pill and the ink are in `recording.mp4` and the keyframes. The ink is drawn again at the same place on the keyframes, so it doesn't double. The candidate frames used to pick keyframes paint over the pill (`linux::media::own_window_masks`), so its ticking clock never counts as activity.

## Movies, transcription, the rest

- **Movies:** GStreamer (`linux::media`): the source above, `videorate` to 30 fps, the first encoder installed of `x264enc`, `openh264enc` and VA-API's, `mp4mux`. Candidate frames come off a `tee` through an `appsink`; it holds end-of-stream back until its last frame is taken, so `Recorder::finish` drains it. Compositors send frames only when the screen changes, so the last frame is reused until a new one comes. Without an encoder Scrawl records no movie and keeps sharper candidates instead.
- **Transcription:** Parakeet through `transcribe.cpp` (`speech.rs`, shared with Windows). As on Windows, Scrawl downloads the model once at launch through `speech_model::ensure` (731 MB with `curl` or `wget`, pinned Hugging Face revision, checked against its SHA-256, into `~/.local/share/scrawl/models`) unless Hex already has one; Settings › Recording shows its progress. A capture made before it lands says so in `summary.md` and is marked `.transcript-pending`, and is transcribed again in place when the download finishes.
- **Tray:** a StatusNotifierItem (`ksni`); left click starts a recording, the menu is the macOS menu bar menu. GNOME shows it with the AppIndicator extension (on by default on Ubuntu). Without a tray, Scrawl says so in a notification, and launching it again opens Settings (`linux::ipc`, a socket at `$XDG_RUNTIME_DIR/scrawl.sock`).
- **Clipboard:** text and PNG through the X clipboard (`linux::clipboard`) on X11 and GNOME; KDE Wayland uses `wl-copy` (bundled in the AppImage) because KWin does not reliably bridge background XWayland selections to native apps.
- **Launch at login:** `~/.config/autostart/scrawl.desktop`. **Trash:** the freedesktop trash. **Captures:** `~/.local/share/scrawl/captures`; **log:** `~/.local/share/scrawl/Logs/scrawl.log`.
- **Command-line tool:** Settings › Agents links `~/.local/bin/scrawl` to the AppImage (`$APPIMAGE`).

## Not on Linux

The cabled iPhone, video export, `agentContext` lookups (`ax_other.rs`), Sparkle updates (**Check for Updates…** opens the releases page) and the dev-site tab URL (`/sdk.js` registrations still route).

## Packaging

`scripts/linux/appimage.sh [binary]` builds `target/appimage/Scrawl-<arch>.AppImage` with linuxdeploy and its GStreamer plugin, so GStreamer, `pipewiresrc`, `ximagesrc` and the H.264 encoders come with it. The binary baseline is **Ubuntu 24.04+**. Install `libfuse2t64` to run the AppImage, or run `./Scrawl-<arch>.AppImage --appimage-extract-and-run app` without FUSE.

`.github/workflows/release-linux.yml` builds x86_64 and aarch64 on Ubuntu 24.04 runners. Pull requests and manual dispatches produce artifacts and run packaged `--help` checks; `v*` tags upload versioned AppImages to the matching GitHub release. The site links directly to those versioned assets. Build/deploy the site only after the appcast version is updated and both release assets exist. Flatpak is out of scope.

CPU portability (#82): `transcribe-cpp` uses `dynamic-backends`, which forces `GGML_NATIVE=OFF`. On x86_64 it builds separate `libggml-cpu-<tier>.so` modules (including separate Alder Lake and Zen 4 tiers); on aarch64 it builds a portable CPU module. `.cargo/config.toml` sets the Rust Linux baselines to `x86-64` and `generic`, respectively. `build.rs` stages the runtime and module directories next to binaries/tests with `$ORIGIN` RPATH. AppImage packaging explicitly includes the modules that dependency scanning cannot discover, all beside `libtranscribe` in `usr/lib`; `init_backends_default` resolves that library with `dladdr` and selects a CPU tier there once per process.

Both Linux workflows run `scripts/linux/check-cpu-tiers.py` before stripping symbols. It checks the core libraries and all x86 tiers for EVEX/AVX-512, VEX AVX-VNNI and AMX against their declared features, including baseline-only backend scoring. Release CI checks the actual CMake cache has native compilation off, then runs Parakeet on two seconds of audio with only the extracted AppImage's libraries available. The x86 job repeats it under Intel SDE `-icl`: AVX512-VNNI present, AVX-VNNI absent, as on the Ryzen 8700G in #82. It also generates a demo bundle with the shipped executable under SDE, covering Rust post-processing without a desktop or model installation.

Linux previews are published manually from the checked AppImage artifacts as `linux-preview-N`, with `--prerelease --latest=false` and a combined `SHA256SUMS`. These tags do not trigger the `v*` release jobs, and neither the site nor the Sparkle appcast selects them.

## Building and testing

Build dependencies (Ubuntu/Debian; CI installs the same):

```sh
./scripts/linux/dependencies.sh
cargo build -j 2 --offline
./target/debug/scrawl app
```

GPUI needs Vulkan (`mesa-vulkan-drivers`; llvmpipe works in a VM). Tests: `cargo test`. A desktop test runs in a Tart Ubuntu VM on the Mac mini (`macvm`, see `~/code/home/tools/macvm`): GNOME on Wayland by default, X11 with `WaylandEnable=false` in `/etc/gdm3/custom.conf`. Never test on Kit's desktop session.
