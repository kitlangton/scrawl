// Exercise the exact compiled movie worker after publication's chunking step,
// without changing the raw Public package consumed by composition.
import { cpSync, mkdtempSync, rmSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { join, resolve } from 'node:path'
import { tmpdir } from 'node:os'
import { preparePagesVideo } from './prepare-pages-video.mjs'

const site = resolve(import.meta.dirname, '../..')
const prepared = mkdtempSync(join(tmpdir(), 'public-prepared-browser-'))
try {
  cpSync(join(site, 'dist-apparent'), prepared, { recursive: true })
  preparePagesVideo(prepared)
  const result = spawnSync(process.execPath, ['--test', 'scripts/pages/test-public-homepage.ts'], {
    cwd: site, env: { ...process.env, SITE_ARTIFACT: prepared }, stdio: 'inherit',
  })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`Prepared Public browser verification failed (${result.status})`)
} finally { rmSync(prepared, { recursive: true, force: true }) }
