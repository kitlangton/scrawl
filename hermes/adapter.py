"""Consumer of the installed full Apparent CLI; never imports product internals."""
from __future__ import annotations

import json
from pathlib import Path
import re
import shutil
import sys
import tempfile

from .evidence import result
from .process import AdapterError, execute


def executable(configured: str = "") -> str:
    if sys.platform not in ("linux", "darwin"):
        raise AdapterError("unsupported_platform", "Apparent supports macOS and Linux.")
    if not isinstance(configured, str):
        raise AdapterError("invalid_config", "executable must be an absolute CLI path.")
    if configured:
        if not Path(configured).is_absolute():
            raise AdapterError("invalid_config", "executable must be an absolute CLI path.")
        return configured
    found = shutil.which("apparent")
    candidates = [Path.home() / ".local/bin/apparent",
                  Path("/Applications/Apparent.app/Contents/MacOS/apparent"),
                  Path.home() / "Applications/Apparent.app/Contents/MacOS/apparent",
                  Path("/Applications/Apparent Preview.app/Contents/MacOS/apparent")]
    if found:
        return found
    for candidate in candidates:
        if candidate.is_file():
            return str(candidate)
    raise AdapterError("setup_required", "Apparent CLI was not found. Locate the installed app or follow https://github.com/tryapparent/apparent#agents to install it on the correct host; set the plugin executable path if needed. Reading recordings on a remote backend requires only the receiving CLI, not a desktop recorder or capture permissions.")


class Adapter:
    def __init__(self, command: str):
        self.command = command

    def inspect(self, source: str, frame: int | None = None):
        if not isinstance(source, str) or not source or len(source) > 4096:
            raise AdapterError("invalid_source", "Pass an absolute bundle path or the exact 32-hex Connect ID from the user's handoff.")
        if frame is not None and (type(frame) is not int or frame < 1):
            raise AdapterError("invalid_frame", "Choose a positive keyframe number from the inspection summary.")
        if re.fullmatch(r"[a-f0-9]{32}", source):
            # The current full CLI separates fetch from inspect. Keep the complete
            # snapshot only while assembling this response, including on failure.
            with tempfile.TemporaryDirectory(prefix="apparent-hermes-") as workspace:
                root = Path(workspace) / (source + ".apparent")
                execute(self.command, ["fetch", source, "--out", str(root), "--wait", "120"])
                return self._inspect(root, source, frame, temporary=True)
        path = Path(source).expanduser()
        if not path.is_absolute() or path.name.startswith("-"):
            raise AdapterError("invalid_source", "Local recordings require an explicit absolute path; latest and arbitrary URLs are not accepted.")
        if path.name == "apparent.json":
            path = path.parent
        return self._inspect(path.resolve(), source, frame)

    def _inspect(self, root: Path, source: str, frame: int | None, temporary=False):
        summary = execute(self.command, ["inspect", str(root)])
        text = f"Source: {source}\n{summary}"
        if temporary:
            text += "\nConnect snapshot paths above are temporary and removed after this call. For another image, call apparent_inspect with the same Connect ID and a frame number. For temporal zoom or a text-only vision route, fetch that ID to your own private directory with the CLI, then inspect/zoom there; clean up when finished."
        try:
            manifest = json.loads(execute(self.command, ["inspect", str(root), "--json"]))
            reference = manifest.get("contactSheet")
            if frame is not None:
                if frame > len(manifest.get("keyframes", [])):
                    raise AdapterError("invalid_frame", "Choose a keyframe number from the inspection summary.")
                reference = manifest["keyframes"][frame - 1]["file"]
            if reference is not None and not isinstance(reference, str):
                raise AdapterError("invalid_manifest", "Malformed image reference.")
        except (ValueError, AttributeError, KeyError, TypeError, OSError, AdapterError) as exc:
            if isinstance(exc, AdapterError) and exc.code == "invalid_frame":
                raise
            return text + "\nVisual evidence unavailable from the CLI manifest: " + str(exc) + "\nAct on clear transcript instructions; explain unresolved visual uncertainty."
        return result(text, root, reference, temporary=temporary)
