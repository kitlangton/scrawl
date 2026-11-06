/** Conservative owner/target promotion. GitHub metadata authenticates source identity;
 * candidate ZIP receipts still undergo the normal byte/digest/attempt validation. */
import { supportsPrCandidate, workflowPathFor, type AdmissionResult } from '../core.ts'

const repository = 'tryapparent/apparent'
type Api = <T>(endpoint: string) => T
export function admitPrCandidate(api: Api, currentSha: string, digest: string, matchesInputs: (testedSha: string, testedBase: string, landedBase: string) => boolean, ownerId = 'engine', target = 'linux-x64'): AdmissionResult | undefined {
  if (!supportsPrCandidate(ownerId, target)) return
  const workflow = workflowPathFor(ownerId)
  const pulls = api<Array<{ number: number; merged_at: string | null; merge_commit_sha: string; head: { sha: string; repo: { full_name: string } | null } }>>(`repos/${repository}/commits/${currentSha}/pulls?per_page=100`)
  const landed = api<{ sha: string; parents: Array<{ sha: string }> }>(`repos/${repository}/git/commits/${currentSha}`)
  if (landed.sha !== currentSha || !landed.parents.length) return
  for (const pull of pulls) {
    if (!pull.merged_at || pull.merge_commit_sha !== currentSha || pull.head.repo?.full_name !== repository || !Number.isSafeInteger(pull.number)) continue
    const { workflow_runs: runs } = api<{ workflow_runs: Array<{ id: number; run_attempt: number; event: string; path: string; conclusion: string; head_sha: string; head_repository: { full_name: string }; pull_requests: Array<{ number: number; head: { sha: string }; base: { sha: string } }> }> }>(`repos/${repository}/actions/workflows/${workflow.split('/').at(-1)}/runs?event=pull_request&head_sha=${pull.head.sha}&per_page=100`)
    for (const run of runs) {
      if (run.event !== 'pull_request' || run.path !== workflow || run.conclusion !== 'success' || run.head_sha !== pull.head.sha || run.head_repository?.full_name !== repository || !Number.isSafeInteger(run.id) || run.id <= 0 || !Number.isSafeInteger(run.run_attempt) || run.run_attempt <= 0) continue
      const { jobs, total_count: jobCount } = api<{ total_count: number; jobs: Array<{ name: string; status: string; conclusion: string }> }>(`repos/${repository}/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs?per_page=100`)
      if (jobCount !== jobs.length) continue
      const required = ['deliver / Delivery contracts', `verify-pr / pr-stream (${ownerId}, ${target})`, 'deliver / Workflow runtime budget']
      if (!required.every(name => jobs.filter(j => j.name === name && j.status === 'completed' && j.conclusion === 'success').length === 1)) continue
      const { artifacts, total_count: artifactCount } = api<{ total_count: number; artifacts: Array<{ id: number; name: string; expired: boolean }> }>(`repos/${repository}/actions/runs/${run.id}/artifacts?per_page=100`)
      if (artifactCount !== artifacts.length) continue
      for (const artifact of artifacts) {
        const match = new RegExp(`^pr-${ownerId}-${target}-([a-f0-9]{40})-([a-f0-9]{64})$`).exec(artifact.name)
        if (!match || match[2] !== digest || artifact.expired || !Number.isSafeInteger(artifact.id) || artifact.id <= 0) continue
        const tested = api<{ sha: string; tree: { sha: string }; parents: Array<{ sha: string }> }>(`repos/${repository}/git/commits/${match[1]}`)
        // Never confuse the run's PR head SHA with the synthetic merge it tested.
        if (tested.sha !== match[1] || tested.parents.length !== 2 || tested.parents[1].sha !== pull.head.sha) continue
        if (!matchesInputs(tested.sha, tested.parents[0].sha, landed.parents[0].sha)) continue
        return { reuse: true, reason: 'verified same-repository PR component inputs match main across an ancestor base', input_digest: digest, prior_artifact_id: artifact.id, prior_source_sha: tested.sha, prior_run_id: run.id, prior_run_attempt: run.run_attempt }
      }
    }
  }
}
