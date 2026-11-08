# Apparent physical source-organization campaign

Status: **integration in progress** from `ba9de38097ec76454b57e095a1254d330b62eb1f` (single existing Rust application crate; no new Rust crate, standalone Agent runtime, or Engine production adoption). This section supersedes the prior completed-boundary milestone and every conflicting target/proposal below it. The older material remains historical evidence only; it is not current authorization, scope, or a migration queue. This campaign's old reservations and exhaustive ledger are historical, not authoritative. The accepted target in `AGENTS.md` now governs: `capture/{engine,desktop,agent}`, `connect`, `public`, `shared`, `enablement`; preserve one root Rust package and ordinary supported modules, with no path shims or new crates/services. The currently completed physical checkpoint is recorded in `/home/simon/.hermes/plans/apparent-root-cleanup-result.md`; it does not claim the final Rust ownership moves are complete.

## Historical proposal, superseded by AGENTS.md

Keep one root `Cargo.toml`/`Cargo.lock` Rust application package and root composition entrypoint. Organize Rust owners as ordinary modules beneath `src/{capture,desktop,agent_proof,connect,shared,composition}`; use normal `mod` declarations and nested `mod.rs`, never `#[path]` relocation shims. The already-existing top-level `capture/engine` and `connect/client` packages remain at those exact paths; this campaign neither adopts nor creates packages for them. Existing user behavior and external identities stay unchanged.

The physical final tree has these anchors (all paths not shown here are enumerated, without omission, in the exhaustive 634-row ledger below):

```text
Cargo.toml / Cargo.lock / build.rs     # one root app package; build.rs is a single integration point
main.rs                             # thin root entrypoint + root module declarations
src/composition/{mod.rs,cli.rs}        # command-line parsing and application dispatch
src/shared/{mod.rs,clock.rs,ids.rs,demo.rs,autocrop_lab.rs,autocrop-corpus.json,autocrop-playground.html}
capture/engine/runtime/mod.rs                      # capture owner module, with existing capture submodules
capture/desktop/mod.rs                      # GPUI/Desktop owner module; web SDK under capture/desktop/sdk.js
src/agent_proof/                        # request/evidence owner modules; no standalone runtime/package
src/connect/                             # in-crate adapter modules; existing connect/client package stays at root
capture/engine/                          # existing package, unchanged ownership and path
connect/client/                          # existing package, unchanged ownership and path
capture/native/, desktop/native/, shared/native/ # explicit owner-native helper inputs
public/site/                             # existing public site, moved by its reserved lane with complete wiring
connect/service/                         # existing hosted service, moved by its reserved lane with complete wiring
```

The native handoff now assigns helper inputs to `capture/native/**`, `desktop/native/**`, and `shared/native/**`; `build.rs` explicitly selects owner-specific source groups sorted by the original basename order. The native source/build lane is frozen after source preservation and Linux build-script metadata checks; Mac native compilation remains a gate. Public site and hosted service physical moves are complete at `public/site/**` and `connect/service/**`. I own their external wiring, root tooling/CI and documentation, plus `connect/web/**` path references only; leaf source/destination content remains owner-frozen.

Do not add or adopt standalone Agent/Engine executables, relocate all source owners in one batch, or alter behavior. No deletion/Windows retirement is included; legacy unsupported code remains unless a separately bounded, verified retirement is authorized.

## Implemented first slice and reservation boundary

This first dependency-complete slice is an in-crate module extraction, not a package cut:

- `main.rs` retains root module declarations and now only delegates to `composition::cli::run`; the root `pub use bundle_processing` facade is removed.
- `src/composition/{mod.rs,cli.rs}` owns the moved CLI parser/dispatch and its existing tests. The helper skill `include_str!` path is corrected for its new location; `Cli`, `Command`, and subcommand types remain private.
- `src/shared/{mod.rs,clock.rs,ids.rs,demo.rs,autocrop_lab.rs,autocrop-corpus.json,autocrop-playground.html}` owns the extracted `now_epoch_ms`, `make_bundle_id`, and `generate_demo_bundle` helpers.
- Direct callers now use the real `shared` and `bundle_processing` modules. The hosted alias regression keeps its behavior through a test-only subprocess adapter rather than exporting CLI types. `capture/desktop/docs/native-testing.md` points `CaptureCommand::Start` to `composition/cli.rs`.
- The extraction preserves helper and CLI/test bodies; it does not move other owner implementations, modify native build inputs, or claim platform/native/GUI proof.

The exact currently modified Rust callers are the `src` paths changed in this batch; do not absorb them into another owner move. The public/service/native leaf workers are terminal. **Next disjoint Rust reservation (not executed here):** Capture owner gets `capture/engine/runtime/mod.rs`→`capture/engine/runtime/mod.rs`, `capture/engine/runtime/audio/audio.rs`→`capture/engine/runtime/audio/audio.rs`, and `capture/engine/runtime/audio/mic.rs`→`capture/engine/runtime/audio/mic.rs`; existing `src/capture/{linux.rs,tests_recorder.rs}` stay in place. Keep `mod capture;` in root `main.rs`, remove root `mod audio;` and `mod mic;`, declare `mod audio; mod mic;` inside `capture/mod.rs`, and rewrite all external imports to `crate::capture::{audio,mic}::...`. Do not add donor reexports. Keep `capture/engine/runtime/phone.rs` and `capture/engine/runtime/screens.rs` separate because of cross-owner/platform callsites. Subsequent non-overlapping reservations: Desktop `capture/desktop/mod.rs`/`src/ui/**` and onboarding/settings/shortcut adapters; Agent-proof `src/agent_proof/context.rs` plus proof-only interfaces; Connect adapter `connect/hosted.rs` plus its tests. Do not edit `main.rs`, `src/composition/**`, `src/shared/**`, `capture/engine/bundle/bundle_processing.rs`, or this batch's rewritten callsites during the Capture move; obtain a direct source/caller/test inventory first.

## Historical documentation ownership (reconcile against current tree)

The following statements preserve the earlier campaign's rationale, not current locations: product contracts were routed to Capture, Desktop, Connect and shared docs; repository delivery governance was considered root-owned; Agent-proof workflow docs and root verification files were recorded at their then-current paths. The current mappings and verification results are kept in the private checkpoint handoff referenced above; this historical ledger must not override those mappings.

## Historical tracked-path destination ledger (not current acceptance)

The following exhaustive baseline ledger documents an earlier campaign proposal and must not be used to infer current owners, completion, or remaining paths. The accepted target is `AGENTS.md`; the live checkpoint and its actual mappings are in `/home/simon/.hermes/plans/apparent-root-cleanup-result.md`.

