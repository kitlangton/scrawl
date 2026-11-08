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
and other backend fields never reach the renderer through this state method.
This observes the **running Rust GUI**, not package metadata or a synthetic backend. It does not make Rust
independent of the current GUI lifecycle.

Missing/stale sockets are disconnected; unsafe permissions, invalid replies,
unknown phases and timeouts are errors. The control directory and socket must
belong to the effective user, be private, and not be symlinks. Nothing creates or
changes the Rust config directory. One absolute 3.5-second deadline bounds a
read. Reads occur only on open/renderer reload or manual refresh, with one
in-flight read and a one-second cooldown. There is no background polling.

The sandboxed, isolated preload exposes runtime state, selected-reference read/copy,
capture-location, retention/usage, Connect and typed error-reporting methods below.
The main process rejects other windows/frames, nonlocal URLs and unexpected arguments.
There is no generic IPC, shell, filesystem or recording command API.
Only bounded Settings workflows save their preferences; Copy ID changes only the clipboard.
Navigation, popups, webviews, network connections and permission requests are
blocked. Electron keeps its own Chromium profile in `apparent-ui-foundation`,
separate from the installed application's identity/config. These constraints do
not grant or reset OS permissions or change login/URL registration.

Synthetic states cover idle, recording, processing, uploading, preparing,
permission failure and error. They never drive the live state display. A copied
link is explicitly not a ready-to-watch recording. Shared controls and light
tokens live in `shared/ui`. After editing
shared sources, rerun `npm ci`; builds reject a stale installed snapshot.

## Experimental General → Apparent Connect

The default-on `connectSettings` flag also gates this opt-in window's Connect
card; an explicit false hides it. Visibility alone never enables uploads. Installed Settings still use their existing native presentation;
this slice does not change routing or onboarding. See
`../../docs/onboarding-and-settings.md` for consent and recovery behavior.

Electron main owns one persistent, same-user Rust socket/controller. Opening and
one-second local refreshes never initiate network access. Each bounded request
uses Electron's monotonic clock; a ten-second idle lease, EOF, renderer reload,
crash or window close drops the controller and cancels pending authorization.
Rust owns all consent, credentials, health probes and durable revocation; JS does
not read config or implement an auth client. Raw errors/config/device secrets do
not enter the renderer. Only an explicit zero-argument **Open browser** command
can open the current Rust-validated public verification URI; no arbitrary URL is
accepted from the renderer. Ambiguous commands are never retried automatically.

`tests/connect/transport.test.cjs` exercises real Unix-stream transport and projection;
`tests/connect/smoke.cjs` exercises shipped Electron main/preload/renderer against
an explicitly synthetic socket. `tests/rust-connect.cjs`, launched by the ignored
Rust `connect_settings_electron_rust_roundtrip` test, exercises the actual Rust
listener/controller/production client using Electron's real clock and a synthetic
loopback pending-device HTTP fixture. It verifies explicit consent admission,
public-only projection, durable cancel/EOF revocation and preserved config;
it does not approve a real account or upload media. Run it from the root with
`APPARENT_CONNECT_TEST_ELECTRON` set to the absolute frontend Electron executable:
`cargo test --offline connect_settings_electron_rust_roundtrip -- --ignored --nocapture`.
On Clanker, wrap the command once with `t3-build` as documented in `AGENTS.md`.
Neither check substitutes for native Linux/macOS acceptance. The earlier shell evidence below does not
certify this new workflow. Roll back by disabling `connectSettings` or closing
this opt-in window and using installed Settings; do not remove the legacy
presentation until both platforms pass equivalent consent/cancel/recovery proof.

Background completion receipts are consumed only on a valid window request,
not while the stream is idle. Closing before the next read therefore retains
pending-login cancellation even when the server has already completed a grant.
File edits disabling `connectSettings` apply at the next command/receipt or
client authorization-predicate check, and within half a second for an otherwise
idle stream. Observing a disabled/unreadable flag closes that controller before
worker authorization. This is not an atomic filesystem-edit/network-dispatch
promise; durable upload withdrawal still uses Disable uploads/Disconnect.
## Selected capture reference

The **Selected capture** card ports only Copy capture ID, not capture review or history.
It reads the saved capture already selected by the running Rust GUI; it never picks
"latest", opens a capture or starts Rust. No selection disables Copy. Rust alone
derives the exact directory basename and updates the system clipboard as plain text.
The normal `copyCaptureId` default applies; an explicit config `false` hides both UIs.

