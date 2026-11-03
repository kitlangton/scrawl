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

The artifact name is `mac-dev-executables-RUN-ATTEMPT`. A future consumer must
select an explicit immutable artifact ID and run/attempt, independently query
GitHub final job AND run metadata and require success, verify the expected
repository/workflow and exact checkout SHA/tree, and validate the entire contract.
Upload-time success is not final producer success. Workflow event SHA can differ
from checked-out `inputs.revision`. This manifest grants no test/native/release
reuse authority. Installer consumption is a separate parent-owned change.

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
