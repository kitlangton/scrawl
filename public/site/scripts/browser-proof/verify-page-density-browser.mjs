// Run against the packaged page: node scripts/verify-page-density-browser.mjs <URL>
import assert from "node:assert/strict"
import { chromium } from "playwright-core"

const url = process.argv[2]
if (!url) throw new Error("Pass the packaged page URL")
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH ?? "/usr/bin/google-chrome", headless: true, args: ["--mute-audio", "--no-sandbox"] })
try {
  for (const [width, height] of [[1920, 1080], [1440, 900], [1280, 800], [1366, 768], [1024, 600], [390, 844], [320, 568]]) {
    const page = await browser.newPage({ viewport: { width, height } })
    await page.goto(url)
    await page.evaluate(() => document.fonts.ready)
    const hero = await page.evaluate(() => {
      const copy = document.querySelector(".pond-copy p").getBoundingClientRect()
      const button = document.querySelector(".pond-download a").getBoundingClientRect()
      const section = document.querySelector(".pond-hero").getBoundingClientRect()
      return { gap: button.top - copy.bottom, height: section.height, buttonBottom: button.bottom - section.top }
    })
    assert.ok(hero.gap >= 32, `CTA needs breathing room: ${JSON.stringify(hero)}`)
    assert.ok(hero.height <= (width <= 760 ? 580 : 560), "shorter hero")
    assert.ok(hero.buttonBottom < hero.height, "CTA is not clipped")
    await page.evaluate(() => document.querySelector(".pond-demo").scrollIntoView())
    const demo = await page.evaluate(() => {
      const stage = document.querySelector(".video-demo .stage-wrap").getBoundingClientRect()
      const controls = document.querySelector(".video-demo .controls").getBoundingClientRect()
      const heading = document.querySelector(".pond-demo h2").getBoundingClientRect()
      return { top: heading.top, bottom: controls.bottom, stageWidth: stage.width, stageHeight: stage.height, overflow: document.documentElement.scrollWidth - innerWidth }
    })
    assert.ok(demo.top >= -1 && demo.bottom <= height - 16, `heading, entire movie and controls fit together at ${width}x${height}: ${JSON.stringify(demo)}`)
    assert.ok(demo.stageWidth <= 1040, "movie has a readable but compact desktop width")
    assert.ok(Math.abs(demo.stageWidth / demo.stageHeight - 1916 / 1164) < .01, "movie is not cropped or distorted")
    assert.ok(demo.overflow <= 0, "no horizontal scrolling")
    for (const label of ["Play", "Replay", "Enable sound", "Chapter: Try it"]) {
      const control = page.getByRole("button", { name: label, exact: true })
      const box = await control.boundingBox()
      assert.ok(box && box.y >= 0 && box.y + box.height <= height, `${label} visible`)
      assert.equal(await control.evaluate(e => {
        const r = e.getBoundingClientRect()
        return e.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2))
      }), true, `${label} reachable`)
    }
    console.log(JSON.stringify({ viewport: `${width}x${height}`, hero, demo, status: "PASS" }))
    await page.close()
  }
} finally {
  await browser.close()
}
