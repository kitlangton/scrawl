import assert from 'node:assert/strict'

/** Build and exercise the Public homepage with enabled synthetic analytics. No Connect source or bytes are consumed. */
import { createServer } from 'node:http'
import { mkdtempSync, rmSync, symlinkSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { extname, join, resolve } from 'node:path'
import { execFileSync } from 'node:child_process'
import { chromium } from 'playwright-core'

const site = resolve(import.meta.dirname, '../..')
const root = mkdtempSync(join(tmpdir(), 'apparent-dev-public-analytics-'))
const packageOutput = join(root, 'public')
const env = {
  ...process.env,
  APPARENT_PUBLIC_SITE: '1',
  PUBLIC_POSTHOG_ENABLED: 'true',
  PUBLIC_POSTHOG_TOKEN: 'phc_synthetic',
  PUBLIC_POSTHOG_HOST: 'https://eu.i.posthog.com',
  PUBLIC_POSTHOG_ENVIRONMENT: 'production',
  PUBLIC_BUILD_DIR: join(root, 'dist'),
  PUBLIC_SSR_DIR: join(root, 'ssr'),
  PUBLIC_PACKAGE_OUTPUT: packageOutput,
}
const run = (command: string, args: string[]) => execFileSync(command, args, { cwd: site, env, stdio: 'inherit' })
const mime: Record<string, string> = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.png': 'image/png', '.webp': 'image/webp', '.jpg': 'image/jpeg', '.ttf': 'font/ttf', '.mp4': 'video/mp4' }

let server: ReturnType<typeof createServer> | undefined
let browser: Awaited<ReturnType<typeof chromium.launch>> | undefined
try {
  symlinkSync(join(site, 'node_modules'), join(root, 'node_modules'), 'dir')
  run(join(site, 'node_modules/.bin/vite'), ['build', '--outDir', env.PUBLIC_BUILD_DIR])
  run(join(site, 'node_modules/.bin/vite'), ['build', '--ssr', 'src/entry-server.tsx', '--outDir', env.PUBLIC_SSR_DIR])
  run('node', ['scripts/site-build/prerender.ts'])
  run('node', ['scripts/site-build/package-apparent.mjs'])

  const publicOrigin = 'https://app.tryapparent.com'
  server = createServer(async (request, response) => {
    const pathname = new URL(request.url!, publicOrigin).pathname
    const relativePath = pathname === '/' ? '/index.html' : pathname
    const file = resolve(packageOutput, `.${relativePath}`)
    if (!file.startsWith(packageOutput + '/')) { response.writeHead(403).end(); return }
    try {
      response.setHeader('Content-Type', mime[extname(file)] || 'application/octet-stream')
      response.end(await readFile(file))
    } catch { response.writeHead(404).end() }
  })
  await new Promise<void>(done => server.listen(0, '127.0.0.1', done))
  const localOrigin = `http://127.0.0.1:${(server.address() as import('node:net').AddressInfo).port}`
  browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), headless: true, args: ['--no-sandbox', '--mute-audio'] })
  {
    const events: Array<{ event: string; properties: Record<string, unknown>; api_key: string }> = []
    const context = await browser.newContext()
    await context.route('**/*', async route => {
      const request = route.request(), url = new URL(request.url())
      if (url.origin === publicOrigin) {
        const response = await route.fetch({ url: localOrigin + url.pathname + url.search, maxRedirects: 0 })
        await route.fulfill({ response }); return
      }
      if (url.origin === 'https://eu.i.posthog.com') {
        assert.equal(url.href, 'https://eu.i.posthog.com/capture/')
        assert.equal(request.method(), 'POST')
        assert.equal(request.headers().cookie, undefined, 'analytics sends omit browser cookies')
        assert.equal(request.headers().referer, undefined, 'analytics sends omit the page referrer')
        const event = request.postDataJSON()
        events.push(event)
        assert.doesNotMatch(JSON.stringify(event), /NEVER-SEND|capture-secret|userCode|csrf|@/)
        assert.equal(event.properties.environment, 'production')
        assert.equal(event.properties.actor, 'browser')
        await route.fulfill({ status: 200, headers: { 'Access-Control-Allow-Origin': '*' }, body: '{}' }); return
      }
      await route.abort()
    })
    const page = await context.newPage()
    const errors: string[] = []
    page.on('pageerror', error => errors.push(error.message))
    await page.goto(publicOrigin + '/?utm_source=NEVER-SEND#capture-secret')
    await page.locator('.pond-page').waitFor()
    assert.equal(events.length, 0, 'analytics requires prior browser choice')
    const pageview = page.waitForRequest(request => request.url() === 'https://eu.i.posthog.com/capture/')
    await page.getByRole('button', { name: 'Allow analytics', exact: true }).click()
    await pageview
    await page.waitForFunction(() => {
      const choice = JSON.parse(localStorage.getItem('apparent.product-analytics.v1') || 'null')
      return choice?.choice === 'allow'
    })
    assert(events.length > 0, 'enabled Public analytics sends after affirmative consent')
    assert.equal(events[0].event, '$pageview')
    assert.equal(events[0].api_key, 'phc_synthetic')
    assert.equal(events[0].properties.page, '/')
    assert.equal(events[0].properties.$current_url, publicOrigin + '/')
    assert.equal(events[0].properties.$pathname, '/')
    assert.equal(events[0].properties.$host, 'app.tryapparent.com')
    assert.equal(events[0].properties.acquisition_source, 'other')
    assert.equal(events[0].properties.acquisition_medium, 'unspecified')
    assert.match(String(events[0].properties.distinct_id), /^anon:[a-f0-9-]{36}$/)
    assert.deepEqual(errors, [])
    await context.close()

    const denied = await browser.newContext()
    await denied.route('**/*', async route => {
      const request = route.request(), url = new URL(request.url())
      if (url.origin === publicOrigin) {
        const response = await route.fetch({ url: localOrigin + url.pathname + url.search, maxRedirects: 0 })
        await route.fulfill({ response }); return
      }
      if (url.origin === 'https://eu.i.posthog.com') {
        assert.equal(url.href, 'https://eu.i.posthog.com/capture/')
        events.push(request.postDataJSON())
        await route.fulfill({ status: 200, headers: { 'Access-Control-Allow-Origin': '*' }, body: '{}' }); return
      }
      await route.abort()
    })
    const deniedPage = await denied.newPage()
    await deniedPage.goto(publicOrigin + '/')
    await deniedPage.locator('.pond-page').waitFor()
    const beforeRefusal = events.length
    await deniedPage.getByRole('button', { name: 'No thanks', exact: true }).click()
    await deniedPage.waitForFunction(() => JSON.parse(localStorage.getItem('apparent.product-analytics.v1') || 'null')?.choice === 'deny')
    await deniedPage.waitForTimeout(200)
    assert.equal(events.length, beforeRefusal, 'refusal prevents analytics sends')
    await denied.close()
  }
} finally {
  try {
    await browser?.close()
    if (server?.listening) await new Promise<void>(done => server!.close(() => done()))
  } finally { rmSync(root, { recursive: true, force: true }) }
}
