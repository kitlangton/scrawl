import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFile, mkdir, writeFile } from 'node:fs/promises'
import { resolve, extname } from 'node:path'
import { chromium } from 'playwright-core'

assert(process.env.SITE_ARTIFACT && process.env.SITE_EVIDENCE, 'SITE_ARTIFACT and SITE_EVIDENCE required')
const root = resolve(process.env.SITE_ARTIFACT), evidence = resolve(process.env.SITE_EVIDENCE)
await mkdir(evidence, { recursive: true })
const server = createServer(async (req, res) => {
  const pathname = new URL(req.url, 'http://localhost').pathname
  const file = resolve(root, '.' + (pathname === '/' ? '/index.html' : pathname))
  if (!file.startsWith(root + '/')) return res.writeHead(403).end()
  try {
    res.setHeader('Content-Type', ({ '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.webp': 'image/webp', '.svg': 'image/svg+xml', '.ttf': 'font/ttf', '.mp4': 'video/mp4' })[extname(file)] || 'application/octet-stream')
    res.end(await readFile(file))
  } catch { res.writeHead(404).end() }
})
await new Promise(r => server.listen(0, '127.0.0.1', r))
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || '/usr/bin/google-chrome', headless: true, args: ['--no-sandbox'] })
try {
  const cases = [
    { name: 'linux-x86', width: 1280, platform: 'Linux', architecture: 'x86', asset: 'Apparent-0.3.13-linux-x86_64.AppImage' },
    { name: 'linux-arm', width: 768, platform: 'Linux', architecture: 'arm', asset: 'Apparent-0.3.13-linux-aarch64.AppImage' },
    { name: 'mac-arm', width: 390, platform: 'macOS', architecture: 'arm', asset: 'Apparent-Preview-0.5.1-arm64.dmg', keyboard: true },
    { name: 'api-failure', width: 320, platform: 'Linux', architecture: 'x86', failAPI: true },
    { name: 'unknown-cpu', width: 1280, platform: 'Linux', architecture: '' },
    { name: 'unsupported', width: 390, platform: 'Unknown', architecture: '' },
  ]
  const results = []
  for (const scenario of cases) {
    const page = await browser.newPage({ viewport: { width: scenario.width, height: 900 }, userAgent: `Mozilla/5.0 (X11; ${scenario.platform === 'Unknown' ? 'FreeBSD' : 'Linux'}) AppleWebKit/537.36 Chrome/154.0.0.0 Safari/537.36` })
    const errors = [], launches = [], requests = []
    page.on('pageerror', error => errors.push(String(error)))

    await page.addInitScript(({ platform, architecture }) => {
      Object.defineProperty(navigator, 'userAgentData', { value: { platform, mobile: false,
        getHighEntropyValues: async () => ({ architecture, bitness: architecture ? '64' : '', platformVersion: '26.0.0' }) } })
    }, scenario)
    const session = await page.context().newCDPSession(page)
    await session.send('Page.enable')
    session.on('Page.frameRequestedNavigation', event => { if (event.url.startsWith('apparent:')) launches.push(event.url) })
    let apiCalls = 0
    await page.route('https://api.github.com/repos/tryapparent/apparent/releases?*', async route => {
      apiCalls++
      // Keep the request pending long enough to exercise the shared busy guard.
      await new Promise(r => setTimeout(r, 250))
      const names = ['Apparent-0.3.13-linux-x86_64.AppImage', 'Apparent-0.3.13-linux-aarch64.AppImage', 'Apparent-Preview-0.5.1-arm64.dmg']
      await route.fulfill({ status: scenario.failAPI ? 503 : 200, contentType: 'application/json', body: JSON.stringify([{ published_at: '2026-10-30T00:00:00Z', assets: names.map(name => ({ name, browser_download_url: `https://github.com/tryapparent/apparent/releases/download/test/${name}` })) }]) })
    })
    await page.route('https://github.com/tryapparent/apparent/releases**', async route => {
      requests.push(route.request().url())
      if (route.request().url().includes('/download/')) await route.fulfill({ contentType: 'application/octet-stream', headers: { 'Content-Disposition': 'attachment; filename="synthetic-fixture.txt"' }, body: 'Synthetic regression fixture, not an installer.' })
      else await route.fulfill({ contentType: 'text/html', body: '<title>Fixture release chooser</title><p>Synthetic release-list fixture.</p>' })
    })
    await page.goto(`http://127.0.0.1:${server.address().port}/`)
    await page.waitForFunction(() => customElements.get('apparent-footer'))
    if (scenario.asset || scenario.failAPI) await page.locator('.pond-download [data-download-platform]').waitFor()
    else await page.waitForFunction(() => !document.querySelector('.pond-download [data-download-platform]'))
    await page.evaluate(() => document.fonts.ready)

    const button = page.locator('.footer-try')
    await button.scrollIntoViewIfNeeded()
    const styles = await page.evaluate(() => ['.pond-download .btn-primary', '.footer-try'].map(selector => {
      const style = getComputedStyle(document.querySelector(selector))
      return ['borderRadius', 'backgroundColor', 'color', 'padding', 'fontSize', 'fontWeight', 'lineHeight'].map(key => style[key])
    }))
    assert.deepEqual(styles[0], styles[1], 'hero and footer share the rounded CTA style')
    await page.screenshot({ path: `${evidence}/${scenario.name}-before.png` })
    if (scenario.keyboard) { await button.focus(); await page.keyboard.press('Enter') }
    else await button.click({ delay: 80 })
    assert.match(await button.textContent(), /Not installed yet\?.*click again to download/, 'first click must turn the same button into the download action')
    await page.waitForTimeout(1800)
    assert.deepEqual(launches, ['apparent://open'], 'one launch attempt, with no capture/session parameters')
    assert.equal(apiCalls, 0, 'no release lookup on the first click or a timer')
    assert.deepEqual(requests, [], 'no automatic download or release navigation')
    // Dismiss any browser-owned protocol prompt before the user's next action.
    await page.keyboard.press('Escape')
    assert.equal(await page.locator('dialog[open]').count(), 0, 'the old install dialog is gone')
    const geometry = await button.evaluate(e => {
      const b = e.getBoundingClientRect(), copy = e.closest('.footer-invite').getBoundingClientRect(), scene = e.closest('.footer-scene').getBoundingClientRect()
      return { left: b.left, right: b.right, contentBottom: copy.bottom, sceneBottom: scene.bottom, overflow: document.documentElement.scrollWidth > innerWidth }
    })
    assert(geometry.left >= 0 && geometry.right <= scenario.width && geometry.contentBottom <= geometry.sceneBottom && !geometry.overflow, 'fallback copy fits the viewport and scene')
    await page.screenshot({ path: `${evidence}/${scenario.name}-after.png` })
    // Reconnecting must preserve the download state and not duplicate listeners.
    await page.evaluate(() => { const footer = document.querySelector('apparent-footer'), parent = footer.parentElement; footer.remove(); parent.append(footer) })

    if (scenario.asset) {
      const downloaded = page.waitForEvent('download')
      downloaded.catch(() => {})
      // The real protocol attempt can leave a browser-owned native prompt in
      // headless Chrome. Dispatch the follow-up DOM click to test routing;
      // native consent/dismissal needs the separate desktop browser proof.
      await button.evaluate(e => e.click())
      await page.waitForFunction(() => document.querySelector('.pond-download [aria-busy="true"]'))
      await button.evaluate(e => e.click())
      const download = await downloaded
      assert.equal(download.suggestedFilename(), 'synthetic-fixture.txt')
      assert.equal(apiCalls, 1, 'the existing download busy guard prevents duplicate lookups')
      assert.deepEqual(requests, [`https://github.com/tryapparent/apparent/releases/download/test/${scenario.asset}`])
    } else {
      await Promise.all([page.waitForURL('https://github.com/tryapparent/apparent/releases'), button.evaluate(e => e.click())])
      assert.deepEqual(requests, ['https://github.com/tryapparent/apparent/releases'])
      assert.equal(apiCalls, scenario.failAPI ? 1 : 0)
    }
    assert.deepEqual(launches, ['apparent://open'], 'second click never retries native launch')
    assert.deepEqual(errors, [])
    results.push({ name: scenario.name, width: scenario.width, launches, apiCalls, requests, geometry, status: 'PASS' })
    await page.close()
  }
  await writeFile(`${evidence}/try-button-results.json`, JSON.stringify(results, null, 2))
  console.log(JSON.stringify(results, null, 2))
} finally {
  await browser.close()
  server.close()
}
