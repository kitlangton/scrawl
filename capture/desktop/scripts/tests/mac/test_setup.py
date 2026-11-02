import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[2] / 'mac/doctor.py'
spec = importlib.util.spec_from_file_location('mac_doctor', SCRIPT)
mac = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mac)


class SetupTests(unittest.TestCase):
    def test_linux_refuses_install_without_invocations(self):
        with patch.object(mac.sys, 'platform', 'linux'), patch.object(mac.subprocess, 'run') as run:
            with self.assertRaisesRegex(RuntimeError, 'Darwin'):
                mac.provision({'PATH': ''}, '22')
            run.assert_not_called()

    def test_cli_requires_disposable_before_any_work(self):
        result = subprocess.run([sys.executable, str(SCRIPT), 'provision'], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn('require --disposable', result.stderr)

    def test_doctor_read_only_probes_and_wrong_node(self):
        calls = []
        def probe(argv, env):
            calls.append(argv)
            return 'v24.0.0' if argv[0] == 'node' else 'fixture version'
        with patch.object(mac.sys, 'platform', 'darwin'), patch.object(mac, 'check_temp_environment') as temp, patch.object(mac, 'run', side_effect=probe):
            with self.assertRaisesRegex(RuntimeError, 'Expected Desktop CI Node 22'):
                mac.doctor({'TMPDIR': '/fixture'}, '22')
        temp.assert_called_once()
        self.assertIn(['rustfmt', '--version'], calls)
        self.assertIn(['ffprobe', '-version'], calls)
        self.assertIn(['xcrun', '--sdk', 'macosx', 'metal', '--version'], calls)
        self.assertFalse(any('install' in c or '-downloadComponent' in c for c in calls))

    def test_bad_temp_prevents_all_tools(self):
        with patch.object(mac.sys, 'platform', 'darwin'), patch.object(mac, 'check_temp_environment', side_effect=RuntimeError('unsafe temp')), patch.object(mac, 'run') as run:
            with self.assertRaisesRegex(RuntimeError, 'unsafe temp'):
                mac.doctor({'TMPDIR': '/fixture'}, '22')
            run.assert_not_called()

    def test_failed_tool_preserves_diagnostic(self):
        with self.assertRaisesRegex(RuntimeError, 'missing Metal fixture'):
            mac.run([sys.executable, '-c', 'import sys; sys.stderr.write("missing Metal fixture"); sys.exit(3)'], os.environ.copy())

    def test_run_environment_arguments_and_exit(self):
        with tempfile.TemporaryDirectory() as root:
            env = dict(os.environ, FIXTURE='kept')
            observed = []
            real_run = subprocess.run
            def child(argv, **kwargs):
                observed.append(kwargs['env']['TMPDIR'])
                return real_run(argv, **kwargs)
            with patch.object(mac.sys, 'platform', 'darwin'), patch.object(mac.sys, 'argv', ['doctor.py', '--disposable', 'run', '--', sys.executable, '-c', 'import os,sys; assert os.environ["FIXTURE"] == "kept"; assert os.path.isdir(os.environ["TMPDIR"]); assert sys.argv[1] == "two words"; sys.exit(7)', 'two words']), patch.object(mac, 'environment', return_value=(env, '22')), patch.object(mac, 'check_temp_environment'), patch.object(mac, 'doctor'), patch.object(mac.tempfile, 'TemporaryDirectory', side_effect=lambda **kw: tempfile_original(dir=root)), patch.object(mac.subprocess, 'run', side_effect=child):
                self.assertEqual(mac.main(), 7)
            self.assertFalse(Path(observed[0]).exists())

    def test_environment_matches_repo_and_preserves_values(self):
        with patch.dict(os.environ, {'FIXTURE': 'unchanged'}):
            env, major = mac.environment()
        self.assertEqual(major, '22')
        self.assertEqual(env['FIXTURE'], 'unchanged')
        self.assertIn('/.cargo/bin', env['PATH'])
        self.assertIn('/opt/node@22/bin', env['PATH'])
        self.assertIn('/opt/rustup/bin', env['PATH'])
        self.assertEqual(env['TOOLCHAINS'], os.environ.get('TOOLCHAINS', 'MetalToolchain'))

    def test_missing_rustup_is_installed_before_toolchain(self):
        calls = []
        def which(tool, **kwargs):
            return None if tool == 'rustup' else '/fixture/' + tool
        def probe(argv, env, **kwargs):
            calls.append((argv, kwargs))
            return 'fixture version'
        with patch.object(mac.sys, 'platform', 'darwin'), patch.object(mac.shutil, 'which', side_effect=which), patch.object(mac, 'run', side_effect=probe):
            mac.provision({'PATH': '/fixture'}, '22')
        self.assertEqual(calls[0], (['brew', 'install', 'node@22', 'ffmpeg', 'rustup'], {'timeout': 600}))
        self.assertEqual(calls[1][0][:3], ['rustup', 'toolchain', 'install'])
        self.assertFalse(any('-downloadComponent' in argv for argv, _ in calls))

    def test_missing_metal_download_has_installation_deadline(self):
        calls = []
        def probe(argv, env, **kwargs):
            calls.append((argv, kwargs))
            if argv[0] == 'xcrun':
                raise RuntimeError('component missing')
            return 'fixture version'
        with patch.object(mac.sys, 'platform', 'darwin'), patch.object(mac.shutil, 'which', return_value='/fixture/tool'), patch.object(mac, 'run', side_effect=probe):
            mac.provision({'PATH': '/fixture'}, '22')
        self.assertEqual(calls[-1], (['xcodebuild', '-downloadComponent', 'MetalToolchain'], {'timeout': 600}))


tempfile_original = tempfile.TemporaryDirectory
if __name__ == '__main__':
    unittest.main()
