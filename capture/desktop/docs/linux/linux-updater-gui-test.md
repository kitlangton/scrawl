# Linux Settings updater: packaged GUI acceptance

This is the repeatable acceptance check for [#85](https://github.com/tryapparent/apparent/issues/85).
It clicks the real Settings controls in a disposable Omarchy/Hyprland guest.
It does not call `update check`, inject Settings model events, or add a production
endpoint override. The fixture exercises the production updater and frog UI.

## Prerequisites and containment

Use a sanitized, **stopped, standalone qcow2 template**, its matching stopped OVMF
variables, a test-only SSH key, and a desktop-autologin test account. No guest
root access is required. The template must have
Hyprland, OpenSSH, Python 3, OpenSSL, curl, and the runtime dependencies required
by the AppImage. It must not contain personal files, accounts, or an autostarted
Apparent process. The scenario refuses an existing Apparent process rather than
replacing a worker's app. Never use X1, a live VM disk, or another worker's guest.
Preparing or replacing a template requires its own coordinated maintenance window;
this test never edits the template or the original VM launcher.

Host prerequisites: Linux/KVM access, QEMU (`qemu-system-x86_64`, `qemu-img`),
OVMF at `/usr/share/OVMF/OVMF_CODE_4M.fd`, SSH/SCP, a working user systemd manager,
Python 3.11+, Pillow, Tesseract with English data, FFmpeg/libx264 and ffprobe.
The runner uses a separate writable overlay, firmware copy, loopback SSH port,
Unix QMP/VNC sockets and known-hosts file for every invocation. An independent
user-systemd expiry stops only that job's QEMU after 660 seconds, even if the
controller is killed. Normal completion stops QEMU before removing writable state.

Coordinate RAM/CPU with other GUI jobs first: this job requests 3072 MiB and four
virtual CPUs. Check both host capacity and the launching worker's cgroup. Keep
builds and packages in the assigned checkout, never a shared worker's cache.

The controller always starts QEMU with `restrict=on`: guest outbound networking
is denied, while the loopback host-to-guest SSH forward remains available. A
rootless CONNECT fixture on guest loopback port 8443 handles only `api.github.com`
and `github.com`; no hosts file or system trust store changes. A generated
guest-local TLS certificate covers both names. The app's curl still verifies TLS,
using the explicit job-local CA and HTTPS proxy; other proxy variables are cleared.
The fixture never proxies unknown routes upstream. No real GitHub release or
asset is created, and no user installation is touched.

## Build, package and run

Build the exact checkout after tests. Use the repository's offline build settings
and reduce concurrency when shared resource capacity requires it:

```sh
cargo test -j 1 --offline -- --test-threads=2
GITHUB_SHA=$(git rev-parse HEAD) cargo build -j 1 --offline
./capture/desktop/scripts/linux/appimage/appimage.sh target/debug/apparent
python3 capture/desktop/scripts/linux/updater/test-updater-gui.py
python3 enablement/native/linux/test-gui-vm.py

python3 capture/desktop/scripts/linux/updater/updater-gui.py \
  --template "$GUI_TEMPLATE/system.qcow2" \
  --ovmf-vars "$GUI_TEMPLATE/OVMF_VARS.fd" \
  --ssh-user "$GUI_TEST_USER" --ssh-key "$GUI_TEST_KEY" \
  --appimage target/appimage/Apparent-x86_64.AppImage \
  --results "$TMPDIR/updater-gui-unique-run"
```

Use a **new, short, private results path** outside git/public assets (Unix sockets
have a path-length limit). The AppImage must be x86_64/type 2; the fixture currently
covers this architecture. Stable remains the default; add `--channel main` for
Main, using a separate new results directory. Neither mode proves aarch64. The packaging
script recreates AppDir and checks its H.264 encoder. `linuxdeploy` patches ELF
library paths, so the pre-package ELF hash can legitimately differ from its
packaged copy. The test binds acceptance to the **whole AppImage hash**, verifies
that hash again before the guest starts the app and at final acceptance, and checks
the actual GUI process's `APPIMAGE` value
and embedded `running.revision` against the checkout's HEAD. The entire repository
must be clean before boot: staged, unstaged and untracked changes are refused,
including docs and tooling. There are no path exclusions; Git-ignored build output
is allowed. Rebuild after committing so the embedded revision matches.
Do not substitute an old AppDir, a debug executable with a forged `APPIMAGE`, or a
previously installed app. `candidate.json` records the source HEAD and image hash;
retain the build/package logs alongside the evidence.

## What the real GUI scenario asserts

1. Launch the packaged candidate with a private config and captures directory.
   Its acknowledged automatic startup check sees a valid current release.
   A second launch opens Settings via the actual IPC path; QMP clicks **About**.
   The disposable guest's Omarchy shell is stopped to hide its first-boot tips;
   the actual app and Hyprland compositor remain running.
2. Click **Check for Updates…**. Hold the fixture's metadata response at an
   explicit gate and assert the rendered **Checking…** button before releasing
   it. Click that disabled button again; the fixture request counts must show
   exactly one manual request. Then assert **No update available**, and click OK.
3. Repeat with a controlled HTTP 503; assert **Update check failed**, dismiss,
   and retry successfully in the same GUI process.
4. Offer a synthetic newer release with a wrong checksum; assert the failure,
   unchanged target hash/inode, absent installed marker, and no staging leftovers.
5. Retry with the correct checksum and the **same exact candidate AppImage bytes**.
   Assert **Update available**, installed synthetic release identity, changed target
   inode, original backup inode/bytes, and a still-running original GUI process.
6. Click **Update later**, recheck, press Escape, and recheck again. The staged
   dialog must disappear after each action, with Settings still visible and the
   same live PID and pending restart. Each recheck requires a fresh dialog; the
   result stays available without another image download. These actions defer
   the restart; they do **not** cancel a download or undo an installed update.
7. Click **Update now** only in this disposable guest. Require a new live PID,
   recorded synthetic update ID, `pendingRestart: false`, and a reopened real
   Settings screen. The payload is the same candidate, not a newer product build.

### Main-channel variant

`--channel main` persists Main in the private config and keeps the same loading,
HTTP failure/retry, checksum, staging, Later, Escape and restart assertions.
Startup advertises `linux-main-v2-<source HEAD>` matching the embedded revision;
no AppImage may be downloaded. The fixture serves the static Main manifest;
every API route returns HTTP403. Asset URLs retain the canonical GitHub release
path. The synthetic newer revision is forty `b` characters (the runner refuses a
matching source HEAD). Its ancestor list includes source HEAD. Before staging,
a controlled divergent list must fail without replacing the image or marker.

After staging, the fixture advertises the original source revision again. A real
manual check must reject the stale manifest, preserve the staged Main ID and
target/backup inodes, and not download another image. `stale-feed.json` retains
the status. Request assertions forbid API requests and require exactly two image
downloads: the rejected checksum and successful stage. Restart must report the
synthetic Main ID with pending restart cleared.

Immediately before restart, Main also starts an owned capture through
`capture start --request-id <32hex>` without waiting for portal consent. It
retains the request ID, start receipt, exact-ID inspection and global state in
`capture-guard-*`/`capture-start.*` artifacts. Starting or recording is accepted
only with an error-free receipt for that request and the same active capture ID.
A real **Update now** click must produce the coordinator's deferred-restart log,
leave the same PID and pending update, and preserve that owned active capture.
Then `capture cancel <exact capture_id>` must return cancelled, and a separate
state read must be idle with no capture ID before the idle restart is attempted.
CLI operations have 12-second limits; the guard-log wait has a five-second limit.
Admission, portal or cancellation failures retain evidence and fail the run
instead of waiting for recording indefinitely or cancelling an unknown take.

`capture-guard-limit.txt` states the exact receipt status at the blocked click.
`starting`/selecting is admitted-startup evidence only, not proof of recording
frames. `recording` means the receipt reports a real frame; neither outcome
proves a saved bundle. Cancellation checks the CLI state, not disappearance of
native portal windows/requests; this is not a portal cleanup acceptance test.

Both channel variants deliberately serve **the same candidate AppImage bytes**
under a synthetic newer identity. Main tests ancestry decisions and local
staging/relaunch, not a genuinely newer binary, live GitHub publication or arm64.
The lightweight Python tests exercise fixture routes and generated scenario
syntax without booting a guest; they do not prove these GUI assertions.

All results are located/asserted through OCR of actual compositor screenshots;
QMP only supplies pointer/key input. Missing or ambiguous text fails closed. The
locator requires the whole spatial text run (splitting separate controls on one
OCR line), so “No update available” cannot satisfy
“Update available”. Request logs, process identity, status JSON, inode and hash
checks provide separate evidence beyond pixels. OCR may remove long bright
button outlines from a derived grayscale image; raw evidence frames stay unchanged.
The fast tests cover harness
contracts and cleanup, **not** GUI behavior; CI names that distinction explicitly.

## Inspect evidence and make the result apparent

A passing run requires **both** top-level `result.json` with `passed: true` and
`vm/result.json` with `scenario_exit: 0`, plus `UPDATER_GUI_PASS` in
`vm/scenario.log`. Inspect `steps.json`, each result frame, and
`vm/artifacts/{requests.jsonl,baseline.json,deferred.json,escape-deferred.json,relaunched.json}`.
Read the actual app log for expected errors. Compare `candidate.json`'s hash
with `vm/artifacts/candidate.sha256`; inspect the process path and saved inode
checks. A successful QMP input command or an unopened Settings screenshot is
not acceptance.

The controller retains **all** timestamped compositor samples, an ffconcat list,
`recording.json` and a playable `raw.mp4`. This is a continuous **QMP observer
recording**, not Apparent's own capture/export or smooth native 30 fps footage.
OCR work makes sampling cadence variable; inspect `max_sample_gap_seconds` and
the actual frames. The movie preserves real-time gaps and encodes at 30 fps; that
does not manufacture intermediate motion. Loading assertions happen while the
response is blocked; they prove feedback before network completion, not a
benchmark of production-network latency.

Run the repository's `capture/agent/skills/make-it-apparent/SKILL.md` workflow for delivery:
inspect the complete raw movie, add timed plain-spoken explanation and readable
captions, disclose generated narration and the local fixture/sampling limits,
inspect the finished movie and deliver it privately in the requesting thread.
Keep original samples and editable captions. Never commit footage, generated
certificates, guest keys, transcripts or private capture bundles, and do not post
private footage to an issue/PR. This proof establishes the packaged Linux GUI and
local release-contract/staging/relaunch path. **It does not certify actual GitHub
publication, another architecture, an unexecuted channel, macOS, or a real-user update.**

If a run fails, retain its result directory and last screenshot/OCR output.
Distinguish harness/environment failure from a reproduced app defect before
changing runtime code. Fix concrete harness issues and repeat against the same
image hash; do not hide failed attempts or manufacture an updater change merely
to give this acceptance work a runtime diff.

### Signing migration variant

Add `--channel main --legacy-appimage /private/path/legacy.AppImage` to the
controller command to exercise the November 2026 signing-history migration. The
controller accepts only the published x86_64 `a5cf47d` asset digest and stages it
as a separate fixture file. The guest first runs that real old AppImage, discovers
the synthetic permanent v1 bridge despite a later v2 release, and uses actual
Settings clicks to stage and restart into the exact packaged candidate. It then
checks the same-source v2 feed, stages a later synthetic v2 identity, defers and
rechecks before restarting again. Both payload downloads use the exact candidate
bytes. Assertions bind the old/new running revisions, all three installed markers,
request counts and both restart receipts. This proves the packaged compatibility
protocol in isolation; production signing, publication and asset reachability
remain separate checks.

### Failed-check cooldown

Failed discovery, validation or installation checks persist a one-hour cooldown
in `.APPIMAGE_NAME.update-retry` beside the owned AppImage, under its transaction
lock. Startup, Settings and CLI checks share it across process restarts. Checks
within that interval fail explicitly with remaining seconds; they do not report
“up to date” or request metadata again. A backwards clock adjustment removes a
future timestamp. This is runtime retry state, not a user preference. Successful
checks keep the existing hourly cadence. Native acceptance must exercise a failed
check, a second Settings check and a restarted app against the fixture, verifying
no additional requests during cooldown, then expiry and successful installation.

### Static Main feed contract

Normal Main checks fetch only
`https://github.com/tryapparent/apparent/releases/download/linux-main-feed/main.json`.
Schema 1 has `source` (full lowercase SHA), sanitized `release` (v2 tag,
publication flags/time and four exact asset names/URLs), `checksums` (AppImage
name to original checksum text), `ancestors` (complete source history including
source), and `bridges` (`id` and canonical `source` pairs). The manifest is bounded
to 4 MiB, 50,000 ancestor identities and 100 bridges; metadata requests have a
30-second deadline. Truncated, duplicate, malformed, stale or diverged identities
fail closed. Main has no release-list/API fallback. Stable retains its existing
single-release API path.

The producer `maintenance/main-feed.ts` runs in the existing Linux workflow
publication lock, rechecks exact Linux admission, requires full Git history and
source membership in trusted Main, and verifies the immutable v2 tag and both
remote artifact/checksum digests before publishing. Bridge mappings require the
legacy parent, canonical source parent and identical trees. It never moves or
rewrites existing tags or v2/legacy assets; only `main.json` is replaced and read
back. GitHub asset replacement can briefly return 404; clients fail closed and
retry after cooldown. A same-source feed is left untouched, and stale/divergent
publication is rejected. The separate metadata job also runs when an already
published v2 release skips rebuilding, recovering interrupted first publication.

Consumers validate the installed Main identity against ancestry on every check,
including cache hits. A bridge identity additionally requires its exact compiled
canonical source. Explicit channel switching retains its existing semantics.
Download SHA-256 verification, failed-image suppression, atomic installation and
rollback are unchanged. Automatic checks may reuse structurally validated metadata
for five minutes in memory. Explicit Settings/CLI checks refresh metadata; failed
refresh never falls back to cached data. Failure cooldown spans process restarts;
success clears it. Invalid/oversized/future-dated retry markers are removed under
the transaction lock. There is no persistent metadata cache to trust after restart.

The isolated Main fixture returns HTTP403 for every API route and serves the
static manifest via the real HTTPS URL. Its Settings checks cover success,
network failure/cooldown, divergence, checksum rejection and stale-feed rejection
without API requests. Old API fixtures remain solely for pre-feed legacy updater
migration. The permanent bridge and v2 publication namespaces remain available to
late upgraders; older installed binaries still use their original discovery path.
Actual GUI and installed-upgrade evidence must be recorded separately from the
fixture and Rust contract tests.
