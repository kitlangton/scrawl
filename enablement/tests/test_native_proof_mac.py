"""Focused Mac workflow admission and actual owned-package cleanup contracts."""
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / '.github/workflows/native-proof-mac.yml'


class NativeProofMacTests(unittest.TestCase):
    def test_manual_standard_runner_read_only_and_bounded(self):
        workflow = WORKFLOW.read_text()
        self.assertRegex(workflow, r'(?m)^on:\n  workflow_dispatch:\s*\n')
        self.assertNotRegex(workflow, r'(?m)^  (push|pull_request|schedule):')
        self.assertIn('runs-on: macos-15-intel\n', workflow)
        self.assertIn('permissions:\n  contents: read\n', workflow)
        self.assertNotIn('secrets.', workflow)
        self.assertNotIn('write\n', workflow)
        self.assertIn("github.event_name == 'workflow_dispatch'", workflow)
        self.assertIn('    timeout-minutes: 10\n', workflow)
        self.assertIn('timeout-minutes: 4\n', workflow)
        self.assertIn('persist-credentials: false', workflow)

    def test_build_installed_candidate_and_checked_worker_without_full_suites(self):
        workflow = WORKFLOW.read_text()
        self.assertIn('working-directory: connect/service\n', workflow)
        self.assertLess(workflow.index('npm ci --no-audit --no-fund'), workflow.index('npm run build'))
        self.assertLess(workflow.index('npm run build'), workflow.index('node connect/service/test/native/desktop-worker.mjs'))
        self.assertIn('APPARENT_NO_LAUNCH:', workflow)
        self.assertIn('bash capture/desktop/scripts/packaging/install-app.sh', workflow)
        self.assertIn('codesign --verify --deep --strict "$APPARENT_APP_DIR"', workflow)
        self.assertIn('"$APPARENT_APP_DIR/Contents/MacOS/apparent"', workflow)
        self.assertNotRegex(workflow, r'cargo test|npm test|verification\.py|release_gate\.py|release-ready|workflow_run')
        self.assertIn('APPARENT_CONNECT_NATIVE_CI:', workflow)
        self.assertIn("if: always()", workflow)
        self.assertIn('path: ${{ runner.temp }}/apparent-connect-native/evidence/', workflow)
        self.assertIn("for name in ('report.json', 'desktop.log'):", workflow)
        self.assertNotIn('include-hidden-files: true', workflow)

    def test_registered_dispatcher_keeps_full_ci_main_only(self):
        dispatcher = (ROOT / '.github/workflows/verification-mac.yml').read_text()
        full = dispatcher.split('  verify:\n', 1)[1].split('  connect-native:', 1)[0]
        self.assertIn("if: github.ref == 'refs/heads/main'\n", full)
        self.assertIn('uses: ./.github/workflows/verification-platform.yml', full)
        focused = dispatcher.split('  connect-native:\n', 1)[1]
        self.assertIn("github.ref != 'refs/heads/main'", focused)
        self.assertIn("github.event_name == 'workflow_dispatch'", focused)
        self.assertIn('&& inputs.connect_worker_proof', focused)
        self.assertIn('uses: ./.github/workflows/native-proof-mac.yml', focused)
        self.assertIn('type: boolean\n        default: false', dispatcher)
        self.assertIn('  workflow_call:', WORKFLOW.read_text())

    def test_main_dependency_cache_namespace_inputs_and_failure_retention(self):
        workflow = WORKFLOW.read_text()
        main = (ROOT / '.github/workflows/verification-platform.yml').read_text()
        block = workflow.split('      - uses: Swatinem/rust-cache@v2\n', 1)[1].split('      - uses:', 1)[0]
        main_root = main.split('  rust:\n', 1)[1]
        main_block = main_root.split('      - uses: Swatinem/rust-cache@v2\n', 1)[1].split('      - uses:', 1)[0]
        def workspaces(text):
            return re.search(r'          workspaces: \|\n((?:            .*\n)+)', text)[1]
        self.assertEqual(workspaces(block), workspaces(main_block))
        self.assertIn('  rust:\n', main)
        self.assertIn('          shared-key: rust\n', block)
        self.assertIn("          cache-on-failure: 'true'\n", block)
        self.assertNotRegex(block, r'prefix-key:|add-job-id-key:|add-rust-environment-hash-key:|env-vars:|key:.*Darwin')
        build = workflow.split('      - name: Build and install the candidate without launching\n', 1)[1].split('      - name:', 1)[0]
        self.assertIn('timeout-minutes: 30\n', build)
        self.assertIn('timeout-minutes: 4\n', workflow)
        for name in ('Verify owned cleanup and retain only allowlisted evidence',
                     'Retain synthetic proof evidence including failures'):
            step = workflow.split('      - name: ' + name + '\n', 1)[1].split('      - name:', 1)[0]
            self.assertIn('if: always()\n', step)
            self.assertIn('timeout-minutes: 2\n', step)

    def test_job_budget_bounds_steps_and_keeps_cleanup_required(self):
        workflow = WORKFLOW.read_text()
        job_cap = int(re.search(r'^    timeout-minutes: (\d+)$', workflow, re.M)[1])
        self.assertEqual(job_cap, 10)
        steps = re.split(r'^      - ', workflow.split('  runtime-budget:\n', 1)[0].split('    steps:\n', 1)[1], flags=re.M)[1:]
        self.assertTrue(steps[0].startswith('name: Verification runtime budget and ownership\n'))
        for step in steps:
            if step.startswith('name: Verification runtime budget and ownership\n'):
                # The notice runs inside the job budget, before setup.
                self.assertIn('runner queue excluded', step)
                self.assertIn('agent whose run hits the limit owns optimization', step)
                self.assertIn('do not raise or bypass the limit', step)
                continue
            cap = re.search(r'^        timeout-minutes: (\d+)$', step, re.M)
            self.assertIsNotNone(cap, step.splitlines()[0])
            self.assertGreater(int(cap[1]), 0)
        # Step caps are ceilings, not additive allocations: the job's 10m
        # deadline wins. They cannot prove a cleanup reserve. The package/cache
        # optimization must leave time for these mandatory final steps; a job
        # timeout is failed proof, never evidence of successful cleanup.
        cleanup_names = ('Verify owned cleanup and retain only allowlisted evidence',
                         'Retain synthetic proof evidence including failures')
        for step, name in zip(steps[-2:], cleanup_names):
            self.assertTrue(step.startswith('name: ' + name + '\n'))
            self.assertIn('        if: always()\n', step)
            self.assertIn('        timeout-minutes: 2\n', step)

    def test_share_build_is_required_and_never_falls_back_to_recompilation(self):
        workflow = WORKFLOW.read_text()
        build, native = workflow.split('  connect-worker:\n', 1)
        self.assertIn('  share-build:\n    name: share-build\n', build)
        self.assertIn("inputs.scenario == 'share' && inputs.producer_run == '' && inputs.producer_artifact == ''", build)
        self.assertIn('--producer-kind focused-share-build', build)
        self.assertIn('artifact: ${{ steps.executables.outputs.artifact-id }}', build)
        self.assertIn('needs: share-build', native)
        self.assertIn('  runtime-budget:\n    if: always()\n    needs: [share-build, connect-worker]', workflow)
        self.assertIn("needs.share-build.result == 'success'", native)
        self.assertIn("needs.share-build.result == 'skipped'", native)
        self.assertNotIn("needs.share-build.result == 'failure'", native)
        self.assertIn('!cancelled()', native)
        self.assertIn("'focused-share-build' || 'verified-main'", native)
        self.assertIn('needs.share-build.outputs.artifact || inputs.producer_artifact', native)
        self.assertEqual(re.findall(r'^    timeout-minutes: (\d+)$', workflow.split('  runtime-budget:\n', 1)[0], re.M), ['10', '10'])

    def test_share_scenario_is_explicit_and_cannot_run_worker_case(self):
        workflow = WORKFLOW.read_text()
        self.assertIn('options: [worker, share]\n        default: worker', workflow)
        worker = workflow.split('      - name: Installed Connect worker restart and encrypted fetch\n', 1)[1].split('      - name:', 1)[0]
        share = workflow.split('      - name: Installed completed-capture public copy\n', 1)[1].split('      - name:', 1)[0]
        self.assertIn("if: inputs.scenario != 'share'", worker)
        self.assertIn("if: inputs.scenario == 'share'", share)
        self.assertIn('desktop-worker.mjs', worker)
        self.assertNotIn('desktop-share.ts', worker)
        self.assertIn('desktop-share.ts', share)
        self.assertNotIn('desktop-worker.mjs', share)
        self.assertIn('name: mac-connect-${{ inputs.scenario || \'worker\' }}-native', workflow)

    def run_cleanup(self, *, owned=False, unsafe=False):
        script = re.search(r'      - name: Verify owned cleanup.*?python3 - <<\'PY\'\n(.*?)^          PY',
                           WORKFLOW.read_text(), re.M | re.S)[1]
        script = textwrap.dedent(script)
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            app = home / 'Applications/Apparent-Connect-42-1.app'
            build = home / '.apparent-connect-build-42-1'
            app.mkdir(parents=True)
            build.mkdir()
            unrelated = home / 'Applications/Other.app'
            unrelated.mkdir()
            proof = home / 'proof'
            scenario = proof / 'scenario/evidence'
            scenario.mkdir(parents=True)
            (scenario / 'report.json').write_text('{"passed":true}')
            (scenario / 'desktop.log').write_text('synthetic')
            (scenario / 'credentials.json').write_text('must not copy')
            binary = home / 'bin'
            binary.mkdir()
            ps = binary / 'ps'
            ps.write_text('#!/bin/sh\nprintf "%s\\n" ' + repr(str(app / 'Contents/MacOS/apparent') if owned else '/unrelated/process') + '\n')
            ps.chmod(0o700)
            env = dict(os.environ, HOME=str(home), PATH=str(binary) + ':' + os.environ['PATH'],
                       APPARENT_APP_DIR=str(unrelated if unsafe else app), APPARENT_BUILD_TMP=str(build),
                       APPARENT_CONNECT_PROOF=str(proof), APPARENT_NATIVE_SCENARIO='worker', GITHUB_RUN_ID='42', GITHUB_RUN_ATTEMPT='1')
            result = subprocess.run(['python3', '-c', script], env=env, capture_output=True)
            self.assertTrue(unrelated.exists())
            self.assertFalse((proof / 'evidence/credentials.json').exists())
            self.assertEqual((proof / 'evidence/report.json').read_text(), '{"passed":true}')
            if owned or unsafe:
                self.assertNotEqual(result.returncode, 0)
                self.assertTrue(app.exists())
                self.assertTrue(build.exists())
            else:
                self.assertEqual(result.returncode, 0, result.stderr.decode())
                self.assertFalse(app.exists())
                self.assertFalse(build.exists())
                self.assertEqual(json.loads((proof / 'evidence/cleanup.json').read_text()), {
                    'noInstalledProcesses': True, 'installedAppRemoved': True, 'buildTemporaryRemoved': True})

    def test_only_owned_package_and_build_tree_removed(self):
        self.run_cleanup()

    def test_live_owned_package_prevents_deletion_and_fails_proof(self):
        self.run_cleanup(owned=True)

    def test_wrong_package_path_fails_without_deletion(self):
        self.run_cleanup(unsafe=True)
