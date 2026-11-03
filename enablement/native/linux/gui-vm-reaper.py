#!/usr/bin/env python3
"""Expire a disposable GUI guest even if its runner was killed."""

import fcntl
import json
import os
from pathlib import Path
import signal
import sys
import time


def qemu_users(disk):
    """Find only QEMU processes whose drive argument names this exact job disk."""
    target = ("file=" + str(disk.resolve())).encode()
    for proc in Path("/proc").iterdir():
        if not proc.name.isdigit():
            continue
        try:
            args = (proc / "cmdline").read_bytes().split(b"\0")
        except OSError:
            continue
        if not args or b"qemu-system-" not in args[0]:
            continue
        if any(target in arg.split(b",") for arg in args[1:]):
            yield int(proc.name)


def signal_owner(pid, disk, signum):
    """Pin the process before rechecking its drive/UID; a reused PID is not ours."""
    try:
        with os.fdopen(os.pidfd_open(pid), 'rb') as owner:
            if Path(f'/proc/{pid}').stat().st_uid != os.getuid():
                raise RuntimeError('foreign QEMU owns job disk; refusing to signal or unlink')
            if pid in qemu_users(disk):
                signal.pidfd_send_signal(owner.fileno(), signum)
    except (ProcessLookupError, FileNotFoundError):
        pass


def reap(directory):
    if directory.is_symlink():
        raise RuntimeError('refusing symlink job directory')
    directory = directory.resolve()
    metadata = directory.stat()
    if metadata.st_uid != os.getuid() or metadata.st_mode & 0o077:
        raise RuntimeError('job directory must be private and owned by this user')
    disk = directory / "disk.qcow2"
    if not disk.exists():
        return
    owners = list(qemu_users(disk))
    for pid in owners:
        signal_owner(pid, disk, signal.SIGTERM)
    for _ in range(10):
        if not list(qemu_users(disk)):
            break
        time.sleep(0.5)
    else:
        for pid in list(qemu_users(disk)):
            signal_owner(pid, disk, signal.SIGKILL)
        for _ in range(10):
            if not list(qemu_users(disk)):
                break
            time.sleep(0.5)
    if list(qemu_users(disk)):
        raise RuntimeError("QEMU still owns job disk; refusing to unlink it")
    # A killed runner may never have copied the evidence. Stop writers, but
    # preserve the only copy on the cold disk until retention was verified
    # after an observed remote exit. A partial live snapshot earns no marker.
    names = ['known_hosts', 'qmp.sock', 'vnc.sock']
    if (directory / 'artifacts-retained').is_file():
        names += ['disk.qcow2', 'OVMF_VARS.fd']
    for name in names:
        (directory / name).unlink(missing_ok=True)


def expire(directory):
    """Keep the runner's first failure even when expiry finds a cleanup failure."""
    error = None
    try:
        reap(directory)
    except (OSError, RuntimeError) as caught:
        error = caught
    # Do not create a receipt through an unowned/symlink directory on rejection.
    if directory.is_symlink() or directory.stat().st_uid != os.getuid() or directory.stat().st_mode & 0o077:
        raise RuntimeError('refusing unowned/private job receipt')
    with (directory / 'result.lock').open('a') as lock:
        os.fchmod(lock.fileno(), 0o600)
        deadline = time.monotonic() + 2
        while True:
            try:
                fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic() >= deadline:
                    raise TimeoutError('private result receipt lock remained busy')
                time.sleep(.02)
        record_expiry(directory, error)


def record_expiry(directory, error):
    path = directory / 'result.json'
    try:
        status = json.loads(path.read_text())
        if not isinstance(status, dict):
            raise ValueError('invalid receipt')
    except (FileNotFoundError, ValueError):
        status = {'version': 1, 'environment': 'unknown', 'driver': 'failed',
                  'product': 'unrun', 'unrun': ['scenario'], 'candidate_sha256': None,
                  'scenario_exit': None, 'progress': []}
    status['stage'] = 'expired'
    status.setdefault('progress', []).append('expired')
    if not status.get('primary_failure'):
        status['primary_failure'] = {'stage': 'expiry', 'type': 'TimeoutError',
                                     'message': 'independent job lifetime expired',
                                     'command': None, 'returncode': None, 'stdout': '', 'stderr': ''}
    status['driver'] = 'failed'
    status['exit_code'] = 1
    status['artifact_retention'] = 'verified' if (directory / 'artifacts-retained').is_file() else 'unverified'
    prior_errors = status.get('cleanup', {}).get('errors', [])
    status['cleanup'] = {'status': 'failed' if error or prior_errors else 'complete',
                         'disk': ('retained_unverified' if error else 'retained_cold') if (directory / 'disk.qcow2').exists() else 'removed',
                         'expiry': 'fired',
                         'errors': prior_errors + ([{'stage': 'expiry', 'message': str(error)}] if error else [])}
    temporary = directory / 'expiry-result.json.tmp'
    with temporary.open('w') as output:
        os.fchmod(output.fileno(), 0o600)
        json.dump(status, output, indent=2)
        output.write('\n')
    temporary.replace(path)
    if error:
        raise error
    if prior_errors:
        raise RuntimeError('prior cleanup failure remains unresolved; see private result.json')


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("usage: gui-vm-reaper.py JOB_DIRECTORY")
    expire(Path(sys.argv[1]))
