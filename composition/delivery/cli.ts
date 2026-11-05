import { appendFileSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { basename } from 'node:path'
import { fileURLToPath } from 'node:url'
import { assessPrior, buildOwner, catalog, inputDigest, makePlan, reusePayload, sealPayload, type Owner, type Target, workflowPath } from './core.ts'

const root = execFileSync('git', ['rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim()
function output(values: Record<string, string | boolean | number | null>) {
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, Object.entries(values).map(([k, v]) => `${k}=${String(v)}\n`).join(''))
}
function gh<T>(repo: string, endpoint: string): T {
  const raw = execFileSync('gh', ['api', endpoint], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env, GH_REPO: repo } })
  return JSON.parse(raw) as T
}
function ghPages<T>(repo: string, endpoint: string): T[] {
  const all: T[] = []
  for (let page = 1; page <= 20; page++) {
    const body = gh<{ total_count: number; jobs?: T[]; artifacts?: T[] }>(repo, `${endpoint}${endpoint.includes('?') ? '&' : '?'}per_page=100&page=${page}`)
    const rows = (body.jobs ?? body.artifacts) as T[] | undefined
    if (!Number.isInteger(body.total_count) || !rows || all.length + rows.length > body.total_count) throw new Error('Incomplete GitHub history response')
    all.push(...rows)
    if (all.length === body.total_count) return all
    if (!rows.length) throw new Error('Incomplete GitHub history response')
  }
  throw new Error('GitHub history exceeds bounded pagination')
}
function ownerFor(id: string): Owner {
  const result = catalog(root)
  const blocked = result.blocked.find(value => value.startsWith(`${id}:`))
  if (blocked) throw new Error(blocked)
  const owner = result.owners.find(x => x.id === id)
  if (!owner) throw new Error(`Unknown or pending delivery owner: ${id}`)
  return owner
}
function targetArg(value: string): Target { if (value !== 'linux-x64' && value !== 'macos-x64') throw new Error('Invalid target'); return value }
async function main(args: string[]) {
  const env = process.env
  if (args.length === 1 && args[0] === 'plan') {
    const plan = makePlan(root)
    output({ matrix: JSON.stringify(plan.matrix), has_ready: plan.has_ready, pending: JSON.stringify(plan.pending), blocked: JSON.stringify(plan.blocked) })
    console.log(JSON.stringify(plan, null, 2))
    if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY, `## Delivery plan\n\nReady targets: ${plan.matrix.include.length}\n\nPending owners: ${plan.pending.join(', ') || 'none'}\n\nBlocked registrations: ${plan.blocked.join('; ') || 'none'}\n`)
    return
  }
  if (args.length === 3 && args[0] === 'admit') {
    const owner = ownerFor(args[1]); const target = targetArg(args[2])
    const repo = env.GITHUB_REPOSITORY ?? ''
    let result
    try {
      if (!/^[\w.-]+\/[\w.-]+$/.test(repo)) throw new Error('Invalid repository identity')
      const workflow = gh<{ id: number; path: string }>(repo, `repos/${repo}/actions/workflows/${basename(workflowPath)}`)
      if (workflow.path !== workflowPath) throw new Error('Workflow API identity mismatch')
      const runsResponse = gh<{ workflow_runs: Array<{ id: number; run_attempt: number; head_sha: string; head_branch: string; event: string; conclusion: string | null }> }>(repo, `repos/${repo}/actions/workflows/${workflow.id}/runs?branch=main&event=push&per_page=100`)
      // The workflow ID was resolved from, and checked against, the exact workflow file path.
      const runs = runsResponse.workflow_runs.map(r => ({ ...r, path: workflowPath }))
      result = assessPrior(root, owner, target, execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(), repo, runs,
        run => ghPages<{ name: string; status: string; conclusion: string | null }>(repo, `repos/${repo}/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs`),
        run => ghPages<{ id: number; name: string; expired: boolean }>(repo, `repos/${repo}/actions/runs/${run.id}/artifacts`))
    } catch { result = { reuse: false, reason: 'GitHub identity or artifact history could not be established; rebuilding', input_digest: inputDigest(root, owner, target), prior_artifact_id: null, prior_source_sha: null, prior_run_id: null, prior_run_attempt: null } }
    output({ reuse: result.reuse, input_digest: result.input_digest, prior_artifact_id: result.prior_artifact_id ?? '', prior_source_sha: result.prior_source_sha ?? '', prior_run_id: result.prior_run_id ?? '', prior_run_attempt: result.prior_run_attempt ?? '' })
    console.log(JSON.stringify(result))
    return
  }
  if (args.length === 4 && args[0] === 'build') {
    const owner = ownerFor(args[1]); const target = targetArg(args[2])
    buildOwner(root, owner, target, args[3])
    return
  }
  if (args.length === 6 && args[0] === 'seal') {
    const owner = ownerFor(args[1]); const target = targetArg(args[2])
    const receipt = sealPayload(root, owner, target, args[3], args[4], args[5])
    console.log(JSON.stringify(receipt))
    return
  }
  if (args.length === 10 && args[0] === 'reuse') {
    console.log(JSON.stringify(reusePayload(root, ownerFor(args[1]), targetArg(args[2]), args[3], args[4], args[5], args[6], args[7], args[8], args[9])))
    return
  }
  throw new Error('Usage: node --experimental-strip-types composition/delivery/cli.ts plan | admit ID TARGET | build ID TARGET OUTPUT_DIR | seal ID TARGET INPUT_DIR OUTPUT_DIR INPUT_DIGEST')
}
if (process.argv[1] === fileURLToPath(import.meta.url)) main(process.argv.slice(2)).catch(error => { console.error(`Delivery error: ${error instanceof Error ? error.message : 'operation failed'}`); process.exitCode = 1 })
