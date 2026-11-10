// Publication-only release registration. No ingestion or runtime credentials.
import { execFileSync } from 'node:child_process'
import { appendFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'

export const releaseIdPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
type Environment = NodeJS.ProcessEnv
type Run = (args: string[], env: Environment) => string

export function registerRelease(platform: string, env: Environment, sha: string, run: Run): string {
  if (!['linux', 'macos'].includes(platform)) throw new Error('Unsupported release platform')
  if (env.VERIFY_ONLY === 'true') return ''
  if (!/^[0-9a-f]{40}$/.test(sha)) throw new Error('Release requires the checked-out full commit')
  const version = platform === 'macos' ? env.APPARENT_PREVIEW_VERSION : (env.TAG || `linux-main-v2-${sha}`)
  if (!version || !/^[a-zA-Z0-9.+_-]{1,128}$/.test(version)) throw new Error('Invalid release version')
  if (!env.POSTHOG_CLI_API_KEY) throw new Error('Missing POSTHOG_CLI_API_KEY with error tracking write scope')
  const id = run([
    '--yes', '@posthog/cli@0.19.0', '--host', 'https://eu.posthog.com',
    'release', 'resolve', '--release-name', `apparent-desktop-${platform}`, '--release-version', version,
  ], {
    ...env, POSTHOG_CLI_PROJECT_ID: '329943', POSTHOG_CLI_HOST: 'https://eu.posthog.com',
    POSTHOG_CLI_DRY_RUN: 'false', GITHUB_SHA: sha, GITHUB_REF_NAME: env.TAG || env.GITHUB_REF_NAME,
  }).trim()
  if (!releaseIdPattern.test(id)) throw new Error('PostHog returned no valid release ID')
  return id
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const sha = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim()
    const id = registerRelease(process.argv[2], process.env, sha, (args, env) =>
      execFileSync('npx', args, { env, encoding: 'utf8', timeout: 120_000, stdio: ['ignore', 'pipe', 'pipe'] }))
    if (!process.env.GITHUB_ENV) throw new Error('Missing workflow environment file')
    appendFileSync(process.env.GITHUB_ENV, `APPARENT_POSTHOG_RELEASE_ID=${id}\n`)
  } catch {
    // Do not forward CLI exceptions/output: they can contain credentials.
    console.error('PostHog release registration failed; check the release credential and project access.')
    process.exitCode = 1
  }
}
