"""Advisory selection cannot become execution or omit structural changes."""
import os
import json
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import owner_lanes as lanes


class OwnerTests(unittest.TestCase):
    def test_dependency_boundaries(self):
        cases = {
            'public/site/src/marketing.tsx': {'public-site'},
            'connect/web/src/Hosted.tsx': {'connect-web', 'public-site'},
            'connect/client/src/lib.rs': {'connect-client', 'capture'},
            'connect/service/src/worker.ts': {'connect-service', 'connect-client', 'connect-web', 'public-site', 'capture'},
            'capture/desktop/ui/frontend/src/App.tsx': {'desktop-ui', 'capture'},
            'capture/desktop/settings.rs': {'capture', 'engine-staged', 'desktop-ui'},
            'capture/engine/bundle/mod.rs': {'capture', 'engine-staged'},
            'shared/ui/src/index.ts': {'desktop-ui', 'connect-service', 'capture'},
        }
        for path, expected in cases.items():
            with self.subTest(path=path):
                self.assertEqual(set(lanes.select([('M', path)])['selected']), expected)
        for asset in lanes.ASSETS:
            self.assertEqual(set(lanes.select([('M', asset)])['selected']), {'public-site', 'connect-web'})

    def test_structural_unknown_and_missing_history(self):
        for changes, known in [([('A', 'connect/web/src/New.tsx')], True),
                               ([('D', 'public/site/src/old.ts')], True),
                               ([('D', 'connect/client/old.rs'), ('A', 'connect/web/new.ts')], True),
                               ([('M', 'unknown/input')], True), ([], False), ([], True)]:
            self.assertEqual(set(lanes.select(changes, known)['selected']), set(lanes.LANES))

    def test_deterministic_and_non_executing(self):
        changes = [('M', 'shared/ui/src/index.ts'), ('M', 'connect/web/src/Hosted.tsx')]
        self.assertEqual(lanes.select(changes), lanes.select(changes[::-1] + changes))
        self.assertFalse(lanes.select(changes)['executed'])
        self.assertIn('no tests executed', lanes.summary(lanes.select(changes)))

    def test_real_git_merge_base_and_structural_delta(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            def git(*args):
                env = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
                return subprocess.check_output(['git', *args], cwd=root, env=env, text=True).strip()
            git('init', '-q'); git('config', 'user.email', 'synthetic@example.invalid')
            git('config', 'user.name', 'Synthetic')
            path = root / 'public/site/src/marketing.tsx'
            path.parent.mkdir(parents=True); path.write_text('first')
            git('add', '.'); git('commit', '-qm', 'base'); base = git('rev-parse', 'HEAD')
            path.write_text('second'); git('commit', '-qam', 'marketing')
            clean_env = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
            with patch.object(lanes, 'ROOT', root), patch.dict(os.environ, clean_env, clear=True):
                self.assertEqual(lanes.plan(base, 'HEAD')['selected'], ['public-site'])
                path.rename(path.with_name('renamed.tsx'))
                git('add', '-A'); git('commit', '-qm', 'rename')
                self.assertEqual(set(lanes.plan(base, 'HEAD')['selected']), set(lanes.LANES))
                self.assertEqual(set(lanes.plan('missing', 'HEAD')['selected']), set(lanes.LANES))

    def test_entrypoints_call_real_suites(self):
        with patch.object(lanes.legacy, 'run_checks') as checks:
            lanes.run_lane('capture')
            self.assertEqual(checks.call_args.args[0], {'root': True, 'engine': False, 'browser': False, 'focused': []})
        with patch.object(lanes.subprocess, 'run') as run, patch.object(lanes.legacy, 'check_temp_environment'):
            lanes.run_lane('connect-client')
            self.assertEqual([call.args[0][1] for call in run.call_args_list], ['build', 'test'])
            self.assertTrue(all('connect/client/Cargo.toml' in call.args[0] for call in run.call_args_list))

    @unittest.skipUnless(shutil.which('node'), 'Public entrypoint requires Node')
    def test_public_entrypoint_environment_and_failure_propagation(self):
        node = shutil.which('node')
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            log = root / 'calls'
            # Inert subprocess boundary: execute the actual owner entrypoint,
            # retaining its environment, order and fail-fast behavior.
            stub = "#!/usr/bin/python3\nimport os,sys,json\nwith open(os.environ['CALL_LOG'],'a') as f: f.write(json.dumps([sys.argv[0],sys.argv[1:],os.environ.get('APPARENT_PUBLIC_SITE'),os.environ.get('VITE_HOSTED_API_ORIGIN')])+ '\\n')\nsys.exit(7 if os.path.basename(sys.argv[0]) == os.environ.get('FAIL_COMMAND') else 0)\n"
            for command in ('bun', 'npm', 'node', 'python3'):
                path = root / command
                path.write_text(stub); path.chmod(0o755)
            env = dict(os.environ, PATH=str(root), CALL_LOG=str(log))
            script = lanes.ROOT / 'public/site/scripts/site-build/verify.mjs'
            result = subprocess.run([node, str(script)], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            self.assertGreaterEqual(len(calls), 5)
            self.assertTrue(all(call[2:] == ['1', 'https://hosted.tryapparent.com'] for call in calls))
            self.assertIn('scripts/site-build/package-apparent.mjs', calls[2][1])
            self.assertTrue(any('scripts/pages/test-public-homepage.ts' in call[1] for call in calls))
            self.assertTrue(any('scripts/pages/verify-prepared.ts' in call[1] for call in calls))
            self.assertEqual(Path(calls[-1][0]).name, 'node')
            self.assertEqual(calls[-1][1], ['scripts/pages/prepare-pages-video.mjs'])
            self.assertFalse(any('connect/' in arg for call in calls for arg in call[1]))
            self.assertFalse(any('scripts/cd' in arg and arg.endswith('.py')
                                 for call in calls for arg in call[1]))
            log.unlink()
            result = subprocess.run([node, str(script)], env=dict(env, FAIL_COMMAND='npm'), capture_output=True)
            self.assertEqual(result.returncode, 7)
            self.assertEqual(len(log.read_text().splitlines()), 2)

    def test_workflow_keeps_full_verification_and_separate_advisory(self):
        workflow = (lanes.ROOT / '.github/workflows/verification-platform.yml').read_text()
        self.assertIn('git hash-object -t tree /dev/null', workflow)
        self.assertIn('verification.py --base "$BASE" --run --online', workflow)
        self.assertIn('owner_lanes.py --base HEAD^', workflow)
        self.assertNotIn('needs.select.outputs.selected', workflow)


if __name__ == '__main__':
    unittest.main()
