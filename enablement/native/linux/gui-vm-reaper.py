#!/usr/bin/env python3
"""Expire a disposable GUI guest even if its runner was killed."""

import fcntl
import json
import os
from pathlib import Path
import signal
import stat
import sys
import time

COLD_REQUEST = 'retain-cold-state.json'
ARTIFACTS_RETAINED = 'remote exit observed; sha256 manifest verified\n'


def private_directory(directory):
    if directory.is_symlink():
        raise RuntimeError('refusing symlink job directory')
    directory = directory.resolve()
    metadata = directory.stat()
    if metadata.st_uid != os.getuid() or metadata.st_mode & 0o077:
        raise RuntimeError('job directory must be private and owned by this user')
    return directory, metadata


def private_read(path, limit=4096):
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW), 'rb') as source:
        before = os.fstat(source.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_uid != os.getuid() or before.st_nlink != 1 or before.st_mode & 0o022 or before.st_size > limit:
            raise RuntimeError('unsafe private retention record')
        data = source.read(limit + 1)
        if len(data) > limit or identity(os.fstat(source.fileno())) != identity(before) or identity(path.lstat()) != identity(before):
            raise RuntimeError('retention record changed')
    return data


def identity(metadata):
    return {key: getattr(metadata, 'st_' + key) for key in ('dev', 'ino', 'uid', 'mode', 'nlink', 'size', 'mtime_ns', 'ctime_ns')}


def exclusive_record(path, value):
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600), 'w') as output:
        os.fchmod(output.fileno(), 0o600)
        json.dump(value, output, sort_keys=True)
        output.write('\n')
        output.flush()
        os.fsync(output.fileno())
    parent = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(parent)
    finally:
        os.close(parent)


def request_cold_retention(directory):
    directory, meta = private_directory(directory)
    exclusive_record(directory / COLD_REQUEST, {'version': 1, 'mode': 'retain-cold-state',
                     'directory': str(directory), 'device': meta.st_dev, 'inode': meta.st_ino, 'uid': meta.st_uid})


def cold_retention_requested(directory):
    directory, meta = private_directory(directory)
    path = directory / COLD_REQUEST
    if not os.path.lexists(path):
        return False
    try:
        value = json.loads(private_read(path))
    except (ValueError, UnicodeError) as error:
        raise RuntimeError('retention request is malformed') from error
    expected = {'version': 1, 'mode': 'retain-cold-state', 'directory': str(directory),
                'device': meta.st_dev, 'inode': meta.st_ino, 'uid': meta.st_uid}
    if value != expected:
        raise RuntimeError('retention request does not bind this owned job')
    return True


def resource_metadata(path):
    meta = path.lstat()
    if not stat.S_ISREG(meta.st_mode) or meta.st_uid != os.getuid() or meta.st_nlink != 1 or meta.st_mode & 0o022:
        raise RuntimeError('cold resource is not an owned single-link regular file')
    return identity(meta)


def open_resource_owners(resources, proc=Path('/proc'), seconds=2):
    """Metadata-only FD census of this UID; unknown access is never absence.

    Match inodes, not argv/path text, including renamed/deleted resources. No
    unrelated descriptor names or contents are retained or printed.
    """
    end = time.monotonic() + seconds
    targets = {(meta['dev'], meta['ino']) for meta in resources.values()}
    owners = set()
    for process in proc.iterdir():
        if time.monotonic() >= end:
            raise TimeoutError('cold resource ownership observation expired')
        if not process.name.isdigit():
            continue
        try:
            # /proc directory ownership can become root for a non-dumpable
            # same-UID process. Read actual UIDs, never infer them from that.
            uids = next((line.split()[1:] for line in (process / 'status').read_text().splitlines()
                         if line.startswith('Uid:')), [])
            if len(uids) != 4:
                raise RuntimeError('process UID observation is incomplete')
            try:
                process_uids = [int(uid) for uid in uids]
            except ValueError as error:
                raise RuntimeError('process UID observation is malformed') from error
            if os.getuid() not in process_uids:
                continue
            for index, descriptor in enumerate((process / 'fd').iterdir()):
                if index >= 4096 or time.monotonic() >= end:
                    raise TimeoutError('cold resource ownership observation incomplete')
                try:
                    meta = descriptor.stat()
                    if (meta.st_dev, meta.st_ino) in targets:
                        owners.add(int(process.name))
                except FileNotFoundError:
                    pass
        except FileNotFoundError:
            pass
        except PermissionError as error:
            raise RuntimeError('cold resource ownership access is unknown') from error
    if time.monotonic() >= end:
        raise TimeoutError('cold resource ownership observation expired')
    return sorted(owners)


def retain_cold_resources(directory, origin):
    """Observe owned cold files, not image equivalence or native acceptance."""
    if origin not in ('runner', 'expiry'):
        raise RuntimeError('unknown cold retention origin')
    directory, _ = private_directory(directory)
    if not cold_retention_requested(directory):
        raise RuntimeError('cold retention was not explicitly requested')
    resources = [directory / name for name in ('disk.qcow2', 'OVMF_VARS.fd')]
    if not any(os.path.lexists(path) for path in resources):
        return None
    before = {path.name: resource_metadata(path) for path in resources}
    if list(qemu_users(resources[0])) or open_resource_owners(before):
        raise RuntimeError('job disk or firmware still has an owner; cold state unqualified')
    verified = (directory / 'artifacts-retained').exists()
    if verified and private_read(directory / 'artifacts-retained').decode() != ARTIFACTS_RETAINED:
        raise RuntimeError('verified artifact retention marker is unknown')
    after = {path.name: resource_metadata(path) for path in resources}
    if after != before:
        raise RuntimeError('cold resource identity changed during observation')
    receipt = {'version': 1, 'origin': origin, 'directory': str(directory), 'resources': after,
               'owners': [], 'artifact_retention': 'verified' if verified else 'unverified',
               'scope': 'owned cold resources only; NOT standalone/equivalence/native admission',
               'source_hashes': 'must be freshly sealed in bounded offline handoff preparation'}
    exclusive_record(directory / ('cold-state-' + origin + '.json'), receipt)
    return receipt


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
    directory, _ = private_directory(directory)
    disk = directory / "disk.qcow2"
    if not os.path.lexists(disk):
        # No initial drive binding means no signalling authority, even if an
        # argv would match. Refuse an incomplete requested pair; preserve it.
        if cold_retention_requested(directory):
            retain_cold_resources(directory, 'expiry')
        return
    # Establish this job's drive authority before any signal. Retention policy
    # and firmware qualification must NOT prevent authorized owned-QEMU drain.
    meta = disk.lstat()
    if not stat.S_ISREG(meta.st_mode) or meta.st_uid != os.getuid() or meta.st_nlink != 1:
        raise RuntimeError('job disk ownership is unknown; refusing to signal')
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
    # Invalid requests fail here, AFTER owned-process teardown, without falling
    # through to the default deletion policy. Preserve the original error/files.
    retain = cold_retention_requested(directory)
    if not disk.exists():
        if retain:
            retain_cold_resources(directory, 'expiry')
        return
    # A killed runner may never have copied the evidence. Stop writers, but
    # preserve the only copy on the cold disk until retention was verified
    # after an observed remote exit. A partial live snapshot earns no marker.
    names = ['known_hosts', 'qmp.sock', 'vnc.sock']
    if retain:
        retain_cold_resources(directory, 'expiry')
    elif (directory / 'artifacts-retained').is_file():
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
