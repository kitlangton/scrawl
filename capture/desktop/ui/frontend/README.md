# Desktop UI foundation

Shared Electron Desktop surfaces use React/TypeScript, shadcn/ui and Tailwind.
Installed Settings routing, the recording pill replacement candidate and retained
native fallbacks have separate contracts and acceptance gates below. The opt-in
development window has Linux and macOS checks with their own proof limits;
foundation evidence does not establish pill, capture or overlay behavior.
No Windows support.

## Verification policy

Follow [Desktop verification](../../docs/native-testing.md): run the shared
renderer/workflow matrix and real Rust↔Electron contract checks on Linux. Mac
checks cover affected native boundaries, not a duplicate UI matrix. Reuse
applicable successful seam evidence for unchanged inputs; shared-only changes
need no new Mac walkthrough before merge. A short current-package Mac smoke and
existing CI/signing gates remain required before Mac publication. This policy
governs the workflow checklists below; specific human holds remain in force.
Synthetic fixtures never establish OS effects, and new native routes still need
focused real-platform checks. Automatic selection/reuse is not implemented by
this documentation change.

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
card; an explicit false hides it. Visibility alone never enables uploads. Installed General connection actions open
`--settings-connect` when its packaged assets are available. This mode loads the
existing Connect card with a Connect-only preload, and no Storage, capture, clipboard
or reporting capabilities. It consumes the originating native action once after
a successful local read; renderer reload never replays it. Navigation returns to
the existing Settings pages. Missing assets and startup/crash failure restore the
native General presentation. No onboarding route changes. See
`../../docs/onboarding-and-settings.md` for consent and recovery behavior.

Electron main owns one persistent, same-user backend socket. Opening and
one-second local refreshes never initiate network access. Each bounded request
uses Electron's monotonic clock; a ten-second idle lease, EOF, renderer reload,
crash or window close closes the lease and cancels pending authorization.
Connect-owned TypeScript (`connect/client/typescript/src/settings-controller.cts`
and `settings-presentation.ts`) owns the asynchronous controller, consent and
confirmation workflow, receipt ownership, activation intent and presentation.
Rust retains local snapshots, auth/renewal, credentials, health probes and durable
mutations behind closed backend RPC operations; these are explicit #529 migration
debt. Native EOF/lease enforcement durably cancels a login until activation has
been explicitly acknowledged against fresh committed gates. JS does
not read config or implement an auth client. Raw errors/config/device secrets do
not enter the renderer. Only an explicit zero-argument **Open browser** command
can open the current Rust-validated public verification URI; no arbitrary URL is
accepted from the renderer. Ambiguous commands are never retried automatically.

`tests/connect/transport.test.cjs` exercises the retained legacy transport;
Connect's owner tests cover the new controller/transport. `tests/connect/backend.smoke.ts`
exercises the shipped Electron controller through main/preload/React, including
consent, activation, confirmation and stale browser cancellation. The retained
`smoke.cjs` scenarios verify old-server fallback against labelled synthetic sockets.
`tests/rust-connect.cjs`, launched by the ignored
Rust `connect_settings_electron_rust_roundtrip` test, exercises the actual Rust
listener/auth client and shipped TypeScript controller using Electron's real clock
and a synthetic loopback device HTTP fixture. It verifies explicit consent,
public projection, durable cancel/EOF revocation, preserved config, rejection,
expiry, offline/retry and public encryption-key enrollment;
it does not approve a real account or upload media. Run it from the root with
`APPARENT_CONNECT_TEST_ELECTRON` set to the absolute frontend Electron executable:
`cargo test --offline connect_settings_electron_rust_roundtrip -- --ignored --nocapture`.
On Clanker, wrap the command once with `t3-build` as documented in `AGENTS.md`.
These checks cover shared consent/cancel/recovery behavior; native launch,
browser opening, focus and fallback need affected-platform seam proof. The earlier
shell evidence below does not certify this new workflow. Roll back by disabling
`connectSettings`; missing packaged Connect assets or a failed Connect window also
restore native General. Remove the legacy presentation only after shared coverage
and affected-platform boundaries pass under the verification policy above.

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
`readRetention()`, `saveRetention(policy)`, `resetRetention()` and
`readCaptureUsage()` are the only additional preload methods. TypeScript owns
retention defaults, custom-value validation, preset mapping, labels and notes:
Week is 7 media/30 bundle days, Month is 30/90, Forever is null/null. An absent
field falls back independently to the 7/30 default; explicit JSON `null` means
forever. Rust returns presence-aware retention values and a stale-edit revision to main;
main retains the revision and exposes only the policy view to the renderer. Save
accepts the two allowlisted `mediaDays`/`bundleDays` values, never a config object,
path or renderer revision. TypeScript selects the Settings capture location from
native facts; the native usage worker scans the actual effective folder and
returns formatted logical bytes.