| Tracked path | Final path | Disposition | Owner |
|---|---|---|---|
| `.cargo/config.toml` | `.cargo/config.toml` | RETAIN | shared-infrastructure |
| `.gitattributes` | `.gitattributes` | RETAIN | shared-infrastructure |
| `.githooks/pre-push` | `.githooks/pre-push` | RETAIN | shared-infrastructure |
| `.github/ISSUE_TEMPLATE/bug_report.yml` | `.github/ISSUE_TEMPLATE/bug_report.yml` | RETAIN | shared-infrastructure |
| `.github/workflows/linux-updater-harness.yml` | `.github/workflows/linux-updater-harness.yml` | RETAIN | shared-infrastructure |
| `.github/workflows/release-linux.yml` | `.github/workflows/release-linux.yml` | RETAIN | shared-infrastructure |
| `.github/workflows/release-mac-preview.yml` | `.github/workflows/release-mac-preview.yml` | RETAIN | shared-infrastructure |
| `.github/workflows/release-windows.yml` | `.github/workflows/release-windows.yml` | RETAIN | desktop (legacy unsupported code; retained, out of scope) |
| `.github/workflows/verification.yml` | `.github/workflows/verification.yml` | RETAIN | shared-infrastructure |
| `.github/workflows/web-delivery.yml` | `.github/workflows/web-delivery.yml` | RETAIN | shared-infrastructure |
| `.gitignore` | `.gitignore` | RETAIN | shared-infrastructure |
| `AGENTS.md` | `AGENTS.md` | RETAIN | shared-infrastructure |
| `CONTRIBUTING.md` | `CONTRIBUTING.md` | RETAIN | shared-infrastructure |
| `Cargo.lock` | `Cargo.lock` | RETAIN | shared-infrastructure |
| `Cargo.toml` | `Cargo.toml` | RETAIN | shared-infrastructure |
| `LICENSE` | `LICENSE` | RETAIN | public |
| `README.md` | `README.md` | RETAIN | public |
| `assets/AppIcon.icon/Assets/brush.png` | `capture/desktop/assets/AppIcon.icon/Assets/brush.png` | MOVE | desktop |
| `assets/AppIcon.icon/Assets/glow.svg` | `capture/desktop/assets/AppIcon.icon/Assets/glow.svg` | MOVE | desktop |
| `assets/AppIcon.icon/icon.json` | `capture/desktop/assets/AppIcon.icon/icon.json` | MOVE | desktop |
| `assets/Apparent.icns` | `capture/desktop/assets/Apparent.icns` | MOVE | desktop |
| `assets/Apparent.ico` | `capture/desktop/assets/Apparent.ico` | MOVE | desktop (legacy unsupported code; retained, out of scope) |
| `assets/app-icon.png` | `capture/desktop/assets/app-icon.png` | MOVE | desktop |
| `assets/brand/apparent-a.png` | `shared/assets/brand/apparent-a.png` | MOVE | shared |
| `assets/brand/apparent-app-icon.svg` | `shared/assets/brand/apparent-app-icon.svg` | MOVE | shared |
| `assets/brand/apparent-wordmark.png` | `shared/assets/brand/apparent-wordmark.png` | MOVE | shared |
| `assets/brand/apparent-wordmark.svg` | `shared/assets/brand/apparent-wordmark.svg` | MOVE | shared |
| `assets/fonts/HostGrotesk-Variable.ttf` | `shared/assets/fonts/HostGrotesk-Variable.ttf` | MOVE | shared |
| `assets/fonts/OFL.txt` | `shared/assets/fonts/OFL.txt` | MOVE | shared |
| `assets/icon.svg` | `capture/desktop/assets/icon.svg` | MOVE | desktop |
| `assets/icons/area.svg` | `capture/desktop/assets/icons/area.svg` | MOVE | desktop |
| `assets/icons/arrow-up-right.svg` | `capture/desktop/assets/icons/arrow-up-right.svg` | MOVE | desktop |
| `assets/icons/check-circle.svg` | `capture/desktop/assets/icons/check-circle.svg` | MOVE | desktop |
| `assets/icons/check.svg` | `capture/desktop/assets/icons/check.svg` | MOVE | desktop |
| `assets/icons/chevrons.svg` | `capture/desktop/assets/icons/chevrons.svg` | MOVE | desktop |
| `assets/icons/click.svg` | `capture/desktop/assets/icons/click.svg` | MOVE | desktop |
| `assets/icons/drive.svg` | `capture/desktop/assets/icons/drive.svg` | MOVE | desktop |
| `assets/icons/gear.svg` | `capture/desktop/assets/icons/gear.svg` | MOVE | desktop |
| `assets/icons/hand.svg` | `capture/desktop/assets/icons/hand.svg` | MOVE | desktop |
| `assets/icons/info.svg` | `capture/desktop/assets/icons/info.svg` | MOVE | desktop |
| `assets/icons/iphone.svg` | `capture/desktop/assets/icons/iphone.svg` | MOVE | desktop |
| `assets/icons/lock.svg` | `capture/desktop/assets/icons/lock.svg` | MOVE | desktop |
| `assets/icons/mic.svg` | `capture/desktop/assets/icons/mic.svg` | MOVE | desktop |
| `assets/icons/person.svg` | `capture/desktop/assets/icons/person.svg` | MOVE | desktop |
| `assets/icons/record.svg` | `capture/desktop/assets/icons/record.svg` | MOVE | desktop |
| `assets/icons/reopen.svg` | `capture/desktop/assets/icons/reopen.svg` | MOVE | desktop |
| `assets/icons/reset.svg` | `capture/desktop/assets/icons/reset.svg` | MOVE | desktop |
| `assets/icons/screen.svg` | `capture/desktop/assets/icons/screen.svg` | MOVE | desktop |
| `assets/icons/spinner.svg` | `capture/desktop/assets/icons/spinner.svg` | MOVE | desktop |
| `assets/icons/terminal.svg` | `capture/desktop/assets/icons/terminal.svg` | MOVE | desktop |
| `assets/icons/window.svg` | `capture/desktop/assets/icons/window.svg` | MOVE | desktop |
| `assets/icons/x.svg` | `capture/desktop/assets/icons/x.svg` | MOVE | desktop |
| `assets/menubar-icon@2x.png` | `capture/desktop/assets/menubar-icon@2x.png` | MOVE | desktop |
| `assets/settings/frog-jump.png` | `capture/desktop/assets/settings/frog-jump.png` | MOVE | desktop |
| `assets/settings/frog-lounge.png` | `capture/desktop/assets/settings/frog-lounge.png` | MOVE | desktop |
| `assets/settings/frog-puzzled.png` | `capture/desktop/assets/settings/frog-puzzled.png` | MOVE | desktop |
| `build.rs` | `build.rs` | RETAIN | shared-infrastructure |
| `enablement/build_version.rs` | `enablement/build_version.rs` | RETAIN | shared-infrastructure |
| `capture/engine/.gitignore` | `capture/engine/.gitignore` | RETAIN | capture |
| `capture/engine/Cargo.lock` | `capture/engine/Cargo.lock` | RETAIN | capture |
| `capture/engine/Cargo.toml` | `capture/engine/Cargo.toml` | RETAIN | capture |
| `capture/engine/README.md` | `capture/engine/README.md` | RETAIN | capture |
| `capture/engine/src/finalizer.rs` | `capture/engine/src/finalizer.rs` | RETAIN | capture |
| `capture/engine/src/identity.rs` | `capture/engine/src/identity.rs` | RETAIN | capture |
| `capture/engine/src/lease.rs` | `capture/engine/src/lease.rs` | RETAIN | capture |
| `capture/engine/src/lib.rs` | `capture/engine/src/lib.rs` | RETAIN | capture |
| `capture/engine/src/locking.rs` | `capture/engine/src/locking.rs` | RETAIN | capture |
| `capture/engine/src/receipt.rs` | `capture/engine/src/receipt.rs` | RETAIN | capture |
| `capture/engine/src/secure.rs` | `capture/engine/src/secure.rs` | RETAIN | capture |
| `capture/engine/src/session.rs` | `capture/engine/src/session.rs` | RETAIN | capture |
| `capture/engine/tests/bootstrap.rs` | `capture/engine/tests/bootstrap.rs` | RETAIN | capture |
| `capture/engine/tests/finalizer.rs` | `capture/engine/tests/finalizer.rs` | RETAIN | capture |
| `capture/engine/tests/process.rs` | `capture/engine/tests/process.rs` | RETAIN | capture |
| `capture/engine/tests/review_regressions.rs` | `capture/engine/tests/review_regressions.rs` | RETAIN | capture |
| `connect/client/.gitignore` | `connect/client/.gitignore` | RETAIN | connect |
| `connect/client/Cargo.lock` | `connect/client/Cargo.lock` | RETAIN | connect |
| `connect/client/Cargo.toml` | `connect/client/Cargo.toml` | RETAIN | connect |
| `connect/client/src/lib.rs` | `connect/client/src/lib.rs` | RETAIN | connect |
| `connect/client/src/test_support.rs` | `connect/client/src/test_support.rs` | RETAIN | connect |
| `connect/web/.gitignore` | `connect/web/.gitignore` | RETAIN | connect |
| `connect/web/AGENTS.md` | `connect/web/AGENTS.md` | RETAIN | connect |
| `connect/web/README.md` | `connect/web/README.md` | RETAIN | connect |
| `connect/web/bun.lock` | `connect/web/bun.lock` | RETAIN | connect |
| `connect/web/index.html` | `connect/web/index.html` | RETAIN | connect |
| `connect/web/package.json` | `connect/web/package.json` | RETAIN | connect |
| `connect/web/scripts/assets.mjs` | `connect/web/scripts/assets.mjs` | RETAIN | connect |
| `connect/web/scripts/prerender.mjs` | `connect/web/scripts/prerender.mjs` | RETAIN | connect |
| `connect/web/scripts/test-browser.mjs` | `connect/web/scripts/test-browser.mjs` | RETAIN | connect |
| `connect/web/src/Hosted.tsx` | `connect/web/src/Hosted.tsx` | RETAIN | connect |
| `connect/web/src/SharedLinks.tsx` | `connect/web/src/SharedLinks.tsx` | RETAIN | connect |
| `connect/web/src/api.ts` | `connect/web/src/api.ts` | RETAIN | connect |
| `connect/web/src/base.css` | `connect/web/src/base.css` | RETAIN | connect |
| `connect/web/src/entry-server.tsx` | `connect/web/src/entry-server.tsx` | RETAIN | connect |
| `connect/web/src/hosted.css` | `connect/web/src/hosted.css` | RETAIN | connect |
| `connect/web/src/main.tsx` | `connect/web/src/main.tsx` | RETAIN | connect |
| `connect/web/test/artifact.test.mjs` | `connect/web/test/artifact.test.mjs` | RETAIN | connect |
| `connect/web/test/browser.mjs` | `connect/web/test/browser.mjs` | RETAIN | connect |
| `connect/web/test/closure.test.mjs` | `connect/web/test/closure.test.mjs` | RETAIN | connect |
| `connect/web/test/routes.test.mjs` | `connect/web/test/routes.test.mjs` | RETAIN | connect |
| `connect/web/tsconfig.json` | `connect/web/tsconfig.json` | RETAIN | connect |
| `connect/web/vite.config.ts` | `connect/web/vite.config.ts` | RETAIN | connect |
| `docs/acceptance/issue-171-sharing.md` | `connect/docs/acceptance/issue-171-sharing.md` | MOVE | connect |
| `agent-proof/docs/agent-video-workflow.md` | `capture/agent/docs/agent-video-workflow.md` | MOVE | capture-agent |
| `docs/capture.md` | `capture/engine/docs/capture.md` | MOVE | capture |
| `docs/continuous-delivery.md` | `enablement/docs/continuous-delivery.md` | MOVE | enablement |
| `docs/export-and-retention.md` | `capture/engine/docs/export-and-retention.md` | MOVE | capture |
| `docs/hosted-api.md` | `connect/docs/hosted-api.md` | MOVE | connect |
| `docs/hosted-client.md` | `connect/docs/hosted-client.md` | MOVE | connect |
| `docs/hosted-web.md` | `connect/docs/hosted-web.md` | MOVE | connect |
| `docs/iphone-and-routing.md` | `capture/engine/docs/iphone-and-routing.md` | MOVE | capture |
| `docs/keyframes-and-text.md` | `capture/engine/docs/keyframes-and-text.md` | MOVE | capture |
| `docs/linux-playback.md` | `capture/desktop/docs/linux/linux-playback.md` | MOVE | desktop |
| `docs/linux-updater-gui-test.md` | `capture/desktop/docs/linux/linux-updater-gui-test.md` | MOVE | desktop |
| `docs/linux.md` | `capture/desktop/docs/linux/linux.md` | MOVE | desktop |
| `docs/mac-signing.md` | `capture/desktop/docs/mac-signing.md` | MOVE | desktop |
| `docs/motion.md` | `capture/desktop/docs/motion.md` | MOVE | desktop |
| `docs/native-launch.md` | `capture/desktop/docs/native-launch.md` | MOVE | desktop |
| `docs/native-testing.md` | `capture/desktop/docs/native-testing.md` | MOVE | desktop |
| `docs/onboarding-and-settings.md` | `capture/desktop/docs/onboarding-and-settings.md` | MOVE | desktop |
| `docs/plans/issue-171-hosted-sharing.md` | `docs/plans/issue-171-hosted-sharing.md` | RETAIN | shared-infrastructure |
| `docs/recording-capsule-drag.md` | `capture/desktop/docs/recording-capsule-drag.md` | MOVE | desktop |
| `docs/release.md` | `capture/desktop/docs/release.md` | MOVE | desktop |
| `docs/routing.md` | `shared/docs/routing.md` | MOVE | shared-infrastructure |
| `docs/windows.md` | `capture/desktop/docs/legacy/windows.md` | MOVE | desktop (legacy unsupported code; retained, out of scope) |
| `helper/Export.swift` | `capture/engine/native/Export.swift` | MOVE | capture |
| `helper/Info.plist` | `shared/native/Info.plist` | MOVE | shared-infrastructure |
| `helper/LiveTranscribe.swift` | `capture/engine/native/LiveTranscribe.swift` | MOVE | capture |
| `helper/Media.swift` | `capture/engine/native/Media.swift` | MOVE | capture |
| `helper/Mirror.swift` | `capture/engine/native/Mirror.swift` | MOVE | capture |
| `helper/Onboarding.swift` | `capture/desktop/native/Onboarding.swift` | MOVE | desktop |
| `helper/OnboardingDemo.swift` | `capture/desktop/native/OnboardingDemo.swift` | MOVE | desktop |
| `helper/PermissionPanel.swift` | `capture/desktop/native/PermissionPanel.swift` | MOVE | desktop |
| `helper/PlaybackVolume.swift` | `capture/engine/native/PlaybackVolume.swift` | MOVE | capture |
| `helper/ScreenProbe.swift` | `capture/engine/native/ScreenProbe.swift` | MOVE | capture |
| `helper/Settings.swift` | `capture/desktop/native/Settings.swift` | MOVE | desktop |
| `helper/ShortcutKeys.swift` | `capture/desktop/native/ShortcutKeys.swift` | MOVE | desktop |
| `helper/main.swift` | `shared/native/main.swift` | MOVE | shared-infrastructure |
| `hosted/.gitignore` | `connect/service/.gitignore` | MOVE | connect |
| `hosted/README.md` | `connect/service/README.md` | MOVE | connect |
| `hosted/migrations/0001.sql` | `connect/service/migrations/0001.sql` | MOVE | connect |
| `hosted/migrations/0002.sql` | `connect/service/migrations/0002.sql` | MOVE | connect |
| `hosted/migrations/0003.sql` | `connect/service/migrations/0003.sql` | MOVE | connect |
| `hosted/package-lock.json` | `connect/service/package-lock.json` | MOVE | connect |
| `hosted/package.json` | `connect/service/package.json` | MOVE | connect |
| `hosted/src/auth.ts` | `connect/service/src/auth.ts` | MOVE | connect |
| `hosted/src/captures.ts` | `connect/service/src/captures.ts` | MOVE | connect |
| `hosted/src/common.ts` | `connect/service/src/common.ts` | MOVE | connect |
| `hosted/src/share-view.ts` | `connect/service/src/share-view.ts` | MOVE | connect |
| `hosted/src/shares.ts` | `connect/service/src/shares.ts` | MOVE | connect |
| `hosted/src/storage.ts` | `connect/service/src/storage.ts` | MOVE | connect |
| `hosted/src/types.ts` | `connect/service/src/types.ts` | MOVE | connect |
| `hosted/src/worker.ts` | `connect/service/src/worker.ts` | MOVE | connect |
| `hosted/test/api.test.mjs` | `connect/service/test/api.test.mjs` | MOVE | connect |
| `hosted/test/budget-safety.test.mjs` | `connect/service/test/budget-safety.test.mjs` | MOVE | connect |
| `hosted/test/budgets.test.mjs` | `connect/service/test/budgets.test.mjs` | MOVE | connect |
| `hosted/test/checksums.test.mjs` | `connect/service/test/checksums.test.mjs` | MOVE | connect |
| `hosted/test/eviction.test.mjs` | `connect/service/test/eviction.test.mjs` | MOVE | connect |
| `hosted/test/helpers.mjs` | `connect/service/test/helpers.mjs` | MOVE | connect |
| `hosted/test/lifecycle.test.mjs` | `connect/service/test/lifecycle.test.mjs` | MOVE | connect |
| `hosted/test/oauth.test.mjs` | `connect/service/test/oauth.test.mjs` | MOVE | connect |
| `hosted/test/security.test.mjs` | `connect/service/test/security.test.mjs` | MOVE | connect |
| `hosted/test/share-races.test.mjs` | `connect/service/test/share-races.test.mjs` | MOVE | connect |
| `hosted/test/share-view.test.mjs` | `connect/service/test/share-view.test.mjs` | MOVE | connect |
| `hosted/test/shares.test.mjs` | `connect/service/test/shares.test.mjs` | MOVE | connect |
| `hosted/test/spending-races.test.mjs` | `connect/service/test/spending-races.test.mjs` | MOVE | connect |
| `hosted/test/spending.test.mjs` | `connect/service/test/spending.test.mjs` | MOVE | connect |
| `hosted/test/stream-failure.test.mjs` | `connect/service/test/stream-failure.test.mjs` | MOVE | connect |
| `hosted/test/streaming.test.mjs` | `connect/service/test/streaming.test.mjs` | MOVE | connect |
| `hosted/test/types/contracts.ts` | `connect/service/test/types/contracts.ts` | MOVE | connect |
| `hosted/test/typescript.test.mjs` | `connect/service/test/typescript.test.mjs` | MOVE | connect |
| `hosted/tsconfig.json` | `connect/service/tsconfig.json` | MOVE | connect |
| `hosted/wrangler.toml` | `connect/service/wrangler.toml` | MOVE | connect |
| `packaging/linux/dev.kitlangton.apparent.desktop` | `capture/desktop/packaging/linux/dev.kitlangton.apparent.desktop` | MOVE | desktop |
| `packaging/linux/dev.kitlangton.apparent.png` | `capture/desktop/packaging/linux/dev.kitlangton.apparent.png` | MOVE | desktop |
| `perf/pill/clips.py` | `capture/desktop/scripts/perf/pill/clips.py` | MOVE | desktop |
| `perf/pill/motion-lint.config.json` | `capture/desktop/scripts/perf/pill/motion-lint.config.json` | MOVE | desktop |
| `perf/pill/record.sh` | `capture/desktop/scripts/perf/pill/record.sh` | MOVE | desktop |
| `perf/screen-guard.sh` | `capture/desktop/scripts/perf/screen-guard.sh` | MOVE | desktop |
| `enablement/refactor.md` | `enablement/refactor.md` | RETAIN | shared-infrastructure |
| `scripts/Apparent.entitlements` | `capture/desktop/scripts/packaging/Apparent.entitlements` | MOVE | desktop |
| `scripts/agent-eval.sh` | `capture/agent/scripts/agent-eval.sh` | MOVE | capture-agent |
| `scripts/apparent-restore-clipboard.py` | `capture/desktop/scripts/support/apparent-restore-clipboard.py` | MOVE | desktop |
| `scripts/autocrop-corpus.mjs` | `public/site/scripts/autocrop-corpus.mjs` | MOVE | public |
| `scripts/autocrop-gallery.mjs` | `public/site/scripts/autocrop-gallery.mjs` | MOVE | public |
| `scripts/bench-processing.ts` | `capture/engine/tools/bench-processing.ts` | MOVE | capture |
| `scripts/bench-start.ts` | `capture/desktop/scripts/support/bench-start.ts` | MOVE | desktop |
| `scripts/build-mac-preview.sh` | `capture/desktop/scripts/packaging/build-mac-preview.sh` | MOVE | desktop |
| `scripts/cd/connect_redirects.json` | `public/site/scripts/cd/connect_redirects.json` | MOVE | public |
| `scripts/cd/hosted_config.py` | `public/site/scripts/cd/hosted_config.py` | MOVE | public |
| `scripts/cd/publish_pages.py` | `public/site/scripts/cd/publish_pages.py` | MOVE | public |
| `scripts/cd/publish_site.py` | `public/site/scripts/cd/publish_site.py` | MOVE | public |
| `scripts/cd/test_hosted_delivery.py` | `public/site/scripts/cd/test_hosted_delivery.py` | MOVE | public |
| `scripts/cd/test_pages_delivery.py` | `public/site/scripts/cd/test_pages_delivery.py` | MOVE | public |
| `scripts/cd/test_publish_site.py` | `public/site/scripts/cd/test_publish_site.py` | MOVE | public |
| `scripts/cd/verify_worker.py` | `public/site/scripts/cd/verify_worker.py` | MOVE | public |
| `scripts/check-ca-roots.sh` | `capture/desktop/scripts/support/check-ca-roots.sh` | MOVE | desktop |
| `scripts/check-permission-panel-dock.sh` | `capture/desktop/scripts/support/check-permission-panel-dock.sh` | MOVE | desktop |
| `scripts/check-windows-cpu-tiers.ps1` | `scripts/check-windows-cpu-tiers.ps1` | RETAIN | desktop (legacy unsupported code; retained, out of scope) |
| `scripts/install-app-remote.sh` | `capture/desktop/scripts/packaging/install-app-remote.sh` | MOVE | desktop |
| `scripts/install-app.sh` | `capture/desktop/scripts/packaging/install-app.sh` | MOVE | desktop |
| `scripts/linux/appimage.sh` | `capture/desktop/scripts/linux/appimage/appimage.sh` | MOVE | desktop |
| `scripts/linux/build-playback.sh` | `capture/desktop/scripts/linux/maintenance/build-playback.sh` | MOVE | desktop |
| `scripts/linux/capture-smoke.py` | `capture/desktop/scripts/linux/gui/capture-smoke.py` | MOVE | desktop |
| `scripts/linux/check-appimage-decoder.sh` | `capture/desktop/scripts/linux/appimage/check-appimage-decoder.sh` | MOVE | desktop |
| `scripts/linux/check-appimage-encoder.sh` | `capture/desktop/scripts/linux/appimage/check-appimage-encoder.sh` | MOVE | desktop |
| `scripts/linux/check-cpu-tiers.py` | `capture/desktop/scripts/linux/appimage/check-cpu-tiers.py` | MOVE | desktop |
| `scripts/linux/dependencies.sh` | `capture/desktop/scripts/linux/appimage/dependencies.sh` | MOVE | desktop |
| `scripts/linux/gui-vm-reaper.py` | `capture/desktop/scripts/linux/gui/gui-vm-reaper.py` | MOVE | desktop |
| `scripts/linux/gui-vm.py` | `capture/desktop/scripts/linux/gui/gui-vm.py` | MOVE | desktop |
| `scripts/linux/migrate-scrawl.py` | `capture/desktop/scripts/linux/maintenance/migrate-scrawl.py` | MOVE | desktop |
| `scripts/linux/publish-main-preview.py` | `capture/desktop/scripts/linux/maintenance/publish-main-preview.py` | MOVE | desktop |
| `scripts/linux/smoke-cpu.sh` | `capture/desktop/scripts/linux/appimage/smoke-cpu.sh` | MOVE | desktop |
| `scripts/linux/test-capture-smoke.py` | `capture/desktop/scripts/linux/gui/test-capture-smoke.py` | MOVE | desktop |
| `scripts/linux/test-gui-vm.py` | `capture/desktop/scripts/linux/gui/test-gui-vm.py` | MOVE | desktop |
| `scripts/linux/test-native-launch.sh` | `capture/desktop/scripts/linux/maintenance/test-native-launch.sh` | MOVE | desktop |
| `scripts/linux/test-updater-gui.py` | `capture/desktop/scripts/linux/updater/test-updater-gui.py` | MOVE | desktop |
| `scripts/linux/test_migrate_scrawl.py` | `capture/desktop/scripts/linux/maintenance/test_migrate_scrawl.py` | MOVE | desktop |
| `scripts/linux/updater-fixture.py` | `capture/desktop/scripts/linux/updater/updater-fixture.py` | MOVE | desktop |
| `scripts/linux/updater-gui-scenario.sh` | `capture/desktop/scripts/linux/updater/updater-gui-scenario.sh` | MOVE | desktop |
| `scripts/linux/updater-gui.py` | `capture/desktop/scripts/linux/updater/updater-gui.py` | MOVE | desktop |
| `scripts/prepare-mac-signing.sh` | `capture/desktop/scripts/release/prepare-mac-signing.sh` | MOVE | desktop |
| `scripts/preview_appcast.py` | `capture/desktop/scripts/release/preview_appcast.py` | MOVE | desktop |
| `scripts/provision-narrator.py` | `capture/agent/scripts/provision-narrator.py` | MOVE | capture-agent |
| `scripts/release-app.sh` | `capture/desktop/scripts/release/release-app.sh` | MOVE | desktop |
| `scripts/setup-sparkle.sh` | `capture/desktop/scripts/release/setup-sparkle.sh` | MOVE | desktop |
| `scripts/tests/local_preview_update.py` | `capture/desktop/scripts/tests/local_preview_update.py` | MOVE | desktop |
| `scripts/tests/test_export_audio.py` | `capture/desktop/scripts/tests/test_export_audio.py` | MOVE | desktop |
| `scripts/tests/test_launch_registration.py` | `capture/desktop/scripts/tests/test_launch_registration.py` | MOVE | desktop |
| `scripts/tests/test_linux_preview_publication.py` | `capture/desktop/scripts/tests/test_linux_preview_publication.py` | MOVE | desktop |
| `scripts/tests/test_preview_appcast.py` | `capture/desktop/scripts/tests/test_preview_appcast.py` | MOVE | desktop |
| `scripts/tests/test_preview_apple_signing.py` | `capture/desktop/scripts/tests/test_preview_apple_signing.py` | MOVE | desktop |
| `scripts/tests/test_preview_release.py` | `capture/desktop/scripts/tests/test_preview_release.py` | MOVE | desktop |
| `scripts/tests/test_preview_sparkle_setup.py` | `capture/desktop/scripts/tests/test_preview_sparkle_setup.py` | MOVE | desktop |
| `scripts/tests/test_provision_narrator.py` | `capture/agent/scripts/tests/test_provision_narrator.py` | MOVE | capture-agent |
| `scripts/verification.py` | `enablement/verification.py` | MOVE | enablement |
| `scripts/verify-preview-sparkle-key.swift` | `capture/desktop/scripts/release/verify-preview-sparkle-key.swift` | MOVE | desktop |
| `scripts/virtual-display.swift` | `capture/desktop/scripts/support/virtual-display.swift` | MOVE | desktop |
| `site/.gitignore` | `public/site/.gitignore` | MOVE | public |
| `site/.wrangler/.metadata_never_index` | `public/site/.wrangler/.metadata_never_index` | MOVE | public |
| `site/bun.lock` | `public/site/bun.lock` | MOVE | public |
| `site/docs/apparent-demo-pi.md` | `public/site/docs/apparent-demo-pi.md` | MOVE | public |
| `site/docs/belief-release.md` | `public/site/docs/belief-release.md` | MOVE | public |
| `site/docs/clanker-deploy.md` | `public/site/docs/clanker-deploy.md` | MOVE | public |
| `site/docs/feature-grid.md` | `public/site/docs/feature-grid.md` | MOVE | public |
| `site/docs/footer-release.md` | `public/site/docs/footer-release.md` | MOVE | public |
| `site/docs/hosted-design.md` | `public/site/docs/hosted-design.md` | MOVE | public |
| `site/docs/pages-deploy.md` | `public/site/docs/pages-deploy.md` | MOVE | public |
| `site/docs/social-preview.md` | `public/site/docs/social-preview.md` | MOVE | public |
| `site/functions/Apparent-ghost-demo-v4-lilac.mp4.js` | `public/site/functions/Apparent-ghost-demo-v4-lilac.mp4.js` | MOVE | public |
| `site/index.html` | `public/site/index.html` | MOVE | public |
| `site/lib/pages-video-metadata.js` | `public/site/lib/pages-video-metadata.js` | MOVE | public |
| `site/package.json` | `public/site/package.json` | MOVE | public |
| `site/public/Apparent-ghost-demo-v4-lilac-poster.jpg` | `public/site/public/assets/media/Apparent-ghost-demo-v4-lilac-poster.jpg` | MOVE | public |
| `site/public/Apparent-ghost-demo-v4-lilac.mp4` | `public/site/public/assets/media/Apparent-ghost-demo-v4-lilac.mp4` | MOVE | public |
| `site/public/HostGrotesk-Variable.ttf` | `public/site/public/assets/fonts/HostGrotesk-Variable.ttf` | MOVE | public |
| `site/public/app-icon-64.png` | `public/site/public/assets/brand/app-icon-64.png` | MOVE | public |
| `site/public/app-icon.png` | `public/site/public/assets/brand/app-icon.png` | MOVE | public |
| `site/public/apparent-demo-pi-poster.jpg` | `public/site/public/assets/media/apparent-demo-pi-poster.jpg` | MOVE | public |
| `site/public/apparent-demo-pi.mp4` | `public/site/public/assets/media/apparent-demo-pi.mp4` | MOVE | public |
| `site/public/apparent-narration-iphone.mp3` | `public/site/public/assets/media/apparent-narration-iphone.mp3` | MOVE | public |
| `site/public/apparent-narration-mac.mp3` | `public/site/public/assets/media/apparent-narration-mac.mp3` | MOVE | public |
| `site/public/apparent-wordmark.svg` | `public/site/public/assets/brand/apparent-wordmark.svg` | MOVE | public |
| `site/public/appcast.xml` | `public/site/public/assets/meta/appcast.xml` | MOVE | public |
| `site/public/apple-touch-icon.png` | `public/site/public/assets/brand/apple-touch-icon.png` | MOVE | public |
| `site/public/favicon.ico` | `public/site/public/assets/brand/favicon.ico` | MOVE | public |
| `site/public/feature-demos/anywhere/index.html` | `public/site/public/assets/feature-demos/anywhere/index.html` | MOVE | public |
| `site/public/feature-demos/handoff/index.html` | `public/site/public/assets/feature-demos/handoff/index.html` | MOVE | public |
| `site/public/feature-demos/how-it-works/.local-media/ghost-after-aligned.png` | `public/site/public/assets/feature-demos/how-it-works/.local-media/ghost-after-aligned.png` | MOVE | public |
| `site/public/feature-demos/how-it-works/.local-media/ghost-before-aligned.png` | `public/site/public/assets/feature-demos/how-it-works/.local-media/ghost-before-aligned.png` | MOVE | public |
| `site/public/feature-demos/how-it-works/.local-media/whiteboard-start.jpg` | `public/site/public/assets/feature-demos/how-it-works/.local-media/whiteboard-start.jpg` | MOVE | public |
| `site/public/feature-demos/how-it-works/.local-media/whiteboard.mp4` | `public/site/public/assets/feature-demos/how-it-works/.local-media/whiteboard.mp4` | MOVE | public |
| `site/public/feature-demos/how-it-works/index.html` | `public/site/public/assets/feature-demos/how-it-works/index.html` | MOVE | public |
| `site/public/feature-demos/timeline/index.html` | `public/site/public/assets/feature-demos/timeline/index.html` | MOVE | public |
| `site/public/hosted/agent-pond-v4.png` | `public/site/public/assets/hosted/agent-pond-v4.png` | MOVE | public |
| `site/public/hosted/agent-reading-v4.png` | `public/site/public/assets/hosted/agent-reading-v4.png` | MOVE | public |
| `site/public/hosted/source-pond-v4.png` | `public/site/public/assets/hosted/source-pond-v4.png` | MOVE | public |
| `site/public/icon-192.png` | `public/site/public/assets/brand/icon-192.png` | MOVE | public |
| `site/public/icon-512.png` | `public/site/public/assets/brand/icon-512.png` | MOVE | public |
| `site/public/icon.svg` | `public/site/public/assets/brand/icon.svg` | MOVE | public |
| `site/public/menubar-icon@2x.png` | `public/site/public/assets/brand/menubar-icon@2x.png` | MOVE | public |
| `site/public/narration-iphone.mp3` | `public/site/public/assets/media/narration-iphone.mp3` | MOVE | public |
| `site/public/narration.mp3` | `public/site/public/assets/media/narration.mp3` | MOVE | public |
| `site/public/og.mp4` | `public/site/public/assets/media/og.mp4` | MOVE | public |
| `site/public/og.png` | `public/site/public/assets/media/og.png` | MOVE | public |
| `site/public/pond/belief/hug.png` | `public/site/public/assets/pond/belief/hug.png` | MOVE | public |
| `site/public/pond/belief/jump-air.png` | `public/site/public/assets/pond/belief/jump-air.png` | MOVE | public |
| `site/public/pond/belief/jump-rest.png` | `public/site/public/assets/pond/belief/jump-rest.png` | MOVE | public |
| `site/public/pond/belief/lily-clean.png` | `public/site/public/assets/pond/belief/lily-clean.png` | MOVE | public |
| `site/public/pond/belief/pair-left.png` | `public/site/public/assets/pond/belief/pair-left.png` | MOVE | public |
| `site/public/pond/belief/pair-right.png` | `public/site/public/assets/pond/belief/pair-right.png` | MOVE | public |
| `site/public/pond/footer-theatre.webp` | `public/site/public/assets/pond/footer-theatre.webp` | MOVE | public |
| `site/public/pond/frog-air-cohesive.png` | `public/site/public/assets/pond/frog-air-cohesive.png` | MOVE | public |
| `site/public/pond/frog-lounge-cohesive.png` | `public/site/public/assets/pond/frog-lounge-cohesive.png` | MOVE | public |
| `site/public/pond/frog-rest-cohesive.png` | `public/site/public/assets/pond/frog-rest-cohesive.png` | MOVE | public |
| `site/public/pond/frog-wave-cohesive.png` | `public/site/public/assets/pond/frog-wave-cohesive.png` | MOVE | public |
| `site/public/pond/lily-clean.png` | `public/site/public/assets/pond/lily-clean.png` | MOVE | public |
| `site/public/robots.txt` | `public/site/public/assets/meta/robots.txt` | MOVE | public |
| `site/public/site.webmanifest` | `public/site/public/assets/meta/site.webmanifest` | MOVE | public |
| `site/public/sitemap.xml` | `public/site/public/assets/meta/sitemap.xml` | MOVE | public |
| `site/public/social-preview-v1.png` | `public/site/public/assets/brand/social-preview-v1.png` | MOVE | public |
| `site/public/team/code-wordmark.svg` | `public/site/public/assets/team/code-wordmark.svg` | MOVE | public |
| `site/public/team/hpi-engine.svg` | `public/site/public/assets/team/hpi-engine.svg` | MOVE | public |
| `site/public/team/imago.png` | `public/site/public/assets/team/imago.png` | MOVE | public |
| `site/public/team/olivia.png` | `public/site/public/assets/team/olivia.png` | MOVE | public |
| `site/public/team/openclaw.svg` | `public/site/public/assets/team/openclaw.svg` | MOVE | public |
| `site/public/team/plug-and-play.png` | `public/site/public/assets/team/plug-and-play.png` | MOVE | public |
| `site/public/team/simon.png` | `public/site/public/assets/team/simon.png` | MOVE | public |
| `site/public/team/start-berlin.png` | `public/site/public/assets/team/start-berlin.png` | MOVE | public |
| `site/published-homepage.sha256.json` | `public/site/published-homepage.sha256.json` | MOVE | public |
| `site/published-homepage/apparent-demo-pi-poster.jpg` | `public/site/published-homepage/apparent-demo-pi-poster.jpg` | MOVE | public |
| `site/published-homepage/apparent-demo-pi.mp4` | `public/site/published-homepage/apparent-demo-pi.mp4` | MOVE | public |
| `site/published-homepage/apparent-narration-mac.mp3` | `public/site/published-homepage/apparent-narration-mac.mp3` | MOVE | public |
| `site/published-homepage/assets/approved-homepage-2a6286bad570af53.css` | `public/site/published-homepage/assets/approved-homepage-2a6286bad570af53.css` | MOVE | public |
| `site/published-homepage/assets/approved-homepage-6084dce1d273a9e1.js` | `public/site/published-homepage/assets/approved-homepage-6084dce1d273a9e1.js` | MOVE | public |
| `site/published-homepage/assets/approved-homepage-a9c35538a7afb605.css` | `public/site/published-homepage/assets/approved-homepage-a9c35538a7afb605.css` | MOVE | public |
| `site/published-homepage/assets/approved-homepage/olivia-dog-cutout.png` | `public/site/published-homepage/assets/approved-homepage/olivia-dog-cutout.png` | MOVE | public |
| `site/published-homepage/assets/approved-homepage/simon-cutout.png` | `public/site/published-homepage/assets/approved-homepage/simon-cutout.png` | MOVE | public |
| `site/published-homepage/assets/belief-009aebee421497d1.css` | `public/site/published-homepage/assets/belief-009aebee421497d1.css` | MOVE | public |
| `site/published-homepage/assets/belief-00faa7821b5a63b3.css` | `public/site/published-homepage/assets/belief-00faa7821b5a63b3.css` | MOVE | public |
| `site/published-homepage/assets/belief-e64a867c8be55ea3.js` | `public/site/published-homepage/assets/belief-e64a867c8be55ea3.js` | MOVE | public |
| `site/published-homepage/assets/brand-selection-96d56e0fb005975e.css` | `public/site/published-homepage/assets/brand-selection-96d56e0fb005975e.css` | MOVE | public |
| `site/published-homepage/assets/feature-grid-overlay-2210f1341833ef4d.js` | `public/site/published-homepage/assets/feature-grid-overlay-2210f1341833ef4d.js` | MOVE | public |
| `site/published-homepage/assets/feature-grid-overlay-2675e8b93ae89dba.css` | `public/site/published-homepage/assets/feature-grid-overlay-2675e8b93ae89dba.css` | MOVE | public |
| `site/published-homepage/assets/feature-grid-overlay-778c14c8999e73a4.js` | `public/site/published-homepage/assets/feature-grid-overlay-778c14c8999e73a4.js` | MOVE | public |
| `site/published-homepage/assets/feature-grid-overlay-eee6c19284c4a593.css` | `public/site/published-homepage/assets/feature-grid-overlay-eee6c19284c4a593.css` | MOVE | public |
| `site/published-homepage/assets/footer-83b82a238a00dcfa.css` | `public/site/published-homepage/assets/footer-83b82a238a00dcfa.css` | MOVE | public |
| `site/published-homepage/assets/footer-c3f2f7237f4c7b43.js` | `public/site/published-homepage/assets/footer-c3f2f7237f4c7b43.js` | MOVE | public |
| `site/published-homepage/assets/gpu-fx-B3EpXVJz.js` | `public/site/published-homepage/assets/gpu-fx-B3EpXVJz.js` | MOVE | public |
| `site/published-homepage/assets/gpu-fx-BaaaOVIX.js` | `public/site/published-homepage/assets/gpu-fx-BaaaOVIX.js` | MOVE | public |
| `site/published-homepage/assets/gpu-fx-CFfuFSu8.js` | `public/site/published-homepage/assets/gpu-fx-CFfuFSu8.js` | MOVE | public |
| `site/published-homepage/assets/gpu-fx-CS8qZKnC.js` | `public/site/published-homepage/assets/gpu-fx-CS8qZKnC.js` | MOVE | public |
| `site/published-homepage/assets/gpu-fx-CrjP7FD_.js` | `public/site/published-homepage/assets/gpu-fx-CrjP7FD_.js` | MOVE | public |
| `site/published-homepage/assets/gpu-fx-D7Mv7mxN.js` | `public/site/published-homepage/assets/gpu-fx-D7Mv7mxN.js` | MOVE | public |
| `site/published-homepage/assets/index-34700c5c8ab2cf56.js` | `public/site/published-homepage/assets/index-34700c5c8ab2cf56.js` | MOVE | public |
| `site/published-homepage/assets/index-538cc7536d6e57cf.js` | `public/site/published-homepage/assets/index-538cc7536d6e57cf.js` | MOVE | public |
| `site/published-homepage/assets/index-698f60bd9494b4da.js` | `public/site/published-homepage/assets/index-698f60bd9494b4da.js` | MOVE | public |
| `site/published-homepage/assets/index-8pgPj-1Q.css` | `public/site/published-homepage/assets/index-8pgPj-1Q.css` | MOVE | public |
| `site/published-homepage/assets/index-B0xlmBkz.js` | `public/site/published-homepage/assets/index-B0xlmBkz.js` | MOVE | public |
| `site/published-homepage/assets/index-BDbPKiAI.js` | `public/site/published-homepage/assets/index-BDbPKiAI.js` | MOVE | public |
| `site/published-homepage/assets/index-BJe9m6k_.css` | `public/site/published-homepage/assets/index-BJe9m6k_.css` | MOVE | public |
| `site/published-homepage/assets/index-BhX-Tny9.js` | `public/site/published-homepage/assets/index-BhX-Tny9.js` | MOVE | public |
| `site/published-homepage/assets/index-CEA7bWJ6.js` | `public/site/published-homepage/assets/index-CEA7bWJ6.js` | MOVE | public |
| `site/published-homepage/assets/index-Cggw_Ws7.js` | `public/site/published-homepage/assets/index-Cggw_Ws7.js` | MOVE | public |
| `site/published-homepage/assets/index-ChbF_rdn.css` | `public/site/published-homepage/assets/index-ChbF_rdn.css` | MOVE | public |
| `site/published-homepage/assets/index-Clv9Exu0.css` | `public/site/published-homepage/assets/index-Clv9Exu0.css` | MOVE | public |
| `site/published-homepage/assets/index-D-jL6J8D.css` | `public/site/published-homepage/assets/index-D-jL6J8D.css` | MOVE | public |
| `site/published-homepage/assets/index-D2cPC4Ch.css` | `public/site/published-homepage/assets/index-D2cPC4Ch.css` | MOVE | public |
| `site/published-homepage/assets/index-DQGXji3Z.js` | `public/site/published-homepage/assets/index-DQGXji3Z.js` | MOVE | public |
| `site/published-homepage/assets/index-DQxqAHDc.css` | `public/site/published-homepage/assets/index-DQxqAHDc.css` | MOVE | public |
| `site/published-homepage/assets/index-D_l_kdZG.js` | `public/site/published-homepage/assets/index-D_l_kdZG.js` | MOVE | public |
| `site/published-homepage/assets/index-DeXkKkRp.css` | `public/site/published-homepage/assets/index-DeXkKkRp.css` | MOVE | public |
| `site/published-homepage/assets/index-DebZEPv6.js` | `public/site/published-homepage/assets/index-DebZEPv6.js` | MOVE | public |
| `site/published-homepage/assets/index-DvqYEFD3.js` | `public/site/published-homepage/assets/index-DvqYEFD3.js` | MOVE | public |
| `site/published-homepage/assets/index-_OItVayx.css` | `public/site/published-homepage/assets/index-_OItVayx.css` | MOVE | public |
| `site/published-homepage/assets/index-cqD0szi5.js` | `public/site/published-homepage/assets/index-cqD0szi5.js` | MOVE | public |
| `site/published-homepage/assets/index-e53eeecce16b9bf0.js` | `public/site/published-homepage/assets/index-e53eeecce16b9bf0.js` | MOVE | public |
| `site/published-homepage/assets/index-no-team-094eee28482a4057.js` | `public/site/published-homepage/assets/index-no-team-094eee28482a4057.js` | MOVE | public |
| `site/published-homepage/assets/index-no-team-def1098d3e09c029.js` | `public/site/published-homepage/assets/index-no-team-def1098d3e09c029.js` | MOVE | public |
| `site/published-homepage/assets/index-thsY-TFv.js` | `public/site/published-homepage/assets/index-thsY-TFv.js` | MOVE | public |
| `site/published-homepage/assets/index-two-click-28e65a0d17ac785d.js` | `public/site/published-homepage/assets/index-two-click-28e65a0d17ac785d.js` | MOVE | public |
| `site/published-homepage/assets/try-button-346d06ceb54f7665.css` | `public/site/published-homepage/assets/try-button-346d06ceb54f7665.css` | MOVE | public |
| `site/published-homepage/index.html` | `public/site/published-homepage/index.html` | MOVE | public |
| `site/scripts/activate-belief-release.py` | `public/site/scripts/activate-belief-release.py` | MOVE | public |
| `site/scripts/agent-files.ts` | `public/site/scripts/agent-files.ts` | MOVE | public |
| `site/scripts/compose-connect.mjs` | `public/site/scripts/compose-connect.mjs` | MOVE | public site (Connect route composition preserved) |
| `site/scripts/expected-homepage-canonical.mjs` | `public/site/scripts/expected-homepage-canonical.mjs` | MOVE | public |
| `site/scripts/feature-audit.ts` | `public/site/scripts/feature-audit.ts` | MOVE | public |
| `site/scripts/generate-social-preview.mjs` | `public/site/scripts/generate-social-preview.mjs` | MOVE | public |
| `site/scripts/motion/README.md` | `public/site/scripts/motion/README.md` | MOVE | public |
| `site/scripts/motion/capture.ts` | `public/site/scripts/motion/capture.ts` | MOVE | public |
| `site/scripts/motion/clips.sh` | `public/site/scripts/motion/clips.sh` | MOVE | public |
| `site/scripts/motion/detect.ts` | `public/site/scripts/motion/detect.ts` | MOVE | public |
| `site/scripts/motion/hitches.ts` | `public/site/scripts/motion/hitches.ts` | MOVE | public |
| `site/scripts/og-video/apparent-fix.js` | `public/site/scripts/og-video/apparent-fix.js` | MOVE | public |
| `site/scripts/og-video/apparent-lasso.js` | `public/site/scripts/og-video/apparent-lasso.js` | MOVE | public |
| `site/scripts/og-video/apparent-pile.js` | `public/site/scripts/og-video/apparent-pile.js` | MOVE | public |
| `site/scripts/og-video/capture.mjs` | `public/site/scripts/og-video/capture.mjs` | MOVE | public |
| `site/scripts/og-video/common.js` | `public/site/scripts/og-video/common.js` | MOVE | public |
| `site/scripts/og-video/stage.html` | `public/site/scripts/og-video/stage.html` | MOVE | public |
| `site/scripts/package-apparent.mjs` | `public/site/scripts/package-apparent.mjs` | MOVE | public |
| `site/scripts/package-belief-release.mjs` | `public/site/scripts/package-belief-release.mjs` | MOVE | public |
| `site/scripts/package-feature-grid.mjs` | `public/site/scripts/package-feature-grid.mjs` | MOVE | public |
| `site/scripts/package-footer-release.mjs` | `public/site/scripts/package-footer-release.mjs` | MOVE | public |
| `site/scripts/package-page-text.mjs` | `public/site/scripts/package-page-text.mjs` | MOVE | public |
| `site/scripts/prepare-pages-video.mjs` | `public/site/scripts/prepare-pages-video.mjs` | MOVE | public |
| `site/scripts/prepare-pages.mjs` | `public/site/scripts/prepare-pages.mjs` | MOVE | public |
| `site/scripts/prerender.ts` | `public/site/scripts/prerender.ts` | MOVE | public |
| `site/scripts/test-belief-packaging.mjs` | `public/site/scripts/test-belief-packaging.mjs` | MOVE | public |
| `site/scripts/test-belief-pond.mjs` | `public/site/scripts/test-belief-pond.mjs` | MOVE | public |
| `site/scripts/test-belief-publication.py` | `public/site/scripts/test-belief-publication.py` | MOVE | public |
| `site/scripts/test-brand-favicon.mjs` | `public/site/scripts/test-brand-favicon.mjs` | MOVE | public |
| `site/scripts/test-brand-selection-browser.mjs` | `public/site/scripts/test-brand-selection-browser.mjs` | MOVE | public |
| `site/scripts/test-compose-connect.mjs` | `public/site/scripts/test-compose-connect.mjs` | MOVE | public |
| `site/scripts/test-expected-canonical-packaging.mjs` | `public/site/scripts/test-expected-canonical-packaging.mjs` | MOVE | public |
| `site/scripts/test-feature-grid.mjs` | `public/site/scripts/test-feature-grid.mjs` | MOVE | public |
| `site/scripts/test-footer-browser.mjs` | `public/site/scripts/test-footer-browser.mjs` | MOVE | public |
| `site/scripts/test-footer-packaging.mjs` | `public/site/scripts/test-footer-packaging.mjs` | MOVE | public |
| `site/scripts/test-github-stars-browser.mjs` | `public/site/scripts/test-github-stars-browser.mjs` | MOVE | public |
| `site/scripts/test-github-stars.mjs` | `public/site/scripts/test-github-stars.mjs` | MOVE | public |
| `site/scripts/test-hosted-browser.mjs` | `public/site/scripts/test-hosted-browser.mjs` | MOVE | public site (Connect route composition preserved) |
| `site/scripts/test-latest-download.mjs` | `public/site/scripts/test-latest-download.mjs` | MOVE | public |
| `site/scripts/test-og-stage-redirect.mjs` | `public/site/scripts/test-og-stage-redirect.mjs` | MOVE | public |
| `site/scripts/test-og-video-stage-browser.mjs` | `public/site/scripts/test-og-video-stage-browser.mjs` | MOVE | public |
| `site/scripts/test-package-feature-grid.mjs` | `public/site/scripts/test-package-feature-grid.mjs` | MOVE | public |
| `site/scripts/test-package-text.mjs` | `public/site/scripts/test-package-text.mjs` | MOVE | public |
| `site/scripts/test-pages-package.mjs` | `public/site/scripts/test-pages-package.mjs` | MOVE | public |
| `site/scripts/test-pages-video-runtime.mjs` | `public/site/scripts/test-pages-video-runtime.mjs` | MOVE | public |
| `site/scripts/test-pages-video.mjs` | `public/site/scripts/test-pages-video.mjs` | MOVE | public |
| `site/scripts/test-platform-os.mjs` | `public/site/scripts/test-platform-os.mjs` | MOVE | public |
| `site/scripts/test-public-homepage.mjs` | `public/site/scripts/test-public-homepage.mjs` | MOVE | public |
| `site/scripts/test-social-preview.mjs` | `public/site/scripts/test-social-preview.mjs` | MOVE | public |
| `site/scripts/test-try-button-browser.mjs` | `public/site/scripts/test-try-button-browser.mjs` | MOVE | public |
| `site/scripts/test-upstream-link-guard.mjs` | `public/site/scripts/test-upstream-link-guard.mjs` | MOVE | public |
| `site/scripts/test-video-demo.mjs` | `public/site/scripts/test-video-demo.mjs` | MOVE | public |
| `site/scripts/upstream-link-guard.mjs` | `public/site/scripts/upstream-link-guard.mjs` | MOVE | public |
| `site/scripts/verify-composed.mjs` | `public/site/scripts/verify-composed.mjs` | MOVE | public |
| `site/scripts/verify-page-density-browser.mjs` | `public/site/scripts/verify-page-density-browser.mjs` | MOVE | public |
| `site/scripts/verify-video-demo-browser.mjs` | `public/site/scripts/verify-video-demo-browser.mjs` | MOVE | public |
| `site/src/App.tsx` | `public/site/src/App.tsx` | MOVE | public |
| `site/src/BeliefPond.tsx` | `public/site/src/BeliefPond.tsx` | MOVE | public |
| `site/src/Brush3D.tsx` | `public/site/src/Brush3D.tsx` | MOVE | public |
| `site/src/Copy.tsx` | `public/site/src/Copy.tsx` | MOVE | public |
| `site/src/Download.tsx` | `public/site/src/Download.tsx` | MOVE | public |
| `site/src/Footer.tsx` | `public/site/src/Footer.tsx` | MOVE | public |
| `site/src/FrogPond.tsx` | `public/site/src/FrogPond.tsx` | MOVE | public |
| `site/src/GithubStars.tsx` | `public/site/src/GithubStars.tsx` | MOVE | public |
| `site/src/IPhone.tsx` | `public/site/src/IPhone.tsx` | MOVE | public |
| `site/src/LatestDownload.tsx` | `public/site/src/LatestDownload.tsx` | MOVE | public |
| `site/src/Team.tsx` | `public/site/src/Team.tsx` | MOVE | public |
| `site/src/appcast.ts` | `public/site/src/appcast.ts` | MOVE | public |
| `site/src/belief/behavior.ts` | `public/site/src/belief/behavior.ts` | MOVE | public |
| `site/src/belief/belief.css` | `public/site/src/belief/belief.css` | MOVE | public |
| `site/src/belief/markup.ts` | `public/site/src/belief/markup.ts` | MOVE | public |
| `site/src/brush-paths.ts` | `public/site/src/brush-paths.ts` | MOVE | public |
| `site/src/cta.css` | `public/site/src/cta.css` | MOVE | public |
| `site/src/demo/Demo.tsx` | `public/site/src/demo/Demo.tsx` | MOVE | public |
| `site/src/demo/VideoDemo.tsx` | `public/site/src/demo/VideoDemo.tsx` | MOVE | public |
| `site/src/demo/apps.tsx` | `public/site/src/demo/apps.tsx` | MOVE | public |
| `site/src/demo/gpu-fx.tsx` | `public/site/src/demo/gpu-fx.tsx` | MOVE | public |
| `site/src/demo/ink.tsx` | `public/site/src/demo/ink.tsx` | MOVE | public |
| `site/src/demo/measure.ts` | `public/site/src/demo/measure.ts` | MOVE | public |
| `site/src/demo/motion.ts` | `public/site/src/demo/motion.ts` | MOVE | public |
| `site/src/demo/script.ts` | `public/site/src/demo/script.ts` | MOVE | public |
| `site/src/demo/sfx.ts` | `public/site/src/demo/sfx.ts` | MOVE | public |
| `site/src/demo/stage-parts.tsx` | `public/site/src/demo/stage-parts.tsx` | MOVE | public |
| `site/src/demo/terminal.tsx` | `public/site/src/demo/terminal.tsx` | MOVE | public |
| `site/src/demo/video-demo.css` | `public/site/src/demo/video-demo.css` | MOVE | public |
| `site/src/entry-server.tsx` | `public/site/src/entry-server.tsx` | MOVE | public |
| `site/src/features.json` | `public/site/src/features.json` | MOVE | public |
| `site/src/features.ts` | `public/site/src/features.ts` | MOVE | public |
| `site/src/features/FeatureGrid.tsx` | `public/site/src/features/FeatureGrid.tsx` | MOVE | public |
| `site/src/features/feature-grid.css` | `public/site/src/features/feature-grid.css` | MOVE | public |
| `site/src/features/homepage-revision.css` | `public/site/src/features/homepage-revision.css` | MOVE | public |
| `site/src/features/homepage-revision.js` | `public/site/src/features/homepage-revision.js` | MOVE | public |
| `site/src/features/overlay-entry.tsx` | `public/site/src/features/overlay-entry.tsx` | MOVE | public |
| `site/src/footer/behavior.ts` | `public/site/src/footer/behavior.ts` | MOVE | public |
| `site/src/footer/footer.css` | `public/site/src/footer/footer.css` | MOVE | public |
| `site/src/footer/markup.ts` | `public/site/src/footer/markup.ts` | MOVE | public |
| `site/src/idle.ts` | `public/site/src/idle.ts` | MOVE | public |
| `site/src/latest-download.ts` | `public/site/src/latest-download.ts` | MOVE | public |
| `site/src/main.tsx` | `public/site/src/main.tsx` | MOVE | public |
| `site/src/map/FeatureMap.tsx` | `public/site/src/map/FeatureMap.tsx` | MOVE | public |
| `site/src/map/map.css` | `public/site/src/map/map.css` | MOVE | public |
| `site/src/map/page.tsx` | `public/site/src/map/page.tsx` | MOVE | public |
| `site/src/platform.ts` | `public/site/src/platform.ts` | MOVE | public |
| `site/src/pond-hero.css` | `public/site/src/pond-hero.css` | MOVE | public |
| `site/src/release.ts` | `public/site/src/release.ts` | MOVE | public |
| `site/src/styles.css` | `public/site/src/styles.css` | MOVE | public |
| `site/tsconfig.json` | `public/site/tsconfig.json` | MOVE | public |
| `site/vite.config.ts` | `public/site/vite.config.ts` | MOVE | public |
| `site/vite.feature-grid.config.ts` | `public/site/vite.feature-grid.config.ts` | MOVE | public |
| `site/worker.ts` | `public/site/worker.ts` | MOVE | public |
| `site/wrangler.jsonc` | `public/site/wrangler.jsonc` | MOVE | public |
| `site/wrangler.pages.jsonc` | `public/site/wrangler.pages.jsonc` | MOVE | public |
| `skills/apparent/SKILL.md` | `capture/agent/skills/apparent/SKILL.md` | MOVE | capture-agent |
| `skills/make-it-apparent/SKILL.md` | `capture/agent/skills/make-it-apparent/SKILL.md` | MOVE | capture-agent |
| `skills/make-it-apparent/references/solution-survey.md` | `capture/agent/skills/make-it-apparent/references/solution-survey.md` | MOVE | capture-agent |
| `src/agent_capture.rs` | `capture/desktop/workflow/agent_capture.rs` | MOVE | desktop |
| `src/agent_proof/context.rs` | `capture/agent/context.rs` | MOVE | capture-agent |
| `src/audio.rs` | `capture/engine/runtime/audio/audio.rs` | MOVE | capture |
| `src/autocrop-corpus.json` | `capture/engine/autocrop/autocrop-corpus.json` | MOVE | shared Rust module |
| `src/autocrop-playground.html` | `capture/engine/autocrop/autocrop-playground.html` | MOVE | shared Rust module |
| `src/autocrop.rs` | `capture/engine/autocrop/autocrop.rs` | MOVE | shared Rust module |
| `src/autocrop_lab.rs` | `capture/engine/autocrop/autocrop_lab.rs` | MOVE | shared Rust module |
| `src/ax.rs` | `capture/engine/accessibility/ax.rs` | MOVE | shared Rust module |
| `src/ax_other.rs` | `capture/engine/accessibility/ax_other.rs` | MOVE | shared Rust module |
| `src/bundle_processing.rs` | `capture/engine/bundle/bundle_processing.rs` | MOVE | shared Rust module |
| `src/capture.rs` | `capture/engine/runtime/mod.rs` | MOVE | capture |
| `capture/engine/runtime/platform/linux.rs` | `capture/engine/runtime/platform/linux.rs` | RETAIN | capture |
| `capture/engine/runtime/platform/macos.rs` | `capture/engine/runtime/platform/macos.rs` | RETAIN | capture |
| `capture/engine/runtime/recording/recorder.rs` | `capture/engine/runtime/recording/recorder.rs` | RETAIN | capture |
| `capture/engine/runtime/recording/tests_recorder.rs` | `capture/engine/runtime/recording/tests_recorder.rs` | RETAIN | capture |
| `capture/engine/runtime/platform/windows.rs` | `capture/engine/runtime/platform/windows.rs` | RETAIN | desktop (legacy unsupported code; retained, out of scope) |
| `src/command_line.rs` | `composition/command_line.rs` | MOVE | desktop |
| `src/command_transcriber.rs` | `capture/engine/runtime/audio/transcription/command_transcriber.rs` | MOVE | shared Rust module |
| `src/export/camera.rs` | `capture/engine/export/camera.rs` | MOVE | shared Rust module |
| `src/export/crop.rs` | `capture/engine/export/crop.rs` | MOVE | shared Rust module |
| `src/export/ink.rs` | `capture/engine/export/ink.rs` | MOVE | shared Rust module |
| `src/export/linux.rs` | `capture/engine/export/linux.rs` | MOVE | shared Rust module |
| `src/export/mod.rs` | `capture/engine/export/mod.rs` | MOVE | shared Rust module |
| `src/export/narration.rs` | `capture/engine/export/narration.rs` | MOVE | shared Rust module |
| `src/feature_map.rs` | `composition/feature_map.rs` | MOVE | shared Rust module |
| `src/handoff.rs` | `shared/handoff/mod.rs` | MOVE | desktop |
| `src/handoff/config_store.rs` | `shared/handoff/config_store.rs` | MOVE | shared Rust module |
| `src/handoff/test_modules.rs` | `shared/handoff/test_modules.rs` | MOVE | shared Rust module |
| `src/helper.rs` | `capture/desktop/helper.rs` | MOVE | desktop |
| `src/hex.rs` | `shared/hex.rs` | MOVE | shared Rust module |
| `src/hosted.rs` | `connect/hosted.rs` | MOVE | connect |
| `src/hosted_logging_tests.rs` | `connect/hosted/hosted_logging_tests.rs` | MOVE | connect |
| `src/hosted_tests.rs` | `connect/hosted/hosted_tests.rs` | MOVE | connect |
| `src/icons.rs` | `capture/desktop/icons.rs` | MOVE | desktop |
| `src/inbox.rs` | `capture/desktop/workflow/inbox.rs` | MOVE | desktop |
| `src/ingest_http.rs` | `capture/desktop/ingest_http.rs` | MOVE | shared Rust module |
| `src/ink_view.rs` | `capture/desktop/ink_view.rs` | MOVE | desktop |
| `src/keyframes.rs` | `capture/engine/keyframes.rs` | MOVE | shared Rust module |
| `src/laser_overlay.metal` | `capture/desktop/native/laser_overlay.metal` | MOVE | shared Rust module |
| `src/laser_overlay.rs` | `capture/desktop/native/laser_overlay.rs` | MOVE | shared Rust module |
| `src/launch_instance.rs` | `capture/desktop/workflow/launch_instance.rs` | MOVE | desktop |
| `src/launch_url.rs` | `capture/desktop/workflow/launch_url.rs` | MOVE | desktop |
| `src/linux/appearance.rs` | `capture/desktop/linux/appearance.rs` | MOVE | desktop |
| `src/linux/atspi.rs` | `capture/desktop/linux/atspi.rs` | MOVE | desktop |
| `src/linux/auto_paste.rs` | `capture/desktop/linux/auto_paste.rs` | MOVE | desktop |
| `src/linux/clipboard.rs` | `capture/desktop/linux/clipboard.rs` | MOVE | desktop |
| `src/linux/helper.rs` | `capture/desktop/linux/helper.rs` | MOVE | desktop |
| `src/linux/hyprland.rs` | `capture/desktop/linux/hyprland.rs` | MOVE | desktop |
| `src/linux/input.rs` | `capture/desktop/linux/input.rs` | MOVE | desktop |
| `src/linux/ipc.rs` | `capture/desktop/linux/ipc.rs` | MOVE | desktop |
| `src/linux/launch_handler.rs` | `capture/desktop/linux/launch_handler.rs` | MOVE | desktop |
| `src/linux/login.rs` | `capture/desktop/linux/login.rs` | MOVE | desktop |
| `src/linux/media.rs` | `capture/desktop/linux/media.rs` | MOVE | desktop |
| `src/linux/microphone.rs` | `capture/desktop/linux/microphone.rs` | MOVE | desktop |
| `src/linux/mod.rs` | `capture/desktop/linux/mod.rs` | MOVE | desktop |
| `src/linux/overlay.rs` | `capture/desktop/linux/overlay.rs` | MOVE | desktop |
| `src/linux/playback.rs` | `capture/desktop/linux/playback.rs` | MOVE | desktop |
| `src/linux/playback/audio-plugin.h` | `capture/desktop/linux/playback/audio-plugin.h` | MOVE | desktop |
| `src/linux/playback/helper.c` | `capture/desktop/linux/playback/helper.c` | MOVE | desktop |
| `src/linux/playback/lease.h` | `capture/desktop/linux/playback/lease.h` | MOVE | desktop |
| `src/linux/playback/plugin.c` | `capture/desktop/linux/playback/plugin.c` | MOVE | desktop |
| `src/linux/playback/policy.lua` | `capture/desktop/linux/playback/policy.lua` | MOVE | desktop |
| `src/linux/playback/test_native.c` | `capture/desktop/linux/playback/test_native.c` | MOVE | desktop |
| `src/linux/playback/test_policy.py` | `capture/desktop/linux/playback/test_policy.py` | MOVE | desktop |
| `src/linux/portal.rs` | `capture/desktop/linux/portal.rs` | MOVE | desktop |
| `src/linux/privacy.rs` | `capture/desktop/linux/privacy.rs` | MOVE | desktop |
| `src/linux/shell.rs` | `capture/desktop/linux/shell.rs` | MOVE | desktop |
| `src/linux/shortcuts.rs` | `capture/desktop/linux/shortcuts.rs` | MOVE | desktop |
| `src/linux/update.rs` | `capture/desktop/linux/update.rs` | MOVE | desktop |
| `src/linux/x11.rs` | `capture/desktop/linux/x11.rs` | MOVE | desktop |
| `src/live_transcribe.rs` | `capture/engine/runtime/audio/transcription/live_transcribe.rs` | MOVE | shared Rust module |
| `src/live_word_probe.rs` | `capture/engine/runtime/audio/transcription/live_word_probe.rs` | MOVE | shared Rust module |
| `src/live_word_warning.rs` | `capture/engine/runtime/audio/transcription/live_word_warning.rs` | MOVE | shared Rust module |
| `main.rs` | `main.rs` | RETAIN | desktop |
| `src/mic.rs` | `capture/engine/runtime/audio/mic.rs` | MOVE | capture |
| `src/model.rs` | `shared/model.rs` | MOVE | shared Rust module |
| `src/native/mod.rs` | `capture/desktop/native/mod.rs` | MOVE | desktop |
| `src/native/pill/atlas.rs` | `capture/desktop/native/pill/atlas.rs` | MOVE | desktop |
| `src/native/pill/mod.rs` | `capture/desktop/native/pill/mod.rs` | MOVE | desktop |
| `src/native/pill/model.rs` | `capture/desktop/native/pill/model.rs` | MOVE | desktop |
| `src/native/pill/other.rs` | `capture/desktop/native/pill/other.rs` | MOVE | desktop |
| `src/native/pill/pill.metal` | `capture/desktop/native/pill/pill.metal` | MOVE | desktop |
| `src/native/pill/types.rs` | `capture/desktop/native/pill/types.rs` | MOVE | desktop |
| `src/onboarding.rs` | `capture/desktop/settings/onboarding.rs` | MOVE | desktop |
| `src/onboarding_window.rs` | `capture/desktop/settings/onboarding_window.rs` | MOVE | desktop |
| `src/page.rs` | `capture/desktop/workflow/page.rs` | MOVE | desktop |
| `src/pause_transcribe.rs` | `capture/engine/runtime/audio/transcription/pause_transcribe.rs` | MOVE | shared Rust module |
| `src/phone.rs` | `capture/engine/runtime/phone.rs` | MOVE | capture |
| `src/playback.rs` | `capture/desktop/playback.rs` | MOVE | shared Rust module |
| `src/recover.rs` | `capture/engine/bundle/recover.rs` | MOVE | shared Rust module |
| `src/render.rs` | `capture/engine/render.rs` | MOVE | shared Rust module |
| `src/retention.rs` | `capture/engine/bundle/retention.rs` | MOVE | shared Rust module |
| `src/routing.rs` | `shared/routing.rs` | MOVE | shared Rust module |
| `src/routing/fixtures/artifact-hex-motion.json` | `capture/desktop/routing/fixtures/artifact-hex-motion.json` | MOVE | shared Rust module |
| `src/routing/fixtures/artifact-hex-shout2.json` | `capture/desktop/routing/fixtures/artifact-hex-shout2.json` | MOVE | shared Rust module |
| `src/routing/fixtures/artifact-lot.json` | `capture/desktop/routing/fixtures/artifact-lot.json` | MOVE | shared Rust module |
| `src/routing/fixtures/artifact-server-onboarding.json` | `capture/desktop/routing/fixtures/artifact-server-onboarding.json` | MOVE | shared Rust module |
| `src/routing/fixtures/dev-site-knobs.json` | `capture/desktop/routing/fixtures/dev-site-knobs.json` | MOVE | shared Rust module |
| `src/routing/fixtures/device-hub-ios-2.json` | `capture/desktop/routing/fixtures/device-hub-ios-2.json` | MOVE | shared Rust module |
| `src/routing/fixtures/device-hub-ios.json` | `capture/desktop/routing/fixtures/device-hub-ios.json` | MOVE | shared Rust module |
| `src/routing/fixtures/opencode-interface.json` | `capture/desktop/routing/fixtures/opencode-interface.json` | MOVE | shared Rust module |
| `src/routing/look.rs` | `capture/desktop/routing/look.rs` | MOVE | shared Rust module |
| `src/routing/served.rs` | `capture/desktop/routing/served.rs` | MOVE | shared Rust module |
| `src/routing/smart.rs` | `capture/desktop/routing/smart.rs` | MOVE | shared Rust module |
| `src/screens.rs` | `capture/engine/runtime/screens.rs` | MOVE | capture |
| `src/sdk.js` | `capture/desktop/sdk.js` | MOVE | desktop |
| `src/settings.rs` | `capture/desktop/settings/settings.rs` | MOVE | desktop |
| `src/settings_window.rs` | `capture/desktop/settings/settings_window.rs` | MOVE | desktop |
| `src/shortcut.rs` | `capture/desktop/settings/shortcut.rs` | MOVE | desktop |
| `src/sketch.rs` | `capture/engine/sketch.rs` | MOVE | shared Rust module |
| `src/sparkle.rs` | `capture/desktop/sparkle.rs` | MOVE | shared Rust module |
| `src/speech.rs` | `capture/engine/runtime/audio/transcription/speech.rs` | MOVE | shared Rust module |
| `src/speech_model.rs` | `capture/engine/runtime/audio/transcription/speech_model.rs` | MOVE | shared Rust module |
| `src/transcribe.rs` | `capture/engine/runtime/audio/transcription/transcribe.rs` | MOVE | shared Rust module |
| `src/ui.rs` | `capture/desktop/mod.rs` | MOVE | desktop |
| `src/ui/desktop_completion.rs` | `capture/desktop/ui/desktop_completion.rs` | MOVE | desktop |
| `src/ui/lab.rs` | `capture/desktop/ui/lab.rs` | MOVE | desktop |
| `src/ui/linux.rs` | `capture/desktop/ui/linux.rs` | MOVE | desktop |
| `src/ui/macos.rs` | `capture/desktop/ui/macos.rs` | MOVE | desktop |
| `src/ui/motion.rs` | `capture/desktop/ui/motion.rs` | MOVE | desktop |
| `src/ui/pill_theme.rs` | `capture/desktop/ui/pill_theme.rs` | MOVE | desktop |
| `src/ui/recording_completion.rs` | `capture/desktop/ui/recording_completion.rs` | MOVE | desktop |
| `src/ui/whiteboard.rs` | `capture/desktop/ui/whiteboard.rs` | MOVE | desktop |
| `src/ui/windows.rs` | `capture/desktop/ui/windows.rs` | MOVE | desktop (legacy unsupported code; retained, out of scope) |
| `src/warm_recognizer.rs` | `capture/engine/runtime/audio/transcription/warm_recognizer.rs` | MOVE | shared Rust module |
| `src/web.rs` | `capture/desktop/web.rs` | MOVE | desktop |
| `capture/engine/runtime/platform/legacy_windows/platform/clipboard.rs` | `capture/engine/runtime/platform/legacy_windows/platform/clipboard.rs` | RETAIN | desktop (legacy unsupported code; retained, out of scope) |
| `capture/engine/runtime/platform/legacy_windows/platform/command_line.rs` | `capture/engine/runtime/platform/legacy_windows/platform/command_line.rs` | RETAIN | desktop (legacy unsupported code; retained, out of scope) |
| `capture/engine/runtime/platform/legacy_windows/capture/helper.rs` | `capture/engine/runtime/platform/legacy_windows/capture/helper.rs` | RETAIN | desktop (legacy unsupported code; retained, out of scope) |
| `capture/engine/runtime/platform/legacy_windows/platform/login.rs` | `capture/engine/runtime/platform/legacy_windows/platform/login.rs` | RETAIN | desktop (legacy unsupported code; retained, out of scope) |
| `capture/engine/runtime/platform/legacy_windows/capture/media.rs` | `capture/engine/runtime/platform/legacy_windows/capture/media.rs` | RETAIN | desktop (legacy unsupported code; retained, out of scope) |
| `capture/engine/runtime/platform/legacy_windows/mod.rs` | `capture/engine/runtime/platform/legacy_windows/mod.rs` | RETAIN | desktop (legacy unsupported code; retained, out of scope) |
| `capture/engine/runtime/platform/legacy_windows/capture/overlay.rs` | `capture/engine/runtime/platform/legacy_windows/capture/overlay.rs` | RETAIN | desktop (legacy unsupported code; retained, out of scope) |
| `capture/engine/runtime/platform/legacy_windows/capture/playback.rs` | `capture/engine/runtime/platform/legacy_windows/capture/playback.rs` | RETAIN | desktop (legacy unsupported code; retained, out of scope) |
| `capture/engine/runtime/platform/legacy_windows/platform/privacy.rs` | `capture/engine/runtime/platform/legacy_windows/platform/privacy.rs` | RETAIN | desktop (legacy unsupported code; retained, out of scope) |
| `capture/engine/runtime/platform/legacy_windows/platform/shell.rs` | `capture/engine/runtime/platform/legacy_windows/platform/shell.rs` | RETAIN | desktop (legacy unsupported code; retained, out of scope) |
| `capture/engine/runtime/platform/legacy_windows/capture/uia.rs` | `capture/engine/runtime/platform/legacy_windows/capture/uia.rs` | RETAIN | desktop (legacy unsupported code; retained, out of scope) |
| `src/windows.rs` | `shared/windows.rs` | MOVE | shared Rust module |
| `src/agent_proof/zoom.rs` | `capture/agent/zoom.rs` | MOVE | capture-agent |
| `tests/test_pre_push_guard.py` | `enablement/tests/test_pre_push_guard.py` | MOVE | enablement |
| `tests/test_verification.py` | `enablement/tests/test_verification.py` | MOVE | enablement |
| `tools/narrator/Cargo.lock` | `capture/agent/narrator/Cargo.lock` | MOVE | capture-agent |
| `tools/narrator/Cargo.toml` | `capture/agent/narrator/Cargo.toml` | MOVE | capture-agent |
| `tools/narrator/README.md` | `capture/agent/narrator/README.md` | MOVE | capture-agent |
| `tools/narrator/src/lib.rs` | `capture/agent/narrator/src/lib.rs` | MOVE | capture-agent |
| `tools/narrator/main.rs` | `capture/agent/narrator/main.rs` | MOVE | capture-agent |
| `tools/narrator/tests/protocol.rs` | `capture/agent/narrator/tests/protocol.rs` | MOVE | capture-agent |
| `video/.gitignore` | `public/site/video-demo/.gitignore` | MOVE | public |
| `video/Cargo.toml` | `public/site/video-demo/Cargo.toml` | MOVE | public |
| `video/media/DMSans-Medium.ttf` | `public/site/video-demo/media/DMSans-Medium.ttf` | MOVE | public |
| `video/media/Inter-Bold.ttf` | `public/site/video-demo/media/Inter-Bold.ttf` | MOVE | public |
| `video/media/JetBrainsMono-Regular.ttf` | `public/site/video-demo/media/JetBrainsMono-Regular.ttf` | MOVE | public |
| `video/media/app_icon.png` | `public/site/video-demo/media/app_icon.png` | MOVE | public |
| `video/media/contact_sheet.png` | `public/site/video-demo/media/contact_sheet.png` | MOVE | public |
| `video/media/soundtrack.wav` | `public/site/video-demo/media/soundtrack.wav` | MOVE | public |
| `video/src/lib.rs` | `public/site/video-demo/src/lib.rs` | MOVE | public |
| `video/main.rs` | `public/site/video-demo/main.rs` | MOVE | public |

