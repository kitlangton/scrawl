---
name: setup
description: Set up or repair Apparent's full recorder and CLI for Hermes; resume onboarding and verify a recording handoff.
---

Own the setup through a working recording handoff. Do the installation work with
available agent tools; do not hand the user a list of terminal commands. Leave
OS permission choices and the recording itself to the user. Respect Hermes
approvals. Never run interactive setup unattended or bypass a declined approval.

## Check and resume

Run `apparent_status` first and after each repair. Its checkpoints are observations
on the **Hermes backend**, not saved setup flags. They never prove permissions,
onboarding completion or a successful recording. The tool makes no changes.

Establish where Hermes runs and where the user records using available session
and host context. Ask one focused question only if still unclear. When the backend
is remote, do not install a GUI there or claim it is the Desktop computer. Use
existing authorized tools on the recording computer; otherwise explain that exact
access blocker. A remote backend may need only the receiving CLI.

- `needs_cli`: locate an existing application before installing another copy.
- `needs_cli_repair`: inspect the selected executable/error before replacing it.
- `needs_cli_update`: inspect the missing command; select a compatible official
  release. A working version string alone is insufficient.
- `needs_recorder_check`: the GUI may be closed, inaccessible or not installed.
  Inspect the existing application and its launch error; do not assume absence.
- `recorder_busy`: preserve the active workflow. No restart, replacement or second
  recorder. Resume setup once that work is finished.
- `needs_recording_check`: CLI and GUI respond; finish onboarding and verify the
  user journey below. This is not a Ready result.
- `unsupported_platform`: explain that Apparent supports macOS and Linux.

Do not read credential stores or dump Apparent's config to diagnose setup.
Never rewrite onboarding progress, reset permissions or enable uploads.

## Install the full application when needed

Use the official releases and owner instructions:

- Releases: https://github.com/tryapparent/apparent/releases
- macOS: https://github.com/tryapparent/apparent/blob/main/capture/desktop/docs/release.md
- Linux: https://github.com/tryapparent/apparent/blob/main/capture/desktop/docs/linux/linux.md
- Receiving CLI only: https://github.com/tryapparent/apparent/blob/main/capture/agent/docs/installation.md

Read the applicable current instructions before acting. Resolve the official
published release for the machine's OS/architecture; prerelease channels differ
between macOS and Linux. Record the selected immutable release tag, exact asset
URL and expected SHA-256 before executing anything. Fetch the matching checksum
from the same official release and verify the downloaded artifact. If a matching
artifact/checksum is unavailable, report that blocker. Do not build from source,
fetch a random fork, disable signature checks or substitute another product.

Reuse compatible installed applications. Never overwrite another installation,
its command link or a running app. If an update is necessary, let an active
capture finish and use the application's documented update route. Preserve user
settings and recordings; do not change startup registration or upload consent.

On **macOS**, use the official signed/notarized DMG, install the app into the
documented Applications location and launch that installed copy with `open`.
Do not launch from the mounted DMG, remove quarantine attributes or ad-hoc sign
the app. The preview has a distinct identity and shares Apparent's data: do not
run it beside production. Its CLI is inside the app; set the plugin's `executable`
to that absolute path through Hermes's supported plugin settings when needed.
Do not force the preview to take over the production `apparent` command.

On **Linux**, use the complete official architecture-matched AppImage in a stable
user-owned location such as `~/Applications`. Verify it before making it
executable. Follow the documented runtime/FUSE requirements and normal approved
package installation flow if necessary. Launch `app` from the graphical session,
without `--record`. The headless recipient installer is **not** proof of recorder
setup. Settings > Agents can link the app's CLI; if the command name is already
owned, preserve it and configure the plugin's absolute executable path instead.
Do not improvise display variables or install a GUI onto a headless server.

## Existing onboarding, then a real handoff

Open the installed app normally, without starting capture. Let its existing
onboarding resume; do not create another permission wizard or edit progress.
Ask the user only for the next required native action. Explain a denied permission
or required relaunch specifically; do not repeatedly launch or prompt.

When the user is ready, ask them to use Apparent's own recording shortcut on
harmless content, say a short instruction, point/draw, stop, and paste its normal
handoff into this Hermes chat. Do not capture their screen autonomously. Preserve
their existing Connect/local routing; local handoffs must be readable on the
backend. Never upload a recording just to make remote setup pass.

Call `apparent_inspect` on that explicit reference. Check the expected spoken
words and actually inspect its contact-sheet pixels. Use `apparent_frame` or
`apparent_zoom` if context is unclear. A filename, CLI help, successful tool
registration or a multimodal envelope alone does not prove model vision.
If Connect asks for read authorization, follow the official receiving-CLI flow,
keep the approval process alive, and never enable uploads to read.

Only report **Apparent is ready** after the user has completed the relevant
onboarding/recording interaction and Hermes has read both transcript and images.
Otherwise state the exact unfinished checkpoint and next action. On resumption,
recheck actual state and continue that step; don't reinstall or repeat completed
onboarding. Report text-only availability separately when vision remains blocked.
