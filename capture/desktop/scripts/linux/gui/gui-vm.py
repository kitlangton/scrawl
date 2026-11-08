#!/usr/bin/env python3
"""Run one isolated Scrawl GUI scenario against a stopped-VM template.

The template is never writable. Each invocation owns its disk overlay, OVMF state,
network forwarding, VNC/QMP sockets, SSH host key cache, and artifact directory.
"""

import argparse
import contextlib
import fcntl
import json
import hashlib
import os
from pathlib import Path
import re
import shutil
import shlex
import signal
import socket
import subprocess
import sys
import time
import uuid


def execute(command, timeout: float = 30, check=True, **kwargs):
    """Keep subprocess diagnostics on the exception, including timeouts."""
    try:
        return subprocess.run(command, check=check, timeout=timeout, text=True,
                              capture_output=True, **kwargs)
    except OSError as error:
        setattr(error, 'cmd', command)
        raise


def run_checked(*args):
    return execute(list(args)).stdout


def bounded(value):
    if isinstance(value, bytes):
        value = value.decode('utf-8', errors='replace')
    return (value or '')[-8192:]


def failure_record(stage, error):
    command = getattr(error, 'cmd', None)
    return {'stage': stage, 'classification': 'driver/unclassified' if stage == 'scenario' else 'driver',
            'type': type(error).__name__, 'message': bounded(str(error)),
            'command': command, 'returncode': getattr(error, 'returncode', None),
            'stdout': bounded(getattr(error, 'stdout', '')),
            'stderr': bounded(getattr(error, 'stderr', ''))}


def write_result(directory, status):
    # Runner and independent expiry may finish together. Serialize read/replace
    # and keep the first persisted failure and terminal expiry facts.
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
        try:
            previous = json.loads((directory / 'result.json').read_text())
        except (FileNotFoundError, ValueError):
            previous = {}
        if not isinstance(previous, dict):
            previous = {}
        primary = previous.get('primary_failure')
        if primary:
            if status.get('primary_failure') and status['primary_failure'] != primary:
                failures = status.setdefault('secondary_failures', [])
                if status['primary_failure'] not in failures:
                    failures.append(status['primary_failure'])
            status['primary_failure'] = primary
            status['driver'] = 'failed'
            if status.get('exit_code') == 0:
                status['exit_code'] = 1
        # A runner may have prepared its receipt before the reaper fired. It
        # cannot turn expiry back into successful cleanup, or erase diagnostics.
        secondary = []
        cleanup_errors = []
        for receipt in (previous, status):
            for error in receipt.get('secondary_failures', []):
                if error not in secondary:
                    secondary.append(error)
            for error in receipt.get('cleanup', {}).get('errors', []):
                if error not in cleanup_errors:
                    cleanup_errors.append(error)
        if previous.get('cleanup', {}).get('expiry') == 'fired':
            status.clear()
            status.update(previous)
        status['secondary_failures'] = secondary
        if cleanup_errors:
            cleanup = status.setdefault('cleanup', {})
            cleanup['errors'] = cleanup_errors
            cleanup['status'] = 'failed'
            if status.get('exit_code') == 0:
                status['exit_code'] = 1
        replace_result(directory, status)


def replace_result(directory, status):
    temporary = directory / 'result.json.tmp'
    with temporary.open('w') as output:
        os.fchmod(output.fileno(), 0o600)
        json.dump(status, output, indent=2)
        output.write('\n')
    temporary.replace(directory / 'result.json')


def validate_socket_paths(directory):
    # sockaddr_un.sun_path is 108 bytes including the trailing NUL on Linux.
    for name in ('qmp.sock', 'vnc.sock'):
        if len(os.fsencode(directory / name)) > 107:
            raise RuntimeError('Unix socket path exceeds 107 bytes; use a shorter --results directory')


def file_digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


# Used verbatim on both ends: successful scp alone is not verified retention.
MANIFEST_SCRIPT = '''import hashlib, json, pathlib, sys
root = pathlib.Path(sys.argv[1])
if not root.is_dir() or root.is_symlink(): raise RuntimeError("missing artifact directory")
result = {}
for path in sorted(root.rglob("*")):
    if path.is_symlink(): raise RuntimeError("symlink in artifact directory")
    if path.is_file():
        with path.open("rb") as source:
            result[str(path.relative_to(root))] = hashlib.file_digest(source, "sha256").hexdigest()
print(json.dumps(result, sort_keys=True))
'''


