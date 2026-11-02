# Keyframes, transcription and the text agents read

Read this before changing `capture/engine/keyframes.rs`, `capture/engine/runtime/audio/transcription/transcribe.rs`, `shared/hex.rs`, `capture/engine/runtime/audio/transcription/command_transcriber.rs`, `capture/engine/sketch.rs` (`summary.md`) or `capture/engine/render.rs` (frames and the contact sheet).

## Experiment `linux-playback-ducking` (`src/desktop/linux/playback/`)

Off by default. Manually configured mono/stereo playback lowering, one capture
at a time, on PipeWire 1.6.9 / WirePlumber 0.5.18. A self-expiring DSP lease
preserves user volume/mute edits and recovers without the app or policy. No
live effectiveness indicator or automatic installer. See [setup and limits](../../desktop/docs/linux/linux-playback.md).

## Images and text

- **Images (`frames/*.png` and `contact-sheet.png`)**: Keep images purely visual. Each keyframe shows the screenshot, active red tldraw stroke(s) for that moment, a subtle click mark (thin translucent ring and small dot, no fill, so the clicked thing stays readable; phone swipes are a thin translucent trail), and the keyframe's number alone (no `#`, no time; `summary.md` pairs each `[#N · M:SS]` with its words) in bold black on a small white rounded square with a thin dark ring, inset 5 px from the top-left corner and dodging to bottom-left when top-left has ink, a click or the cursor. The chip never moves over the top-right action controls in a captured app. The chip is drawn at one size on every contact-sheet tile, about 24 px with its ring: blind reading tests found 20 px failed when the sheet was shown at half size, a bare digit was read as screenshot text, and `#6` was read as `8`. `contact-sheet.png` packs frames in time order (left→right, top→bottom) into aspect-ratio-justified rows separated by tight 6 px gutters on a dark canvas (no card borders or rounded frame masks, capped at 1600 px long edge / ~1.56 MP) with no rasterized transcript headers or caption boxes.
- **Text (`summary.md` & Clipboard)**: Put the transcript, `[#N · M:SS]` keyframe index, and absolute file paths (`Contact sheet:`, `frames/frame-XX.png`, `Video:`, `Bundle:`) in plain text (`NSPasteboardTypeString`) without repeating the transcript multiple times, so `⌘V` pastes cleanly into any terminal (Ghostty / OpenCode TUI) or GUI composer (OpenCode Desktop).

## Checked completion

Screen and phone completion keep construction and the optional final whiteboard append
under one Engine publication lease. Desktop admits Saved, clipboard handoff, Connect
and phone retention only after the outer result validates the persisted manifest,
summary and surviving media and completes the durability barrier. An append failure
may use the intact original bundle only after revalidation; an unfinished whiteboard
stage blocks admission. Failed publication retains recovery and pending-transcription
work. A later transcript update restores the previous manifest and summary on failure.
Existing incremental WAV files remain authoritative and are never replaced by the
in-memory recording at Stop.

This changes the shared completion contract without changing installed Desktop routing
or introducing another completion surface. The existing UI receives only the checked
outer result. Publication checks structure and sample extents for MP4/MOV; native
capture and playback verification remain separate requirements.

Clipboard handoffs put access instructions first and the duration header with timestamped words/marks last, so the end of a paste identifies the request. A retained Connect job supplies fetch/inspect commands without sender-local paths or repeated login instructions; otherwise a shell-quoted local inspect command and local image/crop guidance remain. Both include a one-line CLI installation link and preserve the video-proof opt-out. Notes, capture warnings and semantic timeline entries remain in the prompt. `summary.md` and `apparent inspect` retain the full archived format. The shared Rust formatter serves existing copy/paste callers; no UI routing or capture consent changes. See [Connect client](../../../connect/docs/hosted-client.md).

## Transcription (`capture/engine/runtime/audio/transcription/transcribe.rs`, `shared/hex.rs`, `capture/engine/runtime/audio/transcription/command_transcriber.rs`)

Apple SpeechAnalyzer by default (measured word times). `"transcriber": "hex"` + `"hexModel"` in `~/.config/apparent/config.json` (Settings › Recording, `apparent hex status`, `--transcriber` on `reprocess`/`import`) uses Hex's local API v2 (discovery file `local-api.json`, bearer token, never logged) only when Hex answers `/health` and has a ready model (`installed && verified`; Apparent never downloads). Hex gives text only, so each speech island (`audio::speech_islands`, clips merged to ≤ 24) is transcribed on its own and its words are spread over it by length: `source: "hex:<model>"`, `estimated: true`, which `keyframes.rs` reads to skip spoken cues. Any Hex failure falls back to Apple for that recording; Hex absent falls back silently (logged once). `"transcriber": "command"` (`capture/engine/runtime/audio/transcription/command_transcriber.rs`) runs the argv in `"transcribeCommand"` (no shell, own process group, wall-clock limit, 8 MiB stdout cap) and reads one versioned JSON transcript: `source: "command:<name>"`; any failure means Apple for that recording (Hex never tried; a missing or malformed command is logged once, like Hex being away), and nothing the program says or is given reaches the log. It gets no language from Apparent: `appleLocale` is Apple's, and the user's argv carries the model's. Its keys are read on their own (`Program::load_from`), never through `handoff::Config`. Settings reads `hex::cached_status` (a background poller), never the network.

