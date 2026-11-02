"""Exact-commit publication admission with synthetic GitHub API responses."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('release_gate', Path(__file__).resolve().parents[1] / 'release_gate.py')
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class AdmissionTests(unittest.TestCase):
    def setUp(self):
        self.repo = 'example/project'
        self.sha = 'a' * 40
        self.run = dict(id=42, repository={'full_name': self.repo},
                        head_repository={'full_name': self.repo}, path=gate.WORKFLOW,
                        head_sha=self.sha, head_branch='main', event='push',
                        status='completed', conclusion='success', run_attempt=2)
        self.jobs = [dict(name=name, status='completed', conclusion='success') for name in gate.REQUIRED]
        self.status = 'identical'
        self.calls = []

    def fetch(self, path):
        self.calls.append(path)
        if '/jobs?' in path:
            self.assertIn('/attempts/2/', path)
            return dict(total_count=len(self.jobs), jobs=self.jobs)
        if '/compare/' in path:
            return {'status': self.status}
        if '/workflows/' in path:
            return dict(total_count=1, workflow_runs=[self.run])
        return copy.deepcopy(self.run)

    def admit(self, run_id=None):
        return gate.admit(self.repo, self.sha, run_id, self.fetch)

    def test_exact_successful_main_commit_is_admitted(self):
        self.assertEqual(self.admit('42'), self.sha)
        self.assertEqual(self.admit(), self.sha)

    def test_unrelated_or_unsuccessful_runs_never_admit(self):
        for key, value in [('head_sha', 'b'*40), ('head_branch', 'feature'),
                           ('event', 'pull_request'), ('event', 'workflow_dispatch'),
                           ('path', '.github/workflows/other.yml'), ('status', 'in_progress'),
                           ('conclusion', 'failure'), ('conclusion', 'cancelled'),
                           ('repository', {'full_name': 'other/repo'}),
                           ('head_repository', {'full_name': 'fork/repo'})]:
            with self.subTest(key=key, value=value):
                old = copy.deepcopy(self.run)
                self.run[key] = value
                with self.assertRaises(ValueError): self.admit('42')
                self.run = old

    def test_every_required_job_must_have_run_and_passed(self):
        for job in self.jobs:
            for conclusion in ('failure', 'skipped', 'cancelled', None):
                with self.subTest(name=job['name'], conclusion=conclusion):
                    job['conclusion'] = conclusion
                    with self.assertRaises(ValueError): self.admit()
            job['conclusion'] = 'success'
        self.jobs = [j for j in self.jobs if j['name'] != 'Release verification']
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
        with self.assertRaises(ValueError): self.admit('../42')
        self.assertEqual(self.calls, [])

    def test_paginated_jobs_and_incomplete_history(self):
        def fetch(path):
            if '/jobs?' in path:
                page = int(path.rsplit('page=', 1)[1])
                return dict(total_count=len(self.jobs), jobs=self.jobs[:2] if page == 1 else self.jobs[2:])
            return self.fetch(path)
        self.assertEqual(gate.admit(self.repo, self.sha, '42', fetch), self.sha)
        with self.assertRaises(ValueError):
            gate.pages('jobs', lambda _: dict(total_count=1, workflow_runs=[]))


if __name__ == '__main__': unittest.main()
