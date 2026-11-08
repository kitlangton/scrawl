// Run from public/site/: CHROME_PATH=/path/to/chrome node --test scripts/browser-proof/test-og-video-stage-browser.mjs
// Exercises the real stage and concept scripts without writing a movie or contacting the network.
import assert from "node:assert/strict"
import { readFile, readdir } from "node:fs/promises"
import { createServer } from "node:http"
import { before, after, test } from "node:test"
import { chromium } from "playwright-core"

const concepts = ["apparent-fix", "apparent-lasso", "apparent-pile"]
const stagePath = "/scripts/og-video/stage.html"
const root = new URL("../../", import.meta.url)
let server, browser, origin

before(async () => {
  server = createServer(async (req, res) => {
    const pathname = new URL(req.url, "http://localhost").pathname
    const assets = [stagePath, "/scripts/og-video/common.js", "/node_modules/matter-js/build/matter.min.js",
      "/icon.svg", ...concepts.map((name) => `/scripts/og-video/${name}.js`)]
    if (!assets.includes(pathname)) {
      // An inert attacker payload makes accidental loads observable, not just 404s.
      res.writeHead(200, { "content-type": "text/javascript" })
      return res.end("window.unexpectedScriptExecuted = true")
    }
    try {
      const body = pathname === "/icon.svg"
        ? await readFile(new URL("../../public/assets/brand/icon.svg", import.meta.url))
        : await readFile(new URL(`.${pathname}`, root))
      const type = pathname.endsWith(".html") ? "text/html" : pathname.endsWith(".svg") ? "image/svg+xml" : "text/javascript"
      res.writeHead(200, { "content-type": type }).end(body)
    } catch {
      res.writeHead(404).end()
    }
  })
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve))
  origin = `http://127.0.0.1:${server.address().port}`
  browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, headless: true })
})
after(async () => {
  await browser?.close()
  if (server) await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()))
})

async function openStage(query) {
  const context = await browser.newContext()
  // Never allow an adversarial URL in this test to leave the local fixture.
  await context.route("**/*", (route) => new URL(route.request().url()).origin === origin
    ? route.continue()
    : route.fulfill({ contentType: "text/javascript", body: "window.unexpectedScriptExecuted = true" }))
  const page = await context.newPage()
  const scripts = [], errors = []
  page.on("request", (request) => { if (request.resourceType() === "script") scripts.push(request.url()) })
  page.on("pageerror", (error) => errors.push(error.message))
  await page.goto(`${origin}${stagePath}${query}`, { waitUntil: "networkidle" })
  return { context, page, scripts, errors }
}

const staticScripts = (base) => [`${base}/node_modules/matter-js/build/matter.min.js`, `${base}/scripts/og-video/common.js`]

// This inventory assertion prevents silently dropping a real concept when adding one later.
test("supported concepts match the actual concept files", async () => {
  const files = await readdir(new URL("scripts/og-video/", root))
  assert.deepEqual(files.filter((name) => name.startsWith("apparent-") && name.endsWith(".js")).sort(), concepts.map((name) => `${name}.js`))
})

for (const name of concepts) {
  test(`${name}: capture caller loads the local script and renders deterministic frames`, async () => {
    const { context, page, scripts, errors } = await openStage(`?c=${name}&capture`)
    try {
      assert.equal(await page.evaluate(() => window.ready), true)
      assert.deepEqual(scripts, [...staticScripts(origin), `${origin}/scripts/og-video/${name}.js`])
      const result = await page.evaluate(() => {
        const canvas = document.getElementById("c")
        renderFrame(60)
        const frame = canvas.toDataURL()
        const painted = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data.some((value) => value !== 0)
        renderFrame(60 + window.frameCount)
        return { frames: window.frameCount, painted, loops: frame === canvas.toDataURL(), hud: document.getElementById("hud").textContent }
      })
      assert.ok(result.frames > 0)
      assert.equal(result.painted, true)
      assert.equal(result.loops, true)
      assert.equal(result.hud, "", "capture mode must not start the preview loop")
      assert.deepEqual(errors, [])
    } finally { await context.close() }
  })
}

test("fixed-frame and slow preview callers retain their rendering behavior", async () => {
  const fixed = await openStage("?c=apparent-fix&f=60")
  try {
    assert.equal(await fixed.page.evaluate(() => window.ready), true)
    assert.equal(await fixed.page.evaluate(() => {
      const canvas = document.getElementById("c")
      const initial = canvas.toDataURL()
      renderFrame(60)
      return initial === canvas.toDataURL()
    }), true)
    assert.deepEqual(fixed.errors, [])
  } finally { await fixed.context.close() }
  const preview = await openStage("?c=apparent-lasso&slow=2")
  try {
    await preview.page.waitForFunction(() => /^apparent-lasso \d+\/\d+$/.test(document.getElementById("hud").textContent))
    const first = await preview.page.locator("#hud").textContent()
    await preview.page.waitForFunction((previous) => document.getElementById("hud").textContent !== previous, first)
    assert.deepEqual(preview.errors, [])
  } finally { await preview.context.close() }
})

const invalid = [
  ["missing", ""], ["empty", "?c="], ["unknown concept", "?c=hex-pour"],
  ["absolute URL", "?c=https%3A%2F%2Fattacker.invalid%2Fpayload"],
  ["protocol-relative URL", "?c=%2F%2Fattacker.invalid%2Fpayload"],
  ["backslash authority", "?c=%5C%5Cattacker.invalid%5Cpayload"],
  ["parent traversal", "?c=..%2Fpayload"], ["absolute local path", "?c=%2Fpayload"],
  ["script suffix", "?c=apparent-pile.js"], ["query suffix", "?c=apparent-pile%3Fother"],
  ["fragment suffix", "?c=apparent-pile%23other"], ["data URL", "?c=data%3Atext%2Fjavascript%2Calert(1)%2F%2F"],
  ["javascript URL", "?c=javascript%3Awindow.unexpectedScriptExecuted%3Dtrue%2F%2F"],
  ["object prototype", "?c=__proto__"], ["object constructor", "?c=constructor"],
  ["inherited method", "?c=toString"], ["case mismatch", "?c=Apparent-pile"],
  ["whitespace", "?c=%20apparent-pile"], ["invalid first duplicate", "?c=..%2Fpayload&c=apparent-pile"],
]
for (const [label, query] of invalid) {
  test(`rejects ${label} before any dynamic script request or execution`, async () => {
    const { context, page, scripts, errors } = await openStage(query)
    try {
      assert.deepEqual(scripts, staticScripts(origin), "only the two fixed dependencies may be requested")
      assert.equal(await page.locator("script[src]").count(), 2, "no dynamic script element may be appended")
      assert.equal(await page.evaluate(() => window.unexpectedScriptExecuted === true), false)
      assert.equal(await page.evaluate(() => window.ready === true), false)
      assert.equal(await page.locator("#hud").textContent(), "Unknown video concept")
      assert.deepEqual(errors, [], "invalid input must fail without an uncaught error")
    } finally { await context.close() }
  })
}
