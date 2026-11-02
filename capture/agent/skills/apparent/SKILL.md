---
name: apparent
description: Read and dig into Apparent screen captures, where someone recorded their screen while talking, clicking and drawing red marks on it. Use whenever a message contains an Apparent capture (a line like `[apparent 0:12 · App “Window” · /path/to/….apparent]`, a `.apparent` bundle path, or a contact-sheet.png from one), when the user says "look at my apparent" or refers to something they just recorded or drew, or when you need the `apparent` CLI to zoom into a moment, pull frames, copy a capture, or export one as video.
---

# Apparent captures

Apparent records someone's screen, voice, clicks and red ink, then packs the result into a `.apparent` folder (a bundle). They send it to show you a problem instead of describing it, so the capture is the bug report. Read it closely before you answer, and don't ask them to explain again what the capture already shows. If something is unclear, look closer with `apparent zoom` first.

## What a paste looks like

```text
[apparent 0:33 · OpenCode “Settings” · /Users/me/Library/Application Support/apparent/captures/apparent-1792432139176.apparent]
Open contact-sheet.png and frames/ in that folder before changing anything: the marks and words only make sense on the images, which can show more than the text does.
Cropped to: the activity in OpenCode (full frames in frames-full/)
- #1 (0:04) [clicked “New tab”] “Command T makes a new session, but here it should make a tab.”
- #2 (0:07) “No, it doesn't.”
- #3 (0:10) ⚑ “It says command shift B makes a new tab.”
- #4 (0:18) clicked (x 0.51, y 0.93)
```

- The first line has the length, the app and window recorded, and the bundle path.
- The next line asks you to open the images first. Do: the text is only what was said and done, and a problem can show in a frame without being marked or mentioned.
- Each `#N (M:SS)` line is a keyframe: what they did at that moment in brackets, then what they said while it was on screen. A line with no quote has the action alone.
- `⚑` means they flagged the moment on purpose. `⟨Name⟩` means the app reported a new screen. Lines starting `·` are events or values the app logged.
- Optional lines near the top: `Screen:` with app context, `Note:`, and `Cropped to:`. Older bundles may include `For session:` as recorded metadata; that does not authorize delivery.
- Coordinates are fractions of the captured area, from the top left (x 0.51 is just right of centre).

When you answer, refer to moments the same way, as in "#3 (0:10)", so the person can find them on the contact sheet.

## Reading a bundle

