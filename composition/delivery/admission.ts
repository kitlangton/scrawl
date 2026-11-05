/** Global pull request receipt for the path-filtered value-stream checks. */
import { pathToFileURL } from 'node:url'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
export type Owner = { workflow: string; paths: string[]; targets: number }
const streamFiles = readdirSync('.github/workflows').filter(file => /^stream-.*\.yml$/.test(file) && file !== 'stream-admission.yml')
function readOwners(): Owner[] {
  return streamFiles.map(file => {
    const text = readFileSync(join('.github/workflows', file), 'utf8')
    const trigger = text.split(/^  push:/m, 1)[0]
    const paths = [...trigger.matchAll(/^      - '([^']+)'\s*$/gm)].map(match => match[1])
    const matrix = text.match(/^      matrix: (\{.*\})$/m)?.[1]
    if (paths.some(path => /[!*?\[\]{}]/.test(path.replace(/\/\*\*$/, '')))) throw new Error(`Unsupported owner filter: ${file}`)
    if (!paths.length || !matrix) throw new Error(`Cannot read trusted workflow contract: ${file}`)
    const entries = JSON.parse(matrix).include
    if (!Array.isArray(entries) || !entries.length || entries.some((entry: any) => typeof entry.target !== 'string'))
      throw new Error(`Invalid owner target matrix: ${file}`)
    return { workflow: `.github/workflows/${file}`, paths, targets: entries.length }
  })
}
export const owners = readOwners()

/** GitHub path filters use glob semantics; this subset covers their literals and /** prefixes. */
export function selectedOwners(files: string[], catalog = owners): Owner[] {
  return catalog.filter(o => files.some(file => o.paths.some(pattern => pattern.endsWith('/**')
    ? file.startsWith(pattern.slice(0, -2)) : file === pattern)))
}

export type OwnerResult = { workflow: string; status: 'pending' | 'success' | 'failure' }
export function runResult(owner: Owner, runs: any[], expectedSha: string, jobsFor: (run: any) => any[]) {
  const matching = runs.filter(r => r.path === owner.workflow && r.head_sha === expectedSha)
    .sort((a, b) => b.run_number - a.run_number || b.run_attempt - a.run_attempt)
  const latest = matching[0]
  if (!latest || latest.status !== 'completed') return 'pending' as const
  const jobs = jobsFor(latest)
  const shared = jobs.filter(j => j.name === 'deliver / Delivery contracts')
  const ownerJobs = jobs.filter(j => j.name.startsWith('verify-pr / pr-stream ('))
  const audit = jobs.filter(j => j.name === 'deliver / Workflow runtime budget')
  return latest.conclusion === 'success' && shared.length === 1 && shared[0].status === 'completed' && shared[0].conclusion === 'success' &&
    audit.length === 1 && audit[0].status === 'completed' && audit[0].conclusion === 'success' &&
    ownerJobs.length === owner.targets && ownerJobs.every(j => j.status === 'completed' && j.conclusion === 'success')
    ? 'success' as const : 'failure' as const
}
export function admission(expected: Owner[], results: OwnerResult[]) {
  const byWorkflow = new Map(results.map(r => [r.workflow, r.status]))
  const states = expected.map(o => byWorkflow.get(o.workflow) ?? 'pending')
  if (states.includes('failure')) return { conclusion: 'failure' as const, summary: 'One or more selected owner checks failed.' }
  if (states.includes('pending')) return { conclusion: null, summary: 'Waiting for every selected owner check to finish successfully.' }
  return { conclusion: 'success' as const, summary: expected.length ? 'All selected owner checks passed.' : 'No path-filtered owner checks apply to this change.' }
}