The existing bounded socket worker admits `retention`, `saveretention`,
`resetretention` and `usage` through its unchanged activation/expiry gate.
Main retains the last authoritative retention revision; a concurrent edit
conflicts rather than silently overwriting it. The config transaction compares
fresh presence-aware owner values under the persistent config lock, preserves
unrelated top-level and nested policy keys and secrets, and durably publishes
only after file and directory sync. Reset removes only the recognized retention
fields. A custom hand-edited policy remains visible until explicitly replaced.
Malformed known retention values follow the existing cleanup reader's Week
fallback and are not echoed to the renderer. Writes fail closed on malformed or
unsafe config;
failed or outcome-unknown saves require a refresh and are never retried. Rust
config locking/CAS remains a named temporary executor dependency; the TypeScript
Storage route owns persistence intent. No Node-side config read/modify/write runs.

Saving/resetting nudges the **existing** Rust cleanup worker, as native Settings
does: it schedules a pass about five seconds later, deferring while recording.
The worker reloads policy on each pass; manual config edits apply on the next
pass (normally hourly; first launch pass after about 90 seconds). It retains
its startup capture folder until app restart. Nothing in the bridge runs a
cleanup pass or moves a capture; normal policy cleanup remains unchanged.

Usage scans the freshly resolved native capture location, not a cached renderer
path, on open and manual refresh only. It counts logical file bytes (not free
space or physical allocation), does not follow symlink entries, and returns
an error rather than a partial total on I/O failure, expiry, its 100,000-entry
limit or a directory nesting depth of 256. A missing root is empty.
Main and React discard old replies after a
location mutation, save, reload or unmount; the renderer also checks the returned
effective path. Loading/error states hide old totals, and Retry never starts
Rust. A slow filesystem syscall can outlive client expiry; the existing single
Storage worker stays busy rather than spawning unbounded scans.

The Rust Settings session owns navigation and the Electron child. The installed
page contains Storage only, with the existing Settings sidebar and system-following
light/dark Paper/Carbon/Signal palette. Other sidebar pages return to their native
presentation. Retired windows cannot send commands or close the current page.
Closing Settings cancels the session; an Electron crash restores native Storage.
Missing packaged assets also fall back to native Storage. Reopen Settings after
editing the opt-out. Saved preferences remain readable by both presentations.

`stage-settings.py` builds the locked frontend/runtime into Linux AppImage and Mac
installer/release packages. Installed packages contain no development dashboard or
synthetic examples. Node/npm are build prerequisites, not runtime dependencies.
macOS stages and signs nested Electron code before the enclosing app is signed.
The old Storage presentation is retained solely for the explicit opt-out and
startup/crash fallback. Remove it only after exact-candidate installed Mac/Linux
acceptance and a deliberate retirement of that fallback contract. Functional
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


### Storage policy acceptance (#530)

The policy move uses the existing installed Storage route. Relative to main
`02c3aa92`, it changes neither Electron 45.2.1 nor the native launcher,
window ownership, chooser options/parent, focus, permission, clipboard,
capture-exclusion or cleanup mechanisms. The added policy module is emitted by
`npm run build` before the unchanged packager copies the Electron tree; it has
no runtime imports. The location contract types are owner-local so emission
cannot pull Connect implementation into the Storage runtime.

