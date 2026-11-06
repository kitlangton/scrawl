"""Bounded, shell-free CLI execution. No installation or app launch on import."""
from __future__ import annotations

import os
from pathlib import Path
import selectors
import signal
import subprocess
import time


class AdapterError(Exception):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


def execute(executable: str, args: list[str], *, cwd: Path | None = None,
            timeout: float = 150, limit: int = 2 * 1024 * 1024) -> str:
    try:
        child = subprocess.Popen([executable, *args], stdin=subprocess.DEVNULL,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                 cwd=cwd, start_new_session=True)
    except OSError as exc:
        raise AdapterError("cli_unavailable", "Apparent CLI could not start; check plugin setup.") from exc
    chunks = {child.stdout: bytearray(), child.stderr: bytearray()}
    deadline = time.monotonic() + timeout
    try:
        with selectors.DefaultSelector() as selector:
            for pipe in chunks:
                selector.register(pipe, selectors.EVENT_READ)
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise AdapterError("timeout", "Apparent timed out; retain the source and inspect the operation before retrying.")
                for key, _ in selector.select(min(remaining, 0.2)):
                    data = os.read(key.fileobj.fileno(), 65536)
                    if not data:
                        selector.unregister(key.fileobj)
                    else:
                        chunks[key.fileobj].extend(data)
                        if sum(map(len, chunks.values())) > limit:
                            raise AdapterError("output_limit", "Apparent output exceeded the adapter limit.")
            child.wait(timeout=max(0.01, deadline - time.monotonic()))
        output = bytes(chunks[child.stdout]).decode("utf-8", errors="strict")
        if child.returncode:
            detail = bytes(chunks[child.stderr]).decode("utf-8", errors="replace")[-3000:]
            raise AdapterError("cli_failed", detail or "Apparent command failed.")
        return output
    finally:
        # Also retire descendants that inherited a pipe or survived their CLI parent.
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        child.wait()
        for pipe in chunks:
            pipe.close()
