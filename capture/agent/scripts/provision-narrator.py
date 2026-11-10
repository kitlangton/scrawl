#!/usr/bin/env python3
"""Explicit standalone narrator provisioning (never invoked during synthesis)."""
from pathlib import Path
import argparse
import contextlib
import hashlib
import json
import os
import secrets
import signal
import stat
import subprocess
import tempfile
import time
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, build_opener

DEFAULT_CACHE = Path.home() / ".local/share/apparent/narrator"
ROOT = Path(__file__).resolve().parents[3]
REVISION = "1939ad2a8e416c0acfeecc08a694d14ef25f2231"
MODEL_URL = "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.1/kokoro-v1.0.int8.onnx"
MODEL_BYTES = 114119327
MODEL_SHA256 = "ae315a79b623f244700e4afb9246c46a26066782e049ba174bf3ba433970ee9c"
VOICE_URL = f"https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/{REVISION}/voices/bm_fable.bin"
VOICE_BYTES = 522240
VOICE_SHA256 = "f889083196807b4adb15e9204252165f503b8d33d3982e681c52443c49d798f1"


class HTTPSRedirects(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if urlsplit(newurl).scheme != "https":
            raise ValueError("refusing non-HTTPS redirect")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


# Private opener: do not change urllib's process-global opener.
urlopen = build_opener(HTTPSRedirects()).open


def child_directory(parent_fd, name):
    try:
        os.mkdir(name, mode=0o700, dir_fd=parent_fd)
    except FileExistsError:
        pass
    return os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent_fd)


def open_cache(path, create=True):
    path = Path(os.path.abspath(path))
    if path == ROOT or ROOT in path.parents:
        raise ValueError("cache must be outside the source repository")
    fd = os.open(path.anchor, os.O_RDONLY | os.O_DIRECTORY)
    try:
        for component in path.parts[1:]:
            try:
                os.stat(".git", dir_fd=fd, follow_symlinks=False)
            except FileNotFoundError:
                pass
            else:
                raise ValueError("cache must be outside source repositories")
            child = (child_directory(fd, component) if create else
                     os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd))
            os.close(fd)
            fd = child
        try:
            os.stat(".git", dir_fd=fd, follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            raise ValueError("cache must be outside source repositories")
        info = os.fstat(fd)
        if info.st_uid != os.getuid() or info.st_mode & 0o022:
            raise ValueError("cache must be owned by this user and not writable by others")
        return fd
    except BaseException:
        os.close(fd)
        raise


def verify(fd, size, digest):
    info = os.fstat(fd)
    if not stat.S_ISREG(info.st_mode) or info.st_size != size:
        raise ValueError("asset integrity: wrong type or size")
    hasher = hashlib.sha256()
    with os.fdopen(os.dup(fd), "rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(block)
    if hasher.hexdigest() != digest:
        raise ValueError("asset integrity: SHA256 mismatch; remove damaged cache file explicitly")


def partial(directory_fd):
    name = f".narrator-{secrets.token_hex(16)}.partial"
    fd = os.open(name, os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=directory_fd)
    return name, fd


@contextlib.contextmanager
def download_deadline():
    # Socket timeouts alone do not bound a server that trickles bytes forever.
    def expired(_signum, _frame):
        raise TimeoutError("download exceeded 15 minute deadline")
    previous = signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, 900)
    try:
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)


def ensure_asset(directory_fd, name, url, size, digest):
    if urlsplit(url).scheme != "https":
        raise ValueError("downloads require HTTPS")
    try:
        fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory_fd)
    except FileNotFoundError:
        pass
    else:
        try:
            verify(fd, size, digest)
        finally:
            os.close(fd)
        return
    temporary, fd = partial(directory_fd)
    try:
        deadline = time.monotonic() + 900
        count = 0
        with download_deadline(), os.fdopen(os.dup(fd), "wb") as out, urlopen(url, timeout=30) as response:
            while True:
                if time.monotonic() >= deadline:
                    raise TimeoutError("download exceeded 15 minute deadline")
                block = response.read(min(1024 * 1024, size + 1 - count))
                if not block:
                    break
                count += len(block)
                if count > size:
                    raise ValueError("asset integrity: download exceeds pinned size")
                out.write(block)
            out.flush()
            os.fsync(out.fileno())
        os.lseek(fd, 0, os.SEEK_SET)
        verify(fd, size, digest)
        # No replacement: another provisioner may have won while we downloaded.
        try:
            os.link(temporary, name, src_dir_fd=directory_fd, dst_dir_fd=directory_fd, follow_symlinks=False)
        except FileExistsError:
            other = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory_fd)
            try:
                verify(other, size, digest)
            finally:
                os.close(other)
    finally:
        os.close(fd)
        os.unlink(temporary, dir_fd=directory_fd)


