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
python3 capture/desktop/scripts/linux/gui/test-gui-vm.py

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
Startup advertises `linux-main-<source HEAD>` matching the embedded revision;
no AppImage may be downloaded. The fixture serves the canonical paginated
GitHub releases API: an unrelated stable release on page 1, the usable Main
prerelease on page 2, and an empty page 3. Asset URLs retain the canonical GitHub
release path. The synthetic newer revision is forty `b` characters (the runner
refuses a matching source HEAD). Its compare response for `source...bbbb…` is
`ahead`; the reverse is `behind`. Before checksum/staging, a controlled `diverged`
response must produce a failure without replacing the image or installed marker.
Comparisons use `?per_page=1&page=2`; `base_commit.sha` is the base revision in
each direction, not the candidate revision.

After staging, the fixture advertises the original source revision again. A real
manual check must still offer the pending update, preserve the staged Main ID
and target/backup inodes, and not download another image. `stale-feed.json`
retains the status. Request assertions require the reverse comparison and exactly
two total image downloads: the rejected checksum and successful stage. Restart
must report the synthetic Main ID with pending restart cleared.

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
