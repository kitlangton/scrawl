import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { test } from "node:test"
import { fileURLToPath } from "node:url"

const site = join(dirname(fileURLToPath(import.meta.url)), "../..")
const master = readFileSync(join(site, "../../shared/assets/brand/apparent-app-icon.svg"), "utf8")

test("favicon reuses the canonical boxed a artwork", () => {
  assert.equal(readFileSync(join(site, "public/assets/brand/icon.svg"), "utf8"), master)
})

test("homepage invalidates the cached brush favicon", () => {
  const html = readFileSync(join(site, "index.html"), "utf8")
  assert.match(html, /<link rel="icon" href="\/icon\.svg\?v=brand-a-1" type="image\/svg\+xml" \/>/)
})
