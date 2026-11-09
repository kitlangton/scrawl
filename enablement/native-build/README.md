# Mac native build reuse

`build.rs` calls `build.ts` with Node 22+ on macOS. This TypeScript owner selects
the existing Swift helper sources and two Metal shaders, runs the original
compiler/linker arguments, and reuses outputs only after checking their input
and output SHA-256 hashes. Cargo still owns execution serialization and version
stamping. Linux's native build path is unchanged; no Cargo dependency is added.

Receipts live only inside Cargo's existing `OUT_DIR`. Identity includes the
selected compiler executables and versions, SDK path/version/build/settings,
target architecture, source/plist bytes, build-tool source and relevant native
environment values. Preview helper identity remains separate. Missing, malformed,
incompatible or corrupt entries rebuild. Failure publishes no success receipt.
The runtime helper is restaged beside the application even on a hit.

The script emits Cargo watches for inputs, not outputs it writes. An absolute,
canonical `DEVELOPER_DIR` matching the normalized `xcode-select -p` result omits the global
`/var/db/xcode_select_link` watch: Cargo recursively follows that directory
symlink, even on warm freshness checks. Both Xcode app roots and Developer paths
qualify; app-root discovery is explicitly normalized to `Contents/Developer`.
Mutable aliases retain conservative watches so retargeting them schedules the
build script even when the environment string is unchanged. Developer, compiler
and SDK discovery traverses symlink targets component by component, including
intermediate links and parents introduced by those targets. Unresolvable paths
fail before a receipt is accepted. Selected directory aliases and the containing
directories of file aliases still receive Cargo's recursive watches. Selected
Xcode `Info.plist`/`version.plist`, compiler executables and SDK settings remain
watched and hashed; the selected developer
path and native environment remain part of receipt identity. Unset, empty,
relative or mismatched overrides retain the conservative global watch. Invalid
selection/tool discovery fails before accepting or publishing a receipt; the
platform tools still decide which override forms are usable.

There is one bounded OS-layout exception: `/var` with the literal link target
`private/var`, resolving exactly to `/private/var`, is traversed without watching
that alias. Cryptex-hosted Metal discovery can return `/var/run/.../metal` and
`metallib`; watching `/var` recursively scans unrelated system logs/caches and
can make every warm build dirty. Canonical selected executables remain watched
and hashed. The `metallib` file alias (typically to `air-lld`), its containing
`usr/bin`, and any mutable aliases encountered below or through `/var` retain
their watches. This is not a prefix exclusion for `/var` descendants, nor an
exception for every directory named `var`. Nonstandard targets, including an
aliased `/private/var`, keep conservative watches.

