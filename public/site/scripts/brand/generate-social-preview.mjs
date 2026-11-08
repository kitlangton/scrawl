// Rebuild the minimal wide card from the canonical wordmark. No poster copy.
// CHROME_PATH=/path/to/chrome node scripts/generate-social-preview.mjs
import { chromium } from "playwright-core"
import { readFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"

const site = join(dirname(fileURLToPath(import.meta.url)), "../..")
// Invert the canonical Carbon-on-Lilac artwork without changing its letterforms.
const wordmark = readFileSync(join(site, "public/assets/brand/apparent-wordmark.svg"), "utf8")
  .replace('fill="#B7A5D8"', 'fill="#020102"')
  .replace('<g fill="#020102"', '<g fill="#B7A5D8"')
const image = `data:image/svg+xml;base64,${Buffer.from(wordmark).toString("base64")}`
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || undefined })
try {
  const page = await browser.newPage({ viewport: { width: 1200, height: 630 }, deviceScaleFactor: 1 })
  await page.setContent(`<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Apparent social preview</title><style>
    body { margin: 0; width: 1200px; height: 630px; display: grid; place-items: center; background: #020102; }
    img { display: block; width: 660px; height: auto; }
  </style></head><body>
    <img src="${image}" alt="Apparent wordmark">
  </body></html>`)
  await page.evaluate(async () => {
    await Promise.all([...document.images].map((image) => image.decode()))
  })
  await page.screenshot({ path: join(site, "public/assets/brand/social-preview-v1.png") })
  console.log("Wrote public/assets/brand/social-preview-v1.png (1200 × 630)")
} finally {
  await browser.close()
}