Hex requests use `"hexLanguage"` (Settings › Recording › Hex language), default `en`.
The field accepts a language code or `auto`, trims/lowercases it, and saves on Return/Apply;
blank resets to `en`. Apparent validates URL-safe code syntax only, leaving supported languages
to Hex. The selection is read once per recording and reused for all speech islands/retries.
It does not inherit Hex's dictation settings. An unsupported code follows the existing
Hex-failure fallback. Models may ignore language hints (e.g. Parakeet v3).

Apple's model (`capture/engine/runtime/audio/transcription/speech_model.rs`, `apparent-helper speech-model [--install]`) is SpeechTranscriber where the Mac supports it, else DictationTranscriber (VMs and Macs where `SpeechTranscriber.isAvailable` is false). Its language is `"appleLocale"` (Settings › Recording › Language, e.g. `"zh-TW"`; absent or `auto` is Automatic: the user's locale, the nearest supported one, or en-US). Rust passes it as `--locale ID` to the helper's `transcribe`, `transcribe-stream` and `speech-model`, which match it by `Locale.language.maximalIdentifier` (`zh_TW`, `zh-TW` and `zh-Hant-TW` match, `zh-Hans-TW` doesn't); an unsupported one gets the Automatic pick. `speech-model` lines carry `locales`, the supported ids, for the Settings menu, which marks a saved language this Mac lacks "(not available)" and names the one used. `appleLocale` is Apple's alone: Hex has `hexLanguage`.

Each app reserves the model for itself through `AssetInventory` (attributed to `dev.kitlangton.apparent.helper`), one slot per language (at the limit the helper releases languages Apparent no longer uses), and a fresh Mac doesn't have it, so the app checks and installs it in the background at launch (not when the transcriber is Hex) and retries a failed install. Changing the language (`speech_model::restart`) bumps the worker's generation: the superseded helper is killed, its late lines are ignored, and the new language is checked. Onboarding's Microphone row, Try it and Settings › Recording show the download. `transcribe` never downloads for long: without a model it returns `missing` (`downloading`, `unsupported`, `failed`), the summary gets a `No transcript: …` line (`TranscriptData::missing`), and a retryable bundle gets `.transcript-pending` and is transcribed again in place, `apparent.json` and `summary.md` rewritten, when the model arrives (`apparent retranscribe <bundle>` by hand). On macOS 15 there's no SpeechAnalyzer or `AssetInventory`: the model is SFSpeechRecognizer's on-device Dictation model (`Recognizer` in `helper/Media.swift`), SFSpeechRecognizer needs the Speech Recognition permission, which onboarding asks for as an optional fourth row (`Permission::SpeechRecognition`, shown only when `apparent-helper permissions` reports it, i.e. before macOS 26; also in Settings › Permissions). `speech-model` never prompts: without the permission its status is `permission` (`State::NeedsPermission`, no retries; a fresh grant seen by `read_grants` starts the check again), and otherwise it checks the model by recognizing half a second of silence (`supportsOnDeviceRecognition` can be true before Dictation's model is downloaded), a failed check being `failed` with an `error` that says to turn on Dictation. Recognition is on device only. `transcribe` never prompts either; without the permission it returns `missing: permission`, without the model `missing: dictation`, and both bundles get `.transcript-pending`.

## Keyframe selection (`capture/engine/keyframes.rs`)

Timed CLI screen captures deliberately have no click/Shift ink collector. Their manifest sets `interactionTelemetry: "videoOnly"` and their summary warns that a click visible in the movie is not a recorded event or click keyframe. Reprocessing and journal-backed recovery preserve the marker; old manifests and orphaned movies without it have unknown telemetry provenance. Use the app shortcut for click and ink context (subject to OS permissions). See `capture/engine/docs/capture.md`.

