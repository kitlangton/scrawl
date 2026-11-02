# Portable Cargo download cache

`cache.py` exports only crates.io `.crate` downloads named in the supplied
Cargo.lock, verified against its SHA-256 checksums. Requires Python 3.11+.
It does not export HOME, configuration, credentials, git dependencies, registry
indexes, extracted sources, npm state, captures or `target/`. Use archives from
an approved trusted operator; checksums are integrity checks, not signatures.
Run without concurrent cache writers, in a private operator-owned directory.

[NoMac CLI contract](https://nomac.app/docs/cli), checked 2026-11-02:
parallel sessions run without `~/drive`; stopping removes session files outside
that drive. The existing launcher uses `--parallel --drive none`. This helper
changes neither launcher nor persistence settings. Export and transfer the archive
before stopping an already-authorized session; transfer it back to a later session
using the existing approved file-transfer route. No allocation is needed to run
host-safe tests.

## Standalone commands

From repository root, first populate a dedicated Cargo home on an authorized Mac
(or Linux preparation host), then export. `cargo fetch` still needs network/index
access and uses the repository's unchanged lockfile. No credentials are copied
from the ordinary Cargo home.

```sh
umask 077
export CARGO_HOME="$PWD/.cargo-downloads"
cargo fetch --locked
python3 capture/desktop/scripts/mac/cache.py export \
  --cargo-home "$CARGO_HOME" --archive "$PWD/cargo-downloads.tar"
```

Transfer `cargo-downloads.tar` privately outside the disposable session. In the
next checkout, place that file at repository root and import into an **absent**
dedicated Cargo home (its parent must already exist):

```sh
umask 077
export CARGO_HOME="$PWD/.cargo-downloads-restored"
python3 capture/desktop/scripts/mac/cache.py import \
  --cargo-home "$CARGO_HOME" --archive "$PWD/cargo-downloads.tar"
cargo fetch --locked
```

Exit 0 means a nonempty cache was exported/imported; 3 is a normal miss (missing
archive or no matching downloads); 1 rejects invalid input. Handle 3 explicitly
in automation, then use normal `cargo fetch --locked`; do not hide rejection errors.
Export refuses an existing archive. Import refuses an existing Cargo home rather
than merging into potentially configured or credentialed state. Choose a fresh
operator-owned path for another import. No automatic cleanup or pruning occurs.
Keep these local paths out of commits; remove only your identified test/cache files.

Only the sparse crates.io cache directory is supported. Legacy registry cache
layouts miss safely. Entries absent from a newer lockfile are rejected; export
again against that lockfile. Partial caches are usable: Cargo fetches the rest.
Architecture, Rust version and SDK need no key because these are checksum-bound
source archives, not compiled artifacts. No archive extraction API is used;
symlinks, hardlinks, special files, paths, duplicates, non-lock entries and bad
checksums are rejected. Plain tar only, 512 MiB payload and 10,000-entry maximum;
restored files/directories are private, staging disappears on failure.

This slice avoids repeat dependency **downloads**, not compilation itself. Darwin
speedup is unmeasured. It intentionally does not restore arbitrary compiled
artifacts or claim any test receipt. Continue normal build/test gates. Leave the
build target at the repository default: `packaging/install-app.sh` copies
`target/debug/apparent` and `target/debug/apparent-helper`. Do not set a cache
`CARGO_TARGET_DIR` and mistake a different output directory for installer inputs.
Use existing build-runner and native admission procedures; no launch/install is
performed here. A real Darwin fetch/import/build measurement and native acceptance
remain pending. No full Rust build is needed for this tooling-only slice.

Host regression route (also discovered by cheap PR push checks):

```sh
python3 -m unittest discover -s enablement/tests -p 'test_mac_cache_push.py' -v
```
