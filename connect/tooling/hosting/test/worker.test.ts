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
  for (const path of ["/", "/unknown", "/connect/other", "/hosted", "/connectivity", "/connect%2Fdevice", "/connect/assets/main.js.map", "/connect/assets/.env", "/connect/assets/manifest.json", "/connect/assets/private.json", "/connect/private/secret"]) {
    const response = await worker.fetch(new Request(`https://app.tryapparent.com${path}`), { ASSETS: assets } as never)
    assert.equal(response.status, 404, path)
  }
  for (const path of ["/connect/assets/source-pond-v4.png", "/connect/assets/agent-pond-v4.png", "/connect/assets/agent-reading-v4.png", "/connect/assets/main-a1b2.js"]) {
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

test("exact legacy hosted routes redirect to the canonical origin without touching assets", async () => {
  let assetCalls = 0
  const env = {
    SITE_ORIGIN: "https://app.tryapparent.com",
    ASSETS: { fetch: async () => { assetCalls++; return new Response("unexpected") } },
  } as never
  const routes = [
    ["/hosted", "/connect"], ["/hosted/", "/connect/"],
    ["/hosted/device", "/connect/device"], ["/hosted/device/", "/connect/device/"],
    ["/hosted/shares", "/connect/shares"], ["/hosted/shares/", "/connect/shares/"],
    ["/hosted/source-pond-v4.png", "/connect/assets/source-pond-v4.png"],
    ["/hosted/agent-pond-v4.png", "/connect/assets/agent-pond-v4.png"],
    ["/hosted/agent-reading-v4.png", "/connect/assets/agent-reading-v4.png"],
  ]
  for (const [legacy, canonical] of routes) {
    const query = "?code=AB%2fCD&keep=a+b"
    const response = await worker.fetch(new Request(`https://attacker.invalid${legacy}${query}`, {
      headers: { Cookie: "session=private", Authorization: "Bearer private" },
    }), env)
    assert.equal(response.status, 302, legacy)
    assert.equal(response.headers.get("location"), `https://app.tryapparent.com${canonical}${query}`)
    assert.equal(response.headers.get("cache-control"), "no-store")
    assert.equal(response.headers.get("referrer-policy"), "no-referrer")
    assert.equal(response.headers.get("x-robots-tag"), "noindex, nofollow, noarchive")
    assert.equal(response.headers.has("set-cookie"), false)
    assert.equal(await response.text(), "")
  }
  const head = await worker.fetch(new Request("https://attacker.invalid/hosted/device?code=HEAD%2fCODE", { method: "HEAD" }), env)
  assert.equal(head.status, 302)
  assert.equal(head.headers.get("location"), "https://app.tryapparent.com/connect/device?code=HEAD%2fCODE")
  assert.equal(await head.text(), "")
  for (const path of ["/hosted/other", "/hosted/device/extra", "/hosted/%64evice", "/hosted/private.json", "/hosted/source-pond-v5.png"]) {
    assert.equal((await worker.fetch(new Request(`https://app.tryapparent.com${path}`), env)).status, 404, path)
  }
  const post = await worker.fetch(new Request("https://app.tryapparent.com/hosted/device?code=PRIVATE", { method: "POST" }), env)
  assert.equal(post.status, 405)
  assert.equal(post.headers.get("allow"), "GET, HEAD")
  assert.equal(assetCalls, 0)
  const unsafeOrigin = await worker.fetch(new Request("https://app.tryapparent.com/hosted"), {
    SITE_ORIGIN: "https://attacker.invalid",
    ASSETS: { fetch: async () => { assetCalls++; return new Response("unexpected") } },
  } as never)
  assert.equal(unsafeOrigin.status, 404)
  assert.equal(assetCalls, 0)
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

test("bundled hosting worker keeps both canonical and exact legacy API/auth profiles on synthetic D1/R2", async () => {
  const previous = process.cwd()
  process.chdir(resolve(import.meta.dirname, "../../../service"))
  let f: Awaited<ReturnType<typeof fixture>>
  try {
    f = await fixture({
      API_ORIGIN: "https://app.tryapparent.com/connect",
      SITE_ORIGIN: "https://app.tryapparent.com",
      LEGACY_SITE_ORIGIN: "https://hosted.tryapparent.com",
    }, {
      script: readFileSync(resolve(import.meta.dirname, "../.output/worker.js"), "utf8"),
      assets: { directory: resolve(import.meta.dirname, "../.output/dist"), binding: "ASSETS", routerConfig: { invoke_user_worker_ahead_of_assets: true, has_user_worker: true } },
    })
  } finally { process.chdir(previous) }
  try {
    const ui = await f.mf.dispatchFetch("https://app.tryapparent.com/connect?code=SYNTHETIC")
    assert.equal(ui.status, 200, "the exact deployable bundle must include the UI gateway")
    assert.equal(ui.headers.get("x-apparent-publisher"), "connect-worker")
    assert.match(await ui.text(), /Checking your account/)
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
    const oldRecording = await f.mf.dispatchFetch("https://hosted.tryapparent.com/s/" + "b".repeat(64) + "?source=legacy", { redirect: "manual" })
    assert.equal(oldRecording.status, 302, "compiled legacy document route must be canonical")
    assert.equal(oldRecording.headers.get("location"), "https://app.tryapparent.com/connect/s/" + "b".repeat(64) + "?source=legacy")
    assert.equal(oldRecording.headers.has("set-cookie"), false)
    const oldSession = await f.mf.dispatchFetch("https://hosted.tryapparent.com/v1/session")
    assert.equal(oldSession.status, 200)
    assert.deepEqual(await oldSession.json(), { authenticated: false, csrfToken: null, account: null, slots: { used: 0, limit: 20, remaining: 20 } })
    const oldDenied = await f.mf.dispatchFetch("https://hosted.tryapparent.com/v1/account")
    assert.equal(oldDenied.status, 401)
    const oldSignIn = await f.mf.dispatchFetch("https://hosted.tryapparent.com/auth/google/start?returnTo=%2Fhosted%2Fdevice%3Fcode%3DSYNTHETIC", { redirect: "manual" })
    assert.equal(oldSignIn.status, 302)
    assert.equal(oldSignIn.headers.has("set-cookie"), true)
    const oldGoogle = new URL(oldSignIn.headers.get("location")!)
    assert.equal(oldGoogle.searchParams.get("redirect_uri"), "https://hosted.tryapparent.com/auth/google/callback")
    assert.equal(oldGoogle.searchParams.get("state") !== null, true)

    // Existing host-only Better Auth cookies continue to authenticate against
    // the shared D1 data and legacy Origin/CSRF contract.
    const oldIdentity = await f.identity("legacy-session")
    const oldAuthenticated = await f.mf.dispatchFetch("https://hosted.tryapparent.com/v1/session", {
      headers: { Cookie: oldIdentity.cookie, Origin: "https://hosted.tryapparent.com" },
    })
    assert.equal(oldAuthenticated.status, 200)
    assert.equal((await oldAuthenticated.json()).authenticated, true)
    const siteSession = await f.mf.dispatchFetch("https://hosted.tryapparent.com/v1/session", {
      headers: { Cookie: oldIdentity.cookie, Origin: "https://app.tryapparent.com" },
    })
    assert.equal(siteSession.headers.get("access-control-allow-origin"), "https://app.tryapparent.com")
    assert.equal((await siteSession.json()).authenticated, true)
    const rejectedLogout = await f.mf.dispatchFetch("https://hosted.tryapparent.com/auth/logout", {
      method: "POST", headers: { Cookie: oldIdentity.cookie, Origin: "https://attacker.invalid", "X-CSRF-Token": oldIdentity.csrf },
    })
    assert.equal(rejectedLogout.status, 403)
    const oldLogout = await f.mf.dispatchFetch("https://hosted.tryapparent.com/auth/logout", {
      method: "POST", headers: { Cookie: oldIdentity.cookie, Origin: "https://hosted.tryapparent.com", "X-CSRF-Token": oldIdentity.csrf },
    })
    assert.equal(oldLogout.status, 200)
    assert.equal(oldLogout.headers.has("set-cookie"), true)

    const appIdentity = await f.identity("legacy-app-origin-session")
    const appLogout = await f.mf.dispatchFetch("https://hosted.tryapparent.com/auth/logout", {
      method: "POST", headers: { Cookie: appIdentity.cookie, Origin: "https://app.tryapparent.com", "X-CSRF-Token": appIdentity.csrf },
    })
    assert.equal(appLogout.status, 200, "the actual deployed app-origin CSRF contract is preserved")
    const oldViewer = await f.mf.dispatchFetch("https://hosted.tryapparent.com/share-view.js")
    assert.equal(oldViewer.status, 200)
    assert.match(oldViewer.headers.get("content-type") ?? "", /text\/javascript/)
    const oldViewerHead = await f.mf.dispatchFetch("https://hosted.tryapparent.com/share-font.ttf", { method: "HEAD" })
    assert.equal(oldViewerHead.status, 200)
    assert.equal(await oldViewerHead.text(), "")

    // Unknown legacy paths and attacker hosts do not enter service-private routes.
    for (const [host, path] of [["hosted.tryapparent.com", "/auth/private/secret"], ["hosted.tryapparent.com", "/private/secret"], ["attacker.invalid", "/auth/google/start"], ["hosted.tryapparent.com:444", "/v1/session"]]) {
      const denied = await f.mf.dispatchFetch(`https://${host}${path}`)
      assert.equal(denied.status, 404, `${host}${path}`)
    }
    const font = await f.call("/share-font.ttf")
    assert.equal(font.status, 200)
    assert.match(font.headers.get("content-type") ?? "", /font\/ttf/)
    assert.equal((await f.call("/private/secret")).status, 404)
  } finally { await f.close() }
})


test("legacy recording documents redirect canonically without redirecting metadata or downloads", async () => {
  const token = "a".repeat(64)
  const env = { API_ORIGIN: "https://app.tryapparent.com/connect", SITE_ORIGIN: "https://app.tryapparent.com",
    ASSETS: { fetch: async () => { throw new Error("must not read static assets") } } } as never
  for (const method of ["GET", "HEAD"]) {
    const result = await worker.fetch(new Request(`https://hosted.tryapparent.com/s/${token}?source=old%2flink&x=a+b`, {
      method, headers: { Cookie: "private", Authorization: "Bearer private", Host: "attacker.invalid" },
    }), env)
    assert.equal(result.status, 302)
    assert.equal(result.headers.get("location"), `https://app.tryapparent.com/connect/s/${token}?source=old%2flink&x=a+b`)
    assert.equal(result.headers.get("cache-control"), "no-store")
    assert.equal(result.headers.get("referrer-policy"), "no-referrer")
    assert.match(result.headers.get("x-robots-tag") ?? "", /noindex/)
    assert.equal(result.headers.has("set-cookie"), false)
    assert.equal(await result.text(), "")
  }
  for (const suffix of ["/manifest.json", "/files/summary.md"]) {
    const result = await worker.fetch(new Request(`https://hosted.tryapparent.com/s/${token}${suffix}`), env)
    assert.equal(result.headers.has("location"), false, "old API requests must not redirect")
  }
  const badConfig = await worker.fetch(new Request(`https://hosted.tryapparent.com/s/${token}`), { ...env as object, API_ORIGIN: "https://attacker.invalid/connect" } as never)
  assert.equal(badConfig.status, 404)
  assert.equal(badConfig.headers.has("location"), false)
})