- Only cut keyframes on real visual or input events (`Mark` strokes drawn while holding `⇧`, flags, `Click`/`Drag` inside the capture target or phone taps/swipes, `Laser` points, or settled `SceneChange` transitions)—never cut extra keyframes on speech pauses alone over a static screen.
- The one speech exception is a spoken cue with measured word timings (SpeechAnalyzer; never a Hex transcript, whose times are `estimated`): "now", "here", "look", "watch" snap to a transition that starts within −0.3 s…+1.2 s of the word, else take the frame 150 ms after it. A cue keyframe is dropped unless its image differs (scene diff ≥ 0.03) from every keyframe already chosen, so speech over an unchanged screen still yields one frame. Demonstratives and "see"/"there" are weak (they rank with ordinary scene changes) because they are usually filler.
- Budget (`keyframe_budget`): 6 up to 40 s, then one more per 20 s, at most 12 (each image costs a reading model ~1–1.5k tokens). Ordinary events fill only the first 6; the rest are for marks and coverage.
- Marks are never dropped. Marks drawn on the same screen share one keyframe at the last of them, which draws all their ink (`mark.frame` names it; `markId` is the first): the screen counts as the same while the luma grid around their ink, padded by 8%, changed in under 5% of its cells. A group closes at a click or drag, past 20 s, or past 12 strokes. Groups beyond the budget merge with a neighbour: same-screen pairs with the least ink first, else the most alike screens. Flags stay on their own. No other keyframe cuts into a multi-mark group's span. Auto-crop gives a group's ink to its keyframe's crop alone (`Input::pin_ink`).
- Coverage: after marks, a stretch longer than 100 words or 35 s gets a frame from inside it when one qualifies (a click, laser, settled scene change or novel cue); the lead-in before the first keyframe is its own stretch. Speech over an unchanged screen still yields one frame.
- Text per keyframe: each keyframe owns the words from the start of the clause in progress at its anchor (action start, transition start, or cue word) until the next keyframe's start. Clauses break at clause punctuation, recognizer segment starts, or a ≥1.5 s pause. Words before the first keyframe belong to it.
- Mark shapes (`sketch::marks_of`, measured in pixels): a stroke is a circle when it ends near where it began, is longer than twice its box's long side and fills a third of its box, whose short side is over 0.42 of its long side; flatter loops, to 5:1 (a wide cell or a line of text gets a flat loop), also count when they close well within their short side and go round at most about three times, so a flat C or a scribble over text doesn't. A loop like that which is most of a stroke with a lead-in, tail or overshoot is a circle too. Either way the mark's `box_norm` is the loop alone, without the lead-in or tail (a stroke gone round more than once keeps its whole box). In strokes longer than 512 points a loop's ends are tried at every nth point only, but each stretch is still judged by the whole recorded path's length and area. A straight stroke is a line or underline. An arrow is a line with one or two short strokes near one end (the head), or one stroke whose straight shaft turns back at the tip into one or two straight barbs and whose head reaches both sides of the shaft (two barbs, or one that runs on across it); `from` is the shaft's start and `to` the tip. An underline whose end hooks back to one side, ticks, V's, zigzags, scribbles and C-shaped arcs stay plain marks.

## Experiment `agentContext` (`src/agent_proof/context.rs`)