Linux shared checks exercise the shipped main/preload/renderer and real Rust
socket. Native filesystem probes use the same create/metadata checks as the
retained capture-time safety resolver; config publication uses the same flock,
private temp file, fsync and rename. The new error marker records post-rename
uncertainty, including unchanged-value saves. There is no new native route or
OS effect to certify in this slice. The earlier installed-route evidence in
[PR #357](https://github.com/tryapparent/apparent/pull/357) (Mac `d70b20ce`,
Linux integrated `05bd4d9d`) and the retention evidence above remain evidence
for those original seams, not fresh executions of this policy candidate.
The runtime version is unchanged; subsequent main launcher/packaging work is
preserved, not certified here. No fallback is retired. Existing native holds,
including PR #376's Agents recovery, remain independent and open.

No fresh native GUI or Mac execution is claimed. Before Mac publication the
current packaged-app smoke, relevant native evidence and exact-main signing/CI
gates still apply. Ordinary Storage policy build/unit checks invoke no Rust
compiler; bridge/native changes still require their focused native integration.

## Checks

For ordinary Settings policy and renderer changes, `npm run test:settings` runs
the TypeScript production build, unit/contract tests, shipped Electron smoke
matrix under Xvfb, then the existing real Rust socket roundtrip. The bridge step
runs the prepared Rust test executable directly; it never invokes Cargo or
rustc. Its private receipt hashes the executable and all repository Rust/native
sources, statically embedded resources, build-script inputs, Cargo manifests/lockfile,
toolchain selectors and `.cargo` config. Loader-linked and adjacent staged shared
libraries and the loader environment are hashed too. Compiler identity is recorded
from preparation, not queried or revalidated by the ordinary lane.
Preparation rejects native inputs changing during compilation. The receipt must
name the required Storage test, the harness must list it, and the actual run must
report exactly one executed pass; zero tests and ignored tests fail.
Any native input, runtime library or executable change rejects the receipt and returns the work
to native integration. This proves the current TypeScript policy and shipped
Electron transport against that unchanged native listener; it does not replace
the conservative `desktop-ui-integration` lane or native gates for bridge or OS
contract changes.

Prepare the native test executable once when needed, then run the owner-local
checks without Rust tools available:

```sh
cd capture/desktop/ui/frontend
# Explicit native preparation; compilation occurs only in this step.
npm run prepare:settings-native
export ELECTRON_OVERRIDE_DIST_PATH=/absolute/path/to/preinstalled/electron/dist
# Offline runner also needs npm dependencies and the external Electron dist already installed.
PATH=/usr/bin:/bin npm run test:settings
```

The preparation writes `~/.cache/apparent/desktop-settings-native.json` with
mode 0600 (or under `XDG_CACHE_HOME`). On Clanker, run that preparation command
once through `/home/simon/.local/bin/t3-build -- npm run prepare:settings-native`;
do not nest the runner. The owner-local check requires Linux, Node 22+, Xvfb,
installed npm dependencies and a preinstalled Electron distribution outside
`node_modules`; it does not run `npm ci`, access the network, compile Rust or
launch the desktop app. The existing `desktop-ui-integration` lane remains the
conservative route for native/bridge contract changes.

The existing commands below remain useful independently:

Run Electron smoke files sequentially: Electron's lazy binary installation is
shared by both tests and concurrent first launches can race during extraction.

```sh
npm test                    # real Unix-socket fixtures; no GUI or Rust launch
npm run build               # strict TypeScript and production bundle
npm run test:electron       # real Electron, synthetic socket; needs a display
# On a headless Linux host:
xvfb-run -a npm run test:electron
```

For a Linux integration checkpoint before Mac access, run from the repository root:

```sh
(
TMPDIR="$(mktemp -d /tmp/apparent-ui.XXXXXXXX)" || exit 1
export TMPDIR
python3 enablement/owner_lanes.py --lane desktop-ui-integration --run --online
# Preserve this private directory if fixtures need diagnosis.
)
```

