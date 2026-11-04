# Desktop verification

Reusable VM lifecycle, expiry, evidence retention and owned native-test process
cleanup live in
[enablement/native](../../../enablement/native/README.md). Desktop keeps the
behavior-specific scenarios and acceptance recipes below.

Shared Electron UI is verified primarily on Linux. Mac execution covers affected
OS boundaries and a short packaged integration smoke, rather than a second full
workflow matrix. This policy supersedes blanket two-platform walkthrough gates
in older workflow documentation; it does not certify unrun behavior or remove a
specific human-imposed hold, consent requirement, safety rule or spending limit.

## Coverage and merge gates

| Layer | Required verification |
|---|---|
| Shared React/Electron behavior | Linux tests for rendering, controls, consent/cancel/retry states, errors and accessibility semantics. Use the shipped renderer/main/preload. |
| Shared Rust↔Electron contract | Linux integration through the real Rust handlers, including failure and authorization behavior. Synthetic services are suitable; a fake Rust bridge proves only the fake. |
| Native OS boundaries | Focused execution on the affected OS for permissions/dialogs, shortcuts/input, actual clipboard/paste, ownership/focus/stacking, capture inclusion/exclusion, startup/fallback and packaging. First adoption of a route counts as a boundary change. |
| Mac release package | Short smoke of the current package: launch, a representative interaction and owned cleanup, plus applicable native evidence and existing exact-commit CI/signing checks. |

Before merge, run focused shared checks, independent review and current-base
integration assessment. Require native proof for affected boundaries only.
Changing a bridge contract, Electron/runtime version or package layout requires
assessing native consumers; moving logic into Electron does not make OS effects
platform independent. Native capture, audio, permissions and multi-display changes
retain their relevant real-platform checks.

A shared-renderer-only change does not require a new Mac walkthrough when the
owner establishes that native boundaries are unchanged. Reuse successful seam
evidence after checking the relevant source, dependencies, runtime, packaging and
environment inputs. Record the original evidence identity and why it applies;
do not relabel an older execution as the current binary. An unrelated main
advance or a new commit ID alone is not a reason to repeat native proof. Missing
or incompatible evidence for an affected boundary requires that focused check.
Known product failures must be investigated; a driver/environment failure is
neither a product pass nor a reason to replay all unaffected cases.

Use the existing `desktop-ui-integration` lane in `enablement/owner_lanes.py` for
shared Electron and real Rust bridge checks. Use existing installed Mac scenarios
and isolated Linux recipes for native boundaries. Keep each native checklist
small and tied to the changed effect; do not build a new runner or evidence-cache
service merely to apply this policy.

For Copy link, Linux owns the shared consent/cancel/timeout/retry/repeat/offline
matrix and synthetic recipient/revocation/privacy checks. Mac checks the installed
entry opening the popup, ownership/focus, startup/fallback, actual clipboard and
cleanup. Local Paste needs a real receiver when its OS handoff changes. Reuse
existing valid results; do not edit another owner's active candidate to split its
tests. Preserve actual output inspection when output changes are exercised.

## Transition and publication

This is the governing proof-selection policy now, including for in-flight work.
Owners should replace generic inherited Mac/full-workflow waits with the concrete
affected seam and evidence still needed. Explicit human holds and requested proof
remain; historical generic checklists alone do not recreate a blanket gate.
Before removing a fallback, establish coverage of shared behavior plus each
affected platform boundary and verify remaining consumers/build/package inputs.

This documentation change does not implement automatic test selection, receipt
reuse or alter executable CI/release admission. Existing configured checks still
run and must pass. Owners may assess evidence manually under this policy; that
assessment cannot bypass a required automated check. The current Mac package
smoke and applicable native evidence remain publication requirements even when a
shared-only PR needs no fresh Mac run before merge. Mac delays do not block
Linux/web publication under their own existing gates.

## Linux native capture smoke recipe

This is one supported **guest scenario**, not another VM controller:
`capture/desktop/scripts/linux/gui/capture-smoke.py`, run by `enablement/native/linux/gui-vm.py`. It starts the
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

The baseline runner has no `doctor` subcommand. These are the exact launch-free
preflight commands; do not treat an unknown `doctor` argument as a health pass:

```sh
python3 enablement/native/linux/test-gui-vm.py
python3 capture/desktop/scripts/linux/gui/test-capture-smoke.py
python3 enablement/native/linux/gui-vm.py --help
for tool in qemu-system-x86_64 qemu-img ssh scp systemd-run python3; do
  command -v "$tool" || exit 1
done
test -r /usr/share/OVMF/OVMF_CODE_4M.fd && test -r "$GUI_TEMPLATE/OVMF_VARS.fd"
test -r "$GUI_TEST_KEY" && test -x "$CANDIDATE"
systemctl --user show-environment >/dev/null
qemu-img info --output=json "$GUI_TEMPLATE/system.qcow2"
sha256sum "$CANDIDATE"
```

Require qcow2 with no backing file, stopped writers and sufficient KVM access;
`qemu-img info` alone does not certify sanitization, ownership or capacity.
Guest prerequisites: Python 3, libX11, working XWayland/DISPLAY, the candidate's
Vulkan/GStreamer runtime, a working user session bus, Hyprland and ScreenCast
portal/PipeWire. The scenario imports the desktop environment from
`systemctl --user show-environment`, checks `DISPLAY`/runtime, refuses an existing
`apparent.sock` or same-user Apparent/Scrawl process, and waits for real control
readiness. It does not install codecs or repair a template.

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

```sh
python3 enablement/native/linux/gui-vm.py \
  --template "$GUI_TEMPLATE/system.qcow2" \
  --ovmf-vars "$GUI_TEMPLATE/OVMF_VARS.fd" \
  --ssh-user "$GUI_TEST_USER" --ssh-key "$GUI_TEST_KEY" \
  --binary "$CANDIDATE" --scenario capture/desktop/scripts/linux/gui/capture-smoke.py \
  --results "$RUN" --memory 3072 --isolated-network \
  --boot-timeout 180 --scenario-timeout 300 --job-timeout 660
```

The Python file has a tested shell/Python entry header because the existing
runner copies just one scenario file and invokes it with Bash. No sibling
scenario dependency needs staging. The runner supplies `SCRAWL_GUI_BINARY`,
`SCRAWL_GUI_OUTPUT` and the candidate library path. For an already owned guest
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

For a scoped native input check, the existing recipe accepts `--save-seconds N`
(1..180, default 4) and `--silent` to disable microphone capture in its isolated
config. Keep the hold within the runner's scenario/job deadlines. The longer hold
alone is not input proof: retain the observed interaction, saved notes and bundle
inspection separately. The ordinary recipe defaults remain unchanged.

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