Off by default (see Experiments in `AGENTS.md`). With it on, `sketch::summary_markdown` hands each keyframe to `agent_context::keyframe_texts`, which recovers the keyframe's timed words from its `said` and breaks them up by what happened while they were said: marks, clicks, drags, scrolls, shortcuts, and repeats of one item within 3 s folded into `×N` (`pressed ⌘T ×3`, `clicked ×4`), ordinary windows opening and closing (layer 0, by app name; titles never leave `scrawl.json`), app switches by app name, historical app events and context changes. The text never gives raw coordinates (the images show where): a place is written only when something names it. A mark is its shape, then what it was drawn on when `Mark.on` names it, then its stroke count when more than one (`✎② 0:19 circled button “Window” (2 strokes)`, `✎ 0:05 marked`); a click is `clicked`, plus its role and label when known (`clicked button “Area”`); a point is shown only with a label. An unnamed drag shorter than 0.15 of the screen is left out; a longer one is its direction (`dragged right`), and a named one names its target, `scrubbed` for a slider, scroll bar, scrubber, timeline or progress bar (`scrubbed slider “Playback”`). Drags of one thing less than 3 s apart are one line (`dragged back and forth ×2`), and a drag the dev page already reported as a video scrub is left to that report. Speech before the first item stays on the bullet line; each mark, or run of events with no speech between them, starts an indented line followed by the words said until the next one. An item's place in the speech moves back to the start of its clause if that began within 3 s, else forward past the clause's last word if it ends within 1.2 s, so a clause isn't cut in half. Events belong to the keyframe whose stretch they fall in (a stretch starts at the keyframe's first word, first mark, own moment or, for a click keyframe, its click up to 1.5 s before); marks always belong to their own keyframe. Without word times (a segment-only transcript) the speech stays whole and the items follow it.

When a keyframe draws two or more marks they are numbered ①②③ in drawing order (`sketch::marks_on`) in the text and `apparent.json`, never on the image: frames, contact sheets and exports show only the red ink, so nothing covers the screen. Single-mark keyframes stay unnumbered. `agent_context::mark_entries` / `find_mark` look marks up by those numbers: `apparent inspect --marks [--json]` lists each mark's keyframe, number, time, shape, what it was drawn on and id (`#2 ✎② 0:09 marked (2 strokes) · m2`), and `apparent zoom --mark 2 [--frame 2]` (or `--mark m2`) zooms on that mark, centred on its time span (at least `--span`, the whole mark plus half a second either side), cropped like its keyframe, with its whole ink drawn from the moment it starts. A number on several keyframes needs `--frame`; numbers shown as ①② win over single marks.

A scene-change keyframe says `[changed: the whole screen]` when its `autocrop.changes` boxes, merged when they touch, include one covering 90%. Smaller changed regions have nothing to name them yet, so the text leaves them to the image. A keyframe with no speech or items falls back to its own description without positions (`scene changed`).

With `agentContext` on when a Mac capture starts (not the iPhone mirror, and only with Accessibility granted), `crate::ax` looks up what each click and ⇧ stroke lands on in the Accessibility tree: the element at the mouse-down point, and for a stroke the middle of its box (just above a flat stroke, since an underline sits under what it marks). A click on a button's text or icon is named by the button (up to two parents). It reads the role description, a label (title, description, value, placeholder), `AXIdentifier` or `AXDOMIdentifier`, and the file name of the window's `AXDocument`; never a window title, never the value of a secure or other input field, and every string cut to one line of 60 characters. Lookups run one at a time on the `apparent-ax` thread with a 0.1 s messaging timeout and a 250 ms budget each; lookups that hit Apparent's own windows retry on the app whose window is under the point. `finish_session` waits up to 400 ms for running lookups and puts the results on `Click.role` / `Click.label` (`“New Tab” #tab-new in main.rs`), `Drag.label` and `Stroke.on` (`button “Area”`, which `Mark.on` and the mark's description pick up). Results of an earlier capture are dropped (`Prober::begin`).

Linux does not attribute clicks or marks to OS accessibility elements. AT-SPI cannot reliably bind an accessible top-level to the X11 window where an action occurred, so a matching PID, title, bounds and active state could still name a different window after a replacement. With `agentContext` on, an unnamed Linux click says `clicked (element unavailable)`; marks have no OS-derived `on` label. Context supplied by a registered dev page remains separate and can name its own elements. Native Wayland elements are unavailable too.

Dev pages that load `/sdk.js` add what only the page knows (`capture/desktop/workflow/page.rs`, `capture/desktop/sdk.js`). With `agentContext` on, `/web/register` answers `"context": true` to loopback pages, and the script then hooks in once: the element under each `pointerdown` and under ⇧ mouse moves (every 250 ms) as tag, id, two classes, a label (`aria-label`, `title`, `alt`, `data-testid`, else up to 60 characters of text; for form fields only the placeholder or name, nothing for password fields, never a typed value), and the component and source `file:line` from React's fiber (`_debugSource`, or the first project frame of `_debugStack`) or a locator attribute (`data-source-loc`, `data-inspector-relative-path`/`-line`, `data-sentry-component`/`-source-file`); `<video>`/`<audio>` seeks, plays and pauses (file name, time, duration); `console.error`, uncaught errors and unhandled rejections (message and first project frame); and fetch/XHR requests that fail or answer 400+ (method, path without query, status, duration; no bodies, headers or cookies). It batches them to `POST /web/context`, which takes loopback pages from this Mac only, token or not, and drops them when the experiment is off. The app keeps 3 minutes per tab (300 events, 16 tabs) and cuts every string again. When a capture finishes (`finish_session`, not the iPhone mirror), `page::apply` takes the window's events from tabs that were clicked, marked or focused then: a page click within 400 ms of a Mac click or drag replaces its label (`<button.tab-new> “New tab” in BrowserHeader (src/BrowserHeader.tsx:88)`), the most common element under a stroke's ⇧ moves becomes its `on`, and errors, failed requests and video playback become app events (`console error: …`, `GET /api/tabs → 404`, `video clips.mp4 scrubbed to 0:04 / 0:11`, a scrub's seeks folded into its last). The agentContext summary shows a Mac capture's app events without the `app:` prefix.

## Missing transcript warning (`capture/desktop/mod.rs`)

