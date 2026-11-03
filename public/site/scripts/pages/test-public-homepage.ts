import assert from "node:assert/strict"
import { existsSync, readFileSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { test } from "node:test"
import { fileURLToPath } from "node:url"
import { chromium } from "playwright-core"
import { parseStatus } from "../../src/features.ts"

const site = join(dirname(fileURLToPath(import.meta.url)), "../..")
const publicDir = process.env.SITE_ARTIFACT ? resolve(process.env.SITE_ARTIFACT) : join(site, "dist-apparent")

test("experiment status accepts kebab-case flags while rejecting malformed identifiers", () => {
  assert.deepEqual(parseStatus("experiment:linux-playback-ducking"), {
    kind: "experiment",
    flag: "linux-playback-ducking",
  })
  assert.deepEqual(parseStatus("experiment:hostedSync"), {
    kind: "experiment",
    flag: "hostedSync",
  })
  for (const status of ["experiment:-leading", "experiment:trailing-", "experiment:double--hyphen", "experiment:with space", "experiment:1leading"]) {
    assert.throws(() => parseStatus(status), /unknown status/)
  }
})

test("packaged homepage ships the brand favicon and its cache-busting URL", () => {
  const html = readFileSync(join(publicDir, "index.html"), "utf8")
  assert.match(html, /<link rel="icon" href="\/icon\.svg\?v=brand-a-1" type="image\/svg\+xml"/)
  assert.equal(readFileSync(join(publicDir, "icon.svg"), "utf8"),
    readFileSync(join(site, "../../shared/assets/brand/apparent-app-icon.svg"), "utf8"))
})

test("public homepage has one interactive demonstration", () => {
  const html = readFileSync(join(publicDir, "index.html"), "utf8")
  assert.match(html, /id="demo"/)
  assert.equal(html.match(/aria-label="Apparent demo"/g)?.length, 1)
  assert.match(html, /Play with sound/)
  assert.equal(/id="demo-film"|apparent-demo-pi\.mp4|id="how"|href="#how"/.test(html), false, "duplicate demos must not appear in the public page")
  assert.equal(existsSync(join(publicDir, "apparent-demo-pi.mp4")), false)
  assert.equal(existsSync(join(publicDir, "apparent-demo-pi-poster.jpg")), false)
})

test("product names use the wordmark except the lowercase demo heading", () => {
  const html = readFileSync(join(publicDir, "index.html"), "utf8")
  const body = html.split("<body>")[1]
  assert.match(body, /class="pond-wordmark"[^>]*src="\/apparent-wordmark\.svg"[^>]*alt="Apparent"[^>]*\/> creates the instructions/)
  assert.equal(existsSync(join(publicDir, "apparent-wordmark.svg")), true)
  assert.match(body, /<h2>See what <span>apparent<\/span> captures<\/h2>/)
  assert.match(body, /Your agent makes the changes and uses<br class="hero-return-break"\/> apparent to send back a video walkthrough\./)
})

test("the belief headline highlights enjoyable without changing its words", async (t) => {
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, headless: true })
  try {
    const page = await browser.newPage()
    const assertHeadline = async (html) => {
      // Parse inertly: do not execute scripts or load the packaged page's resources.
      const belief = await page.evaluate((source) => {
        const document = new DOMParser().parseFromString(source, "text/html")
        const heading = document.querySelector("h2#belief")
        return {
          count: document.querySelectorAll("#belief").length,
          text: heading?.textContent,
          highlighted: !!heading?.querySelector(".enjoy-highlight"),
        }
      }, html)
      assert.equal(belief.count, 1, "the belief heading must be unique")
      assert.equal(belief.text, "We believe work should be enjoyable and leave us free for late nights with friends.")
      assert.equal(belief.highlighted, true, "the heading must contain the enjoyable highlight")
    }
    await t.test("packaged headline", async () => {
      await assertHeadline(readFileSync(join(publicDir, "index.html"), "utf8"))
    })
    const headline = '<h2 id="belief">We believe work should be <span class="enjoy-highlight">enjoyable</span> and leave us free for late nights with friends.</h2>'
    await t.test("HTML syntax does not alter the text contract", async () => {
      await assertHeadline(headline.replace('class="enjoy-highlight"', "class='enjoy-highlight' data-note='a > b'").replace("friends.", "frien&#100;s.<!-- not headline text -->"))
    })
    for (const [name, html] of [
      ["changed words", headline.replace("friends.", "colleagues.")],
      ["missing heading", headline.replaceAll("h2", "div")],
      ["duplicate heading", headline + headline],
      ["missing highlight", headline.replace("enjoy-highlight", "other")],
      ["highlight only in an attribute", headline.replace('class="enjoy-highlight"', "data-note='class=\"enjoy-highlight\"'")],
      ["embedded script text", headline.replace("enjoyable</span>", "enjoyable<script>unexpected</script></span>")],
      ["malformed nested markup", headline.replace("enjoyable</span>", "enjoyable<scrip<script>t></span>")],
    ]) {
      await t.test(`rejects ${name}`, async () => {
        await assert.rejects(() => assertHeadline(html), { code: "ERR_ASSERTION" })
      })
    }
  } finally {
    await browser.close()
  }
})

