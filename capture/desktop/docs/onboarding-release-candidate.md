# Shared onboarding integration candidate

This ports the final onboarding from PR #683, source
`f109e3a61b0cdcf496877d722ec2209a1e23c7c6` (private Mac Preview 0.12.8),
onto current main's restored Preview 0.12.2 baseline. The original branch and
private downloads remain intact. This port does not publish a release or feed.

## Behavior and ownership

Welcome → pen → permissions if needed → choose shortcut → non-recording shortcut
trial → **Go again** in the full setup window → real guided capture → whiteboard
drawing → attach a text note → screen drawing → finish → manual paste → locate
the capture. The demonstration page is removed. Shortcut keys are larger and
centered in the compact setup window; the floating coach begins at capture selection.

Rust owns permissions, shortcut validation, progress and capture/note receipts.
Electron renders the shared Mac/Linux setup and finite, replayable guides, with
static reduced-motion alternatives. Linux retains explicit desktop-owned shortcut
acceptance. The Mac permission helper and Linux privacy adapter retain OS mechanics.
Missing assets or presentation failure retain native fallback.

A successful shortcut trial completes setup independently of the longer capture
lesson, so quitting or restarting does not repeat Go again. An explicit Settings
replay resets only tour milestones, including shortcut choice/trial; unrelated
settings, captures and OS grants remain intact. A completed existing user is not
re-enrolled. Unfinished older tours restart once.

The Mac DMG additionally contains the frog/app installer. Clicking either or
dragging the combined item installs into Applications with replacement consent,
signature/version checks and an old-app backup. Finder drag installation and the
existing Sparkle ZIP/feed remain available. No Linux installer UI is introduced.

## Acceptance before marking the PR ready

- Shared frontend build, unit/Electron checks, Rust build and affected/full suite.
- Independent review of shortcut admission, progress, durable note receipts and
  installer replacement/packaging boundaries.
- Isolated Linux: real shortcut choice/desktop binding, trial without recording,
  next press recording, guided drawing/note/paste/location, restart and replay.
- Mac: native compilation/tests and installed setup, permission/restart, real
  shortcut trial, guided capture and installer checks. Reused proof must identify
  its exact source and unchanged inputs; renderer fixtures are not native proof.
- Inspect the real practice bundle text/images and playable media; no private
  evidence goes into the PR. Verify exact-head CI and current-main integration.

Keep the PR draft until those gates pass. The user explicitly retains the merge.
Publication is separate and uses current main's manual-only verification and
release admission; the old baseline-branch exception is not extended by this port.
