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
run('node', ['scripts/site-build/package-apparent.mjs'])
const tests = [
  'brand/test-brand-favicon.mjs', 'browser-proof/test-github-stars.mjs',
  'pages/test-latest-download.mjs', 'pages/test-platform-os.mjs',
  'pages/test-public-homepage.mjs', 'browser-proof/test-video-demo.mjs',
  'commerce/test-package-text.mjs', 'site-build/test-expected-canonical-packaging.mjs',
  'browser-proof/test-upstream-link-guard.mjs', 'commerce/test-package-feature-grid.mjs',
  'browser-proof/test-og-stage-redirect.mjs', 'browser-proof/test-og-video-stage-browser.mjs',
]
run('node', ['--test', ...tests.map(path => `scripts/${path}`)])
run('python3', ['-m', 'unittest', 'discover', '-s', 'scripts/cd', '-p', 'test_*.py'])