test("the first screen has a full-size reversible frog and one wave button", () => {
  const html = readFileSync(join(publicDir, "index.html"), "utf8")
  const body = html.split("<body>")[1]
  assert.match(body, /Tell your coding agent what to change<\/span><span class="hero-feedback-line">by pointing, talking, and drawing\./)
  assert.equal(body.match(/class="pond-cover-frog"/g)?.length, 1)
  assert.equal(body.match(/class="pond-lounging-frog"/g)?.length, 1)
  assert.match(body, /lily-clean\.png\?v=1/, "lily bypasses a cached pre-deploy 404")
  for (const asset of ["lily-clean", "frog-rest-cohesive", "frog-air-cohesive", "frog-lounge-cohesive", "frog-wave-cohesive"]) {
    assert.equal(existsSync(join(publicDir, "pond", `${asset}.png`)), true)
  }
  const source = readFileSync(join(site, "src/components/home/FrogPond.tsx"), "utf8")
  assert.match(source, /atEnd\.current = !returning/)
  assert.match(source, /if \(waving\.current\) return/)
  assert.match(source, /prefers-reduced-motion: reduce/)
  assert.ok(body.indexOf('id="demo"') < body.indexOf('class="preview-statement"'), "statement follows the product demo")
  assert.doesNotMatch(body, /Animation study only|Or send this to your agent/)
})

test("homepage renders the accepted photos and sequential features without overlays", () => {
  const html = readFileSync(join(publicDir, "index.html"), "utf8")
  assert.equal(html.match(/class="feature-grid-card"/g)?.length, 4)
  assert.equal(html.match(/class="feature-copy"/g)?.length, 4)
  assert.match(html, /id="photo-preview"/)
  assert.match(html, /Simon dancing/)
  assert.match(html, /Olivia cuddling/)
  assert.match(html, /We made this\.<br>Show us what you can do\./)
  assert.doesNotMatch(html, /feature-grid-overlay|approved-homepage-|homepage-feedback-|id="team"|class="pond-purpose"/)
  for (const name of ["simon-cutout.png", "olivia-dog-cutout.png"]) {
    assert.equal(existsSync(join(publicDir, "pond", name)), true)
  }
})

test("download actions fall back safely until the visitor's architecture is known", () => {
  const html = readFileSync(join(publicDir, "index.html"), "utf8")
  const bundle = html.match(/<script\b[^>]*\bsrc="\/assets\/([^"]+\.js)"[^>]*><\/script>/)?.[1]
  assert.ok(bundle)
  assert.equal(html.match(/Choose Mac download/g)?.length, 1)
  assert.match(readFileSync(join(publicDir, "assets", bundle), "utf8"), /api\.github\.com\/repos\/tryapparent\/apparent\/releases/)
  assert.doesNotMatch(html, /class="release-note"/)
})

