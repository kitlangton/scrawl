# Desktop UI foundation

Opt-in Electron development window using React/TypeScript, shadcn/ui and Tailwind.
It does **not** replace the installed app, Settings, pill or launcher. The opt-in
window has native Linux and macOS checks; see their distinct scope below. No Windows support.

From the repository root, with Node 22 and npm 10 or newer:

```sh
cd capture/desktop/ui/frontend
npm ci
npm start
```

Open the current Rust Apparent app separately, under the same user and HOME,
then click **Refresh state**. The window never launches Rust itself. With no app,
it shows **Not connected**. `npm run preview` serves the built synthetic UI on
loopback; browser preview has no Rust bridge. It deliberately uses production
assets and CSP rather than a less-restricted Electron development server.

## Narrow contract

The existing producer is `capture/desktop/workflow/agent_capture.rs` and the
`None` action in `App::agent_request` in `capture/desktop/mod.rs`. The Electron
main process connects to `$HOME/.config/apparent/agent-control/capture.sock`,
sends exactly `state\n`, reads a bounded newline-delimited JSON reply, validates
`state.phase`, and returns only `{ status: 'connected', phase }`. Capture IDs
and other backend fields never reach the renderer. This observes the **running
Rust GUI**, not package metadata or a synthetic backend. It does not make Rust
independent of the current GUI lifecycle.

Missing/stale sockets are disconnected; unsafe permissions, invalid replies,
unknown phases and timeouts are errors. The control directory and socket must
belong to the effective user, be private, and not be symlinks. Nothing creates or
changes the Rust config directory. One absolute 3.5-second deadline bounds a
read. Reads occur only on open/renderer reload or manual refresh, with one
in-flight read and a one-second cooldown. There is no background polling.

The sandboxed, isolated preload exposes `readRuntimeState()` and the three
zero-argument capture-location methods below.
The main process rejects other windows/frames, nonlocal URLs and arguments.
There is no generic IPC, shell, filesystem, Settings or capture command API.
Only the captures-only requests described below may save a preference.
Navigation, popups, webviews, network connections and permission requests are
blocked. Electron keeps its own Chromium profile in `apparent-ui-foundation`,
separate from the installed application's identity/config. These constraints do
not grant or reset OS permissions or change login/URL registration.

Synthetic states cover idle, recording, processing, uploading, preparing,
permission failure and error. They never drive the live state display. A copied
link is explicitly not a ready-to-watch recording. Shared controls and light
tokens live in `shared/ui`; Connect adoption is a later slice. After editing
shared sources, rerun `npm ci`; builds reject a stale installed snapshot.

## Checks

Run Electron smoke files sequentially: Electron's lazy binary installation is
shared by both tests and concurrent first launches can race during extraction.

```sh
npm test                    # real Unix-socket fixtures; no GUI or Rust launch
npm run build               # strict TypeScript and production bundle
npm run test:electron       # real Electron, synthetic socket; needs a display
# On a headless Linux host:
xvfb-run -a npm run test:electron
```

The Electron smoke checks controls/keyboard use, disconnected/error/reconnect,
renderer reload, minimum-width layout, sandbox/Node isolation, navigation/network
refusal and rejection of a second renderer. It is not native platform acceptance.
`tests/native-smoke.cjs` is a disposable-guest-only Electron entrypoint. The
existing isolated Linux runner is `capture/desktop/scripts/linux/gui/gui-vm.py`;
follow `../../docs/native-testing.md` for template/access/lifecycle requirements.
In an owned disposable Mac, run it with `SCRAWL_GUI_OUTPUT` set to a private
evidence directory. Use a fresh private HOME for disconnected checks; for live
checks use the running Rust app's HOME and `NATIVE_EXPECT_CONNECTED=1`. It saves
only its own window, never the host screen. Native evidence is private, not a
repository asset.

The Linux native run connected to a frozen Rust baseline with recorded
provenance. NoMac verification on macOS 26.6.2/arm64 with Electron 45.2.1 passed
all 11 socket tests, the Electron interaction/security test and native window
checks. A subsequent run cloned published head
`ff420b5d7539a455e53b20e3c753475291103965`, built and installed the real Rust GUI
with the repository installer, and passed the native smoke with a live
`connected` / `idle` response. Its Error tab was a separately labelled synthetic
fixture, not a failed recording. The guest and queue ticket were cleaned up.
These are shell/bridge checks, not capture, overlay, permission, updater,
physical-device or performance acceptance.

## Experimental Storage card

This separate slice is off by default. Set
`"experiments": {"electronCaptureLocation": true}` in the **Rust app's**
`~/.config/apparent/config.json`, preserving other keys, then open or refresh the
Electron window. File edits apply on the next request; there is no renderer or
environment flag that enables this experiment. The installed native Settings,
launcher and pill are unchanged. Browser preview has no Storage bridge.

`readCaptureLocation()`, `changeCaptureLocation()` and `resetCaptureLocation()`
accept no path, revision, dialog options or other arguments. Main reads the
revision from Rust before opening its parented native folder dialog. Cancel,
reload, navigation and closing the window discard an unfinished selection.
Change and Reset are single-flight. A timeout never retries a mutation: refresh
to establish its actual outcome before trying again.

