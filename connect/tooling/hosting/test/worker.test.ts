import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { resolve } from "node:path"
import test from "node:test"
import worker from "../src/index.ts"
// @ts-expect-error the existing service test fixture is JavaScript
import { fixture } from "../../../service/test/helpers.mjs"

const routes = ["connect", "connect/device", "connect/shares"]

test("only canonical browser routes and owned static files reach assets", async () => {
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
    assert.equal(new URL(forwarded.url).search, "", "device codes must not reach the static binding")
    assert.equal(forwarded.headers.has("cookie"), false)
    assert.equal(forwarded.headers.has("authorization"), false)
  }
  for (const path of ["/", "/unknown", "/connect/other", "/hosted", "/connectivity", "/connect%2Fdevice", "/connect/assets/main.js.map", "/connect/assets/.env", "/connect/assets/manifest.json", "/connect/hosted/private.json", "/connect/private/secret"]) {
    const response = await worker.fetch(new Request(`https://app.tryapparent.com${path}`), { ASSETS: assets } as never)
    assert.equal(response.status, 404, path)
  }
  for (const path of ["/connect/hosted/source-pond-v4.png", "/connect/hosted/agent-pond-v4.png", "/connect/hosted/agent-reading-v4.png", "/connect/assets/main-a1b2.js"]) {
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

test("canonical API and viewer requests dispatch to the existing service handler", async () => {
  let assets = 0
  const env = {
    ASSETS: { fetch: async () => { assets++; return new Response("must not reach assets") } },
    API_ORIGIN: "https://app.tryapparent.com/connect",
    SITE_ORIGIN: "https://app.tryapparent.com",
  } as never
  const unknownApi = await worker.fetch(new Request("https://app.tryapparent.com/connect/v1/private-unknown"), env)
  assert.equal(unknownApi.status, 404)
  assert.deepEqual(await unknownApi.json(), { error: { code: "not_found", message: "not found" } })
  assert.equal(unknownApi.headers.get("cache-control"), "no-store")
  assert.equal(unknownApi.headers.get("x-robots-tag"), "noindex, nofollow, noarchive")
  const viewer = await worker.fetch(new Request("https://app.tryapparent.com/connect/share-view.js"), env)
  assert.equal(viewer.status, 200)
  assert.match(viewer.headers.get("content-type") ?? "", /text\/javascript/)
  assert.match(await viewer.text(), /createRoot|hydrateRoot/)
  assert.equal(assets, 0)
})

test("bundled hosting worker uses synthetic D1/R2 service bindings and canonical OAuth callback", async () => {
  const previous = process.cwd()
  process.chdir(resolve(import.meta.dirname, "../../../service"))
  let f: Awaited<ReturnType<typeof fixture>>
  try {
    f = await fixture({
      API_ORIGIN: "https://app.tryapparent.com/connect",
      SITE_ORIGIN: "https://app.tryapparent.com",
    }, {
      script: readFileSync(resolve(import.meta.dirname, "../.output/worker.js"), "utf8"),
      assets: { directory: resolve(import.meta.dirname, "../.output/dist"), binding: "ASSETS", routerConfig: { invoke_user_worker_ahead_of_assets: true, has_user_worker: true } },
    })
  } finally { process.chdir(previous) }
  try {
    const session = await f.call("/v1/session")
    assert.equal(session.status, 200)
    assert.deepEqual(await session.json(), { authenticated: false, csrfToken: null, account: null, slots: { used: 0, limit: 20, remaining: 20 } })
    const denied = await f.call("/v1/account")
    assert.equal(denied.status, 401)
    assert.equal((await denied.json()).error.code, "unauthorized")
    const signIn = await f.call("/auth/google/start?returnTo=%2Fconnect/device%3Fcode%3DSYNTHETIC")
    assert.equal(signIn.status, 302)
    assert.equal(signIn.headers.has("set-cookie"), true, "dynamic auth cookies pass through the host unchanged")
    const google = new URL(signIn.headers.get("location")!)
    assert.equal(google.origin, "https://accounts.google.com")
    assert.equal(google.searchParams.get("redirect_uri"), "https://app.tryapparent.com/connect/auth/google/callback")
    const font = await f.call("/share-font.ttf")
    assert.equal(font.status, 200)
    assert.match(font.headers.get("content-type") ?? "", /font\/ttf/)
    assert.equal((await f.call("/private/secret")).status, 404)
  } finally { await f.close() }
})
