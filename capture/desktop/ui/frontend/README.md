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

The sandboxed, isolated preload exposes only `readRuntimeState()` with no inputs.
The main process rejects other windows/frames, nonlocal URLs and arguments.
There is no generic IPC, shell, filesystem, Settings or capture command API.
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
