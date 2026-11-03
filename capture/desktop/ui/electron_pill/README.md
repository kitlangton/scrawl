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
`Client::upload_size_failure(bundle, UploadQueueKind)` returns only kind, size and
limit for the latest failed job of that intent. Private encrypted jobs cannot
populate PublicCopy facts. Desktop caches those facts with the exact completed
bundle and capture generation; phase, selection, generation and current intent
must still match before projection. Background results also require unchanged
clipboard-delivery identity. Capture changes invalidate the cache.

The existing five-second sharing tick inspects local private queue failures when
hostedSync is already enabled, without probing public admission. After actual
public-link clipboard delivery it inspects the separate PublicCopy queue instead.
This does not change public-copy preparation failures inside the consent popup;
that surface retains its own owner and error handling. No retry, compression, consent, clipboard or upload policy changes.
Installed routing/rollback and the native fallback stay under the producer's
existing acceptance hold. Remove donors only after accepted platform replacement;
this slice does not activate experiments or retire native presentations.

## Proof route and preflight

The synthetic Electron fixtures and production Rust pipe test have different
sandbox prerequisites. Before the ignored `electron_pill_real_pipe_roundtrip`
fixture starts a child, its Linux preflight requires either the development
runtime's valid root-owned SUID helper or a successful unprivileged user-namespace
probe. A refusal is an **environment** failure before candidate launch, not a
product verdict or a skip/pass. A positive probe still requires actual sandboxed
execution; it proves neither compositor visibility nor native acceptance. Retain
an earlier “No usable sandbox” failure instead of relabelling it.

On a restricted host, continue offline with build/package, renderer/geometry
checks and scenario preparation. The supported live Linux route is
[`gui-vm.py` with the existing capture-smoke scenario](../../docs/native-testing.md),
using a sanitized stopped template, test identity, a new private overlay/results
path, isolated networking and the exact packaged candidate. Check current QEMU,
slot owner, frozen candidate and consumed take/admission constraints before boot;
another worker's guest or a persistent VM is never a fallback. Follow the
runner's documented KVM group route rather than changing device permissions.
A failed host probe does not establish that this guest route is blocked.

For the pocket, prepare the existing save/cancel scenario plus native observation
of a real warning pocket: disclosure open/close at constant row width, upward
placement near the bottom, row anchor, rounded corner/gap click-through, current
capture authority, focus and actual local Paste into a receiving editor. Keep
renderer-only injected states labelled synthetic. Inspect saved bundle text,
images and full movie; capture exclusion and receiving-editor delivery remain
separate gates. Require the producer's exact applicability/release receipt and
pocket-specific Mac/Linux evidence before merge/installed routing. Missing tests
on a feature branch are expected under main-only full CI, not a dependency wait.

Before admitting a live take, prepare the scoped VNC/QMP screenshot and input
route, not just the candidate and scenario. The existing capture recipe's Start
call begins its portal-consent deadline immediately and blocks while waiting;
tool discovery or unrelated coordination inside that window can consume the
take without any native observation. Bind the viewer/input to the new owned
QEMU process, sockets and fresh dialog geometry before posting input. If Start
times out without a retained dialog observation, keep consent-versus-backend
causation unresolved, retain the cold evidence and consumed take identity, and
fix readiness offline. A new take needs fresh admission; never reuse its spent
identity or count the launcher/GUI-ready gates as recording or pocket proof.
