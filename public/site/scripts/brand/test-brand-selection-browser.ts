// Build first, serve dist (or dist-apparent), and set SITE_URL.
// The same check can run against the public site after an authorized deployment.
import assert from "node:assert/strict"
import { mkdirSync } from "node:fs"
import { join } from "node:path"
import { test } from "node:test"
import { chromium } from "playwright-core"

const url = process.env.SITE_URL
const evidence = process.env.SITE_EVIDENCE
const lilac = "rgb(183, 165, 216)" // Canonical wordmark background.
const carbon = "rgb(2, 1, 2)"

function contrast(a, b) {
  const luminance = (color) => color.match(/\d+/g).slice(0, 3).map(Number)
    .map((v) => v / 255).map((v) => v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4)
    .reduce((sum, v, i) => sum + v * [.2126, .7152, .0722][i], 0)
  const values = [luminance(a), luminance(b)].sort((x, y) => y - x)
  return (values[0] + .05) / (values[1] + .05)
}

async function dragText(page, selector) {
  const element = page.locator(selector)
  await element.scrollIntoViewIfNeeded()
  const points = await element.evaluate((el) => {
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT)
    let node
    while ((node = walker.nextNode())) if (node.textContent.trim().length >= 8) break
    const range = document.createRange()
    range.setStart(node, 0)
    range.setEnd(node, 8)
    const box = range.getBoundingClientRect()
    window.getSelection().removeAllRanges()
    return { x: box.x + 1, y: box.y + box.height / 2, end: box.right - 1 }
  })
  await page.mouse.move(points.x, points.y)
  await page.mouse.down()
  await page.mouse.move(points.end, points.y, { steps: 12 })
  await page.mouse.up()
  assert.ok((await page.evaluate(() => window.getSelection().toString())).trim().length >= 5,
    `${selector}: dragging must actually select text`)
}

test("demo heading uses lowercase apparent in wordmark lilac", { skip: !url }, async () => {
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, headless: true })
  try {
    const page = await browser.newPage()
    await page.goto(url)
    assert.equal(await page.locator(".pond-demo h2").textContent(), "See what apparent captures")
    const color = await page.locator(".pond-demo h2 span").evaluate((el) => getComputedStyle(el).color)
    const background = await page.locator(".pond-demo").evaluate((el) => getComputedStyle(el).backgroundColor)
    assert.equal(color, lilac)
    assert.ok(contrast(color, background) >= 4.5)
  } finally { await browser.close() }
})

test("drag selections are readable lilac on light and dark surfaces", { skip: !url }, async () => {
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, headless: true })
  try {
    const page = await browser.newPage()
    await page.goto(url)
    await page.evaluate(() => document.fonts.ready)
    if (evidence) mkdirSync(evidence, { recursive: true })
    for (const width of [1280, 390, 320]) {
      await page.setViewportSize({ width, height: 850 })
      for (const [name, selector] of [["hero", ".pond-copy p"], ["demo", ".pond-demo h2"]]) {
        await dragText(page, selector)
        for (const target of name === "demo" ? [selector, `${selector} span`] : [selector]) {
          const colors = await page.locator(target).evaluate((el) => {
            const style = getComputedStyle(el, "::selection")
            return { background: style.backgroundColor, foreground: style.color }
          })
          assert.deepEqual(colors, { background: lilac, foreground: carbon })
          assert.ok(contrast(colors.background, colors.foreground) >= 4.5)
        }
        assert.equal(await page.evaluate(() => Math.max(document.documentElement.scrollWidth, document.body.scrollWidth) > innerWidth), false)
        if (evidence) await page.screenshot({ path: join(evidence, `${name}-${width}.png`) })
      }
    }
  } finally { await browser.close() }
})
