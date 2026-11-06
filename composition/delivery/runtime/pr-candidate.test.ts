import assert from 'node:assert/strict'
import { test } from 'node:test'
import { admitPrCandidate } from './pr-candidate.ts'
const landed = 'a'.repeat(40), head = 'b'.repeat(40), base = 'c'.repeat(40), tested = 'd'.repeat(40), tree = 'e'.repeat(40), digest = 'f'.repeat(64)
function fixture(ownerId = 'engine', target = 'linux-x64') {
  let matches = true
  const repo = 'tryapparent/apparent'
  const workflow = ['app-shell', 'platform-config'].includes(ownerId) ? 'stream-platform.yml' : ownerId === 'onboarding' ? 'stream-onboarding.yml' : 'stream-engine.yml'
  const data: Record<string, any> = {
    [`repos/${repo}/commits/${landed}/pulls?per_page=100`]: [{ number: 9, merged_at: '2026-11-06', merge_commit_sha: landed, head: { sha: head, repo: { full_name: repo } } }],
    [`repos/${repo}/actions/workflows/${workflow}/runs?event=pull_request&head_sha=${head}&per_page=100`]: { workflow_runs: [{ id: 10, run_attempt: 2, event: 'pull_request', path: `.github/workflows/${workflow}`, conclusion: 'success', head_sha: head, head_repository: { full_name: repo }, pull_requests: [{ number: 9, head: { sha: head }, base: { sha: base } }] }] },
    [`repos/${repo}/actions/runs/10/attempts/2/jobs?per_page=100`]: { total_count: 3, jobs: ['deliver / Delivery contracts', `verify-pr / pr-stream (${ownerId}, ${target})`, 'deliver / Workflow runtime budget'].map(name => ({ name, status: 'completed', conclusion: 'success' })) },
    [`repos/${repo}/actions/runs/10/artifacts?per_page=100`]: { total_count: 1, artifacts: [{ id: 11, name: `pr-${ownerId}-${target}-${tested}-${digest}`, expired: false }] },
    [`repos/${repo}/git/commits/${tested}`]: { sha: tested, tree: { sha: tree }, parents: [{ sha: base }, { sha: head }] },
  }
  const values = Object.values(data)
  data[`repos/${repo}/git/commits/${landed}`] = { sha: landed, parents: [{ sha: base }] }
  return { data, pull: values[0][0], run: values[1].workflow_runs[0], jobs: values[2], artifact: values[3].artifacts[0], commit: values[4], rejectInputs: () => { matches = false }, assess: () => admitPrCandidate(<T>(endpoint: string) => { assert(endpoint in data, `Unexpected API: ${endpoint}`); return data[endpoint] as T }, landed, digest, (sha, testedBase, landedBase) => matches && sha === tested && testedBase === base && landedBase === base, ownerId, target) }
}
test('same tested tree with different SHA qualifies through canonical authenticated API identities', () => {
  const f = fixture(); f.run.pull_requests = [] // GitHub clears associations on merged runs.
  const result = f.assess()!
  assert.equal(result.reuse, true); assert.equal(result.prior_source_sha, tested)
  assert.equal(result.prior_run_attempt, 2); assert.equal(result.prior_run_id, 10)
})
test('changed inputs, wrong merge parents and untrusted/incomplete producers never qualify', () => {
  const mutations = [
    (f: ReturnType<typeof fixture>) => { f.rejectInputs() },
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


test('native Engine targets require their own successful PR job and candidate artifact', () => {
  for (const target of ['linux-x64', 'macos-x64']) {
    const good = fixture('engine-native', target)
    assert.equal(good.assess()?.reuse, true)
    good.artifact.name = good.artifact.name.replace('engine-native', 'engine')
    assert.equal(good.assess(), undefined)
    const wrongTarget = fixture('engine-native', target)
    wrongTarget.jobs.jobs[1].name = 'verify-pr / pr-stream (engine-native, unsupported)'
    assert.equal(wrongTarget.assess(), undefined)
  }
  for (const [id, target] of [['engine', 'macos-x64'], ['engine-native', 'macos-arm64'], ['platform-resources', 'linux-x64']]) {
    assert.equal(admitPrCandidate(() => { throw Error('Unsupported lane must not query GitHub') }, landed, digest, () => true, id, target), undefined)
  }
})


test('App Shell candidates require their exact workflow, platform job and packaged artifact', () => {
  for (const target of ['linux-x64', 'macos-x64']) {
    const f = fixture('app-shell', target)
    assert.equal(f.assess()?.reuse, true)
    f.run.path = '.github/workflows/stream-engine.yml'
    assert.equal(f.assess(), undefined)
    const wrongArtifact = fixture('app-shell', target)
    wrongArtifact.artifact.name = wrongArtifact.artifact.name.replace('app-shell', 'engine-native')
    assert.equal(wrongArtifact.assess(), undefined)
    const skipped = fixture('app-shell', target)
    skipped.jobs.jobs[1].conclusion = 'skipped'
    assert.equal(skipped.assess(), undefined)
  }
})


test('media candidates are qualified independently for Linux and Intel Mac', () => {
  for (const target of ['linux-x64', 'macos-x64']) {
    const f = fixture('engine-media-native', target)
    assert.equal(f.assess()?.reuse, true)
    f.artifact.name = f.artifact.name.replace('engine-media-native', 'engine-native')
    assert.equal(f.assess(), undefined)
    const wrongTarget = fixture('engine-media-native', target)
    wrongTarget.jobs.jobs[1].name = 'verify-pr / pr-stream (engine-media-native, unavailable)'
    assert.equal(wrongTarget.assess(), undefined)
    assert.equal(admitPrCandidate(() => { throw Error('Unsupported target') }, landed, digest, () => true, 'engine-media-native', 'macos-arm64'), undefined)
  }
})


test('Config candidates require the Config workflow and exact staged-helper target', () => {
  for (const target of ['linux-x64', 'macos-x64']) {
    const f = fixture('platform-config', target)
    assert.equal(f.assess()?.reuse, true)
    f.run.path = '.github/workflows/stream-engine.yml'
    assert.equal(f.assess(), undefined)
    const wrong = fixture('platform-config', target)
    wrong.jobs.jobs[1].name = 'verify-pr / pr-stream (engine-native, ' + target + ')'
    assert.equal(wrong.assess(), undefined)
  }
  assert.equal(admitPrCandidate(() => { throw Error('Unsupported Config target') }, landed, digest, () => true, 'platform-config', 'web'), undefined)
})


test('Onboarding candidates require their own package workflow and exact platform', () => {
  for (const target of ['linux-x64', 'macos-x64']) {
    const f = fixture('onboarding', target)
    assert.equal(f.assess()?.reuse, true)
    f.run.path = '.github/workflows/stream-app-shell.yml'
    assert.equal(f.assess(), undefined)
    const wrong = fixture('onboarding', target)
    wrong.artifact.name = wrong.artifact.name.replace('onboarding', 'app-shell')
    assert.equal(wrong.assess(), undefined)
  }
  assert.equal(admitPrCandidate(() => { throw Error('Unsupported package target') }, landed, digest, () => true, 'onboarding', 'web'), undefined)
})

test('different whole trees qualify only after committed-input and base ancestry verification', () => {
  const f = fixture(); f.commit.tree.sha = '0'.repeat(40)
  assert.equal(f.assess()?.reuse, true)
  f.rejectInputs(); assert.equal(f.assess(), undefined)
})
