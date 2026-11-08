#!/usr/bin/env python3
"""Contract tests for disposable Omarchy guest creation; no live VM required."""

import importlib.util
import hashlib
import contextlib
import io
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

MODULE = Path(__file__).with_name("gui-vm.py")
spec = importlib.util.spec_from_file_location("gui_vm", MODULE)
assert spec is not None and spec.loader is not None
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
REAPER = MODULE.with_name("gui-vm-reaper.py")

def load_reaper(path=REAPER):
    spec = importlib.util.spec_from_file_location("gui_vm_reaper", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class GuestIsolationTests(unittest.TestCase):
    def test_command_failure_keeps_exact_command_and_both_streams(self):
        command = [sys.executable, '-c', 'import sys; print("out"); sys.stderr.write("diagnostic\\n"); sys.exit(7)']
        with self.assertRaises(subprocess.CalledProcessError) as failed:
            runner.execute(command, timeout=5)
        receipt = runner.failure_record('stage', failed.exception)
        self.assertEqual(receipt['command'], command)
        self.assertEqual(receipt['returncode'], 7)
        self.assertEqual(receipt['stdout'], 'out\n')
        self.assertEqual(receipt['stderr'], 'diagnostic\n')

    def test_long_socket_path_is_rejected_before_launch(self):
        with self.assertRaisesRegex(RuntimeError, 'socket path'):
            runner.validate_socket_paths(Path('/' + 'x' * 110))

    def test_accepted_socket_boundary_really_binds(self):
        with tempfile.TemporaryDirectory() as root:
            prefix = Path(root).resolve()
            directory = prefix / ('x' * (107 - len(os.fsencode(prefix)) - len('/qmp.sock') - 1))
            directory.mkdir()
            runner.validate_socket_paths(directory)
            with socket.socket(socket.AF_UNIX) as listener:
                listener.bind(str(directory / 'qmp.sock'))
            with self.assertRaisesRegex(RuntimeError, 'socket path'):
                runner.validate_socket_paths(directory / 'x')

    def test_parser_rejects_abbreviated_options(self):
        with tempfile.TemporaryDirectory() as root:
            results = Path(root) / 'result'
            options = [item for name in ('template', 'ovmf-vars', 'ssh-key', 'ssh-user', 'scenario', 'binary')
                       for item in ('--' + name, 'missing')]
            result = subprocess.run([sys.executable, str(MODULE), *options, '--results', str(results), '--templ', 'missing'],
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('unrecognized arguments: --templ', result.stderr)
            self.assertFalse(results.exists())

    @patch.object(runner.os, 'getgroups', return_value=[1234])
    @patch.object(runner.Path, 'stat', return_value=SimpleNamespace(st_gid=1234))
    def test_fixture_network_denies_guest_egress_but_keeps_loopback_ssh(self, _stat, _groups):
        directory = Path('/synthetic/job')
        command = runner.qemu_command(directory / 'disk.qcow2', directory / 'OVMF_VARS.fd', directory, 23456, 3072, True)
        network = command[command.index('-netdev') + 1]
        self.assertEqual(network, 'user,id=net0,hostfwd=tcp:127.0.0.1:23456-:22,restrict=on')

    def test_expiry_is_armed_before_guest_boot_and_fail_closed(self):
        with tempfile.TemporaryDirectory() as scratch:
            with patch.object(runner.subprocess, "run") as command:
                command.return_value.returncode = 0
                unit = runner.arm_expiry(Path(scratch), 45)
                self.assertIn("--on-active=45s", command.call_args.args[0])
                self.assertIn(str(Path(scratch) / "reaper.py"), command.call_args.args[0])
                self.assertEqual((Path(scratch) / "reaper.py").read_bytes(), REAPER.read_bytes())
                self.assertTrue(unit.startswith("scrawl-gui-expire-"))
                command.side_effect = subprocess.CalledProcessError(1, ["systemd-run"])
                with self.assertRaises(subprocess.CalledProcessError):
                    runner.arm_expiry(Path(scratch), 45)

    def test_expiry_kills_only_job_qemu_then_removes_writable_state(self):
        reaper = load_reaper()
        with tempfile.TemporaryDirectory() as scratch:
            directory = Path(scratch)
            disk = directory / "disk.qcow2"
            disk.write_bytes(b"private overlay")
            (directory / "OVMF_VARS.fd").write_bytes(b"firmware")
            (directory / "scenario.log").write_text("keep evidence")
            (directory / 'artifacts-retained').write_text('verified fixture')
            with patch.object(reaper, "qemu_users", side_effect=[[321], [], []]) as users, \
                 patch.object(reaper, "signal_owner") as kill, \
                 patch.object(reaper.time, "sleep"):
                reaper.reap(directory)
            kill.assert_called_once_with(321, disk, signal.SIGTERM)
            self.assertEqual(users.call_args_list[0].args[0], disk)
            self.assertFalse(disk.exists())
            self.assertFalse((directory / "OVMF_VARS.fd").exists())
            self.assertTrue((directory / "scenario.log").exists())

    def test_expiry_preserves_disk_if_qemu_survives(self):
        reaper = load_reaper()
        with tempfile.TemporaryDirectory() as scratch:
            disk = Path(scratch) / "disk.qcow2"
            disk.write_bytes(b"private overlay")
            with patch.object(reaper, "qemu_users", return_value=[321]), \
                 patch.object(reaper, "signal_owner"), patch.object(reaper.time, "sleep"):
                with self.assertRaisesRegex(RuntimeError, "still owns"):
                    reaper.reap(Path(scratch))
            self.assertTrue(disk.exists())


    def test_readiness_requires_our_session_not_the_sddm_greeter(self):
        class RunningGuest:
            def poll(self):
                return None

        with patch.object(runner.subprocess, "run") as probe:
            probe.return_value.returncode = 0
            runner.wait_for_desktop(["ssh", "guest"], RunningGuest(), 1)
        command = probe.call_args.args[0][-1]
        self.assertIn('pgrep -u "$(id -u)" -x Hyprland', command)
        self.assertIn('HYPRLAND_INSTANCE_SIGNATURE=', command)

    def test_stalled_ssh_readiness_probe_obeys_boot_deadline(self):
        class RunningGuest:
            def poll(self):
                return None
        with patch.object(runner.subprocess, 'run', side_effect=subprocess.TimeoutExpired(['ssh'], 1)) as probe, \
             patch.object(runner.time, 'monotonic', side_effect=[0, 0, 0, 2]):
            with self.assertRaises(TimeoutError):
                runner.wait_for_desktop(['ssh', 'guest'], RunningGuest(), 1)
        self.assertEqual(probe.call_args.kwargs['timeout'], 1)

    def test_real_readiness_failure_retains_last_ssh_stderr(self):
        process = SimpleNamespace(poll=lambda: None)
        command = [sys.executable, '-c', 'import sys; print("probe"); sys.stderr.write("SSH diagnostic"); sys.exit(255)']
        with self.assertRaises(TimeoutError) as failed:
            runner.wait_for_desktop(command, process, .1)
        receipt = runner.failure_record('boot', failed.exception)
        self.assertEqual(receipt['command'][:len(command)], command)
        self.assertEqual(receipt['stdout'], 'probe\n')
        self.assertEqual(receipt['stderr'], 'SSH diagnostic')
        self.assertEqual(receipt['returncode'], 255)

    def test_firmware_copy_is_writable_even_when_template_is_read_only(self):
        with tempfile.TemporaryDirectory() as scratch:
            base, job = Path(scratch) / "base.fd", Path(scratch) / "job.fd"
            base.write_bytes(b"firmware")
            base.chmod(0o444)
            runner.copy_firmware(base, job)
            self.assertEqual(job.stat().st_mode & 0o777, 0o600)
            self.assertEqual(base.stat().st_mode & 0o777, 0o444)


    def test_overlay_leaves_template_unchanged(self):
        with tempfile.TemporaryDirectory() as scratch:
            directory = Path(scratch)
            base = directory / "base.qcow2"
            overlay = directory / "job.qcow2"
            subprocess.run(["qemu-img", "create", "-f", "qcow2", str(base), "1M"],
                           check=True, capture_output=True)
            before = base.read_bytes()
            runner.make_overlay(base, overlay)
            info = json.loads(runner.run_checked("qemu-img", "info", "--output=json", str(overlay)))
            self.assertEqual(info["backing-filename"], str(base.resolve()))
            self.assertEqual(base.read_bytes(), before)

    @patch.object(runner.os, 'getgroups', return_value=[1234])
    @patch.object(runner.Path, 'stat', return_value=SimpleNamespace(st_gid=1234))
    def test_distinct_jobs_have_distinct_state_and_loopback_ports(self, _stat, _groups):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            a, b = root / "job-a", root / "job-b"
            port_a, port_b = runner.available_port(), runner.available_port()
            # Kernel may reuse a freed ephemeral port; distinct chosen ports are a launcher requirement.
            while port_b == port_a:
                port_b = runner.available_port()
            commands = [runner.qemu_command(d / "disk.qcow2", d / "OVMF_VARS.fd", d, port, 8192)
                        for d, port in ((a, port_a), (b, port_b))]
            for command in commands:
                joined = " ".join(command)
                self.assertIn("hostfwd=tcp:127.0.0.1:", joined)
                self.assertNotIn("-vnc :", joined)
            self.assertNotEqual(commands[0], commands[1])
            self.assertIn("unix:" + str(a / "vnc.sock"), commands[0])
            self.assertIn("unix:" + str(b / "vnc.sock"), commands[1])

    def test_refuses_backing_chain_as_template(self):
        with tempfile.TemporaryDirectory() as scratch:
            directory = Path(scratch)
            base = directory / "base.qcow2"
            nested = directory / "nested.qcow2"
            runner.run_checked("qemu-img", "create", "-f", "qcow2", str(base), "1M")
            runner.make_overlay(base, nested)
            with self.assertRaisesRegex(RuntimeError, "standalone"):
                runner.make_overlay(nested, directory / "job.qcow2")


class ColdRetentionTests(unittest.TestCase):
    def prepare(self, directory, verified=True):
        module = load_reaper()
        directory.chmod(0o700)
        for name in ('disk.qcow2', 'OVMF_VARS.fd'):
            (directory / name).write_bytes(b'owned synthetic resource')
            (directory / name).chmod(0o600)
        module.request_cold_retention(directory)
        if verified:
            (directory / 'artifacts-retained').write_text(module.ARTIFACTS_RETAINED)
            (directory / 'artifacts-retained').chmod(0o600)
        return module

    def test_actual_fd_guard_detects_own_open_firmware_and_renamed_resource_without_argv_matching(self):
        module = load_reaper()
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            proc = root / 'proc'
            proc.mkdir()
            (proc / str(os.getpid())).symlink_to(Path('/proc') / str(os.getpid()), target_is_directory=True)
            firmware = root / 'OVMF_VARS.fd'
            firmware.write_bytes(b'tiny synthetic firmware')
            firmware.chmod(0o600)
            resources = {'OVMF_VARS.fd': module.resource_metadata(firmware)}
            self.assertEqual(module.open_resource_owners(resources, proc), [])
            with firmware.open('rb'):
                self.assertEqual(module.open_resource_owners(resources, proc), [os.getpid()])
                firmware.rename(root / 'renamed-OVMF.fd')
                self.assertEqual(module.open_resource_owners(resources, proc), [os.getpid()])
            self.assertEqual(module.open_resource_owners(resources, proc), [])

    def test_fd_guard_unknown_access_or_uid_or_deadline_never_means_no_owner(self):
        module = load_reaper()
        with tempfile.TemporaryDirectory() as root:
            proc = Path(root)
            process = proc / '123'
            process.mkdir()
            (process / 'fd').mkdir()
            (process / 'status').write_text('Uid:\t' + '\t'.join([str(os.getuid())] * 4) + '\n')
            real_iterdir = Path.iterdir

            def denied(path):
                if path == process / 'fd':
                    raise PermissionError('synthetic non-dumpable owner')
                return real_iterdir(path)

            with patch.object(Path, 'iterdir', denied):
                with self.assertRaisesRegex(RuntimeError, 'access is unknown'):
                    module.open_resource_owners({}, proc)
            (process / 'status').write_text('unqualified status')
            with self.assertRaisesRegex(RuntimeError, 'incomplete'):
                module.open_resource_owners({}, proc)
            with self.assertRaises(TimeoutError):
                module.open_resource_owners({}, proc, seconds=0)

    def test_fd_guard_rejects_late_final_read_instead_of_reporting_absence(self):
        module = load_reaper()
        with tempfile.TemporaryDirectory() as root:
            process = Path(root) / '123'
            (process / 'fd').mkdir(parents=True)
            (process / 'status').write_text('Uid:\t' + '\t'.join([str(os.getuid())] * 4) + '\n')
            with patch.object(module.time, 'monotonic', side_effect=[0, 0, 3]):
                with self.assertRaises(TimeoutError):
                    module.open_resource_owners({}, Path(root), seconds=2)

    def test_opt_in_expiry_keeps_owned_pair_but_always_records_expiry_failure(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            module = self.prepare(directory)
            with patch.object(module, 'open_resource_owners', return_value=[]):
                module.expire(directory)
            self.assertTrue((directory / 'disk.qcow2').exists())
            self.assertTrue((directory / 'OVMF_VARS.fd').exists())
            self.assertEqual(json.loads((directory / 'cold-state-expiry.json').read_text())['artifact_retention'], 'verified')
            status = json.loads((directory / 'result.json').read_text())
            self.assertEqual(status['exit_code'], 1)
            self.assertEqual(status['driver'], 'failed')
            self.assertEqual(status['cleanup']['status'], 'complete')
            self.assertEqual(status['cleanup']['disk'], 'retained_cold')

    def test_actual_expire_drains_owned_qemu_before_refusing_retention_including_copied_sibling(self):
        for copied in (False, True):
            for mode in ('malformed', 'wrong-job', 'unsafe-firmware'):
                with self.subTest(copied=copied, mode=mode), tempfile.TemporaryDirectory() as root:
                    directory = Path(root)
                    module = self.prepare(directory)
                    if copied:
                        with patch.object(runner, 'execute'):
                            runner.arm_expiry(directory, 45)
                        self.assertEqual((directory / 'reaper.py').read_bytes(), REAPER.read_bytes())
                        module = load_reaper(directory / 'reaper.py')
                    if mode == 'malformed':
                        (directory / module.COLD_REQUEST).write_text('{')
                    elif mode == 'wrong-job':
                        value = json.loads((directory / module.COLD_REQUEST).read_text())
                        value['inode'] += 1
                        (directory / module.COLD_REQUEST).write_text(json.dumps(value))
                    else:
                        (directory / 'OVMF_VARS.fd').chmod(0o666)
                    primary = {'stage': 'scenario', 'message': 'original retained failure'}
                    (directory / 'result.json').write_text(json.dumps({'primary_failure': primary}))
                    live = [321]
                    def owned_users(disk):
                        self.assertEqual(disk, directory / 'disk.qcow2')
                        return list(live)
                    def stop_owned(pid, disk, signum):
                        self.assertEqual((pid, disk, signum), (321, directory / 'disk.qcow2', signal.SIGTERM))
                        live.clear()
                    with patch.object(module, 'qemu_users', side_effect=owned_users) as users, \
                         patch.object(module, 'signal_owner', side_effect=stop_owned) as stop, \
                         patch.object(module, 'open_resource_owners', return_value=[]):
                        with self.assertRaises((RuntimeError, OSError)):
                            module.expire(directory)
                    self.assertGreaterEqual(users.call_count, 3)
                    stop.assert_called_once()
                    self.assertEqual(live, [])
                    self.assertTrue((directory / 'disk.qcow2').exists())
                    self.assertTrue((directory / 'OVMF_VARS.fd').exists())
                    self.assertFalse((directory / 'cold-state-expiry.json').exists())
                    status = json.loads((directory / 'result.json').read_text())
                    self.assertEqual(status['primary_failure'], primary)
                    self.assertEqual(status['driver'], 'failed')
                    self.assertEqual(status['exit_code'], 1)
                    self.assertEqual(status['cleanup']['status'], 'failed')
                    self.assertEqual(status['cleanup']['disk'], 'retained_unverified')

    def test_pidfd_authority_still_refuses_foreign_uid_and_reused_drive_identity(self):
        module = load_reaper()
        read_fd, write_fd = os.pipe()
        try:
            for foreign in (False, True):
                with self.subTest(foreign=foreign), \
                     patch.object(module.os, 'pidfd_open', side_effect=lambda pid: os.dup(read_fd)), \
                     patch.object(Path, 'stat', return_value=SimpleNamespace(st_uid=os.getuid() + int(foreign))), \
                     patch.object(module, 'qemu_users', return_value=[]) as users, \
                     patch.object(module.signal, 'pidfd_send_signal') as send:
                    if foreign:
                        with self.assertRaisesRegex(RuntimeError, 'foreign QEMU'):
                            module.signal_owner(321, Path('/owned/disk.qcow2'), signal.SIGTERM)
                        users.assert_not_called()
                    else:
                        module.signal_owner(321, Path('/owned/disk.qcow2'), signal.SIGTERM)
                        users.assert_called_once()
                    send.assert_not_called()
        finally:
            os.close(read_fd)
            os.close(write_fd)

    def test_initially_missing_drive_never_grants_signal_authority_in_canonical_or_copied_expire(self):
        for copied in (False, True):
            for malformed in (False, True):
                with self.subTest(copied=copied, malformed=malformed), tempfile.TemporaryDirectory() as root:
                    directory = Path(root)
                    module = self.prepare(directory)
                    if copied:
                        with patch.object(runner, 'execute'):
                            runner.arm_expiry(directory, 45)
                        module = load_reaper(directory / 'reaper.py')
                    (directory / 'disk.qcow2').unlink()
                    request = directory / module.COLD_REQUEST
                    if malformed:
                        request.write_text('{')
                    original = request.read_bytes()
                    with patch.object(module, 'qemu_users', return_value=[321]) as users, \
                         patch.object(module, 'signal_owner') as send:
                        with self.assertRaises((RuntimeError, OSError)):
                            module.expire(directory)
                    users.assert_not_called()
                    send.assert_not_called()
                    self.assertEqual(request.read_bytes(), original)
                    self.assertEqual((directory / 'OVMF_VARS.fd').read_bytes(), b'owned synthetic resource')
                    self.assertFalse((directory / 'cold-state-expiry.json').exists())
                    status = json.loads((directory / 'result.json').read_text())
                    self.assertEqual(status['driver'], 'failed')
                    self.assertEqual(status['exit_code'], 1)
                    self.assertEqual(status['cleanup']['status'], 'failed')

    def test_uncertain_drive_authority_is_not_permission_to_signal(self):
        for mode in ('symlink', 'hardlink', 'foreign'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as root:
                directory = Path(root)
                module = self.prepare(directory)
                disk = directory / 'disk.qcow2'
                outside = directory / 'outside'
                outside.write_bytes(b'untouched synthetic outside')
                if mode in ('symlink', 'hardlink'):
                    disk.unlink()
                    if mode == 'symlink':
                        disk.symlink_to(outside)
                    else:
                        os.link(outside, disk)
                real_lstat = Path.lstat
                def foreign(path, *args, **kwargs):
                    meta = real_lstat(path, *args, **kwargs)
                    return SimpleNamespace(st_mode=meta.st_mode, st_uid=os.getuid()+1, st_nlink=1) if path == disk else meta
                with (patch.object(Path, 'lstat', foreign) if mode == 'foreign' else contextlib.nullcontext()), \
                     patch.object(module, 'qemu_users') as users, patch.object(module, 'signal_owner') as send:
                    with self.assertRaisesRegex(RuntimeError, 'ownership is unknown'):
                        module.expire(directory)
                users.assert_not_called()
                send.assert_not_called()
                self.assertEqual(outside.read_bytes(), b'untouched synthetic outside')
                self.assertFalse((directory / 'cold-state-expiry.json').exists())

    def test_expiry_stops_exact_owned_qemu_before_cold_retention_and_keeps_unverified_state_unqualified(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            module = self.prepare(directory, verified=False)
            with patch.object(module, 'qemu_users', side_effect=[[321], [], [], []]), \
                 patch.object(module, 'signal_owner') as signal_owned, \
                 patch.object(module, 'open_resource_owners', return_value=[]):
                module.reap(directory)
            signal_owned.assert_called_once_with(321, directory / 'disk.qcow2', signal.SIGTERM)
            self.assertEqual(json.loads((directory / 'cold-state-expiry.json').read_text())['artifact_retention'], 'unverified')
            self.assertFalse((directory / 'artifacts-retained').exists())

    def test_unsafe_policy_resource_and_artifact_records_preserve_files_without_positive_receipt(self):
        for mode in ('request-symlink', 'request-wrong-job', 'resource-symlink', 'resource-hardlink', 'resource-writable', 'marker-symlink', 'marker-corrupt'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as root:
                directory = Path(root)
                module = self.prepare(directory)
                outside = directory / 'untouched'
                outside.write_bytes(b'foreign synthetic evidence')
                request = directory / module.COLD_REQUEST
                disk = directory / 'disk.qcow2'
                marker = directory / 'artifacts-retained'
                if mode == 'request-symlink':
                    request.unlink()
                    request.symlink_to(outside)
                elif mode == 'request-wrong-job':
                    request.write_text('{}')
                elif mode == 'resource-symlink':
                    disk.unlink()
                    disk.symlink_to(outside)
                elif mode == 'resource-hardlink':
                    disk.unlink()
                    os.link(outside, disk)
                elif mode == 'resource-writable':
                    disk.chmod(0o666)
                elif mode == 'marker-symlink':
                    marker.unlink()
                    marker.symlink_to(outside)
                elif mode == 'marker-corrupt':
                    marker.write_text('not verified artifact retention')
                with patch.object(module, 'open_resource_owners', return_value=[]):
                    with self.assertRaises((RuntimeError, OSError)):
                        module.reap(directory)
                self.assertEqual(outside.read_bytes(), b'foreign synthetic evidence')
                self.assertTrue(os.path.lexists(disk))
                self.assertTrue((directory / 'OVMF_VARS.fd').exists())
                self.assertFalse((directory / 'cold-state-expiry.json').exists())

    def test_open_fd_owner_and_incomplete_pair_and_foreign_owner_refuse_positive_receipt(self):
        for mode in ('open', 'missing-firmware', 'foreign'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as root:
                directory = Path(root)
                module = self.prepare(directory)
                if mode == 'missing-firmware':
                    (directory / 'OVMF_VARS.fd').unlink()
                real_lstat = Path.lstat

                def foreign(path, *args, **kwargs):
                    meta = real_lstat(path, *args, **kwargs)
                    if path == directory / 'disk.qcow2':
                        return SimpleNamespace(st_mode=meta.st_mode, st_uid=os.getuid() + 1, st_nlink=1)
                    return meta

                with patch.object(module, 'open_resource_owners', return_value=[123] if mode == 'open' else []), \
                     (patch.object(Path, 'lstat', foreign) if mode == 'foreign' else contextlib.nullcontext()):
                    with self.assertRaises((RuntimeError, OSError)):
                        module.retain_cold_resources(directory, 'runner')
                self.assertTrue((directory / 'disk.qcow2').exists())
                self.assertFalse((directory / 'cold-state-runner.json').exists())


class RunnerFailureTests(unittest.TestCase):
    """Real owned child/streams and disk cleanup; VM/SSH/systemd are inert adapters."""

    def exercise(self, root, scenario_exit: int | str = 7, transfer_exit=0, cleanup_exit=0, corrupt=False, delayed_writer=False,
                 retain_cold=False, resource_owners=()):
        root = Path(root)
        binary = root / 'candidate'
        binary.write_bytes(b'synthetic candidate')
        for name in ('vars', 'key', 'template', 'scenario'):
            (root / name).write_text('synthetic input')
        guest = root / 'guest'
        guest.mkdir()
        (guest / 'evidence.txt').write_text('only evidence')
        # The transport accepts only these synthetic operations. It never invokes
        # an SSH client, guest, product binary, systemd or the shell command string.
        transport = root / 'transport.py'
        transport.write_text('''import hashlib, pathlib, shutil, subprocess, sys
root = pathlib.Path(sys.argv[1])
operation = sys.argv[2]
if operation == 'ssh':
    command = sys.argv[-1]
    if command.startswith('sha256sum '):
        print(hashlib.sha256((root / 'candidate').read_bytes()).hexdigest())
    elif command.startswith('python3 -c '):
        import shlex
        script = shlex.split(command)[2]
        subprocess.run([sys.executable, '-c', script, str(root / 'guest')], check=True)
    elif ' bash ' in command:
        print('scenario output', flush=True)
        sys.stderr.write('first scenario diagnostic\\n')
        sys.stderr.flush()
        if sys.argv[3] == 'timeout':
            import time
            time.sleep(30)
        sys.exit(int(sys.argv[3]))
elif operation == 'scp' and '-r' in sys.argv:
    if int(sys.argv[3]):
        sys.stderr.write('transfer diagnostic\\n')
        sys.exit(int(sys.argv[3]))
    shutil.copytree(root / 'guest', sys.argv[-1])
    if sys.argv[4] == 'corrupt':
        (pathlib.Path(sys.argv[-1]) / 'evidence.txt').write_text('wrong')
elif operation == 'cleanup':
    sys.stderr.write('cleanup diagnostic\\n')
    sys.exit(int(sys.argv[3]))
''')
        real_execute = runner.execute

        def transport_execute(command, **kwargs):
            if command[0] == 'scp':
                command = [sys.executable, str(transport), str(root), 'scp', str(transfer_exit),
                           'corrupt' if corrupt else 'normal', *command[1:]]
            elif command[0] == 'systemctl':
                command = [sys.executable, str(transport), str(root), 'cleanup', str(cleanup_exit)]
            result = real_execute(command, **kwargs)
            if writer is not None and command[:3] == [sys.executable, '-c', runner.MANIFEST_SCRIPT]:
                # Both manifests now match, but the guest is still live. Let
                # its writer produce evidence before main can shut it down.
                (root / 'release-writer').touch()
                writer.wait(timeout=5)
            return result

        def overlay(_template, destination):
            destination.write_bytes(b'owned synthetic disk')

        results = root / 'r'
        options = ['gui-vm.py', '--template', str(root / 'template'), '--ovmf-vars', str(root / 'vars'),
                   '--ssh-key', str(root / 'key'), '--ssh-user', 'fixture', '--scenario', str(root / 'scenario'),
                   '--binary', str(binary), '--results', str(results), '--scenario-timeout', '1']
        if retain_cold:
            options.append('--retain-cold-state')
        owned_processes = []
        real_popen = runner.subprocess.Popen

        def track_process(*args, **kwargs):
            process = real_popen(*args, **kwargs)
            owned_processes.append(process)
            return process

        # An independent remote-writer analogue survives the local transport.
        # It waits until the copied snapshot has already been hashed.
        writer = None
        if delayed_writer:
            writer = real_popen([sys.executable, '-c', '''import pathlib, sys, time
root = pathlib.Path(sys.argv[1])
while not (root / 'release-writer').exists(): time.sleep(.01)
(root / 'guest/late.txt').write_text('late primary evidence')
''', str(root)])
        try:
            with patch.object(sys, 'argv', options), \
                 patch.object(runner, 'make_overlay', side_effect=overlay), \
                 patch.object(runner, 'qemu_command', return_value=[sys.executable, '-c', 'import time; time.sleep(30)']), \
                 patch.object(runner, 'arm_expiry', return_value='owned-fixture-expiry'), \
                 patch.object(runner, 'wait_for_desktop'), \
                 patch.object(runner, 'ssh_prefix', return_value=[sys.executable, str(transport), str(root), 'ssh', str(scenario_exit)]), \
                 patch.object(runner, 'execute', side_effect=transport_execute), \
                 patch.object(runner.cold_retention, 'open_resource_owners', return_value=list(resource_owners)), \
                 patch.object(runner.subprocess, 'Popen', side_effect=track_process), contextlib.redirect_stdout(io.StringIO()):
                code = runner.main()
        finally:
            if writer is not None:
                (root / 'release-writer').touch()
                try:
                    writer.wait(timeout=5)
                finally:
                    if writer.poll() is None:
                        writer.kill()
                        writer.wait(timeout=5)
        self.assertTrue(owned_processes)
        self.assertTrue(all(process.poll() is not None for process in owned_processes))
        if writer is not None:
            self.assertEqual(writer.returncode, 0)
        status = json.loads((results / 'result.json').read_text())
        self.assertEqual((results / 'result.json').stat().st_mode & 0o777, 0o600)
        self.assertEqual(status['candidate_sha256'], hashlib.sha256(binary.read_bytes()).hexdigest())
        return code, status, results

    def test_first_real_scenario_failure_survives_transfer_and_cleanup_failures(self):
        with tempfile.TemporaryDirectory() as root:
            code, status, results = self.exercise(root, transfer_exit=9, cleanup_exit=11)
            self.assertEqual(code, 7)
            primary = status['primary_failure']
            self.assertEqual(primary['stage'], 'scenario')
            self.assertEqual(primary['returncode'], 7)
            self.assertIn(' bash ', primary['command'][-1])
            self.assertEqual(primary['stdout'], 'scenario output\n')
            self.assertEqual(primary['stderr'], 'first scenario diagnostic\n')
            self.assertEqual(primary['classification'], 'driver/unclassified')
            self.assertEqual(status['product'], 'unknown')
            self.assertEqual(status['scenario_exit'], 7)
            self.assertEqual(status['cleanup']['status'], 'failed')
            self.assertIn('cleanup diagnostic', status['cleanup']['errors'][0]['stderr'])
            self.assertTrue((results / 'disk.qcow2').exists())
            self.assertEqual(status['artifact_retention'], 'unverified')
            self.assertEqual(status['secondary_failures'][0]['returncode'], 9)

    def test_transfer_failure_retains_cold_disk_and_exact_stderr(self):
        with tempfile.TemporaryDirectory() as root:
            code, status, results = self.exercise(root, scenario_exit=0, transfer_exit=9)
            self.assertNotEqual(code, 0)
            self.assertEqual(status['primary_failure']['stage'], 'retention')
            self.assertEqual(status['primary_failure']['returncode'], 9)
            self.assertEqual(status['primary_failure']['stderr'], 'transfer diagnostic\n')
            self.assertTrue((results / 'disk.qcow2').exists())
            self.assertEqual(status['cleanup']['disk'], 'retained_cold')

    def test_cleanup_failure_makes_successful_scenario_nonzero(self):
        with tempfile.TemporaryDirectory() as root:
            code, status, _ = self.exercise(root, scenario_exit=0, cleanup_exit=11)
            self.assertNotEqual(code, 0)
            self.assertIsNone(status['primary_failure'])
            self.assertEqual(status['cleanup']['errors'][0]['returncode'], 11)

    def test_verified_artifact_transfer_allows_owned_disk_deletion(self):
        with tempfile.TemporaryDirectory() as root:
            code, status, results = self.exercise(root, scenario_exit=0)
            self.assertEqual(code, 0)
            self.assertEqual(status['artifact_retention'], 'verified')
            self.assertEqual((results / 'artifacts/evidence.txt').read_text(), 'only evidence')
            self.assertFalse((results / 'disk.qcow2').exists())
            self.assertEqual(status['cleanup']['status'], 'complete')

    def test_successful_but_corrupt_transfer_does_not_delete_disk(self):
        with tempfile.TemporaryDirectory() as root:
            code, status, results = self.exercise(root, scenario_exit=0, corrupt=True)
            self.assertNotEqual(code, 0)
            self.assertIn('manifest mismatch', status['primary_failure']['message'])
            self.assertTrue((results / 'disk.qcow2').exists())

    def test_opt_in_success_preserves_whole_cold_disk_and_final_firmware_without_live_resources(self):
        with tempfile.TemporaryDirectory() as root:
            code, status, results = self.exercise(root, scenario_exit=0, retain_cold=True)
            self.assertEqual(code, 0)
            self.assertEqual(status['artifact_retention'], 'verified')
            self.assertEqual(status['cleanup']['status'], 'complete')
            self.assertEqual(status['cleanup']['expiry'], 'stopped')
            self.assertEqual(status['cleanup']['disk'], 'retained_cold')
            self.assertEqual((results / 'disk.qcow2').read_bytes(), b'owned synthetic disk')
            self.assertEqual((results / 'OVMF_VARS.fd').read_text(), 'synthetic input')
            receipt = json.loads((results / 'cold-state-runner.json').read_text())
            self.assertEqual(receipt['artifact_retention'], 'verified')
            self.assertEqual(receipt['owners'], [])
            self.assertEqual(receipt['resources']['OVMF_VARS.fd']['ino'], (results / 'OVMF_VARS.fd').stat().st_ino)
            reaper = load_reaper()
            with patch.object(reaper, 'open_resource_owners', return_value=[]):
                reaper.reap(results)
            self.assertTrue((results / 'disk.qcow2').exists())
            self.assertTrue((results / 'OVMF_VARS.fd').exists())
            self.assertEqual(json.loads((results / 'cold-state-expiry.json').read_text())['artifact_retention'], 'verified')

    def test_opt_in_failure_keeps_unqualified_cold_state_not_fake_success(self):
        with tempfile.TemporaryDirectory() as root:
            code, status, results = self.exercise(root, retain_cold=True)
            self.assertNotEqual(code, 0)
            self.assertEqual(status['scenario_exit'], 7)
            receipt = json.loads((results / 'cold-state-runner.json').read_text())
            self.assertEqual(receipt['artifact_retention'], 'unverified')
            self.assertFalse((results / 'artifacts-retained').exists())
            self.assertTrue((results / 'OVMF_VARS.fd').exists())

    def test_opt_in_open_resource_owner_refuses_positive_cold_receipt(self):
        with tempfile.TemporaryDirectory() as root:
            code, status, results = self.exercise(root, scenario_exit=0, retain_cold=True, resource_owners=[123])
            self.assertNotEqual(code, 0)
            self.assertEqual(status['cleanup']['status'], 'failed')
            self.assertEqual(status['cleanup']['disk'], 'retained_unverified')
            self.assertFalse((results / 'cold-state-runner.json').exists())
            self.assertTrue((results / 'disk.qcow2').exists())
            self.assertTrue((results / 'OVMF_VARS.fd').exists())

    def test_failed_or_unknown_remote_exit_keeps_disk_despite_matching_partial_snapshot(self):
        for remote_exit in ('timeout', 255, 7):
            with self.subTest(remote_exit=remote_exit), tempfile.TemporaryDirectory() as root:
                code, status, results = self.exercise(root, scenario_exit=remote_exit, delayed_writer=True)
                self.assertNotEqual(code, 0)
                self.assertEqual(status['primary_failure']['stage'], 'scenario')
                self.assertEqual(status['primary_failure']['stdout'], 'scenario output\n')
                self.assertEqual(status['primary_failure']['stderr'], 'first scenario diagnostic\n')
                self.assertEqual((results / 'artifacts/evidence.txt').read_text(), 'only evidence')
                self.assertEqual((Path(root) / 'guest/late.txt').read_text(), 'late primary evidence')
                self.assertFalse((results / 'artifacts/late.txt').exists())
                self.assertFalse((results / 'artifacts-retained').exists())
                self.assertEqual(status['artifact_retention'], 'partial_snapshot')
                self.assertTrue((results / 'disk.qcow2').exists())
                self.assertEqual(status['cleanup']['disk'], 'retained_cold')
                reaper = load_reaper()
                reaper.expire(results)
                self.assertTrue((results / 'disk.qcow2').exists())

    def test_preflight_failure_writes_receipt_without_allocating_vm(self):
        with tempfile.TemporaryDirectory() as root:
            results = Path(root) / ('x' * 110)
            options = [item for name in ('template', 'ovmf-vars', 'ssh-key', 'scenario', 'binary')
                       for item in ('--' + name, 'missing')]
            process = subprocess.run([sys.executable, str(MODULE), *options, '--ssh-user', 'fixture',
                                      '--results', str(results)], text=True, capture_output=True, timeout=5)
            self.assertNotEqual(process.returncode, 0)
            status = json.loads((results / 'result.json').read_text())
            self.assertIn('socket path', status['primary_failure']['message'])
            self.assertEqual(status['primary_failure']['stage'], 'preflight')
            self.assertEqual(status['product'], 'unrun')
            self.assertFalse((results / 'disk.qcow2').exists())

    def test_expiry_preserves_unverified_disk_and_primary_receipt(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            (root / 'disk.qcow2').write_bytes(b'only evidence')
            primary = {'message': 'original failure'}
            (root / 'result.json').write_text(json.dumps({'primary_failure': primary}))
            process = subprocess.run([sys.executable, str(REAPER), str(root)], capture_output=True, timeout=5)
            self.assertEqual(process.returncode, 0, process.stderr)
            self.assertTrue((root / 'disk.qcow2').exists())
            status = json.loads((root / 'result.json').read_text())
            self.assertEqual(status['primary_failure'], primary)
            self.assertEqual(status['cleanup']['disk'], 'retained_cold')

    def test_real_expiry_signals_only_exact_owned_disk_process(self):
        reaper = load_reaper()
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            disk = root / 'disk.qcow2'
            disk.write_bytes(b'only evidence')
            processes = []
            try:
                for path in (disk, root / 'disk.qcow2-other'):
                    # Python, not QEMU: exec argv supplies an inert QEMU-shaped
                    # drive owner to the real /proc scanner and pidfd signal path.
                    processes.append(subprocess.Popen(['qemu-system-fixture', '-c', 'import time; time.sleep(30)',
                                                       '-drive', 'file=' + str(path) + ',format=qcow2'],
                                                      executable=sys.executable))
                self.assertEqual(list(reaper.qemu_users(disk)), [processes[0].pid])
                with self.assertRaisesRegex(RuntimeError, 'in use'):
                    runner.make_overlay(disk, root / 'forbidden-overlay.qcow2')
                self.assertFalse((root / 'forbidden-overlay.qcow2').exists())
                process = subprocess.run([sys.executable, str(REAPER), str(root)], capture_output=True, timeout=5)
                self.assertEqual(process.returncode, 0, process.stderr)
                self.assertEqual(processes[0].wait(timeout=2), -signal.SIGTERM)
                self.assertIsNone(processes[1].poll())
                self.assertTrue(disk.exists())
            finally:
                for process in processes:
                    if process.poll() is None:
                        process.terminate()
                    process.wait(timeout=5)

    def test_expiry_returns_nonzero_for_unresolved_prior_cleanup(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            primary = {'stage': 'scenario', 'message': 'original'}
            (root / 'result.json').write_text(json.dumps({'primary_failure': primary,
                                                        'cleanup': {'errors': [{'message': 'original cleanup failure'}]}}))
            process = subprocess.run([sys.executable, str(REAPER), str(root)], capture_output=True, timeout=5)
            self.assertNotEqual(process.returncode, 0)
            status = json.loads((root / 'result.json').read_text())
            self.assertEqual(status['primary_failure'], primary)
            self.assertEqual(status['cleanup']['status'], 'failed')


    def test_real_reaper_rejects_symlink_and_public_job_without_mutation(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            job = root / 'job'
            job.mkdir(mode=0o700)
            disk = job / 'disk.qcow2'
            disk.write_bytes(b'only evidence')
            alias = root / 'alias'
            alias.symlink_to(job, target_is_directory=True)
            for target in (alias, job):
                if target == job:
                    job.chmod(0o755)
                process = subprocess.run([sys.executable, str(REAPER), str(target)], capture_output=True, timeout=5)
                self.assertNotEqual(process.returncode, 0)
                self.assertEqual(disk.read_bytes(), b'only evidence')
                self.assertFalse((job / 'result.json').exists())

    def test_stale_writer_cannot_overwrite_primary_failure_after_expiry(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            primary = {'stage': 'scenario', 'message': 'first failure'}
            runner.write_result(root, {'version': 1, 'primary_failure': primary})
            reaper = load_reaper()
            with patch.object(reaper, 'reap', side_effect=RuntimeError('injected reap failure')):
                with self.assertRaisesRegex(RuntimeError, 'injected reap failure'):
                    reaper.expire(root)
            expired = json.loads((root / 'result.json').read_text())
            stale = {'primary_failure': None, 'driver': 'passed', 'exit_code': 0,
                     'stage': 'finished', 'progress': ['cleanup'],
                     'secondary_failures': [{'message': 'late runner diagnostic'}],
                     'cleanup': {'status': 'complete', 'disk': 'removed', 'expiry': 'stopped',
                                 'errors': [{'message': 'late cleanup diagnostic'}]}}
            runner.write_result(root, stale)
            status = json.loads((root / 'result.json').read_text())
            self.assertEqual(status['primary_failure'], primary)
            self.assertEqual(status['driver'], 'failed')
            self.assertNotEqual(status['exit_code'], 0)
            self.assertEqual(status['stage'], 'expired')
            self.assertIn('expired', status['progress'])
            self.assertEqual(status['cleanup']['expiry'], 'fired')
            self.assertEqual(status['cleanup']['status'], 'failed')
            self.assertEqual(status['cleanup']['disk'], expired['cleanup']['disk'])
            self.assertIn({'stage': 'expiry', 'message': 'injected reap failure'}, status['cleanup']['errors'])
            self.assertIn({'message': 'late cleanup diagnostic'}, status['cleanup']['errors'])
            self.assertIn({'message': 'late runner diagnostic'}, status['secondary_failures'])


if __name__ == "__main__":
    unittest.main()
