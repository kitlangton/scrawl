# Disposable Mac prerequisites

Use an already authorized disposable Mac image with Python 3, full Xcode selected,
Homebrew installed from its official distribution. Missing rustup is installed
through Homebrew’s verified package. This helper does
not acquire a machine. Never designate a personal host disposable.

From the checkout root:

```sh
# Read-only: caller supplies an existing short private TMPDIR.
capture/desktop/scripts/mac/setup.sh doctor
# Explicit installation using Homebrew bottles and rustup's verified distribution.
capture/desktop/scripts/mac/setup.sh --disposable provision
# Fresh private short TMPDIR stays alive for the whole child; flags pass intact.
capture/desktop/scripts/mac/setup.sh --disposable run -- python3 enablement/owner_lanes.py --lane capture --run
```

Provision installs Node matching Desktop CI's major (currently 22), ffmpeg
(including ffprobe), stable Rust with rustfmt, and Apple's Metal component.
Homebrew/rustup versions resolve at provisioning time; this is a repeatable
prerequisite check, not a frozen dependency image. No shell profiles, rustup
defaults, Xcode selection, permissions, Gatekeeper or app launches are changed.
Doctor checks Rust/rustfmt, Node/npm, ffmpeg/ffprobe and Xcode/Metal before builds.
A failure requires fixing the named prerequisite, then rerunning; no build starts.
Run selects the Metal toolchain and stable Rust unless TOOLCHAINS or
RUSTUP_TOOLCHAIN is explicitly supplied; provision
installs and checks stable without changing global defaults.

The executable recipe wraps existing verification without changing CI admission
or full native suites. On Clanker put each heavy command through the existing
`/home/simon/.local/bin/t3-build` once, never nested, respecting T3_BUILD_JOBS.
Installer workflows still require `target/debug/apparent`: rebuild that binary
before installing; `cargo test` alone does not update it. This helper does not
change CARGO_TARGET_DIR or the installer contract.

Linux regression tests simulate tool responses and reject Linux provisioning;
they do not prove Darwin execution. Pending acceptance: run provision and doctor
on an authorized disposable Mac, then a harmless child that verifies PATH and
TMPDIR, followed by the existing Mac build/test gates. Each dependency installation has a bounded ten-minute timeout; the caller must
still fit the complete setup and checks inside its authorized session deadline.
No TCC or capture proof is
claimed. Independent review remains required before integration.
