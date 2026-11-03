import assert from 'node:assert/strict'
import test from 'node:test'
import { executionSeconds, enforceBudget, loadJobs, auditName, watchWorkflow, watchedPaths, type Job } from './workflow-budget.ts'
import { readFileSync, readdirSync } from 'node:fs'

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


const identity = { api: 'https://api.github.com', repository: 'owner/repo', run: '42', attempt: '3', token: 'synthetic-token' }
const metadata = (overrides = {}) => ({ id: 42, run_attempt: 3, status: 'in_progress', path: '.github/workflows/verification.yml', repository: { full_name: 'owner/repo' }, head_repository: { full_name: 'owner/repo' }, ...overrides })
const live = (start: number) => ({ ...job(start, start), status: 'in_progress', conclusion: null, completed_at: null })
function harness(states: object[], inventories: Job[][], options: { cancelStatus?: number; now?: number } = {}) {
  const mutations: string[] = []
  const reports: string[] = []
  const sleeps: number[] = []
  let clock = options.now ?? 600_000
  const request = async (url: string | URL | Request, init?: RequestInit) => {
    assert.equal(init?.redirect, 'error')
    if (init?.method === 'POST') {
      mutations.push(String(url))
      return new Response(null, { status: options.cancelStatus ?? 202 })
    }
    if (String(url).includes('/jobs?')) {
      const jobs = inventories.shift()
      assert.ok(jobs, 'unexpected jobs request')
      return new Response(JSON.stringify({ total_count: jobs.length, jobs }))
    }
    const next = states.shift()
    assert.ok(next, 'unexpected target metadata request')
    return new Response(JSON.stringify(next))
  }
  return { mutations, reports, sleeps, options: { request: request as typeof fetch, now: () => clock,
    sleep: async (ms: number) => { sleeps.push(ms); clock += ms }, report: (message: string) => reports.push(message) } }
}
test('live execution includes running jobs once and excludes queue-only time', () => {
  assert.equal(executionSeconds([job(0, 100), live(50), { ...live(0), status: 'queued' }], 200_000), 200)
  assert.equal(executionSeconds([{ ...live(0), status: 'queued' }], 999_000), 0)
})
test('watchdog requests normal cancellation at the exact shared deadline', async () => {
  const h = harness([metadata(), metadata()], [[job(0, 300), live(300), live(500)]])
  assert.equal(await watchWorkflow(identity, h.options), 'cancelled')
  assert.deepEqual(h.mutations, ['https://api.github.com/repos/owner/repo/actions/runs/42/cancel'])
  assert.equal(h.sleeps.length, 0)
  assert.ok(h.reports.some(r => r.includes('Optimize the tests')))
  assert.ok(h.reports.some(r => r.includes('Cancellation accepted')))
})
test('watchdog keeps waiting through allocation gaps and cancels when work resumes', async () => {
  const h = harness([metadata(), metadata()], [[], [live(5)]])
  assert.equal(await watchWorkflow(identity, h.options), 'cancelled')
  assert.deepEqual(h.sleeps, [60_000])
})
test('superseded attempts and completed targets are never cancelled', async () => {
  for (const state of [metadata({run_attempt: 4}), metadata({status: 'completed'})]) {
    const h = harness([state], [])
    assert.ok(['completed', 'superseded'].includes(await watchWorkflow(identity, h.options)))
    assert.deepEqual(h.mutations, [])
  }
  const h = harness([metadata(), metadata({run_attempt: 4})], [[live(0)]])
  assert.equal(await watchWorkflow(identity, h.options), 'superseded')
  assert.deepEqual(h.mutations, [])
})
test('untrusted run identity, fork and workflow cannot acquire cancellation authority', async () => {
  for (const patch of [{id: 43}, {repository: {full_name: 'other/repo'}}, {head_repository: {full_name: 'fork/repo'}}, {path: '.github/workflows/runtime-budget.yml'}]) {
    const h = harness([metadata(patch)], [])
    await assert.rejects(watchWorkflow(identity, h.options), /Untrusted/)
    assert.deepEqual(h.mutations, [])
  }
})
test('cancellation races are accepted only when target is already complete', async () => {
  const h = harness([metadata(), metadata(), metadata({status: 'completed'})], [[live(0)]], {cancelStatus: 409})
  assert.equal(await watchWorkflow(identity, h.options), 'completed')
  const rejected = harness([metadata(), metadata()], [[live(0)]], {cancelStatus: 403})
  await assert.rejects(watchWorkflow(identity, rejected.options), /not accepted: HTTP 403/)
})
test('controller observes every supported root workflow without observing itself or granting workload write access', () => {
  const folder = new URL('../../.github/workflows/', import.meta.url)
  const controller = readFileSync(new URL('runtime-budget.yml', folder), 'utf8')
  assert.match(controller, /types: \[in_progress\]/)
  assert.match(controller, /actions: write/)
  assert.match(controller, /ref: \$\{\{ github.workflow_sha \}\}/)
  assert.doesNotMatch(controller, /github.event.workflow_run.head_sha|workflow_call:|force-cancel/)
  assert.ok(!watchedPaths.has('.github/workflows/runtime-budget.yml'))
  for (const name of readdirSync(folder)) {
    if (name === 'runtime-budget.yml' || name === 'release-windows.yml') continue
    const text = readFileSync(new URL(name, folder), 'utf8')
    if (!/^  (push|workflow_dispatch|workflow_run|schedule):/m.test(text)) continue
    assert.ok(watchedPaths.has(`.github/workflows/${name}`), name)
    const workflowName = text.match(/^name: (.+)$/m)![1]
    assert.ok(controller.includes(`      - ${workflowName}\n`), name)
  }
})

