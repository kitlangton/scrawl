# Install the official CLI for a recipient agent

For a fresh ChatGPT or other Linux agent session, install the **latest published,
verified Linux preview**, then inspect the ID in the user's paste. No desktop launch,
recording permission, speech model or upload consent is needed.

## Linux quick start

Requires Linux Ubuntu 24.04+ (x86_64 or aarch64), Node.js 22.18+,
`curl`, CA certificates, and runtime libraries `libasound2t64` and `libxcb1`.
If a prerequisite is missing, provision it through the environment's normal package
manager where authorized. A sandbox that prohibits downloaded executables cannot
use this CLI; report that boundary rather than launching a desktop app.

```sh
(
set -eu
apparent_setup_dir=$(mktemp -d)
trap 'rm -rf -- "$apparent_setup_dir"' EXIT
curl --fail --location --proto '=https' --proto-redir '=https' https://raw.githubusercontent.com/tryapparent/apparent/main/capture/agent/scripts/install-cli.ts --output "$apparent_setup_dir/install.ts"
node --experimental-strip-types "$apparent_setup_dir/install.ts"
export PATH="$HOME/.local/bin:$PATH"
apparent inspect <capture-id>
)
```

The installer reads the official GitHub release list on every invocation,
selects the host architecture and verifies SHA-256 before executing the AppImage.
It extracts the complete tree without FUSE and tests headless CLI startup. Linux previews publish through the existing release-verification gates; “latest” means the
newest **published Linux build**, not an arbitrary main commit or Mac release.
Downloaded CLI versions remain under `~/.local/share/apparent-cli/`. The installer
atomically updates its own `~/.local/bin/apparent` link and preserves another
program at that path, printing the exact installed command to use instead.
Rerun installation to get a newer build; version numbers alone do not identify
previews. Installing a skill does not install the executable.

## First private capture

If inspection asks for read approval, run (keep `~/.local/bin` on PATH, or use the installed absolute command):

```sh
apparent connect login
```

Give the user the printed verification URL and code. The command waits for their
approval; keep it running while they approve, then retry `apparent inspect <capture-id>`.
Server defaults to the configured origin, or `https://hosted.tryapparent.com` on a
fresh machine. Scope defaults to read; the verification page is printed without
trying to open a browser in the agent's environment. `--browser` explicitly opens
it there. `--server <origin>` and `--scope upload --consent-upload` remain explicit
overrides. Never grant upload consent merely to inspect a capture.

Read approval includes the receiving agent's public encryption key; its private
key remains local. No experiment edit is needed for CLI read access. For encrypted
captures the sender must have included this approved recipient when preparing the
upload. Approving a new receiver cannot unlock ciphertext already encrypted for
another receiver. Do not copy another environment's credentials or private keys.
Ephemeral ChatGPT environments can require fresh approval; installing a CLI cannot
carry private account authorization between them.

If login fails before printing an approval link, report its transport stage,
curl exit code and timing. Do not infer a missing capture or request repeated
account approval. A proxied agent can reach HTTPS even when direct DNS fails;
preserve its proxy settings and never print their values. An unauthenticated
GET to the official `/v1/account` returning 401 proves API reachability, not that
the authorization POST or capture fetch succeeds. Retry login deliberately only
after connectivity is restored. Installer success alone does not prove login readiness.

`inspect` downloads a hosted ID to `./<id>.evidence.apparent`, waits up to 120 seconds for
readiness, and prints the summary. It never overwrites existing output: inspect a
previous download using `apparent inspect ./<id>.evidence.apparent`. For custom paths/waits,
use `apparent fetch <id> --out <new-path> --wait <seconds>`.
Open `contact-sheet.png` and relevant full frames before acting. A summary alone
is not visual inspection. See [Connect](../../../connect/docs/hosted-client.md).

## macOS and skills

On Mac use the installed Apparent.app CLI or follow the official
[packaging instructions](../../desktop/docs/release.md); this Linux installer does
not install a Mac app. Settings → Agents can install its command-line link.
Windows is unsupported. Desktop/clipboard commands and media commands have
additional platform dependencies; see [Linux](../../desktop/docs/linux/linux.md).

Optionally install the [capture-reading skill](../skills/apparent/SKILL.md):

```sh
bunx skills add https://github.com/tryapparent/apparent/tree/main/capture/agent/skills/apparent
```