def qemu_users(path):
    """Fail closed if a QEMU command line references this disk as a live drive."""
    target = ('file=' + str(path.resolve())).encode()
    for proc in Path("/proc").iterdir():
        if not proc.name.isdigit():
            continue
        try:
            args = (proc / "cmdline").read_bytes().split(b"\0")
        except (OSError, PermissionError):
            continue
        if not args or b"qemu-system-" not in args[0]:
            continue
        if any(target in arg.split(b',') for arg in args[1:]):
            yield proc.name


def available_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def make_overlay(template, destination):
    if list(qemu_users(template)):
        raise RuntimeError("template is in use by a running QEMU process")
    if not template.is_file():
        raise RuntimeError(f"missing template: {template}")
    info = json.loads(run_checked("qemu-img", "info", "--output=json", str(template)))
    if info.get("format") != "qcow2" or info.get("backing-filename"):
        raise RuntimeError("template must be a standalone qcow2 image")
    run_checked("qemu-img", "create", "-f", "qcow2", "-F", "qcow2", "-b", str(template.resolve()), str(destination))

def copy_firmware(template, destination):
    shutil.copyfile(template, destination)
    destination.chmod(0o600)

def arm_expiry(directory, seconds):
    unit = "scrawl-gui-expire-" + uuid.uuid4().hex
    # Keep the reaper beside its job: a temporary code worktree may disappear
    # after the runner is killed but before the independent timer fires.
    reaper = directory / "reaper.py"
    shutil.copyfile(Path(__file__).with_name("gui-vm-reaper.py"), reaper)
    reaper.chmod(0o700)
    execute(["systemd-run", "--user", "--quiet", "--collect",
                    "--timer-property=RemainAfterElapse=no", "--unit=" + unit,
                    f"--on-active={seconds}s", "/usr/bin/python3",
                    str(reaper), str(directory)],
                   timeout=10)
    return unit


def qemu_command(overlay, vars_copy, directory, port, memory, isolated_network=False):
    command = [
        "qemu-system-x86_64", "-name", "scrawl-gui-" + directory.name,
        "-machine", "q35,accel=kvm", "-cpu", "host", "-smp", "4", "-m", str(memory),
        "-drive", "if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd",
        "-drive", "if=pflash,format=raw,file=" + str(vars_copy),
        "-drive", "if=virtio,format=qcow2,file=" + str(overlay) + ",cache=none",
        "-boot", "order=c", "-device", "virtio-vga", "-display", "none",
        "-vnc", "unix:" + str(directory / "vnc.sock"),
        "-device", "qemu-xhci", "-device", "usb-tablet", "-device", "usb-kbd",
        "-netdev", f"user,id=net0,hostfwd=tcp:127.0.0.1:{port}-:22" + (",restrict=on" if isolated_network else ""),
        "-device", "virtio-net-pci,netdev=net0",
        "-qmp", "unix:" + str(directory / "qmp.sock") + ",server=on,wait=off",
        "-monitor", "none", "-serial", "none",
    ]
    kvm_group = Path("/dev/kvm").stat().st_gid
    if kvm_group in os.getgroups():
        return command
    groups = sorted(set(os.getgroups()) | {kvm_group})
    # Drop straight back to the calling UID; only the supplementary KVM group is new.
    return ["sudo", "-n", "env", "HOME=" + str(Path.home()),
            "XDG_CACHE_HOME=" + str(Path.home() / ".cache"), "setpriv",
            "--reuid=" + str(os.getuid()), "--regid=" + str(os.getgid()),
            "--groups=" + ",".join(map(str, groups))] + command


def ssh_prefix(port, key, known_hosts, user):
    return ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=3",
            "-o", "StrictHostKeyChecking=accept-new", "-o", "UserKnownHostsFile=" + str(known_hosts),
            "-i", str(key), "-p", str(port), f"{user}@127.0.0.1"]


