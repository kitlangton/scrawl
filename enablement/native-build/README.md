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

The script emits Cargo watches for inputs, not outputs it writes. SDK/tool
identity is rechecked whenever Cargo invokes the script; this does not claim to
detect arbitrary output deletion while Cargo considers the build fresh. No
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

The latter uses synthetic compiler commands and proves Cargo scheduling/reuse,
not native compilation or GUI acceptance. Compare real Mac compiler invocations
and timings on the exact candidate before claiming delivery-time savings. Keep
all existing hosted Mac/Linux build, integration and native assertions required.
