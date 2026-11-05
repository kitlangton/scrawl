import { test } from 'node:test'
import assert from 'node:assert/strict'
import { admission, owners, publish, runResult, selectedOwners } from './admission.ts'

test('selects every owner whose path filter matches and preserves independent owners', () => {
  assert.deepEqual(selectedOwners(['public/site/src/a.ts', 'record/src/a.ts']).map(o => o.workflow),
    ['.github/workflows/stream-public.yml', '.github/workflows/stream-record.yml'])
  assert.equal(selectedOwners(['composition/delivery/core.ts']).length, owners.length)
  assert.deepEqual(selectedOwners(['unrelated/file.txt']), [])
})

const owner = owners.find(o => o.workflow.endsWith('stream-app-shell.yml'))!
const completed = { path: owner.workflow, head_sha: 'a'.repeat(40), run_number: 3, run_attempt: 1, status: 'completed', conclusion: 'success' }
const jobs = [
  { name: 'deliver / Delivery contracts', status: 'completed', conclusion: 'success' },
  { name: 'deliver / Workflow runtime budget', status: 'completed', conclusion: 'success' },
  { name: 'verify-pr / pr-stream (app-shell, linux-x64)', status: 'completed', conclusion: 'success' },
  { name: 'verify-pr / pr-stream (app-shell, macos-x64)', status: 'completed', conclusion: 'success' },
]
test('requires exact owner targets and shared contracts for success', () => {
  assert.equal(runResult(owner, [completed], completed.head_sha, () => jobs), 'success')
  assert.equal(runResult(owner, [], completed.head_sha, () => []), 'pending')
  assert.equal(runResult(owner, [{ ...completed, status: 'in_progress', conclusion: null }], completed.head_sha, () => []), 'pending')
})
test('failed, skipped, incomplete and stale owner results cannot pass', () => {
  assert.equal(runResult(owner, [completed], completed.head_sha, () => jobs.slice(0, 2)), 'failure')
  assert.equal(runResult(owner, [completed], completed.head_sha, () => jobs.map((j, i) => i === 2 ? { ...j, conclusion: 'skipped' } : j)), 'failure')
  assert.equal(runResult(owner, [{ ...completed, conclusion: 'failure' }], completed.head_sha, () => jobs), 'failure')
  assert.equal(runResult(owner, [completed], completed.head_sha, () => jobs.map((j, i) => i === 0 ? { ...j, conclusion: 'failure' } : j)), 'failure')
  assert.equal(runResult(owner, [completed, { ...completed, run_number: 4, status: 'in_progress', conclusion: null }], completed.head_sha, () => jobs), 'pending')
  assert.equal(runResult(owner, [{ ...completed, head_sha: 'b'.repeat(40) }], completed.head_sha, () => jobs), 'pending')
})
test('admission remains pending until all expected owners pass and fails closed on failures', () => {
  const expected = [owners[0], owners[1]]
  assert.equal(admission(expected, [{ workflow: owners[0].workflow, status: 'success' }]).conclusion, null)
  assert.equal(admission(expected, expected.map(o => ({ workflow: o.workflow, status: 'success' as const }))).conclusion, 'success')
  assert.equal(admission(expected, expected.map(o => ({ workflow: o.workflow, status: 'failure' as const }))).conclusion, 'failure')
  assert.equal(admission([], []).conclusion, 'success')
})

test('API adapter paginates changed files, honors rename origins, and writes a passing exact-head check', async () => {
  const sha = 'c'.repeat(40), calls: Array<{ url: string; init?: RequestInit }> = []
  const dispatch = async (input: URL | RequestInfo, init?: RequestInit) => {
    const url = String(input); calls.push({ url, init })
    if (url.includes('/pulls/7/files?')) return Response.json([{ filename: 'unrelated/new.txt', previous_filename: 'record/old.ts' }])
    if (url.endsWith('/pulls/7')) return Response.json({ state: 'open', base: { ref: 'main', repo: { full_name: 'tryapparent/apparent' } }, head: { sha }, changed_files: 1 })
    if (url.includes('/stream-record.yml/runs?')) return Response.json({ workflow_runs: [{ id: 91, run_attempt: 1, run_number: 8, head_sha: sha, path: '.github/workflows/stream-record.yml', status: 'completed', conclusion: 'success' }] })
    if (url.includes('/runs/91/attempts/1/jobs?')) return Response.json({ jobs: [
      { name: 'deliver / Delivery contracts', status: 'completed', conclusion: 'success' },
      { name: 'deliver / Workflow runtime budget', status: 'completed', conclusion: 'success' },
      { name: 'verify-pr / pr-stream (record, linux-x64)', status: 'completed', conclusion: 'success' },
    ] })
    if (url.includes('/check-runs?')) return Response.json({ check_runs: [] })
    if (/\/check-runs(?:\/2)?$/.test(url)) {
      const payload = JSON.parse(String(init?.body))
      if (init?.method === 'PATCH') assert(!('head_sha' in payload), 'check SHA is immutable on updates')
      else assert.equal(payload.head_sha, sha)
      return Response.json({ id: 2 })
    }
    throw new Error(`Unexpected API request ${url}`)
  }
  const event = { pull_request: { number: 7, head: { sha }, base: { ref: 'main', repo: { full_name: 'tryapparent/apparent' } } } }
  const { mkdtempSync, writeFileSync, rmSync } = await import('node:fs')
  const { tmpdir } = await import('node:os'), { join } = await import('node:path')
  const dir = mkdtempSync(join(tmpdir(), 'admission-test-')), eventPath = join(dir, 'event.json')
  writeFileSync(eventPath, JSON.stringify(event))
  try {
    await publish({ GITHUB_TOKEN: 'test', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_EVENT_PATH: eventPath }, dispatch as typeof fetch)
    const priorWrites = calls.filter(c => ['POST', 'PATCH'].includes(c.init?.method ?? '')).length
    writeFileSync(eventPath, JSON.stringify({ pull_request: { number: 7, head: { sha: 'b'.repeat(40) } } }))
    await publish({ GITHUB_TOKEN: 'test', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_EVENT_PATH: eventPath }, dispatch as typeof fetch)
    assert.equal(calls.filter(c => ['POST', 'PATCH'].includes(c.init?.method ?? '')).length, priorWrites, 'stale event cannot rewrite current candidate')
  } finally { rmSync(dir, { recursive: true, force: true }) }
  const write = calls.find(c => c.url.endsWith('/check-runs/2') && c.init?.method === 'PATCH')
  assert.ok(write)
  assert.equal(JSON.parse(String(write.init?.body)).conclusion, 'success')
  const create = calls.find(c => c.init?.method === 'POST')!
  assert.equal(JSON.parse(String(create.init?.body)).head_sha, sha)
  assert.ok(calls.some(c => c.url.includes('per_page=100&page=1')))
})

