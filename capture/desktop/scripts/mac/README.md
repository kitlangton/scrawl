# Disposable Mac prerequisites

## Packaged Settings smoke

`packaged-smoke.cjs` is a separate, read-only smoke check after installation. On
an ephemeral GitHub-hosted Mac, set `APPARENT_PACKAGED_SMOKE=1`, `APPARENT_APP_DIR`
to the candidate installed by `packaging/install-app.sh` and
`APPARENT_SMOKE_OUTPUT` to a fresh evidence directory, then run
`node capture/desktop/scripts/mac/packaged-smoke.cjs` from the candidate checkout.
The frontend's locked Playwright dependency must already be installed. The check
verifies the package signature, launches the installed Rust app in a synthetic
private HOME, and opens/closes the packaged Storage Settings twice using its
packaged Electron runtime. It checks rendered location, real Rust readback and
executable/app/profile identity, retaining screenshots, logs and a JSON result.
It has a 75-second operation budget plus bounded teardown; successful fixtures are removed, failed fixtures
remain private for investigation. No personal/self-hosted Mac is admitted.

This checks package assembly and the bounded Settings child lifecycle. It does
not exercise menu routing, update installation, recording, permissions, overlays,
focus or shortcuts; those retain their existing targeted native acceptance.
It does not certify Apple Silicon behavior when run on Intel. CI keeps compilation,
this smoke step and deeper native scenarios distinct, but still runs the full
platform suites. No selection, proof reuse or release-admission rule changes.
Its process supervisor and cleanup helpers are canonical in
[enablement/native](../../../../enablement/native/README.md). This Desktop
entrypoint retains package admission, Storage assertions and private evidence
handling, and temporarily re-exports the shared helpers for existing consumers.
Run shared lifecycle contracts with
`node --test enablement/native/owned-processes.test.ts`.
Desktop host regressions run with
`python3 -m unittest discover -s capture/desktop/scripts/tests/mac -p test_packaged_smoke.py`.

## Toolchain setup

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
