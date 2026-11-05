import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { test } from "node:test"
import { fileURLToPath } from "node:url"

const site = join(dirname(fileURLToPath(import.meta.url)), "../..")
const artifact = process.env.SITE_ARTIFACT ? resolve(process.env.SITE_ARTIFACT) : join(site, "dist-apparent")
const imageUrl = "https://app.tryapparent.com/social-preview-v1.png"
const alt = "Apparent wordmark in lilac on a carbon background."

function meta(html, key) {
  const tags = [...html.matchAll(/<meta\b[^>]*>/g)].map(([tag]) =>
    Object.fromEntries([...tag.matchAll(/([\w:]+)="([^"]*)"/g)].map(([, key, value]) => [key, value])))
  const matches = tags.filter((tag) => tag.property === key || tag.name === key)
  assert.equal(matches.length, 1, `expected exactly one ${key}`)
  return matches[0].content
}

for (const [file, htmlPath] of [["index.html", join(site, "index.html")], ["dist-apparent/index.html", join(artifact, "index.html")]]) {
  test(`${file} exposes a minimal wide brand preview without JavaScript`, () => {
    const head = readFileSync(htmlPath, "utf8").split("</head>")[0]
    assert.equal(meta(head, "og:image"), imageUrl)
    assert.equal(meta(head, "og:image:type"), "image/png")
    assert.equal(meta(head, "og:image:width"), "1200")
    assert.equal(meta(head, "og:image:height"), "630")
    assert.equal(meta(head, "og:image:alt"), alt)
    assert.equal(meta(head, "twitter:card"), "summary_large_image")
    assert.equal(meta(head, "twitter:image"), imageUrl)
    assert.equal(meta(head, "twitter:image:alt"), alt)
    assert.equal(meta(head, "og:url"), "https://app.tryapparent.com/")
  })
}

test("the public package includes the exact branded PNG, not the legacy Scrawl card", () => {
  const source = readFileSync(join(site, "public/assets/brand/social-preview-v1.png"))
  const packaged = readFileSync(join(artifact, "social-preview-v1.png"))
  assert.deepEqual(packaged, source)
  assert.equal(source.subarray(0, 8).toString("hex"), "89504e470d0a1a0a")
  assert.equal(source.readUInt32BE(16), 1200)
  assert.equal(source.readUInt32BE(20), 630)
  assert.ok(source.length < 1_000_000, "keep crawler downloads small")
})
