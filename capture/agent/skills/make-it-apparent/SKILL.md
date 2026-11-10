---
name: make-it-apparent
description: Use when told "make it apparent" or asked for video proof.
---

# Make It Apparent: make the situation understandable

When Simon says "make it apparent," produce a short video that makes the current topic understandable. A finished implementation is not a prerequisite. Choose the mode from the viewer's question and available evidence:

- **Change demo — What works now?** Record the exact working build, meaningful action and visible result. Verify claims that pixels alone cannot prove.
- **Problem demonstration — What goes wrong?** Record a safely reproducible current failure or limitation. Unresolved behavior is a valid subject. Distinguish an observed failure from a suspected cause; do not invent a fix or successful ending.
- **Technical explainer — Why does this happen?** Use source-checked diagrams, illustrations, code excerpts or inspected evidence with timed explanation. Label conceptual examples, hypotheses and proposed changes inside the video. Created explanatory visuals are valid primary material, not failed proof.

For a generic request about a user workflow, use the real-interaction or interactive-prototype route below, even when implementation is unfinished. For a question about an invisible technical mechanism, a source-backed conceptual explainer can be the primary video. State the selected mode and limits briefly, then act. Explicit live-app, bug-reproduction, Apparent-capture and exact-build requests require that evidence or a precise blocker; a conceptual supplement cannot satisfy them. Never present a hypothetical sequence as an observed reproduction.

The capture instructions below apply to scenes claiming live behavior, not as prerequisites for conceptual explanation. Keep raw evidence unchanged and private, inspect the entire final video and never commit generated media. A recording is not a substitute for tests. A screen-share of the coding session is not a demonstration of a finished result.

## Show the workflow from the user's seat

For user-facing workflows, default to a narrated recording of the actual interaction, including when the request is only “make it apparent.” Show the starting screen, the action and its visible result in the environment the user would use. Use an isolated VM for desktop/OS workflows under the host's existing safety rules; check for active workers before staging or launching. Do not use a VM as a reason to bypass platform, consent or spending gates.

If the workflow is proposed rather than implemented, record a small interactive prototype with recognizable controls and visible state changes. Keep it within the approved design scope, outside shared implementation files, and label simulated behavior in the video. Do not build production integration merely to make a design walkthrough. If neither a real workflow nor an interactive prototype can be recorded, report the precise blocker; a deck describing the steps does not fulfill a workflow walkthrough.

For example, to explain recording-to-link handoff, show recording a harmless task, finishing, pasting into a private scratch destination, and opening the resulting recording as a recipient would. Show “Preparing recording” separately from playable readiness when applicable. Use isolated synthetic data and approved destinations; never post to a real agent session or publish a recording for a demo. In a prototype, label clipboard, upload, recipient playback and revocation simulations explicitly. Text such as “Finish → Paste with public link” is a storyboard instruction, not evidence that the viewer has seen that flow.

Narrate the action as it happens: what you do, what changes, and why it matters. Choose the approved narrator before rendering; prefer provisioned local narration and never send private script text to an online service without authorization. Keep subtitles synchronized, at most two short lines, clear of controls and results. Let the UI carry the visual story; omit explanatory paragraphs, fake text buttons, decorative watermarks and title-card sequences that compete with it. Add only a brief label, measured highlight or close-up when needed.

Use diagrams for a mechanism that cannot be seen through interaction, or when explanatory visuals are explicitly requested. An unfinished implementation or unavailable recorder alone does not turn a user workflow into a technical-explainer request. Keep any conceptual supplement clearly separate from the unfulfilled live-proof claim.

Before delivery, inspect the whole final movie at the intended player size. Check that each narrated action has a corresponding visible action and result, the flow is understandable without reading a deck, and proposals remain distinguishable from verified behavior. Reject and redo a take that merely reads steps over static cards. Codec/decode checks alone cannot pass this review. Skill wording changes need fresh agent trials before claiming improved recording behavior.

