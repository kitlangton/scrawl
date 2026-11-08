import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { test } from "node:test"

// Check the actual prerendered page, not an unused navigation component.
test("homepage header links to the repository with an honest star-count fallback", () => {
  const html = readFileSync(new URL("../../dist-apparent/index.html", import.meta.url), "utf8")
  const header = html.match(/<header\b[^>]*>([\s\S]*?)<\/header>/)?.[1] ?? ""
  assert.match(header, /class="github-stars"/)
  assert.match(header, /href="https:\/\/github.com\/tryapparent\/apparent"/)
  assert.match(header, /aria-label="Star Apparent on GitHub"/)
  assert.match(header, />GitHub<\/span>/)
  assert.doesNotMatch(header, />0<\/span>/, "unknown counts must not pretend to be zero")
})