Main/preload expose only `readCaptureReference()` and `copyCaptureReference()` with
no renderer arguments. The typed socket family is `{"reference":"read","deadline":N}`
or `{"reference":"copy","expected":ID,"deadline":N}`. Main retains the read ID, not
the renderer. Rust compares it to its live selection on the GUI thread, rejects stale
IDs before writing, and checks a maximum three-second admission deadline using the
same clock translation as Storage. Replies expose only a nullable basename, copied
boolean or enumerated error—not paths, summaries, history, config or raw errors.
There is no automatic retry, paste, upload or mark-handled effect. Refresh is explicit.

Routing remains `npm start` opt-in; installed routing and the working GPUI action are
unchanged. Close the foundation window to roll back to the existing presentation;
`experiments.copyCaptureId: false` disables the action in both. Remove the GPUI donor
only after the corresponding shared capture/handoff routing is separately accepted
and its macOS/Linux native behavior is verified. This slice does not authorize that
routing switch or a desktop review workspace.

`tests/reference-native.cjs` exercises real Electron, Rust selection, native clipboard
PNG replacement, repeat dispatch, stale-selection refusal, no-selection and opt-out.
It refuses a personal desktop: macOS requires a disposable GitHub-hosted runner;
Linux native mode requires the isolated VM, while its explicit Xvfb mode is integration
proof only. Pass absolute `APPARENT_REFERENCE_BINARY` (installed App on Mac) and a fresh
`APPARENT_REFERENCE_OUTPUT`; set `APPARENT_REFERENCE_VM=1` in the Linux guest, or
`APPARENT_REFERENCE_XVFB=1` under `xvfb-run` for integration. Screenshots contain only
the owned generated-fixture windows and stay outside git.

## Storage retention and usage (#291)

Installed Settings routes Storage to the branded React presentation by default.
`experiments.electronCaptureLocation=false` restores the native Storage page on
its next open. No retention policy is changed by enabling the presentation.
`readRetention()`, `saveRetention(preset)`, `resetRetention()` and
`readCaptureUsage()` are the only additional preload methods. Only Save takes
an argument: a validated `week`, `month` or `forever` identifier. Rust supplies
the option labels, current/custom policy, policy note, effective path and
formatted logical-byte usage. No renderer-provided path, config, revision,
deadline, filesystem, shell or arbitrary IPC operation is accepted.

The existing bounded socket worker admits `retention`, `saveretention`,
`resetretention` and `usage` through its unchanged activation/expiry gate.
Main retains the last authoritative retention revision; a concurrent edit
conflicts rather than silently overwriting it. The existing config transaction
preserves unrelated top-level and nested policy keys and secrets; Reset removes
only the recognized retention fields, restoring `Policy::default()` (7-day
media/30-day bundle policy). A custom hand-edited policy remains visible until
explicitly replaced. Writes fail closed, including malformed/unsafe config;
failed or outcome-unknown saves require a refresh and are never retried.

Saving/resetting nudges the **existing** Rust cleanup worker, as native Settings
does: it schedules a pass about five seconds later, deferring while recording.
The worker reloads policy on each pass; manual config edits apply on the next
pass (normally hourly; first launch pass after about 90 seconds). It retains
its startup capture folder until app restart. Nothing in the bridge runs a
cleanup pass or moves a capture; normal policy cleanup remains unchanged.

Usage scans the freshly resolved Rust effective location, not a cached renderer
path, on open and manual refresh only. It counts logical file bytes (not free
space or physical allocation), does not follow symlink entries, and returns
an error rather than a partial total on I/O failure, expiry, its 100,000-entry
limit or a directory nesting depth of 256. A missing root is empty.
Main and React discard old replies after a
location mutation, save, reload or unmount; the renderer also checks the returned
effective path. Loading/error states hide old totals, and Retry never starts
Rust. A slow filesystem syscall can outlive client expiry; the existing single
Storage worker stays busy rather than spawning unbounded scans.

On macOS the Rust Settings session owns navigation and the Electron child. The installed
page contains Storage only, with the existing Settings sidebar and system-following
light/dark Paper/Carbon/Signal palette. Other sidebar pages return to their native
presentation. Retired windows cannot send commands or close the current page.
Closing Settings cancels the session; an Electron crash restores native Storage.
Missing packaged assets also fall back to native Storage. Reopen Settings after
editing the opt-out. Saved preferences remain readable by both presentations.

Electron Storage uses the native Mac Settings' 720×530 frame and 210px black
sidebar. macOS centers after applying its inset titlebar and accepts the first
inactive click; content scrolls inside the frame to keep lower controls reachable.
The other Settings pages keep their native routing.
The Linux parent supplies the X11 backend option before Electron starts when it
has selected XWayland; a live native-Wayland opt-in keeps its existing selection.

