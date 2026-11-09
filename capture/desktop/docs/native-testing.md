# Linux native capture smoke recipe

This is one supported **guest scenario**, not another VM controller:
`capture/desktop/scripts/linux/gui/capture-smoke.py`, run by `capture/desktop/scripts/linux/gui/gui-vm.py`. It starts the
candidate GUI, creates a changing synthetic X11 window, saves one normal GUI
screen capture, then starts and cancels a second. Both requests use persisted,
32-hex-character request IDs; all stop/cancel/inspect calls use those IDs only.
There is no messaging, paste into a composer, audio playback, tray targeting,
OCR, simulated picker clicks or acknowledgement protocol.

## Source-backed limits

At baseline `6f043ff4ab4f4bdfa7e180c97ef7c37a397ff2ef`,
`CaptureCommand::Start` (`composition/cli.rs`) has no target/window option. The GUI's
`agent_request` calls `record_screen_now` (`capture/desktop/mod.rs`). Do not add `--window`
to this command or substitute the separate timed `record --window` recorder:
that would test a different path. On Omarchy/Hyprland the normal screen path
uses ScreenCast/PipeWire and can require native portal consent. X11 window
capture support elsewhere in the backend does not change this CLI contract.
See [capture control](../../capture/engine/docs/capture.md#agent-control-of-the-running-gui) and
[Linux capture limits](linux.md#picker-and-portal-limits).

The scene uses Python's stdlib ctypes and `libX11.so.6`, not Tk, a browser,
xterm or a package install. It changes background, text and a white block twice
per second. `scene.json` identifies its XID/title/requested size. XSync and the
scene process remaining alive are not proof of visible or selected pixels.

## Preflight (doctor), before spending a guest run

### Prepare the decoder in a new template

The normal Omarchy/Arch GUI template must include **`gst-libav`** for GStreamer
H.264 decoding, plus **`ffmpeg`** with `libx264` for the smoke sample. Encoding a
movie successfully does not prove decoding. Install these **inside an owned
template-preparation guest**, never on the host or during an ordinary test:

```sh
sudo pacman -Syu --needed gst-libav ffmpeg
pacman -Q gst-libav gstreamer gst-plugins-base-libs ffmpeg
gst-inspect-1.0 avdec_h264
```

For an offline derivative of a pinned template, use its existing coherent
pacman database to resolve `gst-libav` (`pacman -Si` / `pacman -Sp`). Obtain that
exact package and detached signature, verify against the guest's Arch keyring,
and install with `pacman -U` in the owned preparation image. Require all package
dependencies to be satisfied; do not refresh only the database (`pacman -Sy`),
skip signatures/dependencies, or mix GStreamer versions. For example, a template
with `gstreamer` and `gst-plugins-base-libs` 1.28.8-1 needs `gst-libav` 1.28.8-1.
If unavailable, rebuild a coherent template instead of substituting a newer plugin.

Keep the published template immutable: derive a separately named, private image
from the stopped sanitized source, install there, stop all preparation writers,
and publish it as a **standalone** qcow2 with its matching stopped firmware.
Preserve the source and its hashes; do not replace a shared image or another
worker's guest. Retain package versions/signature evidence, new disk/firmware
hashes and the existing sanitization provenance. Keep the test-only account/key,
clean home and no product autostart. Select the prepared image explicitly with
`GUI_TEMPLATE`; consumers coordinate adoption rather than changing a live path.

Before accepting that derivative, run the smoke recipe below in a **fresh
disposable overlay**. Its `h264_decoder` gate generates a 160×90 H.264/YUV420P
sample and invokes the actual staged candidate's `decoder-check` under the same
runtime environment as the GUI. It requires native sample dimensions and retains
both command streams. Missing tools, a failed decode or wrong dimensions stop
before launching the scene/GUI or requesting a capture. Factory discovery and
host FFmpeg decoding alone cannot pass this gate.

Keep the original decoder-less template for explicit warning/fallback checks
in separate disposable runs. Those checks should expect `decoder-check` to fail
and inspect the warning/fallback; they are not normal smoke passes. Existing
`decoder_fallback_keeps_moving_target_in_agent_images` and decoder-warning tests
remain separate product regression coverage. AppImage decoder acceptance remains
the separate `check-appimage-decoder.sh` packaging check.

### Stage the candidate

Use a sanitized, powered-off, standalone qcow2 template with matching stopped
OVMF variables, a test-only SSH key and a desktop-autologin test account. It
must contain no personal material or autostarted Apparent/Scrawl. Do not use
X1, the persistent live VM disk, or another worker's resources. Coordinate RAM,
CPU and cgroup capacity before launching. The existing runner owns the overlay,
firmware copy, SSH forwarding, Unix VNC/QMP sockets, expiry and teardown.

Set `GUI_TEMPLATE`, `GUI_TEST_KEY`, `GUI_TEST_USER`, `CANDIDATE` to your owned
inputs and `RUN` to a **new short private** path under `$TMPDIR`. Keep candidate
build provenance/source revision beside the run; the scenario records the
actual executable SHA-256, not an invented source-to-build attestation. A raw
ELF needs its runtime closure (runner stages sibling `lib*.so*`); an AppImage
needs its own complete runtime. Neither an old installed app nor `--help`
proves the candidate.

Run the launch-free checks before scarce build/VM admission. Use a separate
new private results directory for preparation, then pass its receipt to the
admitted run. The runner has no `doctor` subcommand.

```sh
python3 capture/desktop/scripts/linux/gui/test-gui-vm.py
python3 capture/desktop/scripts/linux/gui/test-capture-smoke.py
python3 capture/desktop/scripts/linux/gui/gui-vm.py --help
for tool in qemu-system-x86_64 qemu-img ssh scp systemd-run python3; do
  command -v "$tool" || exit 1
done
test -r /usr/share/OVMF/OVMF_CODE_4M.fd && test -r "$GUI_TEMPLATE/OVMF_VARS.fd"
test -r "$GUI_TEST_KEY" && test -x "$CANDIDATE"
systemctl --user show-environment >/dev/null
qemu-img info --output=json "$GUI_TEMPLATE/system.qcow2"
sha256sum "$CANDIDATE"
python3 capture/desktop/scripts/linux/gui/gui-vm.py \
  --template "$GUI_TEMPLATE/system.qcow2" \
  --ovmf-vars "$GUI_TEMPLATE/OVMF_VARS.fd" \
  --ssh-user "$GUI_TEST_USER" --ssh-key "$GUI_TEST_KEY" \
  --binary "$CANDIDATE" --scenario capture/desktop/scripts/linux/gui/capture-smoke.py \
  --results "$RUN-preflight" --isolated-network --preflight-only
```

Preflight checks required local files, candidate execute permission, dangling
sibling libraries and scenario syntax without executing the candidate or scenario.
Ordinary scenarios must parse as Bash. Python recipes must use the exact four-line
shell/Python header in `capture-smoke.py`; their Python body is compiled without
execution. A plain Python shebang fails before overlay creation.

The private receipt fingerprints candidate, sibling libraries, scenario, optional
audio, firmware, access key and runner/reaper bytes, plus the desktop/network
selection and template path/stat identity. `--prepared-inputs` rejects changed
inputs before overlay allocation after queueing. It also binds the library file
set, and staging verifies every transferred candidate/library/scenario/audio hash.
Keep source-to-build provenance separately. Template stat identity detects local
replacement; verify its published hash and provenance independently. Preparation
does not execute the package or validate guest ABI, NSS, dlopen or decoder closure.
Those checks still require the matching guest and normal package entry. For raw
ELFs, stage all required versioned libraries and dlopen modules as real files
beside the candidate. For AppImages, retain the normal launcher and existing
packaging encoder/decoder checks.

Require qcow2 with no backing file, stopped writers and sufficient KVM access;
`qemu-img info` alone does not certify sanitization, ownership or capacity.
Guest prerequisites: Python 3, libX11, working XWayland/DISPLAY, the candidate's
Vulkan/GStreamer runtime, a working user session bus, Hyprland and ScreenCast
portal/PipeWire. The scenario imports the desktop environment from
`systemctl --user show-environment`, checks `DISPLAY`/runtime, refuses an existing
`apparent.sock` or same-user Apparent/Scrawl process, and waits for real control
readiness. It checks actual H.264 decoding before capture, but does not install
codecs or repair a template.

The private output directory is also `HOME`, with private config/data/cache,
`APPARENT_CONFIG` and `APPARENT_CAPTURES_DIR`. The real desktop runtime and bus
remain available for portals/Hyprland. The HOME-based control socket must fit
Linux's 107-byte pathname limit; the scenario refuses longer paths before GUI
launch. For the baseline runner's `/home/simon/scrawl-gui-output-<32hex>` path,
using the output itself as HOME fits; adding a nested `home/` does not. A longer
guest username may require the runner owner to shorten its guest staging path.

Use `--isolated-network` **always** for this recipe. The app's normal launch can
attempt model/update downloads; no supported `transcriber: off` setting exists
in this baseline. A blocked download is not an audio/transcription test. No
speaker stimulus is used. This recipe makes no microphone-quality claim.

## Run and native consent

The runner defaults to `--desktop hyprland`. For a separately prepared GNOME or
KDE Wayland image, pass `--desktop gnome` or `--desktop kde`. It requires the
test account's compositor process and exported X11/Wayland/runtime environment,
Wayland session type and matching desktop identity before staging the candidate.
A GDM/SDDM greeter or another desktop is not readiness. The selected desktop is
recorded in `result.json`; this check does not establish product acceptance.
Use a stopped, sanitized standalone image/firmware pair with its preparation
provenance and hashes. The Omarchy templates contain neither GNOME Shell nor
KWin; selecting an option does not install or configure a desktop. The existing
capture smoke recipe below remains an Omarchy-tested recipe until separately
verified on those desktops. Keep compositor/version, multi-output and native
product evidence separate from driver contracts.

### Released GNOME environment (Clanker, 2026-11-08)

The read-only standalone template is
`/home/simon/VMs/apparent-gnome-noble-20261108/`: `system.qcow2`, matching
`OVMF_VARS.fd`, test-only `test_access` key and SSH user `scrawl`. Inspect its
`preparation.json`, `versions.tsv` and `template.sha256` before use. Disk SHA256:
`822c0c89fd0d84aac4a6eb53b30f46d3452dfcbe6252825d84513a88853d1da7`;
firmware SHA256:
`eb0a2e4955aa6549433e8f8371913916559660e9fd2571a248a9527bd653f232`.
The source is Canonical's Ubuntu 24.04 noble 20261017 image, verified against
its signed SHA manifest; full source hash and signing provenance are in the
preparation receipt.

Fresh environment qualification established same-user GNOME Wayland
(`DISPLAY=:0`, `WAYLAND_DISPLAY=wayland-0`), gnome-shell 46.0/libmutter 46.2,
two logical 1280×800 outputs at 1× and USB mouse EV_REL X/Y. Optional
`--outputs 2 --relative-mouse` requires runner commit `aaf681411` (PR #1082);
the default launcher is unchanged. StatusNotifierWatcher is absent, the account
has no broad passwordless sudo, and the image has no Apparent state/autostart or
personal data. This is environment qualification only: capture, scale/coordinates,
focus/ink, package and receiver acceptance remain unrun by this qualification.

Use `--desktop gnome --isolated-network` with an owned private per-job `--results`
directory; the runner creates disposable disk/firmware copies. Never open the
template writable or replace it. Admin changes require a separately owned
derivative. KDE is a separate template and is not supplied by selecting
`--desktop kde` on this image.

The current runner consumes the released private-staging fix from commit
`1278ea9a2` in PR #1082. It creates fresh guest staging/output directories exactly once
with mode 0700 regardless of guest umask, refusing collisions. Earlier environment
qualification remains valid at its stated scope; it does not prove app writes.
Do not weaken config/bundle ancestor validation or modify the template to repair
an incorrectly staged test. Preserve the actual exported `XAUTHORITY` when
launching the XWayland app under a private test HOME; KDE exports an authority
file under its runtime directory rather than necessarily `~/.Xauthority`.

The separate stopped KDE variant is
`/home/simon/VMs/apparent-kde-noble-20261108/`, with the same file names and SSH
user. Disk SHA256:
`56626132d5f210813798d16d07bfceb34e4a9eb387cbcd74cbd18daa2ac989c8`;
firmware SHA256 matches the GNOME entry above. Its preparation receipt records
the signed Ubuntu source and released GNOME parent. Fresh environment
qualification with runner `1278ea9a2 --desktop kde --outputs 2 --relative-mouse`
established KWin 5.27.11/Plasma 5.27.12, XWayland 23.2.6, two 1280×800 DRM
outputs at 1× and EV_REL USB mouse. Fixture-only locking/power management is
disabled; no product state or privilege grants were added. This is environment
proof only, with the same read-only/disposable-consumer rules and unrun product
gates as GNOME.

```sh
python3 capture/desktop/scripts/linux/gui/gui-vm.py \
  --template "$GUI_TEMPLATE/system.qcow2" \
  --ovmf-vars "$GUI_TEMPLATE/OVMF_VARS.fd" \
  --ssh-user "$GUI_TEST_USER" --ssh-key "$GUI_TEST_KEY" \
  --binary "$CANDIDATE" --scenario capture/desktop/scripts/linux/gui/capture-smoke.py \
  --prepared-inputs "$RUN-preflight/result.json" \
  --results "$RUN" --memory 3072 --isolated-network \
  --boot-timeout 180 --scenario-timeout 300 --job-timeout 660
```

Use the same `--desktop` in preparation and execution when selecting GNOME/KDE.
On Clanker, run the execution command once through `/home/simon/.local/bin/t3-build`
after measured capacity and ownership checks. Preparation itself allocates no VM.

The Python file has a tested shell/Python entry header because the existing
runner copies just one scenario file and invokes it with Bash. No sibling
scenario dependency needs staging. The runner supplies `SCRAWL_GUI_BINARY`,
`SCRAWL_GUI_OUTPUT` and `SCRAWL_GUI_LIBRARY_PATH`. Bash and system tools keep
their session library environment. Both maintained Python recipes set
`LD_LIBRARY_PATH` only for the exact candidate's GUI, CLI and decoder invocations.
Custom Bash recipes must do the same at each candidate call, for example
`LD_LIBRARY_PATH="$SCRAWL_GUI_LIBRARY_PATH" "$SCRAWL_GUI_BINARY" app`.
Update older private scenarios that relied on the runner exporting a global
`LD_LIBRARY_PATH`. Preserve exported `XAUTHORITY`, desktop bus/runtime and
normal AppImage entry when isolating HOME. For an already owned guest
with that same environment, the direct recipe invocation is:

```sh
python3 /path/to/capture-smoke.py --consent-timeout 40
```

Before starting the runner, have the parent operator ready to attach **only**
to this run's private `vnc.sock` through its existing VNC tooling. Follow
`scenario.log` for `save:` and `cancel:` consent messages. If a ScreenCast chooser
appears, visibly identify the synthetic scene and approve the output containing
it within 40 seconds. No automatic input is sent. A second chooser can appear
for the cancel take. If the scene is hidden, the wrong output is selected, or
consent cannot be performed safely, let the bounded run fail; do not click a
guessed target or claim pixel acceptance. A timeout inspects the same request
and cleanup cancels that owned pending/recording ID. It does not retry Start or
spend a third take. Consent-versus-backend causation remains unresolved until
native observation/logs distinguish them.

The recipe waits for the owned `recording` receipt **and** matching recording
state, records four seconds, sends Stop once and waits up to 60 seconds for that
ID's production `saved` receipt. It verifies the same saved path via Inspect,
checks that it is inside private captures, and records bundle paths/files.
The second capture records two seconds before Cancel, checks its cancelled
receipt by Inspect and requires no active ID in idle/completed state. Saved
means the successful finalizer's receipt, not exit zero, `latest`, a directory
scan or merely `completed` state.

## Retain, shut down, then inspect

The runner copies all guest output into `$RUN/artifacts` before teardown. Keep
the original report, command streams and bundles even when the recipe or outer
runner fails. An artifact-copy error or outer timeout requires the runner's
retention/recovery path; a bundle on a deleted guest is not retained evidence.
Verify the runner's exact owned QEMU/expiry/writable-state cleanup separately.

Only a successful trusted scenario plus a matching copied-file manifest permits
disk deletion. A failed, interrupted or transport-unknown scenario keeps its cold
disk and firmware, even if an initial copy matches: remote writers can outlive
the shell or SSH client. That copy is labeled `partial_snapshot`, never verified
final retention. Preserve the cold disk until recovery is explicitly complete.
The scenario cannot certify host VM teardown.

After shutdown resolve only the saved receipt's relative path, never `latest`:

```sh
BUNDLE=$(python3 - "$RUN/artifacts" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1]).resolve()
r = json.loads((root / 'capture-smoke-report.json').read_text())
p = (root / r['saved_bundle_relative_to_output']).resolve()
assert p.is_relative_to(root) and (p / 'apparent.json').is_file()
print(p)
PY
)
python3 -m json.tool "$BUNDLE/apparent.json"
ffprobe -v error -show_streams -show_format "$BUNDLE/recording.mp4"
ffmpeg -v error -i "$BUNDLE/recording.mp4" -f null -
```

Use `recording.mov` instead if that is the retained movie. Read `summary.md`,
visually inspect `contact-sheet.png` and every `frames/*.png` (and
`frames-full/*.png` when present), then view the complete movie silently. Check
that the named scene is readable, backgrounds/block/tick actually change and
the selected output is correct, not black or an unrelated desktop. Full decode
and nonblack/changing content are separate gates. Record these post-shutdown
observations in a separate parent evidence note tied to the candidate hash and
exact bundle; preserve the scenario's original report rather than rewriting it
into a stronger verdict. Audio, shortcuts, picker UI acceptance, ink and
cross-display correctness remain unrun unless separately exercised.

For normal-decoder acceptance, also require no missing-decoder/fallback warning
in the app log or `summary.md`, and verify native-sized keyframes against the
movie (use `frames-full/` when normal autocrop creates cropped frames). Inspect
the contact sheet and semantic summary together; decoder success alone does not
prove useful crop content or text. Do not remove the fallback warning to pass.

## Report vocabulary and failure boundaries

`capture-smoke-report.json` records candidate SHA-256, desktop environment,
request IDs, stage, individual `PASS`/`FAIL`/`UNRUN` gates, exact guest saved path,
its output-relative retained path, and bundle metadata/media filenames.
`command-NNN.stdout`/`.stderr` preserve each argv, exit or transport timeout;
`app.log`, private `.local/share/apparent/Logs/`, and `scene.log` retain diagnostics.

- `PASS_RECIPE_GATES_ONLY`: requested receipt/state/structural checks passed;
  **not** native pixel/media/shortcut acceptance. Exit zero never upgrades an
  `UNRUN` gate.
- `failure.kind: environment`: explicit missing runtime/session/control or
  scene/launcher readiness prerequisite. Inspect logs before blaming product.
- `driver`: malformed protocol/path or subprocess transport failure. Original
  streams remain available; this does not assert a product regression.
- `product`: explicit rejected/failed or contradictory production receipt/state,
  or missing required saved-bundle files. It identifies an observed failure,
  not a proven root cause.
- `unresolved`: bounded Start timeout; consent versus backend failure is not
  inferred. The request stays owned and is inspected/cancelled, not toggled.
- `cleanup_errors`: separate from the first failure. Cleanup inspects only owned
  IDs, cancels only starting/recording owned receipts, terminates only launched
  process groups, waits and checks group disappearance. It preserves media.
  After writers stop it removes only the two known private capture/launch
  socket entries, which otherwise make recursive SCP retention fail; the
  shared desktop runtime is not swept.
  This proves neither cancel-time disk deletion nor guest/portal resource
  teardown; `cancel_disk_cleanup` and `runner_cleanup` remain `UNRUN` here.

The focused Python tests run real harmless subprocess peers through the recipe,
including saved/cancelled receipts, retained nonzero stderr/stdout and a pending
start timeout. Their media are deliberately invalid labelled strings and their
scene producer is replaced. They test driver wiring, **not** native execution,
X11 visibility, portal consent, encoder output or candidate product behavior.
