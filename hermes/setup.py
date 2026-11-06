"""Read-only setup checkpoints. Neither discovery nor checks install or launch apps."""
from __future__ import annotations

import json

from .adapter import executable
from .process import AdapterError, execute


def status(configured: str = "") -> str:
    report = {
        "setup_skill": "apparent:setup",
        "host": "Hermes backend; not necessarily the computer running Desktop",
        "checks": {},
        "readiness": "unverified",
    }

    def finish(state, next_action, error=None):
        report.update(state=state, next_action=next_action)
        if error is not None:
            report["error"] = {"code": getattr(error, "code", "invalid_response"), "message": str(error)}
        return json.dumps(report)

    try:
        command = executable(configured)
    except AdapterError as exc:
        return finish("unsupported_platform" if exc.code == "unsupported_platform" else "needs_cli",
                      "Load apparent:setup. Locate the existing recorder before installing anything.", exc)
    report["cli"] = command
    try:
        version = execute(command, ["--version"], timeout=10).strip()
        if not version.startswith("apparent "):
            raise AdapterError("unexpected_cli", "The configured executable did not identify itself as Apparent.")
        report["checks"]["version"] = version
    except (AdapterError, UnicodeError) as exc:
        return finish("needs_cli_repair", "Load apparent:setup and repair the selected CLI; preserve other installations.", exc)

    for name in ("inspect", "zoom", "app", "capture"):
        try:
            args = [name, "state", "--help"] if name == "capture" else [name, "--help"]
            execute(command, args, timeout=10)
            report["checks"][name] = "available"
        except (AdapterError, UnicodeError) as exc:
            report["checks"][name] = "unavailable"
            return finish("needs_cli_update", "Load apparent:setup. This CLI lacks a required capability; inspect the failure before updating.", exc)

    try:
        recorder = json.loads(execute(command, ["capture", "state"], timeout=10))
        if not isinstance(recorder, dict) or recorder.get("error"):
            raise AdapterError("recorder_unavailable", "The recorder did not return a usable capture state.")
        state = recorder.get("state")
        phase = state.get("phase") if isinstance(state, dict) else None
        if phase not in {"idle", "selecting", "starting", "recording", "processing", "completed"}:
            raise AdapterError("unknown_recorder_state", "The recorder returned an unknown phase; do not infer readiness.")
    except (AdapterError, UnicodeError, ValueError) as exc:
        return finish("needs_recorder_check", "Load apparent:setup. Confirm the recorder host, locate/open the installed app and finish existing onboarding. An unavailable control connection does not prove the app is missing.", exc)
    # Do not expose other captures' identities or imply authority to stop them.
    report["checks"]["recorder"] = {"phase": phase}
    if phase not in {"idle", "completed"}:
        return finish("recorder_busy", "Preserve the active workflow. Do not stop, restart, replace or start another recorder during setup.")
    return finish("needs_recording_check", "The CLI and recorder respond. Resume existing onboarding if needed, then have the user make a harmless recording and paste its handoff here. Inspect its transcript and pixels before calling setup complete. Permissions, shortcuts, audio and Hermes model vision remain unverified.")
