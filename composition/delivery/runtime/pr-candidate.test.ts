import assert from 'node:assert/strict'
import { test } from 'node:test'
import { admitPrCandidate } from './pr-candidate.ts'
const landed = 'a'.repeat(40), head = 'b'.repeat(40), base = 'c'.repeat(40), tested = 'd'.repeat(40), tree = 'e'.repeat(40), digest = 'f'.repeat(64)
function fixture() {
  const repo = 'tryapparent/apparent'
  const data: Record<string, any> = {
    [`repos/${repo}/commits/${landed}/pulls?per_page=100`]: [{ number: 9, merged_at: '2026-11-06', merge_commit_sha: landed, head: { sha: head, repo: { full_name: repo } } }],
    [`repos/${repo}/actions/workflows/stream-engine.yml/runs?event=pull_request&head_sha=${head}&per_page=100`]: { workflow_runs: [{ id: 10, run_attempt: 2, event: 'pull_request', path: '.github/workflows/stream-engine.yml', conclusion: 'success', head_sha: head, head_repository: { full_name: repo }, pull_requests: [{ number: 9, head: { sha: head }, base: { sha: base } }] }] },
    [`repos/${repo}/actions/runs/10/attempts/2/jobs?per_page=100`]: { total_count: 3, jobs: ['deliver / Delivery contracts', 'verify-pr / pr-stream (engine, linux-x64)', 'deliver / Workflow runtime budget'].map(name => ({ name, status: 'completed', conclusion: 'success' })) },
    [`repos/${repo}/actions/runs/10/artifacts?per_page=100`]: { total_count: 1, artifacts: [{ id: 11, name: `pr-engine-linux-x64-${tested}-${digest}`, expired: false }] },
    [`repos/${repo}/git/commits/${tested}`]: { sha: tested, tree: { sha: tree }, parents: [{ sha: base }, { sha: head }] },
  }
  const values = Object.values(data)
  data[`repos/${repo}/git/commits/${landed}`] = { sha: landed, parents: [{ sha: base }] }
  return { data, pull: values[0][0], run: values[1].workflow_runs[0], jobs: values[2], artifact: values[3].artifacts[0], commit: values[4], assess: () => admitPrCandidate(<T>(endpoint: string) => { assert(endpoint in data, `Unexpected API: ${endpoint}`); return data[endpoint] as T }, landed, tree, digest) }
}
test('same tested tree with different SHA qualifies through canonical authenticated API identities', () => {
  const f = fixture(); f.run.pull_requests = [] // GitHub clears associations on merged runs.
  const result = f.assess()!
  assert.equal(result.reuse, true); assert.equal(result.prior_source_sha, tested)
  assert.equal(result.prior_run_attempt, 2); assert.equal(result.prior_run_id, 10)
})
test('main changes, wrong merge parents and untrusted/incomplete producers never qualify', () => {
  const mutations = [
    (f: ReturnType<typeof fixture>) => { f.commit.tree.sha = '0'.repeat(40) },
    (f: ReturnType<typeof fixture>) => { f.commit.parents[0].sha = head },
    (f: ReturnType<typeof fixture>) => { f.commit.parents[1].sha = base },
    (f: ReturnType<typeof fixture>) => { f.pull.merge_commit_sha = tested },
    (f: ReturnType<typeof fixture>) => { f.pull.merged_at = null },
    (f: ReturnType<typeof fixture>) => { f.pull.head.repo.full_name = 'someone/fork' },
    (f: ReturnType<typeof fixture>) => { f.run.head_repository.full_name = 'someone/fork' },
    (f: ReturnType<typeof fixture>) => { f.run.event = 'workflow_dispatch' },
    (f: ReturnType<typeof fixture>) => { f.run.path = '.github/workflows/other.yml' },
    (f: ReturnType<typeof fixture>) => { f.run.conclusion = 'failure' },
    (f: ReturnType<typeof fixture>) => { f.run.head_sha = tested },
    (f: ReturnType<typeof fixture>) => { f.artifact.expired = true },
    (f: ReturnType<typeof fixture>) => { f.artifact.name = f.artifact.name.replace(digest, '0'.repeat(64)) },
    (f: ReturnType<typeof fixture>) => { f.jobs.jobs[1].conclusion = 'skipped' },
    (f: ReturnType<typeof fixture>) => { f.jobs.jobs.push(f.jobs.jobs[1]); f.jobs.total_count++ },
    (f: ReturnType<typeof fixture>) => { f.jobs.total_count++ },
  ]
  for (const mutate of mutations) { const f = fixture(); mutate(f); assert.equal(f.assess(), undefined, mutate.toString()) }
})
