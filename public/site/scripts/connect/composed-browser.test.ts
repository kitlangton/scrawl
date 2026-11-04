/** Exercise the composed production bytes with synthetic intercepted transport. */
import assert from 'node:assert/strict'
import test from 'node:test'
import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'
import { resolve, extname } from 'node:path'
import { chromium } from 'playwright-core'
import { routes } from './compose-connect.mjs'

test('composed routes, queries and analytics work without rebuilding Connect', async () => {
  const root = resolve(process.env.SITE_ARTIFACT || 'dist-apparent')
  const publicOrigin = 'https://app.tryapparent.com', apiOrigin = 'https://hosted.tryapparent.com'
  const redirects = (await readFile(resolve(root, '_redirects'), 'utf8')).trim().split('\n').map(line => line.split(' '))
  const server = createServer(async (request, response) => {
    const url = new URL(request.url!, publicOrigin)
    const redirect = redirects.find(([path]) => path === url.pathname)
    if (redirect) { response.writeHead(Number(redirect[2]), { Location: redirect[1] + url.search }); response.end(); return }
    let path = url.pathname
    if (path === '/') path = '/index.html'
    else if (routes.some(route => path === `/${route}` || path === `/${route}/`)) path = path.replace(/\/$/, '') + '/index.html'
    const file = resolve(root, '.' + path)
    try {
      assert(file.startsWith(root + '/'))
      const bytes = await readFile(file)
      response.setHeader('Content-Type', ({ '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.png': 'image/png', '.ttf': 'font/ttf', '.mp4': 'video/mp4', '.jpg': 'image/jpeg' } as Record<string, string>)[extname(file)] || 'application/octet-stream')
      response.end(bytes)
    } catch { response.writeHead(404, { 'Content-Type': 'text/html' }); response.end(await readFile(resolve(root, '404.html'))) }
  })
  await new Promise<void>(done => server.listen(0, '127.0.0.1', done))
  const local = `http://127.0.0.1:${(server.address() as import('node:net').AddressInfo).port}`
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), headless: true, args: ['--no-sandbox', '--mute-audio'] })
  const context = await browser.newContext(), events: any[] = [], calls: string[] = [], errors: string[] = []
  try {
    await context.route('**/*', async route => {
      const request = route.request(), url = new URL(request.url())
      if (url.origin === publicOrigin) {
        const response = await route.fetch({ url: local + url.pathname + url.search, maxRedirects: 0 })
        await route.fulfill({ response }); return
      }
      if (url.origin === apiOrigin && ['/v1/session', '/v1/analytics/consent'].includes(url.pathname)) {
        assert.equal(request.method(), 'GET', 'anonymous smoke must never write account state')
        calls.push(url.href)
        await route.fulfill({ status: 200, headers: { 'Content-Type': 'application/json', 'Access-Control-Allow-Origin': publicOrigin, 'Access-Control-Allow-Credentials': 'true', 'Cache-Control': 'no-store' }, body: JSON.stringify(url.pathname === '/v1/session' ? { authenticated: false, csrfToken: null, account: null, slots: { used: 0, limit: 20, remaining: 20 } } : { available: false, consented: false, revision: null, noticeVersion: 'product-analytics-v1' }) }); return
      }
      if (url.origin === 'https://eu.i.posthog.com') {
        assert.equal(url.href, 'https://eu.i.posthog.com/capture/')
        const event = request.postDataJSON(); events.push(event)
        assert.doesNotMatch(JSON.stringify(event), /NEVER-SEND|capture-secret|userCode|csrf|@/)
        assert.equal(event.properties.environment, 'production')
        await route.fulfill({ status: 200, headers: { 'Access-Control-Allow-Origin': '*' }, body: '{}' }); return
      }
      await route.abort() // Includes Google, release hosts and every real external transport.
    })
    const page = await context.newPage(); page.on('pageerror', error => errors.push(error.message))
    await page.goto(publicOrigin + '/?utm_source=NEVER-SEND#capture-secret')
    await page.locator('.pond-page').waitFor()
    assert.equal(events.length, 0, 'analytics requires prior browser choice')
    const allow = page.getByRole('button', { name: 'Allow analytics', exact: true })
    const configured = Boolean(process.env.PUBLIC_POSTHOG_TOKEN) && process.env.PUBLIC_POSTHOG_ENABLED !== 'false'
    if (configured) {
      await allow.click()
      await assert.doesNotReject(async () => { await page.waitForFunction(() => localStorage.getItem('apparent.product-analytics.v1') !== null) })
    }
    for (const route of routes) {
      const response = await page.goto(`${publicOrigin}/${route}?userCode=NEVER-SEND`)
      assert.equal(response?.status(), 200)
      await page.locator('.hosted-page').waitFor()
      await page.getByText('Checking your account', { exact: false }).waitFor({ state: 'hidden' })
      assert.equal(new URL(page.url()).searchParams.get('userCode'), 'NEVER-SEND')
      assert.equal(new URL(page.url()).pathname.replace(/\/$/, ''), '/' + route.replace(/^hosted/, 'connect'))
      assert.equal(await page.locator('link[rel=canonical]').count(), 0)
      assert.equal(await page.locator('meta[name=robots]').getAttribute('content'), 'noindex, nofollow')
      assert.equal(await page.locator('.pond-hero').count(), 0)
    }
    assert(calls.length >= routes.length, 'all composed routes reach the configured API')
    if (configured) assert(events.length > 0, 'enabled production browser stream uses intercepted transport after consent')
    else assert.equal(events.length, 0)
    assert.equal((await page.goto(publicOrigin + '/private-unknown'))?.status(), 404)
    assert.deepEqual(errors, [])
  } finally { await context.close(); await browser.close(); await new Promise<void>(done => server.close(() => done())) }
})