The same-user socket accepts strict JSON `{storage:"read",deadline}`,
`{storage:"set",path,expected,deadline}` and `{storage:"reset",expected,deadline}`
frames with a trailing newline. `deadline` is same-host monotonic milliseconds;
Rust caps admission at three seconds and rechecks it after acquiring the config
lock. Storage uses libuv's clock: `CLOCK_MONOTONIC` on Linux,
`CLOCK_MONOTONIC_RAW` on macOS. The legacy Rust-client clock is unchanged.
`expected` is an opaque 16-hex stale-edit fingerprint of **only** the raw
`capturesDir` value, not a security token or portable content digest.
Other config edits survive; a changed capture preference refuses the stale save.
One worker handles Storage without blocking the GUI or queuing config writers.
Legacy state/start/inspect/stop/cancel requests keep their existing contract.

Rust validates paths, preserves unknown config keys, removes the saved key on
Reset, and returns authoritative saved/effective/display paths and source
(`environment`, `configured`, `builtin`, `fallback`). Revisions remain in main;
raw config and diagnostics never reach the renderer. An environment override
still wins over the saved preference. An unusable saved folder falls back to
the built-in folder. Existing bundles keep their recorded directory; nothing
is relocated. Subsequent captures resolve the new preference. Existing
background retention/transcript workers retain their startup directory until
the Rust app restarts; this slice does not change their lifecycle.

Rust config/socket tests and synthetic Electron tests are not native picker or
capture proof. The earlier Linux/macOS foundation results above do **not**
establish changed Storage behavior.

A separate disposable Omarchy run on the repaired Storage candidate exercised
the real Change and Reset buttons, real portal folder chooser, Rust config
write/readback and visible effective-location updates. Cancel preserved the
file; unknown fixture keys and an existing-file sentinel survived. Electron
45.2.1 and the rebuilt Rust GUI ran in a private HOME with outbound networking
blocked. The guest, disk overlay and expiry timer were cleaned up. This proves
that Linux Settings roundtrip, not capture output or Mac behavior.

The first guest inherited `GDK_SCALE=2` from its Omarchy monitor config despite
its scale-1 display, making the GTK portal chooser 2510×2002. Matching the
disposable guest's GTK scale to its display enabled real pointer Choose/Cancel.
A separate 1920×1080, scale-1 run verified the entire native dialog fits, actual
pointer selection/Cancel, renderer Change/Reset, Rust persistence and visible
readback. The app, persistent VM template and host settings were not changed.
The guest and expiry were cleaned up after the successful runs.

On 1280×800 the scale-1 GTK dialog still requests 1255×1001, even when given a
smaller GTK geometry preference. This remains a small-display desktop/toolkit
limitation, not an app-side size fix; do not mutate users' GTK settings to hide
it.

[CI run 41354248911](https://github.com/tryapparent/apparent/actions/runs/41354248911)
on slice `5d6fceed` passed the explicit Electron/libuv-to-Rust regression on Linux
and macOS. Its GitHub-hosted `macos-26`/arm64 job also passed the actual installed
Rust GUI + shipped Electron Storage flow with Electron 45.2.1: parented native
Choose, visible custom-path readback, Cancel with identical config bytes/inode/
mtime, and Reset to the built-in path. Unknown config fields and the existing-file
sentinel survived. The native picker, saved-state and reset-state images were
inspected; owned processes exited and the private fixture was removed. This
closes this slice's Mac Settings gate, not capture, permissions, overlays,
performance or replacement of installed Settings. The experiment remains off by
default; this evidence does not authorize enabling, merging or releasing it.

For an actual Electron/libuv → Rust clock and persistence regression (no GUI or
host config writes), after installing the frontend Electron binary, run from
the repository root:

```sh
APPARENT_STORAGE_TEST_ELECTRON="$PWD/capture/desktop/ui/frontend/node_modules/electron/dist/electron" \
  cargo test --offline storage_electron_rust_roundtrip -- --ignored --nocapture
# On macOS the executable is dist/Electron.app/Contents/MacOS/Electron.
# On Clanker wrap this command once with /home/simon/.local/bin/t3-build.
```

The default Rust suite ignores this dependency-bearing integration test; an
ignored case is not a pass. Root CI explicitly runs it after the existing
Rust checks on Linux and macOS, using those same jobs rather than a paid Mac
rental. The explicit run uses the real Rust listener and
shipped JS transport, actual hrtime deadlines, synthetic file-backed activation,
set/readback/conflict/expired/reset and malformed-config refusal.

The existing Mac root job also builds an isolated app with the repository
installer (`APPARENT_NO_LAUNCH=1`) and explicitly runs
`tests/mac-storage-native.cjs`. That CI-only probe uses the shipped renderer,
installed Rust GUI and native parented picker. System Events input is restricted
to the owned Electron PID; it checks the runner's existing automation permission
and fails rather than granting or resetting TCC. Synthetic reports and owned
window images are retained for seven days, including failures. A Linux skip or
an unexecuted CI definition is not native Mac proof. No Mac rental is required.
