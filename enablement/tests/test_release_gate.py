"""Exact-commit, per-platform publication admission using synthetic GitHub APIs."""
import copy
import re
import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest

spec = importlib.util.spec_from_file_location('release_gate', Path(__file__).resolve().parents[1] / 'release_gate.py')
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class AdmissionTests(unittest.TestCase):
    def test_baseline_restoration_does_not_publish_or_start_full_platform_runs(self):
        root = Path(__file__).resolve().parents[2] / '.github/workflows'
        for name in ('verification.yml', 'verification-mac.yml',
                     'release-linux.yml', 'release-mac-preview.yml', 'web-delivery.yml'):
            workflow = (root / name).read_text()
            events = workflow.split('\non:\n', 1)[1].split('\npermissions:', 1)[0]
            triggers = re.findall(r'^  ([a-z_]+):', events, re.M)
            self.assertEqual(triggers, ['workflow_dispatch'], name)
        checks = (root / 'checks.yml').read_text()
        self.assertIn('  pull_request:', checks)
        self.assertIn('  push:', checks)
        self.assertIn('enablement/verification.py --base "$BASE_SHA" --pr', checks)
        self.assertNotIn('contents: write', checks)

    def setUp(self):
        self.repo = 'example/project'
        self.sha = 'a' * 40
        self.runs = {
            platform: dict(id=number, repository={'full_name': self.repo},
                           head_repository={'full_name': self.repo}, path=gate.WORKFLOWS[platform],
                           head_sha=self.sha, head_branch='main', event='push',
                           status='completed', conclusion='success', run_attempt=2)
            for platform, number in [('linux', 42), ('macos', 84)]
        }
        self.jobs = {
            platform: [dict(name=name, status='completed', conclusion='success')
                       for name in gate.REQUIRED[platform]]
            for platform in self.runs
        }
        self.status = 'identical'
        self.calls = []

    def fetch(self, path):
        self.calls.append(path)
        if '/compare/' in path:
            return {'status': self.status}
        if '/workflows/' in path:
            platform = 'macos' if '/verification-mac.yml/' in path else 'linux'
            return dict(total_count=1, workflow_runs=[copy.deepcopy(self.runs[platform])])
        number = int(path.split('/actions/runs/')[1].split('/')[0])
        platform = 'linux' if number == 42 else 'macos'
        if '/jobs?' in path:
            self.assertIn('/attempts/2/', path)
            return dict(total_count=len(self.jobs[platform]), jobs=copy.deepcopy(self.jobs[platform]))
        return copy.deepcopy(self.runs[platform])

    def admit(self, platform='linux', run_id=None):
        return gate.admit(self.repo, self.sha, run_id, self.fetch, platform=platform)

    def test_exact_successful_main_commit_is_admitted(self):
        for platform, number in [('linux', '42'), ('macos', '84')]:
            self.assertEqual(self.admit(platform, number), self.sha)
            self.assertEqual(self.admit(platform), self.sha)

    def test_manual_feature_checks_preserve_publication_boundary(self):
        root = Path(__file__).resolve().parents[2]
        workflow = (root / '.github/workflows/verification-platform.yml').read_text()
        for job in ('desktop-ui', 'rust'):
            section = re.search(r'^  ' + job + r':\n(.*?)(?=^  [a-z]|\Z)', workflow, re.M | re.S)[1]
            self.assertIn("if: github.ref == 'refs/heads/main' || github.event_name == 'workflow_dispatch'", section)
        receipt = workflow.split('  release-ready:\n', 1)[1]
        self.assertIn("github.ref == 'refs/heads/main'", receipt)
        self.assertIn("github.ref == 'refs/heads/t3/onboarding-mac-release-baseline'", receipt)
        self.assertIn("github.repository == 'tryapparent/apparent'", receipt)
        self.assertIn("github.event_name == 'workflow_dispatch'", receipt)
        for platform in self.runs:
            self.runs[platform].update(event='workflow_dispatch', head_branch='feature')
        for platform in self.runs:
            with self.assertRaises(ValueError):
                self.admit(platform)

    def test_intel_mac_receipt_matches_workflow_and_is_required(self):
        workflow = Path(__file__).resolve().parents[2] / gate.WORKFLOWS['macos']
        self.assertIn('runner: macos-15-intel', workflow.read_text())
        names = ['verify / select', 'verify / desktop-ui (macos-15-intel)',
                 'verify / rust (macos-15-intel)', 'verify / Release verification']
        self.jobs['macos'] = [dict(name=name, status='completed', conclusion='success')
                              for name in names]
        self.assertEqual(self.admit('macos'), self.sha)
        for job in self.jobs['macos']:
            job['name'] = job['name'].replace('macos-15-intel', 'macos-latest')
        with self.assertRaises(ValueError): self.admit('macos')

    def test_linux_checks_can_publish_without_a_mac_receipt(self):
        self.runs['macos'].update(status='queued', conclusion=None)
        self.jobs['macos'] = []
        self.assertEqual(self.admit(run_id='42'), self.sha)
        self.assertEqual(self.admit(), self.sha)
        self.assertFalse(any('verification-mac' in call or '/runs/84' in call for call in self.calls))
        with self.assertRaisesRegex(ValueError, 'macos main verification'):
            self.admit('macos')

    def test_mac_release_requires_linux_shared_and_mac_checks(self):
        for platform in self.runs:
            old = self.runs[platform]['conclusion']
            self.runs[platform]['conclusion'] = 'failure'
            with self.assertRaisesRegex(ValueError, platform + ' main verification'):
                self.admit('macos')
            self.runs[platform]['conclusion'] = old
        with self.assertRaises(ValueError): self.admit('macos', '42')
        with self.assertRaises(ValueError): self.admit('linux', '84')

    def test_unrelated_or_unsuccessful_runs_never_admit(self):
        for platform in self.runs:
            for key, value in [('head_sha', 'b'*40), ('head_branch', 'feature'),
                               ('event', 'pull_request'), ('event', 'workflow_dispatch'),
                               ('path', '.github/workflows/other.yml'), ('status', 'in_progress'),
                               ('conclusion', 'failure'), ('conclusion', 'cancelled'),
                               ('repository', {'full_name': 'other/repo'}),
                               ('head_repository', {'full_name': 'fork/repo'})]:
                with self.subTest(platform=platform, key=key, value=value):
                    old = copy.deepcopy(self.runs[platform])
                    self.runs[platform][key] = value
                    with self.assertRaises(ValueError): self.admit(platform)
                    self.runs[platform] = old

    def test_every_required_job_must_have_run_and_passed(self):
        for platform in self.runs:
            original = copy.deepcopy(self.jobs[platform])
            for job in self.jobs[platform]:
                for conclusion in ('failure', 'skipped', 'cancelled', None):
                    with self.subTest(platform=platform, name=job['name'], conclusion=conclusion):
                        job['conclusion'] = conclusion
                        with self.assertRaises(ValueError): self.admit(platform)
                job['conclusion'] = 'success'
                job['status'] = 'queued'
                with self.assertRaises(ValueError): self.admit(platform)
                job['status'] = 'completed'
            self.jobs[platform] = [j for j in self.jobs[platform] if j['name'] != 'verify / Release verification']
            with self.assertRaises(ValueError): self.admit(platform)
            self.jobs[platform] = original

    def test_legacy_combined_or_wrong_platform_job_names_cannot_supply_receipt(self):
        self.jobs['linux'] = copy.deepcopy(self.jobs['macos'])
        with self.assertRaises(ValueError): self.admit()
        self.jobs['linux'] = [dict(name=name, status='completed', conclusion='success') for name in
                              ['select', 'desktop-ui (ubuntu-24.04)', 'rust (ubuntu-24.04)', 'Release verification']]
        with self.assertRaises(ValueError): self.admit()

    def test_diverged_or_removed_commit_is_refused(self):
        for status in ('behind', 'diverged', 'unknown'):
            self.status = status
            with self.assertRaises(ValueError): self.admit()

    def test_empty_history_and_api_failure_fail_closed(self):
        with self.assertRaises(ValueError):
            gate.admit(self.repo, self.sha, fetch=lambda _: dict(total_count=0, workflow_runs=[]))
        def failed(_): raise RuntimeError('API unavailable')
        with self.assertRaises(RuntimeError): gate.admit(self.repo, self.sha, fetch=failed)

    def test_malformed_inputs_are_refused_before_api(self):
        for sha in ('main', '', 'a'*39, 'A'*40):
            with self.assertRaises(ValueError): gate.admit(self.repo, sha, fetch=self.fetch)
        with self.assertRaises(ValueError): self.admit(run_id='../42')
        with self.assertRaises(ValueError): self.admit(platform='windows')
        self.assertEqual(self.calls, [])

    def test_cli_requires_explicit_publication_platform(self):
        result = subprocess.run([sys.executable, gate.__file__, '--sha', self.sha], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn('--platform', result.stderr)

    def test_paginated_jobs_and_incomplete_history(self):
        def fetch(path):
            if '/jobs?' in path:
                platform = 'linux' if '/runs/42/' in path else 'macos'
                page = int(path.rsplit('page=', 1)[1])
                jobs = self.jobs[platform]
                return dict(total_count=len(jobs), jobs=jobs[:2] if page == 1 else jobs[2:])
            return self.fetch(path)
        self.assertEqual(gate.admit(self.repo, self.sha, '42', fetch), self.sha)
        self.assertEqual(gate.admit(self.repo, self.sha, '84', fetch, platform='macos'), self.sha)
        with self.assertRaises(ValueError):
            gate.pages('jobs', lambda _: dict(total_count=1, workflow_runs=[]))



class OnboardingAdmissionTests(unittest.TestCase):
    fetch_main = AdmissionTests.fetch
    def setUp(self):
        AdmissionTests.setUp(self)
        self.repo = 'tryapparent/apparent'
        self.status = 'ahead'
        self.branch_sha = self.sha
        self.jobs['macos'].append(dict(name='onboarding-candidate', status='completed', conclusion='success'))
        for run in self.runs.values():
            run.update(repository={'full_name': self.repo}, head_repository={'full_name': self.repo},
                       head_branch='t3/onboarding-mac-release-baseline', event='workflow_dispatch')

    def fetch(self, path):
        if '/git/ref/heads/' in path:
            return {'object': {'type': 'commit', 'sha': self.branch_sha}}
        return self.fetch_main(path)

    def candidate(self):
        return gate.admit(self.repo, self.sha, fetch=self.fetch, platform='macos', onboarding_candidate=True)

    def test_scoped_candidate_requires_exact_both_platforms_and_current_branch(self):
        self.assertEqual(self.candidate(), self.sha)
        for platform in self.runs:
            original = copy.deepcopy(self.runs[platform])
            for field, value in [('head_sha', 'b'*40), ('head_branch', 'feature'), ('event', 'push'),
                                 ('conclusion', 'failure'), ('status', 'queued'), ('path', 'other.yml')]:
                with self.subTest(platform=platform, field=field):
                    self.runs[platform][field] = value
                    with self.assertRaises(ValueError): self.candidate()
                    self.runs[platform] = copy.deepcopy(original)
            receipt = self.jobs[platform].pop()
            with self.assertRaises(ValueError): self.candidate()
            self.jobs[platform].append(receipt)
        self.branch_sha = 'b'*40
        with self.assertRaisesRegex(ValueError, 'branch moved'): self.candidate()

    def test_scoped_candidate_requires_published_baseline_ancestry(self):
        for status in ['behind', 'diverged', 'identical']:
            self.status = status
            with self.assertRaisesRegex(ValueError, 'descend'): self.candidate()

    def test_exception_cannot_admit_other_repositories_or_linux_publication(self):
        for repo, platform in [('example/project', 'macos'), (self.repo, 'linux')]:
            with self.assertRaises(ValueError):
                gate.admit(repo, self.sha, fetch=self.fetch, platform=platform, onboarding_candidate=True)


if __name__ == '__main__': unittest.main()