// API adapter is called only by pull_request_target and trusted workflow_run events.
const api = 'https://api.github.com'
type Fetcher = typeof fetch
async function request(path: string, token: string, transport: Fetcher) {
  const response = await transport(`${api}${path}`, { headers: { authorization: `Bearer ${token}`, accept: 'application/vnd.github+json', 'x-github-api-version': '2022-11-28' } })
  if (!response.ok) throw new Error(`GitHub API ${response.status}: ${path}`)
  return response.json() as Promise<any>
}
async function pages(path: string, token: string, key: string, transport: Fetcher): Promise<any[]> {
  const all: any[] = []
  for (let page = 1; page <= 100; page++) {
    const result = await request(`${path}${path.includes('?') ? '&' : '?'}per_page=100&page=${page}`, token, transport)
    if (!Array.isArray(result[key])) throw new Error(`Invalid paginated GitHub response: ${key}`)
    all.push(...result[key]); if (result[key].length < 100) return all
  }
  throw new Error('GitHub API pagination limit exceeded')
}
async function arrayPages(path: string, token: string, transport: Fetcher): Promise<any[]> {
  const all: any[] = []
  for (let page = 1; page <= 30; page++) {
    const result = await request(`${path}${path.includes('?') ? '&' : '?'}per_page=100&page=${page}`, token, transport)
    if (!Array.isArray(result)) throw new Error('Invalid paginated GitHub array response')
    all.push(...result); if (result.length < 100) return all
  }
  throw new Error('GitHub file pagination limit exceeded')
}
export async function publish(env: Record<string, string | undefined>, transport: Fetcher = fetch) {
  const token = env.GITHUB_TOKEN!, repository = env.GITHUB_REPOSITORY!, event = JSON.parse((await import('node:fs')).readFileSync(env.GITHUB_EVENT_PATH!, 'utf8'))
  if (!token || !/^[^/]+\/[^/]+$/.test(repository)) throw new Error('Admission identity unavailable')
  if (event.workflow_run && (event.workflow_run.event !== 'pull_request' || event.workflow_run.repository?.full_name !== repository)) return
  let linked = event.pull_request ? [event.pull_request] : event.workflow_run?.pull_requests ?? []
  if (!event.pull_request && event.workflow_run && !linked.length) {
    linked = await arrayPages(`/repos/${repository}/commits/${event.workflow_run.head_sha}/pulls?state=open`, token, transport)
  }
  if (!linked.length) return // Main pushes have no PR receipt to update.
  const candidateSha = event.pull_request?.head?.sha ?? event.workflow_run?.head_sha
  if (typeof candidateSha !== 'string' || !/^[a-f0-9]{40}$/.test(candidateSha)) throw new Error('Event candidate identity unavailable')
  for (const linkedPr of linked) await publishForPr(Number(linkedPr.number), candidateSha, token, repository, transport)
}
async function publishForPr(number: number, candidateSha: string, token: string, repository: string, transport: Fetcher) {
  if (!Number.isSafeInteger(number) || number <= 0) throw new Error('Invalid pull request identity')
  const current = await request(`/repos/${repository}/pulls/${number}`, token, transport)
  if (current.state !== 'open' || current.base?.ref !== 'main' || current.base?.repo?.full_name !== repository) return
  const sha = current.head?.sha
  if (typeof sha !== 'string' || !/^[a-f0-9]{40}$/.test(sha)) throw new Error('Exact pull request head unavailable')
  if (sha !== candidateSha) return // A stale completion must not race a newer candidate's receipt.
  const checkName = 'value-stream admission'
  const checks = await pages(`/repos/${repository}/commits/${sha}/check-runs`, token, 'check_runs', transport)
  let existing = checks.filter(c => c.name === checkName && c.app?.slug === 'github-actions').sort((a,b)=>b.id-a.id)[0]
  async function writeCheck(state: { conclusion: 'success' | 'failure' | null; summary: string }) {
    const payload = { name: checkName, ...(!existing ? { head_sha: sha } : {}), status: state.conclusion ? 'completed' : 'in_progress',
    ...(state.conclusion ? { conclusion: state.conclusion } : {}), output: { title: 'Value-stream admission', summary: state.summary } }
    const url = existing ? `/repos/${repository}/check-runs/${existing.id}` : `/repos/${repository}/check-runs`
    const response = await transport(`${api}${url}`, { method: existing ? 'PATCH' : 'POST', headers: { authorization: `Bearer ${token}`, accept: 'application/vnd.github+json', 'content-type': 'application/json', 'x-github-api-version': '2022-11-28' }, body: JSON.stringify(payload) })
    if (!response.ok) throw new Error(`Could not publish admission check: ${response.status}`)
    existing = await response.json()
  }
  await writeCheck({ conclusion: null, summary: 'Checking affected owner evidence.' })
  try {
    if (!Number.isInteger(current.changed_files) || current.changed_files > 3000) throw new Error('Changed-file API limit makes owner selection uncertain')
    const files = await arrayPages(`/repos/${repository}/pulls/${number}/files`, token, transport)
    if (files.length !== current.changed_files) throw new Error('Incomplete changed-file inventory')
    const changedPaths = files.flatMap(f => [f.filename, ...(typeof f.previous_filename === 'string' ? [f.previous_filename] : [])])
    const expected = selectedOwners(changedPaths)
    const results: OwnerResult[] = []
    const cache = new Map<string, any[]>()
    for (const o of expected) {
      const runs = await pages(`/repos/${repository}/actions/workflows/${encodeURIComponent(o.workflow.split('/').at(-1)!)}/runs?event=pull_request&head_sha=${sha}`, token, 'workflow_runs', transport)
      const exact = runs.filter(r => r.head_sha === sha)
      const latest = exact.filter(r => r.path === o.workflow).sort((a, b) => b.run_number - a.run_number || b.run_attempt - a.run_attempt)[0]
      if (latest?.status === 'completed') cache.set(`${latest.id}/${latest.run_attempt}`, await pages(`/repos/${repository}/actions/runs/${latest.id}/attempts/${latest.run_attempt}/jobs`, token, 'jobs', transport))
      results.push({ workflow: o.workflow, status: runResult(o, runs, sha, run => cache.get(`${run.id}/${run.run_attempt}`) ?? []) })
    }
    const state = admission(expected, results)
    await writeCheck(state)
  } catch (error) {
    await writeCheck({ conclusion: 'failure', summary: 'Admission could not establish complete verification evidence.' })
    throw error
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  publish(process.env).catch(error => { console.error(error); process.exitCode = 1 })
}