def wait_for_desktop(prefix, process, seconds):
    deadline = time.monotonic() + seconds
    command = prefix + [
        'pgrep -u "$(id -u)" -x Hyprland >/dev/null && '
        'systemctl --user show-environment | grep -q "^HYPRLAND_INSTANCE_SIGNATURE="'
    ]
    last = None
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"guest exited before readiness (code {process.returncode})")
        try:
            probe = execute(command, check=False,
                            timeout=min(5, max(0.1, deadline - time.monotonic())))
            last = probe
        except subprocess.TimeoutExpired as error:
            last = error
            continue
        if probe.returncode == 0:
            return
        time.sleep(min(2, max(0, deadline - time.monotonic())))
    error = TimeoutError(f"guest user desktop did not become ready within {seconds}s")
    for name, value in (('cmd', command), ('stdout', getattr(last, 'stdout', '')),
                        ('stderr', getattr(last, 'stderr', '')),
                        ('returncode', getattr(last, 'returncode', None))):
        setattr(error, name, value)
    raise error


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--template", type=Path, required=True, help="offline, standalone qcow2 disk")
    parser.add_argument("--ovmf-vars", type=Path, required=True, help="template's powered-off OVMF vars")
    parser.add_argument("--ssh-key", type=Path, required=True)
    parser.add_argument("--ssh-user", required=True)
    parser.add_argument("--scenario", type=Path, required=True, help="guest bash script; runs after Hyprland starts")
    parser.add_argument("--audio-file", type=Path, help="optional WAV stimulus staged inside the guest")
    parser.add_argument("--binary", type=Path, required=True, help="candidate executable (sibling .so files are staged too)")
    parser.add_argument("--results", type=Path, required=True, help="new directory; retained after the run")
    parser.add_argument("--memory", type=int, default=3072, help="guest MiB (default: 3072)")
    parser.add_argument("--isolated-network", action="store_true", help="deny guest outbound networking; preserve loopback SSH forwarding")
    parser.add_argument("--boot-timeout", type=int, default=180)
    parser.add_argument("--scenario-timeout", type=int, default=300)
    parser.add_argument("--job-timeout", type=int, default=900,
                        help="hard lifetime in seconds; independent systemd timer (default: 900)")
    args = parser.parse_args()
    if args.results.exists():
        parser.error("results directory already exists")
    args.results.mkdir(parents=True, mode=0o700)
    args.results.chmod(0o700)
    directory = args.results.resolve()
    process = None
    expiry_unit = None
    scenario_completed = False
    start = time.monotonic()
    deadline = start + args.job_timeout
    status = {'version': 1, 'stage': 'preflight', 'progress': ['preflight'],
              'environment': 'unknown', 'driver': 'running', 'product': 'unrun',
              'unrun': ['scenario'], 'scenario_exit': None, 'candidate_sha256': None,
              'primary_failure': None, 'secondary_failures': [], 'artifact_retention': 'not_attempted',
              'cleanup': {'status': 'pending', 'disk': 'unknown', 'expiry': 'not_armed', 'errors': []}}
    code = 1

    def stage(name):
        status['stage'] = name
        status['progress'].append(name)
        write_result(directory, status)

    def failure(error):
        if status['primary_failure'] is None:
            status['primary_failure'] = failure_record(status['stage'], error)
        else:
            status['secondary_failures'].append(failure_record(status['stage'], error))

    def call(command, timeout=30, check=True):
        remaining = deadline - time.monotonic() - 25  # reserve owned shutdown
        if remaining <= 0:
            raise subprocess.TimeoutExpired(command, args.job_timeout)
        return execute(command, timeout=min(timeout, remaining), check=check)

    try:
        write_result(directory, status)
        validate_socket_paths(directory)
        for path in (args.template, args.ovmf_vars, args.ssh_key, args.scenario, args.binary, args.audio_file):
            if path is not None and not path.is_file():
                raise RuntimeError(f'missing file: {path}')
        if not re.fullmatch(r'[a-z_][a-z0-9_-]*', args.ssh_user):
            raise RuntimeError('ssh-user must be a simple Unix account name')
        if args.memory < 2048 or args.job_timeout < 30 or min(args.boot_timeout, args.scenario_timeout) <= 0:
            raise RuntimeError('require memory >= 2048, job-timeout >= 30 and positive boot/scenario timeouts')
        if any(',' in str(path.resolve()) for path in (directory, args.template, args.ovmf_vars)):
            raise RuntimeError('QEMU disk and firmware paths must not contain commas')
        status['candidate_sha256'] = file_digest(args.binary)
        stage('overlay')
        overlay = directory / "disk.qcow2"
        make_overlay(args.template, overlay)
        vars_copy = directory / "OVMF_VARS.fd"
        copy_firmware(args.ovmf_vars, vars_copy)
        port = available_port()
        command = qemu_command(overlay, vars_copy, directory, port, args.memory, args.isolated_network)
        # The user manager survives a SIGKILL or OOM kill of this runner.
        # Refuse to boot at all if that independent expiry cannot be armed.
        expiry_unit = arm_expiry(directory, args.job_timeout)
        status['cleanup']['expiry'] = 'armed'
        stage('boot')
        if deadline - time.monotonic() <= 25:
            raise subprocess.TimeoutExpired(command, args.job_timeout)
        with (directory / "qemu.log").open("wb") as log:
            try:
                process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            except OSError as error:
                setattr(error, 'cmd', command)
                raise
            prefix = ssh_prefix(port, args.ssh_key.resolve(), directory / "known_hosts", args.ssh_user)
            try:
                wait_for_desktop(prefix, process, min(args.boot_timeout, max(0, deadline - time.monotonic() - 25)))
            except RuntimeError as error:
                # A QEMU exit is a boot/driver failure, not product evidence.
                setattr(error, 'cmd', command)
                setattr(error, 'returncode', process.poll())
                with (directory / 'qemu.log').open('rb') as source:
                    source.seek(max(0, source.seek(0, os.SEEK_END) - 8192))
                    setattr(error, 'stderr', source.read())
                raise
            boot_seconds = round(time.monotonic() - start, 2)
            status['boot_seconds'] = boot_seconds
            status['environment'] = 'ready'
            stage('staging')
            remote = "/home/" + args.ssh_user + "/scrawl-gui-scenario-" + uuid.uuid4().hex + ".sh"
            guest_output = "/home/" + args.ssh_user + "/scrawl-gui-output-" + uuid.uuid4().hex
            guest_bin = "/home/" + args.ssh_user + "/scrawl-gui-bin-" + uuid.uuid4().hex
            call(prefix + ["mkdir -p " + shlex.quote(guest_bin)], timeout=10)
            files = [args.binary.resolve()] + sorted(args.binary.parent.glob("lib*.so*"))
            transfer = ["scp", "-q", "-P", str(port), "-i", str(args.ssh_key.resolve()),
                        "-o", "StrictHostKeyChecking=accept-new",
                        "-o", "UserKnownHostsFile=" + str(directory / "known_hosts")]
            call(transfer + [*[str(path) for path in files],
                            f"{args.ssh_user}@127.0.0.1:{guest_bin}/"], timeout=120)
            digest = status['candidate_sha256']
            guest_binary = guest_bin + "/" + args.binary.name
            actual = call(prefix + ["sha256sum " + shlex.quote(guest_binary)]).stdout.split()[0]
            if actual != digest:
                raise RuntimeError("staged executable hash differs from candidate")
            scp = transfer + [str(args.scenario), f"{args.ssh_user}@127.0.0.1:{remote}"]
            call(scp)
            audio_file = ""
            if args.audio_file:
                call(prefix + ["mkdir -p " + shlex.quote(guest_output)], timeout=10)
                audio_file = guest_output + "/stimulus.wav"
                call(transfer + [str(args.audio_file.resolve()),
                                 f"{args.ssh_user}@127.0.0.1:{audio_file}"], timeout=120)
            try:
                stage('scenario')
                status['unrun'] = []
                status['product'] = 'unknown'
                scenario_command = prefix + ["mkdir -p " + shlex.quote(guest_output) +
                                            " && SCRAWL_GUI_OUTPUT=" + shlex.quote(guest_output) +
                                            " SCRAWL_GUI_BINARY=" + shlex.quote(guest_binary) +
                                            " SCRAWL_GUI_AUDIO_FILE=" + shlex.quote(audio_file) +
                                            " LD_LIBRARY_PATH=" + shlex.quote(guest_bin) +
                                            " bash " + shlex.quote(remote)]
                # Files keep full private output without buffering a long recipe in RAM.
                with (directory / 'scenario.log').open('w') as output, (directory / 'scenario.stderr').open('w') as errors:
                    try:
                        result = subprocess.run(scenario_command, stdout=output, stderr=errors,
                                                timeout=min(args.scenario_timeout, max(0.1, deadline - time.monotonic() - 25)))
                        status['scenario_exit'] = result.returncode
                        # Only successful trusted scenarios certify their writer
                        # cleanup. A failure may leave descendants alive, even
                        # when SSH returned the remote shell's exit status.
                        scenario_completed = result.returncode == 0
                        code = result.returncode
                        if code:
                            raise subprocess.CalledProcessError(code, scenario_command)
                    except (OSError, subprocess.SubprocessError, KeyboardInterrupt, SystemExit) as error:
                        if not hasattr(error, 'cmd'):
                            setattr(error, 'cmd', scenario_command)
                        for key, path in (('stdout', directory / 'scenario.log'), ('stderr', directory / 'scenario.stderr')):
                            with path.open('rb') as source:
                                source.seek(max(0, path.stat().st_size - 8192))
                                setattr(error, key, source.read())
                        failure(error)
                        code = (143 if isinstance(error, SystemExit) else
                                130 if isinstance(error, KeyboardInterrupt) else status['scenario_exit'] or 1)
            finally:
                # Preserve guest diagnostics even when the scenario times out.
                stage('retention')
                status['artifact_retention'] = 'unverified'
                expected = json.loads(call(prefix + ['python3 -c ' + shlex.quote(MANIFEST_SCRIPT) + ' ' + shlex.quote(guest_output)]).stdout)
                call(transfer[:2] + ["-r"] + transfer[2:] +
                     [f"{args.ssh_user}@127.0.0.1:{guest_output}/.", str(directory / "artifacts")], timeout=60)
                actual = json.loads(call([sys.executable, '-c', MANIFEST_SCRIPT, str(directory / 'artifacts')]).stdout)
                if actual != expected:
                    raise RuntimeError('artifact transfer hash manifest mismatch; keeping cold job disk')
                if scenario_completed:
                    status['artifact_retention'] = 'verified'
                    (directory / 'artifacts-retained').write_text('remote exit observed; sha256 manifest verified\n')
                else:
                    # Matching hashes prove only this snapshot, not that a
                    # remote writer cannot produce more evidence after it.
                    status['artifact_retention'] = 'partial_snapshot'
    except Exception as error:
        failure(error)
        code = code or 1
    except (KeyboardInterrupt, SystemExit) as error:
        failure(error)
        code = 143 if isinstance(error, SystemExit) else 130
    finally:
        status['driver'] = 'failed' if status['primary_failure'] else 'passed'
        if status['environment'] == 'unknown':
            status['environment'] = 'failed'
        status['stage'] = 'cleanup'
        status['progress'].append('cleanup')
        try:
            if process is not None and process.poll() is None:
                # Only signal the group while its owned leader is still unreaped.
                with contextlib.suppress(ProcessLookupError):
                    os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    with contextlib.suppress(ProcessLookupError):
                        os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=10)
            if list(qemu_users(directory / 'disk.qcow2')):
                status['cleanup']['disk'] = 'retained_live'
                raise RuntimeError('QEMU still owns job disk; expiry remains armed')
            status['cleanup']['disk'] = 'retained_cold'
            if status['artifact_retention'] == 'verified' or process is None:
                for filename in ('disk.qcow2', 'OVMF_VARS.fd'):
                    (directory / filename).unlink(missing_ok=True)
                status['cleanup']['disk'] = 'removed'
            for filename in ('known_hosts', 'qmp.sock', 'vnc.sock'):
                (directory / filename).unlink(missing_ok=True)
            if expiry_unit is not None:
                execute(['systemctl', '--user', 'stop', expiry_unit + '.timer'], timeout=10)
                status['cleanup']['expiry'] = 'stopped'
            status['cleanup']['status'] = 'complete'
        except Exception as error:
            status['cleanup']['status'] = 'failed'
            status['cleanup']['errors'].append(failure_record('cleanup', error))
            code = code or 1
        status['elapsed_seconds'] = round(time.monotonic() - start, 2)
        status['exit_code'] = code
        status['stage'] = 'finished'
        write_result(directory, status)
        code = status['exit_code']
    # Diagnostics may contain private guest output; only the private receipt
    # carries them, not a caller's potentially shared console transcript.
    print(json.dumps({'result_path': str(directory / 'result.json'),
                      'scenario_exit': status['scenario_exit'], 'exit_code': code}))
    return code


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    sys.exit(main())
