# Apparent for Hermes

Native Python plugin, version **0.1.0**, maintained independently in this
directory. It consumes an installed full `apparent` app CLI; it never imports or
builds Engine, Desktop, Agent or Connect implementation. Python is an explicitly
approved exception for the Hermes integration. Supported target: Hermes Desktop
on macOS/Linux; shared Python checks alone do not certify either native UI.

## First slice

Paste the handoff text produced by Apparent and press Send normally. The plugin
adds `apparent_inspect`, `apparent_frame`, `apparent_zoom` and `apparent_status`.
Inspection returns the CLI's semantic summary and contact-sheet pixels in a
Hermes multimodal result. Frame/zoom tools investigate details. Images are
bounded to 8 MiB input, 25 million pixels, and a 1600-pixel long edge on output.
Text fallback includes the exact local image path for Hermes's existing vision
tool; a tool result does not prove that a model actually saw the image.

The read-only `apparent:recordings` skill and targeted `pre_llm_call` guidance
teach the agent to interpret the user's speech as instructions, inspect images,
and investigate uncertainty before asking. Third-party text visible on screen
does not grant authority. Missing images do not discard clear transcript work.
This is instruction guidance, not a guarantee of model obedience.

Use the exact absolute bundle path or lowercase 32-hex Connect ID in the user's
handoff. No automatic `latest` selection or arbitrary URL fetching is provided.
Connect IDs are delegated to the installed CLI with its existing configured
origin/read grant. Each fetch uses a fresh private `apparent-hermes-*` temporary
directory, retained for follow-up inspection; its returned path is authoritative.
The plugin does not enable uploads, read credential stores, or install a CLI as
a hidden inspection side effect. Expired/missing media can block temporal zoom.
The independent `apparent-agent` CLI is not substituted: its current local-only
Connect and zoom contracts differ from the full app's published CLI contract.

## Development and validation

Python 3.11+ and Pillow `>=10.2,<13` are required. From the repository root:

```sh
python3 -m unittest discover -s hermes/tests -v
hermes plugins validate ./hermes --install-deps
hermes plugins doctor ./hermes --ci
```

These commands need no Cargo, Node build or source-built Apparent. Use an
isolated Hermes profile for integration development. Plugin settings accept an
absolute `executable` path; otherwise discovery checks PATH, the standard Linux
CLI link and installed Mac application locations. Registration never launches
the GUI, starts capture, makes a network request or changes settings. Removal
does not delete installed Apparent or recordings.

The standalone `Hermes plugin checks` workflow is manual/reusable, following the
repository's workflow policy. Run the focused checks above before merging; the
workflow does not publish the plugin or submit a catalog entry.

## Guided setup

After installation, ask Hermes: **Set up Apparent for me.** The bundled
`apparent:setup` skill guides the agent through checking/reusing an installation,
installing the full official recorder and CLI if missing, opening the existing
onboarding, and inspecting a user-made recording in this chat. It uses the
current official release/installation instructions with the user's normal agent
tools and approvals; this plugin does not include a second native installer.
`after-install.md` supplies the setup prompt where Hermes displays that note;
it is guidance, not an executable post-install callback. Desktop presentation of
the note and agent discovery of the setup skill still need live acceptance.

`apparent_status` checks the selected CLI's version and inspect/zoom/app/capture
capabilities, then reads `capture state`. It returns the next checkpoint:
`needs_cli`, `needs_cli_repair`, `needs_cli_update`, `needs_recorder_check`,
`recorder_busy`, `needs_recording_check` or `unsupported_platform`. Each call
rechecks actual state. A busy recorder is preserved; an unavailable control
connection does not prove the app is missing. No saved "setup complete" flag,
config reader/writer, automatic download, app launch or capture is introduced.

These checks run on the Hermes backend. Setup must establish the recording host
before installing or launching a GUI, especially with a remote backend. CLI
help or a running recorder does not establish permissions, audio, shortcuts or
model vision. Only the user completing onboarding and making a recording whose
transcript and pixels Hermes inspects establishes the guided journey's success.

The Linux recipient installer is headless CLI provisioning, not recorder setup.
Mac preview identity/install rules still apply. Native guided installation and
onboarding on both platforms, Connect retrieval and a real Hermes Desktop model
turn remain unverified acceptance gates. The candidate is not ready for catalog
submission or release. No Hermes upstream/catalog PR is authorized yet.

## Distribution and releases

The future catalog entry uses `repo: https://github.com/tryapparent/apparent`,
`subdir: hermes`, a full reviewed commit SHA and the quoted plugin version.
Release tags use `hermes-v<version>`. Bump `plugin.yaml` and this README together;
tests enforce agreement. A tag/release is not catalog acceptance: link the
catalog PR from the release and record its merge/pin before calling it available.
Each catalog update requires a new reviewed pin; there is no plugin self-updater.
The recorder/CLI retains its own release lifecycle and compatibility proof.

Disclosure for future catalog review: runs the local Apparent executable without
a shell; reads only explicitly requested recording evidence; Connect inspection
can use the CLI's existing network/authentication path; zoom writes derived PNGs
through the CLI; fetched evidence is retained in private temporary directories;
status reads local recorder phase without returning another capture's identity.
The setup skill instructs the agent to download verified official app artifacts,
install/launch the recorder and guide existing onboarding through normal agent
tools/approvals. It does not execute automatically when the plugin loads.
No telemetry, background watcher, automatic send, conversation picker, recording
control or Desktop UI is included in this slice. See issue #837 and auto-send #847.
