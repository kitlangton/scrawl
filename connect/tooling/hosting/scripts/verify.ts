import { spawnSync } from 'node:child_process'
import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve, join } from 'node:path'

const root = resolve(import.meta.dirname, '..')
const web = resolve(root, '../../web')
const service = resolve(root, '../../service')
const evidence = mkdtempSync(join(tmpdir(), 'connect-hosting-proof-'))
// PR jobs inherit a synthetic analytics key. The publishable graph must never
// contain it; production main retains its actual, fingerprinted configuration.
const artifactEnvironment: Record<string, string> = process.env.GITHUB_EVENT_NAME === 'pull_request'
  ? { PUBLIC_POSTHOG_ENABLED: 'false', PUBLIC_POSTHOG_TOKEN: '', PUBLIC_POSTHOG_ENVIRONMENT: 'production' }
  : {}
function run(command: string, args: string[], cwd = root, extra: Record<string, string> = {}) {
  const result = spawnSync(command, args, { cwd, stdio: 'inherit', env: { ...process.env, NODE_ENV: 'production', CONNECT_EVIDENCE: evidence, TMPDIR: process.env.TMPDIR || tmpdir(), WRANGLER_SEND_METRICS: 'false', ...extra } })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`Connect hosting verification failed: ${command}`)
}
run('bun', ['install', '--frozen-lockfile'], web, { NODE_ENV: 'development' })
run(process.execPath, ['--experimental-strip-types', 'scripts/playwright-runtime.ts'], web)
run('npm', ['test'], web, artifactEnvironment)
run('npm', ['test'], root, { ...artifactEnvironment, VITE_HOSTED_API_ORIGIN: 'https://app.tryapparent.com/connect', API_ORIGIN: 'https://app.tryapparent.com/connect', SITE_ORIGIN: 'https://app.tryapparent.com' })
// Initiation alone cannot prove that the mounted callback resolves or issues a session.
run('npm', ['run', 'build'], service, artifactEnvironment)
run(process.execPath, ['--test', '--test-concurrency=1', 'test/access/oauth.test.mjs', 'test/access/mounted-base.test.ts'], service, artifactEnvironment)
console.log(`Synthetic browser evidence retained privately: ${evidence}`)
