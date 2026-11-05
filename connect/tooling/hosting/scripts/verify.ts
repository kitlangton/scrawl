import { spawnSync } from 'node:child_process'
import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve, join } from 'node:path'

const root = resolve(import.meta.dirname, '..')
const web = resolve(root, '../../web')
const evidence = mkdtempSync(join(tmpdir(), 'connect-hosting-proof-'))
function run(command: string, args: string[], cwd = root, extra: Record<string, string> = {}) {
  const result = spawnSync(command, args, { cwd, stdio: 'inherit', env: { ...process.env, NODE_ENV: 'production', CONNECT_EVIDENCE: evidence, TMPDIR: process.env.TMPDIR || tmpdir(), WRANGLER_SEND_METRICS: 'false', ...extra } })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`Connect hosting verification failed: ${command}`)
}
run('bun', ['install', '--frozen-lockfile'], web, { NODE_ENV: 'development' })
run(process.execPath, ['--experimental-strip-types', 'scripts/playwright-runtime.ts'], web)
run('npm', ['test'], web)
run('npm', ['test'])
console.log(`Synthetic browser evidence retained privately: ${evidence}`)
