# Install the official CLI for a recipient agent

The CLI can read a capture on a headless machine without launching the desktop app.
Installing an agent skill installs instructions, **not** the `apparent` executable.
Start here if a received capture asks you to run `apparent fetch`.

## Linux (Ubuntu 24.04+, x86_64 or aarch64)

Use the official [GitHub releases](https://github.com/tryapparent/apparent/releases).
Linux `linux-main-<commit>` releases are **preview** builds of main, not stable releases;
Mac previews and Mac appcast availability do not describe Linux availability.
The example below pins the official 0.3.13 preview. To update, choose one Linux
release and replace **both** `tag` and `version` with its exact values. Confirm it
contains the matching AppImage and `.sha256` asset; do not select an arbitrary glob
or use GitHub's latest release (which may be for another platform).

Prerequisites: `curl`, CA certificates, `sha256sum`, and the Ubuntu runtime
libraries `libasound2t64` and `libxcb1` (plus their dependencies). Minimal
`ubuntu:24.04` containers omit these libraries even for headless commands because
the existing app binary links them. Have the environment provision these Ubuntu
packages first; the user installation below does not change system packages.
If you cannot provision prerequisites, report the missing library rather than
launching the GUI or requesting recording permissions.

Run this Bash block once prerequisites are available. It needs
no sudo, FUSE or desktop session once runtime prerequisites are available. Use a new empty directory
under your writable home; it deliberately refuses to overwrite an existing install.

```bash
set -euo pipefail
case "$(uname -m)" in
  x86_64) arch=x86_64 ;;
  aarch64|arm64) arch=aarch64 ;;
  *) echo 'Supported Linux architectures: x86_64, aarch64' >&2; exit 1 ;;
esac
tag=linux-main-e758a9b8a50891718498b6ad6ae7b76072647f26
version=0.3.13
asset="Apparent-${version}-linux-${arch}.AppImage"
install_dir="$HOME/.local/share/apparent-cli/$tag-$arch"
mkdir -p "$(dirname "$install_dir")"
mkdir "$install_dir"
cd "$install_dir"
base="https://github.com/tryapparent/apparent/releases/download/$tag"
curl --fail --location --proto '=https' --tlsv1.2 "$base/$asset" -o "$asset"
curl --fail --location --proto '=https' --tlsv1.2 "$base/$asset.sha256" -o "$asset.sha256"
sha256sum --check "$asset.sha256"
chmod +x "$asset"
./"$asset" --appimage-extract >/dev/null
# Keep the entire extracted tree, including usr/lib; do not copy just the binary.
cat > apparent <<'WRAPPER'
#!/bin/sh
root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export LD_LIBRARY_PATH="$root/squashfs-root/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$root/squashfs-root/usr/bin/apparent" "$@"
WRAPPER
chmod +x apparent
export PATH="$install_dir:$PATH"
apparent --version
apparent connect login --help
apparent fetch --help
```

Run `pwd` and add that installation directory to your shell's PATH for future
sessions, or invoke its `apparent` wrapper by absolute path. Keep that wrapper
beside `squashfs-root`; moving it alone breaks library discovery. Re-run these
instructions with a different release directory to update; switch PATH only
after checking the new version. Extracted CLI installs are managed this way,
not through the desktop AppImage updater.

This extraction path avoids mounting via FUSE and bypasses the AppImage launcher.
It does not establish why a particular sandbox's ordinary launcher fails. A host
that forbids executing downloaded binaries remains unsupported by these commands;
installation success does not prove ChatGPT fetch compatibility.

## Read a remote capture

Use the server origin and capture ID supplied by the sender (the values below
are placeholders). Login prints a verification URL/code for a human to approve:

```sh
apparent connect login --server https://api.example.com --scope read --no-browser
apparent inspect <32hex-id>
```

Read login/fetch needs network access, `curl`, system SHA-256 tooling and separate
read approval. It requires no recording, screen/input permissions, speech-model
download or upload consent. Do not enable uploads or reuse a sender's token to
read. The output path must not exist and its parent must exist. Fetch validates
and publishes the complete bundle; open its contact sheet and frames visually
before acting. Service availability, authorization and sandbox network policy
remain separate from installation. See [Connect instructions](../../../connect/docs/hosted-client.md).

Other commands have different dependencies: `zoom`, `frames` and video export
need media processing; clipboard and desktop capture need platform services.
For recording and desktop setup see [Linux](../../desktop/docs/linux/linux.md).

## macOS and skills

Use a Mac release matching Apple silicon and its documented macOS minimum.
Install the app through the [official packaging instructions](../../desktop/docs/release.md).
The executable is `Apparent.app/Contents/MacOS/apparent`; Settings → Agents can
install a CLI link. Linux AppImages cannot run on macOS. Windows is unsupported.

Optionally install the [capture-reading skill](../skills/apparent/SKILL.md):

```sh
bunx skills add https://github.com/tryapparent/apparent/tree/main/capture/agent/skills/apparent
```

This installs neither the binary nor recording permissions.

A bare Connect ID passed to `inspect` downloads to `./<id>.apparent`, waits up to 120 seconds for readiness, and prints the summary. Existing output is never overwritten; use `apparent inspect ./<id>.apparent` to inspect a previous download, or explicit `apparent fetch <id> --out <new-path> --wait <seconds>` followed by local inspection for custom transfer options.