Ledger invariant at this reconciliation: 634 unique baseline inputs and unique final destinations; **525 MOVE / 109 RETAIN** by exact old-path/final-path comparison. All 634 destinations exist in the current working tree; all 525 MOVE donors are absent; all 109 RETAIN paths exist with owner and concrete responsibility. No duplicate destinations, deletes, or missing destinations. The current checkout has 644 physical files: 602 tracked files present plus 42 untracked moved destinations; the committed HEAD tree has 644 tracked paths, including donor paths removed only in this dirty worktree. Ten new module files are listed separately in the private inventory, outside baseline rows. The 39 moved asset files and three moved embedded inputs are byte-identical to their baseline Git blobs. Current work remains unpublished and integration in progress pending required CI and independent review.

## Final physical reconciliation and verification status

The full baseline-to-current tree and path-level checks are recorded in `/home/simon/.hermes/plans/apparent-structure-final-ledger-result.md`; the machine-readable inventory keeps the original 634 `old_path` values unchanged and records 634 unique destinations, the 644-file current working tree, all ten additional module paths, and per-file asset/embedded byte hashes. The Connect test destinations are nested under `src/connect/hosted/`; the eight routing fixture destinations are `src/desktop/routing/fixtures/`; the seven playback files were already at `src/desktop/linux/playback/` and were not moved again. The full inventory has no pending destination, donor, or retain exception.

