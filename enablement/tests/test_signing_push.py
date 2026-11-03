"""Real Git transactions exercise signing independently of author identity."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class SigningHookTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='signing-hook-')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        self.env = {k: v for k, v in os.environ.items() if not k.startswith('GIT_')}
        home = self.root / 'home'
        home.mkdir()
        self.env.update(HOME=str(home), GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull)
        self.git('init', '-q')
        self.git('config', 'user.name', 'Unrestricted fixture')
        self.git('config', 'user.email', 'fixture@example.invalid')
        self.key = self.root / 'key'
        subprocess.run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(self.key)],
                       env=self.env, check=True, capture_output=True, timeout=15)
        self.key.chmod(0o600)
        self.allowed = self.root / 'allowed_signers'
        self.allowed.write_text('fixture@example.invalid ' + self.key.with_suffix('.pub').read_text())
        self.git('config', 'gpg.format', 'ssh')
        self.git('config', 'user.signingkey', str(self.key))
        self.git('config', 'gpg.ssh.allowedSignersFile', str(self.allowed))
        self.git('config', 'commit.gpgsign', 'true')
        hooks = self.repo / '.githooks'
        hooks.mkdir()
        shutil.copy2(ROOT / '.githooks/reference-transaction', hooks / 'reference-transaction')
        self.git('config', 'core.hooksPath', str(hooks))

    def git(self, *args, check=True):
        return subprocess.run(['git', '-C', str(self.repo), *args], env=self.env,
                              check=check, capture_output=True, text=True, timeout=15)

    def commit(self, *args, check=True):
        return self.git('commit', '--allow-empty', '-qm', 'fixture', *args, check=check)

    def head(self):
        return self.git('rev-parse', '--verify', 'HEAD', check=False).stdout.strip()

    def test_unsigned_initial_commit_rejected(self):
        result = self.commit('--no-gpg-sign', check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('a verifiable signature is required', result.stderr)
        self.assertFalse(self.head())

    def test_signed_commit_with_different_author_and_committer(self):
        self.commit('--author=Another Author <another@example.invalid>')
        self.assertIn('Another Author', self.git('show', '-s', '--format=%an').stdout)
        self.git('verify-commit', 'HEAD')
        self.git('config', 'user.name', 'Different Committer')
        self.git('config', 'user.email', 'different@example.invalid')
        self.commit()
        self.git('verify-commit', 'HEAD')

    def test_unsigned_override_and_no_verify_cannot_advance_head(self):
        self.commit()
        head = self.head()
        for args in [('--no-gpg-sign',), ('--no-gpg-sign', '--no-verify')]:
            with self.subTest(args=args):
                result = self.commit(*args, check=False)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.head(), head)
        result = self.git('-c', 'commit.gpgsign=false', 'commit', '--allow-empty', '-qm',
                          'unsigned config override', check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.head(), head)

    def test_unsigned_amend_and_detached_commit_rejected(self):
        self.commit()
        head = self.head()
        result = self.commit('--amend', '--no-gpg-sign', check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.head(), head)
        self.git('checkout', '--detach')
        result = self.commit('--no-gpg-sign', check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.head(), head)

    def test_untrusted_signature_rejected(self):
        self.allowed.write_text('')
        result = self.commit(check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.head())

    def test_batch_checks_unsigned_parent_not_just_signed_tip(self):
        self.commit()
        head = self.head()
        tree = self.git('rev-parse', 'HEAD^{tree}').stdout.strip()
        unsigned = self.git('commit-tree', tree, '-p', head, '-m', 'unsigned').stdout.strip()
        signed = self.git('commit-tree', '-S', tree, '-p', unsigned, '-m', 'signed').stdout.strip()
        result = self.git('update-ref', 'HEAD', signed, head, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.head(), head)

    def test_symbolic_head_transaction_does_not_treat_ref_as_commit(self):
        self.commit()
        result = subprocess.run(
            [str(self.repo / '.githooks/reference-transaction'), 'prepared'],
            input='ref:refs/heads/main ref:refs/heads/side HEAD\n',
            cwd=self.repo, env=self.env, capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_symbolic_head_transaction_still_checks_new_branch_commit(self):
        self.commit()
        head = self.head()
        tree = self.git('rev-parse', 'HEAD^{tree}').stdout.strip()
        unsigned = self.git('commit-tree', tree, '-p', head, '-m', 'unsigned').stdout.strip()
        result = subprocess.run(
            [str(self.repo / '.githooks/reference-transaction'), 'prepared'],
            input=('ref:refs/heads/main ref:refs/heads/side HEAD\n'
                   f'{head} {unsigned} refs/heads/side\n'),
            cwd=self.repo, env=self.env, capture_output=True, text=True, timeout=15)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('a verifiable signature is required', result.stderr)
        self.assertEqual(self.head(), head)

    def test_existing_unsigned_history_and_remote_updates_unchanged(self):
        self.git('-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgsign=false',
                 'commit', '--allow-empty', '-qm', 'old unsigned history')
        old = self.head()
        self.commit()
        signed = self.head()
        self.git('branch', 'old-history', old)
        self.git('checkout', '--detach', old)
        self.commit()
        self.git('checkout', '-')
        self.git('update-ref', 'refs/remotes/origin/main', old)
        self.git('tag', 'old-history', old)
        self.git('branch', '-D', 'old-history')
        self.git('reset', '--hard', signed)

    def test_unsigned_merge_rejected_and_signed_merge_allowed(self):
        self.commit()
        self.git('checkout', '-b', 'side')
        self.commit()
        self.git('checkout', '-')
        head = self.head()
        result = self.git('merge', '--no-ff', '--no-gpg-sign', '-m', 'unsigned merge',
                          'side', check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.head(), head)
        self.git('merge', '--abort')
        self.git('merge', '--no-ff', '-S', '-m', 'signed merge', 'side')
        self.git('verify-commit', 'HEAD')


if __name__ == '__main__':
    unittest.main()
