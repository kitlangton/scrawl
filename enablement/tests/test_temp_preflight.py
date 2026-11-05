"""Synthetic, side-effect-free environment rejection before any build."""
import os
from pathlib import Path
import tempfile
import socket
import sys
import unittest
from unittest.mock import patch
from test_verification import verification


class TempPreflightTests(unittest.TestCase):
    def setUp(self):
        # Trusted home ancestry, never chmod an existing user directory.
        self.tmp = tempfile.TemporaryDirectory(prefix='.pf-', dir=Path.home())
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def check(self, root):
        with patch.dict(os.environ, TMPDIR=str(root)):
            verification.check_temp_environment()

    def test_valid_and_unchanged(self):
        self.check(self.root)
        self.assertEqual(list(self.root.iterdir()), [])
        self.assertEqual(self.root.stat().st_mode & 0o777, 0o700)

    def test_long_path_rejected_without_side_effects(self):
        child = self.root / ('a' * 90)
        child.mkdir(mode=0o700)
        with self.assertRaisesRegex(RuntimeError, 'socket fixture'):
            self.check(child)
        self.assertEqual(list(child.iterdir()), [])

    def test_permissions_and_sticky_parent_rejected(self):
        child = self.root / 'p'
        child.mkdir(mode=0o700)
        for mode in (0o777, 0o1777, 0o770):
            self.root.chmod(mode)
            with self.assertRaisesRegex(RuntimeError, 'writable permissions'):
                self.check(child)
            self.assertEqual(self.root.stat().st_mode & 0o7777, mode)
        self.root.chmod(0o700)

    def test_symlink_rejected_without_touching_target(self):
        link = self.root / 'link'
        target = self.root / 'target'
        target.mkdir(mode=0o700)
        link.symlink_to(target)
        with self.assertRaisesRegex(RuntimeError, 'symlink'):
            self.check(link)
        self.assertTrue(link.is_symlink())
        self.assertEqual(list(target.iterdir()), [])

    def test_foreign_owner_rejected_without_mutation(self):
        real = os.fstat
        def foreign(fd):
            info = real(fd)
            values = list(info)
            values[4] = os.geteuid() + 10000
            return os.stat_result(values)
        with patch.object(verification.os, 'fstat', side_effect=foreign):
            with self.assertRaisesRegex(RuntimeError, 'ownership'):
                self.check(self.root)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_exact_socket_boundary_and_unicode_bytes(self):
        suffix = ('/ac-' + '0' * 32 + '/s' if sys.platform == 'darwin'
                  else '/apparent-owner-' + '0' * 32 + '/capture.sock')
        limit = 103 if sys.platform == 'darwin' else 107
        padding = limit - len(os.fsencode(str(self.root) + '/' + suffix))
        self.assertGreaterEqual(padding, 2)
        child = self.root / ('é' + 'a' * (padding - 2))
        child.mkdir(mode=0o700)
        self.check(child)
        address = str(child) + suffix
        Path(address).parent.mkdir(mode=0o700)
        with socket.socket(socket.AF_UNIX) as listener:
            listener.bind(address)
        longer = self.root / (child.name + 'a')
        longer.mkdir(mode=0o700)
        with self.assertRaisesRegex(RuntimeError, 'socket fixture'):
            self.check(longer)
        self.assertEqual(list(longer.iterdir()), [])

    def test_pr_contracts_include_preflight_tests_without_environment_admission(self):
        with patch.dict(os.environ, TMPDIR='/missing-preflight-fixture'), patch.object(verification.subprocess, 'run') as run, patch.object(
                verification, 'run_public_site_build') as public_build, patch.object(
                verification, 'run_connect_client_typescript_tests') as client_tests, patch.object(
                verification, 'run_app_shell_check') as app_check:
            verification.run_pr_checks()
        public_build.assert_called_once_with()  # Unknown history includes Public feedback.
        client_tests.assert_called_once_with()  # Keep owner checks selected without live dependency setup.
        patterns = [call.args[0][-1] for call in run.call_args_list]
        self.assertEqual(patterns, ['enablement/file-length/check.test.ts', 'shared/ui/scripts/check-theme.py', 'composition/delivery/core.test.ts', 'test_*.py',
                                    'test_*push*.py', 'test_verification.py', 'test_release_gate.py',
                                    'test_temp_preflight.py', 'test_owner_lanes.py'])
        app_check.assert_called_once_with()
        self.assertTrue(all(call.kwargs['check'] for call in run.call_args_list))

    def test_rejection_precedes_all_subprocesses(self):
        with patch.dict(os.environ, TMPDIR='/missing-preflight-fixture'), patch.object(verification.subprocess, 'run') as run:
            with self.assertRaisesRegex(RuntimeError, 'TMPDIR preflight'):
                verification.run_checks({'root': True, 'engine': True})
            run.assert_not_called()