The current physical changes are still dirty/unpublished. Remote `origin/main` was `4927502ab1cdcf311725a74eaaff38cb6cee875d`. Parent-reported normal hook passed. Mac CI run `41298031752` failed on a missing import; a local import fix is present in the dirty integration, but it has not been rerun and is unverified. The optional native playback proof in `/home/simon/.hermes/plans/apparent-structure-playback-proof.md` passed 13 policy cases and native descriptor-safety tests using the source-header route; it is not Mac CI, GUI/audio playback, or full campaign acceptance. Do not mark this campaign complete or claim final remote publication. Independent review and passing required CI remain explicit gates.

## Integrated external wiring (owned follow-up)

The site and service move handoffs are integrated in this batch: website build/package/publish inputs now root at `public/site/**`; the Worker package and generated Wrangler config use `connect/service/**`; `connect/web/**` approved-assets/docs references point into `public/site/public/**`. Rust feature-map/watch fixtures and `capture/desktop/settings/onboarding.rs` compile-time native inputs point at the current destinations. CI gates and deployment identities/routes remain unchanged; selector unknown-path behavior still broadens. The private inventory records the handoff-resolved native paths. Public owner reports 237 moved files, 132 checks and artifact verification; service owner reports 35 moved files and 71 tests; native owner reports 13 source hashes and preserved sorted compiler ordering. Those are retained owner receipts, not reruns here. Mac native compilation is still pending.