test('API uncertainty replaces a previous success with a failed receipt', async () => {
  const writes: any[] = []
  const dispatch = async (input: URL | RequestInfo, init?: RequestInit) => {
    const url = String(input)
    if (url.endsWith('/pulls/7')) return Response.json({ state: 'open', base: { ref: 'main', repo: { full_name: 'tryapparent/apparent' } }, head: { sha: 'd'.repeat(40) }, changed_files: 3001 })
    if (url.endsWith('/pulls/7/files?per_page=100&page=1')) return Response.json([])
    if (url.includes('/check-runs?')) return Response.json({ check_runs: [{ id: 2, name: 'value-stream admission', app: { slug: 'github-actions' } }] })
    if (url.endsWith('/check-runs/2')) { writes.push(JSON.parse(String(init?.body))); return Response.json({ id: 2 }) }
    throw new Error(`Unexpected API request ${url}`)
  }
  const { mkdtempSync, writeFileSync, rmSync } = await import('node:fs'), { tmpdir } = await import('node:os'), { join } = await import('node:path')
  const dir = mkdtempSync(join(tmpdir(), 'admission-test-')), eventPath = join(dir, 'event.json')
  writeFileSync(eventPath, JSON.stringify({ pull_request: { number: 7, head: { sha: 'd'.repeat(40) } } }))
  try { await assert.rejects(publish({ GITHUB_TOKEN: 'test', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_EVENT_PATH: eventPath }, dispatch as typeof fetch)) }
  finally { rmSync(dir, { recursive: true, force: true }) }
  assert.equal(writes[0].status, 'in_progress')
  assert.equal(writes.at(-1).conclusion, 'failure')
  assert(!writes.some(w => w.conclusion === 'success'))
})

test('main owner completion without a linked PR is a clean no-op', async () => {
  const dispatch = async () => { throw new Error('No API request expected') }
  const { mkdtempSync, writeFileSync, rmSync } = await import('node:fs'), { tmpdir } = await import('node:os'), { join } = await import('node:path')
  const dir = mkdtempSync(join(tmpdir(), 'admission-test-')), eventPath = join(dir, 'event.json')
  writeFileSync(eventPath, JSON.stringify({ workflow_run: { event: 'push', repository: { full_name: 'tryapparent/apparent' } } }))
  try { await publish({ GITHUB_TOKEN: 'test', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_EVENT_PATH: eventPath }, dispatch as typeof fetch) }
  finally { rmSync(dir, { recursive: true, force: true }) }
})

test('workflow completion recomputes every linked open PR instead of only the first', async () => {
  const writes: string[] = []
  const dispatch = async (input: URL | RequestInfo, init?: RequestInit) => {
    const url = String(input)
    if (/\/pulls\/\d+\?/.test(url) || /\/pulls\/\d+$/.test(url)) {
      return Response.json({ state: 'open', base: { ref: 'main', repo: { full_name: 'tryapparent/apparent' } }, head: { sha: 'f'.repeat(40) }, changed_files: 0 })
    }
    if (url.includes('/files?')) return Response.json([])
    if (url.includes('/check-runs?')) return Response.json({ check_runs: [] })
    if (url.endsWith('/check-runs') && init?.method === 'POST') { writes.push(JSON.parse(String(init.body)).head_sha); return Response.json({ id: writes.length }) }
    if (/\/check-runs\/\d+$/.test(url) && init?.method === 'PATCH') return Response.json({ id: Number(url.split('/').at(-1)) })
    throw new Error(`Unexpected API request ${url}`)
  }
  const { mkdtempSync, writeFileSync, rmSync } = await import('node:fs'), { tmpdir } = await import('node:os'), { join } = await import('node:path')
  const dir = mkdtempSync(join(tmpdir(), 'admission-test-')), eventPath = join(dir, 'event.json')
  writeFileSync(eventPath, JSON.stringify({ workflow_run: { event: 'pull_request', head_sha: 'f'.repeat(40), repository: { full_name: 'tryapparent/apparent' }, pull_requests: [{ number: 7 }, { number: 8 }] } }))
  try { await publish({ GITHUB_TOKEN: 'test', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_EVENT_PATH: eventPath }, dispatch as typeof fetch) }
  finally { rmSync(dir, { recursive: true, force: true }) }
  assert.deepEqual(writes, ['f'.repeat(40), 'f'.repeat(40)])
})
