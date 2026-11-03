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

def load_reaper():
    spec = importlib.util.spec_from_file_location("gui_vm_reaper", REAPER)
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


class RunnerFailureTests(unittest.TestCase):
    """Real owned child/streams and disk cleanup; VM/SSH/systemd are inert adapters."""

    def exercise(self, root, scenario_exit: int | str = 7, transfer_exit=0, cleanup_exit=0, corrupt=False, delayed_writer=False):
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
