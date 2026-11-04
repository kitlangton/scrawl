/** Select the entire delta since successful web verification, including deletions. */
import { execFileSync } from 'node:child_process'
import { appendFileSync, readFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'
export function relevant(path: string) {
  return ['public/site/', 'connect/web/', 'connect/delivery/', 'shared/ui/', 'shared/analytics/', '.github/actions/runtime-budget/', 'enablement/verification/web/'].some(prefix => path.startsWith(prefix)) ||
    ['.github/workflows/web-delivery.yml', '.github/workflows/connect-delivery.yml', 'enablement/verification/workflow-budget.ts'].includes(path)
}
export function select(paths: string[], known: boolean, force = false) {
  return { required: force || !known || paths.some(relevant), reason: force ? 'Explicit main refresh requires web checks' : !known ? 'No successful main web baseline; full web checks required' : paths.some(relevant) ? 'Web or shared consumer inputs changed' : 'No web consumer inputs changed' }
}
export function gate(required: string, result: string) {
  if (!['true', 'false'].includes(required)) throw Error('Missing web selection')
  if (required === 'true' ? result !== 'success' : result !== 'skipped') throw Error(`Unexpected web verification result: ${required}/${result}`)
  return required === 'true' ? 'verified' : 'not-applicable'
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (process.argv[2] === 'gate') console.log(gate(process.env.WEB_REQUIRED ?? '', process.env.WEB_RESULT ?? ''))
  else {
    const sha = process.env.GITHUB_SHA!, repository = 'tryapparent/apparent'
    const response = await fetch(`https://api.github.com/repos/${repository}/actions/workflows/web-delivery.yml/runs?branch=main&status=success&per_page=100`, { headers: { Authorization: `Bearer ${process.env.GH_TOKEN}`, Accept: 'application/vnd.github+json' } })
    if (!response.ok) throw Error(`Cannot establish web verification baseline: ${response.status}`)
    const { workflow_runs: runs } = await response.json() as any
    let paths: string[] = [], known = false
    for (const run of runs) {
      if (run.head_sha === sha || run.head_branch !== 'main' || run.head_repository?.full_name !== repository || run.repository?.full_name !== repository || run.path !== '.github/workflows/web-delivery.yml' || !['push', 'workflow_dispatch', 'workflow_run'].includes(run.event) || run.status !== 'completed' || run.conclusion !== 'success' || !/^[a-f0-9]{40}$/.test(run.head_sha)) continue
      try {
        execFileSync('git', ['merge-base', '--is-ancestor', run.head_sha, sha])
        paths = execFileSync('git', ['diff', '--no-renames', '--name-only', '-z', run.head_sha, sha], { encoding: 'utf8' }).split('\0').filter(Boolean)
        known = true; break
      } catch { /* Missing/nonancestor history cannot narrow verification. */ }
    }
    const force = process.env.GITHUB_EVENT_NAME === 'workflow_dispatch' || (process.env.GITHUB_EVENT_NAME === 'workflow_run' && JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH!, 'utf8')).workflow_run?.event === 'workflow_dispatch')
    const selection = select(paths, known, force)
    if (!process.env.GITHUB_OUTPUT) throw Error('Missing selection output')
    appendFileSync(process.env.GITHUB_OUTPUT, `required=${selection.required}\n`)
    console.log(JSON.stringify({ ...selection, paths }))
  }
}