// Exercise the actual packaged source application, including hydration and lazy frames.
// External accounts/downloads are synthetic; no audio is played and nothing is recorded.
test("source homepage preserves accepted responsive layout and interactions", async () => {
  const { createServer } = await import("node:http")
  const { extname } = await import("node:path")
  const { mkdir, writeFile } = await import("node:fs/promises")
  const { firefox } = await import("playwright-core")
  const types: Record<string, string> = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".png": "image/png", ".webp": "image/webp", ".jpg": "image/jpeg", ".ttf": "font/ttf", ".mp4": "video/mp4" }
  const server = createServer((request, response) => {
    const pathname = new URL(request.url!, "http://localhost").pathname
    const file = resolve(publicDir, "." + (pathname.endsWith("/") ? pathname + "index.html" : pathname))
    if (!file.startsWith(publicDir + "/")) { response.writeHead(403).end(); return }
    try {
      response.setHeader("Content-Type", types[extname(file)] || "application/octet-stream")
      response.end(readFileSync(file))
    } catch { response.writeHead(404).end() }
  })
  await new Promise<void>(done => server.listen(0, "127.0.0.1", done))
  const origin = `http://127.0.0.1:${(server.address() as import("node:net").AddressInfo).port}`
  const receipts = []
  try {
    for (const engine of [chromium, firefox]) {
      const browser = await engine.launch({ headless: true, ...(engine === chromium && process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) })
      try {
        for (const width of [1280, 768, 390, 320]) {
          const page = await browser.newPage({ viewport: { width, height: 900 }, reducedMotion: "reduce" })
          const errors: string[] = []
          page.on("pageerror", error => errors.push(error.message))
          page.on("console", message => { if (message.type() === "error" && /React|hydrat/i.test(message.text())) errors.push(message.text()) })
          await page.route("https://api.github.com/**", route => route.fulfill({ contentType: "application/json", body: "[]" }))
          await page.route("https://www.linkedin.com/**", route => route.fulfill({ contentType: "text/html", body: "<title>Synthetic LinkedIn destination</title>" }))
          await page.goto(origin)
          await page.evaluate(() => document.fonts.ready)
          await page.waitForFunction(() => customElements.get("apparent-footer"))
          assert.equal(await page.locator(".feature-grid-card").count(), 4)
          assert.equal(await page.locator(".feature-grid-section").count(), 1)
          assert.equal(await page.locator(".pond-purpose:visible, #team:visible").count(), 0)
          assert.equal(await page.locator("h1").textContent(), "Tell your coding agent what to changeby pointing, talking, and drawing.")
          const frog = page.locator(".pond-cover-frog")
          await frog.click()
          await page.waitForFunction(() => document.querySelector(".pond-cover-frog")?.getAttribute("data-landings") === "1")
          assert.equal(await frog.getAttribute("aria-label"), "Jump back to the first lily")
          await frog.click()
          await page.waitForFunction(() => document.querySelector(".pond-cover-frog")?.getAttribute("data-landings") === "2")
          assert.equal(await frog.getAttribute("aria-label"), "Jump to the other lily")
          await page.locator(".pond-lounging-frog").click()
          assert.equal(await page.locator(".pond-lounging-frog").getAttribute("data-waves"), "1")
          for (const card of await page.locator(".feature-grid-card").all()) {
            await card.scrollIntoViewIfNeeded()
            await card.locator("iframe").waitFor()
            const geometry = await card.evaluate(element => {
              const copy = element.querySelector(".feature-copy")!.getBoundingClientRect()
              const frame = element.querySelector(".feature-grid-viewport")!.getBoundingClientRect()
              return { copy: { x: copy.x, y: copy.y, right: copy.right, bottom: copy.bottom }, frame: { x: frame.x, y: frame.y, right: frame.right, bottom: frame.bottom } }
            })
            if (width > 700) assert(geometry.copy.right <= geometry.frame.x, "feature copy sits beside its animation")
            else assert(geometry.copy.bottom <= geometry.frame.y, "feature copy precedes its animation on mobile")
          }
          await page.locator(".photo-story").scrollIntoViewIfNeeded()
          await page.waitForFunction(() => [...document.querySelectorAll<HTMLImageElement>(".cutout-person img")].every(image => image.complete && image.naturalWidth > 0))
          await page.locator(".footer-hi").click()
          const dialog = page.locator("dialog[data-dialog=social]")
          assert.equal(await dialog.evaluate((element: HTMLDialogElement) => element.open), true)
          await dialog.locator('a[href*="simon"]').click()
          assert.equal(await dialog.locator(".rude-bubble").isVisible(), true)
          await dialog.locator(".footer-close").click()
          assert.equal(await dialog.locator(".rude-bubble").isVisible(), false)
          assert.equal(await page.locator(".footer-hi").evaluate(element => element === document.activeElement), true)
          assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, "no horizontal page overflow")
          assert.deepEqual(errors, [], "no page or hydration errors")
          if (process.env.SITE_EVIDENCE) {
            await mkdir(process.env.SITE_EVIDENCE, { recursive: true })
            await page.screenshot({ path: join(process.env.SITE_EVIDENCE, `source-${engine.name()}-${width}.png`), fullPage: true })
          }
          receipts.push({ engine: engine.name(), width, errors })
          await page.close()
        }
      } finally { await browser.close() }
    }
    if (process.env.SITE_EVIDENCE) await writeFile(join(process.env.SITE_EVIDENCE, "source-homepage.json"), JSON.stringify(receipts, null, 2))
  } finally { await new Promise<void>(done => server.close(() => done())) }
})