On Clanker, replace the Python command inside the subshell with
`/home/simon/.local/bin/t3-build -- python3 enablement/owner_lanes.py --lane desktop-ui-integration --run --online`.
Use the runner once; do not wrap an already admitted invocation.
The lane honors the runner's `T3_BUILD_JOBS` and runs npm unit tests, the existing
sequential Electron smoke script (including TypeScript/build) under Xvfb, then
both ignored `_electron_rust_roundtrip` Rust tests, with the absolute Electron
package executable supplied to `APPARENT_STORAGE_TEST_ELECTRON` and
`APPARENT_CONNECT_TEST_ELECTRON`. Those tests use the actual Rust listener and
controller with private temporary fixtures; the Connect HTTP account fixture is
synthetic. They run Electron in Node mode and need no display. No standalone
application build or installed app launch is performed.

The temporary root must be short, owned and mode 0700; failed fixtures may be
retained for diagnosis; the subshell preserves the caller’s TMPDIR.

Prerequisites: Node 22/npm 10+, Xvfb (`xvfb-run` and `xauth`), Electron's Linux
runtime libraries, Rust/system build dependencies and available build capacity.
The command above permits npm/Electron installation and uses Cargo `--locked`.
Omit `--online` for `npm ci --offline` and `cargo test --offline`; populate both
package caches first and set `ELECTRON_OVERRIDE_DIST_PATH` to an absolute,
preinstalled Linux Electron distribution directory outside this frontend's
`node_modules` (matching its locked Electron version). Offline mode requires
that executable before npm ci, preventing Electron's lazy downloader from
silently fetching a binary. Missing prerequisites fail rather than skip proof.
The ordinary `desktop-ui` lane remains lightweight and unchanged.

This checkpoint supplies shared renderer/synthetic transport and real Rust bridge
proof. It does not establish OS effects: changed portal/picker, permissions,
capture/output, focus or packaging boundaries still need focused native checks.
It need not be followed by a duplicate full Mac/Linux workflow walkthrough.
Follow `../../docs/native-testing.md` and `../../docs/linux/linux.md` for affected
seams; required exact-SHA CI/release receipts remain separate.

The Electron smoke checks controls/keyboard use, disconnected/error/reconnect,
renderer reload, minimum-width layout, sandbox/Node isolation, navigation/network
refusal and rejection of a second renderer. It is not native platform acceptance.
`tests/native-smoke.cjs` is a disposable-guest-only Electron entrypoint. The
existing isolated Linux runner is `enablement/native/linux/gui-vm.py`;
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

The installed Storage bridge is on unless `experiments.electronCaptureLocation`
is explicitly false in the Rust app's `~/.config/apparent/config.json`. File edits
apply at the next request; reopen Settings to change presentation routing. The
separate `npm start` window remains a developer harness; it is not installed or
used by the Settings route. Browser preview has no Storage bridge.

Installed Storage and Connect windows stay hidden until Electron's first paint
and the React surface's DOM-commit acknowledgement, including when the
native parent sends `show` during loading. React acknowledges its committed DOM
without waiting for hidden-page animation callbacks; the host still separately
requires first paint. A first paint alone can be an empty
document when its entry module is missing. Startup without that acknowledgement
exits after 15 seconds so the parent restores its native fallback; a startup
failure does not emit the user-dismissal action. Later `show`
requests still bring the painted window forward. Load failure and renderer crash
retain the native fallback route. `tests/storage/visibility.test.ts` exercises
the shipped host's event ordering with a delayed first paint; it is synthetic
lifecycle coverage, not macOS focus or rendering acceptance. The real Electron
`tests/storage/readiness.smoke.ts` removes the shipped React entry from a private
app copy and verifies hidden failure/exit rather than an empty visible window.
It also withholds animation callbacks while loading the real Settings renderer
and requires the committed surface to become visible.
Reverting this guard
restores immediate window display. The native/Electron pane replacement remains
in place; this guard alone does not resolve or certify issue #562's intermittent
blanking. Both Settings renderers open at 720×530 with a 210px black sidebar;
Electron uses inset macOS title controls, matches the native fixed-size window,
and centers after AppKit applies the titlebar metrics. Inactive sidebar clicks
select a pane on the first click, matching the native helper. It scrolls its content pane to keep
controls reachable at that size. The content still follows system appearance.
Reverting those layout styles/options restores the larger Electron presentation.
Removing the native fallback still requires the
accepted Settings routing switch and its platform checks.

