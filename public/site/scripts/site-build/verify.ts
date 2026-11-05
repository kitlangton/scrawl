// Standalone Public verification. Composed Connect/cross-origin and Pages
// preparation remain in the web consumer; this is not a publication command.
import { accessSync, constants, existsSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const args = new Set(process.argv.slice(2))
for (const arg of args) if (!['--installed', '--install-browser'].includes(arg)) throw new Error(`Unknown argument: ${arg}`)
const cwd = fileURLToPath(new URL('../../', import.meta.url))
const env: NodeJS.ProcessEnv = { ...process.env, APPARENT_PUBLIC_SITE: '1' }
function run(command: string, commandArgs: string[]): void {
  const result = spawnSync(command, commandArgs, { cwd, env, stdio: 'inherit' })
  if (result.error) throw result.error
  if (result.status !== 0) process.exit(result.status ?? 1)
}
function executable(path: string): boolean {
  try { accessSync(path, constants.X_OK); return true } catch { return false }
}
if (!args.has('--installed')) run('bun', ['install', '--frozen-lockfile'])
if (env.CHROME_PATH && !executable(env.CHROME_PATH)) throw new Error(`CHROME_PATH is not executable: ${env.CHROME_PATH}`)
if (args.has('--install-browser')) {
  const { chromium, firefox } = await import('playwright-core')
  // The responsive homepage suite exercises Firefox even with a custom Chrome.
  const browsers = env.CHROME_PATH ? ['firefox'] : ['chromium', 'firefox']
  run('./node_modules/.bin/playwright-core', ['install', '--with-deps', ...browsers])
  env.CHROME_PATH ??= chromium.executablePath()
  for (const path of [env.CHROME_PATH, firefox.executablePath()]) {
    if (!existsSync(path)) throw new Error(`Installed browser is missing: ${path}`)
  }
}

run('npm', ['run', 'build'])
run('node', ['scripts/site-build/package-apparent.ts'])
run('npm', ['run', 'build:pages-worker'])
env.PAGES_WORKER = 'dist-apparent/_worker.js'
run('node', ['--test', 'scripts/pages/test-pages-video-runtime.ts'])
delete env.PAGES_WORKER
const tests = [
  'brand/test-brand-favicon.ts', 'browser-proof/test-github-stars.ts',
  'pages/test-latest-download.ts', 'pages/test-platform-os.ts',
  'artifact/public.test.ts', 'pages/test-pages-package.ts', 'pages/test-public-homepage.ts', 'browser-proof/test-video-demo.ts',
  'commerce/test-package-text.ts', 'site-build/test-expected-canonical-packaging.ts',
  'browser-proof/test-upstream-link-guard.ts',
  'browser-proof/test-og-stage-redirect.ts', 'browser-proof/test-og-video-stage-browser.ts',
]
run('node', ['--test', ...tests.map(path => `scripts/${path}`)])
run('node', ['--test', 'scripts/cd/publish-pages.test.ts', 'scripts/cd/source-boundary.test.ts', 'scripts/cd/stream-artifact.test.ts'])
run('node', ['--experimental-strip-types', 'scripts/cd/analytics-smoke.ts'])
