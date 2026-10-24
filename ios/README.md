# Scrawl for iPhone

Show an agent what's on your phone. Record the whole screen, whatever app you open, while you
talk; or share screenshots or a screen recording from any app. Scrawl on your Mac builds the same
bundle it builds for a Mac capture (keyframes, contact sheet, `summary.md`) and queues it into an
OpenCode session, which you can pick from the phone.

```
iPhone                                Mac                                      OpenCode
Scrawl app ── GET /sessions ───────▶ Scrawl.app or scrawl serve (:47777) ──▶ the session list
broadcast extension ── POST /ingest ▶ builds the .scrawl bundle ───────────▶ queued prompt
share extension ───── POST /ingest ▶ (keyframes, transcript, contact sheet)   + contact sheet
```

It speaks ScrawlKit's protocol (README › ScrawlKit): the same `scrawl link` pairing, the same
`POST /ingest` (a 4-byte big-endian timeline length, the timeline JSON, the MP4) and the same
replies, so the Mac treats its recordings like ScrawlKit's. The timeline's `capture` says what it
is: `broadcast` (the whole screen) or `shared` (screenshots or a recording shared into it).

## What's in here

| Target | Folder | Does |
|---|---|---|
| Scrawl (app) | `Scrawl/` | Pairing (scan or paste the `scrawl link` code), the Mac's status, which session captures go to, the Record button, the list of captures and what became of each, Send a Test Capture, the Flag Moment shortcut |
| ScrawlBroadcast | `ScrawlBroadcast/` | A ReplayKit broadcast upload extension: records the screen and microphone into an H.264/AAC `.mp4` (1920 px long edge, 30 fps, as ScrawlKit records), writes the timeline, and hands the upload to iOS when the broadcast stops |
| ScrawlShare | `ScrawlShare/` | Up to ten screenshots (made into a movie, two seconds and a `⚑` each, named `Screenshot 2 of 3`) or one screen recording, with a note and the session to send to |
| Shared | `Shared/` | Compiled into all three: pairing (Keychain group), captures (app group container), the uploader, the timeline, the movie writers, notifications |

## Build and run

Needs Xcode 26 or later and [XcodeGen](https://github.com/yonaskolb/XcodeGen) 2.46 or later
(`brew install xcodegen`). `project.yml` is the source of truth; the project isn't checked in.

```sh
cd ios
xcodegen generate
open Scrawl.xcodeproj
```

The simulator runs everything but the broadcast extension (ReplayKit broadcasts need a device).

**On your iPhone:** copy `Config/Local.xcconfig.example` to `Config/Local.xcconfig` and set your
team and an app ID of your own (`SCRAWL_APP_ID = com.you.scrawl`). The extensions, the app group and
the Keychain group follow from it, and automatic signing registers them. Then pick your phone in
Xcode and run.

## Pair

Scrawl runs on the Mac (the app, or `scrawl serve` on an always-on one). In a terminal there:

```sh
scrawl link
```

It prints a `scrawl://scrawl?url=…&token=…` link and its QR code, with the Mac's Tailscale and LAN
addresses. Scan the QR code with the iPhone's Camera (it opens Scrawl), or scan or paste it in the
app. `scrawl link --session ses_…` addresses every capture to that session; otherwise pick one in
the app under **Send to**, or leave **Newest session** for the Mac's most recently active one.
**Send a Test Capture** (Settings) checks the whole way to OpenCode. The iPhone and Mac need to be
on the same network or tailnet; an `https://` link (behind Tailscale Serve, say) works anywhere.

## Record

**Record Screen** in the app, or Control Center's Screen Recording (hold it, pick Scrawl), starts
a broadcast; turn the microphone on in the sheet. Open any app and show the problem while you talk.
Stop from the red indicator. The recording goes to the Mac as soon as it stops, and a notification
says how it went ("Sent to SCRAWL", "On your Mac", "Couldn't reach your Mac").

iOS doesn't let one app draw over another or see its touches, so there's no ink and no tap
tracking. Instead:

- Saying "look here" or "now" as the screen changes cuts a keyframe there (the Mac's spoken cues).
- **Flag Moment** (an App Shortcut, for the Action button, Back Tap or Siri) flags the moment: it
  gets a keyframe marked `⚑`, as the cabled mirror's M key does.
- Recordings stop at 4 minutes for a Mac (about 180 MB at 6 Mbps), 3 for an HTTPS endpoint (3 Mbps,
  under its 95 MB).
- Turning the phone mid-recording is noted on the timeline (`turned to landscape`); the movie keeps
  the orientation it started in.
- ReplayKit sends frames only while the screen changes; the movie runs on to the end of what you
  said, holding the last frame.

## Delivery

A capture is a folder in the app group container (`Captures/<id>/`: the movie and timeline, named
alike so `scrawl import` pairs them after an AirDrop, plus `capture.json` with its state). Uploads
are file uploads on background `URLSession`s, so the broadcast and share extensions can hand one to
iOS and exit; iOS wakes the app to hear the Mac's reply. Each attempt has its own session identifier
(`<app group>.upload.<capture>.<attempt>`), since only one process may use a background session.

- The Mac answers once it has built the bundle and queued it (`X-Scrawl-Wait: 45`), with the same
  `delivered`/`state` reply ScrawlKit reads.
- The token goes in `Authorization` and `X-Scrawl-Token`: iOS may drop or replace `Authorization`
  on background uploads, and the Mac takes either.
- An attempt owns its capture: sending claims it (`.sending`) under a file lock the app and both
  extensions take, so two senders never upload one capture, and only the owning attempt's outcome
  counts. An upload iOS still has is left to finish however long it takes; one it lost (the app
  was quit from the switcher mid-upload, say) is found when the app opens and sent again.
- Offline, a timeout, 408, 429 or 5xx: waiting, retried with backoff (15 s doubling to 30 min) when
  the app comes to the front, on a background refresh (scheduled as soon as one is left waiting),
  and after pairing again. A 500 (the Mac couldn't build it) is tried three times. 401/403 (pairing
  revoked), 413 and other 4xx stay failed, with Share. The Mac dedupes on `recordedAt` (to the
  millisecond), so sending again is safe.
- A broadcast is a waiting capture as soon as its movie and timeline are saved, before it's handed
  over: if iOS stops the extension in between, the app sends it.
- Captures the Mac has are kept 7 days with their movie and 30 without, as on the Mac.

## Tests

```sh
cd ios && xcodegen generate
xcodebuild test -project Scrawl.xcodeproj -scheme Scrawl -destination 'platform=iOS Simulator,name=iPhone 17 Pro'
```

Unit tests cover pairing links, upload outcomes and the framed payload, the timeline, the capture
store and retention, and both movie writers on synthetic ReplayKit buffers (scaled H.264, 48 kHz
AAC, no audio track with the microphone off, screenshots on time).

`scripts/e2e.sh` runs the whole way in the simulator without touching your own Scrawl or OpenCode:
a `scrawl serve` with its own home folder, captures and port, whose OpenCode is a stand-in
(`scripts/fake-opencode.py` lists sessions; a fake `opencode2` writes down the prompt instead of
posting it). The UI tests pair with it, pick a session, send a test capture, and share a screenshot
from Photos through the share extension; then the script checks both bundles and both prompts. The
UI tests skip themselves without it.