`readCaptureLocation()`, `changeCaptureLocation()` and `resetCaptureLocation()`
accept no path, revision, dialog options or other arguments. Installed main reads
presence-aware native facts, then the emitted `electron/storage-policy.cjs`
(built from `src/storage/location-policy.ts`) owns saved-path normalization,
environment precedence, Settings candidate selection and fallback presentation.
The native adapter supplies home/built-in paths, environment presence/value, the
owner-key revision and a narrow availability probe for the TS-selected path.
Main reads the revision before opening its parented native folder dialog. Cancel,
reload, navigation and closing the window discard an unfinished selection.
Change and Reset are single-flight. A timeout never retries a mutation: refresh
to establish its actual outcome before trying again.

The same-user socket accepts strict JSON `{storage:"locationfacts",deadline}`,
`{storage:"probelocation",path,expected,deadline}`,
`{storage:"setlocation",path,expected,deadline}` and
`{storage:"resetlocation",expected,deadline}` frames with a trailing newline.
`deadline` is same-host monotonic milliseconds; Rust caps admission at three
seconds and rechecks it after acquiring the config lock for a mutation. Storage
uses libuv's clock: `CLOCK_MONOTONIC` on Linux, `CLOCK_MONOTONIC_RAW` on macOS.
`expected` is an opaque 16-hex stale-edit fingerprint of **only** the
presence-tagged raw `capturesDir` value (so absent and explicit JSON `null`
differ), not a security token or portable content digest. Other config edits
survive; a changed capture preference refuses the stale save. One worker handles
Storage without blocking the GUI or queuing config writers.

Rust keeps the flock/CAS transaction, preserves unknown config keys and secrets,
removes only `capturesDir` on Reset, and reports durable success only after file
fsync, atomic rename and directory fsync. It returns bounded location facts and
probes only the absolute candidate selected by TypeScript; the renderer receives
no revision, raw config, filesystem capability or diagnostics. TypeScript applies
`APPARENT_CAPTURES_DIR` presence before a saved preference, then built-in; empty
and relative environment overrides remain effective as before. A selected saved
folder that cannot be created/written displays the built-in fallback without
deleting the preference. The native capture-time resolver remains authoritative
for actual capture and independently enforces the same safe fallback for older
supported readers. Existing bundles keep their recorded directory; nothing is
relocated. Subsequent captures resolve the saved preference. Existing background
retention/transcript workers retain their startup directory until the Rust app
restarts; this slice does not change their lifecycle. Config CAS remains an
explicit low-level executor debt until a coordinated all-writer cutover.

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

## Drawing controls (#355)

The development window's **Draw across displays** section reads active screen
recording status and mark count, and **Undo last mark** invokes the same Rust
undo operation as the native shortcut (desktop ink or a newer pin). Hold Shift
and move over the desktop to draw; Electron does not intercept pointer events.
Refresh explicitly after drawing. Pin Undo first persists deletion of its typed note; a failed or timed-out receipt preserves the pin for recovery. This receipt has a two-second bound. No recording is started, target changed or
permission granted by this section. Phone and lab sessions are unavailable.

Routing: `npm start` opens the development controls; the separately running Rust
GUI owns admission, capture, input and overlays. This adds no feature flag and
does not activate any existing experiment or switch installed launchers. Linux and macOS use per-display native
panels. These are OS adapters, not competing React ink canvases. The native
shortcut remains usable alongside this control, as keyboard access to the same
operation. Closing Electron rolls back to the installed native controls without
changing config or active capture state. Remove the legacy control surface only
after the shared shell is routed and accepted on both platforms and all its
callers/build/package inputs are verified; this slice does not authorize that
routing change or removal.

The same-user private socket accepts typed `drawing: read|undo` JSON with a
monotonic deadline. Main holds the observed recording revision; the renderer
cannot supply it. Rust validates the deadline and exact recording identity on
the GUI thread before mutation. Navigation invalidates the receipt. Requests
are serialized, never retried automatically; after an ambiguous timeout refresh
before another undo. No paths, capture contents or raw errors reach React.

