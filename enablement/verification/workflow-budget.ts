/** Whole-workflow execution accounting; API/metadata uncertainty fails closed. */
import { appendFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export const budgetSeconds = 600
export const auditName = 'Workflow runtime budget'
export const optimization = 'Optimize the tests. The agent whose run exceeded the limit owns optimization and a successful rerun; do not raise or bypass the limit.'
export type Job = { name: string; status: string; conclusion: string | null; started_at: string | null; completed_at: string | null }

export function executionSeconds(jobs: Job[], now?: number): number {
  const intervals: [number, number][] = []
  for (const job of jobs) {
    // Auditing does not count as workload, including nested reusable gates.
    if (job.name === auditName || job.name.endsWith(` / ${auditName}`)) continue
    if (job.conclusion === 'skipped') continue
    if (now !== undefined && ['queued', 'waiting', 'pending', 'requested'].includes(job.status)) continue
    if (job.status !== 'completed' && !(now !== undefined && job.status === 'in_progress'))
      throw new Error(`Workflow job is not terminal: ${job.name}`)
    const start = Date.parse(job.started_at ?? '')
    const end = job.status === 'in_progress' ? now! : Date.parse(job.completed_at ?? '')
    if (!Number.isFinite(start) || !Number.isFinite(end) || end < start)
      throw new Error(`Invalid execution timestamps: ${job.name}`)
    intervals.push([start, end])
  }
  intervals.sort((a, b) => a[0] - b[0])
  let total = 0
  let previousEnd = -Infinity
  for (const [start, end] of intervals) {
    total += Math.max(0, end - Math.max(start, previousEnd))
    previousEnd = Math.max(previousEnd, end)
  }
  return total / 1000
}

export async function loadJobs(api: string, repository: string, run: string, attempt: string,
                               token: string, request: typeof fetch = fetch, allowEmpty = false): Promise<Job[]> {
  if (!/^https:\/\//.test(api) || !/^[\w.-]+\/[\w.-]+$/.test(repository) ||
      !/^\d+$/.test(run) || !/^\d+$/.test(attempt) || !token)
    throw new Error('Missing or invalid workflow budget API identity')
  const jobs: Job[] = []
  for (let page = 1; ; page++) {
    const response = await request(`${api}/repos/${repository}/actions/runs/${run}/attempts/${attempt}/jobs?per_page=100&page=${page}`, {
      headers: { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json' },
      signal: AbortSignal.timeout(30_000), redirect: 'error',
    })
    if (!response.ok) throw new Error(`Cannot establish workflow execution budget: HTTP ${response.status}`)
    const body = await response.json() as { total_count: number; jobs: Job[] }
    if (!Number.isInteger(body.total_count) || body.total_count < (allowEmpty ? 0 : 1) || !Array.isArray(body.jobs) || jobs.length + body.jobs.length > body.total_count) throw new Error('Invalid jobs response')
    jobs.push(...body.jobs)
    if (jobs.length >= body.total_count) return jobs
    if (body.jobs.length === 0 || page >= 100) throw new Error('Incomplete workflow job inventory')
  }
}

export function enforceBudget(jobs: Job[]): string {
  const seconds = executionSeconds(jobs)
  const report = `Workflow workload execution: ${seconds.toFixed(1)}s / ${budgetSeconds}s (parallel jobs counted once; runner-only waiting and budget audits excluded).`
  if (seconds > budgetSeconds) throw new Error(`${report} ${optimization}`)
  return report
}

export const watchedPaths = new Set([
  '.github/workflows/verification.yml', '.github/workflows/verification-mac.yml',
  '.github/workflows/native-proof-mac.yml', '.github/workflows/linux-updater-harness.yml',
  '.github/workflows/release-linux.yml', '.github/workflows/release-mac-preview.yml',
  '.github/workflows/web-delivery.yml',
])
type WatchIdentity = { api: string; repository: string; run: string; attempt: string; token: string }
type Run = { id: number; run_attempt: number; status: string; path: string;
  repository: { full_name: string }; head_repository: { full_name: string } }

export async function watchWorkflow(identity: WatchIdentity, options: {
  request?: typeof fetch; now?: () => number; sleep?: (ms: number) => Promise<void>;
  report?: (message: string) => void
} = {}): Promise<'completed' | 'superseded' | 'cancelled'> {
  const { api, repository, run, attempt, token } = identity
  // Validate identity before any request, including a cancellation request.
  if (!/^https:\/\//.test(api) || !/^[\w.-]+\/[\w.-]+$/.test(repository) ||
      !/^\d+$/.test(run) || !/^\d+$/.test(attempt) || !token)
    throw new Error('Invalid watchdog identity')
  const request = options.request ?? fetch
  const now = options.now ?? Date.now
  const sleep = options.sleep ?? (ms => new Promise(resolve => setTimeout(resolve, ms)))
  const report = options.report ?? console.log
  function rateDelay(response: Response): number | null {
    const after = response.headers.get('retry-after')
    const reset = response.headers.get('x-ratelimit-reset')
    if (response.status !== 429 && !(response.status === 403 &&
        (after !== null || response.headers.get('x-ratelimit-remaining') === '0'))) return null
    if (after !== null) {
      const seconds = Number(after)
      const delay = Number.isFinite(seconds) ? seconds * 1000 : Date.parse(after) - now()
      if (Number.isFinite(delay)) return Math.max(1000, delay)
    }
    if (reset !== null && Number.isFinite(Number(reset))) return Math.max(1000, Number(reset) * 1000 - now() + 1000)
    return 60_000
  }
  const read: typeof fetch = async (url, init) => {
    let rateRetries = 0
    for (let retry = 0; ; retry++) {
      try {
        const response = await request(url, { ...init, signal: AbortSignal.timeout(30_000) })
        const delay = rateDelay(response)
        if (delay !== null && rateRetries++ < 12) {
          report(`GitHub API rate limited; resume monitoring after ${Math.ceil(delay / 1000)}s.`)
          await sleep(delay)
          retry--
          continue
        }
        if (retry === 2 || (response.status !== 429 && response.status < 500)) return response
      } catch (error) {
        if (retry >= 2) throw error
      }
      await sleep(500 * (retry + 1))
    }
  }
  const url = `${api}/repos/${repository}/actions/runs/${run}`
  const headers = { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json' }
  async function metadata(): Promise<Run> {
    const response = await read(url, { headers, redirect: 'error', signal: AbortSignal.timeout(30_000) })
    if (!response.ok) throw new Error(`Cannot establish watchdog target: HTTP ${response.status}`)
    const body = await response.json() as Run
    if (String(body.id) !== run || !Number.isInteger(body.run_attempt) ||
        body.repository?.full_name !== repository || body.head_repository?.full_name !== repository ||
        !watchedPaths.has(body.path?.split('@')[0]) ||
        !['queued', 'in_progress', 'completed', 'waiting', 'pending', 'requested'].includes(body.status)) throw new Error('Untrusted watchdog target')
    return body
  }
  report(`Watching workflow ${run}, attempt ${attempt}; cancellation at ${budgetSeconds}s. ${optimization}`)
  let lastMetadata = -Infinity
  let state: Run
  for (;;) {
    // Job snapshots carry the clock; metadata is only needed periodically and
    // immediately before mutation. Idle observers must not exhaust API quota.
    if (now() - lastMetadata >= 120_000) {
      state = await metadata()
      lastMetadata = now()
      if (String(state.run_attempt) !== attempt) return 'superseded'
      if (state.status === 'completed') return 'completed'
    }
    const jobs = await loadJobs(api, repository, run, attempt, token, read, true)
    const seconds = executionSeconds(jobs, now())
    if (seconds >= budgetSeconds) {
      for (let retry = 0; ; retry++) {
        // Recheck immediately before every mutation, including after backoff.
        const current = await metadata()
        if (String(current.run_attempt) !== attempt) return 'superseded'
        if (current.status === 'completed') return 'completed'
        report(`::error::Workflow ${run} reached ${seconds.toFixed(1)}s / ${budgetSeconds}s. ${optimization}`)
        let response: Response
        try {
          response = await request(`${url}/cancel`, { method: 'POST', headers, redirect: 'error', signal: AbortSignal.timeout(30_000) })
        } catch (error) {
          if (retry >= 2) throw error
          await sleep(500 * (retry + 1))
          continue
        }
        if (response.status === 202) {
          report(`Cancellation accepted for workflow ${run}, attempt ${attempt}; normal GitHub cleanup grace applies.`)
          return 'cancelled'
        }
        const delay = rateDelay(response)
        if (delay !== null && retry < 12) {
          await sleep(delay)
          continue
        }
        if (response.status >= 500 && retry < 2) {
          await sleep(500 * (retry + 1))
          continue
        }
        // A run can finish between the last GET and POST.
        if (response.status === 409 && (await metadata()).status === 'completed') return 'completed'
        throw new Error(`Workflow cancellation was not accepted: HTTP ${response.status}`)
      }
    }
    const running = jobs.some(job => job.status === 'in_progress' &&
      job.name !== auditName && !job.name.endsWith(` / ${auditName}`))
    await sleep(running ? Math.min(15_000, Math.max(100, (budgetSeconds - seconds) * 1000)) : 60_000)
  }

}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const env = process.env
    if (process.argv[2] === '--watch') {
      const result = await watchWorkflow({ api: env.GITHUB_API_URL!, repository: env.GITHUB_REPOSITORY!,
        run: env.BUDGET_RUN_ID!, attempt: env.BUDGET_RUN_ATTEMPT!, token: env.GH_TOKEN! })
      if (result === 'cancelled') process.exitCode = 1
    } else {
      const jobs = await loadJobs(env.GITHUB_API_URL!, env.GITHUB_REPOSITORY!, env.GITHUB_RUN_ID!, env.GITHUB_RUN_ATTEMPT!, env.GH_TOKEN!)
      const report = enforceBudget(jobs)
      console.log(report)
      if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY, `\n${report}\n\n${optimization}\n`)
    }
  } catch (error) {
    console.error(`::error::${String(error).replace(/[\r\n]/g, ' ')}`)
    process.exitCode = 1
  }
}