## Verification policy for this campaign

Preservation proof and `git diff --check` apply to each physical slice. Current first-slice build and focused CLI, bundle-processing, and feature-map tests ran through `t3-build`; their exact logs are retained under `/srv/clanker-hermes/cache/scratch/apparent-first-slice-*.log`. The leaf lanes are terminal and native inputs are frozen. This batch runs only inexpensive Python selector/path tests before the integrated Linux root compilation through `t3-build`; no broad Rust tests or deployment are included. The parent normal verifier and Mac native compilation remain integration gates. No commit or push is part of this handoff.

---

## Historical target and campaign evidence (superseded; not live work)

The following text records the earlier package-boundary proposal and prior milestone. It is retained for provenance only. Where it conflicts with the authoritative target above—including extra crates, standalone Agent/Engine adoption, different temporary module locations, or owner maps—the target above wins.

## Target and dependency rules

Optimize for ordinary features and recovery staying with one accountable owner,
not a perfect directory tree. These are ownership targets, not invented packages:

| Owner | Owns | Must not own |
|---|---|---|
| Capture Engine | Recording mechanics and OS adapters; sessions/resources; actual-frame readiness; stop/cancel; processing, bundles and archive safety | Desktop UI, clipboard/paste, onboarding, upload-consent or login policy |
| Capture Desktop | Human GPUI workflow: shortcuts, picker, ink, controls, review, Settings, onboarding and local handoff | A second recorder/finalizer or Connect auth/storage internals |
| Capture Agent | Automation, explicit owned requests, deadlines, cancellation, machine-readable receipts and evidence export | Dependence on a Desktop installation, GPUI event loop or hidden Desktop helper fallback |
| Connect | Authorized client transfer, accounts/devices, service/storage/auth and authenticated browser UI | Calling product's consent or presentation decisions |
| Public | Marketing, explanation, brand and download presentation | Authenticated application implementation or private pitch/capture artifacts |