Transport and renderer fixtures are synthetic checks, not native drawing proof.
Exact-candidate isolated Linux and macOS cross-display proof, bundle text/image
inspection and a verified playable video remain required before acceptance.

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
until shared behavior and affected native seams pass under the verification policy
above and the routing switch is authorized. Mac proof covers installed entry,
popup ownership/focus, startup/fallback and actual clipboard; shared consent,
retry and recipient assertions belong in Linux checks. Socket/controller/browser
fixtures alone are not proof of native effects.

## Recording pill replacement candidate

`pill/index.html`, `src/pill`, `electron/pill` and `../electron_pill` implement the
shared pill presentation and parent-owned Rust transport. This candidate is
not evidence of accepted installed/native behavior. Do not remove the old
pill or treat the synthetic tests as native capture proof.

Rust launches the packaged pill alongside the existing Settings runtime; no
new config key or off-by-default product feature is introduced. Missing assets,
startup failure, bounded Hyprland pinning failure or child failure retain/restore the old pill for the remainder
of the app process. The old pill stays until Electron reports readiness;
Electron remains hidden until Rust closes the old pill and activates it.
Linux pin discovery waits for activation acknowledgement; hidden placement and
uncommitted renderer layout do not start native retries. Failure diagnostics
retain only phase and activation/readiness/visibility flags.
Authoritative dismissal hides the native window during renderer loading too;
hidden-state heartbeats do not repeatedly request native hiding.
Recording and finalization remain owned by Rust across renderer reload/exit.
A failed or ambiguous action is never automatically retried. The keyboard
shortcut and tray remain independent controls.

Before installed routing may land, verify shared selection/state/action rendering,
warnings, processing/failure states, sharing refusals and theme/reduced-motion
on Linux. Verify affected native seams on each OS: actual microphone input where
supported, Finish/Cancel/Whiteboard admission, clipboard/drag-to-paste,
drag/clamping/new-capture placement, click-through, focus return,
fullscreen/workspace visibility, capture exclusion and startup/crash/fallback/
shutdown. Reuse applicable evidence for unchanged boundaries; do not replay the
shared matrix on Mac. Inspect actual bundle text,
contact sheet, full-quality frames and movie for interaction preservation and
capture inclusion/exclusion. Preserve native glass/config compatibility until
its replacement is established. Compare response and resource measurements
with the same native baseline; copied-link and watch-ready timing differ.

Remove old presentations only after those applicable gates pass and their
consumers/build/package inputs are verified. Until then the native adapters and
fallback are required. The implementation's remaining parity gaps must be
closed before enabling the installed replacement through a release.

`tests/pill/smoke.cjs` runs shipped Electron/main/preload/React against an
explicitly synthetic parent stream. It checks state rendering, stale and
unavailable action refusal, keyboard cancellation, themes/reduced motion,
renderer reload and isolation. It does not prove real Rust command dispatch,
native desktop mechanics, clipboard or capture output.

On Linux, a user-owned runtime without a valid root-owned SUID helper requests
Chromium's namespace sandbox instead. Kernel restrictions still refuse startup
and restore the native controls; production never adds `--no-sandbox`. The local
Xvfb renderer fixtures use that flag for testing and do not establish OS sandbox
acceptance. Native runtime/namespace proof remains required.

Known expired commands return a new revision and a visible rejection without
dispatch; an old command never consumes a newer capture's revision. Readiness
requires the owned React renderer to acknowledge its committed state/layout,
not just successful HTML loading. Missing renderer assets time out without
retiring the functioning previous controls. Mac paste gestures temporarily keep
mouse delivery through release/cancel; programmatic size changes clamp every
phase to its display. Native glass and Mac capture-exclusion opt-out continue
on their original presentation, including changes between captures; retiring
Electron windows remain explicitly excluded from the next stream.

