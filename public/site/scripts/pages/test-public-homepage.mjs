import assert from "node:assert/strict"
import { existsSync, readFileSync, readdirSync } from "node:fs"
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
  assert.match(body, /class="pond-wordmark"[^>]*src="\/apparent-wordmark\.svg"[^>]*alt="Apparent"[^>]*\/> lets you show an agent/)
  assert.equal(existsSync(join(publicDir, "apparent-wordmark.svg")), true)
  assert.match(body, /<h2>See what <span>apparent<\/span> captures<\/h2>/)
  assert.doesNotMatch(body.replace('<h2>See what <span>apparent</span> captures</h2>', ""), />\s*(?:Apparent|apparent)\b/)
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
  assert.match(body, /Visual feedback gets lost<br\/>in .*text\./)
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
  assert.ok(body.indexOf('id="demo"') < body.indexOf('class="pond-purpose"'), "purpose headline follows the product demo")
  assert.doesNotMatch(body, /Animation study only|Or send this to your agent/)
})

test("public homepage carries the source site's complete team section", () => {
  const html = readFileSync(join(publicDir, "index.html"), "utf8")
  const body = html.split("<body>")[1]
  assert.match(body, /id="team"/)
  assert.match(body, /href="#team">Team<\/a>/)
  assert.ok(body.indexOf('class="pond-purpose"') < body.indexOf('id="team"'), "team follows the interactive belief")
  assert.ok(body.indexOf('id="team"') < body.indexOf('class="frog-footer"'), "team precedes the current footer")
  for (const name of ["Olivia", "Simon", "START Berlin", "Plug and Play", "IMAGO", "OpenClaw", "HPI Engine", "CODE"]) {
    assert.ok(body.includes(name), `missing team content: ${name}`)
  }
  for (const name of [
    "olivia.png", "simon.png", "start-berlin.png", "plug-and-play.png",
    "imago.png", "openclaw.svg", "hpi-engine.svg", "code-wordmark.svg",
  ]) {
    assert.equal(existsSync(join(publicDir, "team", name)), true, `missing team asset: ${name}`)
  }
})

test("team portraits fit without cropping and every affiliation logo links to its official site", () => {
  const html = readFileSync(join(publicDir, "index.html"), "utf8")
  const css = readFileSync(join(site, "src/styles/pond-hero.css"), "utf8")
  assert.match(css, /\.pond-team-profile > img \{[^}]*object-fit: contain;/)
  for (const [domain, image] of [
    ["start-berlin.com", "start-berlin.png"],
    ["plugandplaytechcenter.com", "plug-and-play.png"],
    ["imago-images.com", "imago.png"],
    ["openclaw.ai", "openclaw.svg"],
    ["engine.hpi.de", "hpi-engine.svg"],
    ["code.berlin", "code-wordmark.svg"],
  ]) {
    const url = `https://${domain}`
    assert.match(html, new RegExp(`<a[^>]*href="${url}"[^>]*><img[^>]*src="/team/${image.replaceAll(".", "\\.")}"`), `missing official link for ${image}`)
  }
})

test("download actions fall back safely until the visitor's architecture is known", () => {
  const html = readFileSync(join(publicDir, "index.html"), "utf8")
  const bundle = readdirSync(join(publicDir, "assets")).find((name) => /^index-.*\.js$/.test(name))
  assert.ok(bundle)
  assert.equal(html.match(/Choose Mac download/g)?.length, 1)
  assert.match(readFileSync(join(publicDir, "assets", bundle), "utf8"), /api\.github\.com\/repos\/tryapparent\/apparent\/releases/)
  assert.doesNotMatch(html, /class="release-note"/)
})
