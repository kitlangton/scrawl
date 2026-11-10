# Apparent for Hermes

Native plugin source candidate, version **0.3.0**. Paste an Apparent recording handoff into
Hermes and press Send. `apparent_inspect` returns the spoken instructions,
timestamped interaction context and actual contact-sheet pixels together.
Pass `frame` to read a numbered keyframe when a detail needs closer inspection.
The bundled `apparent:recordings` skill guides Hermes to inspect the images,
investigate uncertainty and act under its existing approvals.

The staged package also adds **Record with Apparent** beside the composer.
Click it, confirm **Record and send when finished** in Apparent, then talk, point
and draw. Finish submits the recording to the original conversation, starting
or queueing an agent turn. Switching chats never changes that destination.
Cancel sends nothing. New unsaved chats are refused. Native confirmation prevents
an external URL from silently starting capture; normal permissions still apply.

This is a source candidate for Hermes Desktop on macOS/Linux, not a catalog
release. Stock 0.23.0 Desktop routing passed with a mock model; separately, a real
Hermes model read the images from a native Linux recording. The installed macOS
app and Linux AppImage passed normal URL launch, confirmation, screen consent,
GUI Finish and signed return on isolated synthetic desktops with the microphone
disabled. The Mac bundle also retained the original app and window context.
Fresh onboarding and microphone capture on both platforms remain release gates.

## Desktop package and routing

The desktop half is TypeScript. Hermes loads `desktop/plugin.js`, so generate a
package before installing it to get the button:

```sh
npm ci --ignore-scripts --prefix capture/desktop/ui/frontend
node --experimental-strip-types hermes/desktop/package.ts /tmp/apparent-dev-hermes-JOB/plugin
hermes plugins validate /tmp/apparent-dev-hermes-JOB/plugin
```

Use a new private per-job `/tmp/apparent-dev-…` directory. This build needs Node
and the frontend's locked dependencies, without Rust or cached Rust outputs.
Install the staged package through Hermes's normal reviewed install flow, then
enable its Desktop and Agent halves. Source/Git installation alone has no
generated desktop entry. This command publishes no catalog artifact.

Recorder adapters that preflight `capture state` must use
`captureCanStart` from `desktop/controller.ts`: both `idle` and `completed`
allow a new recording. `completed` is the saved recording's Paste controls;
selecting, starting, recording and processing remain occupied. This preflight
does not replace the native Start request's atomic admission check.

Requires SDK support for `focusedSessionOwner`, `focusedSessionId`,
`profileRoutes`, `requestProfile`, `retainProfile` and composer actions, plus a full Apparent
build containing the native bridge. Older SDKs fail before recording. The OS
must have the installed [Apparent URL handler](../capture/desktop/docs/native-launch.md).
The plugin never registers handlers or enables `nativeLaunch`.

Stock 0.23.0 refuses custom app schemes through its external-link API despite
the SDK documentation. For a local conversation, the button therefore uses
the public session-scoped `shell.exec` API to ask `open` (macOS) or `xdg-open`
(Linux) to open only the validated Apparent URI. It rechecks the original
route, runtime and session first. Hermes retains its local-terminal safety
checks; sandboxed/remote terminals can refuse. No shell settings, handler
registration or Hermes source are changed. Stock remote connections require
the existing manual recording-and-paste flow.

The launch URL carries a public random request ID and loopback port, not a signing
key, gateway credentials or session IDs. Apparent publishes a separate signing key
in a mode-0600 file inside a mode-0700 user runtime directory. The plugin reads it
once through the original local session's checked `shell.exec` API; no folder grant
is needed. The key stays in memory and is removed on normal helper exit. It never
appears in URL/process arguments, HTTP requests or plugin storage. Authenticated
state reads, signed responses and a separate authenticated acknowledgement prevent a listener that knows the
public request ID from forging completion. A bounded local endpoint returns only this
recording's status
and saved reference. It cannot control capture or read arbitrary files. It binds
IPv4 loopback, checks Host/origin/signature and expires after one hour. Keep Hermes
and the bridge open until handoff completes. Native confirmation is required
because an external URL does not establish the caller's identity.

