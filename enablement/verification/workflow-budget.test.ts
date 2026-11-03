import assert from 'node:assert/strict'
import test from 'node:test'
import { executionSeconds, enforceBudget, loadJobs, auditName, type Job } from './workflow-budget.ts'
import { readFileSync, readdirSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const job = (start: number, end: number, name = 'work'): Job => ({ name, status: 'completed', conclusion: 'success', started_at: new Date(start * 1000).toISOString(), completed_at: new Date(end * 1000).toISOString() })

test('union counts parallel execution once and excludes queue gaps', () => {
  assert.equal(executionSeconds([job(0, 100), job(30, 80), job(90, 200), job(500, 650)]), 350)
})
test('sequential reusable jobs share one workflow budget', () => {
  assert.throws(() => enforceBudget([job(0, 350), job(400, 700)]), /agent whose run exceeded the limit owns optimization/)
  assert.doesNotThrow(() => enforceBudget([job(0, 600)]))
  assert.equal(executionSeconds([job(0, 350, 'verify / rust'), job(400, 700, 'publish')]), 650)
  assert.equal(executionSeconds([job(0, 600)]), 600)
})
test('skip placeholders and audits do not consume the workload budget', () => {
  assert.equal(executionSeconds([{ ...job(10, 0), conclusion: 'skipped' },
    { ...job(0, 999), name: `runtime-budget / ${auditName}`, status: 'in_progress' },
    job(0, 20)]), 20)
})
test('failed and cancelled execution still counts; uncertain timestamps fail closed', () => {
  assert.equal(executionSeconds([{ ...job(0, 30), conclusion: 'failure' }, { ...job(30, 60), conclusion: 'cancelled' }]), 60)
  for (const entry of [{ ...job(0, 10), status: 'queued' }, { ...job(0, 10), started_at: null }, job(20, 10)])
    assert.throws(() => executionSeconds([entry]))
})
test('read all pages from exact run attempt, without redirecting credentials', async () => {
  const urls: string[] = []
  const request = async (url: string | URL | Request, options?: RequestInit) => {
    urls.push(String(url))
    assert.equal(options?.redirect, 'error')
    return new Response(JSON.stringify({ total_count: 2, jobs: [job(urls.length * 10, urls.length * 10 + 5)] }))
  }
  const jobs = await loadJobs('https://api.github.com', 'owner/repo', '42', '3', 'synthetic-token', request as typeof fetch)
  assert.equal(jobs.length, 2)
  assert.match(urls[0], /runs\/42\/attempts\/3\/jobs\?per_page=100&page=1$/)
  assert.match(urls[1], /page=2$/)
})
test('API denial and incomplete inventories cannot pass', async () => {
  for (const response of [new Response('', { status: 403 }), new Response(JSON.stringify({total_count: 1, jobs: []})), new Response('{}'), new Response(JSON.stringify({total_count: 0, jobs: []})), new Response(JSON.stringify({total_count: 1, jobs: [job(0, 1), job(1, 2)]}))])
    await assert.rejects(loadJobs('https://api.github.com', 'owner/repo', '42', '1', 'token', (async () => response) as typeof fetch))
})
test('every supported root workflow waits for all payload jobs before auditing', () => {
  const folder = new URL('../../.github/workflows/', import.meta.url)
  for (const name of readdirSync(folder)) {
    const text = readFileSync(new URL(name, folder), 'utf8')
    if (name === 'release-windows.yml' || name === 'runtime-budget.yml') continue
    // Only root workflows: callable platform verification is covered by caller.
    if (!/^  (push|workflow_dispatch|workflow_run|schedule):/m.test(text)) continue
    const jobs = [...text.split('jobs:\n')[1].matchAll(/^  ([\w-]+):$/gm)].map(m => m[1])
    assert.ok(jobs.includes('runtime-budget'), name)
    const gate = text.split('  runtime-budget:\n')[1]
    assert.match(gate, /    if: always\(\)/)
    assert.ok(gate.includes(`needs: [${jobs.filter(j => j !== 'runtime-budget').join(', ')}]`), name)
    assert.match(gate, /actions: read/)
    assert.match(gate, /uses: .\/.github\/actions\/runtime-budget/)
  }
})


test('watched workflow starts have no standalone watchdog to spawn', () => {
  const folder = new URL('../../.github/workflows/', import.meta.url)
  for (const name of readdirSync(folder)) {
    const text = readFileSync(new URL(name, folder), 'utf8')
    assert.doesNotMatch(text, /Workflow runtime watchdog|--watch|runtime-watchdog-/)
    assert.doesNotMatch(text, /types: \[in_progress\]/)
  }
  const audit = readFileSync(new URL('../../.github/actions/runtime-budget/action.yml', import.meta.url), 'utf8')
  assert.match(audit, /workflow-budget.ts/)
  assert.doesNotMatch(audit, /--watch|actions: write|\/cancel/)
})

test('completion audit CLI fails an overrun without cancelling the underlying workflow', () => {
  const script = fileURLToPath(new URL('./workflow-budget.ts', import.meta.url))
  for (const [seconds, expected] of [[599, 0], [600, 0], [601, 1]]) {
    const bootstrap = `globalThis.fetch = async (_url, init) => {
      if (init?.method && init.method !== 'GET') throw new Error('Unexpected workflow mutation');
      return new Response(JSON.stringify({ total_count: 1, jobs: [${JSON.stringify(job(0, seconds))}] }));
    }; process.argv = [process.execPath, ${JSON.stringify(script)}]; await import(${JSON.stringify(new URL('./workflow-budget.ts', import.meta.url).href)});`
    const result = spawnSync(process.execPath, ['--experimental-strip-types', '--input-type=module', '-e', bootstrap], {
      encoding: 'utf8', env: { ...process.env, GITHUB_API_URL: 'https://api.github.com', GITHUB_REPOSITORY: 'synthetic/repo',
        GITHUB_RUN_ID: '42', GITHUB_RUN_ATTEMPT: '1', GH_TOKEN: 'synthetic', GITHUB_STEP_SUMMARY: '' },
    })
    assert.equal(result.status, expected, result.stderr)
    if (expected) assert.match(result.stderr, /Optimize the tests/)
    assert.doesNotMatch(result.stderr, /Unexpected workflow mutation/)
  }
})

test('obsolete watch invocation fails before making any API request', () => {
  const result = spawnSync(process.execPath, ['--experimental-strip-types', fileURLToPath(new URL('./workflow-budget.ts', import.meta.url)), '--watch'], { encoding: 'utf8' })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /completion-time audit only/)
})
