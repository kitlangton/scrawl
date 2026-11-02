# Apparent: Agent Instructions

Apparent records screen, voice, cursor, clicks and ink so a human can point, talk and draw while an agent builds. It produces `.apparent` bundles for inspection and handoff. Supported platforms are **macOS and Linux**; do not add Windows support, tests, release assets or new docs. Existing Windows code is not a support promise; retire it only in a separately scoped, verified change.

## Product invariants

- Minimize human effort: capture, marking and handoff should be quick. Record known window, element, URL and session context instead of making agents infer it from pixels or vague references.
- Keep images visual and text semantic. Preserve time-aligned transcript, interactions and absolute paths in bundle text; avoid rasterized captions/card chrome. Cut keyframes on meaningful action or visual change, not speech pauses; retain full frames when cropping. See `capture/engine/docs/keyframes-and-text.md`.
- Persist user-controlled settings and feature flags in documented, inspectable config (currently `~/.config/apparent/config.json`). Settings writes update it; file edits apply at a documented time. Preserve unrelated keys and secrets. Reconcile OS-owned effects such as permissions and login registration; config values alone do not prove application. Active capture state is runtime state, but must be discoverable before restart/change.
- Share user-facing behavior and UI by default; platform adapters supply OS mechanics. Document actual limitations in `public/site/src/features.json` and test visible behavior on every claimed platform before release.
- Keep changes owner-local and bounded. Move a legacy mechanism with its callers/tests, preserve contracts and verify consumers before removing donors. Coordinate real cross-owner contract changes; routine owner work need not wait for an integrator.
- Delete verified dead implementations and obsolete tests/references rather than copying them into an archive. Establish requiredness from consumers, build/runtime/package inputs, supported compatibility/data readers and owner reservations—not merely deployment. Preserve active/uncommitted work, private captures, credentials and evidence. See `enablement/refactor.md` for campaign gates.

## Desktop UI direction

Electron is the selected desktop UI shell, prioritizing a shared macOS/Linux experience and fewer rendering differences. This is the target direction, not a claim that migration is implemented. Keep capture and processing in Rust, independent of UI animations and responsiveness; measure pill responsiveness and resource contention on both platforms. Native adapters may still be needed for OS integration; the exact overlay boundary remains to be established.

Desktop is a fast capture-and-handoff tool, not a workspace: no desktop review screen; most conventional UI belongs in infrequently opened Settings. The Loom-like recording-sharing workspace for coworkers and their agents belongs in web/Connect. Reuse UI components where useful without embedding that workspace in Desktop. Shared UI does not replace platform checks for permissions, capture, focus, overlays and packaging.

Minimize both stop-to-paste and stop-to-watch delays. Connect links are expected to be the common handoff and may be copied before upload/processing completes; a recipient-facing “Preparing recording” state is acceptable. Keep “link copied” distinct from “ready to watch”; never present the former as proof of the latter. This direction does not authorize an unscoped migration or release.

## Ownership and layout

The accepted ownership target is Engine, Desktop and Agent under `capture/`, with `connect`, `public`, `shared` and `enablement` as separate owners. Their dependency direction is architectural: Desktop and Agent may depend on Engine; Engine depends on neither. Keep capture-internal shared mechanisms in `capture`. One root Rust application package uses ordinary modules; no path shims, extra crates or services to simulate ownership.

Production sources and owner materials have physically relocated: Engine code/docs/native files are in `capture/engine/`; Desktop code/docs/native files and performance tools are in `capture/desktop/`; Agent code, proof/narrator tools, skills and docs are in `capture/agent/`. `composition/` holds CLI/composition code, `shared/` cross-stream code/docs, and `enablement/` repository verification and refactor guidance. `public/site/video-demo/` holds marketing video tooling. These are physical paths within the root application, not proof that all production behavior has been adopted into an independently running Engine. Treat the ownership and dependency direction above as the target; do not claim that migration is complete.

| Area | Current physical paths | Read first |
|---|---|---|
| Capture lifecycle, recording, processing and bundles | `capture/engine/` (including `runtime/`, `bundle/`, `export/`, `native/`, `docs/`); related cross-stream code in `shared/` | `capture/engine/docs/capture.md`, `capture/engine/docs/keyframes-and-text.md`, `capture/engine/docs/export-and-retention.md` |
| Human controls, settings, annotation and handoff | `capture/desktop/`; CLI in `composition/command_line.rs` | `capture/desktop/docs/onboarding-and-settings.md`, `capture/desktop/docs/linux/linux.md` |
| Agent requests, inspection and proof-video workflows | `capture/agent/` | `capture/agent/docs/agent-video-workflow.md`, `capture/engine/docs/keyframes-and-text.md` |
| iPhone capture and routing | `capture/engine/runtime/phone.rs`, `capture/desktop/{web.rs,sdk.js}`, `shared/{routing.rs,native/}` | `capture/engine/docs/iphone-and-routing.md` |
| Capsule motion/lab | `capture/desktop/ui/{lab.rs,motion.rs}`, `capture/desktop/scripts/perf/` | `capture/desktop/docs/motion.md` |
| Linux shell, portals, input, GStreamer and AppImage | `capture/desktop/linux/`, `capture/desktop/ui/linux.rs`, `capture/engine/runtime/platform/linux.rs`, `capture/desktop/scripts/linux/` | `capture/desktop/docs/linux/linux.md` |

For repo-owned source, assets, docs and tooling, target at most 10 immediate files and 10 immediate directories per directory. Count hidden repo-owned entries; exclude generated output, dependencies, build caches and Git internals. Keep tool-required exceptions (such as root Cargo manifests, `.github`, `.githooks`, `.cargo`) visible and explicitly bounded; do not hide or misclassify overfull directories. Keep docs minimal and beside their owner; retain non-obvious constraints, avoid duplication, and do not add `root/docs/` material.

