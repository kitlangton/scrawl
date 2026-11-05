import assert from "node:assert/strict"
import test from "node:test"
import worker from "../src/index.ts"

const routes = ["connect", "connect/device", "connect/shares", "hosted", "hosted/device", "hosted/shares"]

test("only the six browser management routes and owned static files reach assets", async () => {
  const seen: Request[] = []
  const assets = { fetch: async (request: Request) => {
    seen.push(request)
    return new Response(request.url.includes("index.html") ? "<meta name=robots content='noindex, nofollow'>" : "asset", { headers: { "Content-Type": request.url.includes("index.html") ? "text/html" : "image/png" } })
  } } as unknown as Fetcher
  for (const route of routes) for (const slash of ["", "/"]) {
    const requestUrl = `https://app.tryapparent.com/${route}${slash}?code=USERCODE&keep=1`
    const response = await worker.fetch(new Request(requestUrl, { headers: { Cookie: "session=private", Authorization: "Bearer private" } }), { ASSETS: assets } as never)
    assert.equal(response.status, 200)
    assert.equal(response.headers.get("Cache-Control"), "no-store")
    assert.equal(response.headers.get("Referrer-Policy"), "no-referrer")
    assert.equal(response.headers.get("X-Robots-Tag"), "noindex, nofollow, noarchive")
    const forwarded = seen.at(-1)!
    assert.equal(new URL(forwarded.url).search, "?code=USERCODE&keep=1")
    assert.equal(forwarded.headers.has("cookie"), false)
    assert.equal(forwarded.headers.has("authorization"), false)
  }
  for (const path of ["/", "/unknown", "/connect/other", "/hosted/device/extra", "/connectivity", "/connect%2Fdevice", "/connect/assets/main.js.map", "/connect/assets/.env", "/connect/assets/manifest.json", "/hosted/private.json"]) {
    const response = await worker.fetch(new Request(`https://app.tryapparent.com${path}`), { ASSETS: assets } as never)
    assert.equal(response.status, 404, path)
  }
  for (const path of ["/hosted/source-pond-v4.png", "/hosted/agent-pond-v4.png", "/hosted/agent-reading-v4.png", "/connect/assets/main-a1b2.js"]) {
    const response = await worker.fetch(new Request(`https://app.tryapparent.com${path}`), { ASSETS: assets } as never)
    assert.equal(response.status, 200)
  }
})

test("HEAD is bodyless and other methods fail closed without touching assets", async () => {
  let calls = 0
  const assets = { fetch: async () => { calls++; return new Response("body", { headers: { "Content-Type": "text/html" } }) } } as unknown as Fetcher
  const head = await worker.fetch(new Request("https://app.tryapparent.com/connect/device?code=USERCODE", { method: "HEAD" }), { ASSETS: assets } as never)
  assert.equal(head.status, 200)
  assert.equal(await head.text(), "")
  const post = await worker.fetch(new Request("https://app.tryapparent.com/connect", { method: "POST" }), { ASSETS: assets } as never)
  assert.equal(post.status, 405)
  assert.equal(calls, 1)
})
