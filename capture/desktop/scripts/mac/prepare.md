# Prepare the device-free Darwin regression packet on Linux

From a clean checkout, freeze the complete candidate and validate the real
`engine/native/tests/live-transcribe.py` input closure before requesting
Mac execution. This command launches nothing, allocates nothing and needs only
Python 3 and Git. It does not replace the separately owned Linux frontend/Rust
bridge verification command.

```sh
candidate=$(git rev-parse HEAD)
packet=$(mktemp -d /tmp/apparent-mac-prep.XXXXXXXX)
python3 capture/desktop/scripts/mac/prepare.py \
  --candidate "$candidate" --receipt "$packet/preparation.json"
```

The receipt is private (0600), exclusively created in an existing private
outside-checkout directory. Preserve it privately; paths and hashes describe
source inputs, not native evidence. The command refuses a stale SHA, tracked or
untracked changes, hidden index flags, missing/symlink inputs, altered committed
bytes, invalid Python or malformed Info.plist. It hashes `build.rs`, the Cargo
manifest, every complete helper Swift input in basename compiler order, the
linker plist, and both moved Swift fixtures plus the maintained Python entry.
There are no generated capture request IDs or external media fixtures in this
scenario: PCM is generated in memory by the existing Swift tests. Do not use
this packet to claim capture-control, GUI or hardware acceptance.

Missing fixture/startup dependencies are a known native harness failure class
(see the filesystem fixture and environment boundaries in
[Native testing](../../docs/native-testing.md)). The regression removes the
actual sibling Swift fixture from a committed synthetic checkout and requires
preparation to fail. This is a faithful missing-input discriminator, not replay
of a retained Mac execution log: no private original failure log is present in
this checkout.

After separate authorized Mac admission, transfer/check out the complete same
commit/tree, rerun preparation there with a new receipt, compare every input
hash, and check the guest prerequisites listed in the receipt. In particular,
`xcrun swiftc --version` must report Swift 6.2+ and a usable macOS SDK must be
installed. Then run from the repository root:

```sh
python3 engine/native/tests/live-transcribe.py
```

Compile the complete helper with the source order and flags from `build.rs`,
then run the existing RGB proof:

```sh
target="$(uname -m)-apple-macos15.0"
xcrun swiftc -O -swift-version 5 -target "$target" \
  engine/native/Export.swift engine/native/LiveTranscribe.swift engine/native/Media.swift \
  capture/engine/native/Mirror.swift capture/desktop/native/Onboarding.swift \
  capture/desktop/native/OnboardingDemo.swift capture/desktop/native/PermissionPanel.swift \
  engine/native/PlaybackVolume.swift engine/native/ScreenProbe.swift \
  capture/desktop/native/Settings.swift capture/desktop/native/ShortcutKeys.swift shared/native/main.swift \
  -o "$TMPDIR/apparent-helper" \
  -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker shared/native/Info.plist
xcrun swiftc -O -swift-version 5 -target "$target" -parse-as-library \
  engine/native/Export.swift engine/native/Media.swift engine/native/tests/ink-rgb.swift \
  -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker shared/native/Info.plist \
  -o "$TMPDIR/ink-rgb-proof"
rgb_output=$(mktemp -d "$TMPDIR/apparent-ink-rgb.XXXXXXXX")
rmdir "$rgb_output"
"$TMPDIR/ink-rgb-proof" "$rgb_output"
rm -rf "$rgb_output" "$TMPDIR/apparent-helper" "$TMPDIR/ink-rgb-proof"
```

Retain the actual command, exit, compiler output and candidate identity. The
entrypoint compiles its production/test Swift combination and executes the
result with bounded subprocess deadlines. It uses no audio device, model,
permission reset or speaker playback. The preparation receipt leaves every
native gate `UNRUN`; it cannot certify SDK availability, compilation or test
success. Installed-app, permission, GUI, full media and platform acceptance
remain separate requirements where applicable. Never rewrite this receipt into
a native pass. Any candidate/input change requires new preparation; previous
native evidence can only be reused under the repository's unchanged-input gates.

Host regressions (no Rust build or Mac required):

```sh
python3 -m unittest discover -s enablement/tests -p test_mac_prepare_push.py -v
```

The existing `*push*.py` discovery in `enablement/verification.py --pr` includes
these tests without changing native CI or release admission.
