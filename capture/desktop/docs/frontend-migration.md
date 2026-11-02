# Frontend migration plan

> **For Hermes:** This is a planning artifact, not authorization to migrate, publish or release. Follow the assigned execution scope; delegate only authorized independent work.

**Goal:** Replace desktop rendering without losing capture, settings or handoff behaviour, using standard shared controls rather than pixel-perfect replicas.

**Architecture:** Electron hosts React/TypeScript UI styled with shadcn/ui and Tailwind. Rust retains capture, processing and authoritative runtime state. Desktop and Connect share canonical component sources and theme tokens, not application policy or account state.

**Tech stack:** Electron, React, TypeScript, shadcn/ui, Tailwind CSS, existing Rust application and necessary native adapters. The opt-in read-only foundation lives in `capture/desktop/ui/frontend`; canonical controls and tokens live in `shared/ui`. See `../ui/frontend/README.md` for launch and platform-check scope. This does not migrate the installed app or establish capture/native-overlay acceptance.

## Boundaries

- Desktop is capture-and-handoff, not a review workspace. Connect owns collaboration; building that workspace is not part of this migration.
- Preserve workflows, capability, consent, shortcuts, config semantics and output. Slight layout/style changes are acceptable to avoid bespoke ordinary controls.
- Keep capture independent of renderer reloads, animation and responsiveness. No second recording state machine in JavaScript.
- No generic renderer shell/filesystem execution. Explicit bridge commands, validated inputs and trusted local content only; design Electron isolation before exposing controls.
- Keep the working GPUI/native path until a replacement passes equivalent checks. Remove a donor only after its consumers, packaging and runtime inputs are verified.
- No Windows work. Do not assume Electron removes Wayland or macOS permission constraints.

## Delivery shape

Use short-lived, independently reviewable PRs against current `main`, not a long-lived migration branch. Keep new frontend code owner-local; reserve shared manifests and integration points for explicit, small edits. Do not combine broad moves, cleanup or unrelated feature work with a slice. Temporary coexistence is acceptable, but Rust remains the single owner of settings and capture policy. Each replacement needs its own acceptance evidence and a defined removal point for the old implementation.

## 1. Read-only foundation

The first implementation uses the existing same-user Rust GUI control socket's read-only `state` request, displaying only the validated runtime phase. It does not add a version protocol, launch a backend, or treat packaged/mock data as live state. Electron owns only the connection and display; the Rust GUI remains authoritative. Synthetic previews are separate. See the foundation README for explicit disconnected/error states, permission checks, deadlines and forbidden-command coverage.

Do not change the default launcher, installed app identity, permissions, capture lifecycle, recording controls or config in this slice. Do not expose capture or settings mutation commands. Verify the shell and read-only bridge on macOS/Linux; this proves neither overlay behaviour nor capture independence. No full capture migration gate is attached to this read-only slice unless its actual diff changes capture inputs.

## 2. First useful migration: one Settings section

Investigate the existing serializable `View` in `capture/desktop/settings/settings.rs` and its production producers/consumers. Choose one bounded section, then render it with standard controls while retaining Rust's existing policy. Define its exact actions, readback, error states and native checks before implementation; do not expose the entire Settings mutation surface merely because the view contains it. Keep unrelated sections on their current path. Preserve config concurrency and OS effects wherever the selected section touches them.

## 3. Separate pill/overlay investigation

Read `capture/desktop/mod.rs`, `capture/desktop/native/pill/{mod.rs,types.rs}`, `capture/desktop/ui/linux.rs`, and their callers. Trace Engine calls, timers, input/ink ownership, handoff, shutdown and startup. Inspect root build inputs and desktop packaging before choosing new file paths.

Use `capture/engine/docs/capture.md#agent-control-of-the-running-gui` as the existing control contract: it has request-owned start/stop/cancel and receipts, but currently requires the running GUI. Do not present it as an already independent headless Engine or replace it with the telemetry-incomplete timed recorder.

Deliver a small surface/owner table and the proposed narrow capture bridge. Identify which UI-owned lifecycle responsibilities must move before the Rust backend can outlive an Electron renderer. Keep the Engine-to-Desktop dependency prohibition. This work does not block independent ordinary Settings slices and is not a prerequisite for the read-only foundation.

### Later candidate: real pill-to-handoff slice

Build only the minimum shell and renderer needed to start a normal screen capture, show authoritative state, finish or cancel, and retain existing local bundle/clipboard handoff. Add a synthetic-state preview for development, clearly separated from real capture. Do not build a settings showcase in place of the integration.

Prove input/focus, pointer pass-through, window stacking, display scaling/placement and workspace behaviour before choosing the final pill/ink boundary. Keep native adapters if the required behaviour cannot be provided reliably by Electron. Do not redesign the picker, whiteboard or output contract in this slice.

Exercise repeated start/stop, pending portal cancellation, rejected occupied requests, finalization failure, renderer reload/disconnect, and stale capture IDs. An older request must never stop a newer recording. Inspect real bundle text and images for missing interactions, UI contamination or privacy regressions.

### Capture/overlay measurements and native gates

Record a baseline from the current application before choosing numerical performance limits. Compare idle resources, recording contention, pill response, stop-to-local-handoff and stop-to-watch separately on the same host/scenario. Link copied is not ready to watch. Report unrun measurements rather than inventing budgets.

Linux: follow `linux/linux.md` and `native-testing.md`; inspect the actual isolated VM launcher and existing VM processes before staging the exact candidate. Existing capture-smoke checks are useful but do not prove pill clicks, focus, ink or clipboard/paste acceptance: add explicit native evidence for those changed paths.

macOS: follow `onboarding-and-settings.md` and `motion.md`; use approved preview/virtual-display or disposable test setups and the prescribed app install scripts. Preserve app/helper signing and TCC identity; never reset real grants or record Kit's real displays.

A browser preview/build is not native proof. Set the shell/native boundary from these results; do not reopen the selected Electron direction merely because an adapter is necessary.

## 4. Expand in bounded, independently verified slices

Continue onboarding and Settings migration using standard shadcn controls plus shared theme tokens, without waiting on unrelated overlay experiments. Preserve permission refresh/revocation, shortcut ownership/validation, microphone fallback/meter semantics, config concurrency and unknown-key preservation, and actual OS registration effects. See `onboarding-and-settings.md` for the contracts; cosmetic animation fidelity is not required. Each slice carries checks proportional to its changed behaviour, not all future migration gates.

Inventory any existing review surface and preserve required handoff actions outside it before removal; the target has no desktop review screen. Coordinate canonical shared UI placement across owners before editing manifests. `connect/web/AGENTS.md` currently limits its staging extraction to behaviour-identical copies: do not silently repurpose that slice or confuse its staging package with deployed Connect.

Then migrate remaining picker/whiteboard/annotation surfaces in bounded increments, with relevant platform proof. Keep Connect functionality and marketing redesign out of scope; shared controls do not authorize a workspace rewrite.

## Verification and delivery

For each slice list exact changed files, focused test commands and actual results after source discovery. Use the repository verification selector and existing Clanker build-runner rules for affected Rust builds/tests; rebuild before running the debug app. Run browser type/build/interaction checks for changed web packages, and both-platform native checks for changed desktop behaviour.

Update `public/site/src/features.json` when visible capability/status changes. Verify the exact diff; preserve unrelated work. Commit, PR publication, merge and release require the applicable explicit authorization. This document has no executed native/performance acceptance evidence.
