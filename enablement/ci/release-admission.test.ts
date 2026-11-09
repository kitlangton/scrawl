import { test } from 'node:test';
import assert from 'node:assert/strict';
import { admitsCD } from './release-admission.ts';

const repo = 'tryapparent/apparent', sha = 'a'.repeat(40);
const run = { repository: { full_name: repo }, head_repository: { full_name: repo },
  path: '.github/workflows/checks.yml', head_sha: sha, head_branch: 'main',
  event: 'push', status: 'completed', conclusion: 'success' };
const names = ['contracts', 'select', 'Hermes inspection (ubuntu-24.04)', 'Hermes inspection (macos-15-intel)',
  ...['linux', 'macos'].flatMap(platform => [`${platform} / select`,
    ...['desktop-ui', 'connect-client', 'rust'].map(name => `${platform} / ${name} (${platform === 'linux' ? 'ubuntu-24.04' : 'macos-15-intel'})`)])];
const jobs = names.map(name => ({ name, status: 'completed', conclusion: 'success' }));

test('exact main CD supplies both platforms without manual receipts', () => {
  for (const platform of ['linux', 'macos']) assert.ok(admitsCD(run, jobs, repo, sha, platform));
});
test('untrusted identities, events and incomplete runs fail closed', () => {
  for (const change of [{ head_sha: 'b'.repeat(40) }, { head_branch: 'feature' },
    { event: 'pull_request' }, { event: 'workflow_dispatch' }, { status: 'queued' },
    { conclusion: 'cancelled' }, { conclusion: null }, { path: 'other.yml' },
    { repository: { full_name: 'fork/repo' } }, { head_repository: { full_name: 'fork/repo' } }]) {
    assert.equal(admitsCD({ ...run, ...change }, jobs, repo, sha, 'linux'), false);
  }
});
test('every platform and shared job must actually pass in the selected attempt', () => {
  for (const platform of ['linux', 'macos']) {
    for (const job of jobs.filter(job => !job.name.startsWith(platform === 'linux' ? 'macos /' : 'linux /') && !(platform === 'linux' && job.name === 'Hermes inspection (macos-15-intel)'))) {
      for (const conclusion of ['skipped', 'failure', 'cancelled', null]) {
        assert.equal(admitsCD(run, jobs.map(value => value === job ? { ...value, conclusion } : value), repo, sha, platform), false, job.name);
      }
      assert.equal(admitsCD(run, jobs.filter(value => value !== job), repo, sha, platform), false);
    }
  }
});
test('failed Mac cannot hold Linux publication; Linux cannot authorize Mac', () => {
  const failed = jobs.map(job => job.name.startsWith('macos /') ? { ...job, conclusion: 'failure' } : job);
  assert.ok(admitsCD({ ...run, conclusion: 'failure' }, failed, repo, sha, 'linux'));
  assert.equal(admitsCD({ ...run, conclusion: 'failure' }, failed, repo, sha, 'macos'), false);
});

test('Linux can publish while Mac remains pending', () => {
  const pending = jobs.map(job => job.name.startsWith('macos /') ? { ...job, status: 'queued', conclusion: null } : job);
  const active = { ...run, status: 'in_progress', conclusion: null };
  assert.ok(admitsCD(active, pending, repo, sha, 'linux'));
  assert.equal(admitsCD(active, pending, repo, sha, 'macos'), false);
});
