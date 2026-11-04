/** Whole-workflow execution accounting; API/metadata uncertainty fails closed. */
import { appendFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export function workflowBudgetSeconds(platform = 'linux'): number {
  // Temporary release exception explicitly authorized by Simon; restore after delivery.
  if (platform === 'macos') return 3600
  if (platform === 'linux') return 900
  if (platform === 'web') return 600
  throw new Error('Invalid workflow budget platform')
}
export const auditName = 'Workflow runtime budget'
export const optimization = 'Optimize the tests. The agent whose run exceeded the limit owns optimization and a successful rerun; do not raise or bypass the limit.'
export type Job = { name: string; status: string; conclusion: string | null; started_at: string | null; completed_at: string | null;
  runner_id?: number | null; runner_name?: string | null; steps?: unknown[] }

export function executionSeconds(jobs: Job[]): number {
  const intervals: [number, number][] = []
  for (const job of jobs) {
    // Auditing does not count as workload, including nested reusable gates.
    if (job.name === auditName || job.name.endsWith(` / ${auditName}`)) continue
    if (job.status !== 'completed') throw new Error(`Workflow job is not terminal: ${job.name}`)
    if (job.conclusion === 'skipped') continue
    // GitHub gives unacquired, cancelled jobs queue timestamps in started_at.
    // Require all three explicit no-execution fields; missing metadata still counts.
    if (job.conclusion === 'cancelled' && job.runner_id === 0 && job.runner_name === '' &&
        Array.isArray(job.steps) && job.steps.length === 0) continue
    const start = Date.parse(job.started_at ?? '')
    const end = Date.parse(job.completed_at ?? '')
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
                               token: string, request: typeof fetch = fetch): Promise<Job[]> {
  if (!/^https:\/\//.test(api) || !/^[\w.-]+\/[\w.-]+$/.test(repository) ||
      !/^[1-9][0-9]*$/.test(run) || !/^[1-9][0-9]*$/.test(attempt) || !token)
    throw new Error('Missing or invalid workflow budget API identity')
  const jobs: Job[] = []
  for (let page = 1; ; page++) {
    const response = await request(`${api}/repos/${repository}/actions/runs/${run}/attempts/${attempt}/jobs?per_page=100&page=${page}`, {
      headers: { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json' },
      signal: AbortSignal.timeout(30_000), redirect: 'error',
    })
    if (!response.ok) throw new Error(`Cannot establish workflow execution budget: HTTP ${response.status}`)
    const body = await response.json() as { total_count: number; jobs: Job[] }
    if (!Number.isInteger(body.total_count) || body.total_count < 1 || !Array.isArray(body.jobs) || jobs.length + body.jobs.length > body.total_count) throw new Error('Invalid jobs response')
    jobs.push(...body.jobs)
    if (jobs.length >= body.total_count) return jobs
    if (body.jobs.length === 0 || page >= 100) throw new Error('Incomplete workflow job inventory')
  }
}

export function enforceBudget(jobs: Job[], platform = 'linux'): string {
  const budgetSeconds = workflowBudgetSeconds(platform)
  const seconds = executionSeconds(jobs)
  const report = `Workflow workload execution: ${seconds.toFixed(1)}s / ${budgetSeconds}s (parallel jobs counted once; runner-only waiting and budget audits excluded).`
  if (seconds > budgetSeconds) throw new Error(`${report} ${optimization}`)
  return report
}

/** Reserve a bounded publication job against completed work before it can mutate durable releases. */
export function enforcePublicationReservation(jobs: Job[], publisherName: string, reservedSeconds: number,
                                             expectedJobNames: string[], platform = 'web'): string {
  const budgetSeconds = workflowBudgetSeconds(platform)
  if (!publisherName || !Number.isSafeInteger(reservedSeconds) || reservedSeconds <= 0 || reservedSeconds > budgetSeconds ||
      !Array.isArray(expectedJobNames) || !expectedJobNames.length || expectedJobNames.some(name => typeof name !== 'string' || !name) ||
      new Set(expectedJobNames).size !== expectedJobNames.length)
    throw new Error('Invalid publication reservation')
  const actualNames = jobs.map(job => job.name).sort()
  const expectedNames = [...expectedJobNames].sort()
  if (actualNames.length !== expectedNames.length || actualNames.some((name, index) => name !== expectedNames[index]))
    throw new Error('Workflow job inventory does not match the expected publication attempt')
  const publishers = jobs.filter(job => job.name === publisherName)
  if (publishers.length !== 1) throw new Error('Missing or ambiguous publication job identity')
  const publisher = publishers[0]
  if (publisher.status !== 'in_progress' || publisher.conclusion !== null ||
      !Number.isFinite(Date.parse(publisher.started_at ?? '')) || publisher.completed_at !== null)
    throw new Error('Publication job is not the active job in this workflow attempt')

  const audits = jobs.filter(job => job.name === auditName || job.name.endsWith(` / ${auditName}`))
  if (audits.length !== 1 || !['queued', 'pending'].includes(audits[0].status) || audits[0].conclusion !== null)
    throw new Error('Runtime audit job is missing, ambiguous, or not pending')

  const completed = jobs.filter(job => job !== publisher && job !== audits[0])
  const observedSeconds = executionSeconds(completed)
  const reservedTotal = observedSeconds + reservedSeconds
  const report = `Publication reservation: ${observedSeconds.toFixed(1)}s completed + ${reservedSeconds}s reserved = ${reservedTotal.toFixed(1)}s / ${budgetSeconds}s (parallel jobs counted once; audit and runner-only waiting excluded).`
  if (reservedTotal > budgetSeconds) throw new Error(`${report} ${optimization}`)
  return report
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const mode = process.argv[2] ?? 'audit'
    if (process.argv.length > 3 || !['audit', '--publication-preflight'].includes(mode))
      throw new Error('Usage: workflow-budget.ts (completion-time audit only; optional --publication-preflight)')
    const env = process.env
    const jobs = await loadJobs(env.GITHUB_API_URL!, env.GITHUB_REPOSITORY!, env.GITHUB_RUN_ID!, env.GITHUB_RUN_ATTEMPT!, env.GH_TOKEN!)
    let report: string
    if (mode === '--publication-preflight') {
      if (env.GITHUB_JOB !== 'recipient-release' || env.GITHUB_WORKFLOW !== 'Connect producer' ||
          env.GITHUB_REPOSITORY !== 'tryapparent/apparent' || env.GITHUB_REF !== 'refs/heads/main')
        throw new Error('Publication preflight must run in the canonical Connect producer on main')
      const reserve = Number(env.WORKFLOW_BUDGET_RESERVE_SECONDS)
      let expectedNames: string[]
      try { expectedNames = JSON.parse(env.WORKFLOW_BUDGET_EXPECTED_JOB_NAMES ?? '') }
      catch { throw new Error('Missing or invalid expected workflow job inventory') }
      report = enforcePublicationReservation(jobs, env.WORKFLOW_BUDGET_PUBLISHER_NAME!, reserve, expectedNames, env.WORKFLOW_BUDGET_PLATFORM)
    } else report = enforceBudget(jobs, env.WORKFLOW_BUDGET_PLATFORM)
    console.log(report)
    if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY, `\n${report}\n\n${optimization}\n`)

  } catch (error) {
    console.error(`::error::${String(error).replace(/[\r\n]/g, ' ')}`)
    process.exitCode = 1
  }
}
