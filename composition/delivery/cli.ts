import { admitPrCandidate } from './runtime/pr-candidate.ts'
import { appendFileSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { basename } from 'node:path'
import { fileURLToPath } from 'node:url'
import { prInputsMatch, supportsPrCandidate, assessPrior, buildOwner, catalog, inputDigest, makePlan, reusePayload, sealPayload, type Owner, type Target, workflowPathFor } from './core.ts'

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
function ghBytes(repo: string, endpoint: string): Buffer {
  return execFileSync('gh', ['api', endpoint], { encoding: 'buffer', maxBuffer: 1024 * 1024 * 1024, env: { ...process.env, GH_REPO: repo } })
}
function ownerFor(id: string): Owner {
  const result = catalog(root)
  const blocked = result.blocked.find(value => value.startsWith(`${id}:`))
  if (blocked) throw new Error(blocked)
  const owner = result.owners.find(x => x.id === id)
  if (!owner) throw new Error(`Unknown or pending delivery owner: ${id}`)
  return owner
}
function targetArg(value: string): Target { if (value !== 'linux-x64' && value !== 'macos-x64' && value !== 'web') throw new Error('Invalid target'); return value }
export function matchPrCandidateInputs(checkout: string, owner: Owner, testedSha: string, testedBase: string, landedBase: string): boolean {
  // Full main history does not necessarily contain GitHub's synthetic merge.
  // Fetch only the authenticated candidate object, without changing refs/worktree.
  if (![testedSha, testedBase, landedBase].every(sha => /^[a-f0-9]{40}$/.test(sha))) return false
  try {
    try { execFileSync('git', ['cat-file', '-e', `${testedSha}^{commit}`], { cwd: checkout, stdio: 'ignore' }) }
    catch { execFileSync('git', ['fetch', '--no-tags', '--no-write-fetch-head', 'origin', testedSha], { cwd: checkout, stdio: 'pipe', timeout: 30000 }) }
    return prInputsMatch(checkout, owner, testedSha, testedBase, landedBase)
  } catch { return false }
}
export function assertBuildWorkflowIdentity(env: NodeJS.ProcessEnv, ownerId: string) {
  if (env.GITHUB_EVENT_NAME !== 'pull_request') return
  const pullRef = /^refs\/pull\/([1-9][0-9]*)\/merge$/.exec(env.GITHUB_REF ?? '')
  const expectedPath = workflowPathFor(ownerId)
  if (env.GITHUB_REPOSITORY !== 'tryapparent/apparent' || !pullRef ||
      env.GITHUB_WORKFLOW_REF !== `tryapparent/apparent/${expectedPath}@${pullRef[0]}` ||
      (env.APPARENT_DELIVERY_WORKFLOW_PATH && env.APPARENT_DELIVERY_WORKFLOW_PATH !== expectedPath)) {
    throw new Error('PR builds require canonical owner workflow identity')
  }
}
async function main(args: string[]) {
  const env = process.env
  if (args.length === 1 && args[0] === 'plan') {
    const plan = makePlan(root)
    // Public has a dedicated producer and deployment job outside the shared diagnostic matrix.
    const matrix = { include: plan.matrix.include.filter(owner => owner.id !== 'public') }
    output({ matrix: JSON.stringify(matrix), has_ready: matrix.include.length > 0, pending: JSON.stringify(plan.pending), blocked: JSON.stringify(plan.blocked) })
    console.log(JSON.stringify({ ...plan, matrix, has_ready: matrix.include.length > 0 }, null, 2))
    if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY, `## Delivery plan\n\nMatrix targets: ${matrix.include.length}; Public uses its dedicated producer/deploy jobs.\n\nPending owners: ${plan.pending.join(', ') || 'none'}\n\nBlocked registrations: ${plan.blocked.join('; ') || 'none'}\n`)
    return
  }
  if (args.length === 3 && args[0] === 'admit') {
    const owner = ownerFor(args[1]); const target = targetArg(args[2])
    const repo = env.GITHUB_REPOSITORY ?? ''
    let result
    try {
      if (!/^[\w.-]+\/[\w.-]+$/.test(repo)) throw new Error('Invalid repository identity')
      const producerPath = workflowPathFor(owner.id)
      const workflow = gh<{ id: number; path: string }>(repo, `repos/${repo}/actions/workflows/${basename(producerPath)}`)
      if (workflow.path !== producerPath) throw new Error('Workflow API identity mismatch')
      const runsResponse = gh<{ workflow_runs: Array<{ id: number; run_attempt: number; head_sha: string; head_branch: string; event: string; conclusion: string | null }> }>(repo, `repos/${repo}/actions/workflows/${workflow.id}/runs?branch=main&event=push&per_page=100`)
      // The workflow ID was resolved from, and checked against, the exact workflow file path.
      const runs = runsResponse.workflow_runs.map(r => ({ ...r, path: producerPath }))
      result = assessPrior(root, owner, target, execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(), repo, runs,
        run => ghPages<{ name: string; status: string; conclusion: string | null }>(repo, `repos/${repo}/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs`),
        run => ghPages<{ id: number; name: string; expired: boolean }>(repo, `repos/${repo}/actions/runs/${run.id}/artifacts`))
      if (!result.reuse && supportsPrCandidate(owner.id, target)) {
        result = admitPrCandidate(endpoint => gh(repo, endpoint), execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(), result.input_digest,
          (testedSha, testedBase, landedBase) => matchPrCandidateInputs(root, owner, testedSha, testedBase, landedBase), owner.id, target) ?? result
      }
    } catch { result = { reuse: false, reason: 'GitHub identity or artifact history could not be established; rebuilding', input_digest: inputDigest(root, owner, target), prior_artifact_id: null, prior_source_sha: null, prior_run_id: null, prior_run_attempt: null } }
    output({ reuse: result.reuse, input_digest: result.input_digest, prior_artifact_id: result.prior_artifact_id ?? '', prior_source_sha: result.prior_source_sha ?? '', prior_run_id: result.prior_run_id ?? '', prior_run_attempt: result.prior_run_attempt ?? '' })
    if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY, `Artifact admission: ${result.reason}.\n`)
    console.log(JSON.stringify(result))
    return
  }
  if (args.length === 4 && args[0] === 'build') {
    const owner = ownerFor(args[1]); const target = targetArg(args[2])
    assertBuildWorkflowIdentity(env, owner.id)
    buildOwner(root, owner, target, args[3])
    return
  }
  if (args.length === 3 && args[0] === 'candidate-support') {
    output({ supported: supportsPrCandidate(args[1], args[2]) }); return
  }
  if ((args.length === 4 || args.length === 5) && args[0] === 'consume') {
    const { handoff, matchesInvocation } = await import('./consumer/handoff.ts')
    const owner = ownerFor(args[1]); const target = targetArg(args[2]); const sourceSha = args[3]
    if (!/^[0-9a-f]{40}$/.test(sourceSha)) throw new Error('Producer commit must be a full 40-character SHA')
    const repo = 'tryapparent/apparent'
    const workflow = gh<{ id: number; path: string }>(repo, `repos/${repo}/actions/workflows/${basename(workflowPathFor(owner.id))}`)
    if (workflow.path !== workflowPathFor(owner.id)) throw new Error('Producer workflow API identity mismatch')
    const runs: Array<{ id: number; run_attempt: number; head_sha: string; head_branch: string; event: string; conclusion: string | null; path: string }> = []
    let totalRuns = 0
    for (let page = 1; page <= 20; page++) {
      const response = gh<{ workflow_runs: typeof runs; total_count: number }>(repo, `repos/${repo}/actions/workflows/${workflow.id}/runs?branch=main&event=push&head_sha=${sourceSha}&per_page=100&page=${page}`)
      if (!Array.isArray(response.workflow_runs) || runs.length + response.workflow_runs.length > response.total_count) throw new Error('Incomplete producer workflow history')
      totalRuns = response.total_count
      runs.push(...response.workflow_runs.map(run => ({ ...run, path: workflowPathFor(owner.id) })))
      if (runs.length >= response.total_count || response.workflow_runs.length === 0) break
    }
    if (runs.length < totalRuns) throw new Error('Producer history exceeds bounded pagination; retry after narrowing or retaining the exact producer run')
    const api = {
      runs: async () => runs,
      jobs: async (run: { id: number; run_attempt: number }) => ghPages<{ name: string; status: string; conclusion: string | null }>(repo, `repos/${repo}/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs`),
      artifacts: async (run: { id: number }) => ghPages<{ id: number; name: string; expired: boolean }>(repo, `repos/${repo}/actions/runs/${run.id}/artifacts`),
      download: async (artifactId: number) => ghBytes(repo, `repos/${repo}/actions/artifacts/${artifactId}/zip`),
      attest: async (path: string, _owner: Owner, _target: Target, sha: string, runId: number, attempt: number) => {
        const output = execFileSync('gh', ['attestation', 'verify', path, '--repo', repo, '--signer-workflow', `${repo}/${workflowPathFor(owner.id)}`, '--source-digest', sha, '--source-ref', 'refs/heads/main', '--format', 'json'], { encoding: 'utf8', maxBuffer: 4 * 1024 * 1024, env: { ...process.env, GH_REPO: repo } })
        const attestations = JSON.parse(output) as Array<{ verificationResult?: { statement?: { subject?: Array<{ digest?: { sha256?: string } }>; predicate?: { runDetails?: { metadata?: { invocationId?: string } } } } } }>
        if (!attestations.some(entry => matchesInvocation(entry.verificationResult?.statement?.predicate?.runDetails?.metadata?.invocationId, runId, attempt))) throw new Error('Verified attestation does not identify the admitted producer run and attempt')
      },
    }
    const result = await handoff(root, owner, target, sourceSha, api, (runs, jobs, artifacts, exactDigest) => assessPrior(root, owner, target, sourceSha, repo, runs, jobs, artifacts, sourceSha, exactDigest), args[4])
    console.log(JSON.stringify(result))
    return
  }
  if (args.length === 5 && args[0] === 'candidate') {
    const owner = ownerFor(args[1]); const target = targetArg(args[2])
    assertBuildWorkflowIdentity(env, owner.id)
    const digest = inputDigest(root, owner, target)
    const receipt = sealPayload(root, owner, target, args[3], args[4], digest, env, true)
    output({ candidate_name: `pr-${owner.id}-${target}-${receipt.sourceSha}-${digest}` })
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
  throw new Error('Usage: node --experimental-strip-types composition/delivery/cli.ts plan | admit ID TARGET | consume ID TARGET PRODUCER_SHA [EMPTY_DESTINATION] | build ID TARGET OUTPUT_DIR | seal ID TARGET INPUT_DIR OUTPUT_DIR INPUT_DIGEST')
}
if (process.argv[1] === fileURLToPath(import.meta.url)) main(process.argv.slice(2)).catch(error => { console.error(`Delivery error: ${error instanceof Error ? error.message : 'operation failed'}`); process.exitCode = 1 })
