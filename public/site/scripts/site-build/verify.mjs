// Standalone Public verification and Pages preparation. Connect integration
// stays in web-delivery.yml; this command uses no Connect implementation.
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const cwd = fileURLToPath(new URL('../../', import.meta.url))
const env = { ...process.env, APPARENT_PUBLIC_SITE: '1',
  VITE_HOSTED_API_ORIGIN: 'https://hosted.tryapparent.com' }
function run(command, args, extraEnv = {}) {
  const result = spawnSync(command, args, { cwd, env: { ...env, ...extraEnv }, stdio: 'inherit' })
  if (result.error) throw result.error
  if (result.status !== 0) process.exit(result.status ?? 1)
}
run('bun', ['install', '--frozen-lockfile'])
run('npm', ['run', 'build'])
run('node', ['scripts/site-build/package-apparent.mjs'])
const tests = [
  'brand/test-brand-favicon.mjs', 'browser-proof/test-github-stars.mjs',
  'pages/test-latest-download.mjs', 'pages/test-platform-os.mjs',
  'pages/test-public-homepage.ts', 'browser-proof/test-video-demo.ts',
  'commerce/test-package-text.mjs', 'site-build/test-expected-canonical-packaging.mjs',
  'browser-proof/test-upstream-link-guard.mjs', 'commerce/test-feature-grid.mjs',
  'browser-proof/test-og-stage-redirect.mjs', 'browser-proof/test-og-video-stage-browser.mjs',
]
run('node', ['--test', ...tests.map(path => `scripts/${path}`)])
run('node', ['--test', 'scripts/artifact/public.test.ts', 'scripts/cd/publish-pages.test.ts',
  'scripts/pages/test-pages-video.mjs', '../../shared/analytics/config.test.ts'])
run('python3', ['-m', 'unittest', 'discover', '-s', '../../shared/ui/test', '-p', 'test_*.py'])
run('node', ['../../shared/ui/test/appearance.mjs'], { PUBLIC_THEME_ARTIFACT: 'public/site/dist-apparent', THEME_SURFACES: 'public' })
run('npm', ['run', 'build:pages-worker'])
run('node', ['--test', 'scripts/pages/test-pages-video-runtime.ts'], { PAGES_WORKER: 'dist-apparent/_worker.js' })
run('node', ['scripts/pages/verify-prepared.ts'])
run('node', ['scripts/cd/analytics-smoke.ts'])
run('node', ['scripts/artifact/prepare.ts'])
run('node', ['scripts/pages/prepare-pages-video.mjs'])