## Understand, change and verify

Read the relevant owner docs before changing a subsystem. Keep `Cargo.toml` and `Cargo.lock` stable unless explicitly required. After Rust changes, rebuild before running `target/debug/apparent` (`cargo test` does not rebuild it). For broad root checkpoints use `cargo build -j 3 --offline` and `cargo test -j 3 --offline -- --test-threads=2` (on macOS add `taskpolicy -c utility`); on Clanker run each heavy command or hook-backed push once through `/home/simon/.local/bin/t3-build`, never nested, honoring `T3_BUILD_JOBS`.

Verification is proportional to changed behavior, not commit IDs. For mechanical moves verify preserved content/build inputs, imports/visibility, affected compilation on macOS/Linux and focused existing tests. Reuse evidence only for unchanged inputs and state its source. Broad suites are for shared/build/security/lifecycle changes, integration checkpoints or uncertain impact. New native proof is for changed native behavior. Never turn an unrun check into a pass or waive unresolved failures/platform gates; investigate red tests, fix the underlying issue and rerun focused then full affected-platform tests.

`.githooks/pre-push` and `.github/workflows/verification.yml` use `enablement/verification.py`; selector tests are in `enablement/tests/`. Its reviewed document and Engine-test-only rules may narrow checks; unknown paths, shared inputs and selector/hook/CI changes broaden to root plus Engine build/tests. It does not assume an arbitrary Rust diff is mechanical. Preserve existing website/Connect, updater and release gates; never bypass hooks/protection. Feature pushes compare the full delta from local `origin/main` merge-base (fetch main before publishing); CI uses PR base or main push before-SHA, and missing history broadens. Details: `enablement/docs/continuous-delivery.md`.

When output/UX changes or a capture test affects output, inspect the relevant real bundle's text and images; for mechanical moves reuse relevant evidence after checking source/import/build-input preservation. Report unavailable evidence and concrete output/UX issues; do not make a capture solely because files moved. Documentation/tooling-only changes need no bundle inspection.

For Linux-facing changes, a Mac build on Linux is not a Linux test. Follow the Linux/Omarchy procedure in `capture/desktop/docs/linux/linux.md`, inspect the actual Linux checkout instructions and VM launcher, and test the exact candidate in the isolated VM. Check for existing Apparent VM processes before staging/launching; never replace a worker's app or substitute a host build, `--help`, or X1 install for a GUI test. For Mac Settings/native/cabled-mirror proof, use a Mac. See `capture/desktop/docs/onboarding-and-settings.md` and `capture/desktop/docs/native-testing.md`.

## Deliver, safety and consent

At assignment start, identify the requested artifact, checks/platform proof, and whether merge/release is explicitly authorized. For implementation normally deliver a scoped commit and draft PR; obey explicit exceptions (including no Git/publication). Verify the exact diff and any remote state before claiming it. A draft PR does not authorize merge, deployment, release, issue closure or outbound messages. Never include secrets, private evidence or another writer's work in a delivery.

- Never reset Kit's real TCC grants; use the preview workflow in `capture/desktop/docs/onboarding-and-settings.md`. Never wake/flash Kit's Studio displays or record/benchmark its real screen; use the safety gate `capture/desktop/scripts/perf/screen-guard.sh` and approved Pro/virtual-display/VM setup in `capture/desktop/docs/motion.md`.
- Never play audio aloud on Kit's Macs to test capture/transcription; use recorded WAVs, approved silent loopback or Mac mini VMs. Tests must not post to a real OpenCode session.
- Preserve old bundle session metadata but do not treat it as permission to forward captures. Use `apparent copy <bundle>` for clipboard handoff; ask before any separate delivery to a person, organization or session. Keep captures, recordings, transcripts, credentials and personal data out of public issues/uploads; sanitize evidence or use a synthetic reproduction. Search existing issues before filing an evidence-backed report; if access fails, report the blocker and retain a local draft.
- Do not claim permission, GUI, native, platform parity or video proof from a build, feature-map entry, CLI help or bundle path alone. When Simon says “make it apparent,” follow `capture/agent/skills/make-it-apparent/SKILL.md`, inspect the complete recording and deliver a playable video in the current conversation; state limits and blockers truthfully.

## Product gates and operations

User-visible changes to handoff/output ship behind an off-by-default experiment until Kit enables it for everyone; bug fixes ship normally. Config is `~/.config/apparent/config.json`, the code checks `shared::handoff::experiment("<name>")`, tests use `shared::handoff::enable_experiment`, and `apparent reprocess`/`inspect` accept `--experiment <name>`. Invalid config values are skipped/logged and a bad key drops only that key. Current flags and output contract: `capture/engine/docs/keyframes-and-text.md`.

Keep `public/site/src/features.json` accurate in the same PR when adding/removing/renaming a user-facing feature or changing its status. It records status, platforms, shortcuts, config keys and docs; release changes `main` entries to `stable@<version>`. `cargo test feature_map` checks code/config entries. Release procedures: `capture/desktop/docs/release.md`.

Install/launch the Mac app through `capture/desktop/scripts/packaging/install-app.sh`, not a `target/` daemon: cleanup can unlink it and disturb TCC identity. For remote Mac installs use `capture/desktop/scripts/packaging/install-app-remote.sh`; release signing and update storage stay outside `target/` and `/tmp`. Details and recovery are in `capture/desktop/docs/onboarding-and-settings.md` and `capture/desktop/docs/release.md`.
