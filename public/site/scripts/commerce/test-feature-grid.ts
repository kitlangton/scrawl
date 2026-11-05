import assert from 'node:assert/strict'
import { readFileSync, existsSync } from 'node:fs'
import { join, dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { after, before, test } from 'node:test'
import { chromium } from 'playwright-core'
let browser, browserPage
before(async () => {
  browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, args: ['--no-sandbox'] })
  browserPage = await browser.newPage()
  await browserPage.route('**/*', route => route.abort())
})
after(async () => { await browser?.close() })
async function animationScripts(html) {
  // Parse in a detached document to inspect script text without executing it.
  return browserPage.evaluate(source => {
    const document = new DOMParser().parseFromString(source, 'text/html')
    return Array.from(document.querySelectorAll('script'), script => script.textContent)
  }, html)
}
const artifact = fileURLToPath(new URL('../../dist-apparent/', import.meta.url))
const root = readFileSync(join(artifact, 'index.html'), 'utf8')
test('packaged homepage places four explanations between the video and belief sections', () => {
  assert.ok(root.indexOf('id="demo"') < root.indexOf('id="feature-grid-title"'))
  assert.ok(root.indexOf('id="feature-grid-title"') < root.indexOf('id="belief"'))
  assert.equal((root.match(/class="feature-grid-card"/g) ?? []).length, 4)
  assert.match(root, /Pause animations/)
})
test('every local animation asset is packaged, including the cropped whiteboard movie', async () => {
  for (const page of ['how-it-works', 'timeline', 'handoff', 'anywhere']) {
    const file = join(artifact, 'feature-demos', page, 'index.html')
    const html = readFileSync(file, 'utf8')
    assert.doesNotMatch(html, /127\.0\.0\.1|\/Users\/liv\//)
    for (const [, reference] of html.matchAll(/(?:src|poster)="([^"]+)"/g)) {
      if (/^(?:https?:|data:)/.test(reference)) continue
      const target = reference.startsWith('/') ? join(artifact, reference) : resolve(dirname(file), reference)
      assert.ok(existsSync(target), `${page}: missing ${reference}`)
    }
    for (const script of await animationScripts(html)) assert.doesNotThrow(() => new Function(script), `${page}: invalid animation script`)
  }
})
test('animation script extraction follows HTML parsing, including malformed markup', async () => {
  for (const html of [
    '<SCRIPT>const value = 1;</SCRIPT>',
    '<ScRiPt data-label="a > b">const value = 1;</sCrIpT>',
    '<script>const value = 1;</script ignored="yes">',
    '<script>const value = 1;',
    '<!-- ignored --!><script>const value = 1;</script>',
  ]) assert.deepEqual(await animationScripts(html), ['const value = 1;'], html)
  assert.deepEqual(await animationScripts('<!-- <script>not JavaScript</script> -->'), [])
  assert.deepEqual(await animationScripts('<textarea><script>not JavaScript</script></textarea>'), [])
  assert.deepEqual(await animationScripts('<scripture>not JavaScript</scripture>'), [])
  assert.deepEqual(await animationScripts('<script>const first = "&amp;";</script><script>const second = 2;</script>'), ['const first = "&amp;";', 'const second = 2;'])
})
test('invalid scripts cannot hide behind uppercase or malformed closing tags', async () => {
  for (const html of ['<SCRIPT>const = ;</SCRIPT>', '<script>const = ;</script ignored>']) {
    const scripts = await animationScripts(html)
    assert.equal(scripts.length, 1, html)
    assert.throws(() => new Function(scripts[0]), SyntaxError)
  }
})
test('extracting animation scripts never executes their contents', async () => {
  const script = 'globalThis.animationScriptExecuted = true;'
  assert.deepEqual(await animationScripts(`<script>${script}</script>`), [script])
  assert.equal(await browserPage.evaluate(() => globalThis.animationScriptExecuted), undefined)
})
