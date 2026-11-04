/** Build only a private Public analytics fixture; reuse downloaded Connect bytes. */
import { mkdtempSync, cpSync, rmSync, symlinkSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { execFileSync } from 'node:child_process'
import { composeArtifacts } from './compose-connect.mjs'
const site = resolve(import.meta.dirname, '../..')
const connect = resolve(process.env.CONNECT_BROWSER_ARTIFACT || join(site, '../../connect/web/dist'))
const root = mkdtempSync(join(tmpdir(), 'public-analytics-smoke-'))
symlinkSync(join(site, 'node_modules'), join(root, 'node_modules'), 'dir')
const env = { ...process.env, APPARENT_PUBLIC_SITE: '1', PUBLIC_POSTHOG_ENABLED: 'true', PUBLIC_POSTHOG_TOKEN: 'phc_synthetic', PUBLIC_POSTHOG_HOST: 'https://eu.i.posthog.com', PUBLIC_POSTHOG_ENVIRONMENT: 'production', PUBLIC_BUILD_DIR: join(root, 'dist'), PUBLIC_SSR_DIR: join(root, 'ssr'), PUBLIC_PACKAGE_OUTPUT: join(root, 'public') }
const run = (command: string, args: string[], extra = {}) => execFileSync(command, args, { cwd: site, env: { ...env, ...extra }, stdio: 'inherit' })
try {
  run(join(site, 'node_modules/.bin/vite'), ['build', '--outDir', env.PUBLIC_BUILD_DIR])
  run(join(site, 'node_modules/.bin/vite'), ['build', '--ssr', 'src/entry-server.tsx', '--outDir', env.PUBLIC_SSR_DIR])
  run('node', ['scripts/site-build/prerender.ts'])
  run('node', ['scripts/site-build/package-apparent.mjs'])
  const out = join(root, 'composed')
  composeArtifacts(env.PUBLIC_PACKAGE_OUTPUT, connect, join(site, 'public'), out)
  // Provider metadata is copied byte-for-byte from the checked composition.
  const production = resolve(process.env.SITE_ARTIFACT || join(site, 'dist-apparent'))
  for (const path of ['_headers', '_redirects', '404.html']) cpSync(join(production, path), join(out, path))
  run('node', ['--experimental-strip-types', '--test', 'scripts/connect/composed-browser.test.ts'], { SITE_ARTIFACT: out })
} finally { rmSync(root, { recursive: true, force: true }) }