The parent receives `BrowserWindow.getMediaSourceId()`'s native window ID with
readiness, before activation. Mac compatibility retirement therefore does not
depend on a window being onscreen. Linux registers the XID for candidate-only
masking before showing the pill and after each session reset; delivered movies
and full-quality frames retain their existing inclusion behavior. A Mac route
that returns to native compatibility stays native until the next app launch,
avoiding overlapping Electron profiles during asynchronous retirement.

## Shift-click notes

Linux recording controls retain their native drawing/input shape across unchanged
measurements. Geometry, note-extension, pocket and display-scale changes reapply it;
clock and microphone heartbeats keep the existing region.

While the Electron recording pill is routed, its flush Lilac right extension
shows a shared Lucide note icon and the durable saved count, hidden at zero.
The main row stays at fixed coordinates across count changes; its transparent
72px native reserve is not an input target at zero. The extension opens the
existing list, without changing the editor. Native fallback retains the separate
Notes control. Reverting the count integration restores that control; remove its
fallback only when the native pill routing and package consumers are retired.
Shared Linux coverage checks count transitions, screen coordinates, hit shapes,
themes, revision-scoped list actions and the real Rust note authority. Native
window shaping/placement and popup ownership are the affected platform seams.

The default-available notes surface runs `electron/notes/main.cjs`, launched by
Rust for a screen capture. It renders only the nearby text popup and a small
compact Notes control/list beside the existing capture controls; picker, ink and
capture lifecycle remain on their current native path. There is no competing
Electron capture state machine. This is independent of the foundation Settings
window and requires no experiment. Packagers ship its production assets and
Electron runtime via `scripts/package-notes.mjs`; development must run `npm ci`
and `npm run build` before starting the rebuilt Rust app.

A private inherited pipe carries click-time anchors and bounded note commands.
Only trusted local main frames can submit draft/save/edit/delete/dismiss/list;
no renderer paths, window IDs, IPC names, shell or network commands are accepted.
Rust writes saved notes to `notes.json` before acknowledging/counting them.
An acknowledged nonempty save briefly flies a click-through letter to the counter.
It reuses the owned popup, which the native input filter temporarily treats as
passthrough; new notes and Finish cancel the animation. Reduced motion hides the
decorative flight. Animation never delays saving or the Finish barrier.
Linux anchors convert GPUI DIPs to the panel's physical X11 geometry, including
fractional display scales. Native focus arrival refocuses an open textarea.

Unsent drafts stay in the capture's Rust session rather than note metadata/summary. Linux display video still includes the visible popup and its text. Enter with no text closes the
field and retains the pin. Escape keeps the in-capture draft; Shift+Enter adds
a newline. Shift-move outside the note retains its draft and returns to drawing
once the pointer has moved beyond the existing ink threshold; Shift typing and
selection inside the editor stay protected. Capture completion closes this capture-local surface. Restarting or
ending a capture does not promise recovery of unsubmitted drafts.

The native menu and Electron Recording control share `microphoneEnabled` in
`~/.config/apparent/config.json`. It defaults on, preserves device selection,
and applies at the next capture. Off skips microphone admission/acquisition;
the bundle records `context.microphoneCapture=off`, including local recovery,
so intentional silence does not masquerade as an audio failure. Config writes
use Rust's existing atomic writer and preserve unrelated keys. No setting edit
interrupts active audio.

On Linux, Electron uses X11/XWayland. Rust checks each XID against this capture's
child PID and prepares it while hidden; Electron alone shows and hides it. The
compositor's visible pinned order can diverge from X11's input stack after focus
returns to the recorded app. The notes worker checks that stack at most every
100 ms and raises its visible surfaces only when needed. Renderer commands cannot
place windows. Counter/editor clicks are excluded from capture telemetry.
After an explicit editor/list opening, native placement also restores X11 keyboard
focus after Hyprland placement completes. The compositor's active-client identity
alone is insufficient: X11 focus can be cleared during that handoff. Only the
still-active compositor note can receive this repair. Dismissed or replaced
surface requests cannot reclaim focus; counters and decorative flights
remain inactive. This is a Linux adapter correction; notes routing is unchanged.
Focus ownership is checked after geometry placement, before compositor activation
and again before the X11 repair, so an intervening app switch wins.

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