## Explain the cause, not the tool log

Before drafting, ask: **What does this viewer already know, and where must I pick them up?** Use the conversation and available context. Restore the immediate project context one local step before the unfamiliar mechanism; do not reteach familiar technical basics. If the evidence is thin, add one relevant bridge instead of asking the viewer to choose a level.

Open with one sentence stating the question, actual status and any required human action. Use **background → problem → mechanism/change → now → open → action**. These are story beats, not six mandatory slides. Give each beat's whole point first, then explain only the terms and causal links needed to follow it. Define unfamiliar things by a familiar class and relevant distinguishing feature. Use short active sentences and consistent names.

Show one concrete example moving through the mechanism: parts doing something to each other, not a component inventory or tool log. For example, a hypothetical timeline can save an earlier volume, lower it temporarily, accept a person's new choice, then show a naive restore overwriting that choice. Label this a **hypothetical example**, not a current app defect. Check any source before applying it to a product; show proposed protections separately from verified behavior.

For unfinished work, **now** explains the actual remaining gap, not a fictional success. State risks and untested claims before the ending. Finish with the real next action and owner, or "No human action is needed." Work still assigned to the agent stays with the agent.

Every mode needs timed, plain-spoken explanation, readable burned captions and editable timed text so the story works muted. Add synchronized generated voice by default unless the user requests silence or suitable narration already exists; disclose generated narration. Never play it through speakers during capture. Use only authorized speech services: do not send private text online or incur charges without permission. If narration is unavailable, report that production blocker; do not silently replace it with silence. An explicitly silent scenario can use captions alone, disclosed in the video and handoff.

### Use Apparent's local narrator

Use Apparent's Kokoro narrator (`bm_fable`) for generated speech. Check the installed CLI and narrator before searching for generic TTS tools; finding FFmpeg's Flite filter is not a reason to substitute it.

