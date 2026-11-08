#!/usr/bin/env python3
""":"
exec python3 "$0" "$@"
":"""
"""One guest-only production capture/save/cancel scenario; no VM controller."""

import argparse
import ctypes as C
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import uuid


class Failure(Exception):
    def __init__(self, kind, message):
        super().__init__(message)
        self.kind = kind


def state(value):
    s = value.get("state", {})
    if not isinstance(s, dict) or s.get("phase") not in {"idle", "selecting", "starting", "recording", "processing", "completed"}:
        raise Failure("driver", "invalid state response")
    if "capture_id" not in s or (s["capture_id"] is not None and not isinstance(s["capture_id"], str)):
        raise Failure("driver", "invalid state identity")
    return s


def receipt(value, owned, status):
    if value.get("capture_id") != owned or value.get("status") != status:
        raise Failure("product", f"expected owned {status} receipt, got {value}")
    if value.get("error") is not None:
        raise Failure("product", f"receipt error: {value}")
    path = value.get("bundle_path")
    if status == "saved":
        if not isinstance(path, str) or not Path(path).is_absolute():
            raise Failure("driver", "saved receipt has no absolute bundle path")
    elif path is not None:
        raise Failure("driver", "non-saved receipt unexpectedly contains a bundle path")
    return path


def scene(ready):
    """Changing ordinary X11 window, using libX11 already required by GPUI."""
    x = C.CDLL("libX11.so.6")
    pointer, ulong, integer = C.c_void_p, C.c_ulong, C.c_int
    signatures = {
        "XOpenDisplay": (pointer, [C.c_char_p]),
        "XDefaultRootWindow": (ulong, [pointer]),
        "XCreateSimpleWindow": (ulong, [pointer, ulong, integer, integer, C.c_uint, C.c_uint, C.c_uint, ulong, ulong]),
        "XStoreName": (integer, [pointer, ulong, C.c_char_p]),
        "XMapWindow": (integer, [pointer, ulong]),
        "XCreateGC": (pointer, [pointer, ulong, ulong, pointer]),
        "XSetForeground": (integer, [pointer, pointer, ulong]),
        "XFillRectangle": (integer, [pointer, ulong, pointer, integer, integer, C.c_uint, C.c_uint]),
        "XDrawString": (integer, [pointer, ulong, pointer, integer, integer, C.c_char_p, integer]),
        "XSync": (integer, [pointer, integer]),
    }
    for name, (result, args) in signatures.items():
        getattr(x, name).restype, getattr(x, name).argtypes = result, args
    display = x.XOpenDisplay(None)
    if not display:
        raise Failure("environment", "cannot open X11 DISPLAY")
    window = x.XCreateSimpleWindow(display, x.XDefaultRootWindow(display), 60, 100, 800, 500, 0, 0, 0x17335B)
    x.XStoreName(display, window, b"Apparent capture smoke - SYNTHETIC")
    gc = x.XCreateGC(display, window, 0, None)
    x.XMapWindow(display, window)
    for tick in range(3600):
        x.XSetForeground(display, gc, 0x17335B if tick % 2 else 0x8B2750)
        x.XFillRectangle(display, window, gc, 0, 0, 800, 500)
        x.XSetForeground(display, gc, 0xFFFFFF)
        text = f"SYNTHETIC capture smoke | tick {tick:04d} | no personal content".encode()
        x.XDrawString(display, window, gc, 40, 120, text, len(text))
        x.XFillRectangle(display, window, gc, 40 + (tick % 12) * 50, 220, 90, 110)
        x.XSync(display, 0)
        if tick == 0:
            Path(ready).write_text(json.dumps({"xid": window, "title": "Apparent capture smoke - SYNTHETIC", "requested_size": [800, 500]}))
        time.sleep(.5)


