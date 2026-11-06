import assert from "node:assert/strict"
import { existsSync, readFileSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { test } from "node:test"
import { fileURLToPath } from "node:url"
import { chromium } from "playwright-core"
import { DEMO_CURSOR } from "../../src/demo/content/cursor-track.ts"
import { cameraOffsetAt } from "../../src/demo/core/cursor-camera.ts"
import { parseStatus } from "../../src/features.ts"
import { browserAnalyticsDefinitions } from "../../../../shared/analytics/config.ts"

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
  assert.equal(html.match(/aria-label="apparent demo"/g)?.length, 1)
  assert.match(html, /Play with sound/)
  assert.equal(/id="demo-film"|apparent-demo-pi\.mp4|id="how"|href="#how"/.test(html), false, "duplicate demos must not appear in the public page")
  assert.equal(existsSync(join(publicDir, "apparent-demo-pi.mp4")), false)
  assert.equal(existsSync(join(publicDir, "apparent-demo-pi-poster.jpg")), false)
})

test("product names use the boxed wordmark including the demo heading", () => {
  const html = readFileSync(join(publicDir, "index.html"), "utf8")
  const body = html.split("<body>")[1]
  assert.match(body, /class="pond-wordmark"[^>]*src="\/apparent-wordmark\.svg"[^>]*alt="apparent"[^>]*\/> creates the instructions/)
  assert.equal(existsSync(join(publicDir, "apparent-wordmark.svg")), true)
  assert.match(body, /<h2>See what <span class="demo-heading-product"><img[^>]*src="\/apparent-wordmark\.svg"[^>]*alt="apparent"[^>]*\/> captures<\/span><\/h2>/)
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
      const bytes = readFileSync(file)
      if (extname(file) === ".mp4") {
        response.setHeader("Accept-Ranges", "bytes")
        const range = request.headers.range?.match(/^bytes=(\d+)-(\d*)$/)
        if (range) {
          const start = Number(range[1])
          const end = range[2] ? Math.min(Number(range[2]), bytes.length - 1) : bytes.length - 1
          if (start > end) { response.writeHead(416, { "Content-Range": `bytes */${bytes.length}` }).end(); return }
          response.writeHead(206, { "Content-Range": `bytes ${start}-${end}/${bytes.length}`, "Content-Length": end - start + 1 })
          response.end(bytes.subarray(start, end + 1))
          return
        }
      }
      response.setHeader("Content-Length", bytes.length)
      response.end(bytes)
    } catch { response.writeHead(404).end() }
  })
  await new Promise<void>(done => server.listen(0, "127.0.0.1", done))
  const origin = `http://127.0.0.1:${(server.address() as import("node:net").AddressInfo).port}`
  const analyticsEnabled = browserAnalyticsDefinitions(process.env).__APPARENT_ANALYTICS_ENABLED__ === "true"
  const receipts = []
  try {
    for (const engine of [chromium, firefox]) {
      const browser = await engine.launch({ headless: true, ...(engine === chromium && process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) })
      try {
        for (const width of [1280, 768, 390, 320]) {
          const page = await browser.newPage({ viewport: { width, height: 900 }, reducedMotion: "reduce" })
          // Reproduce late consent arrival over the mobile frog: the original
          // flow could lose pointer-up after pointer-down had reached the frog.
          if (analyticsEnabled && width === 320) await page.addInitScript(() => {
            const append = Element.prototype.append
            let held: (Node | string)[] | undefined
            const release = () => { if (held) { append.apply(document.body, held); held = undefined } }
            Element.prototype.append = function (...nodes) {
              if (nodes.some(node => node instanceof Element && node.matches('aside[aria-label="Analytics preferences"]'))) {
                held = nodes
                Element.prototype.append = append
                window.setTimeout(release, 2000)
                return
              }
              append.apply(this, nodes)
            }
            document.addEventListener("pointerdown", event => {
              if ((event.target as Element)?.closest(".pond-cover-frog")) release()
            }, true)
          })
          const errors: string[] = []
          page.on("pageerror", error => errors.push(error.message))
          page.on("console", message => { if (message.type() === "error" && /React|hydrat/i.test(message.text())) errors.push(message.text()) })
          await page.route("https://api.github.com/**", route => route.fulfill({ contentType: "application/json", body: "[]" }))
          // Page routes do not intercept a popup's initial navigation.
          await page.context().route("https://www.linkedin.com/**", route => route.fulfill({ contentType: "text/html", body: "<title>Synthetic LinkedIn destination</title>" }))
          const analyticsRequests: string[] = []
          await page.route("https://eu.i.posthog.com/**", route => {
            analyticsRequests.push(route.request().url())
            return route.abort()
          })
          await page.goto(origin)
          await page.evaluate(() => document.fonts.ready)
          await page.waitForFunction(() => customElements.get("apparent-footer"))
          // The notice loads independently of React. Resolve the visitor's
          // choice before it can intercept a homepage interaction mid-click.
          const refuseAnalytics = page.getByRole("button", { name: "No thanks", exact: true })
          const suppressed = await page.evaluate(() => navigator.doNotTrack === "1" ||
            (navigator as Navigator & { globalPrivacyControl?: boolean }).globalPrivacyControl === true)
          if (analyticsEnabled && !suppressed) await refuseAnalytics.waitFor()
          if (await refuseAnalytics.isVisible()) {
            await refuseAnalytics.click()
            await page.getByRole("button", { name: "Analytics settings", exact: true }).waitFor()
          }
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
          // Two activations retain the first greeting and open the QR Easter egg.
          const lounge = page.locator(".pond-lounging-frog")
          await page.waitForTimeout(450)
          await lounge.dblclick({ delay: 80 })
          const splash = page.locator(".frog-splash-dialog")
          await page.waitForFunction(() => document.querySelector(".frog-splash-dialog")?.getAttribute("data-stage") === "reveal")
          assert.equal(await splash.locator(".splash-destination").getAttribute("href"), "https://app.tryapparent.com/")
          await page.waitForFunction(() => {
            const image = document.querySelector<HTMLImageElement>(".splash-destination img")
            return image?.complete && image.naturalWidth > 0
          })
          assert.equal(await splash.locator(".splash-flood").evaluate(element => getComputedStyle(element).opacity), "0.9")
          assert.equal(await splash.locator(".splash-destination img").evaluate(element => getComputedStyle(element).opacity), "1")
          await page.keyboard.press("Escape")
          assert.equal(await splash.count(), 0)
          assert.equal(await lounge.evaluate(element => element === document.activeElement), true)
          assert.equal(await page.evaluate(() => document.documentElement.style.overflow), "")
          if (width === 1280) {
            await lounge.press("Enter")
            await lounge.press("Enter")
            await splash.locator(".splash-close").click()
            assert.equal(await splash.count(), 0)
          }
          if (width === 390) {
            // React unmount cancels an in-flight jump; reopening starts a fresh sequence.
            await page.emulateMedia({ reducedMotion: "no-preference" })
            await lounge.dblclick({ delay: 80 })
            await page.waitForFunction(() => document.querySelector(".frog-splash-dialog")?.getAttribute("data-stage") === "jump")
            await page.keyboard.press("Escape")
            assert.equal(await splash.count(), 0)
            assert.equal(await lounge.evaluate(element => (element as HTMLElement).style.visibility), "")
            assert.equal(await lounge.evaluate(element => element === document.activeElement), true)
            await lounge.dblclick({ delay: 80 })
            await page.evaluate(() => window.dispatchEvent(new Event("pagehide")))
            await page.waitForFunction(() => !document.querySelector(".frog-splash-dialog"))
            assert.equal(await page.evaluate(() => document.documentElement.style.overflow), "")
            await page.waitForTimeout(1300)
            assert.equal(await splash.count(), 0)

            await page.emulateMedia({ reducedMotion: "no-preference" })
            await lounge.dblclick({ delay: 80 })
            await page.waitForFunction(() => document.querySelector(".frog-splash-dialog")?.getAttribute("data-stage") === "ripple")
            assert.equal(await page.locator(".pond-page").evaluate(element => (element as HTMLElement).style.filter), "url(\"#pond-water-refraction\")")
            await page.keyboard.press("Escape")
            assert.equal(await page.locator(".pond-page").evaluate(element => (element as HTMLElement).style.filter), "")
            assert.equal(await page.evaluate(() => document.documentElement.style.overflow), "")
            await page.emulateMedia({ reducedMotion: "reduce" })
            await page.setViewportSize({ width: 568, height: 320 })
            // Flush resize layout before cancelling its scroll. A pending smooth
            // scroll can otherwise move the frog between pointer down and up.
            await page.evaluate(() => new Promise<void>(done => requestAnimationFrame(() => requestAnimationFrame(() => done()))))
            await lounge.evaluate(element => element.scrollIntoView({ behavior: "instant", block: "center" }))
            await lounge.scrollIntoViewIfNeeded()
            await lounge.dblclick({ delay: 80 })
            await page.waitForFunction(() => document.querySelector(".frog-splash-dialog")?.getAttribute("data-stage") === "reveal")
            const fits = await splash.locator(".splash-message").evaluate(element => [...element.querySelectorAll("a,button,img")].every(child => {
              const r = child.getBoundingClientRect()
              return r.top >= 0 && r.bottom <= innerHeight && r.left >= 0 && r.right <= innerWidth
            }))
            assert.equal(fits, true, "QR and dismissal fit a short landscape viewport")
            await splash.locator(".splash-close").click()
            await page.setViewportSize({ width, height: 900 })
          }

          for (const [index, card] of (await page.locator(".feature-grid-card").all()).entries()) {
            await card.scrollIntoViewIfNeeded()
            await card.locator("iframe").waitFor({ state: "attached" })
            await page.waitForFunction(index => document.querySelectorAll<HTMLIFrameElement>(".feature-grid-card iframe")[index]?.contentDocument?.querySelector("#apparent-feature-theme"), index)
            const geometry = await card.evaluate(element => {
              const copy = element.querySelector(".feature-copy")!.getBoundingClientRect()
              const frame = element.querySelector(".feature-grid-viewport")!.getBoundingClientRect()
              return { copy: { x: copy.x, y: copy.y, right: copy.right, bottom: copy.bottom }, frame: { x: frame.x, y: frame.y, right: frame.right, bottom: frame.bottom } }
            })
            if (width > 700 && index % 2 === 1) assert(geometry.frame.right <= geometry.copy.x, "alternating feature animation precedes its copy")
            else if (width > 700) assert(geometry.copy.right <= geometry.frame.x, "feature copy sits beside its animation")
            else assert(geometry.copy.bottom <= geometry.frame.y, "feature copy precedes its animation on mobile")
          }
          for (const colorScheme of ["dark", "light"] as const) {
            await page.emulateMedia({ colorScheme })
            const target = await page.locator(".feature-grid-card").nth(1).evaluate(element => {
              const host = element.querySelector(".feature-grid-viewport")!
              const probe = document.createElement("span")
              probe.style.backgroundColor = getComputedStyle(host).getPropertyValue("--color-muted")
              document.body.append(probe)
              const color = getComputedStyle(probe).backgroundColor
              probe.remove()
              return color
            })
            await page.waitForFunction(({ expected, phone }) => {
              const host = document.querySelectorAll(".feature-grid-viewport")[1]
              const doc = host.querySelector("iframe")!.contentDocument!
              return getComputedStyle(host).backgroundColor === expected && (phone || doc.defaultView!.getComputedStyle(doc.querySelector(".stage")!).backgroundColor === expected)
            }, { expected: target, phone: width <= 700 })
            const frameTheme = await page.locator(".feature-grid-card").nth(1).evaluate(element => {
              const host = element.querySelector(".feature-grid-viewport")!
              const doc = element.querySelector("iframe")!.contentDocument!
              return { host: getComputedStyle(host).backgroundColor, frame: doc.defaultView!.getComputedStyle(doc.querySelector(".stage")!).backgroundColor }
            })
            if (width > 700) assert.equal(frameTheme.frame, frameTheme.host, "visible desktop illustration follows the parent palette while open")
            if (colorScheme === "dark") {
              const contrastState = await page.locator(".feature-grid-card").nth(3).evaluate(element => {
                const doc = element.querySelector("iframe")!.contentDocument!
                return { panel: doc.defaultView!.getComputedStyle(doc.querySelector(".window")!).backgroundColor, text: doc.defaultView!.getComputedStyle(doc.querySelector(".response-note")!).color }
              })
              if (width > 700) assert.notEqual(contrastState.panel, contrastState.text, "visible walkthrough text never shares the pale panel color")
              const highlight = await page.locator(".enjoyable > span").evaluate(element => ({ color: getComputedStyle(element).color, background: getComputedStyle(element).backgroundColor }))
              assert.deepEqual(highlight, { color: "rgb(2, 1, 2)", background: "rgb(183, 165, 216)" }, "the complete highlighted word has a contrasting foreground and background")
            }
          }
          if (width <= 700) {
            assert.equal(await page.locator(".feature-mobile:visible").count(), 4)
            const smallest = await page.locator(".feature-mobile").evaluateAll(elements => Math.min(...elements.flatMap(element => [...element.querySelectorAll("p,time,li,figcaption")].map(child => parseFloat(getComputedStyle(child).fontSize)))))
            assert(smallest >= 14, "phone examples retain readable text without scaling")
          } else {
            const toggle = page.getByRole("button", { name: "Play animations", exact: true })
            assert.equal(await toggle.isVisible(), true)
            await toggle.click()
            await page.getByRole("button", { name: "Pause animations", exact: true }).click()
          }
          const track = page.getByRole("slider", { name: "Demo timeline" })
          await track.focus()
          await track.press("ArrowRight")
          assert.equal(await track.getAttribute("aria-valuenow"), "5")
          await track.press("Home")
          assert.equal(await track.getAttribute("aria-valuenow"), "0")
          assert.equal(await page.locator(".track .seg").first().evaluate(element => getComputedStyle(element).borderTopWidth), "1px")
          assert.equal(await page.getByRole("button", { name: /close-up|Full recording/ }).count(), 0)
          if (width <= 700) {
            const stage = page.locator(".video-demo .stage-wrap")
            const fullHeight = (await stage.boundingBox())!.height
            await page.emulateMedia({ reducedMotion: "no-preference" })
            await page.waitForFunction(() => document.querySelector(".video-demo")?.getAttribute("data-camera") === "following")
            await page.waitForFunction(height => document.querySelector(".video-demo .stage-wrap")!.getBoundingClientRect().height > height * 1.8, fullHeight)
            const left = DEMO_CURSOR.reduce((a, b) => a.x < b.x ? a : b)
            const right = DEMO_CURSOR.reduce((a, b) => a.x > b.x ? a : b)
            await page.waitForFunction(() => {
              const element = document.querySelector(".video-demo video") as HTMLVideoElement
              return element.readyState >= 1 && !element.seeking && element.currentTime < .1
            })
            for (const point of [left, right, left]) {
              await stage.scrollIntoViewIfNeeded()
              await page.locator(".video-demo video").evaluate((element: HTMLVideoElement, time) => { element.currentTime = time }, point.t)
              await page.waitForFunction(time => {
                const element = document.querySelector(".video-demo video") as HTMLVideoElement
                return !element.seeking && Math.abs(element.currentTime - time) < .1
              }, point.t)
              await page.waitForFunction(expected => Math.abs(Number((document.querySelector(".video-demo video") as HTMLElement).style.getPropertyValue("--cursor-offset")) - expected) < .001, cameraOffsetAt(point.t, DEMO_CURSOR))
            }
            await page.emulateMedia({ reducedMotion: "reduce" })
            await page.waitForFunction(() => document.querySelector(".video-demo")?.getAttribute("data-camera") === "full")
            await page.waitForFunction(height => Math.abs(document.querySelector(".video-demo .stage-wrap")!.getBoundingClientRect().height - height) < .1, fullHeight)
            for (const button of await page.locator(".controls-left button, .beats button").all()) {
              const bounds = (await button.boundingBox())!
              assert(bounds.height >= 44 && bounds.x >= 0 && bounds.x + bounds.width <= width, "mobile playback and chapter controls fit with full touch targets")
            }
          }
          await page.locator(".photo-story").scrollIntoViewIfNeeded()
          await page.waitForFunction(() => [...document.querySelectorAll<HTMLImageElement>(".cutout-person img")].every(image => image.complete && image.naturalWidth > 0))
          await page.locator(".footer-hi").click()
          const dialog = page.locator("dialog[data-dialog=social]")
          assert.equal(await dialog.evaluate((element: HTMLDialogElement) => element.open), true)
          const destination = page.waitForEvent("popup")
          await dialog.locator('a[href*="simon"]').click()
          const popup = await destination
          await popup.waitForLoadState()
          assert.match(popup.url(), /linkedin\.com\/in\/simon-van-laak/)
          assert.equal(await popup.title(), "Synthetic LinkedIn destination", "the first profile activation reaches the isolated fixture")
          await popup.close()
          await page.bringToFront()
          assert.equal(page.url(), origin + "/", "opening the profile preserves the homepage")
          assert.equal(await dialog.evaluate((element: HTMLDialogElement) => element.open), true, "the original social dialog remains open")
          await dialog.locator(".footer-close").click()
          assert.equal(await page.locator(".footer-hi").evaluate(element => element === document.activeElement), true)
          if (width === 320) {
            // A native video API may reject entering fullscreen before playback.
            await page.locator(".video-demo video").evaluate(element => {
              Object.defineProperty(element, "requestFullscreen", { value: undefined })
              Object.defineProperty(element, "webkitEnterFullscreen", { value: () => { throw new DOMException("Unavailable", "InvalidStateError") } })
            })
            await page.getByRole("button", { name: "Fullscreen", exact: true }).click()
            await page.getByRole("alert").getByRole("link", { name: "Open video", exact: true }).waitFor()
          }
          assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, "no horizontal page overflow")
          assert.deepEqual(errors, [], "no page or hydration errors")
          assert.deepEqual(analyticsRequests, [], "homepage interactions must not send analytics without consent")
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
