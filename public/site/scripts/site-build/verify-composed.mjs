// Final composed graph, after the one explicitly hash-bound homepage overlay,
// before the unchanged Pages video transformation. No deployment occurs here.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve, join } from 'node:path'
import { createHash } from 'node:crypto'
import { inventory, resourceGraph, routes, validateConnect, validatePreservedFiles } from '../connect/compose-connect.mjs'
import { withAppearance } from '../pages/system-appearance.mjs'
const site = resolve(import.meta.dirname, '../..')
const root = resolve(process.env.SITE_ARTIFACT || join(site, 'dist-apparent'))
const syntheticOrigin = process.env.COMPOSE_SYNTHETIC_ORIGIN
const files = inventory(root), connect = inventory(join(site, '../../connect/web', syntheticOrigin ? 'dist-test' : 'dist'))
validateConnect(connect, inventory(join(site, 'public')), { syntheticOrigin })
validatePreservedFiles(files, connect) // No overlay may remove/change any Connect-owned byte.
const expected = JSON.parse(readFileSync(join(site, 'published-homepage.sha256.json'), 'utf8'))
for (const [path, digest] of Object.entries(expected)) {
  if (path === 'index.html') {
    const source = readFileSync(join(site, 'published-homepage/index.html'))
    assert.equal(createHash('sha256').update(source).digest('hex'), digest, 'accepted homepage source')
    assert.equal(files.get(path).toString(), withAppearance(source.toString()), 'exact approved theme transform')
  } else assert.equal(createHash('sha256').update(files.get(path)).digest('hex'), digest, `accepted homepage ${path}`)
}
const landing = resourceGraph(files, ['index.html'])
const browser = resourceGraph(files, routes.map(route => `${route}/index.html`), true)
for (const path of browser) if (landing.has(path) && /\.(?:js|css)$/.test(path)) {
  assert.match(files.get(path).toString(), /react/i)
  assert.doesNotMatch(files.get(path).toString(), /hosted-|pond-|X-CSRF-Token/)
}
for (const path of files.keys()) if (path.endsWith('.html') && !routes.some(route => path === `${route}/index.html`)) resourceGraph(files, [path])
assert.doesNotMatch(files.get('index.html').toString(), /href="\/(?:connect|hosted)/)
assert.doesNotMatch(files.get('sitemap.xml').toString(), /\/(?:connect|hosted)/)
const headers = files.get('_headers').toString(), redirects = files.get('_redirects').toString()
for (const route of ['connect', 'hosted']) assert(headers.includes(`/${route}*\n  Cache-Control: no-store\n  X-Robots-Tag: noindex, nofollow`))
for (const suffix of ['', '/device', '/shares']) for (const slash of ['', '/']) assert(redirects.split('\n').includes(`/hosted${suffix}${slash} /connect${suffix} 302`))
assert(files.has('404.html')); assert(!files.has('connect.html'))
console.log(JSON.stringify({ artifact: root, acceptedHomepageFiles: Object.keys(expected).length, landingResources: landing.size, connectResources: browser.size, synthetic: Boolean(syntheticOrigin), limits: 'Static composed graph and metadata; not live Pages, OAuth or native proof.' }))