Desktop and Agent consume supported Engine and Connect contracts. Engine never
imports either product. Connect accepts authorized work; the caller retains
consent/presentation policy. Public may link to Connect, not import its authenticated
implementation. Thin composition assembles owner outputs; it does not absorb policy.
Keep product behavior shared across macOS/Linux. Adapters provide native mechanics
and explicit capabilities. No new Windows support, web Desktop migration, new daemon/IPC,
service, database or repository is part of this campaign. Bounded removal of verified
obsolete Windows code is explicitly authorized under the retirement rule below.
Keep GPUI working.

Agent's final target is independently installed recording on clean macOS and Linux
machines where Desktop has **never** been installed. A graphical session and OS
permissions are legitimate prerequisites. A second root binary, GUI remote control,
injected worker, package build or headless `--help` is not that acceptance.

## Authority and supported halfway state

Current main source and merged PR acceptance are implementation authority. This
accepted campaign and AGENTS.md set future scope and verification policy. Subsystem
docs describe existing contracts; reconcile stale statements against source and
later explicit decisions. Historical threads, staging plans and passing old logs
are evidence, not permission to revive their larger scope or claim main adoption.

Main currently remains a Rust binary with GPUI/native dependencies. The Engine
package is real but unconsumed by production. Connect browser entry isolation and
an independent additive browser package are real; `site/` still owns the production
build/publisher and authenticated implementation. Existing GUI automation still requires
the running Desktop app. These are supported migration states, not failures to hide.

