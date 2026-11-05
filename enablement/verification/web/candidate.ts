/** Public delivery admission; native application receipts cannot admit or hold it. */
import { execFileSync } from 'node:child_process'
import { appendFileSync, readFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'

export function candidate(env: NodeJS.ProcessEnv, git = (...args: string[]) =>
  execFileSync('git', args, { encoding: 'utf8' }).trim()): string {
  if (env.GITHUB_REPOSITORY !== 'tryapparent/apparent' ||
      env.GITHUB_REF !== 'refs/heads/main' ||
      !['push', 'workflow_dispatch', 'workflow_run'].includes(env.GITHUB_EVENT_NAME ?? '') ||
      env.GITHUB_WORKFLOW_REF !== 'tryapparent/apparent/.github/workflows/value-stream-delivery.yml@refs/heads/main')
    throw new Error('Public delivery requires the canonical main workflow')
  if (env.GITHUB_EVENT_NAME === 'workflow_run') {
    const run = JSON.parse(readFileSync(env.GITHUB_EVENT_PATH!, 'utf8')).workflow_run
    const connect = run?.name === 'Connect producer' && run.path === '.github/workflows/connect-delivery.yml' && run.conclusion === 'success'
    if (!connect || run.status !== 'completed' || run.head_branch !== 'main' ||
        run.head_repository?.full_name !== 'tryapparent/apparent' ||
        !['push', 'workflow_dispatch'].includes(run.event)) throw Error('Untrusted producer completion event')
  }
  const sha = env.GITHUB_SHA ?? ''
  if (!/^[a-f0-9]{40}$/.test(sha)) throw new Error('Web delivery requires an exact source SHA')
  if (git('rev-parse', 'HEAD') !== sha) throw new Error('Web checkout does not match the candidate')
  git('merge-base', '--is-ancestor', sha, 'refs/remotes/origin/main')
  return sha
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const sha = candidate(process.env)
    if (!process.env.GITHUB_OUTPUT) throw new Error('Missing workflow output file')
    appendFileSync(process.env.GITHUB_OUTPUT, `sha=${sha}\n`)
    console.log(`Public candidate ${sha}; publication requires this run's verified stream artifact.`)
  } catch (error) {
    console.error(String(error))
    process.exitCode = 1
  }
}
