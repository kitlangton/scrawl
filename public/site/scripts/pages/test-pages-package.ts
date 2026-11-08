import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, cpSync, readFileSync, writeFileSync, rmSync, existsSync, symlinkSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve, join } from 'node:path'
import { spawnSync } from 'node:child_process'

const site = resolve(import.meta.dirname, '../..')
const routes = ['connect', 'connect/device', 'connect/shares', 'hosted', 'hosted/device', 'hosted/shares']
function fixture() {
  const outer = mkdtempSync(join(tmpdir(), 'pages-package-'))
  const root = join(outer, 'public/site')
  mkdirSync(join(root, 'scripts/pages'), { recursive: true })
  mkdirSync(join(root, 'dist-apparent/assets'), { recursive: true })
  cpSync(join(site, 'scripts/pages/prepare-pages.mjs'), join(root, 'scripts/pages/prepare-pages.mjs'))
  // Leftover snapshot files have no authority over the source build.
  mkdirSync(join(root, 'published-homepage'))
  writeFileSync(join(root, 'published-homepage/index.html'), 'stale snapshot')
  writeFileSync(join(root, 'dist-apparent/index.html'), '<html>fresh source homepage</html>')
  writeFileSync(join(root, 'dist-apparent/assets/source.js'), 'fresh source JS')
  for (const route of routes) {
    mkdirSync(join(root, 'dist-apparent', route), { recursive: true })
    writeFileSync(join(root, 'dist-apparent', route, 'index.html'), `fresh ${route}`)
  }
  return { root, cleanup: () => rmSync(outer, { recursive: true, force: true }) }
}
const run = (root: string) => spawnSync(process.execPath, [join(root, 'scripts/pages/prepare-pages.mjs')], { encoding: 'utf8' })
test('Pages preserves source-built homepage, assets and independently composed Connect bytes', () => {
  const { root, cleanup } = fixture()
  try {
    const result = run(root)
    assert.equal(result.status, 0, result.stderr)
    assert.equal(readFileSync(join(root, 'dist-apparent/index.html'), 'utf8'), '<html>fresh source homepage</html>')
    assert.equal(readFileSync(join(root, 'dist-apparent/assets/source.js'), 'utf8'), 'fresh source JS')
    for (const route of routes) assert.equal(readFileSync(join(root, 'dist-apparent', route, 'index.html'), 'utf8'), `fresh ${route}`)
    assert(existsSync(join(root, 'dist-apparent/404.html')))
    const redirects = readFileSync(join(root, 'dist-apparent/_redirects'), 'utf8')
    for (const suffix of ['', '/device', '/shares']) for (const slash of ['', '/']) assert(redirects.includes(`/hosted${suffix}${slash} /connect${suffix} 302`))
    const headers = readFileSync(join(root, 'dist-apparent/_headers'), 'utf8')
    assert.match(headers, /X-Apparent-Publisher: cloudflare-pages/)
    assert.match(headers, /Cache-Control: no-cache, no-transform/)
    for (const route of ['connect', 'hosted']) assert(headers.includes(`/${route}*\n  Cache-Control: no-store\n  X-Robots-Tag: noindex, nofollow`))
    assert.equal(run(root).status, 1, 'provider metadata must not be reprocessed as public input')
  } finally { cleanup() }
})
test('Pages retains the verified compiled movie worker', () => {
  const { root, cleanup } = fixture()
  try {
    writeFileSync(join(root, 'dist-apparent/_worker.js'), 'verified compiled worker')
    const result = run(root)
    assert.equal(result.status, 0, result.stderr)
    assert.equal(readFileSync(join(root, 'dist-apparent/_worker.js'), 'utf8'), 'verified compiled worker')
  } finally { cleanup() }
})
for (const path of ['pitch/index.html', 'map-tyyxsf5b/index.html', 'hosted/private.png', 'deployment.json', 'connect/secret.json']) {
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