Move one authoritative implementation and its tests at a time. Existing paths may
forward **toward** the new owner through direct re-exports or thin adapters; never
maintain two behavioral implementations. New work uses the accepted seam immediately.
Freeze the donor against new responsibilities, allowing bounded bug repairs when
needed. Record each facade's owner, consumers and deletion condition in its slice:
remove it when all supported callers/build inputs/docs have migrated and compatibility
checks pass. Keep only aliases required by an actual external contract. Do not widen
legacy imports to make a new feature easy, or freeze unrelated feature delivery.

### Retire before moving

After this strategy merges, workers may delete verified-obsolete tracked code, docs
and assets in bounded coherent slices without asking for each deletion. Verify
requiredness first: trace consumers/references, current build/runtime/package inputs,
supported compatibility and historical-data readers, and active owner reservations.
Delete retired implementations with obsolete tests/references instead of migrating or
archive-copying them; git history is the archive. Candidate examples include retired
OpenCode routing glossary/`CONTEXT.md`, old icons and legacy sites, **only after that
check**. “Not deployed” alone is not obsolete. This also authorizes bounded legacy
Windows removal, never new Windows support. Preserve active work, supported readers
and current inputs until verified retired. Private captures, credentials, evidence
and uncommitted work are outside cleanup authority. This PR changes policy only;
no bulk application/site deletion starts before the parent merges the strategy.

Parallel lanes own useful outcomes and their tests/PRs, not arbitrary files. One writer
owns each shared hotspot/module declaration/manifest. Use short-lived main-targeted
PRs, not the #199 umbrella as an integration destination. Branch from a committed
provider only when necessary; integrate accepted prerequisites into main promptly.
Application edits remain limited to assigned slices and exclusive owner reservations.

## Verification policy for this campaign

Mechanical means behavior and runtime/build inputs are preserved, aside from explained
module/import/visibility wiring. Require inverse reconstruction or precise moved-body
comparison, test-ID mapping, import/dependency review, affected macOS/Linux compilation
and focused existing tests. Reuse exact relevant historical evidence after comparing
source and embedded/native/build inputs; name its revision, limits and reason for reuse.
Do not repeat a full suite, review or native session merely because a SHA changed.

A move mixed with a fix is a behavior change. Shared inputs, build/package changes,
security/privacy/consent, recording lifecycle/readiness/stop/cancel, resource ownership,
publication and uncertain impact broaden checks. Use focused regressions plus broad
integration/shared-build checkpoints; batch relevant native proof at material behavior
boundaries. Never credit Linux compilation as Mac/GUI proof, waive a red test, or turn
an unavailable check into green. Keep synthetic media/auth evidence distinct from real
OS capture and live OAuth. Follow AGENTS.md's failure repair and platform safety rules.

This explicitly replaces the old blanket full-suite/native/bundle-inspection demand
for every mechanical move, including old PR checklists. It does not erase actual
unresolved compile/test failures. Reconcile #221/#225 gate descriptions with their
current delta and retained proof before acceptance; do not relabel old native work.

`./.githooks/pre-push` retains clean exact-HEAD, hidden-index and final-tree guards.
`enablement/verification.py` selects from the complete main-merge-base delta, not the last
commit or last push. Missing history, an empty/unknown delta, changed shared/build/
security/lifecycle behavior and selector/hook/CI edits broaden to root plus Engine build/tests.
Document-only changes run hook/selector contracts without Cargo. Engine-test-only
changes build/test Engine without the root app. Rename/delete selection includes
both donor and destination. `.github/workflows/verification.yml` uses the same selector
on PR base/main-before deltas, on macOS and Linux, with no secrets or publication.
Existing website/Connect, updater and release workflows remain intact.

The follow-through selector recognizes the exact reviewed config #221 (`1ea0d146`)
and recorder #225 (`28d83f3d`), plus processing `84ef8f56`, source pairs using complete Git blob identities and
regular-file modes, including donor bodies, destination imports/re-exports and tests.
Only those complete deltas (alone, combined or with narrow docs/package tests) select
root compilation plus existing owning test sets: handoff/hosted/Settings for config,
capture/agent_capture for recorder. The pinned config fixture repair is included.
Merged #246 adds the exact current-main config three-file recipe (logging repair
already present) and processing recipe `d59796c3` → `f135448c`; historical recipes
remain revision-bounded. No current config extraction acceptance follows from routing.
Processing runs bundle_processing/capture/recover/sketch; its complete production
block and all ten artifact test bodies were compared byte-for-byte to `f1b46d2a`.
Each filter must execute passing tests; a zero-match run fails. Any altered body,
missing test, mode change, shared/native/build change or unknown extra path restores
full root/Engine checks. These are source-pair gates, not commit-message, label,
environment or user-selected bypasses. New donor bytes require review, not a guessed
"mechanical" classification.

The known `connect/web` package inventory from #200, checked at 5b checkpoint
`c5bb1b78`, selects its independent frozen Bun 1.3.10 install, production build,
artifact/resource-closure and dev/preview route fixtures, and separate synthetic browser
build/fixtures, without Rust. Its current closure reads only the five approved
`site/public` assets outside the package; changes there or in publisher/service/shared
inputs still broaden. CI provisions Node/Chrome for that route. This is executable
routing for the additive package, now merged through #244; historical package
acceptance is recorded in 5b below. It is not production adoption. Unknown new
browser paths remain broad. #246 admits only the exact additional
`connect/web/src/SharedLinks.tsx` filename; unknown, shared-input and unreviewed mixed
deltas still broaden rather than acquiring a generic browser/Rust exemption.

Do not certify arbitrary `main.rs` edits: that file composes every subsystem. A
later extraction revision must be source-compared and its exact blobs reviewed
before replacing the pinned processing pair; the registry is deliberately bounded.
Routing #241 alone did not complete any checklist step beyond 1. This remains
a small selector, not a Rust parser, broad test framework or native harness.

On Clanker run all new heavy builds/tests and hook-backed pushes once through
`/home/simon/.local/bin/t3-build`; never nest it. Lightweight selector/shell fixtures
may run directly. Keep full logs and receipts outside git. Commit coherent checkpoints
before long verification, then finish the commands and PR publication yourself.
After 30 minutes without a credible path, preserve/checkpoint, record blocker and
resume condition, and take the next independent step. Do not start an infrastructure
project to unstick a move. Coordinator checks disk every two hours; at 90% or higher,
clean only verified idle regenerable artifacts, never active work or retained proof.

## Completed on actual main (bounded acceptance)

Checked boxes below certify only the named merged slice, not the complete target.
PR links use the canonical repository; main ancestry was checked at the pinned base.

