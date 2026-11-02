// Controlled synthetic API + exact packaged website. The API origin is build-specific.
// No real Google or Connect service; loopback builds are not production artifacts.
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFile, mkdir } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { resolve, extname } from 'node:path'
import { chromium } from 'playwright-core'
const root = resolve(process.env.CONNECT_ARTIFACT || 'dist-test')
const evidence = resolve(process.env.CONNECT_EVIDENCE || `${process.env.TMPDIR}/connect-web-evidence`)
await mkdir(evidence, { recursive: true })
const connectGraphs = []
async function closure(path, files = new Map()) {
  if (files.has(path)) return files
  const text = await readFile(`${root}${path}`, 'utf8')
  files.set(path, text)
  for (const match of text.matchAll(/["']((?:\/assets\/|\.\/|assets\/)[^"'\s]+\.(?:js|css))["']/g)) await closure(new URL(match[1], `http://artifact${path}`).pathname, files)
  return files
}
for (const route of ['connect', 'connect/device', 'connect/shares', 'hosted', 'hosted/device', 'hosted/shares']) {
  const html = await readFile(`${root}/${route}/index.html`, 'utf8')
  assert.match(html, /noindex, nofollow/); assert.match(html, /Checking your account/)
  assert.match(html, route.endsWith('/shares') ? /<title>Manage shared links — Apparent<\/title>/ : /<title>Connect(?: a device)? — Apparent<\/title>/)
  assert.match(html, /name="referrer" content="no-referrer"/)
  assert.match(html, /http-equiv="Cache-Control" content="no-store"/)
  const graph = await closure(`/${route}/index.html`)
  for (const [path, text] of graph) assert.doesNotMatch(text, /Visual feedback gets lost|pond-hero|pond-demo|pond-team|fonts\.bunny\.net/, path)
  connectGraphs.push(graph)
  assert(!html.includes('verified@example.com'))
}
// The route-only artifact contract separately rejects root HTML, sitemap and navigation.
// The packaged artifact must retain the exact approved art and original brand assets.
for (const asset of ['hosted/source-pond-v4.png', 'hosted/agent-pond-v4.png', 'hosted/agent-reading-v4.png', 'HostGrotesk-Variable.ttf', 'apparent-wordmark.svg']) {
  assert.deepEqual(await readFile(`${root}/${asset}`), await readFile(`public/${asset}`), `packaged ${asset}`)
}
for (const graph of connectGraphs) assert.deepEqual([...graph.keys()].filter(p => p.startsWith('/assets/')), [...connectGraphs[0].keys()].filter(p => p.startsWith('/assets/')))
const sharesCursor = '1793437200000:' + 'b'.repeat(32)
let scenario = {}, writes = [], calls = [], shareReads = [], approved = false, signedOut = false, interest = false, revoked = false
let uiOrigin = ''
const server = createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost')
  if (url.pathname === '/v1/session' && scenario.sessionFailure) return res.writeHead(403).end()
  if (url.pathname.startsWith('/v1/') || url.pathname === '/auth/logout') {
    res.setHeader('Access-Control-Allow-Origin', uiOrigin)
    res.setHeader('Access-Control-Allow-Credentials', 'true')
    res.setHeader('Access-Control-Allow-Headers', 'Content-Type, X-CSRF-Token')
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, DELETE, OPTIONS')
    if (req.method === 'OPTIONS') return res.writeHead(204).end()
    calls.push({ host: req.headers.host, cookie: req.headers.cookie })
    res.setHeader('Content-Type', 'application/json'); res.setHeader('Cache-Control', 'no-store')
    if (scenario.loading && url.pathname === '/v1/session') await new Promise(r => setTimeout(r, 650))
    if (scenario.unavailable) return res.writeHead(503).end('{"error":{"message":"synthetic-provider-secret"}}')
    if (req.method === 'POST' || req.method === 'DELETE') {
      const chunks = []; for await (const chunk of req) chunks.push(chunk)
      const body = JSON.parse(Buffer.concat(chunks).toString() || '{}')
      writes.push({ method: req.method, path: url.pathname, body, csrf: req.headers['x-csrf-token'], cookie: req.headers.cookie, origin: req.headers.origin })
      if (req.headers['x-csrf-token'] !== 'synthetic-csrf' || !req.headers.cookie?.includes('fixture=1') || req.headers.origin !== uiOrigin || scenario.forbidden) return res.writeHead(403).end('{}')
      await new Promise(r => setTimeout(r, 180))
      if (url.pathname === '/v1/device/approve') approved = true
      if (url.pathname === '/v1/waitlist') interest = body.interestedAtFiveDollars
      if (url.pathname === '/auth/logout') signedOut = true
      if (url.pathname === '/v1/shares/share-1' && !scenario.unconfirmed) revoked = true
      return res.end('{"ok":true}')
    }
    if (url.pathname === '/v1/shares') {
      shareReads.push(url.pathname + url.search)
      if (scenario.readbackUnavailable && writes.length && url.searchParams.has('before')) return res.writeHead(503).end('{}')
      const row = { id: 'share-1', captureId: 'a'.repeat(32), title: '<script>Captured title</script>', status: revoked ? 'revoked' : scenario.shareStatus || 'ready', createdAt: '2026-10-31T10:00:00Z', expiresAt: new Date(Date.now() + (scenario.past ? -1000 : 604800000)).toISOString(), url: scenario.unsafeUrl || 'https://share.example/s/synthetic' }
      if (scenario.paginated && !url.searchParams.has('before')) return res.end(JSON.stringify({ shares: Array.from({length:100}, (_,i) => ({ ...row, id:`older-${i}`, title:`Other recording ${i+1}`, status:'revoked' })), nextCursor: scenario.badCursor ? 'malformed' : sharesCursor }))
      if (scenario.paginated) assert.equal(url.searchParams.get('before'), sharesCursor)
      return res.end(JSON.stringify({ shares: scenario.empty ? [] : [{...row, ...(scenario.duplicateId ? {id:'older-0'} : {})}], nextCursor: scenario.repeatedCursor ? sharesCursor : null }))
    }
    if (url.pathname === '/v1/device') return res.end(JSON.stringify({ scope: approved && scenario.changedScope ? 'upload' : scenario.scope || 'read', status: approved && !scenario.unconfirmed ? 'approved' : scenario.status || 'pending', ...(scenario.withExpiry || scenario.clockExpiry ? { expiresAt: new Date(scenario.clockExpiry ? scenario.expiresAt : Date.now() + 600000).toISOString() } : {}) }))
    if (scenario.anonymous || signedOut) return res.end('{"authenticated":false,"csrfToken":null,"account":null,"slots":{"used":0,"limit":20,"remaining":20}}')
    return res.end(JSON.stringify({ authenticated: true, csrfToken: 'synthetic-csrf', account: { email: 'verified@example.com', beta: scenario.waitlist ? 'waitlist' : 'active', quotaBytes: 500000000, usedBytes: 0, waitlist: { interestedAtFiveDollars: interest } }, slots: { used: scenario.full ? 20 : 0, limit: scenario.invalid ? 50 : 20, remaining: scenario.full ? 0 : 20 }, accessToken: 'must-not-render' }))
  }
  let path = url.pathname
  if (/^\/(connect|hosted)(\/(device|shares))?\/?$/.test(path)) path = path.replace(/\/$/, '') + '/index.html'
  if (path === '/') path = '/index.html'
  const file = resolve(root, '.' + path)
  if (!file.startsWith(root + '/')) return res.writeHead(403).end()
  try { res.setHeader('Content-Type', ({ '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.ttf': 'font/ttf' })[extname(file)] || 'application/octet-stream'); res.end(await readFile(file)) }
  catch { res.writeHead(404).end() }
})
await new Promise(r => server.listen(Number(process.env.HOSTED_FIXTURE_PORT || 0), '127.0.0.1', r))
uiOrigin = `http://127.0.0.1:${server.address().port}`
const apiOrigin = process.env.HOSTED_TEST_API_ORIGIN || ''
const apiServer = apiOrigin ? createServer(server.listeners('request')[0]) : null
if (apiServer) await new Promise(r => apiServer.listen(Number(new URL(apiOrigin).port), '127.0.0.1', r))
console.log(`Synthetic hosted fixture at http://127.0.0.1:${server.address().port}/hosted/`)
if (process.env.HOSTED_FIXTURE_ONLY) await new Promise(() => {})
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || '/home/simon/.cache/ms-playwright/chromium-1228/chrome-linux64/chrome', headless: true, args: ['--no-sandbox'] })
const results = []
const routeResults = []
let viteServer
try {
  for (const test of [
    { name: 'session-empty-403', failure: 'cors' },
    { name: 'session-device-empty-403', failure: 'cors', device: true },
    { name: 'session-legacy-network-failure', failure: 'network', legacy: true },
    { name: 'session-legacy-device-invalid-json', failure: 'json', legacy: true, device: true },
  ]) {
    scenario = { scope: 'upload', sessionFailure: test.failure === 'cors' }; writes = []; calls = []; approved = false; signedOut = false
    const page = await browser.newPage({ viewport: { width: test.device ? 390 : 1100, height: 900 } })
    const requests = [], failures = []
    page.on('request', request => requests.push(request.url()))
    page.on('requestfailed', request => failures.push(request.url()))
    await page.context().addCookies([{ name: 'fixture', value: '1', url: uiOrigin }])
    if (test.failure !== 'cors') await page.route('**/v1/session', route => test.failure === 'network' ? route.abort('failed') : route.fulfill({
      status: 200,
      headers: { 'Content-Type': 'application/json', 'Access-Control-Allow-Origin': uiOrigin, 'Access-Control-Allow-Credentials': 'true' },
      body: 'invalid-json',
    }))
    const path = `/${test.legacy ? 'hosted' : 'connect'}${test.device ? '/device?code=SYNTHETIC' : '/'}`
    await page.goto(uiOrigin + path)
    await page.getByRole('alert').waitFor()
    assert.match(await page.getByRole('alert').innerText(), /We couldn’t check whether you’re signed in\./)
    assert.equal(await page.getByRole('link', { name: 'Continue with Google' }).getAttribute('href'), apiOrigin + '/auth/google/start?returnTo=' + encodeURIComponent(test.device ? '/connect/device?code=SYNTHETIC' : '/connect'))
    assert.equal(await page.getByRole('button', { name: /^Allow / }).count(), 0)
    assert.equal(await page.locator('.hosted-identity').count(), 0)
    assert.equal(requests.some(url => url.includes('/v1/device')), false)
    assert.equal(writes.length, 0)
    assert.equal(new URL(page.url()).pathname + new URL(page.url()).search, path)
    assert.equal((await page.context().cookies(uiOrigin)).find(cookie => cookie.name === 'fixture')?.value, '1')
    if (test.failure !== 'json') assert(failures.some(url => url.endsWith('/v1/session')), 'exercise a browser-level request failure')
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
    await page.evaluate(() => document.fonts.ready)
    const signIn = page.getByRole('link', { name: 'Continue with Google' })
    const retry = page.getByRole('button', { name: 'Try again', exact: true })
    const signInBox = await signIn.boundingBox(), retryBox = await retry.boundingBox()
    assert(signInBox.height >= 44 && retryBox.height >= 44, 'both recovery actions have full-size targets')
    const gap = Math.abs(signInBox.y - retryBox.y) < 2
      ? retryBox.x - signInBox.x - signInBox.width
      : retryBox.y - signInBox.y - signInBox.height
    assert(gap >= 11.5, 'recovery actions stay visibly separated, including when wrapped')
    assert.match(await signIn.evaluate(element => getComputedStyle(element).fontFamily), /Host Grotesk/)
    await page.screenshot({ path: `${evidence}/${test.name}.png`, fullPage: true })
    scenario.sessionFailure = false
    await page.unroute('**/v1/session')
    await retry.click()
    await page.waitForFunction(() => document.querySelector('.hosted-panel')?.getAttribute('aria-busy') === 'false')
    assert.equal(await page.getByRole('alert').count(), 0)
    assert.equal(await page.getByRole('link', { name: 'Continue with Google' }).count(), 0)
    assert.equal(writes.length, 0, 'retry never approves a device or logs out')
    if (test.device) assert.equal(await page.getByRole('button', { name: 'Allow uploads', exact: true }).isDisabled(), true, 'recovered upload preview still requires explicit consent')
    else assert.equal(await page.getByRole('button', { name: 'Connect', exact: true }).count(), 1)
    await page.close(); results.push(test.name)
  }
  for (const mode of ['dev', 'preview']) {
    const vite = await import('vite')
    viteServer = mode === 'dev' ? await vite.createServer({server:{port:0}}) : await vite.preview({build:{outDir:root},preview:{port:0}})
    if (mode === 'dev') await viteServer.listen()
    uiOrigin = `http://127.0.0.1:${viteServer.httpServer.address().port}`
    for (const route of ['connect','connect/device','connect/shares','hosted','hosted/device','hosted/shares']) for (const slash of ['', '/']) {
      scenario = {}; writes = []; calls = []
      const page = await browser.newPage(), errors = [], resources = []
      await page.route('**/*', route => new URL(route.request().url()).hostname === '127.0.0.1' ? route.continue() : route.abort())
      page.on('pageerror', e => errors.push(String(e)))
      page.on('request', r => resources.push({url:r.url(),type:r.resourceType()}))
      await page.context().addCookies([{name:'fixture',value:'1',url:uiOrigin}])
      const query = '?code=SYNTHETIC&extra=kept%2Bvalue'
      await page.goto(`${uiOrigin}/${route}${slash}${query}`)
      await page.waitForFunction(() => document.querySelector('.hosted-panel')?.getAttribute('aria-busy') === 'false')
      assert.equal(new URL(page.url()).search, query)
      if (route.endsWith('shares')) await page.getByRole('heading',{name:'Manage shared links'}).waitFor()
      else assert.equal(await page.getByRole('tab',{name:route.endsWith('device')?'Your agent':'This computer',exact:true}).getAttribute('aria-selected'),'true')
      assert.equal(writes.length,0); assert.deepEqual(errors,[])
      assert(calls.length > 0, `${mode} /${route}${slash}: ${await page.locator('main').innerText()}; API=${apiOrigin}; requests=${JSON.stringify(resources)}`)
      for (const r of resources) {
        assert.notEqual(new URL(r.url).hostname,'fonts.bunny.net')
        assert.doesNotMatch(new URL(r.url).pathname,/\/src\/(?:App|styles|pond-hero|cta|belief|Belief|pond|footer|github)/)
      }
      routeResults.push({mode,route:`/${route}${slash}`,resources}); await page.close()
    }
    await viteServer.close(); viteServer = null
  }
  uiOrigin = `http://127.0.0.1:${server.address().port}`
  for (const test of [
    { name: 'anonymous', anonymous: true, width: 1440 }, { name: 'anonymous-device', anonymous: true, device: true }, { name: 'loading', loading: true },
    { name: 'desktop-1440', width: 1440 }, { name: 'desktop-1280', width: 1280 }, { name: 'phone-390', width: 390 }, { name: 'empty-account', width: 320 }, { name: 'zoom-200-equivalent', width: 640, height: 450 },
    { name: 'full-waitlist', waitlist: true, full: true, width: 390 },
    { name: 'read-approval', device: true }, { name: 'upload-approval', device: true, scope: 'upload', width: 390, withExpiry: true },
    { name: 'expired', device: true, status: 'expired' }, { name: 'rejected', device: true, status: 'rejected' },
    { name: 'invalid-scope', device: true, scope: 'admin' }, { name: 'no-code', device: true, noCode: true },
    { name: 'csrf-denied', device: true, forbidden: true }, { name: 'unavailable', unavailable: true }, { name: 'invalid-count', invalid: true },
    { name: 'approved-read', device: true, status: 'approved' }, { name: 'approved-upload', device: true, scope: 'upload', status: 'approved', width: 320 },
    { name: 'unconfirmed-approval', device: true, unconfirmed: true }, { name: 'changed-scope-readback', device: true, changedScope: true },
    { name: 'clock-expiry', device: true, clockExpiry: true },
    { name: 'legacy-account', legacy: true }, { name: 'legacy-anonymous', legacy: true, anonymous: true },
    { name: 'legacy-device', legacy: true, device: true }, { name: 'legacy-anonymous-device', legacy: true, device: true, anonymous: true },
    { name: 'legacy-no-slash', legacy: true, noSlash: true }, { name: 'legacy-device-slash', legacy: true, device: true, deviceSlash: true }, { name: 'legacy-anonymous-device-slash', legacy: true, device: true, deviceSlash: true, anonymous: true },
    { name: 'canonical-no-slash', noSlash: true }, { name: 'canonical-device-slash', device: true, deviceSlash: true },
  ]) {
    scenario = test; writes = []; calls = []; approved = false; signedOut = false; interest = false
    scenario.expiresAt = Date.now() + 2500
    const page = await browser.newPage({ viewport: { width: test.width || 1100, height: test.height || 900 } })
    const errors = []; page.on('pageerror', e => errors.push(String(e)))
    await page.context().addCookies([{ name: 'fixture', value: '1', url: `http://127.0.0.1:${server.address().port}` }])
    await page.goto(`http://127.0.0.1:${server.address().port}/${test.legacy ? 'hosted' : 'connect'}${test.device ? `/device${test.deviceSlash ? '/' : ''}${test.noCode ? '' : '?code=SYNTHETIC'}` : `${test.noSlash ? '' : '/'}?returnTo=https://evil.example`}`)
    if (test.loading) { await page.getByRole('status').filter({ hasText: 'Checking your account' }).waitFor(); await page.screenshot({ path: `${evidence}/loading-pending.png` }) }
    await page.waitForFunction(() => document.querySelector('.hosted-panel')?.getAttribute('aria-busy') === 'false')
    assert.equal(writes.length, 0, 'never auto approve or mutate on load')
    if (!test.device && !test.waitlist && !test.unavailable && !test.invalid) {
      const computer = page.getByRole('tab', { name: 'This computer', exact: true })
      const agent = page.getByRole('tab', { name: 'Your agent', exact: true })
      assert.equal(await page.getByRole('heading', { name: 'Upload new captures', exact: true }).count(), 1)
      await page.evaluate(() => document.fonts.ready)
      const geometry = await page.evaluate(() => {
        const sheet = document.querySelector('.hosted-main'), title = document.querySelector('h1'), cta = document.querySelector('.hosted-main-action')
        return { width: sheet.getBoundingClientRect().width, radius: getComputedStyle(sheet).borderRadius, title: getComputedStyle(title).fontSize, ctaRadius: getComputedStyle(cta).borderRadius, ctaColor: getComputedStyle(cta).backgroundColor, font: getComputedStyle(sheet).fontFamily, color: getComputedStyle(sheet).color }
      })
      assert.equal(geometry.width, test.width >= 760 ? 720 : (test.width || 1100) > 550 ? Math.min(720, (test.width || 1100) - 40) : test.width - 24)
      assert.equal(geometry.radius, '0px'); assert.equal(geometry.ctaRadius, '0px'); assert.equal(geometry.ctaColor, 'rgb(25, 22, 29)')
      assert.equal(geometry.color, 'rgb(2, 1, 2)', 'retain inherited pond text color, not the hosted variable fallback')
      assert.equal(geometry.title, test.width <= 550 ? '32px' : '36px'); assert.match(geometry.font, /Host Grotesk/)
      await page.screenshot({ path: `${evidence}/${test.name}-computer.png`, fullPage: true })
      await computer.focus(); await page.keyboard.press('ArrowRight')
      assert.equal(await agent.getAttribute('aria-selected'), 'true'); assert.equal(await agent.evaluate(e => e === document.activeElement), true)
      assert.equal(await page.getByRole('heading', { name: 'Read your captures', exact: true }).count(), 1)
      assert.equal(await page.locator('.hosted-agent').count(), 0)
      assert.match(await page.locator('.hosted-source').getAttribute('src'), /agent-reading-v4/)
      await page.waitForFunction(() => [...document.querySelectorAll('.hosted-scene img, .hosted-header img')].every(e => e.complete && e.naturalWidth > 0))
      const lock = page.getByRole('button', { name: 'View separate agent read permission details' })
      assert.equal(await lock.evaluate(e => { const r = e.getBoundingClientRect(); return document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2) === e || e.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2)) }), true)
      await page.screenshot({ path: `${evidence}/${test.name}-agent.png`, fullPage: true })
      await lock.click(); await page.getByRole('dialog').waitFor()
      assert.match(await page.getByRole('dialog').innerText(), /all current and future uploads/)
      await page.keyboard.press('Escape'); assert.equal(await lock.evaluate(e => e === document.activeElement), true)
      const details = page.locator('.hosted-details > summary')
      await details.focus(); await page.keyboard.press('Enter')
      assert.equal(await page.locator('.hosted-details').getAttribute('open'), '')
      await page.locator('.hosted-details .hosted-limits > summary').click()
      const limits = await page.locator('.hosted-details').innerText()
      assert.match(limits, /500,000,000[\s\S]*90,000,000[\s\S]*20 accounts[\s\S]*renew each approved device connection automatically/)
      assert.match(limits, /Revoked or expired renewal requires CLI approval again/)
      assert.match(limits, /Older apps need an update and one new approval; website sign-in does not renew device access/)
      assert.doesNotMatch(limits, /no automatic renewal/)
      await page.getByRole('button', { name: 'Review access', exact: true }).click()
      if (!test.anonymous) {
        assert.equal(await page.getByRole('dialog').locator('pre code').innerText(), `apparent connect login --server ${apiOrigin || 'https://YOUR-HOSTED-API-ORIGIN'} --scope read --no-browser`)
        assert.match(await page.getByRole('dialog').innerText(), /Update Apparent to use apparent connect\. Older apps can use apparent cloud; it remains a compatibility alias\./)
      }
      await page.keyboard.press('Escape')
      await agent.focus(); await page.keyboard.press('Home')
      assert.equal(await computer.getAttribute('aria-selected'), 'true')
      assert.match(await page.locator('.hosted-source').getAttribute('src'), /source-pond-v4/)
      assert.equal(await page.locator('.hosted-agent').count(), 1)
      assert.equal(await page.locator('.hosted-details').getAttribute('open'), null)
      const connect = page.getByRole('button', { name: 'Connect', exact: true })
      await connect.click()
      if (!test.anonymous) {
        assert.deepEqual(await page.getByRole('dialog').locator('pre code').allInnerTexts(), [
          `apparent connect login --server ${apiOrigin || 'https://YOUR-HOSTED-API-ORIGIN'} --scope upload --consent-upload`,
          'apparent connect status\napparent connect status --account',
        ])
        assert.match(await page.getByRole('dialog').innerText(), /Update Apparent to use apparent connect\. Older apps can use apparent cloud; it remains a compatibility alias\./)
      }
      assert.equal(writes.length, 0, 'tabs, details and setup never mutate')
      if (!test.anonymous) { await page.screenshot({ path: `${evidence}/${test.name}-setup.png`, fullPage: true }); await page.keyboard.press('Escape'); assert.equal(await connect.evaluate(e => e === document.activeElement), true) }
    }
    if (test.anonymous) { const href = await page.getByRole('link', { name: 'Continue with Google' }).getAttribute('href'); assert.equal(href, apiOrigin + (test.device ? '/auth/google/start?returnTo=%2Fconnect%2Fdevice%3Fcode%3DSYNTHETIC' : '/auth/google/start?returnTo=%2Fconnect')) }
    if (test.waitlist) {
      assert.match(await page.locator('main').innerText(), /beta is full/)
      await page.getByRole('checkbox').check()
      await page.locator('form').evaluate(e => { e.requestSubmit(); e.requestSubmit() })
      await page.waitForFunction(() => document.querySelector('.hosted-panel')?.getAttribute('aria-busy') === 'false')
      assert.equal(writes.length, 1); assert.deepEqual(writes[0].body, { interestedAtFiveDollars: true })
    }
    const allow = page.getByRole('button', { name: /^Allow / })
    if (test.clockExpiry) {
      await page.getByRole('heading', { name: 'That code expired' }).waitFor()
      assert.equal(await allow.count(), 0); assert.equal(writes.length, 0)
    } else if (test.device && !test.anonymous && !test.status && !test.noCode && test.scope !== 'admin') {
      assert.equal(await allow.count(), 1)
      const selectedTab = page.getByRole('tab', { name: test.scope === 'upload' ? 'This computer' : 'Your agent', exact: true })
      assert.equal(await selectedTab.getAttribute('aria-selected'), 'true')
      assert.equal(await page.getByRole('tab', { name: test.scope === 'upload' ? 'Your agent' : 'This computer', exact: true }).isDisabled(), true)
      await selectedTab.focus(); await page.keyboard.press('ArrowRight'); assert.equal(await selectedTab.getAttribute('aria-selected'), 'true')
      if (test.scope === 'upload') { assert.equal(await allow.isDisabled(), true); await page.getByRole('checkbox').check(); assert.equal(writes.length, 0) }
      assert.match(await page.locator('main').innerText(), test.scope === 'upload' ? /add, read and delete/ : /All your current and future uploads/)
      await page.screenshot({ path: `${evidence}/${test.name}-pending.png`, fullPage: true })
      await allow.evaluate(e => { e.click(); e.click() })
      await page.waitForFunction(() => document.querySelector('.hosted-panel')?.getAttribute('aria-busy') === 'false')
      assert.equal(writes.length, 1); assert.equal(writes[0].csrf, 'synthetic-csrf'); assert.deepEqual(writes[0].body, { userCode: 'SYNTHETIC' })
      assert.match(await page.locator('main').innerText(), test.forbidden ? /not authorized/ : test.unconfirmed || test.changedScope ? /Approval could not be confirmed/ : /Approval granted[\s\S]*CLI must finish/)
    } else if (test.device) assert.equal(await allow.count(), 0)
    if (test.unavailable || test.invalid || test.scope === 'admin') assert.equal(await page.getByRole('alert').count(), 1)
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
    assert.equal(await page.evaluate(() => [...document.querySelectorAll('main *, dialog[open] *')].filter(e => e.getClientRects().length && getComputedStyle(e).position !== 'absolute').some(e => { const r = e.getBoundingClientRect(); return r.width > innerWidth + 1 || r.left < -1 || r.right > innerWidth + 1 })), false, 'no clipped content hidden by global overflow CSS')
    const manage = page.getByRole('link', {name:'Manage shared links',exact:true})
    const hasManagement = !test.anonymous && !test.waitlist && !test.unavailable && !test.invalid
    assert.equal(await manage.count(), hasManagement ? 1 : 0)
    if (hasManagement) { assert.equal(await manage.getAttribute('href'),'/connect/shares'); assert.equal(await page.locator('.hosted-identity .hosted-manage-link').count(),1) }
    const productText = await page.locator('body').evaluate(body => {
      const link = body.querySelector('.hosted-identity a.hosted-manage-link[href="/connect/shares"]')
      return link?.innerText === 'Manage shared links' ? body.innerText.replace(link.innerText,'') : body.innerText
    })
    assert.doesNotMatch(productText, /Uploads are on|Read access is on|Manage|simulated|Try a state|\bHosting\b|\bHosted\b/)
    assert.equal(await page.evaluate(() => localStorage.length + sessionStorage.length), 0)
    assert(!(await page.content()).includes('must-not-render'))
    assert(!(await page.content()).includes('synthetic-csrf'))
    assert(!(await page.content()).includes('synthetic-provider-secret'), 'raw provider failure bodies never reach the UI')
    assert.deepEqual(errors, [])
    await page.screenshot({ path: `${evidence}/${test.name}.png`, fullPage: true })
    if (test.name === 'empty-account') {
      await page.getByRole('button', { name: 'Log out' }).click()
      await page.getByRole('button', { name: 'Connect', exact: true }).click()
      await page.getByRole('link', { name: 'Continue with Google' }).waitFor()
      assert.equal(writes[0].path, '/auth/logout'); assert.equal(writes[0].csrf, 'synthetic-csrf')
    }
    assert(calls.length > 0)
    for (const call of calls) {
      assert.equal(call.host, apiOrigin ? new URL(apiOrigin).host : `127.0.0.1:${server.address().port}`, 'fetches must use configured API origin')
      assert.match(call.cookie || '', /fixture=1/, 'credentialed API fetch retains the host cookie')
    }
    results.push(test.name); await page.close()
  }
  assert.equal(results.length, 36, 'all account/device scenarios must execute')
  for (const test of [
    {name:'shares',width:390},{name:'shares-320',width:320},{name:'shares-empty',empty:true},
    {name:'shares-denied',forbidden:true},{name:'shares-unconfirmed',unconfirmed:true},
    {name:'shares-page-two',paginated:true},{name:'shares-page-two-unconfirmed',paginated:true,unconfirmed:true},
    {name:'shares-page-two-readback-unavailable',paginated:true,readbackUnavailable:true},
    {name:'legacy-shares',legacy:true},{name:'legacy-shares-slash',legacy:true,slash:true},{name:'canonical-shares-slash',slash:true},
    {name:'shares-anonymous',anonymous:true},{name:'shares-loading',loading:true},{name:'shares-unavailable',unavailable:true},
    {name:'shares-malformed-cursor',paginated:true,badCursor:true},{name:'shares-repeated-cursor',paginated:true,repeatedCursor:true},
    {name:'shares-duplicate-id',paginated:true,duplicateId:true},
    ...['pending','ready','paused','failed','expired','revoked','deleted'].map(shareStatus=>({name:`shares-status-${shareStatus}`,shareStatus,inspectOnly:true})),
    {name:'shares-past',past:true,inspectOnly:true},{name:'shares-http',unsafeUrl:'http://share.example/s/x'},
    {name:'shares-credentials',unsafeUrl:'https://user:password@share.example/s/x'},
  ]) {
    scenario=test; writes=[]; calls=[]; shareReads=[]; signedOut=false; revoked=false
    const page=await browser.newPage({viewport:{width:test.width||1100,height:900}}), errors=[]
    page.on('pageerror',e=>errors.push(String(e)))
    await page.context().addCookies([{name:'fixture',value:'1',url:uiOrigin}])
    await page.goto(`${uiOrigin}/${test.legacy?'hosted':'connect'}/shares${test.slash?'/':''}?returnTo=https://evil.example`)
    if(test.loading) { await page.getByRole('status').waitFor(); assert.equal(writes.length,0) }
    await page.waitForFunction(()=>document.querySelector('.hosted-panel')?.getAttribute('aria-busy')==='false')
    assert.equal(writes.length,0)
    assert.equal(await page.getByRole('link',{name:'Account',exact:true}).getAttribute('href'),'/connect/')
    const invalid = test.unavailable || test.badCursor || test.repeatedCursor || test.duplicateId || test.unsafeUrl
    if(test.anonymous) {
      assert.equal(await page.getByRole('link',{name:'Continue with Google'}).getAttribute('href'),apiOrigin+'/auth/google/start?returnTo=%2Fconnect')
      assert.equal(shareReads.length,0)
    } else if(invalid) {
      assert.equal(await page.getByRole('alert').count(),1)
      assert.equal(await page.locator('.hosted-shares > li').count(),0,'never publish a partial/invalid list')
    } else if(test.empty) assert.match(await page.locator('main').innerText(),/No shared links/)
    else {
      if(test.paginated) assert.equal(await page.locator('.hosted-shares > li').count(),101)
      assert.equal(await page.getByRole('heading',{name:'<script>Captured title</script>',exact:true}).count(),1)
      const link=page.getByRole('link',{name:'Open shared recording'})
      const visible=!test.past && !['expired','revoked','deleted'].includes(test.shareStatus)
      assert.equal(await link.count(),visible?1:0)
      if(visible) { assert.equal(await link.getAttribute('href'),'https://share.example/s/synthetic'); assert.equal(await link.getAttribute('referrerpolicy'),'no-referrer') }
      if(!test.inspectOnly) {
        await page.getByRole('button',{name:'Stop sharing'}).evaluate(e=>{e.click();e.click()})
        await page.waitForFunction(()=>document.querySelector('.hosted-panel')?.getAttribute('aria-busy')==='false')
        assert.equal(writes.length,1); assert.equal(writes[0].method,'DELETE'); assert.equal(writes[0].path,'/v1/shares/share-1')
        assert.equal(writes[0].csrf,'synthetic-csrf'); assert.equal(writes[0].origin,uiOrigin); assert.match(writes[0].cookie||'',/fixture=1/)
        if(test.forbidden||test.unconfirmed||test.readbackUnavailable) assert.equal(await page.getByRole('alert').count(),1)
        else {assert.equal(await link.count(),0);assert.match(await page.locator('main').innerText(),/revoked/)}
        if(test.paginated) assert.equal(shareReads.filter(p=>p==='/v1/shares?before='+encodeURIComponent(sharesCursor)).length,2,'read back the entire list, including page two')
      }
    }
    assert.equal(await page.locator('script').filter({hasText:'Captured title'}).count(),0)
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false)
    assert.equal(await page.evaluate(()=>localStorage.length+sessionStorage.length),0)
    assert.doesNotMatch(await page.content(),/synthetic-csrf|must-not-render|synthetic-provider-secret/)
    assert.deepEqual(errors,[])
    for(const call of calls) {assert.equal(call.host,new URL(apiOrigin).host);assert.match(call.cookie||'',/fixture=1/)}
    await page.screenshot({path:`${evidence}/${test.name}.png`,fullPage:!test.paginated}); await page.close(); results.push(test.name)
  }
  // StrictMode starts two loads. Hold the first session until the second has
  // published its list; a stale success or failure must not replace current UI.
  viteServer=await (await import('vite')).createServer({server:{port:0}})
  await viteServer.listen()
  uiOrigin=`http://127.0.0.1:${viteServer.httpServer.address().port}`
  for (const staleFailure of [false, true]) {
    scenario={}; writes=[]; calls=[]; shareReads=[]; signedOut=false; revoked=false
    const page=await browser.newPage()
    await page.context().addCookies([{name:'fixture',value:'1',url:uiOrigin}])
    let release, intercepted=0
    const held = new Promise(resolve=>{release=resolve})
    await page.route('**/v1/session',async route=>{
      if (++intercepted !== 1) return route.continue()
      await held
      await route.fulfill({status:staleFailure?503:200,contentType:'application/json',body:staleFailure?'{}':'{"authenticated":false}'})
    })
    await page.goto(`${uiOrigin}/connect/shares`)
    await page.getByRole('link',{name:'Open shared recording'}).waitFor()
    const staleResponse=page.waitForResponse(r=>r.url().endsWith('/v1/session'))
    release(); await staleResponse
    await page.waitForTimeout(100)
    assert(intercepted>=2,'exercise an actual superseded load')
    assert.equal(await page.getByRole('link',{name:'Open shared recording'}).count(),1)
    assert.equal(await page.getByRole('alert').count(),0)
    assert.equal(await page.getByRole('link',{name:'Continue with Google'}).count(),0)
    assert.equal(writes.length,0)
    await page.close();results.push(staleFailure?'shares-stale-load-failure':'shares-stale-load-success')
  }
  await viteServer.close();viteServer=null
  uiOrigin=`http://127.0.0.1:${server.address().port}`
  scenario={unavailable:true}; writes=[]; signedOut=false; revoked=false
  const recovery=await browser.newPage()
  await recovery.context().addCookies([{name:'fixture',value:'1',url:uiOrigin}])
  await recovery.goto(`${uiOrigin}/connect/shares`)
  await recovery.getByRole('alert').waitFor()
  scenario={}
  await recovery.getByRole('button',{name:'Reload shared links'}).click()
  await recovery.getByRole('link',{name:'Open shared recording'}).waitFor()
  assert.equal(await recovery.getByRole('alert').count(),0);assert.equal(writes.length,0)
  await recovery.close();results.push('shares-load-recovery')
  assert.equal(results.length,66,'all auth and management scenarios must execute')
  assert.equal(routeResults.length,24,'all real dev/preview route variants must execute')
  const receipt = { synthetic: true, routes:routeResults, closures:connectGraphs.map(g=>[...g.keys()]), artifact: root, apiOrigin: apiOrigin || 'same-origin', passed: results, count: results.length, screenshots: evidence, assets: Object.fromEntries(await Promise.all(['hosted/source-pond-v4.png', 'hosted/agent-pond-v4.png', 'hosted/agent-reading-v4.png', 'HostGrotesk-Variable.ttf', 'apparent-wordmark.svg'].map(async name => [name, createHash('sha256').update(await readFile(`${root}/${name}`)).digest('hex')]))), limits: 'Controlled synthetic API only; not live OAuth, service, native, or deployment proof. 640×450 CSS pixels is a 200%-equivalent layout, not browser zoom.' }
  await import('node:fs/promises').then(fs => fs.writeFile(`${evidence}/receipt.json`, JSON.stringify(receipt, null, 2)))
  console.log(JSON.stringify(receipt, null, 2))
} finally { await browser.close(); if (viteServer) await viteServer.close(); if (apiServer) await new Promise(r => apiServer.close(r)); await new Promise(r => server.close(r)) }