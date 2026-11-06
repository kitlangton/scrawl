"""Consumer of the installed full Apparent CLI; never imports product internals."""
from __future__ import annotations

import json
import math
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
    if configured:
        if not isinstance(configured, str) or not Path(configured).is_absolute():
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
    raise AdapterError("setup_required", "Apparent CLI was not found. Load apparent:setup and run apparent_status to locate the recorder or guide installation; an absent CLI does not prove the recorder is missing.")


class Adapter:
    def __init__(self, command: str):
        self.command = command

    def source(self, source: str) -> tuple[Path, Path | None]:
        if not isinstance(source, str) or not source or len(source) > 4096:
            raise AdapterError("invalid_source", "Pass an absolute bundle path or the exact 32-hex Connect ID from the user's handoff.")
        if re.fullmatch(r"[a-f0-9]{32}", source):
            # Each fetch uses fresh private output. Return its exact path for later tools.
            workspace = Path(tempfile.mkdtemp(prefix="apparent-hermes-"))
            return workspace / (source + ".evidence.apparent"), workspace
        path = Path(source).expanduser()
        if not path.is_absolute() or path.name.startswith("-"):
            raise AdapterError("invalid_source", "Local recordings require an explicit absolute path; latest and arbitrary URLs are not accepted.")
        if path.name == "apparent.json":
            path = path.parent
        return path.resolve(), None

    def inspect(self, source: str, frame: int | None = None):
        root, workspace = self.source(source)
        summary = execute(self.command, ["inspect", source if workspace else str(root)], cwd=workspace)
        try:
            manifest = json.loads(execute(self.command, ["inspect", str(root), "--json"]))
            reference = manifest.get("contactSheet")
            if frame is not None:
                if type(frame) is not int or frame < 1 or frame > len(manifest.get("keyframes", [])):
                    raise AdapterError("invalid_frame", "Choose a keyframe number from the inspection summary.")
                reference = manifest["keyframes"][frame - 1]["file"]
            if reference is not None and not isinstance(reference, str):
                raise AdapterError("invalid_manifest", "Malformed image reference.")
        except (json.JSONDecodeError, AttributeError, KeyError, TypeError) as exc:
            return summary + "\nCannot resolve visual evidence from the CLI manifest: " + str(exc)
        return result(f"Bundle: {root}\n{summary}", root, reference)

    def zoom(self, source: str, *, at=None, mark=None, frame=None, span=2, frames=12, full=False):
        root, workspace = self.source(source)
        if workspace:
            raise AdapterError("inspect_first", "Inspect the Connect ID first, then zoom using the returned local bundle path.")
        if (at is None) == (mark is None):
            raise AdapterError("invalid_focus", "Choose exactly one of at (time/phrase) or mark.")
        focus = at if at is not None else mark
        if not isinstance(focus, str) or not focus.strip() or focus.startswith("-") or len(focus) > 512:
            raise AdapterError("invalid_focus", "Pass a nonempty time, phrase or mark ID.")
        if type(span) not in (int, float) or not math.isfinite(span) or not 0.1 <= span <= 60:
            raise AdapterError("invalid_span", "span must be between 0.1 and 60 seconds.")
        if type(frames) is not int or not 1 <= frames <= 24 or type(full) is not bool:
            raise AdapterError("invalid_options", "frames must be 1–24 and full must be boolean.")
        args = ["zoom", str(root), "--at" if at is not None else "--mark", focus,
                "--span", str(span), "--frames", str(frames)]
        if frame is not None:
            if type(frame) is not int or frame < 1 or mark is None:
                raise AdapterError("invalid_frame", "frame is a positive keyframe number used with mark.")
            args += ["--frame", str(frame)]
        if full:
            args += ["--full"]
        output = execute(self.command, args)
        # The published full CLI prints the sheet path on its own line.
        for line in output.splitlines():
            candidate = Path(line)
            if candidate.is_absolute():
                try:
                    reference = str(candidate.relative_to(root))
                except ValueError:
                    continue
                if reference.startswith("zoom/") and candidate.suffix == ".png":
                    return result(output, root, reference)
        return output + "\nCLI returned no supported zoom image path; inspect the source frame instead."
