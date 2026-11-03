// Public contract: this directory is a route-only artifact, not a second marketing site.
import assert from 'node:assert/strict'
import { readFile, readdir } from 'node:fs/promises'
import { resolve } from 'node:path'
import test from 'node:test'
const root = resolve(process.env.CONNECT_ARTIFACT || 'dist')
const assetSource = resolve(process.env.CONNECT_ASSET_SOURCE || '../../public/site/public/assets')
const routes = ['connect', 'connect/device', 'connect/shares', 'hosted', 'hosted/device', 'hosted/shares']
test('independent artifact publishes all private loading routes and nothing else', async () => {
  let entries = []
  try { entries = await readdir(root) } catch { /* Assert the product boundary below. */ }
  assert(entries.includes('connect'), 'independent Connect artifact is absent')
  assert.deepEqual(entries.sort(), ['HostGrotesk-Variable.ttf', 'apparent-wordmark.svg', 'assets', 'connect', 'hosted'].sort())
  for (const route of routes) {
    const html = await readFile(`${root}/${route}/index.html`, 'utf8')
    assert.match(html, /Checking your account/)
    assert.match(html, /name="robots" content="noindex, nofollow"/)
    assert.match(html, route.endsWith('/shares') ? /<title>Manage shared links — Apparent<\/title>/ : /<title>Connect(?: a device)? — Apparent<\/title>/)
    assert.match(html, /name="referrer" content="no-referrer"/)
    assert.match(html, /http-equiv="Cache-Control" content="no-store"/)
    if (route.endsWith('/shares')) assert.match(html, /href="\/connect\/"/)
    assert.doesNotMatch(html, /canonical|sitemap|application\/ld\+json|csrfToken|accessToken|verified@example/)
    assert.match(html, /\/assets\/[^" ]+\.js/)
    assert.match(html, /\/assets\/[^" ]+\.css/)
  }
})
test('only approved public assets are copied without changing their URLs or bytes', async () => {
  for (const [served, source] of [
    ['hosted/source-pond-v4.png', 'hosted/source-pond-v4.png'],
    ['hosted/agent-pond-v4.png', 'hosted/agent-pond-v4.png'],
    ['hosted/agent-reading-v4.png', 'hosted/agent-reading-v4.png'],
    ['HostGrotesk-Variable.ttf', 'fonts/HostGrotesk-Variable.ttf'],
    ['apparent-wordmark.svg', 'brand/apparent-wordmark.svg'],
  ]) {
    assert.deepEqual(await readFile(`${root}/${served}`), await readFile(`${assetSource}/${source}`))
  }
})
test('runtime and SSR graph cannot import the legacy site implementation', async () => {
  for (const name of await readdir('src')) {
    if (!/\.(tsx?|css)$/.test(name)) continue
    const raw = await readFile(`src/${name}`, 'utf8')
    // Only the canonical theme and framework-independent analytics broker may cross owners.
    const source = (name === 'base.css' ? raw.replace('@import "../../../shared/ui/src/tokens.css";', '') : raw)
      .replaceAll('from "../../../shared/analytics/browser"', 'from "approved-shared-analytics"')
    assert.doesNotMatch(source, /(?:from\s*|import\s*)["'][^"']*(?:site\/|\.\.\/)/)
    assert.doesNotMatch(source, /localStorage|sessionStorage|console\./)
  }
})