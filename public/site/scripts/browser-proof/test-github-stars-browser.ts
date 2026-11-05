// Build/package first, then serve dist-apparent and set SITE_URL to its URL.
// CHROME_PATH selects a locally installed Chromium; no browser download needed.
import assert from "node:assert/strict"
import { test } from "node:test"
import { chromium } from "playwright-core"

const url = process.env.SITE_URL

test("star badge handles counts, failure, keyboard focus and narrow headers", { skip: !url }, async () => {
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, headless: true })
  try {
    const page = await browser.newPage()
    let reply = { stargazers_count: 2 }
    let status = 200
    let requests = 0
    await page.route("https://api.github.com/repos/tryapparent/apparent", (route) => {
      requests++
      return route.fulfill({ status, json: reply })
    })
    for (const [value, expected] of [[2, "2"], [0, "0"], [93000, "93K"], [1250, "1.3K"]]) {
      reply = { stargazers_count: value }
      await page.goto(url)
      await page.waitForFunction((text) => document.querySelector(".github-stars span")?.textContent === text, expected)
      assert.equal(await page.locator(".github-stars").getAttribute("href"), "https://github.com/tryapparent/apparent")
      assert.equal(await page.locator(".github-stars").getAttribute("aria-label"), `Apparent on GitHub: ${value.toLocaleString("en")} stars`)
    }
    for (const width of [1280, 375, 320]) {
      await page.setViewportSize({ width, height: 800 })
      const bounds = await page.locator(".github-stars").boundingBox()
      const brand = await page.locator(".pond-nav .brand").boundingBox()
      assert.ok(bounds.x >= brand.x + brand.width && bounds.x + bounds.width <= width)
      assert.ok(bounds.height >= 44)
      await page.locator(".brand").first().focus()
      await page.keyboard.press("Tab")
      assert.equal(await page.locator('.pond-nav a[href="#team"]').evaluate((el) => document.activeElement === el), true)
      await page.keyboard.press("Tab")
      assert.equal(await page.locator(".github-stars").evaluate((el) => document.activeElement === el), true)
      assert.equal(await page.locator(".github-stars").evaluate((el) => getComputedStyle(el).outlineStyle), "solid")
    }
    for (const invalid of [-1, "93k", 1.2, null]) {
      reply = { stargazers_count: invalid }
      await page.goto(url)
      await page.waitForLoadState("networkidle")
      assert.equal(await page.locator(".github-stars span").textContent(), "GitHub")
    }
    status = 403
    await page.goto(url)
    await page.waitForLoadState("networkidle")
    assert.equal(await page.locator(".github-stars span").textContent(), "GitHub")
    assert.equal(requests, 9, "one metadata request per mount, without retries")
  } finally {
    await browser.close()
  }
})