Storage starts hidden, including when its parent requests Show early. React
acknowledges its committed DOM in a layout effect; the isolated preload waits
two animation callbacks before sending the mount and first-frame acknowledgements.
`paintWhenInitiallyHidden` keeps those callbacks moving while hidden. The host
requires both the mount and either Chromium's `ready-to-show` or the validated
first-frame acknowledgement, recovering when Electron omits that native event.
Missing renderer startup exits unsuccessfully after 15 seconds without a
user-dismiss signal, allowing the existing native fallback. Load failure/crash
also fail; close/parent EOF retires the window so late readiness cannot reopen it.
Parent retirement emits no user-dismiss event. A broken parent output pipe also
retires quietly instead of opening an Electron error dialog.
The readiness bridge admits only the owned local main frame and no arguments.
The broader intermittent blanking reported in #562 remains unconfirmed.

`stage-settings.py` builds the locked frontend/runtime into Linux AppImage and Mac
installer/release packages. Installed packages contain no development dashboard or
synthetic examples. Node/npm are build prerequisites, not runtime dependencies.
macOS stages and signs nested Electron code before the enclosing app is signed.
Linux keeps every installed Settings pane inside one native GPUI frame, including
the folder picker, retention and usage controls. Switching to Storage does not
replace the window. The Electron development harness and Rust bridge remain
available on Linux; the installed macOS route retains its explicit opt-out and
startup/crash fallback. Remove a native presentation only after applicable native
acceptance and a deliberate retirement of its fallback contract. Functional
transport tests and synthetic backend fixtures remain verification tooling.

`tests/retention-smoke.cjs` exercises production Electron with a **synthetic**
socket (save/reset/readback/write error/loading/usage error/stale results,
keyboard/responsive layout and bridge rejection). `tests/rust-storage.cjs`
extends the real Rust/Electron contract regression. The shared native assertion
helper `tests/native-retention.cjs` exercises production Electron against a real
Rust GUI in an exclusively owned disposable desktop, including genuine private
directory write/usage failures and file preservation. The existing Mac CI probe
calls it; only an executed, inspected run proves the Mac gate. Prior #277
native evidence does not cover these additional controls.

An isolated Omarchy run on `29cf0819` passed the unchanged native retention
helper against the frozen Rust GUI and shipped Electron 45.2.1. Inspected images
showed Month (30-day media/90-day bundles), reset to Week, genuine private-config
write refusal, usage failure without a stale total, and recovery to the measured
1 MB synthetic folder. Unrelated config and the synthetic file survived; Rust,
Electron, owned VM/overlay/firmware/sockets and expiry were cleaned up. This is
Linux Settings proof only, not timed Trash/cleanup, capture permissions or Mac.
No recording/bundle was produced; that guest reports a missing H.264 decoder,
and its walkthrough capture/export path was not exercised. Private evidence is
retained outside the repository; the PR records candidate identity and gates.


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

The Electron Storage bridge is on unless `experiments.electronCaptureLocation`
is explicitly false in the Rust app's `~/.config/apparent/config.json`. File edits
apply at the next request; on macOS reopen Settings to change presentation routing. Linux
installed Settings always uses its continuous native frame. The
separate `npm start` window remains a developer harness; it is not installed or
used by the Settings route. Browser preview has no Storage bridge.

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
and fails rather than granting or resetting TCC. It also checks the reporting
toggle's default-on state, opt-out, authoritative readback, Rust-process restart
plus renderer-reload persistence, and explicit re-enable/disable. Synthetic reports and owned
window images are retained for seven days, including failures. A Linux skip or
an unexecuted CI definition is not native Mac proof. No Mac rental is required.

## Technical error reports

The reporting card is available without an experiment. `readErrorReporting()`
and `setErrorReporting(boolean)` share the existing private socket using only
`{errorReporting:"read",deadline}` / `{errorReporting:"set",enabled,deadline}`.
Main accepts the one Boolean only from its owned local main frame. Rust owns
default-on policy, fail-closed config reads, atomic persistence and consent
generation rotation; no config paths, tokens, raw config or error text reach the
renderer. Both Settings contracts share one rendezvous worker so a config-lock
wait cannot block capture controls. A mutation's deadline is checked again under
the config lock; unknown outcomes are never retried automatically.

This window does not replace installed Settings routing. The native toggle
remains until replacement Settings acceptance; rollback is to close this opt-in
window, not change the saved consent. Remove the old presentation only with the
future Settings routing switch. The reporting preference itself is not an
off-by-default experiment. A packaged PostHog token is required to send reports.

## Explicit public link from the selected capture (#171)

The Selected capture card includes Copy public link, followed by an explicit
anyone-with-link/non-E2E confirmation. Rust pins the selected completed bundle
path, filesystem identity and capture generation. Preparation runs off the GUI
thread; short `publicShare` start/read/deliver/cancel requests never carry a
renderer path or URL. Electron main retains the operation ID and expected capture
reference, rejects foreign frames/arguments and cancels on reload/close. Rust
checks current operation, selection, expiry and client authorization before its
single foreground clipboard write. Failed/ambiguous writes are never retried.
Cancellation stops future work; data already transmitted cannot be recalled.

