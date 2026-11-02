// Controlled synthetic API + exact packaged website. The API origin is build-specific.
// No real Google or Connect service; loopback builds are not production artifacts.
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFile, mkdir, access } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { resolve, extname } from 'node:path'
import { chromium } from 'playwright-core'
import { renderShare, shareViewScript, shareViewStyles } from '../../../../connect/service/src/share-view.ts'
const root = resolve(process.env.SITE_ARTIFACT || 'dist-apparent')
const evidence = resolve(process.env.SITE_EVIDENCE || `${process.env.TMPDIR}/hosted-web-evidence`)
await mkdir(evidence, { recursive: true })
// Traverse emitted imports, modulepreloads, CSS links and Vite preload maps.
// A different entry filename alone does not prove an isolated browser graph.
async function closure(document) {
  const files = new Map()
  async function visit(path) {
    if (files.has(path)) return
    const text = await readFile(`${root}${path}`, 'utf8')
    files.set(path, text)
    for (const match of text.matchAll(/["']((?:\/assets\/|\.\/|assets\/)[^"'\s]+\.(?:js|css))["']/g)) {
      await visit(new URL(match[1], `http://artifact${path}`).pathname)
    }
  }
  await visit(document)
  return files
}
const landingGraph = await closure('/index.html')
const connectGraphs = []
for (const route of ['connect', 'connect/device', 'connect/shares', 'hosted', 'hosted/device', 'hosted/shares']) {
  const html = await readFile(`${root}/${route}/index.html`, 'utf8')
  assert.match(html, /noindex, nofollow/); assert.match(html, /Checking your account/)
  assert.match(html, route.endsWith('/shares') ? /<title>Manage shared links — Apparent<\/title>/ : /<title>Connect(?: a device)? — Apparent<\/title>/)
  assert.match(html, /name="referrer" content="no-referrer"/)
  assert.match(html, /http-equiv="Cache-Control" content="no-store"/)
  if (route.endsWith('/shares')) {
    assert.match(html, /Manage shared links/)
    assert.match(html, /href="\/connect\/"/)
  }
  assert(!html.includes('verified@example.com'))
  const graph = await closure(`/${route}/index.html`)
  assert.doesNotMatch(html, /fonts\.bunny\.net|rel="canonical"|property="og:|name="twitter:/)
  for (const [path, text] of graph) {
    assert.doesNotMatch(text, /Visual feedback gets lost|pond-hero|pond-demo|pond-team/, `${route}: marketing closure in ${path}`)
  }
  connectGraphs.push(graph)
}
for (const [path, text] of landingGraph) assert.doesNotMatch(text, /X-CSRF-Token|hosted-panel|hosted-page/, `landing includes Connect/API closure: ${path}`)
const connectGraph = connectGraphs[0]
for (const graph of connectGraphs) assert.deepEqual([...graph.keys()].filter(p => p.startsWith('/assets/')), [...connectGraph.keys()].filter(p => p.startsWith('/assets/')))
// Shared React runtime is allowed; application-owned JS/CSS must be disjoint.
for (const [path, text] of connectGraph) if (landingGraph.has(path)) {
  assert.match(text, /react/i, `only the React runtime may be shared: ${path}`)
  assert.doesNotMatch(text, /hosted-|pond-|X-CSRF-Token/)
}
await assert.rejects(access(`${root}/connect.html`), 'temporary HTML input must not be published')
assert(!(await readFile(`${root}/index.html`, 'utf8')).includes('href="/hosted'))
assert(!(await readFile(`${root}/sitemap.xml`, 'utf8')).includes('/hosted'))
const sharesCursor = '1793437200000:' + 'b'.repeat(32)
let scenario = {}, writes = [], calls = [], shareReads = [], approved = false, signedOut = false, interest = false, revoked = false, recipientStatus = 'ready', recipientUploadedBytes = 5
assert(!(await readFile(`${root}/index.html`, 'utf8')).includes('href="/connect'))
assert(!(await readFile(`${root}/sitemap.xml`, 'utf8')).includes('/connect'))
// The packaged artifact must retain the exact approved art and original brand assets.
// Public source groups these five served paths under assets/{hosted,fonts,brand}.
const approvedAssets = new Map([
  ['hosted/source-pond-v4.png', 'assets/hosted/source-pond-v4.png'],
  ['hosted/agent-pond-v4.png', 'assets/hosted/agent-pond-v4.png'],
  ['hosted/agent-reading-v4.png', 'assets/hosted/agent-reading-v4.png'],
  ['HostGrotesk-Variable.ttf', 'assets/fonts/HostGrotesk-Variable.ttf'],
  ['apparent-wordmark.svg', 'assets/brand/apparent-wordmark.svg'],
])
for (const [asset, source] of approvedAssets) {
  assert.deepEqual(await readFile(`${root}/${asset}`), await readFile(`public/${source}`), `packaged ${asset}`)
}
let uiOrigin = ''
const server = createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost')
  if (url.pathname === '/share-view.js') { res.setHeader('Content-Type','text/javascript'); return res.end(shareViewScript) }
  if (url.pathname === '/share-view.css') { res.setHeader('Content-Type','text/css'); return res.end(shareViewStyles) }
  if (url.pathname === '/s/synthetic/manifest.json') { res.setHeader('Content-Type','application/json'); return res.end(JSON.stringify({status:recipientStatus,uploadedBytes:recipientUploadedBytes,totalBytes:10})) }
  if (url.pathname === '/s/synthetic') {
    res.setHeader('Content-Type','text/html');res.setHeader('Content-Security-Policy',"default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; media-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'")
    return res.end(renderShare({url:`http://127.0.0.1:${server.address().port}/s/synthetic`,status:recipientStatus,title:'Feedback <script>window.injected=true</script>',summary:'Synthetic summary <img src=x onerror=evil()>',transcript:'[00:02] Move this button.',uploadedBytes:recipientStatus==='ready'?10:5,totalBytes:10,files:[{path:'recording.mp4',size:10,sha256:'a'.repeat(64),url:`http://127.0.0.1:${server.address().port}/s/synthetic/files/recording.mp4`}]}))
  }
  if (url.pathname.startsWith('/v1/') || url.pathname === '/auth/logout') {
    res.setHeader('Access-Control-Allow-Origin', uiOrigin)
    res.setHeader('Access-Control-Allow-Credentials', 'true')
    res.setHeader('Access-Control-Allow-Headers', 'Content-Type, X-CSRF-Token')
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, DELETE, OPTIONS')
    if (req.method === 'OPTIONS') return res.writeHead(204).end()
    calls.push({ host: req.headers.host, cookie: req.headers.cookie })
    res.setHeader('Content-Type', 'application/json'); res.setHeader('Cache-Control', 'no-store')
    if (scenario.loading && url.pathname === '/v1/session') await new Promise(r => setTimeout(r, 650))
    if (scenario.unavailable) return res.writeHead(503).end('{}')
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
      if (scenario.paginated && !url.searchParams.has('before')) return res.end(JSON.stringify({shares:Array.from({length:100},(_,i)=>({id:`older-${i}`,captureId:'b'.repeat(32),title:`Other recording ${i+1}`,status:'revoked',createdAt:'2026-10-31T10:00:00Z',expiresAt:new Date(Date.now()+604800000).toISOString()})),nextCursor:sharesCursor}))
      if (scenario.paginated) assert.equal(url.searchParams.get('before'),sharesCursor)
      return res.end(JSON.stringify({shares: scenario.empty ? [] : [{id:'share-1',captureId:'a'.repeat(32),title:'<script>Captured title</script>',status:revoked?'revoked':'ready',createdAt:'2026-10-31T10:00:00Z',expiresAt:new Date(Date.now()+604800000).toISOString(),...(!revoked ? {url:'https://share.example/s/synthetic'} : {})}],nextCursor:null}))
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
  catch { res.setHeader('Content-Type', 'text/html'); res.writeHead(404).end(await readFile(`${root}/404.html`)) }
})
await new Promise(r => server.listen(Number(process.env.HOSTED_FIXTURE_PORT || 0), '127.0.0.1', r))
uiOrigin = `http://127.0.0.1:${server.address().port}`
const apiOrigin = process.env.HOSTED_TEST_API_ORIGIN || ''
const apiServer = apiOrigin ? createServer(server.listeners('request')[0]) : null
if (apiServer) await new Promise(r => apiServer.listen(Number(new URL(apiOrigin).port), '127.0.0.1', r))
console.log(`Synthetic Connect fixture at http://127.0.0.1:${server.address().port}/connect/`)
if (process.env.HOSTED_FIXTURE_ONLY) await new Promise(() => {})
const mode = process.env.HOSTED_TEST_MODE || 'package'
assert(['package', 'dev', 'preview'].includes(mode))
let viteServer
if (mode !== 'package') {
  const vite = await import('vite')
  // Public no longer imports Connect. Dev/preview exercise Connect's own server;
  // the composed homepage still comes from the integration fixture below.
  const config = { root: resolve('../../connect/web'), configFile: resolve('../../connect/web/vite.config.ts'), build: { outDir: resolve('../../connect/web/dist-test') } }
  viteServer = mode === 'dev'
    ? await vite.createServer({ ...config, server: { host: '127.0.0.1', port: 0 } })
    : await vite.preview({ ...config, preview: { host: '127.0.0.1', port: 0 } })
  if (mode === 'dev') await viteServer.listen()
  uiOrigin = `http://127.0.0.1:${viteServer.httpServer.address().port}`
}
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || '/home/simon/.cache/ms-playwright/chromium-1228/chrome-linux64/chrome', headless: true, args: ['--no-sandbox'] })
const results = []
const routeResults = []
function assertResources(resources, connect) {
  for (const { url, type } of resources) {
    const resource = new URL(url), path = resource.pathname
    if (connect) assert.notEqual(resource.hostname, 'fonts.bunny.net', 'Connect must not request the demo font service')
    if (!['script', 'stylesheet'].includes(type)) continue
    if (mode === 'dev') {
      assert.doesNotMatch(path, connect ? /\/src\/(?:App|styles|pond-hero|cta|belief|Belief|pond|footer|github)/ : /\/src\/(?:Hosted|SharedLinks|api)/, `wrong dev resource owner: ${url}`)
    } else {
      const other = connect ? landingGraph : connectGraph
      const own = connect ? connectGraph : landingGraph
      assert(!other.has(path) || own.has(path), `wrong built resource owner: ${url}`)
      if (path.startsWith('/assets/')) assert(own.has(path), `resource outside browser closure: ${url}`)
    }
  }
}
try {
  // Exercise the real Vite middleware, not a fixture that implements the rewrite.
  for (const route of ['connect', 'connect/device', 'connect/shares', 'hosted', 'hosted/device', 'hosted/shares']) for (const slash of ['', '/']) {
    scenario = {}; writes = []; calls = []
    const page = await browser.newPage(), resources = [], errors = []
    page.on('request', r => resources.push({ url: r.url(), type: r.resourceType() }))
    page.on('pageerror', e => errors.push(String(e)))
    await page.context().addCookies([{ name: 'fixture', value: '1', url: uiOrigin }])
    const query = '?code=SYNTHETIC&extra=kept%2Bvalue'
    await page.goto(`${uiOrigin}/${route}${slash}${query}`)
    await page.waitForFunction(() => document.querySelector('.hosted-panel')?.getAttribute('aria-busy') === 'false')
    assert.equal(new URL(page.url()).search, query)
    if (route.endsWith('shares')) await page.getByRole('heading', { name: 'Manage shared links' }).waitFor()
    else assert.equal(await page.getByRole('tab', { name: route.endsWith('device') ? 'Your agent' : 'This computer', exact: true }).getAttribute('aria-selected'), 'true')
    assert.equal(writes.length, 0)
    assertResources(resources, true); assert.deepEqual(errors, [])
    routeResults.push({ route: `/${route}${slash}`, resources }); await page.close()
  }
  for (const route of ['/', '/connect-unrelated?code=untouched', '/hosted/device/extra', '/pitch/', '/.env']) {
    const page = await browser.newPage(), resources = [], errors = []
    page.on('request', r => resources.push({ url: r.url(), type: r.resourceType() }))
    page.on('pageerror', e => errors.push(String(e)))
    await page.route('https://api.github.com/**', r => r.fulfill({ json: { stargazers_count: 2 } }))
    const response = await page.goto(`http://127.0.0.1:${server.address().port}` + route)
    if (route !== '/') {
      assert.equal(response.status(), 404, 'unknown/private paths have no marketing or Connect fallback')
      assert.equal(await page.locator('.hosted-page, .pond-demo').count(), 0)
      routeResults.push({ route, status: response.status() }); await page.close(); continue
    }
    await page.locator('.pond-demo').waitFor()
    assert.equal(await page.locator('.hosted-page').count(), 0)
    assertResources(resources, false); assert.deepEqual(errors, [])
    assert(!resources.some(r => new URL(r.url).pathname.startsWith('/v1/')), 'landing must not fetch Connect API')
    if (route === '/') await page.screenshot({ path: `${evidence}/homepage.png`, fullPage: true })
    routeResults.push({ route, resources }); await page.close()
  }
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
    { name: 'canonical-no-slash', noSlash: true }, { name: 'canonical-device-slash', device: true, deviceSlash: true },
  ]) {
    scenario = test; writes = []; calls = []; approved = false; signedOut = false; interest = false
    scenario.expiresAt = Date.now() + 2500
    const page = await browser.newPage({ viewport: { width: test.width || 1100, height: test.height || 900 } })
    const errors = [], resources = []; page.on('pageerror', e => errors.push(String(e)))
    page.on('request', r => resources.push({ url: r.url(), type: r.resourceType() }))
    await page.context().addCookies([{ name: 'fixture', value: '1', url: uiOrigin }])
    await page.goto(`${uiOrigin}/${test.legacy ? 'hosted' : 'connect'}${test.device ? `/device${test.deviceSlash ? '/' : ''}${test.noCode ? '' : '?code=SYNTHETIC'}` : `${test.noSlash ? '' : '/'}?returnTo=https://evil.example`}`)
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
        return { width: sheet.getBoundingClientRect().width, radius: getComputedStyle(sheet).borderRadius, title: getComputedStyle(title).fontSize, textColor: getComputedStyle(title).color, ctaRadius: getComputedStyle(cta).borderRadius, ctaColor: getComputedStyle(cta).backgroundColor, font: getComputedStyle(sheet).fontFamily }
      })
      assert.equal(geometry.width, test.width >= 760 ? 720 : (test.width || 1100) > 550 ? Math.min(720, (test.width || 1100) - 40) : test.width - 24)
      assert.equal(geometry.radius, '0px'); assert.equal(geometry.ctaRadius, '0px'); assert.equal(geometry.ctaColor, 'rgb(25, 22, 29)')
      assert.equal(geometry.title, test.width <= 550 ? '32px' : '36px'); assert.match(geometry.font, /Host Grotesk/)
      assert.equal(geometry.textColor, 'rgb(2, 1, 2)', 'preserve the current marketing cascade text colour')
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
      assert.match(await page.locator('.hosted-details').innerText(), /500,000,000[\s\S]*90,000,000[\s\S]*20 accounts[\s\S]*renew each approved device connection automatically/)
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
    const manage = page.getByRole('link', { name: 'Manage shared links', exact: true })
    const hasManagement = !test.anonymous && !test.waitlist && !test.unavailable && !test.invalid
    assert.equal(await manage.count(), hasManagement ? 1 : 0)
    if (hasManagement) {
      assert.equal(await manage.getAttribute('href'), '/connect/shares')
      assert.equal(await page.locator('.hosted-identity .hosted-manage-link').count(), 1)
    }
    // Only the real account sharing link is allowed; no prototype administration or invented state.
    const productText = await page.locator('body').evaluate(body => {
      const link = body.querySelector('.hosted-identity a.hosted-manage-link[href="/connect/shares"]')
      return link?.innerText === 'Manage shared links' ? body.innerText.replace(link.innerText, '') : body.innerText
    })
    assert.doesNotMatch(productText, /Uploads are on|Read access is on|Manage|simulated|Try a state|\bHosting\b|\bHosted\b/)
    assert.equal(await page.evaluate(() => localStorage.length), 0)
    assert(!(await page.content()).includes('must-not-render'))
    assert(!(await page.content()).includes('synthetic-csrf'))
    assert.deepEqual(errors, [])
    assertResources(resources, true)
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
  for (const test of [{name:'shares',width:390},{name:'shares-320',width:320},{name:'shares-empty',empty:true},{name:'shares-denied',forbidden:true},{name:'shares-unconfirmed',unconfirmed:true},{name:'shares-page-two',paginated:true,width:390},{name:'shares-page-two-unconfirmed',paginated:true,unconfirmed:true},{name:'shares-page-two-readback-unavailable',paginated:true,readbackUnavailable:true},{name:'legacy-shares',legacy:true},{name:'legacy-shares-slash',legacy:true,slash:true},{name:'canonical-shares-slash',slash:true}]) {
    scenario=test; writes=[];calls=[];shareReads=[];signedOut=false;revoked=false
    const page=await browser.newPage({viewport:{width:test.width||1100,height:900}})
    await page.context().addCookies([{name:'fixture',value:'1',url:uiOrigin}])
    await page.goto(`${uiOrigin}/${test.legacy?'hosted':'connect'}/shares${test.slash?'/':''}`)
    await page.getByRole('heading',{name:'Manage shared links'}).waitFor({timeout:3000})
    await page.waitForFunction(()=>document.querySelector('.hosted-panel')?.getAttribute('aria-busy')==='false')
    assert.match(await page.locator('main').innerText(),/Anyone with this link/)
    assert.equal(await page.getByRole('link',{name:'Account',exact:true}).getAttribute('href'),'/connect/')
    assert.equal(writes.length,0)
    if(test.paginated) {assert.equal(await page.locator('.hosted-shares > li').count(),101);assert.ok(shareReads.includes('/v1/shares?before='+encodeURIComponent(sharesCursor)))}
    if(test.empty) assert.match(await page.locator('main').innerText(),/No shared links/)
    else {
      assert.equal(await page.getByRole('link',{name:'Open shared recording'}).getAttribute('href'),'https://share.example/s/synthetic')
      await page.getByRole('button',{name:'Stop sharing'}).evaluate(e=>{e.click();e.click()})
      await page.waitForFunction(()=>document.querySelector('.hosted-panel')?.getAttribute('aria-busy')==='false')
      assert.equal(writes.length,1);assert.equal(writes[0].method,'DELETE');assert.equal(writes[0].path,'/v1/shares/share-1');assert.equal(writes[0].csrf,'synthetic-csrf')
      assert.equal(writes[0].origin,uiOrigin)
      assert.match(writes[0].cookie || '',/fixture=1/)
      if(test.forbidden||test.unconfirmed||test.readbackUnavailable) assert.equal(await page.getByRole('alert').count(),1)
      else {assert.equal(await page.getByRole('link',{name:'Open shared recording'}).count(),0);assert.match(await page.locator('main').innerText(),/revoked/)}
      if(test.paginated) assert.equal(shareReads.filter(path=>path==='/v1/shares?before='+encodeURIComponent(sharesCursor)).length,2,'readback must traverse page two again, not infer revocation from absence on page one')
    }
    assert.equal(await page.locator('script').filter({hasText:'Captured title'}).count(),0)
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false)
    assert.equal(await page.evaluate(() => [...document.querySelectorAll('main *, .pond-nav *')].filter(e => e.getClientRects().length && getComputedStyle(e).position !== 'absolute').some(e => { const r = e.getBoundingClientRect(); return r.width > innerWidth + 1 || r.left < -1 || r.right > innerWidth + 1 })), false, 'management content stays within the viewport')
    assert.equal(await page.locator('[aria-label="Shared links"]').evaluate(e => getComputedStyle(e).textAlign), 'left')
    assert.ok(!(await page.content()).includes('synthetic-csrf'))
    assert(calls.length > 0)
    for (const call of calls) {
      assert.equal(call.host,apiOrigin ? new URL(apiOrigin).host : `127.0.0.1:${server.address().port}`,'share management must use configured API origin')
      assert.match(call.cookie || '',/fixture=1/,'share management retains credentialed cookies')
    }
    if(test.paginated && !test.unconfirmed && !test.readbackUnavailable) await page.locator('.hosted-shares > li').last().scrollIntoViewIfNeeded()
    await page.screenshot({path:`${evidence}/${test.name}.png`,fullPage:!test.paginated});await page.close();results.push(test.name)
  }
  for(const status of ['ready','pending','paused','failed','expired','revoked','deleted']) {
    recipientStatus=status
    const context=await browser.newContext({javaScriptEnabled:false,viewport:{width:390,height:900}})
    const page=await context.newPage()
    await page.goto(`http://127.0.0.1:${server.address().port}/s/synthetic`)
    const text=await page.locator('main').innerText()
    if(status==='ready') {assert.match(text,/Synthetic summary <img/);assert.match(text,/\[00:02\] Move this button/);assert.equal(await page.locator('video').count(),1);assert.equal(await page.getByRole('link',{name:'Machine-readable manifest'}).count(),1)}
    else {assert.ok(!text.includes('Synthetic summary'));assert.equal(await page.locator('video').count(),0)}
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false)
    await page.screenshot({path:`${evidence}/recipient-${status}.png`,fullPage:true});await context.close();results.push(`recipient-${status}-no-js`)
  }
  recipientStatus='pending'
  const page=await browser.newPage()
  const errors=[];page.on('pageerror',e=>errors.push(String(e)))
  await page.goto(`http://127.0.0.1:${server.address().port}/s/synthetic`)
  await page.getByRole('heading',{name:'Uploading',exact:true}).first().waitFor()
  recipientUploadedBytes=8
  await page.getByText('8 of 10 bytes uploaded.',{exact:true}).waitFor({timeout:10000})
  recipientUploadedBytes=11
  await page.waitForResponse(response=>response.url().endsWith('/manifest.json'))
  await page.waitForTimeout(100)
  assert.equal(await page.getByText('8 of 10 bytes uploaded.',{exact:true}).count(),1,'invalid counters must not replace confirmed progress')
  recipientStatus='ready'
  await page.getByRole('heading',{name:'Summary',exact:true}).waitFor({timeout:10000})
  assert.equal(await page.evaluate(()=>window.injected),undefined);assert.deepEqual(errors,[])
  await page.close();results.push('recipient-progress-validation-pending-ready-csp')
  const receipt = { synthetic: true, artifact: root, apiOrigin: apiOrigin || 'same-origin', passed: results, count: results.length, screenshots: evidence, assets: Object.fromEntries(await Promise.all(['hosted/source-pond-v4.png', 'hosted/agent-pond-v4.png', 'hosted/agent-reading-v4.png', 'HostGrotesk-Variable.ttf', 'apparent-wordmark.svg'].map(async name => [name, createHash('sha256').update(await readFile(`${root}/${name}`)).digest('hex')]))), limits: 'Controlled synthetic API only; not live OAuth, service, native, or deployment proof. 640×450 CSS pixels is a 200%-equivalent layout, not browser zoom.' }
  Object.assign(receipt, { mode, routes: routeResults, closures: { landing: [...landingGraph.keys()], connect: [...connectGraph.keys()] } })
  await import('node:fs/promises').then(fs => fs.writeFile(`${evidence}/receipt.json`, JSON.stringify(receipt, null, 2)))
  console.log(JSON.stringify(receipt, null, 2))
} finally { await browser.close(); if (viteServer) await viteServer.close(); if (apiServer) await new Promise(r => apiServer.close(r)); await new Promise(r => server.close(r)) }
