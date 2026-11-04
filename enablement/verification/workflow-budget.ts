/** Whole-workflow execution accounting; API/metadata uncertainty fails closed. */
import { appendFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export function workflowBudgetSeconds(platform = 'linux'): number {
  if (platform === 'macos') return 900
  if (platform === 'linux') return 600
  throw new Error('Invalid workflow budget platform')
}
export const auditName = 'Workflow runtime budget'
export const optimization = 'Optimize the tests. The agent whose run exceeded the limit owns optimization and a successful rerun; do not raise or bypass the limit.'
export type Job = { name: string; status: string; conclusion: string | null; started_at: string | null; completed_at: string | null }

export function executionSeconds(jobs: Job[]): number {
  const intervals: [number, number][] = []
  for (const job of jobs) {
    // Auditing does not count as workload, including nested reusable gates.
    if (job.name === auditName || job.name.endsWith(` / ${auditName}`)) continue
    if (job.status !== 'completed') throw new Error(`Workflow job is not terminal: ${job.name}`)
    if (job.conclusion === 'skipped') continue
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

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 2) throw new Error("Usage: workflow-budget.ts (completion-time audit only)")
    const env = process.env
    const jobs = await loadJobs(env.GITHUB_API_URL!, env.GITHUB_REPOSITORY!, env.GITHUB_RUN_ID!, env.GITHUB_RUN_ATTEMPT!, env.GH_TOKEN!)
    const report = enforceBudget(jobs, env.WORKFLOW_BUDGET_PLATFORM)
    console.log(report)
    if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY, `\n${report}\n\n${optimization}\n`)

  } catch (error) {
    console.error(`::error::${String(error).replace(/[\r\n]/g, ' ')}`)
    process.exitCode = 1
  }
}