- **Bundle export:** read [export narration](../../../engine/docs/export-and-retention.md#local-narration), check `apparent export --help`, and use `apparent export /absolute/private/path/walkthrough.apparent --narration /absolute/private/path/cues.json --out /absolute/private/path/walkthrough.mp4 --fps 30`. The JSON script uses exported-video milliseconds: `{"version":1,"cues":[{"startMs":500,"endMs":5500,"text":"Here is the first change."}]}`. Shorten text or extend the slot if speech does not fit. Narration is available by default but remains an explicit export choice; respect an existing `experiments.exportNarration: false` opt-out. Preserve the source bundle and unrelated config keys.
- **Generated explainer without a bundle:** use the same [standalone narrator](../../narrator/README.md) directly, then align the visuals and captions to its WAV duration. Run `~/.local/share/apparent/narrator/apparent-narrator ~/.local/share/apparent/narrator /absolute/private/path/cue.txt /absolute/private/path/fresh-output.wav` for each plain-text cue. The output path must be fresh; require successful synthesis and validate the WAV before mixing it into the movie. This route needs no bundle export or export configuration changes.

Both routes use the worker and pinned assets at `~/.local/share/apparent/narrator` on macOS and Linux. Run `node capture/agent/scripts/provision-narrator.ts --check` from a checkout with Node 22.18+ and Python 3.10+: readiness requires verified pinned assets and a real synthetic WAV, not executable existence. If assets are missing for an existing worker, run setup without `--check`. If the worker is absent, the current releases provide no prebuilt narrator: follow the explicit source-build prerequisites in the narrator docs and use `--build-from-source`; synthesis afterward is offline. Setup reports unavailable, downloading/building, ready or failed; interrupted partials never count as ready. Kokoro is TTS; Parakeet and Apple Speech transcription models do not fulfill this requirement. Respect existing network/build authority and verify provisioning on the actual OS/architecture. Report a concrete setup, gate or synthesis failure before proposing a substitute; do not silently downgrade to Flite/espeak or a cloud service.

Listen to the entire final narration in sync with the video through an approved private playback route; check intelligibility, pronunciation, pacing, clipping and caption alignment. Successful synthesis, decoding and frame inspection do not establish audio quality. If listening review is unavailable, report that verification gap and do not call the export fully reviewed.

### Produce a conceptual video for an invisible mechanism

Use an existing local renderer/editor and encoder; no guest or desktop recorder is needed. Do not install a renderer, add a product dependency or use a paid service just for the video without authorization. Resolve Apparent's current visual assets from `shared/assets/brand/`, site styles and supplied fonts. Use Paper `#F5F0E8`, Carbon `#020102`, restrained Signal `#7A58C7` and Lilac `#B7A5D8` if no newer palette exists. Prefer supplied Host Grotesk, otherwise a readable system sans. Keep labels large and beside their objects, use flat fills and clear arrow direction, and do not recolor real footage. Keep the established readable rounded dark subtitle style. Do not fabricate a logo or imply a conceptual diagram is an app screenshot.

Render a timed visual sequence, synthesize permitted narration when required, and encode a private playable MP4 at 30 fps with burned captions. Keep editable visual sources, narration, timed text and timing map outside git and public asset directories. A storyboard, script or HTML page alone is not delivery. Inspect every scene, transition and caption boundary at the intended player size and review the whole final export; verify timing, labels, privacy and playback. Check frame rate, duration and decoding with available media tools. No generated video can establish live app behavior. If rendering or delivery fails, report that exact production blocker rather than claiming there was no subject to explain.

## Point out the claim

1. Write one viewer question: “Can I now ___?”, “What goes wrong when ___?” or “Why does ___ cause ___?” Choose one checked scenario or labelled conceptual worked example. For a requested video, choose an explanatory visual sequence if live interaction adds nothing; do not silently downgrade to prose. For a verified bug fix, show the old failure if a safe capture exists, then the same task succeeding; never stage an old failure on the new build.
2. For live-proof scenes, identify the **exact build and environment**: commit or installed version, local/deployed URL where relevant, OS/device, and target window. Verify the app process or server actually serves that build. A local preview does not prove the released app, another platform, or the backend. Rehearse and confirm the result, not merely a successful click. For conceptual scenes, identify the source and separate observed facts, assumptions and proposed behavior instead.
3. Within a live-proof scene, capture starting state → meaningful action → visible result. Place it within the story above. Keep results readable and cut idle waiting, not evidence. Record the relevant assertion or test result separately if pixels alone cannot prove it.

Point at the claim, not every click. For a visual comparison, hold the same viewport and crop before and after. Use a sparse callout, outline or zoom only when the viewer might otherwise miss the changed region. Measure positions from the settled UI rather than guessing coordinates; recheck marks after scrolling or layout changes. Keep captions in empty space, and never cover the result they describe. Do not add arrows, measurements or captions merely to make the video look produced.

Make walkthrough videos at **30 fps by default**, including browser and generated review videos; use **60 fps only when the requester specifically asks for it**. A browser's live rendering rate is not the video's frame rate. Preserve real-time duration when encoding and verify the playable file's rate and duration before sharing it.

## Choose and prepare the capture surface

Prefer the installed **Apparent** on the same machine showing the result for native windows, screen motion, or iPhone. Follow the host repo's `AGENTS.md` and capture safety rules first. On Apparent's development Macs, read `capture/engine/docs/capture.md`; never wake, flash, or record Kit's real displays to make a demo. On another host, check its own permissions and capture capabilities; do not run a binary from a different machine and call it that host's proof.

Prepare a clean window before capture: hide notifications, unrelated windows and tabs, personal or customer records, tokens, credentials and auth prompts. Use synthetic **data** if necessary, but label a mocked or simulated **behavior** as such. Authenticate off camera. A crop or blur made after recording does not remove private material from the retained raw file; if a secret appears, discard that take and capture again. Keep the source bundle private, outside public/static asset paths and git.

For a browser-only change, an existing browser recorder may be more precise **if it is already available and the user has not asked specifically for Apparent footage**. Rehearse locators, then record the actual flow at a fixed viewport; wait for the semantic result, not just a timer or `networkidle`. Close the recording context to flush the video. Preserve the original and say “browser recording,” not “Apparent capture.” Do not install a recorder or add a project dependency just for a one-off proof without checking the project's workflow. A browser recording cannot establish native app or OS behavior; repo-local recording helpers from other projects are not portable commands.

## Record the proof with Apparent

For polished desktop demos, use `apparent capture start --request-id <id> --wait --timeout 30` through the already-running GUI's normal interaction-aware path: it collects the clicks, marks and cursor path that drive automatic zoom. Generate and privately retain a 32-hex-character request ID before sending. Retain the reply's top-level `capture_id`, including on failure or timeout, and require `status == "recording"` before acting: a real frame has arrived. Occupied sessions are refused, not toggled; pending portal consent is not recording. Check installed help and input permissions first; older builds lack these controls, and the CLI does not launch the app or bypass consent.

Finish agent-authored videos with `apparent capture stop <owned_id> --wait --timeout 60`, rather than hunting for Finish with OCR; require the visible button only when that button is the claim being tested. Success requires `status == "saved"` and that capture's exact `bundle_path`; a non-waited `stopping` reply is only acceptance. After timeout or disconnect, use `apparent capture inspect <id>` or retry the same start request ID or owned-ID Stop, not a shortcut toggle, `latest` or a directory scan. An admitted capture remains intact after a readiness timeout; explicitly cancel only your own ID if discarding it. Rejected request IDs remain rejected, and receipts last only for this GUI process, not an app restart. Follow [the agent video workflow](../../docs/agent-video-workflow.md) for bounded recovery and saved-bundle checks. Shared tests do not prove native Mac capture; verify the exact candidate on each claimed platform before release.

Timed `record --duration` captures do **not** collect that interaction telemetry; do not choose them silently and expect camera motion. Use a timed take only when interaction-aware recording is unavailable or a static camera is intentional, and disclose that limit. For that fallback, check `apparent record --help`, explicitly choose a clean screen and perform the action *while* recording:

```sh
apparent record --duration 15 --out /absolute/private/path/walkthrough.apparent
```

Window recording is temporarily disabled on macOS and Linux; `--window` fails before recording and never substitutes a screen. The timed command above records a display, so use it only when that broader scope is explicitly intended and the whole display is safe. Check the actual target in the bundle. For a supported cabled phone: `apparent record --iphone --duration 15 --out /absolute/private/path/walkthrough.apparent`. This command waits for the duration; it does not perform UI actions for you. Arrange the interaction separately; the GUI `capture` controls do not control a timed recorder. Start before the meaningful action and finish after the result settles. Use the returned exact bundle path, not `latest` when other captures could finish concurrently. If the CLI is unavailable, inspect the installed app path and permissions; don't pretend a transcript, synthetic `apparent demo` bundle, or a command that never recorded the action is a walkthrough.

## Inspect and render the live share copy

Read `summary.md` and `apparent.json`; open `contact-sheet.png` and the relevant `frames/*.png`. Play the **whole raw** `recording.mp4` or `recording.mov`, including first paint, transitions and ending. Check that it shows the correct build/window, all three beats, readable result, correct audio if narrated, no private frames, no error toast or misleading splice. The contact sheet indexes moments but cannot certify the intervening video. If an automatic crop hides context, inspect full frames/source before approving it. Verify the result independently with the relevant test, app state or log where appropriate; the recording proves only what it visibly shows.

For appearance changes, distinguish offscreen renders from a live window: render both schemes to inspect every state, then capture the exact running build opening while the OS is already dark and switching light→dark→light in the same process. Read back the OS appearance alongside screenshots; restore its original setting. A settled screenshot checks that frame, not every transition frame or a live permission grant.

If export works on this host, use `--fps 30` (or `--fps 60` on explicit request). Smooth activity zoom is on by default; keep it on for demos. Use `--no-zoom` only for an intentionally static share copy (for example, a motion-sensitive viewer or a fixed-frame comparison). Do not hand back the unprocessed original or a 60 fps original by accident:

```sh
apparent export /absolute/private/path/walkthrough.apparent --out /absolute/private/path/walkthrough.mp4 --fps 30
```

Do not invent a frame-rate flag for `apparent record`; set it during export or browser encoding without changing real-time duration. Check installed `export --help` before using the flags: older builds require `--zoom` to opt in, whereas current builds use `--no-zoom` to opt out. Compare the exported video to the source and watch it end-to-end too. If no camera motion appears, inspect the interaction telemetry rather than claiming the default flag guarantees motion. Browser-only recordings bypass Apparent's camera planner; prefer an Apparent capture of the browser when polished interaction zoom is required, and disclose an unpolished fallback rather than calling it equivalent. Apparent export supplies crop/zoom and recorded audio (or explicit generated narration via `--narration`), **not** burned explanatory captions or speed cues. Use an available editor to burn the required explanatory captions into a separate MP4; check every caption's timing against the visible state and keep the original. If no captioning route is available, report that production blocker. Never promise captioning or fast-forward as built-in Apparent flags. Do not silently speed through a state transition or overlay a success claim before the result exists. Check the final file opens in the intended player; use `ffprobe` to verify frame rate and duration, but not truth or privacy. A browser WebM may need an approved MP4 conversion for its destination; do not re-encode reflexively. Missing source media blocks live proof; use the workflow routing above before considering a conceptual supplement. For an explicit live-proof request, disclose the missing media and give truthful partial evidence without calling it the requested video.

## Hand it back

Embed or link the **verified playable file** in a channel the viewer can reach. Name the mode, exact build/environment for live proof (or source and hypothetical status for an explainer), and principal limit. State whether narration is generated or explicitly silent. Include a useful timestamp when it makes inspection faster. A path on an inaccessible machine is not delivery. Do not upload, post to a PR, send to a person, or publish raw or edited media without authorization for that destination; attachment visibility may inherit repository visibility. If production or delivery is blocked, name the exact failed step and best truthful partial evidence without calling the video done. Never claim a playable video exists until you have opened it.

## Agent recording trial lessons

- Read `capture state` for context, not ownership; a socket pathname is not readiness. Start with a retained request ID and require the request's own `recording` receipt before acting. Atomic admission refuses occupied sessions. Never adopt or stop a capture that was already active.
- Derive gestures from settled window bounds and verify the visible mark before continuing. A successful input command proves only that input was accepted.
- Use owned-ID `stop --wait` and its exact saved `bundle_path`, not a global state check. The command checks identity at the action; retrying it cannot stop a newer capture. If Finish already ended the take, inspect that ID or repeat its waited Stop to recover the same result. Unknown, cancelled, failed, wrong-ID and timeout replies are not saved success.
- Tie QA grids and cue-boundary frames to the final movie's hash; regenerate them after every edit. Inspect the actual deliverable before diagnosing a defect from an old grid.
- Keep a private attempt ledger with build/runner identity, elapsed times, retries and evidence paths. Separate harness failures from product defects. When comparing workflow-only retests, keep app and runner bytes fixed; disclose a different stop route and measure readable holds separately. Do not call different routes a Finish-button benchmark or infer total-time improvements from unequal setup.

## Failure traps

- A staged click after the result was already present proves nothing; reset to the starting state and capture the action.
- A smooth mock or browser replay is not evidence for a live native app; name the provenance visibly and in the handoff.
- Automatic zoom is the default presentation for live footage. Burned explanatory captions are required; fast-forward is optional and must be disclosed. None permits concealing failures or inventing causality.
- A complete source bundle is not a shareable artifact until its entire media and delivery destination have been checked.
