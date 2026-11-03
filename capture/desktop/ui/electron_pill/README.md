# Shared recording pill

React/TypeScript and shared shadcn controls render Rust's bounded `View`.
Rust retains selection, capture admission, microphone warnings, processing,
clipboard/paste, sharing policy and finalizer receipts. The child receives no
bundle paths, capture contents, config or credentials. It cannot choose a
capture ID, path, URL, shell command or window coordinates.

The parent launches the packaged `electron/pill/main.cjs` from the existing
Desktop Electron runtime. Development builds resolve the built frontend beside
the source checkout. Linux selects X11 on the process command line, before
Electron initializes Ozone; a switch appended in JavaScript is too late on
Wayland desktops. There is no second backend or renderer recording clock.
State frames carry an action revision; Rust checks the current action set and
capture generation, consumes admitted revisions, and refuses expired commands.
Pipes and queues are bounded; writes and process reaping run off the GUI thread.
On Hyprland, show/move/resize notifications coalesce into bounded background
pin attempts through the existing Linux adapter. It matches the owned Electron
PID and XID-specific title; other Apparent windows keep their workspace behavior.
Linux lead presses use the existing native drag adapter: Rust checks the current
revision, drag policy, owned child/window and held OS button, then polls compositor
coordinates, clamps and remaps as required. CSS caption dragging cannot move an
override-redirect XWayland pill reliably. macOS retains native caption dragging.
When available, the existing kernel input reader supplies the held button:
XWayland's mask resets on remap. No new input access or collection is requested.
Native Linux moves also reconcile Electron's cached origin through the bounded,
revision-scoped parent pipe. The notes badge uses the actual pill frame on both
platforms, so it follows the replacement rather than the retired GPUI window.

## Notification pocket candidate

`View.pocket` optionally carries tone (`info`, `warning`, `failure`), title
(80 UTF-16 units), description (300), and at most three action IDs from the
existing authoritative action set. Unknown fields are stripped; no new mutation
route exists. Collapsible + Alert live in shared/ui; Desktop owns presentation.
Warnings, completion notes and bridge failures use the same vertical pocket.
Escape first collapses it, then retains the existing recording Cancel behavior.
The existing indicator position contains its disclosure; opening adds no width.

The native footprint is the union of the leading row and rounded pocket, with a
four-point transparent gap. Expansion preserves the leading-row screen anchor,
opens upward near the bottom, and clamps to the display work area. Main measures
trusted DOM and retains revision/readiness checks; the renderer chooses no native
coordinates. Synthetic Electron and geometry checks do not establish native
compositor, drag, capture exclusion, focus or receiving-editor acceptance.

`with_size_failure(size, limit)` projects numeric facts into a local Paste fallback.
Live upload-state caching/admission remains a separate integration hunk coordinated
with the public-sharing owner; this helper alone does not display a real queue
failure. No retry, compression, consent, clipboard or upload policy changes.
Installed routing/rollback and the native fallback stay under the producer's
existing acceptance hold. Remove donors only after accepted platform replacement;
this slice does not activate experiments or retire native presentations.