Mac Rust test binaries request software rendering for the Notes bridge fixture,
whose hosted environment may lack an EGL display. Production launches retain
normal GPU behavior and sandboxing. The fixture still requires the real renderer's
Ready acknowledgement within eight seconds; software-rendered bridge results do
not certify native GPU, focus or window behavior.
Rust test binaries also request fixed-label Notes startup diagnostics on stderr.
CI retains only the synthetic fixture's `notes-ui.log` before deleting its temporary
root; labels contain no note text or error contents. Diagnostics do not acknowledge
Ready, change the eight-second deadline or certify a failed bridge test.
The CI Rust-to-Electron bridge fixtures run serially on their shared desktop;
independent application startup is not a concurrent desktop stress test. The
Notes native-startup labels distinguish activation-policy and window-construction
stalls without increasing the readiness deadline.

The shared Rust resolver is `capture::desktop::workflow::electron::resolve() -> Result<Resources>`;
`Resources { executable, root }` supplies paths only. Surface owners launch their own
entry and own protocol, child cleanup and native acceptance. Assets remain in
`Contents/Resources/note-ui` on Mac and `usr/lib/apparent/note-ui` in Linux AppImages.
The executable is the existing Settings runtime at `Contents/Resources/settings/runtime`
or `usr/share/apparent/settings/runtime`; popup packaging adds no second runtime.
`package-notes.mjs` stages the required `electron/notes/` and includes `electron/share/`
when its `main.cjs` exists, alongside the same `dist/` and single `runtime/`. The absent
share consumer adds no entry or routing. A share owner must verify affected installed
route seams on each platform before enabling it, reusing applicable evidence under
the verification policy above; resolver availability alone is not that proof.

### Installed public-copy confirmation (#171)

Completed Copy link opens the independent `electron/share` entry in the one
shared packaged runtime resolved by `workflow::electron`. It shows only the
selected capture and the explicit anyone-with-link/non-E2E disclosure. Its
private parent pipe carries capture name/status and bounded confirm/cancel/deliver
commands, never a capture path, share URL or account token to the renderer.
Rust pins the completed bundle path, inode and generation before opening; every
poll revalidates them, and only the GUI thread may deliver a prepared clipboard
write. Electron requests delivery only while focused; Rust independently rechecks the current OS foreground owner immediately before clipboard admission. Losing focus between those steps cancels the operation without a clipboard write; close and choose Copy link again to retry. Closing,
selection changes, new recording and expiry cancel future effects; an admitted
clipboard write or transmitted data cannot be recalled. A copied result remains
separate from upload/recipient readiness. No automatic ambiguous-write retry.

The completed pill's ordinary timeout pauses while this capture-bound popup is
open. New capture/phase transitions still close it; local Paste stays available.
Disconnected users use the existing General connection setup, then choose Copy
link again. Missing assets fail with an actionable local-Paste message; there is
no legacy direct-public-copy fallback. The old direct reservation path is removed.
Existing hostedSharing and service activation holds are preserved; this adds no
new experiment. Keep this routing only after macOS/isolated Linux acceptance;
rollback must restore a verified consent surface, never bypass public-copy consent.
The development foundation card remains a diagnostic surface, not an installed
second capture workspace. No #172 ChatGPT-web or deployed-account proof is implied.

The scoped hosted macOS native route is `native-proof-mac.yml`, dispatched with
`scenario=share` against the frozen candidate. The default worker case remains
unchanged. The share case uses a synthetic full-screen scene, normal capture
start/stop, the installed completed-pill action, native owned-window OCR/input,
explicit consent/cancel/repeated clipboard delivery, and the actual loopback
Worker. It retains synthetic screenshots/bundle and available decodable video;
it does not edit TCC or establish deployed-account/ChatGPT acceptance. A long
open confirmation pauses completion dismissal; closing grants a fresh retry interval.

The share Electron entry/preload are authored as `.cts`. `npm run build` checks
and emits their ignored `.cjs` runtime files; popup packaging stages only those
outputs. The share smoke and hosted native case are TypeScript run by Node 22.
