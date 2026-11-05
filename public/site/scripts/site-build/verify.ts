// Standalone Public verification. Composed Connect/cross-origin and Pages
// preparation remain in web-delivery.yml; this is not a publication command.
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const cwd = fileURLToPath(new URL('../../', import.meta.url))
const env = { ...process.env, APPARENT_PUBLIC_SITE: '1',
  VITE_HOSTED_API_ORIGIN: 'https://hosted.tryapparent.com' }
function run(command, args) {
  const result = spawnSync(command, args, { cwd, env, stdio: 'inherit' })
  if (result.error) throw result.error
  if (result.status !== 0) process.exit(result.status ?? 1)
}
run('bun', ['install', '--frozen-lockfile'])
run('npm', ['run', 'build'])
run('node', ['scripts/site-build/package-apparent.ts'])
const tests = [
  'brand/test-brand-favicon.ts', 'browser-proof/test-github-stars.ts',
  'pages/test-latest-download.ts', 'pages/test-platform-os.ts',
  'pages/test-public-homepage.ts', 'browser-proof/test-video-demo.ts',
  'commerce/test-package-text.ts', 'site-build/test-expected-canonical-packaging.ts',
  'browser-proof/test-upstream-link-guard.ts',
  'browser-proof/test-og-stage-redirect.ts', 'browser-proof/test-og-video-stage-browser.ts',
]
run('node', ['--test', ...tests.map(path => `scripts/${path}`)])
run('node', ['--test', 'scripts/cd/publish-pages.test.ts', 'scripts/cd/source-boundary.test.ts'])