def install_worker(directory_fd, binary):
    temporary, fd = partial(directory_fd)
    try:
        with binary.open("rb") as source, os.fdopen(os.dup(fd), "wb") as dest:
            for block in iter(lambda: source.read(1024 * 1024), b""):
                dest.write(block)
            dest.flush()
            os.fchmod(dest.fileno(), 0o700)
            os.fsync(dest.fileno())
        # rename replaces the directory entry, never follows a planted symlink.
        os.replace(temporary, "apparent-narrator", src_dir_fd=directory_fd, dst_dir_fd=directory_fd)
        os.fsync(directory_fd)
    finally:
        os.close(fd)
        with contextlib.suppress(FileNotFoundError):
            os.unlink(temporary, dir_fd=directory_fd)


def inspect_cache(path):
    try:
        cache = open_cache(path, create=False)
    except FileNotFoundError:
        return {"assets": False, "worker": False}
    try:
        assets = True
        for relative, size, digest in [
            ("model.onnx", MODEL_BYTES, MODEL_SHA256),
            ("voices/bm_fable.bin", VOICE_BYTES, VOICE_SHA256),
        ]:
            parent = cache
            voices = None
            try:
                name = relative
                if "/" in relative:
                    voices = os.open("voices", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=cache)
                    info = os.fstat(voices)
                    if info.st_uid != os.getuid() or info.st_mode & 0o022:
                        raise ValueError("unsafe voices directory")
                    parent, name = voices, "bm_fable.bin"
                fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
                try:
                    verify(fd, size, digest)
                finally:
                    os.close(fd)
            except FileNotFoundError:
                assets = False
            finally:
                if voices is not None:
                    os.close(voices)
        try:
            info = os.stat("apparent-narrator", dir_fd=cache, follow_symlinks=False)
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o022:
                raise ValueError("worker must be a regular user-owned file, not writable by others")
            worker = bool(info.st_mode & stat.S_IXUSR)
        except FileNotFoundError:
            worker = False
        return {"assets": assets, "worker": worker}
    finally:
        os.close(cache)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", type=Path, default=DEFAULT_CACHE)
    parser.add_argument("--operation", choices=["check", "assets", "build"])
    args, extra = parser.parse_known_args()
    if args.operation is None:
        # Compatibility entry point; product setup policy has one TypeScript owner.
        os.execvp("node", ["node", str(Path(__file__).with_suffix(".ts")), *os.sys.argv[1:]])
    if extra:
        parser.error("unexpected arguments")
    if args.operation == "check":
        print(json.dumps(inspect_cache(args.cache)))
        return
    cache = open_cache(args.cache)
    try:
        if args.operation == "assets":
            voices = child_directory(cache, "voices")
            try:
                info = os.fstat(voices)
                if info.st_uid != os.getuid() or info.st_mode & 0o022:
                    raise ValueError("unsafe voices directory")
                ensure_asset(cache, "model.onnx", MODEL_URL, MODEL_BYTES, MODEL_SHA256)
                ensure_asset(voices, "bm_fable.bin", VOICE_URL, VOICE_BYTES, VOICE_SHA256)
            finally:
                os.close(voices)
        else:
            with tempfile.TemporaryDirectory(prefix="apparent-dev-narrator-build-", dir="/tmp") as target:
                jobs = str(max(1, min(3, int(os.environ.get("T3_BUILD_JOBS", "3")))))
                command = ["cargo", "build", "--release", "--locked", "-j", jobs, "--manifest-path",
                           str(ROOT / "capture/agent/narrator/Cargo.toml"), "--target-dir", target]
                subprocess.run(command, check=True, timeout=1800)
                install_worker(cache, Path(target) / "release/apparent-narrator")
    finally:
        os.close(cache)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        raise SystemExit(f"narrator provisioning failed: {error}") from error
