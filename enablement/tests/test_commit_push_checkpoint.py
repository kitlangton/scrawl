"""Run the installed post-commit hook against isolated real Git repositories."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

HOOK = Path(__file__).resolve().parents[2] / '.githooks/post-commit'


class CommitCheckpointTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.env = {k: v for k, v in os.environ.items() if not k.startswith('GIT_')}
        self.env['GIT_CONFIG_NOSYSTEM'] = '1'
        self.env['GIT_CONFIG_GLOBAL'] = '/dev/null'
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        self.git('init', '-q', '-b', 'main')
        self.git('config', 'user.name', 'Test')
        self.git('config', 'user.email', 'test@example.invalid')
        self.git('config', 'core.hooksPath', '/dev/null')
        (self.repo / 'file').write_text('base\n')
        self.git('add', '.')
        self.git('commit', '-qm', 'base')
        self.main = self.git('rev-parse', 'HEAD').stdout.strip()
        self.remote = self.root / 'remote.git'
        self.git('clone', '-q', '--bare', str(self.repo), str(self.remote))
        self.git('remote', 'add', 'origin', str(self.remote))
        self.git('switch', '-qc', 'feature')
        self.hooks = self.root / 'hooks'
        self.hooks.mkdir()
        shutil.copy2(HOOK, self.hooks / 'post-commit')
        self.git('config', 'core.hooksPath', str(self.hooks))

    def git(self, *args, cwd=None, check=True):
        return subprocess.run(['git', *args], cwd=cwd or self.repo, env=self.env,
                              text=True, capture_output=True, check=check)

    def advance_main(self):
        # Create an independent main commit without touching the feature checkout.
        tree = self.git('rev-parse', 'HEAD^{tree}').stdout.strip()
        result = subprocess.run(['git', 'commit-tree', tree, '-p', self.main, '-m', 'incoming'],
                                cwd=self.repo, env=self.env, text=True, capture_output=True, check=True)
        self.incoming = result.stdout.strip()
        self.git('push', '-q', 'origin', f'{self.incoming}:refs/heads/main')

    def commit(self, cwd=None):
        return self.git('commit', '--allow-empty', '-qm', 'checkpoint', cwd=cwd)

    def test_current_main_is_silent(self):
        self.assertEqual(self.commit().stderr, '')

    def test_commit_fetches_new_main_without_rewriting_head(self):
        self.advance_main()
        result = self.commit()
        self.assertIn('INTEGRATION CHECKPOINT', result.stderr)
        self.assertIn(self.incoming[:12], result.stderr)
        self.assertEqual(self.git('rev-parse', 'HEAD^').stdout.strip(), self.main)
        self.assertEqual(self.git('status', '--porcelain').stdout, '')
        self.assertEqual(self.git('rev-parse', 'origin/main').stdout.strip(), self.incoming)

    def incoming_file(self, path):
        other = self.root / 'writer'
        self.git('clone', '-q', str(self.remote), str(other))
        self.git('config', 'user.name', 'Test', cwd=other)
        self.git('config', 'user.email', 'test@example.invalid', cwd=other)
        target = other / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text('incoming contract\n')
        self.git('add', '.', cwd=other)
        self.git('commit', '-qm', 'incoming path', cwd=other)
        self.git('push', '-q', 'origin', 'main', cwd=other)

    def test_unrelated_site_change_allows_finishing_slice(self):
        self.incoming_file('public/site/index.html')
        result = self.commit()
        self.assertIn('public/site/index.html', result.stderr)
        self.assertIn('otherwise finish the bounded slice', result.stderr)
        self.assertNotIn('Before the next work section', result.stderr)
        self.assertIn('Before merge, fetch current main', result.stderr)
        self.assertIn('disjoint paths do not prove independence', result.stderr)

    def test_all_incoming_paths_are_reported_even_for_large_deltas(self):
        # Build an incoming tree in the index, then restore it before committing
        # the feature checkpoint. The real hook fetches this tree from origin.
        paths = [f'public/site/asset-{n:02}.txt' for n in range(35)]
        for name in paths:
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('asset\n')
        self.git('add', 'public')
        tree = self.git('write-tree').stdout.strip()
        incoming = self.git('commit-tree', tree, '-p', self.main, '-m', 'many paths').stdout.strip()
        self.git('push', '-q', 'origin', f'{incoming}:refs/heads/main')
        self.git('read-tree', 'HEAD')
        shutil.rmtree(self.repo / 'public')
        result = self.commit()
        for path in paths:
            self.assertIn(repr(path), result.stderr)

    def test_shared_contract_change_prompts_assessment(self):
        self.incoming_file('shared/contract.rs')
        result = self.commit()
        self.assertIn('assess consumers before continuing dependent work', result.stderr)
        self.assertIn("Incoming areas: 'shared'", result.stderr)

    def test_overlapping_paths_are_reported(self):
        self.incoming_file('file')
        (self.repo / 'file').write_text('local contract\n')
        self.git('add', 'file')
        result = self.commit()
        self.assertIn("Changed on both branches (assess conflicts): 'file'", result.stderr)

    def test_partial_commit_preserves_unstaged_and_untracked_work(self):
        self.advance_main()
        (self.repo / 'file').write_text('unfinished\n')
        (self.repo / 'scratch').write_text('private fixture\n')
        result = self.commit()
        self.assertIn('Uncommitted work remains', result.stderr)
        self.assertEqual((self.repo / 'file').read_text(), 'unfinished\n')
        self.assertTrue((self.repo / 'scratch').exists())
        self.assertEqual(self.git('stash', 'list').stdout, '')

    def test_detached_and_main_commits_are_silent(self):
        self.advance_main()
        self.git('checkout', '-q', '--detach')
        self.assertEqual(self.commit().stderr, '')
        self.git('switch', '-q', 'main')
        self.assertEqual(self.commit().stderr, '')

    def test_integration_states_suppress_notifications(self):
        self.advance_main()
        for state in ('rebase-merge', 'rebase-apply', 'MERGE_HEAD', 'CHERRY_PICK_HEAD',
                      'REVERT_HEAD', 'sequencer', 'BISECT_START'):
            with self.subTest(state=state):
                path = Path(self.git('rev-parse', '--git-path', state).stdout.strip())
                if not path.is_absolute():
                    path = self.repo / path
                path.touch()
                try:
                    result = subprocess.run([str(self.hooks / 'post-commit')], cwd=self.repo,
                                            env=self.env, text=True, capture_output=True)
                    self.assertEqual((result.returncode, result.stderr), (0, ''))
                finally:
                    path.unlink()

    def test_failed_fetch_preserves_successful_commit_and_reports_unknown(self):
        self.git('remote', 'set-url', 'origin', str(self.root / 'missing'))
        result = self.commit()
        self.assertIn('freshness is unknown', result.stderr)
        self.assertNotEqual(self.git('rev-parse', 'HEAD').stdout.strip(), self.main)

    def test_linked_worktree_uses_its_own_state(self):
        self.advance_main()
        linked = self.root / 'linked'
        self.git('worktree', 'add', '-qb', 'linked', str(linked))
        result = self.commit(cwd=linked)
        self.assertIn('INTEGRATION CHECKPOINT', result.stderr)
        self.assertEqual(self.git('rev-parse', 'HEAD').stdout.strip(), self.main)

    def test_fetch_timeout_is_bounded(self):
        # Exercise the actual timeout/process-group cleanup without network access.
        bin_dir = self.root / 'bin'
        bin_dir.mkdir()
        real_git = shutil.which('git')
        wrapper = bin_dir / 'git'
        wrapper.write_text('#!/bin/sh\ncase "$*" in\n*credential.interactive*) exec sleep 30;;\nesac\nexec "' + real_git + '" "$@"\n')
        wrapper.chmod(0o755)
        self.env['PATH'] = f"{bin_dir}:{self.env['PATH']}"
        result = subprocess.run([str(self.hooks / 'post-commit')], cwd=self.repo,
                                env=self.env, text=True, capture_output=True, timeout=10)
        self.assertIn('freshness is unknown', result.stderr)
