"""Bounded, shell-free CLI execution. No installation or app launch on import."""
from __future__ import annotations

import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import time


# Keep the session/group leader alive after the CLI exits. Its unreaped PID is
# the ownership anchor until the caller signals the group, then waits for it.
_SUPERVISOR = """
import os, signal, subprocess, sys
status = int(sys.argv[1])
try:
    cli = subprocess.Popen(sys.argv[2:], stdin=subprocess.DEVNULL)
except OSError:
    os.close(1)
    os.close(2)
    os.write(status, b'unavailable')
else:
    os.close(1)
    os.close(2)
    os.write(status, str(cli.wait()).encode('ascii'))
os.close(status)
while True:
    signal.pause()
"""


class AdapterError(Exception):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


def execute(executable: str, args: list[str], *, cwd: Path | None = None,
            timeout: float = 150, limit: int = 2 * 1024 * 1024) -> str:
    status_read, status_write = os.pipe()
    try:
        child = subprocess.Popen([sys.executable, "-c", _SUPERVISOR, str(status_write), executable, *args], stdin=subprocess.DEVNULL,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                 cwd=cwd, start_new_session=True, pass_fds=(status_write,))
    except BaseException as exc:
        os.close(status_read)
        if isinstance(exc, OSError):
            raise AdapterError("cli_unavailable", "Apparent CLI could not start; check plugin setup.") from exc
        raise
    finally:
        os.close(status_write)
    chunks = {child.stdout: bytearray(), child.stderr: bytearray()}
    status = bytearray()
    deadline = time.monotonic() + timeout
    try:
        with selectors.DefaultSelector() as selector:
            for pipe in chunks:
                selector.register(pipe, selectors.EVENT_READ)
            selector.register(status_read, selectors.EVENT_READ)
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise AdapterError("timeout", "Apparent timed out; retain the source and inspect the operation before retrying.")
                for key, _ in selector.select(min(remaining, 0.2)):
                    data = os.read(key.fd, 65536)
                    if not data:
                        selector.unregister(key.fileobj)
                    else:
                        if key.fileobj == status_read:
                            status.extend(data)
                            if len(status) > 32:
                                raise AdapterError("cli_failed", "Invalid CLI supervisor result.")
                            continue
                        chunks[key.fileobj].extend(data)
                        if sum(map(len, chunks.values())) > limit:
                            raise AdapterError("output_limit", "Apparent output exceeded the adapter limit.")
        if status == b"unavailable":
            raise AdapterError("cli_unavailable", "Apparent CLI could not start; check plugin setup.")
        try:
            returncode = int(status)
        except ValueError as exc:
            raise AdapterError("cli_failed", "CLI supervisor ended without a result.") from exc
        output = bytes(chunks[child.stdout]).decode("utf-8", errors="strict")
        if returncode:
            detail = bytes(chunks[child.stderr]).decode("utf-8", errors="replace")[-3000:]
            raise AdapterError("cli_failed", detail or "Apparent command failed.")
        return output
    except subprocess.TimeoutExpired as exc:
        raise AdapterError("timeout", "Apparent timed out; retain the source and inspect the operation before retrying.") from exc
    finally:
        # Also retire descendants that inherited a pipe or survived their CLI parent.
        try:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()
        finally:
            os.close(status_read)
            for pipe in chunks:
                pipe.close()
