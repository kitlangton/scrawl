// Run: node --test public/site/scripts/browser-proof/test-og-stage-redirect.mjs
// OG_STAGE_HTML may point at the fix owner's candidate without editing this tree.
// This executes the real inline loader with inert DOM sinks, not a browser or
// the concept renderer. No script is fetched, evaluated, or sent over a network.
import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { test } from "node:test"
import { createContext, runInContext } from "node:vm"
import { parse } from "parse5"

const html = readFileSync(process.env.OG_STAGE_HTML || new URL("../og-video/stage.html", import.meta.url), "utf8")
function inlineScriptCode(html) {
  const scripts = []
  function visit(node) {
    if (node.namespaceURI === "http://www.w3.org/1999/xhtml" && node.tagName === "script" && !node.attrs.some((attr) => attr.name === "src")) {
      scripts.push(node.childNodes.map((child) => child.value ?? "").join(""))
    }
    // Template content is inert and is not part of document.childNodes.
    for (const child of node.childNodes ?? []) visit(child)
  }
  visit(parse(html))
  return scripts
}
const inlineScripts = inlineScriptCode(html)
assert.ok(inlineScripts.length, "the stage must have an executable inline loader")

test("extracts actual inline scripts with HTML closing syntax and attributes", () => {
  assert.deepEqual(inlineScriptCode(`<script SRC="external.js"></script>
    <template><script>inert()</script></template>
    <SCRIPT data-note=">">actual()</sCrIpT\t\n bar>`), ["actual()"])
})
const base = "https://stage.example/scripts/og-video/stage.html"
const concepts = ["apparent-fix", "apparent-lasso", "apparent-pile"]

function load(search) {
  const assigned = []
  const appended = []
  const ctx = { setTransform() {}, save() {}, restore() {} }
  const document = {
    createElement(tag) {
      assert.equal(tag, "script")
      return {
        set src(value) {
          this.url = new URL(String(value), base).href
          assigned.push(this.url)
        },
        get src() { return this.url },
      }
    },
    body: { appendChild(script) { appended.push(script) } },
    fonts: { load: async () => [], ready: Promise.resolve() },
    getElementById(id) {
      return id === "c" ? { getContext: () => ctx } : { textContent: "" }
    },
  }
  const sandbox = createContext({
    document, location: { search }, URLSearchParams, URL,
    performance: { now: () => 0 }, requestAnimationFrame() {}, FPS: 60,
  })
  sandbox.window = sandbox
  let error
  try {
    for (const code of inlineScripts) runInContext(code, sandbox, { timeout: 1000 })
  } catch (caught) {
    // Explicit refusal is allowed, but must precede both URL assignment and
    // attachment. Positive controls below prevent broken loaders passing here.
    error = caught
  }
  return { assigned, appended, sandbox, ctx, error }
}

for (const name of concepts) {
  for (const selection of [name, name.replace("apparent", "%61pparent")]) {
    test(`valid concept ${selection} retains capture initialization`, async () => {
      const result = load(`?c=${selection}&capture`)
      assert.equal(result.error, undefined)
      const expected = new URL(`${name}.js`, base).href
      assert.deepEqual(result.assigned, [expected])
      assert.deepEqual(result.appended.map((script) => script.src), [expected])
      let initialized = 0
      result.sandbox.register({ frames: 12, init(ctx) {
        assert.equal(ctx, result.ctx)
        initialized++
      }, draw() {} })
      await result.appended[0].onload()
      assert.equal(initialized, 1)
      assert.equal(result.sandbox.ready, true)
      assert.equal(result.sandbox.frameCount, 12)
      assert.equal(typeof result.sandbox.renderFrame, "function")
    })
  }
}

const invalid = [
  "", "unknown", "common", "capture", "constructor", "__proto__", "toString",
  "https://evil.example/payload", "http://evil.example/payload",
  "//evil.example/payload", "///evil.example/payload",
  "\\\\evil.example/payload", "https:\\evil.example/payload",
  "javascript:alert(1)", "data:text/javascript,alert(1)",
  "/payload", "../payload", "../../payload", "./apparent-pile",
  "apparent-pile/../payload", "apparent-pile.js", "APPARENT-PILE",
  " apparent-pile", "apparent-pile ", "apparent-pile\n", "apparent-pile\0",
  "apparent-pile?redirect=https://evil.example", "apparent-pile#payload",
  "apparent-pile&capture", "%2f%2fevil.example/payload", "%2e%2e/payload",
  "%61pparent-pile", "apparent-pile%3fpayload", "apparent-pile%23payload",
]
for (const selection of invalid) {
  test(`reject selection ${JSON.stringify(selection)} before touching a script URL`, () => {
    const result = load(`?c=${encodeURIComponent(selection)}&capture`)
    assert.deepEqual(result.assigned, [], "invalid selection reached script.src")
    assert.deepEqual(result.appended, [], "invalid script was attached")
    assert.notEqual(result.sandbox.ready, true)
  })
}

for (const search of [
  "", "?capture", "?c&capture",
  "?c=%2f%2fevil.example%2fpayload&capture",
  "?c=%2e%2e%2fpayload&capture",
  "?c=apparent-pile%3fpayload&capture",
  "?c=apparent-pile%23payload&capture",
  "?c=%E0%A4%A&capture",
  "?c=//evil.example/payload&c=apparent-pile&capture",
]) {
  test(`reject raw query ${JSON.stringify(search)} before touching a script URL`, () => {
    const result = load(search)
    assert.deepEqual(result.assigned, [], "invalid query reached script.src")
    assert.deepEqual(result.appended, [], "invalid script was attached")
    assert.notEqual(result.sandbox.ready, true)
  })
}