1. Look at `contact-sheet.png` in the bundle first (it's often attached already). It shows every keyframe in time order, left to right and top to bottom, each with its number `N` in a small white chip in a top corner (the `#N` in `summary.md`).
2. Read the transcript lines next to it. Speech recognition mishears names and code terms, so trust the screen when the two disagree.
3. Open `frames/frame-NN.png` for any keyframe you need at full size (`frame-03.png` is #3). When the summary says `Cropped to:`, the uncropped screens are in `frames-full/`.

If the contact sheet is not attached, try opening it at the bundle path in the first line. A plain `apparent copy` is text-only by default; the Paste control offers the image first and the text second, but a paste keystroke does not prove the composer accepted either. If you cannot open the local path (for example, the capture is on another machine), ask for `contact-sheet.png` as an attachment and check that you can actually view it. Do not claim to know what a click or mark targeted from “element unavailable” or coordinates alone; wait for the image when that target matters to the request. Never send a draft on the user's behalf.

What the drawings mean:

- Red strokes, circles, underlines and arrows are the person's marks, and they point at the problem. Marks drawn on the same screen all show on one keyframe. When there are several, the text numbers them in drawing order (`✎① 0:06 …`, `✎② 0:09 …`); the image shows only the ink, so use `apparent zoom --mark` to see which ink is which.
- A ripple around the cursor shows a click or tap, and a finger trail shows a swipe.
- A glowing red trail that tapers off is the laser pointer, used to point without leaving a mark.

Other files: `apparent.json` is the full manifest (keyframes, `marks` with shape and box, `events`, `transcript.words` with times, app `context`, and `autocrop`). `recording.mp4` or `recording.mov` and `audio.wav` are the source media. A `.media-pending` file means the video is still being copied from another Mac.

## The apparent CLI

### Configuring Apparent

Native app launch is a default-off candidate experiment on Mac and Linux. With an
updated, OS-registered app and `"experiments": {"nativeLaunch": true}`, the exact
`apparent://open` URL opens the idle app; it never records or sends, and leaves
active capture work alone. Other URLs/parameters are rejected. Linux registration
is explicit: `apparent launch-handler install`; `apparent launch-handler status`
reads the OS selection. See `capture/desktop/docs/native-launch.md`. Browser timers cannot confirm
installation or successful opening. Do not change a user's handler to test this.

The Settings controls for recording, shortcuts and retention use `~/.config/apparent/config.json` (`APPARENT_CONFIG` overrides the path). The feature map at `public/site/src/features.json` in the Apparent repository lists config keys, accepted values and platforms. Change only the needed key, preserve other keys, and keep the file private (`0600` on Unix): old configs may contain private values. Never paste or print the whole config. A programmatic edit must preserve valid JSON, rather than replacing the file with a partial object. The UI writes changes there; an explicit `"launchAtLogin": true` or `false` also updates OS registration on app launch or within two seconds while it runs. A disabled OS login item can still need the user's approval. Some values (such as captures folder) are read on demand; shortcuts are registered at app startup or when changed through Settings, so restart Apparent after editing `shortcuts` by hand. Permissions are OS grants, not JSON settings; do not claim to enable them by editing this file. Installing the CLI or the agent skill is an installation action, not an app preference.

`apparent` is the installed Apparent app's binary on macOS or Linux. If `command -v apparent` finds nothing on a Mac, use `/Applications/Apparent.app/Contents/MacOS/apparent` (or the same path under `~/Applications`); on Linux, use the installed `~/.local/bin/apparent` link or the AppImage's path. Settings › Agents can install the CLI link without replacing another program. Run the command on the computer with Apparent and access to the bundle; elsewhere, read `contact-sheet.png`, `frames/` and `apparent.json` directly. Most commands take a bundle path or `latest`, the newest capture. Quote paths containing spaces, such as macOS `Application Support` paths.

### A Connect capture on another computer

A paste may include `Connect capture: <32-lowercase-hex-id> (pending; ...)` (or `Hosted capture:` from an older app) and an `apparent fetch` command. The local path belongs to the recording computer, not necessarily yours. With a Connect-capable CLI, authorize a **separate read grant** and fetch the complete bundle. Update Apparent to use `apparent connect`; older apps use `apparent cloud`, which remains a compatibility alias in updated apps:

```sh
apparent connect login --server https://api.example.com --scope read --no-browser
apparent fetch <32hex-id> --out ./capture.apparent --wait 120
```

Use the server origin in the actual paste, not the example. Let the person approve the read scope at the printed verification URI; do not request or paste bearer tokens, reuse an upload token, or silently expose one through clipboard text. Read login does not enable uploads. `--out` must not exist and its parent must exist. Fetch waits for readiness, validates all allowed files and SHA-256 hashes, and atomically publishes the complete bundle; it never merges into existing output. `--wait` is pending polling time (0–900 seconds), not the full download deadline. If pending/offline/quota/auth fails, report that boundary and keep the local original; never infer image content from the ID alone. After successful fetch, open the sheet and frames visually before acting, as with a local bundle.

Uploader account/queue state is discoverable with `apparent connect status` (`--account` explicitly queries quota). Upload is default off behind `hostedSync` plus `cloud.enabled` and explicit `cloud.uploadConsent` in config.json. Only `apparent connect login --scope upload --consent-upload` opts in; do not do that merely to read a capture. `apparent connect disable` cancels pending uploads; `apparent connect logout` also removes local credentials. `apparent connect retry` retries eligible jobs under their existing IDs; it cannot revive cancelled or expired captures. Explicit `apparent connect upload <bundle>` queues a fresh ID on every invocation, including historical or changed bundles; do not use it as an ordinary retry. Updated clients renew approved device credentials automatically without changing the queued capture IDs or upload consent. Revoked or expired renewal requires device approval again; a network failure is not a reason to request new approval. Legacy credentials require an app update and one new login. Browser sign-in alone does not renew device access. Service login never adds trusted decryption keys. V1 rejects files above 90,000,000 bytes, never drops media to fit. See `connect/docs/hosted-client.md` for private credential artifacts, retry limits and complete-bundle restrictions.

### Look closer at a moment

Use `zoom` when the problem is motion (an animation, a jump, a flicker) or happens between keyframes, or when you can't tell what a mark points at:

```sh
apparent zoom <bundle> --at 0:21                 # a time, M:SS or seconds
apparent zoom <bundle> --at "the width changes"  # or a phrase they said
apparent zoom <bundle> --at 0:21 --span 1 --frames 16 --full
apparent zoom <bundle> --mark 2                  # the mark shown as ✎② in the text
apparent zoom <bundle> --mark 1 --frame 4        # ✎① of keyframe #4, when several have a ✎①
```

It prints the path of a PNG sheet of frames around that moment, the time of each cell, and what was said. Open the PNG and look at it before you act on the feedback. `--span` is the seconds covered (default 2, a quarter of it before the moment), `--frames` the number of cells (default 12), and `--full` shows the whole screen instead of the crop. The cells are numbered from #1 within the sheet, so those numbers aren't keyframe numbers.

`--mark` takes a mark's number from the text or its id (`m2`) and centres the sheet on the time it was drawn, with only that mark's ink, drawn whole from the moment it starts. `apparent inspect <bundle> --marks` lists every mark (`#2 ✎② 0:09 marked (2 strokes) · m2`: keyframe, number, time, shape, what it was drawn on, id), and `--marks --json` gives the same as JSON.

### Other commands

### Report an Apparent bug or improvement

Use `apparent issue --title 'Short title' --body 'Description'` without an experiment flag.
This prints a JSON preview without contacting GitHub. `--body-file feedback.md` reads
the exact Markdown description instead. Show the user the repository
`tryapparent/apparent`, title and full body and obtain explicit approval before
rerunning with `--submit`. Submission is public and uses `gh`'s github.com login;
it returns JSON with the verified issue URL. Never include private capture content,
transcripts, paths, credentials or personal details without approval. Nothing is
attached automatically. Search existing issues with `gh issue list --repo
tryapparent/apparent --state all --search '<symptom>'` first to avoid duplicates.
On timeout or failed readback, inspect the repository before retrying: an issue
may already exist. This files feedback only; it never starts implementation.
See `capture/desktop/docs/issue-reporting.md` for requirements and usage.

### Capture control

For a running GUI app on macOS or Linux, use `apparent capture start --wait --timeout 30`
to start through the normal interaction-aware screen path and wait for a real frame. It refuses
occupied sessions rather than toggling. Retain the reply's top-level `capture_id`, including on
failure or timeout; do not infer ownership from `capture state`. For driver crash recovery,
retain your own 32-hex request ID first and pass `--request-id <id>`; retrying that ID cannot
start twice. Portal consent and existing permission checks still apply; no app is auto-launched.

`apparent capture stop <id> --wait --timeout 60` stops only that ID and returns `status: "saved"`
with its exact `bundle_path` only after successful save. Without `--wait`, Stop still returns
acceptance, not completion. Stop while starting is `not_ready`; `capture cancel <id>` can discard
that owned pending capture. `capture inspect <id>` recovers its status/result after a timeout or
disconnect; retrying Stop cannot stop a newer capture. Receipts survive only in this GUI process,
not an app restart. Rejected start IDs remain rejected; use a new ID only for a deliberate new take.

`capture state` remains a read-only global phase/active-ID view. Keep stdout JSON separate from
stderr; failures exit nonzero. Never use a shortcut toggle, `latest` or a directory scan to recover
an owned take. These commands do not control timed `record --duration`, change export audio,
send to a session or add automatic paste. See `capture/agent/docs/agent-video-workflow.md` in the repository
for bounded recovery and video QA. Check installed help before using new flags on an older build.

```sh
apparent inspect <bundle>               # summary.md, the text above
apparent inspect <bundle> --json        # apparent.json
apparent inspect <bundle> --marks       # the marks, numbered as in the text [--json]
apparent frames <bundle> --from 0:04 --to 0:06 [--fps 4]   # plain PNGs of a range; prints their paths
apparent copy <bundle>                  # copy the capture to the clipboard
apparent export <bundle> [--out demo.mp4] [--no-zoom] [--crop auto|full|x,y,w,h] [--size 1920]
```

- `zoom` and `frames` write into `<bundle>/zoom/`. Leave the rest of the bundle as it is, because Apparent manages those files and cleans them up.
- Apparent does not send captures to agent sessions. Use `copy` to put a finished capture on the clipboard.
- `export` works on macOS and Linux (Linux requires `ffmpeg` and `ffprobe`). It writes one `.mp4` with ink and clicks burned in, audio in sync, and smooth activity zoom enabled by default (default `~/Movies/Apparent/<bundle>.mp4`). It opens on the whole capture for 0.5 s before zooming; explicit `--crop auto` or rectangles still bound the view. `--no-zoom` disables camera motion. The original recording stays unchanged. Without click/mark telemetry the camera stays still; use the app's recording shortcut for interaction-aware demos, not the timed `record --duration` path. `--no-ink`, `--no-clicks`, `--hevc` and `--fps` are also available.
- On Linux, `apparent update status` prints JSON with the selected channel, the **live process** version (`running.version`, or `null` if none), the installed AppImage identity, and whether a restart is pending. The `running` object in `~/.config/apparent/config.json` is a launch record, not proof the process is still alive after a crash; use `status` to validate its PID and start time. `apparent update channel stable|main` chooses the feed; `apparent update check` checks and stages an update for the next launch (installed AppImages only).
- Video and audio go to the Trash 7 days after a capture unless the person changed that in Settings. After that `zoom`, `frames` and `export` can't read the video, but `frames/`, the contact sheet and `apparent.json` remain.
- `apparent <command> --help` lists every flag, and `apparent skill` prints this guide as the installed version knows it.

The optional `make-it-apparent` workflow for a change demo, unresolved-problem demonstration or conceptual technical explainer is a **separate** skill. A finished implementation is not required; explicit live-app proof still requires real footage. The default capture-reading skill install does not guarantee it is present; install it explicitly in the agent's environment with `bunx skills add https://github.com/tryapparent/apparent/tree/main/capture/agent/skills/make-it-apparent`. Installing a skill does not install the desktop app or grant recording permissions.