class Recipe:
    def __init__(self, output, binary, seconds):
        self.out, self.binary, self.seconds = output, binary, seconds
        self.env = os.environ.copy()
        self.jobs, self.ids, self.serial = [], [], 0
        self.report = {"schema": 1, "proof": "native execution required; synthetic transport tests are not native proof",
                       "gates": {key: "UNRUN" for key in (
                           "environment", "scene", "gui_ready", "save_recording", "saved_receipt", "bundle_structure",
                           "cancel_recording", "cancelled_receipt", "cancel_idle", "owned_process_cleanup",
                           "selected_target_pixels", "movie_full_decode", "movie_nonblack_changing",
                           "visual_bundle_review", "shortcut_picker_acceptance", "audio", "cancel_disk_cleanup", "runner_cleanup")},
                       "commands": [], "cleanup_errors": [], "requests": []}

    def save(self):
        (self.out / "capture-smoke-report.json").write_text(json.dumps(self.report, indent=2) + "\n")

    def command(self, *args, timeout=8, allow_error=False):
        self.serial += 1
        stem = f"command-{self.serial:03d}"
        command = [str(self.binary), "capture", *args]
        record = {"argv": command, "stdout": stem + ".stdout", "stderr": stem + ".stderr"}
        self.report["commands"].append(record)
        self.save()
        with (self.out / record["stdout"]).open("w") as stdout, (self.out / record["stderr"]).open("w") as stderr:
            try:
                p = subprocess.run(command, env=self.env, stdout=stdout, stderr=stderr, timeout=timeout)
                record["exit"] = p.returncode
            except subprocess.TimeoutExpired as error:
                record["timeout"] = timeout
                raise Failure("driver", f"command exceeded {timeout}s: {command}") from error
        self.save()
        try:
            value = json.loads((self.out / record["stdout"]).read_text())
            if not isinstance(value, dict):
                raise ValueError("not an object")
        except (ValueError, OSError) as error:
            raise Failure("driver", f"invalid CLI JSON; see {stem} streams") from error
        if p.returncode and not isinstance(value.get("error"), str):
            raise Failure("driver", f"nonzero CLI exit without symbolic error; see {stem} streams")
        if p.returncode and not allow_error:
            kind = "environment" if value.get("error") == "control_unavailable" else "product"
            raise Failure(kind, f"CLI failed: {value}; see {stem} streams")
        return value

    def launch(self, argv, log):
        with (self.out / log).open("w") as stream:
            p = subprocess.Popen(argv, env=self.env, stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
        self.jobs.append(p)
        return p

    def wait(self, predicate, seconds, message):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(.25)
        raise Failure("environment", message)

    def prepare(self):
        self.report["stage"] = "environment"
        # Keep the real desktop bus/runtime (portals and Hyprland); isolate all app data.
        desktop = subprocess.run(["systemctl", "--user", "show-environment"], capture_output=True, text=True, timeout=8)
        if desktop.returncode:
            raise Failure("environment", "cannot read user desktop environment")
        for line in desktop.stdout.splitlines():
            key, sep, value = line.partition("=")
            if sep and key in {"DISPLAY", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS", "HYPRLAND_INSTANCE_SIGNATURE", "XAUTHORITY", "XDG_CURRENT_DESKTOP", "XDG_SESSION_TYPE"}:
                self.env[key] = value
        if not self.env.get("DISPLAY") or not self.env.get("XDG_RUNTIME_DIR"):
            raise Failure("environment", "desktop has no DISPLAY or XDG_RUNTIME_DIR")
        if (Path(self.env["XDG_RUNTIME_DIR"]) / "apparent.sock").exists():
            raise Failure("environment", "existing apparent.sock: refuse to replace or forward to another GUI")
        for entry in Path("/proc").iterdir():
            try:
                if entry.name.isdigit() and entry.stat().st_uid == os.getuid() and (entry / "comm").read_text().strip() in {"apparent", "scrawl"}:
                    raise Failure("environment", "existing product process: use a clean owned guest")
            except (FileNotFoundError, PermissionError, ProcessLookupError):
                pass
        authority = Path(self.env.get("HOME", "")) / ".Xauthority"
        if "XAUTHORITY" not in self.env and authority.is_file():
            self.env["XAUTHORITY"] = str(authority)
        home = self.out
        control_socket = home / ".config/apparent/agent-control/capture.sock"
        if len(os.fsencode(control_socket)) > 107:
            raise Failure("environment", "guest output/HOME is too long for capture.sock; runner must stage a shorter guest output path")
        for key, path in {"HOME": home, "XDG_CONFIG_HOME": home / ".config", "XDG_DATA_HOME": home / ".local/share",
                          "XDG_CACHE_HOME": home / ".cache", "APPARENT_CONFIG": home / ".config/apparent/config.json",
                          "APPARENT_CAPTURES_DIR": self.out / "captures", "TMPDIR": self.out / "tmp"}.items():
            self.env[key] = str(path)
        for key in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "APPARENT_CAPTURES_DIR", "TMPDIR"):
            Path(self.env[key]).mkdir(parents=True, mode=0o700)
        config = Path(self.env["APPARENT_CONFIG"])
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text(json.dumps({"onboarding": {"seen": True}, "launchAtLogin": False, "autoPasteDraft": False}))
        self.env["APPIMAGE_EXTRACT_AND_RUN"] = "1"
        self.report["candidate"] = {"path": str(self.binary), "sha256": hashlib.sha256(self.binary.read_bytes()).hexdigest()}
        self.report["desktop"] = {key: self.env.get(key) for key in ("DISPLAY", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "XDG_SESSION_TYPE", "XDG_CURRENT_DESKTOP")}
        self.report["gates"]["environment"] = "PASS"

    def start(self, label):
        self.report["stage"] = label + "_recording"
        owned = uuid.uuid4().hex
        self.ids.append(owned)
        self.report["requests"].append({"label": label, "capture_id": owned})
        self.save()  # Persist identity BEFORE a command that can outlive its caller.
        print(f"{label}: portal consent may be needed. On this owned guest, select the output containing 'Apparent capture smoke - SYNTHETIC' and approve sharing within {self.seconds}s. No automatic input is sent.", flush=True)
        value = self.command("start", "--request-id", owned, "--wait", "--timeout", str(self.seconds), timeout=self.seconds + 6, allow_error=True)
        if value.get("error") == "timeout":
            current = self.command("inspect", owned, allow_error=True)
            self.report["pending_receipt"] = current
            raise Failure("unresolved", f"bounded start wait expired for {owned}; consent versus backend failure is unresolved. Inspect the owned VNC desktop and app.log; approve ScreenCast within the budget on a NEW run. This run's owned request will be cancelled.")
        if value.get("error") == "control_unavailable":
            raise Failure("environment", f"GUI control unavailable: {value}")
        receipt(value, owned, "recording")
        current = state(self.command("state"))
        if current != {"phase": "recording", "capture_id": owned}:
            raise Failure("product", f"recording receipt/state mismatch: {current}")
        self.report["gates"][label + "_recording"] = "PASS"
        self.save()
        return owned

    def run(self):
        self.prepare()
        self.report["stage"] = "scene"
        ready = self.out / "scene.json"
        scene_job = self.launch([sys.executable, str(Path(__file__).resolve()), "--scene", str(ready)], "scene.log")
        self.wait(lambda: ready.exists() or scene_job.poll() is not None, 8, "X11 scene did not start")
        if not ready.exists() or scene_job.poll() is not None:
            raise Failure("environment", "X11 scene failed; see scene.log (libX11 and working DISPLAY required)")
        self.report["scene"] = json.loads(ready.read_text())
        self.report["gates"]["scene"] = "PASS"  # XSync completed, NOT observed screen pixels.
        self.report["stage"] = "gui_ready"
        app = self.launch([str(self.binary), "app"], "app.log")
        def gui_ready():
            if app.poll() is not None:
                raise Failure("environment", "candidate launcher exited before readiness; see app.log")
            value = self.command("state", allow_error=True)
            if value.get("error"):
                return False
            current = state(value)
            if current != {"phase": "idle", "capture_id": None}:
                raise Failure("product", f"new GUI is not idle: {current}")
            return True
        self.wait(gui_ready, 20, "GUI control not ready; see command streams and app.log")
        self.report["gates"]["gui_ready"] = "PASS"
        owned = self.start("save")
        time.sleep(4)
        if scene_job.poll() is not None:
            raise Failure("environment", "synthetic scene exited during recording")
        self.report["stage"] = "saved_receipt"
        value = self.command("stop", owned, "--wait", "--timeout", "60", timeout=66)
        saved = receipt(value, owned, "saved")
        if receipt(self.command("inspect", owned), owned, "saved") != saved:
            raise Failure("product", "inspect changed the saved path")
        assert isinstance(saved, str)
        bundle = Path(saved).resolve()
        captures = Path(self.env["APPARENT_CAPTURES_DIR"]).resolve()
        if not bundle.is_relative_to(captures) or bundle == captures:
            raise Failure("driver", "saved receipt escaped the private captures directory")
        self.report["saved_bundle_guest"] = saved
        self.report["saved_bundle_relative_to_output"] = str(bundle.relative_to(self.out))
        self.report["gates"]["saved_receipt"] = "PASS"
        self.report["stage"] = "bundle_structure"
        self.save()
        files = sorted(str(p.relative_to(bundle)) for p in bundle.rglob("*") if p.is_file())
        self.report["bundle_files"] = files
        for name in ("apparent.json", "summary.md", "contact-sheet.png"):
            if not (bundle / name).is_file() or not (bundle / name).stat().st_size:
                raise Failure("product", f"saved bundle missing nonempty {name}")
        json.loads((bundle / "apparent.json").read_text())
        if not any(name.startswith("frames/") and name.endswith(".png") for name in files) or not any(name in files for name in ("recording.mp4", "recording.mov")):
            raise Failure("product", "saved bundle missing frames or movie")
        self.report["gates"]["bundle_structure"] = "PASS"
        owned = self.start("cancel")
        time.sleep(2)
        self.report["stage"] = "cancelled_receipt"
        receipt(self.command("cancel", owned), owned, "cancelled")
        receipt(self.command("inspect", owned), owned, "cancelled")
        self.report["gates"]["cancelled_receipt"] = "PASS"
        self.report["stage"] = "cancel_idle"
        current = state(self.command("state"))
        if current["capture_id"] is not None or current["phase"] not in {"idle", "completed"}:
            raise Failure("product", f"cancel did not return to idle/completed: {current}")
        self.report["gates"]["cancel_idle"] = "PASS"
        self.report["stage"] = "finished"

    def cleanup(self):
        # Same IDs only. Never use current global state to choose a cancellation target.
        for owned in self.ids:
            try:
                current = self.command("inspect", owned, allow_error=True)
                if current.get("capture_id") == owned and current.get("status") in {"starting", "recording"}:
                    receipt(self.command("cancel", owned), owned, "cancelled")
            except Exception as error:
                self.report["cleanup_errors"].append(str(error))
        for job in reversed(self.jobs):
            try:
                # poll()/wait() elsewhere may already have reaped this child.
                # A reaped PID is not an ownership token, even for killpg(pid, 0).
                if job.returncode is not None:
                    raise ChildProcessError("ownership anchor already reaped; runner must retain/stop guest")
                os.waitid(os.P_PID, job.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                # This recipe is single-threaded: no poll/wait or SIGCHLD reaper
                # can release the child until the final wait below. Its unreaped
                # PID pins the numeric group ID, including when the leader exited.
                def stopped():
                    if os.waitid(os.P_PID, job.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is None:
                        return False
                    # killpg(0) includes our zombie anchor. Instead require every
                    # remaining member to be dead before releasing that anchor.
                    for entry in Path('/proc').iterdir():
                        if not entry.name.isdigit():
                            continue
                        try:
                            fields = (entry / 'stat').read_text().rsplit(')', 1)[1].split()
                        except (FileNotFoundError, ProcessLookupError):
                            continue
                        if int(fields[2]) == job.pid and fields[0] not in {'Z', 'X'}:
                            return False
                    return True

                try:
                    os.killpg(job.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                # Never let a non-atomic /proc scan skip the final kernel group
                # kill. Descendants can fork during enumeration after TERM.
                # Keep the child anchor pinned through KILL and verification.
                try:
                    os.killpg(job.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                self.wait(stopped, 3, "owned group still active; runner must retain/stop guest")
                job.wait(timeout=3)  # No numeric group operations after this reap.
            except (OSError, ValueError, IndexError, Failure, subprocess.SubprocessError) as error:
                self.report["cleanup_errors"].append(f"process group {job.pid}: {error}")
        # SIGTERM leaves Unix socket directory entries behind. SCP cannot retain
        # sockets: remove only the two known private endpoints after writers stop.
        if not self.report["cleanup_errors"]:
            self.report["removed_private_sockets"] = []
            for relative in (".config/apparent/agent-control/capture.sock", ".config/apparent/launch/open.sock"):
                path = self.out / relative
                try:
                    if path.is_socket():
                        path.unlink()
                        self.report["removed_private_sockets"].append(relative)
                except OSError as error:
                    self.report["cleanup_errors"].append(f"private socket {relative}: {error}")
        self.report["gates"]["owned_process_cleanup"] = "FAIL" if self.report["cleanup_errors"] else "PASS"
        self.save()


def main():
    parser = argparse.ArgumentParser(description="Guest-only production capture/save/cancel scenario; invoked by gui-vm.py.")
    parser.add_argument("--scene", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--consent-timeout", type=int, default=40)
    args = parser.parse_args()
    if args.scene:
        scene(args.scene)
        return 0
    if not 1 <= args.consent_timeout <= 120:
        parser.error("consent-timeout must be 1..120 seconds")
    os.umask(0o077)
    output = Path(os.environ["SCRAWL_GUI_OUTPUT"]).resolve()
    output.mkdir(parents=True, exist_ok=True)
    if (output / "capture-smoke-report.json").exists() or (output / ".config").exists():
        parser.error("output already used; preserve it and choose a new run")
    recipe = Recipe(output, Path(os.environ["SCRAWL_GUI_BINARY"]).resolve(), args.consent_timeout)
    signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(Failure("driver", "scenario terminated")))
    try:
        recipe.run()
        recipe.report["result"] = "PASS_RECIPE_GATES_ONLY"
    except (Failure, OSError, ValueError, subprocess.SubprocessError) as error:
        recipe.report["result"] = "FAIL"
        stage = recipe.report.get("stage")
        if stage in recipe.report["gates"]:
            recipe.report["gates"][stage] = "FAIL"
        recipe.report["failure"] = {"kind": getattr(error, "kind", "driver"), "stage": stage, "message": str(error)}
        print(str(error), file=sys.stderr)
    finally:
        recipe.cleanup()
    return 0 if recipe.report.get("result") == "PASS_RECIPE_GATES_ONLY" and not recipe.report["cleanup_errors"] else 1


if __name__ == "__main__":
    sys.exit(main())
