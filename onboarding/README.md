# Onboarding package

`@apparent/onboarding` is a TypeScript setup and practice owner. It exports a pure controller and React contribution factory. It owns versioned progress, resume/defer/completion, coaching stages, practice association, and manual paste confirmation. It consumes the exact packed public `@apparent/record/capabilities` contract; it imports no Record implementation, EngineClient, Electron API, or Rust.

## Build and tests

Run `npm ci`, `npm run typecheck`, `npm test`, `npm run build`, and `npm pack --dry-run` in this directory. The development dependency is the committed public Record tarball `vendor/record-0.1.0-db88348.tgz`, built from main `db88348d64d41a25346929503d0fd701ad8ca0a8` (reviewed change `a6b6e5402ddafa0d9d13493692fd6d59f719de49`), SHA-256 `9c46689db9a8c2909905633921497123d522dc928f21daa65816457ea900b555`. The lockfile pins its packed declarations/runtime; Onboarding install does not build the Record source tree. Tests use `createRecordFixture` from that package and mount the React contribution in jsdom for real button clicks. This proves fixture behavior, not installed behavior. No Cargo/rustc is required.

The shell injects the canonical `Button` from `@apparent/ui/button` and loads `@apparent/onboarding/styles.css`. That keeps this owner independently typecheckable/buildable while sharing canonical controls and design tokens at composition.

## Record contract mapping and adoption limits

Onboarding imports the public capabilities from the packed Record package. Commands use a unique request ID and the observed provider epoch/revision. `accepted` means dispatch only; only provider facts or `applied` plus readback can advance a milestone. Disposal unsubscribes from observation; it never stops or cancels a recording.

Practice excludes captures present when the practice step begins and requires successful shortcut and first-frame facts on one recording. However the packed Record API has no fact proving that shortcut admission was enforced as local-only, so practice intentionally remains pending instead of treating any observed capture as eligible. Permission continuation requires all required requirements usable now with no relaunch pending. Shortcut continuation requires a nonempty effective binding, no draft, and confirmed registration; an externally-managed boolean alone is not proof. Pen selection advances only after an `applied` result whose snapshot reads back the selected effective value and the animation completes (or reduced motion is enabled).

Record currently provides generic `PenPreference.supported` values in fixtures (`ink`, `laser`, `cursor`), not the donor's coral/purple/blue/green colors. Onboarding offers exactly the provider-supported values and uses a neutral puddle for unknown styles; the shipped colored-pen journey needs a typed Record capability for those colors. Record also has no typed picker target or accepted-stroke surface. Onboarding never interprets arbitrary detail strings as proof and keeps these steps pending until the typed facts exist.

Processing, save and copy are separate outcomes on the associated capture. Paste confirmation is a manual user action; no private composer is observed. Locate calls `hasConfirmedArtifactOpen(snapshot, captureId)`, which requires the saved exact capture and current artifact identity with `viewer-ack` or `os-observation`. Dispatch alone never completes the lesson. #665 added the typed fact/helper and fixture coverage; no production provider emits the confirmation yet. Installed setup remains pending at Locate until Record maps trusted viewer acknowledgement or supported OS observation to the saved artifact identity. Never infer opened/watched from focus, process, path, or dispatch success.

Record exposes `start-capture` with `delivery: "local-only"`, but practice must be associated with the user's effective shortcut and actual first frame. The current API has no fact proving that shortcut-started admission was enforced as local-only. The installed adapter must enforce local-only for that route or refuse adoption. Onboarding does not issue a second menu start or claim local-only proof from fixtures.

## Persistence, host adoption and rollback

`ProgressStore` requires read and compare-and-swap `(revision,value)`, distinguishing saved, conflict/current snapshot, and unknown publication. Unknown is not success. The only store in tests is in-memory. Installed progress must use the sole transactional writer after verified cutover; no filesystem read/modify/write or competing writer is supplied. The shell provides activation reasons (`firstLaunch`, `settings`, `resume`) and declarative coach placement, then renders `createOnboardingContribution({controller, Button, close, onManualPasteInstructions})`. No arbitrary IPC/window/URL/filesystem effect is exposed. There is no production host/preload adapter or second installed surface.

The shipped Preview 0.12.2 journey in the read-only release-baseline checkout is the design donor for frog/puddle artwork, approved copy, and pen-hop motion. Only the frog asset is copied here. Host routing remains unchanged until this contribution and its adapters are accepted. Rollback removes the host contribution registration and returns to the existing setup route; progress is not dual-written or migrated by this package.

Remove the donor only after typed Record capabilities, truthful local-only admission, production artifact-open evidence, transactional progress writer, host contribution/preload, shared UI checks and affected native seams are verified; existing resume/defer/completion state is preserved; and every source/build/package consumer has moved. A passing fixture package is not installed adoption or native proof.