This assumes the standard OS root alias stays fixed between builds. After an
OS/admin change to that layout, clean the affected Cargo target before building;
the script cannot inspect a retarget while Cargo considers it fresh. Cargo has
no link-only `rerun-if-changed` mode: its
[recursive mtime check](https://github.com/rust-lang/cargo/blob/master/crates/cargo-util/src/paths.rs)
follows directory links, whereas file watches stat their targets. Thus selected
file aliases still need their parent directory watches to catch retargeting to
older files. The repair intentionally retains that bounded directory traversal,
global-selector fallback and all native receipt/output validation.

Callers must export `DEVELOPER_DIR` before invoking Cargo to use this optimization.
Both `verification-platform.yml` and the private onboarding candidate in
`verification-mac.yml` persist the canonical global selection immediately after
setup-xcode, clearing any inherited override during discovery. The workflow
contract executes both pinning steps and checks their order before native builds.
Xcode selection, SDK version and verification gates stay the same.

SDK/tool identity is rechecked whenever Cargo invokes the script; this does not
claim to detect arbitrary output deletion while Cargo considers the build fresh. No
cross-worktree cache, verification receipt or publication permission is created.
Reverting this adapter restores the prior unconditional native compilation.

The existing early prerequisite suite imports the launch-free TypeScript tests:

```sh
node --experimental-strip-types --test enablement/prerequisites/native.test.ts
```

To exercise the actual production Rust adapter in a tiny Cargo package too:

```sh
APPARENT_NATIVE_CARGO_FIXTURE=1 node --experimental-strip-types --test enablement/native-build/build.test.ts
```

The latter uses the production Rust adapter with synthetic compiler commands and
real Cargo. Its cryptex fixture redirects only OS-owned absolute path literals
into a private filesystem tree, preserving `/var -> private/var` and the Metal
file-alias chain. It checks three unrelated churn cycles stay Fresh with no
Running commands, exact system watches, selected-input edits, older file-target
retargeting, transitive/target-parent aliases and dangling/cyclic refusal. These
checks prove Cargo scheduling/reuse, not native compilation or GUI acceptance.
On Clanker run tests once through `t3-build`, exporting `TMPDIR` to a private
`/tmp/apparent-dev-<job>-<run>/` directory; child Cargo inherits its job allocation.
Compare real Mac compiler invocations and timings on the exact candidate before
claiming delivery-time savings. Keep
all existing hosted Mac/Linux build, integration and native assertions required.

## Controlled warm Mac check

Use one idle, approved Mac checkout with dependencies already available offline,
no active capture and a clean tree. Set `BASE` to the PR's exact base and
`CANDIDATE` to its exact head. Run this in that checkout (no installation, GUI
launch, global Xcode switch or rental is needed). Both revisions use the same
target directory, toolchain, environment and ordinary build command. Each switch
gets an unmeasured warm-up; repeat the baseline after the candidate to expose
machine drift. Keep logs private and record any concurrent load.

Revalidation must include the ordinary cryptex Metal layout that exposed the
regression: canonical `DEVELOPER_DIR`, `TOOLCHAINS=MetalToolchain`, and the same
compiler/SDK/native environment throughout. Record the actual OS/Xcode versions,
raw and canonical tool paths, `/var` link target, SDK settings and compiler hashes
before and after **each** round. Do not infer those identities from provider
image metadata. The earlier e36 candidate failed its first strict Fresh sample;
later diagnostic samples with an extra warm-up did not qualify that recipe.

```sh
set -eu
: "${BASE:?Set the exact baseline SHA}" "${CANDIDATE:?Set the exact candidate SHA}"
test -z "$(git status --porcelain)" || exit 1
original=$(git symbolic-ref --short -q HEAD || git rev-parse HEAD)
proof=$(mktemp -d /tmp/apparent-dev-native-watch-XXXXXX)
export CARGO_TARGET_DIR="$proof/target" TMPDIR="$proof"
selected="$(env -u DEVELOPER_DIR /usr/bin/xcode-select -p)"
export DEVELOPER_DIR="$(cd "$selected" && pwd -P)"
test -e /var/db/xcode_select_link || exit 1
trap 'git switch "$original" || git switch --detach "$original"' EXIT
record_inputs() {
  sw_vers; uname -m; node --version; cargo --version; rustc -Vv
  printf 'DEVELOPER_DIR=%s\n' "$DEVELOPER_DIR"
  printf 'SDKROOT=%s TOOLCHAINS=%s\n' "${SDKROOT-}" "${TOOLCHAINS-}"
  readlink /var
  xcodebuild -version
  xcrun --sdk macosx --show-sdk-path
  xcrun --sdk macosx --show-sdk-version
  xcrun --sdk macosx --show-sdk-build-version
  sdk=$(xcrun --sdk macosx --show-sdk-path)
  for settings in "$sdk/SDKSettings.json" "$sdk/SDKSettings.plist"; do
    test ! -f "$settings" || shasum -a 256 "$settings"
  done
  for tool in swiftc swift-frontend clang ld metal metallib; do
    path=$(xcrun --sdk macosx --find "$tool")
    node -e 'console.log(process.argv[1], require("node:fs").realpathSync.native(process.argv[1]))' "$path"
    shasum -a 256 "$path"
  done
}
round=0
for revision in "$BASE" "$CANDIDATE" "$BASE"; do
  round=$((round + 1))
  git switch --detach "$revision" || exit 1
  git rev-parse HEAD > "$proof/$round.sha"
  shasum -a 256 Cargo.lock build.rs enablement/native-build/build.ts \
    .github/workflows/verification-platform.yml .github/workflows/verification-mac.yml > "$proof/$round.sources"
  taskpolicy -c utility cargo build -j 3 --offline -vv > "$proof/$round.warm.log" 2>&1 || exit 1
  record_inputs > "$proof/$round.before.inputs" 2>&1
  cmp "$proof/1.before.inputs" "$proof/$round.before.inputs" || exit 1
  for output in "$CARGO_TARGET_DIR"/debug/build/apparent-*/output; do
    test -f "$output" || continue
    cp "$output" "$proof/$round.$(basename "$(dirname "$output")").output"
  done
  for sample in 1 2 3; do
    /usr/bin/time -p taskpolicy -c utility cargo build -j 3 --offline -vv \
      > "$proof/$round.$sample.log" 2>&1 || exit 1
    # Stop at the first failed sample; no extra warm-ups or diagnostic
    # continuation can turn a failed round into a qualified comparison.
    grep -q 'Fresh apparent v' "$proof/$round.$sample.log" || exit 1
    if grep -q 'Running `' "$proof/$round.$sample.log"; then exit 1; fi
  done
  record_inputs > "$proof/$round.after.inputs" 2>&1
  cmp "$proof/$round.before.inputs" "$proof/$round.after.inputs" || exit 1
done
printf 'Private comparison logs: %s\n' "$proof"
```

Require `Fresh apparent` and no build-script/native compiler execution in every
measured sample; compare the three warm wall times per round, not warm-up times.
Inspect the saved build-script `output` under the target directory during each
round: the baseline watches the selector, while the explicit candidate must not.
For the standard cryptex layout, also require the candidate to omit the exact
`/var` watch (and any substitute broad ancestor such as `/private/var`), while
retaining canonical Metal executable watches, `metallib`'s alias/parent/`air-lld`
watches and the selected Xcode/SDK/Swift watches. Native outputs must remain
unchanged and valid; a forced metadata-only rerun must still report verified
reuse. Perform that diagnostic outside the measured rounds.
Confirm the platform's actual `xcode-select`/`xcrun` discovery for canonical,
app-root, empty and relative overrides; synthetic fixtures do not establish
Apple's behavior. Keep compiler/SDK identity unchanged across the timing rounds.
Retain separate native acceptance: these timings alone do not certify a playable
recording, onboarding or Hermes behavior, or hosted/platform verification.
