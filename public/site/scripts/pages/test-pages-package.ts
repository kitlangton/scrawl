import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, cpSync, readFileSync, writeFileSync, rmSync, existsSync, symlinkSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve, join } from 'node:path'
import { spawnSync } from 'node:child_process'
const site = resolve(import.meta.dirname, '../..')
function fixture() {
  const outer = mkdtempSync(join(tmpdir(), 'pages-package-'))
  const root = join(outer, 'public/site')
  mkdirSync(join(root, 'scripts/pages'), { recursive: true })
  mkdirSync(join(root, 'dist-apparent/assets'), { recursive: true })
  cpSync(join(site, 'scripts/pages/prepare-pages.ts'), join(root, 'scripts/pages/prepare-pages.ts'))
  writeFileSync(join(root, 'dist-apparent/index.html'), '<html>fresh source homepage</html>')
  writeFileSync(join(root, 'dist-apparent/assets/source.js'), 'fresh source JS')
  writeFileSync(join(root, 'dist-apparent/_worker.js'), 'verified compiled worker')
  return { root, cleanup: () => rmSync(outer, { recursive: true, force: true }) }
}
const run = (root: string) => spawnSync(process.execPath, [join(root, 'scripts/pages/prepare-pages.ts')], { encoding: 'utf8' })
test('Pages preserves only source-built Public homepage and assets', () => {
  const { root, cleanup } = fixture()
  try {
    // A stale snapshot, if accidentally left on disk, has no authority.
    mkdirSync(join(root, 'published-homepage'), { recursive: true })
    writeFileSync(join(root, 'published-homepage/index.html'), 'stale snapshot')
    const result = run(root)
    assert.equal(result.status, 0, result.stderr)
    assert.equal(readFileSync(join(root, 'dist-apparent/index.html'), 'utf8'), '<html>fresh source homepage</html>')
    assert.equal(readFileSync(join(root, 'dist-apparent/assets/source.js'), 'utf8'), 'fresh source JS')
    assert.equal(readFileSync(join(root, 'dist-apparent/_worker.js'), 'utf8'), 'verified compiled worker')
    assert(existsSync(join(root, 'dist-apparent/404.html')))
    const redirects = readFileSync(join(root, 'dist-apparent/_redirects'), 'utf8')
    assert.equal(redirects, '', 'Public must not publish Connect redirects')
    assert.match(readFileSync(join(root, 'dist-apparent/_headers'), 'utf8'), /X-Apparent-Publisher: cloudflare-pages/)
    const headers = readFileSync(join(root, 'dist-apparent/_headers'), 'utf8')
    assert.match(headers, /\/\*\n  Cache-Control: no-cache, no-transform\n/)
    assert.doesNotMatch(headers, /connect|hosted|noindex|nofollow/)
    assert.equal(run(root).status, 1, 'provider metadata must not be reprocessed as public input')
  } finally { cleanup() }
})
for (const path of ['pitch/index.html', 'map-tyyxsf5b/index.html', 'hosted/private.png', 'deployment.json', 'connect/secret.json', 'connect/index.html', 'connect/device/index.html', 'connect/shares/index.html', 'hosted/index.html', 'hosted/source-pond-v4.png']) {
  test(`Pages refuses unexpected public path ${path} before changing homepage`, () => {
    const { root, cleanup } = fixture()
    try {
      const target = join(root, 'dist-apparent', path)
      mkdirSync(resolve(target, '..'), { recursive: true }); writeFileSync(target, 'private sentinel')
      assert.notEqual(run(root).status, 0)
      assert.equal(readFileSync(join(root, 'dist-apparent/index.html'), 'utf8'), '<html>fresh source homepage</html>')
      assert(!existsSync(join(root, 'dist-apparent/_headers')))
    } finally { cleanup() }
  })
}
test('Pages refuses symlinked files before adding metadata', () => {
  const { root, cleanup } = fixture()
  try {
    symlinkSync('/etc/hostname', join(root, 'dist-apparent/assets/link.js'))
    assert.notEqual(run(root).status, 0)
    assert(!existsSync(join(root, 'dist-apparent/_headers')))
  } finally { cleanup() }
})

test('legacy composition configuration cannot redirect Public metadata output', () => {
  const { root, cleanup } = fixture()
  try {
    const result = spawnSync(process.execPath, [join(root, 'scripts/pages/prepare-pages.ts')], {
      encoding: 'utf8', env: { ...process.env, COMPOSE_SYNTHETIC_ORIGIN: 'http://127.0.0.1:1234' },
    })
    assert.equal(result.status, 0, result.stderr)
    assert(existsSync(join(root, 'dist-apparent/_headers')))
    assert(!existsSync(join(root, 'dist/synthetic-composition')))
  } finally { cleanup() }
})