Hermes retains the exact session, route mapping and gateway instance/profile/epoch,
checks them again, resumes that exact session on a retained socket, then calls
`prompt.submit` through that connection. Changed
runtime identities or mappings fail closed. Missing sessions never fall back
to titles or new chats. A lost reply gets one retry with the same `input_id` and
text, using admission deduplication. Further failure retains the reference; the
composer button checks/retries that same handoff. The command palette's
**Apparent: discard pending handoff** abandons delivery without deleting a recording.
Reload replays neither Start
nor a new message identity. Native replay receipts hold only consumed tokens
and creation times, not media or transcripts.

Remote delivery needs an existing Connect reference. Local paths are refused for
remote routes; uploads/read grants are never enabled automatically. “Recording
sent” means gateway admission, not model vision or a finished agent turn.
Disabling the plugin removes its control; manual paste remains the fallback.
Remove the native bridge only after installed plugins no longer consume its URL
and `capture handoff` contracts.

## Setup and use

Install a compatible full Apparent CLI on the **Hermes backend host** using the
[official agent installation guide](../capture/agent/docs/installation.md).
The existing app includes that CLI. The recording computer needs the full
recorder and its normal onboarding; a headless receiving backend needs only
the CLI and its runtime libraries, without capture permissions. Reuse an
existing installation. Installing this plugin does not install or launch apps.

For development, copy this directory into the active Hermes profile's
`plugins/apparent/` directory, run the validation below, enable it with
`hermes plugins enable apparent`, then start a fresh conversation. Dependencies
are Python 3.11+, stock Hermes >=0.23.0 and Pillow >=10.2,<13. If the executable is not
on PATH, set the plugin's `executable` setting to its absolute app CLI path
through Hermes's plugin settings. No global Apparent settings are rewritten.

Paste the handoff generated by Apparent. The tool accepts an explicit absolute
`.apparent` bundle path (also its `apparent.json`) or a lowercase 32-hex Connect
ID, rather than guessing the latest recording. Local paths must be readable on
the receiving host. It uses the full installed `apparent` CLI:

```sh
apparent inspect /absolute/recording.apparent
apparent inspect /absolute/recording.apparent --json
```

For Connect it first runs `apparent fetch <id> --out <private-path> --wait 120`,
using the CLI's configured origin and existing read grant. Missing read approval,
pending/expired recordings and CLI failures remain actionable errors. The
plugin neither enables uploads nor reads credential stores. Follow the normal
CLI read-approval flow when needed. A 150-second subprocess deadline bounds
each CLI invocation; `--wait 120` is the fetch readiness budget.

## Evidence and privacy

Image reads stay inside the selected bundle and reject traversal, symlinks and
nonregular files. Inputs are bounded to 8 MiB and 25 million pixels; output is
JPEG with a 1600-pixel maximum edge. Original files are unchanged. Missing or
malformed visuals preserve clear transcript instructions and report uncertainty.
An image envelope alone does not establish that the model actually saw it.

Each Connect call downloads to a new mode-0700 temporary directory and removes
its snapshot on success or failure, once result pixels have been embedded. Use
the original ID for another keyframe request; this downloads again. Snapshot
paths in the CLI summary no longer exist after the call. Temporal zoom or a
separate text-only-model vision route can use the existing CLI to fetch into an
agent-owned private directory, inspect/zoom there, then clean up that download.
Local bundles remain available for normal frame/zoom/vision tools.

The plugin runs bounded subprocesses and reads only the explicitly selected
recording. Connect network/authentication belongs to the existing CLI. Loading
registers the tool, skill, message hook and desktop control; it starts no new
capture. An in-flight handoff can resume with its original destination and
message identity. No settings or credentials are rewritten.
Removing the plugin does not remove Apparent or local recordings. Private
transcripts and images must stay out of public issues and uploads.

## Verification and distribution

```sh
python3 -m unittest discover -s hermes/tests -v
hermes plugins validate ./hermes --install-deps
hermes plugins doctor ./hermes --ci
```

These checks need no Rust build. The repository check workflow runs the focused
suite on Linux and macOS. Integration proof separately exercises an installed
Apparent CLI and Hermes registry/model. Python is the explicitly authorized
Hermes integration exception; this package imports no application internals.

Plugin versions are independent of app versions. A future `hermes-v<version>`
tag and catalog PR must identify the exact tested SHA (`subdir: hermes`), with
the same version as `plugin.yaml`. No catalog submission or self-updater is
part of this slice. [#837](https://github.com/tryapparent/apparent/issues/837)
tracks the broader experience.