On by default. After a local capture, the Completed capsule keeps its Copy/Send status and uses an amber warning icon for three distinct outcomes: no microphone samples (including a zero-sample WAV header or a missing file: “No audio”), saved samples with a successful empty transcript (“No words”), or saved samples whose transcription failed or did not run (“Transcript unavailable”). Review and the handoff text explain the corresponding outcome; a failed or unavailable speech model retains its reason rather than posing as silence. Neither the microphone meter nor the audio level proves intelligible speech: silent samples still count as recorded audio. iPhone bundles do not use this local-microphone warning. The separate live warning during recording remains off by default.

## Experiment `liveWordWarning` (`capture/engine/runtime/audio/transcription/live_word_probe.rs`, `capture/engine/runtime/audio/transcription/live_word_warning.rs`)

Off by default. During a microphone recording, a separate best-effort recognizer checks completed six-second chunks of captured 16 kHz audio, including continuous noise without silence. Only after two active chunks have completed successfully without recognized words does the recording capsule show the inline amber “No words yet” warning. Words clear it; after words arrive it does not claim “yet” again. A missing model or recognition error is not an empty result and cannot trigger that warning. The energy threshold qualifies audio for a check; it is not proof of speech or microphone health. The probe does not supply the final transcript; after Stop the saved audio and final transcription determine the Completed warning above. On Mac the probe runs the helper with a short deadline; on Linux and Windows it uses a separate Parakeet session. The live experiment does not change final transcription or delivered bundle content.

## Local export narration (`exportNarration`) (`capture/engine/export/narration.rs`)

