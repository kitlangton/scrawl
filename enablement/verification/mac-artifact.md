# Intel Mac development executable producer

`mac-artifact.ts FRESH_OUTPUT_DIRECTORY` retains existing `target/debug/apparent`
and `apparent-helper`, plus `manifest.json`, after the existing Rust job checks
and evidence uploads succeed. It never compiles, stages a package or signs.
Failures fail the job. The ten-minute cap and terminal job dependencies remain.
Node 22 discovers its tests through `enablement/verification/*.test.ts`.

Schema 1 (`mac-dev-executables`) pins actual HEAD and tree, not event SHA or an
assumed workflow input. It records workflow/run/attempt/job identity, lock/config
hashes, Intel dev target, deployment target, probed toolchain identities and
allowlisted environment, executable bytes/hashes/modes and Mach-O closure.
Only three fixed raw names are uploaded. Consumers must reject extra files,
links, unsafe names and hash/mode mismatches, restoring recorded modes because
Actions artifact transport does not preserve executable permission bits.

The artifact name is `mac-dev-executables-RUN-ATTEMPT`. The consumer must
select an explicit immutable artifact ID and run/attempt, independently query
GitHub final job AND run metadata and require success, verify the expected
repository/workflow and exact checkout SHA/tree, and validate the entire contract.
Upload-time success is not final producer success. Workflow event SHA can differ
from checked-out `inputs.revision`. This manifest grants no test/native/release
reuse authority. Installer consumption follows the admission below.

Runtime audit: build.rs adds system Swift and selected-Xcode Swift rpaths. Only
reviewed rpaths and OS load dependencies are admitted. An @rpath Swift dependency
must resolve first to an existing system file; Xcode-only, loader-relative,
producer-local, unknown or unparsed dependencies fail closed. System libraries
may live in the dyld shared cache; an unresolved Swift cache-only entry currently
fails conservatively rather than guessing. No Xcode runtime is copied. Actual
Mac output is needed to establish whether this strict closure admits the build.
Linux fixtures establish parser/creator behavior, not Mac runtime compatibility.

The source audit found Linux-only dynamic transcribe staging in build.rs;
Cargo.toml's dynamic transcribe dependency is Linux/Windows-only. Mac Sparkle
is loaded dynamically from the app package (`capture/desktop/sparkle.rs`) and
must be freshly staged by the installer, along with Electron/frontend/resources
and signing. The two files are therefore executable inputs, NOT a complete app.
Future runtime changes require reviewing this closure contract. Main artifact
availability, final job success and completion within ten minutes are unverified;
no native workflow was started for this producer slice.

## Focused proof consumption

The focused Mac workflow accepts explicit `producer_run` and `producer_artifact`
IDs. Both are required together. `mac-artifact-consumer.ts admit` queries the
trusted repository's final automatic main Mac run, its exact-attempt Rust job and
immutable artifact before download. `prepare` downloads the immutable artifact itself, verifies its GitHub archive
digest, and extracts only three bounded regular files into a fresh directory. It
additionally checks this checkout's
SHA/tree, lock/config hashes, fixed regular-file payload, executable hashes/modes,
toolchain identity and runtime audit before restoring executable permissions.
The installer uses this route only with a fresh `APPARENT_PREBUILT_DIR`,
`APPARENT_PREBUILT_RUN` and `APPARENT_PREBUILT_ARTIFACT`; GitHub read access must be
available through `gh`. Invalid inputs fail without compiling. Ordinary installs
still build normally. The selected SDK, fresh frontend/Electron, package resources,
Sparkle, running-app guard and signing are retained. Input manifest and installed
post-signing executable hashes are saved separately in proof evidence.

A timed-out main producer is not usable even if its build finished. The producer must first complete its existing required work within the cap. No qualifying
artifact or successful sub-ten-minute native execution has yet been established.

### Focused Settings build inputs

`APPARENT_PREBUILT_KIND` defaults to `verified-main`, preserving final automatic
main run and Rust-job success. The explicit `focused-settings-build` kind instead
requires the current `verification-mac.yml` manual candidate workflow and attempt
to be in progress, with a completed successful job named `settings-build`.
Only its `settings-native` consumer job may use this admission. The current
workflow ref, workflow SHA, event SHA, actual HEAD/tree and API run head must agree;
main-branch, cross-run, cross-attempt and cross-candidate inputs are refused.

The manifest must explicitly identify `producer.kind=focused-settings-build`,
`producer.job=settings-build` and the exact candidate workflow ref/SHA. Payload,
immutable artifact ID/digest, configuration, toolchain and runtime checks are
unchanged. Run/artifact/destination environment names and CLI remain the same.
This is a build input for the same run's subsequent native test, never a full
verification, release or successful native receipt. The Settings owner supplies
its emitter and workflow composition; the consumer adds no candidate build job.


### Focused public-share build inputs

The separate `focused-share-build` identity uses `native-proof-mac.yml`, build
job `share-build` and consumer `connect-worker`. It has the same exact-candidate,
same-run/attempt and successful-build requirements as Settings; neither kind can
consume the other's artifact. The producer CLI selects the kind explicitly.

A share dispatch without explicit main artifact IDs builds once in `share-build`,
then passes its immutable artifact ID to the native job. Each job retains a
10-minute whole-job limit. Failed/cancelled builds cannot start the native job or
fall back to rebuilding. Explicit verified-main inputs and the existing worker
scenario retain their routes. Build reuse removes repeated compilation from
installed acceptance; it does not remove or certify any native assertions.
Both measured complete job times, runtime closure and actual installed proof
remain required; local contract tests establish none of those outcomes.