This shares exactly one separate service-readable copy through #346, without
changing private encrypted transfers or local Paste. A copied link may still show
Preparing recording; the UI directs the recipient to the link for readiness.
Disconnected devices use existing Connect Settings; signing in does not create a
public link. No automatic-sharing preference or historical scan is introduced.

Routing remains the existing Electron development window (`npm start`); installed
native Copy link retains its refusal for encrypted captures. Close this window to
roll back. Do not remove the legacy donor or switch installed capsule routing
until this workflow passes real macOS/isolated Linux acceptance and the routing
switch is authorized. Socket/controller/browser fixtures are not native proof.

## Shift-click notes

The default-available notes surface runs `electron/notes/main.cjs`, launched by
Rust for a screen capture. It renders only the nearby text popup and a small
saved-note badge/list beside the existing capture controls; picker, ink and
capture lifecycle remain on their current native path. There is no competing
Electron capture state machine. This is independent of the foundation Settings
window and requires no experiment. Packagers ship its production assets and
Electron runtime via `scripts/package-notes.mjs`; development must run `npm ci`
and `npm run build` before starting the rebuilt Rust app.

A private inherited pipe carries click-time anchors and bounded note commands.
Only trusted local main frames can submit draft/save/edit/delete/dismiss/list;
no renderer paths, window IDs, IPC names, shell or network commands are accepted.
Rust writes saved notes to `notes.json` before acknowledging/counting them;
unsent drafts stay in the capture's Rust session rather than note metadata/summary. Linux display video still includes the visible popup and its text. Enter with no text closes the
field and retains the pin. Escape keeps the in-capture draft; Shift+Enter adds
a newline. Capture completion closes this capture-local surface. Restarting or
ending a capture does not promise recovery of unsubmitted drafts.

The native menu and Electron Recording control share `microphoneEnabled` in
`~/.config/apparent/config.json`. It defaults on, preserves device selection,
and applies at the next capture. Off skips microphone admission/acquisition;
the bundle records `context.microphoneCapture=off`, including local recovery,
so intentional silence does not masquerade as an audio failure. Config writes
use Rust's existing atomic writer and preserve unrelated keys. No setting edit
interrupts active audio.

On Linux, Electron uses X11/XWayland. Rust checks each XID against this capture's
child PID and prepares it while hidden; Electron alone shows and hides it.
On the Hyprland Lua backend, notes remain managed toolbars so their first map
belongs to a real output even when its layout origin is offset. Other backends
retain override-redirect preparation. Popup activation restores web-content focus.
The pipe carries X11 root pixels: Electron converts incoming anchors to DIPs and
outgoing window bounds back to pixels. Hyprland placement then matches the
output by name and converts those pixels to its scaled, potentially offset
layout. Do not send an Electron DIP rectangle directly to either native adapter.
The compositor's visible pinned order can diverge from X11's input stack after focus
returns to the recorded app. The notes worker checks that stack at most every
100 ms and raises its visible surfaces only when needed. Renderer commands cannot
place windows. Counter/editor clicks are excluded from capture telemetry.

Rust owns the private Chromium profile and removes it after reaping Electron,
before the Finish receipt. Deleting it in Electron's `quit` event races Chromium's
last writes and can open an error dialog. Notes use the shared system-appearance
tokens while keeping the window background transparent.

Notes route solely to this Electron surface. Closing it leaves capture and
existing drawing running; a lost notes renderer must not claim a saved note.
The old pin/drawing path is retained, while no legacy note editor exists to
remove. A rollback reverts this slice and keeps already saved note events
readable. Native focus, positioning, Shift input and real output acceptance on
both platforms remain distinct from synthetic Electron/pipe tests.

The shared Rust resolver is `capture::desktop::workflow::electron::resolve() -> Result<Resources>`;
`Resources { executable, root }` supplies paths only. Surface owners launch their own
entry and own protocol, child cleanup and native acceptance. Assets remain in
`Contents/Resources/note-ui` on Mac and `usr/lib/apparent/note-ui` in Linux AppImages.
The executable is the existing Settings runtime at `Contents/Resources/settings/runtime`
or `usr/share/apparent/settings/runtime`; popup packaging adds no second runtime.
`package-notes.mjs` stages the required `electron/notes/` and includes `electron/share/`
when its `main.cjs` exists, alongside the same `dist/` and single `runtime/`. The absent
share consumer adds no entry or routing. A share owner must verify its installed route
on both platforms before enabling it; resolver availability alone is not that proof.