Available by default. Explicit `apparent export B --narration cues.json` replaces recorded
audio in the exported movie with locally generated Kokoro `bm_fable` speech. It never
changes the bundle or its transcript. An explicit `experiments.exportNarration: false`
disables it; provisioning is separate and export does not download models.
See [export audio](export-and-retention.md#explicit-export-audio) for the versioned
timed script, integrity pins, limits, provenance and failure/recovery contract.

## Experiment `liveTranscribe` (`capture/engine/runtime/audio/transcription/live_transcribe.rs`, `capture/engine/native/LiveTranscribe.swift`)

Off by default. With Apple Speech selected, this uses one continuous SpeechAnalyzer,
with the preferred SpeechTranscriber or existing DictationTranscriber fallback and
installed assets only. It emits natural finalized results; Apple may defer all
results until EOF. There are no finite recognition chunks or forced pause boundaries.
Source samples and timestamps use the saved WAV clock; lexical and timing identity
with batch recognition is not promised.

The feeder reads the recorder's rolling history by absolute sample offset every
160 ms. Overwritten history, input overflow, malformed/over-budget helper output,
invalid ranges, failed completion or timeout discard the live result and retain
complete saved-WAV transcription as the fallback. Finish sends only the unwritten
WAV tail and closes input. A six-second helper I/O deadline, measured from Finish
entry, covers tail writes, output drain and helper exit. Feeder spawn/join and saved
WAV decoding are synchronous; this is not a total Finish latency guarantee. Success requires exact source sample count, Ready/results/Done order,
stdout EOF and successful exit. Cancellation uses nonblocking writes and helper
process-group cleanup. The native audio queue holds at most 32 buffers; retained
helper output is capped at 8 MiB, 64 KiB per line and 16,384 events. Exceeding a
budget fails rather than dropping audio. Linux, Hex, iPhone and imported recordings
keep their existing paths. `apparent reprocess B --out O --bench 1 --live-speed 1`
replays audio and logs the streamed sample count and tail latency. Helper proof does
not establish preferred-engine availability or installed-app capture acceptance.

On macOS with Swift 6.2+, `python3 capture/engine/native/tests/live-transcribe.py`
checks the actual production source/converter/completion seams using generated PCM
without speech models or audio devices. The macOS Rust suite invokes it too. These
regressions do not replace actual selected-module and installed-app capture proof.


## Linux pause transcription (`capture/engine/runtime/audio/transcription/pause_transcribe.rs`, `capture/engine/runtime/audio/transcription/speech.rs`)

On by default on Linux with the local Parakeet transcriber, without an experiment
flag. The separate Mac `liveTranscribe` experiment is unchanged. The worker reads only
new microphone samples, uses `audio::speech_islands` and Hex's short-blip filter,
and waits for at least three seconds of audio plus 700 ms of quiet before sending
a chunk to its reusable model session. Each early chunk starts 200 ms before its
first speech island and ends 200 ms after the last, instead of feeding a long
silent lead-in to Parakeet. Its pending buffer is capped at 60 seconds; if it
falls behind or finds no safe cut, it falls back to whole-WAV transcription
rather than dropping audio. At stop it reads the finished WAV to transcribe only
the unprocessed tail, joins ordered text and absolute word
times, and falls back to the normal whole-WAV path on any failure. Hex and command
transcribers are unchanged. To compare without a live microphone, use
`apparent reprocess B --out O --bench 1 --live-speed 1`
on a Linux machine with Parakeet installed; `--live-speed` replays the saved WAV
at real speed before timing finalization. Compare with a normal reprocess of the
same bundle: chunk boundaries can change recognition even when finish is faster.

## Warm recognizer (`capture/engine/runtime/audio/transcription/warm_recognizer.rs`)

On by default for Linux local Parakeet pause transcription. Set the top-level
`"warmRecognizer": false` in `~/.config/apparent/config.json` to disable it;
remove that key or set it to `true` to restore reuse. The old
`experiments.warmRecognizer` flag is no longer read.
The next recording uses one cached model/session, loaded lazily on its first
recognition, not at app launch or capture start. It can be reused by subsequent
recordings. One owner thread serializes recognition and owns native destruction;
no new unsafe thread-safety declarations are needed. Each native batch run
replaces its result, and each capture starts with fresh transcript accumulation.

The fixed 30-minute idle timeout starts only when the last capture's pause worker
has actually ended, after recognition and tail processing. Starting another
recording cancels expiry for that recording's lifetime. Cancel does not release
the lease while a detached worker is still recognizing. Idle expiry releases
both session and model; it never runs during capture or processing.

The first recognition pins the selected model for that capture, including its
selected filename for transcript metadata. Selection changes apply to the next
capture's first recognition. A changed file identity invalidates the old cached
model between captures; if an old lease needs to reload a replaced file, it
fails into the existing whole-WAV fallback instead of mixing models. Recognition
errors discard the session. Setting `warmRecognizer` to `false`, or selecting Hex/command,
clears the cache on the next recording start (after any already-active worker
ends). Config edits do not interrupt a capture already in progress. Hex, command,
whole-WAV fallback, Mac speech, saved bundle formats and manual Paste are unchanged.

Local logs report load time, reuse, recognition time and idle expiry without
transcript, audio or model paths. To measure cold versus warm, replay at least two
recordings **in the same process** with `--live-speed`; separate `apparent reprocess`
processes cannot share an in-memory cache. For a saved Screen bundle, use
`apparent reprocess B --out O --bench 2 --live-speed 1`
and compare with the same command using an isolated `APPARENT_CONFIG` file with
`"warmRecognizer": false`. Use the same model and WAV. Retaining the native
recognizer uses additional idle RAM (about 1 GiB in a two-capture Linux VM test;
actual use depends on the model and machine). Deterministic lifecycle tests are
not proof of real recognizer speed, transcript equivalence or GUI capture behavior.

## Finish timing trace (`main.rs`)

Every successfully built bundle has `finish-trace.json` beside `apparent.json`, with `stopMs`, `audioMs`, `candidatesMs`, `transcribeMs`, `selectMs`, `decodeMs`, `autocropMs`, `sheetMs`, `writeMs`, `totalMs`, and `preparedVisuals`. Missing stages are null. The trace holds durations and a boolean only, not transcript or screen content; it is not copied into `summary.md` or clipboard. Transcription and visual preparation can overlap, so stage durations do not add up to `totalMs`. Failed builds do not get a trace; use the local `apparent.log` for errors. The existing `finish timings:` log line includes the bundle ID. Bundle retention also removes its trace when it removes the bundle.

## Click-based visual handoff (`shared/handoff/mod.rs`, `capture/engine/sketch.rs`)

The old automatic OpenCode attachment policy is inactive. The full capture, keyframes, contact sheet, and video are still built and saved. Manual Paste remains image-first; `apparent zoom` is available for closer inspection.
`apparent copy` is text-only by default, and its confirmation points to the contact sheet. Copying or dispatching Paste does not prove a receiving composer accepted the image. Check the unsent draft; if the image is absent, attach the saved `contact-sheet.png` manually. An agent that cannot open an unattached local path must ask for the image instead of guessing a click target from coordinates or “element unavailable.”

## Passive auto-crop safety (`capture/engine/autocrop/autocrop.rs`)

A crop inferred only from cursor dwell or screen changes must retain the whole frontmost regular window recorded at that keyframe's time. A resting pointer cannot justify a tight focus crop inside a large window. If the final crop does not retain that window, or no usable window metadata is available, Apparent keeps the whole frame. This check runs after grouping and content-edge trimming. Whole-capture scope keeps one shared full-frame fallback if any keyframe needs it. Deliberate input (ink, clicks, drags, laser or scroll) can still select another window. Window snapshots are never applied before their timestamps, and an app name appears in a crop label only when one of its recorded windows intersects the delivered crop. This is a correction to existing auto-crop behavior, not a new experiment; it does not change capture mode, window metadata collection or full-frame preservation.

## Experiment `onboardingFirstCapture`

Off by default. The welcome tour requires the user to choose a Record shortcut,
then complete a local recording started with it. Dismissal leaves the tour
unfinished. Enable it before opening onboarding; see
[onboarding and settings](../../desktop/docs/onboarding-and-settings.md#first-capture-experiment).
It changes neither capture contents nor outbound delivery.

## Experiment `hostedSharing` (`connect/hosted.rs`, `capture/desktop/mod.rs`, `capture/desktop/settings/settings.rs`)

Off by default on macOS and Linux. Adds **Copy link** beside local **Paste** in
the completed capsule and **Hosted account** controls in Settings → General.
Set `"experiments": {"hostedSharing": true}` in isolated test config; edits apply
on the next UI/operation check. The configured `cloud.server`, `hostedSync` and
explicit upload consent are separate requirements. The backend additionally
requires deployment `SHARING_ENABLED='true'`, off by default.

Copy link remembers only the triggering completed bundle through account consent.
Closing Settings or dismissing/replacing that completed capture cancels its
foreground request. Account login and ordinary upload do not create public links
for the backlog. Offline disables Copy link with “Connect to the internet to
share”; it never schedules a later clipboard change. Reservation returns before
media upload, and the existing worker continues under the same identity after
the capsule closes. Progress uses “Estimating…” or “Waiting for connection”, not
a countdown without measured throughput. Interrupted/failed senders best-effort
report paused/failed state to the server outside queue/config locks.

The copied handoff explains that anyone with the link can view and download this
capture. Shared links can be stopped without deleting the private recording;
downloads and third-party preview caches cannot be recalled. CLI
`apparent connect shares list` follows encoded `nextCursor` pages to exhaustion;
`apparent connect shares revoke <management-id>` reads back the complete list.

**Candidate, not native acceptance:** first-time API configuration still uses the
CLI (`connect login --server <API origin> --scope read`). Actual Linux GUI and Mac
native pill/Swift Settings proof, onboarding cancellation and foreground
clipboard tests, and the real-feature walkthrough remain release gates. See
[hosted client](../../../connect/docs/hosted-client.md) for limits and isolated synthetic test commands.

## Native URL launch (`nativeLaunch`)

The default-on `nativeLaunch` setting opens the app by the exact
`apparent://open` URL, never a capture or handoff. Its OS registration and validation
contract are in [native launch](../../desktop/docs/native-launch.md).

## Text-only GitHub feedback

The default-available `apparent issue` command is documented in
[text-only GitHub feedback](../../desktop/docs/issue-reporting.md). It provides an
explicit CLI issue preview/submission path, never changes capture output, and
never automatically attaches or submits a bundle.

## Experiment `agentDemoVideo` (`shared/handoff/mod.rs`, `capture/desktop/settings/settings.rs`)

On by default; set `"experiments": {"agentDemoVideo": false}` or switch it off in Settings › Agents to disable it. With it on, copies and manual or guarded draft pastes ask the agent to use the repository's `make-it-apparent` skill for a short video of a complex visible change, inspect the finished recording, and embed or link a playable file where the viewer can access it. A local path alone only works on a shared filesystem. The agent names the exact build/OS and what the video does not verify; if the skill, recording surface or delivery is unavailable, it reports the blocker rather than calling the walkthrough done. This does not change the saved `summary.md` or the capture; automatic OpenCode delivery has been removed. A config edit applies on the next copy/paste without a restart.

## Experiment `copyCaptureId` (`capture/desktop/mod.rs`)

On by default. To hide it, set `"experiments": {"copyCaptureId": false}` in
`~/.config/apparent/config.json`, preserving other keys. Reopen the existing
capture window after editing. **Copy capture ID** (**Copy ID** in compact windows)
copies only the displayed bundle directory's exact name, including `.apparent`,
as plain text. Browsing older/newer captures changes the copied ID. This is the
local folder name, not a Connect identifier or share link. It does not copy a
path, summary or image, upload, paste, or mark the capture handled. Clipboard
failures are shown in the window. An explicit opt-out is preserved; unrelated
experiments remain opt-in.

## Experiment `clipboardImage` (`shared/handoff/mod.rs`, `capture/engine/runtime/platform/legacy_windows/platform/clipboard.rs`, `capture/desktop/linux/clipboard.rs`)

Off by default. A finished capture (and `apparent copy`) normally copies the summary text alone. With this experiment enabled, `copy_bundle_to_clipboard` offers `contact-sheet.png` and the summary as formats of one clipboard selection: PNG and string on macOS, and `image/png` plus UTF-8 plain text on KDE/Hyprland Wayland. X11 and other Linux sessions retain text-only copy. A missing contact sheet or unsupported image offer falls back to text. A failed offer after publication returns an error instead of overwriting a newer clipboard owner. The Paste chip and paste shortcut remain the explicit one-action, image-first **two-paste** handoff; this experiment does not enable automatic draft paste. Retained Windows code offers `PNG`/`CF_DIBV5` and Unicode text, but Windows is unsupported.

**Offering both is not proof of attachment.** The receiver decides what a single Ctrl/⌘V means. The Windows Claude desktop trial accepted image and text together; a macOS Safari contenteditable draft accepted the PNG but omitted the text on one ⌘V, despite both clipboard types being present. Inspect the unsent draft. If the image is missing, attach the saved `contact-sheet.png` manually and paste the summary; if the text is missing, paste the summary from the bundle. No clipboard API can confirm that a chat composer attached the image or that its agent can read it. Do not enable the experiment by default on the strength of offered formats alone.

## Experiment `draftHandoff` (`shared/handoff/mod.rs`, `capture/desktop/linux/clipboard.rs`)

Off by default; Linux Hyprland Wayland only. The guarded `handoff::run_draft_handoff` / `DraftHandoffOps` contract is available to in-crate receivers: capture an exact receiver+editor target, preflight support, offer and paste the PNG, poll for a **visible attachment** without repeating the paste, then insert text only if focus and offered clipboard data still match. Confirm visible text, and restore the previous **plain-text-only** clipboard only when the offered MIME and bytes still match. This is a best-effort contention check, not atomic ownership: Wayland can replace an identical offer or change owner between the check and restore. Refused paste, missing attachment, focus loss, unsupported receiver, and clipboard contention return errors; the user must upload the sheet and paste the summary manually. The shipped Linux receiver has **no verified attachment observer**, so this experiment currently refuses *before* touching the clipboard or draft. An image offered to the clipboard, visible attachment, visible text, and receiving-agent access are separate claims; only a consented synthetic send can test the last one. Do not enable this experiment until the intended composer observer and unsent live draft are verified.

## Experiment `autoPasteDraft` (`capture/desktop/linux/auto_paste.rs`, `capture/desktop/mod.rs`)

Off by default; currently **fails closed on Linux**. When enabled, a finished capture saves the bundle and copies the text summary, but does not dispatch image or text paste keystrokes. The paper says “Saved · paste manually” and points to `contact-sheet.png`: attach it in the intended unsent composer, then use Ctrl+V for the summary. Apparent never sends the draft. The prior window-only trial could paste into another T3 thread within the same window. A window address is not a stable draft/editor identity, and a paste keystroke is not an image attachment receipt. Automatic handoff must wait for a receiver that can verify the capture-start and finish target, writable composer, accepted attachment, and visible text without duplicate insertions. The explicit user-directed `apparent paste` command remains available, but it cannot prove receiver acceptance either. Ordinary Ctrl+V and Omarchy Super+V are unchanged.

To compare a summary experiment on real captures, `capture/agent/scripts/agent-eval.sh [--experiment NAME] [--out DIR] [bundle…]` reprocesses each bundle with and without it (Kit's own `experiments` removed from the baseline's config) and writes `before.md`, `after.md`, both contact sheets and frames per bundle, and an `index.md`, by default to `~/code/home/artifacts/apparent-context/`. Both summaries come from the baseline run's transcript. What a capture recorded live (Accessibility lookups, page events) is kept as recorded, so those only show on captures made with the experiment on.

## Typed capture notes

During screen capture, Shift-click opens a compact Electron text field at the
point; Shift drawing is unchanged. Enter or Attach saves; empty Enter keeps
only the pin. Escape preserves the in-capture draft. The count reflects durable
saved notes; its compact list edits/deletes them. Unsubmitted drafts are not
handoff content and do not survive ending/restarting the capture.

Rust stores submitted text plus the pin's original timestamp and normalized
position in `notes.json`. Finalization/recovery includes it as typed `note`
events in `apparent.json` and an explicit Typed note section in `summary.md`,
with the pin's keyframe link where available. Reprocessing replaces these
events from the local notes file rather than duplicating them. The note's
clock/anchor never moves to the time typing or editing finished. Text stays
semantic; popup controls are not rasterized captions.

`microphoneEnabled: false` allows intentional silent captures without a
microphone grant; set it from the menu or Electron Recording control. Its
value is sampled at admission, and file edits apply at the next capture. The
selected microphone and all unrelated config keys remain intact. The saved
`context.microphoneCapture=off` distinguishes this choice from an audio error.