test('transient read failures retry within a bound without acquiring mutation authority', async () => {
  let calls = 0
  const waits: number[] = []
  const result = await watchWorkflow(identity, {
    request: (async (_url, init) => {
      assert.notEqual(init?.method, 'POST')
      if (++calls < 3) return new Response(null, {status: 503})
      return new Response(JSON.stringify(metadata({status: 'completed'})))
    }) as typeof fetch,
    sleep: async ms => { waits.push(ms) }, report: () => {},
  })
  assert.equal(result, 'completed')
  assert.deepEqual(waits, [500, 1000])
  calls = 0
  await assert.rejects(watchWorkflow(identity, {
    request: (async () => { calls++; return new Response(null, {status: 503}) }) as typeof fetch,
    sleep: async () => {}, report: () => {},
  }), /HTTP 503/)
  assert.equal(calls, 3)
})

test('rate-limited observer honors reset, resumes, and cancels at the deadline', async () => {
  const h = harness([metadata(), metadata()], [[live(0)]])
  const original = h.options.request
  let limited = false
  const request = async (url: string | URL | Request, init?: RequestInit) => {
    if (!limited && String(url).includes('/jobs?')) {
      limited = true
      return new Response(null, { status: 403, headers: { 'x-ratelimit-remaining': '0', 'x-ratelimit-reset': '601' } })
    }
    return original(url, init)
  }
  assert.equal(await watchWorkflow(identity, { ...h.options, request: request as typeof fetch }), 'cancelled')
  assert.deepEqual(h.sleeps, [2000])
  assert.equal(h.mutations.length, 1)
})
test('rate-limited cancellation rechecks attempt before retrying a mutation', async () => {
  const h = harness([metadata(), metadata(), metadata({ run_attempt: 4 })], [[live(0)]], { cancelStatus: 429 })
  assert.equal(await watchWorkflow(identity, h.options), 'superseded')
  assert.deepEqual(h.sleeps, [60_000])
  assert.equal(h.mutations.length, 1)
})

test('hour-long queue wait remains cheap and never consumes the execution budget', async () => {
  let clock = 0
  let requests = 0
  const result = await watchWorkflow(identity, {
    request: (async (url, init) => {
      requests++
      assert.notEqual(init?.method, 'POST')
      if (String(url).includes('/jobs?'))
        return new Response(JSON.stringify({ total_count: 1, jobs: [{ ...live(0), status: 'queued' }] }))
      return new Response(JSON.stringify(metadata({status: clock >= 3600_000 ? 'completed' : 'queued'})))
    }) as typeof fetch,
    now: () => clock, sleep: async ms => { clock += ms }, report: () => {},
  })
  assert.equal(result, 'completed')
  assert.ok(requests < 100, `${requests} requests during an hour-long wait`)
})