- [x] **Staged Engine admission/lease package** — [#220](https://github.com/tryapparent/apparent/pull/220), merge `77dd5bbf`. Independent package/process contract accepted; 22 package tests and retained source-identical Mac package evidence. Root combined Linux evidence reported 668 passed/3 ignored with #222. No production recorder, finalizer or retention caller consumes Engine; no native capture acceptance is implied. See `capture/engine/README.md`.
- [x] **Connect entry isolation** — [#222](https://github.com/tryapparent/apparent/pull/222), merge `d21115b5`. Canonical/legacy routes, built dependency closure and synthetic packaged/dev/preview auth/consent/browser checks retained. `site/` still owns build/package/publisher; no independent Connect package or live OAuth claim. See `connect/docs/hosted-web.md`.
- [x] **Recording versus Desktop completion seam** — [#233](https://github.com/tryapparent/apparent/pull/233), implementation `b6c3fd39478c357f6f79b3a5d6e2199bb02707c1`, merge `92b76040`. Linux UI selection 51 passed; full suite 668 passed/3 ignored; build and independent source review passed. Native/GUI/E2E intentionally omitted, not claimed. `capture/desktop/ui/recording_completion.rs` still uses the crate-root finalizer; whiteboard and GPUI coupling remain. No Engine adoption or standalone Agent.
- [x] **Bounded verification-routing follow-through** — [#241](https://github.com/tryapparent/apparent/pull/241), tested head `c3f72f7a`, merge `1ca21c67`. Exact source-pair and additive browser rules above are merged, not extraction acceptance. Retained normal-hook evidence reports Linux root build/671 passed/3 ignored, Engine build/22 passed and 15 hook + 13 selector contracts; final-head CI checks passed. The real browser driver receipt at `c5bb1b78` is historical synthetic evidence, not a fresh current-main run.
- [x] **Current source-pair/inventory repair** — [#246](https://github.com/tryapparent/apparent/pull/246), tested `e73bb366455a3156daa3f5b423e26fdb103cf894`, merge `76792fa186cf1f3ad88348596ac360f119de6706`. Parent inspected the terminal independent Luna review: no findings. Parent normal-hook proof: root build/683 passed/3 ignored, full Engine suite, 15 hook + 16 selector contracts. Retained final CI [41281996167](https://github.com/tryapparent/apparent/actions/runs/41281996167) passed Ubuntu 683/Mac 607 root tests (3 ignored each), with Engine checks. No new Rust CodeQL analysis was configured for this run; earlier #242/#245 Rust analyses remain historical evidence only. This accepts selector repair, not config, browser adoption or native behavior.
- [x] **Retired OpenCode glossary deletion (one C slice)** — [#243](https://github.com/tryapparent/apparent/pull/243), tested head `ea3ddd38`, merge `3b5d715b`. Deleted `CONTEXT.md` and its sole direct guide reference in `shared/docs/routing.md`; no source, supported historical metadata reader, runtime or package input changed. Current-main tracked references are only the intentional campaign example and selector/deletion regression. Retained final normal hook passed 15 guard + 13 selector tests on the exact docs-only rule; CI Rust execution was skipped, not a Rust pass. The earlier broad invocation's Engine trusted-ancestry failure remains a failed historical invocation, not waived or relabelled green. C and the final sweep remain unchecked.

## Ordered remaining checklist

An unchecked box stays unchecked until implementation is merged **and** the stated
acceptance is met. Record merged PR, tested revision, reused/fresh results and remaining
limits in the same PR where possible. Strategy acceptance is recorded below; no
other pending item becomes complete merely because a worker finishes it.

- [x] **1. Strategy and verification tooling.** Merged #239 as `f1b46d2a62145abae6f93a99d29de9da023e675b`. All ten required checks passed, including CodeQL's five language analyses/aggregate, harness contracts, selector, and macOS/Linux Rust jobs. Final Actions run [41256544899](https://github.com/tryapparent/apparent/actions/runs/41256544899) verified root builds/full suites (Linux 671 passed/3 ignored; Mac 595 passed/3 ignored), Engine builds/22 tests on each, and hook/selector fixtures (14 + 7). Test/tooling repairs through `bdcd00650e22c0af79699aa597e137957a091233` fixed the HOME display assertion, short socket fixture, owned Mac ACL teardown and actual ACL test selector; the focused regressions passed before full suites. This is merged-main acceptance with retained final-head CI evidence, not a fresh run on this documentation change. No native GUI/capture, deployment or release acceptance is claimed.
- [ ] **2a. Reuse config #221.** After 1. Reconcile head `1ea0d146` against current main and original reviewed blobs; integrate the existing storage/test extraction, not another implementation. Accept inverse reconstruction, preserved config/unknown keys/secrets and queue→config contract, affected compile and handoff/hosted-config/Settings tests. Reuse retained full results where inputs match. Excludes consent/schema changes, second preference store and unnecessary native replay.
- [ ] **2b. Reuse recorder #225 (minimal current-main replacement).** Re-cut on accepted #221 merge `476b1361` from frozen recorder `2085f796`, rather than reconciling #225's obsolete config ancestry (`28d83f3d`). The three recorder blobs are identical to the independently reviewed extraction; inverse reconstruction restores the complete current-main capture donor, with all nine moved and twelve retained test bodies/attributes preserved. Existing `capture` exports are the sole compatibility facade, owned by the recorder slice; Desktop, timed CLI, bundle processing and native samplers keep their existing paths. Remove that facade only after all supported consumers and build inputs migrate to the accepted Engine contract, not by adding another staging layer. Normal selector/hook root compilation and capture/agent_capture tests, plus actual macOS compilation/focused tests, remain required before parent acceptance; historical source review/test evidence is reused only with exact provenance and input comparison. No new native capture is required solely for this file move. Milestone B's native/runtime acceptance remains fully in scope for the campaign; this slice is not Engine adoption or a standalone Agent. Native adapters, production readiness/start/stop/join/retention and window/screen behavior are unchanged; #233 is already complete and is not repeated. Keep this box unchecked until the parent merges and records the replacement PR's exact results.
- [x] **3. Common bundle processing out of main.rs (bounded in-crate extraction).** [#242](https://github.com/tryapparent/apparent/pull/242), tested `f135448c8b252066b104fc4790b2a2273500a9c8`, merge `715a8918e58cdc48dfe28e1a829145fbb44d8bc9`. Parent fresh independent review: **APPROVE_SOURCE**; 30,642 production bytes, all ten complete tests and eleven root exports preserved, with inverse reconstruction against current donor base. Exact-head CI [41280661463](https://github.com/tryapparent/apparent/actions/runs/41280661463) logs confirm Linux 683/Mac 607 passed (3 ignored each), root builds and Engine 23 passed on each platform. Retained focused proof: 10 processing/43 capture/8 recovery/19 sketch; `pr242-current-main` output records show synthetic manifest/summary/PNG parity within documented identity/path exclusions. Merge preserves both owned blobs; only #245 ledger/#246 tooling advance its base, and reconstructed full merge tree matches. This reuses exact-source execution evidence, not a fresh docs-branch run. Demo and compatibility exports stay root; publication 6, Engine/standalone adoption and native capture remain unaccepted.
- [ ] **4. Useful Desktop coordination seams.** After 3 and the already-merged #233. Extract only cohesive human workflow/presentation state needed by upcoming work; keep exact request receipts before stale-UI guards, current consent before Connect admission, feedback local and clipboard/onboarding in Desktop. Accept focused completion/cancel/stale-result tests, source preservation for moves and compile. Excludes a wholesale #208 split, empty folder taxonomy, renderer replacement and relocating active picker/ink owners' code.
- [ ] **5a. Independent Connect client.** After 1/2a and explicit client path reservation following merged #191 (`1a8f19d3`); an obsolete open-PR block is not a reservation. Own client manifest and fresh config-provider reads/transactions, preserving queue→config lock order, grant generations, revocation and local fallback. Accept independent build without GPUI/native build inputs and existing synthetic transfer/auth/consent/fetch tests plus failure/disable races. A protocol leaf is optional preparation, not client independence; avoid it if a coordinated full-package slice is feasible. No auth/renewal redesign in the mechanical extraction.
- [x] **5b. Additive Connect browser package (historical merged staging acceptance only).** [#244](https://github.com/tryapparent/apparent/pull/244), reviewed final package `41bf5780a6b62cdffce2876a2df9ecfaf6f32e30`, merge `89a41b877857c5ef29d05db22d90563786d2765f`. Parent accepted independent staging, not 5c. Fresh Git comparison here confirms the complete `connect/web` tree `8c90c1c8ea56d71a3495daf8a4dd8cc25f70fcf1` is identical at reviewed head, merge and pinned current main. At the merge, `Hosted.tsx`, `api.ts`, `hosted.css` and `base.css` exactly match `site/src/hosted` donors. Retained final-head normal-hook evidence reports Bun 1.3.10 frozen install, TypeScript/production build, four artifact/closure + two dev/preview route tests and all 32 synthetic browser scenarios, including raw-failure-body redaction; 15 hook + 13 selector tests passed. This reuses historical execution/review, not a fresh browser run or live OAuth/native/deployment proof. **Current donor drift:** merged #191 changes legacy `Hosted.tsx`, `api.ts` and `hosted.css` and adds `SharedLinks.tsx`; the staged package remains unchanged, while `base.css` still matches. Do not claim current-main behavior parity. Reconcile those contracts in the owner-gated 5c adoption before replacing production donors; production build separation remains incomplete.
- [ ] **5c. Adopt browser package and remove donors.** After 5b; parent now exclusively owns composition, replacing the cancelled Pages-owner reservation. Sole publisher composes independent output while preserving accepted homepage overlay, canonical/legacy routes and query/auth/origin contracts. Accept synthetic route/consent checks, built graph separation, allowlist/privacy/collision rejection and packaging behavior. Delete old authenticated implementation only after production composition consumes the new one. Excludes publisher replacement, deploy, private pitch publication and route redesign.
- [ ] **6. Checked publication correctness.** After 3; separate behavior PR from moves. Use #210 (successor of #209) as evidence for `coordinate`/`publish_after`/`checked`, porting only dependency-complete work. Whiteboard mutation **and fallback** remain inside the outer publication closure through validation and terminal effects. Accept failed writes/media/cleanup never produce Saved, durability/identity checks, pending transcript/recovery writer coordination and synthetic MP4/MOV plus relevant batched platform proof. Excludes premature Engine success or format redesign.
- [ ] **7a. Paired shared model/candidate leaves.** After 2b and Engine dependency/build decision. Reassess #214/#215 together; move authoritative types/buffer/tests and delete donors in the same adoption slice. Accept wire/constructor/type-identity parity and package/root compilation/tests. No duplicate long-lived leaves, session adoption or standalone claim.
- [ ] **7b. Shared recorder worker orchestration.** After 7a; reassess #216 injected-worker contract. Accept actual-frame readiness through Stop/Drop joins, resource lifetime/retry preservation and owning tests on both platforms. Native adapters remain explicit dependencies until moved. This is not standalone native acquisition.
- [ ] **7c. Production Engine ownership.** After 6 and required 7a/7b contracts. Reassess #211, covering Desktop and timed screen/device paths together before any media side effect and through detached finalization/terminal publication. Agent-via-GUI follows the same owner. Timeout does not cancel or release ownership. Accept real multiprocess contention/restart/kill/wrong-owner/stop/cancel tests, correct first-frame readiness, exact finalizer receipt and batched native proof on macOS/Linux. Preserve socket/API compatibility and caller consent; no second recorder or PID/TTL ownership shortcut.
- [ ] **8. Archive coordination.** After 7c and publication writer coordination. Cover each reader/writer/export/reprocess/recovery/retention/import/transfer path and destructive bypass in coherent operation-sized PRs. Read/Write/Finalize Engine leases are all **shared maintenance exclusions**, not writer serialization. Keep publication serialization and BUSY/process-local claims until tested replacements cover their invariant. Retain persisted bundle identity across rename; filename alone is insufficient. Accept contention, reader/maintenance races, rename/retry/deletion/failure tests and operation parity. Excludes blanket replacement of all guards from package-only proof.
- [ ] **9a. Native/runtime/build closure.** After 7c/8 and active retained-capability/native-producer handoffs. Trace manifests, Swift/native helpers, generated resources, models, config callbacks, dynamic libraries, install/signing/TCC identity and embedded inputs. Move capture mechanics without Desktop policy. Accept independently built Engine/Agent closure and accurate platform capability errors; package/install regressions receive relevant native proof. No helper fallback into an installed Desktop or developer tree.
- [ ] **9b. Standalone Agent.** After 9a. Own automation deadlines/cancellation/evidence; consume Engine directly while preserving explicit request identity, inspect/retry and local-only behavior. Accept clean macOS/Linux install and real record/readiness/stop/cancel/saved/evidence flow where Desktop was never installed; inspect resulting bundles and dynamic/runtime dependencies, plus coexistence/admission with Desktop. Graphical sessions/permissions may be required and must be stated. No claim from a root second binary or injected worker alone.
- [ ] **10. Finish product/build separation and delete facades.** After relevant product adoptions, not necessarily after every unrelated feature. Complete Desktop/Agent/Connect/Public build, test, docs and packaging references; remove superseded implementations and forwarding paths when consumer inventories are empty. Accept fresh-checkout supported builds, import/dependency checks, installation/update/CLI/URL compatibility and privacy-clean public artifact. Keep external aliases only where needed. Excludes a cosmetic repo-wide rename, new topology or web Desktop campaign.

- [ ] **C. Opportunistic coherent deletion slices.** After 1 and clearance of any active owner; may accompany a relevant slice or run independently, without waiting for 10. Verify requiredness as above, delete retired code/docs/assets with obsolete tests/references, and record why supported consumers/readers/build inputs survive. Use reference/import/artifact checks and affected verification for the actual deletion; do not exhaustively refactor dead paths or create archive copies. No private/uncommitted/evidence cleanup. Record each merged slice and acceptance; check C only after the final sweep meets acceptance.
- [ ] **11. Final legacy sweep.** After 10 and each applicable owner handoff. Recheck remaining legacy code/docs/assets, Windows remnants and stale references against supported consumers/readers and packaged artifacts. Delete only verified-retired paths in bounded PRs, or record the remaining requirement/blocker and resume condition. Accept no unneeded donor implementations/archive copies and passing affected checks; do not turn required compatibility paths into deletion targets or refactor every historical path.

## Blockers, staging disposition and resumption

No unchecked dependent item may be skipped into correctness. An independent lane may
continue after a bounded blocker is recorded. Recheck live ownership before assignment.

| Work | Current limit | Resume condition |
|---|---|---|
| #221 config | Still unaccepted; current-main reconciliation assigned. #242 processing is now merged/accepted only as checklist 3 | Worker returns exact source/evidence and proposed text to parent, not edits to this sole-writer ledger; exact-pair routing alone is not acceptance |
| #225 recorder | Open draft includes #221; waits for config reconciliation/merge | After accepted #221, reconcile recorder-only delta, reviewed bodies/test mapping, compile and focused proof; do not replay native work solely for a move |
| Connect client | #191 merged as `1a8f19d3`; its source changes are now main authority, not an open-PR blocker | Explicit current owner/path reservation and accepted 2a before client extraction; preserve sharing/consent contracts |
| Browser source sync / production adoption | Sync `31281ba5` independently reviewed but unmerged; no current-main donor parity claim. Former Pages owner cancelled; parent exclusively reserves 5c composition | Parent accepts source sync and composed artifact before donor deletion; reserve publisher/Pages runtime, functions/lib, wrangler, homepage and workflow paths. Isolated ten-path retirement re-cut `2ac8cb21` is source-only, not completed cleanup, deployment or infrastructure proof |
| Native dependency closure | #185 retained-capability changes and #235 Linux producer | Owner handoff and exact retained capabilities/build inputs agreed; no racing native moves |
| Other live feature paths | #234 mark text, Mac share-picker, retirement/hosted-sharing and website Pages writers; #237 audio ducking merged while drafting | Explicit exclusive reservation or wait for merge; do not touch their production files here; retain #237's new build inputs in future delta comparisons |
| Standalone Agent/native behavior | No clean-machine runtime proof yet | Complete prerequisites and obtain owned bounded platform slot; no new harness project after 30-minute dead end |
| Web Desktop host | #213 is diagnostic staging, not accepted native implementation | Separate future campaign/authorization; excluded here, GPUI remains |

Staging #199–216 is a library of evidence, not a merge queue: #199 umbrella is not
the target; #200 informs 5b/5c; #201 informs 3; #202's bounded package already reached
main through #220; #203/#205 already merged unrelated CI/build-label work; #204 is
superseded for the recorder seam by #225; #206 recovery popup is separate product
work; #207's manifest/hook policy is not automatically imported; #208 is optional
coordination evidence, not mandatory file splitting; #209 is superseded by #210;
#211 needs correctness and production-lifetime adoption; #212's CI assumes packages
not on main and is not transplanted; #213 is excluded; #214/#215 require paired donor
deletion; #216's injected worker is useful but not native closure. Re-cut against
current main and preserve attribution; do not blindly cherry-pick these branches.

## Evidence and limits

Historical owner threads: `6ad16648-03aa-45cd-aacd-9031295ba670` (stopped original),
`7b7cdc5b-2de9-455a-8037-ce43069c025d` (continuation),
`ac8137ae-287f-439b-8016-9f803b6deae8` (strategy/discovery),
`6309b1b1-5327-41fd-98e1-b6d1d25aed1f` (#233 completion). Their proposals for a web
Desktop, broad staging merge and blanket native checks are superseded by this scope.

Config/recorder retained Linux receipts were inspected at
`~/.local/share/apparent/refactor-evidence/serial-main-221225-v1/verification-results.json`:
config `61338307`, recorder `63cbbb5b`, base `d21115b5`, 668 passed/3 ignored each,
focused tests and exact namespace mapping. These are historical revisions, not fresh
runs against `6f043ff4`. Parent discovery additionally reports Mac 592 passed/3 ignored
after canonical-root fixture correction (`mac-execute-v2` logs), and exact inverse
reconstruction of current heads #221 `1ea0d146`/#225 `28d83f3d` against current main.
Read those logs and compare all changed inputs before carrying that evidence to a new
candidate. This document does not claim a fresh native capture, Mac compile or install.

The Linux documentation rows above retain their original source paths; the files were
subsequently grouped under `capture/desktop/docs/linux/`, so the destinations shown
are their current paths.

Relevant contracts: `capture/engine/README.md`, `capture/engine/docs/capture.md`,
`capture/engine/docs/keyframes-and-text.md`, `docs/hosted-{api,client,web}.md`,
`capture/engine/docs/export-and-retention.md`, `capture/desktop/docs/linux/linux.md`, `capture/agent/docs/agent-video-workflow.md` and
`capture/desktop/docs/onboarding-and-settings.md`; read the matching guide before each implementation.
No private capture directory exists on this implementation host at the checked default
Mac/Linux locations. No private bundle, recording, transcript or secret is committed.
Strategy verification results belong in its PR/handoff; unchecked steps above remain
pending until the parent records their actual merged acceptance on main.
