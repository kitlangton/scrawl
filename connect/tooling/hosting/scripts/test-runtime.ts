import { createRequire } from "node:module"
import { spawn } from "node:child_process"
import { createServer } from "node:net"
import { readdir, mkdtemp, cp, rm } from "node:fs/promises"
import assert from "node:assert/strict"
import { setTimeout as delay } from "node:timers/promises"
import { fileURLToPath } from "node:url"
import { validateOutput } from "./validate-output.ts"
import { tmpdir } from "node:os"
import { dirname, resolve } from "node:path"

const here = dirname(fileURLToPath(import.meta.url))
const packageRoot = resolve(here, "..")
const socket = createServer()
await new Promise<void>(r => socket.listen(0, "127.0.0.1", r))
const address = socket.address()
if (!address || typeof address === "string") throw new Error("could not reserve local test port")
const port = address.port
await new Promise<void>((resolveClose, reject) => socket.close(error => error ? reject(error) : resolveClose()))
await validateOutput(resolve(packageRoot, ".output"))
const runtimeRoot = await mkdtemp(resolve(tmpdir(), "connect-runtime-state-"))
// Test an unchanged copy of the deployment package: Wrangler may create cache
// directories beside its config even when persistence is redirected.
const deployment = resolve(runtimeRoot, "deployment")
await cp(resolve(packageRoot, ".output"), deployment, { recursive: true }).catch(async error => {
  await rm(runtimeRoot, { recursive: true, force: true })
  throw error
})
const runtimeState = resolve(runtimeRoot, "state")
const child = spawn(resolve(packageRoot, "node_modules/.bin/wrangler"), ["dev", "--config", resolve(deployment, "wrangler.json"), "--no-bundle", "--persist-to", runtimeState, "--local", "--ip", "127.0.0.1", "--port", String(port)], { cwd: runtimeRoot, detached: true, stdio: "ignore", env: { ...process.env, NO_COLOR: "1" } })
let spawnError: Error | undefined
const exited = new Promise<void>(resolveExit => {
  child.once("exit", () => resolveExit())
  child.once("error", error => { spawnError = error; resolveExit() })
})
const origin = `http://127.0.0.1:${port}`
try {
  let ready = false
  for (let attempt = 0; attempt < 80 && !ready; attempt++) {
    if (spawnError) throw spawnError
    if (child.exitCode !== null) throw new Error(`wrangler dev exited ${child.exitCode}`)
    try { ready = (await fetch(`${origin}/connect`)).status === 200 } catch { await delay(250) }
  }
  assert(ready, "Workers local runtime did not become ready")
  for (const route of ["connect", "connect/device", "connect/shares"]) for (const slash of ["", "/"]) {
    const url = `${origin}/${route}${slash}?code=RUNTIME-CODE&source=synthetic`
    const response = await fetch(url)
    assert.equal(response.status, 200, url)
    assert.equal(new URL(response.url).search, "?code=RUNTIME-CODE&source=synthetic")
    assert.equal(response.headers.get("cache-control"), "no-store")
    assert.equal(response.headers.get("referrer-policy"), "no-referrer")
    assert.match(response.headers.get("x-robots-tag") ?? "", /noindex.*nofollow/)
    const html = await response.text()
    assert.match(html, /Checking your account/)
    assert.match(html, /noindex, nofollow/)
    assert.match(html, /\/connect\/assets\//)
  }
  const head = await fetch(`${origin}/connect/device?code=HEAD-CODE`, { method: "HEAD" })
  assert.equal(head.status, 200)
  assert.equal(await head.text(), "")
  assert.equal((await fetch(`${origin}/connect`, { method: "POST" })).status, 405)
  for (const path of ["/", "/unknown", "/connect/other", "/hosted/other", "/connectivity", "/connect%2Fdevice", "/connect/assets/%2e%2e%2findex.js", "/connect/assets/.env", "/connect/assets/manifest.json", "/connect/assets/private.json", "/connect/assets/missing.js", "/connect/private/secret"]) {
    const response = await fetch(`${origin}${path}`)
    assert.equal(response.status, 404, path)
    assert.equal(response.headers.get("cache-control"), "no-store")
  }
  const browserAssets = await readdir(resolve(packageRoot, ".output/dist/connect/assets"))
  const script = browserAssets.find(name => name.endsWith(".js"))
  const stylesheet = browserAssets.find(name => name.endsWith(".css"))
  assert(script && stylesheet, "packaged JS and CSS exist under the owned asset namespace")
  for (const [path, mediaType, cacheDirective] of [["/connect/assets/apparent-wordmark.svg", "image/svg+xml", "no-cache"], ["/connect/assets/HostGrotesk-Variable.ttf", "font/ttf", "no-cache"], ["/connect/assets/source-pond-v4.png", "image/png", "no-cache"], [`/connect/assets/${script}`, "text/javascript", "no-cache"], [`/connect/assets/${stylesheet}`, "text/css", "no-cache"]]) {
    const response = await fetch(`${origin}${path}`)
    assert.equal(response.status, 200, path)
    assert.match(response.headers.get("content-type") ?? "", new RegExp(mediaType.replace("+", "\\+")), path)
    assert.match(response.headers.get("cache-control") ?? "", new RegExp(cacheDirective), path)
  }
  const noCookieEffect = await fetch(`${origin}/connect`, { headers: { Cookie: "synthetic=private", Authorization: "Bearer synthetic" } })
  assert.equal(noCookieEffect.status, 200)
  assert.equal(noCookieEffect.headers.has("set-cookie"), false)
  const serviceNotFound = await fetch(`${origin}/connect/v1/private-unknown`)
  assert.equal(serviceNotFound.status, 404)
  assert.deepEqual(await serviceNotFound.json(), { error: { code: "not_found", message: "not found" } })
  const viewerAsset = await fetch(`${origin}/connect/share-view.js`, { headers: { Cookie: "synthetic=private" } })
  assert.equal(viewerAsset.status, 200)
  assert.match(viewerAsset.headers.get("content-type") ?? "", /text\/javascript/)
  assert.equal(viewerAsset.headers.has("set-cookie"), false)
  const legacyRoutes = [
    ["/hosted", "/connect"], ["/hosted/", "/connect/"],
    ["/hosted/device", "/connect/device"], ["/hosted/device/", "/connect/device/"],
    ["/hosted/shares", "/connect/shares"], ["/hosted/shares/", "/connect/shares/"],
    ["/hosted/source-pond-v4.png", "/connect/assets/source-pond-v4.png"],
    ["/hosted/agent-pond-v4.png", "/connect/assets/agent-pond-v4.png"],
    ["/hosted/agent-reading-v4.png", "/connect/assets/agent-reading-v4.png"],
  ]
  for (const [legacy, canonical] of legacyRoutes) {
    const query = "?code=RUNTIME%2fCODE&keep=a+b"
    const response = await fetch(`${origin}${legacy}${query}`, { redirect: "manual", headers: { Cookie: "synthetic=private", Authorization: "Bearer synthetic" } })
    assert.equal(response.status, 302, legacy)
    assert.equal(response.headers.get("location"), `https://app.tryapparent.com${canonical}${query}`)
    assert.equal(response.headers.get("cache-control"), "no-store")
    assert.equal(response.headers.get("referrer-policy"), "no-referrer")
    assert.equal(response.headers.get("x-robots-tag"), "noindex, nofollow, noarchive")
    assert.equal(response.headers.has("set-cookie"), false)
    assert.equal(await response.text(), "")
  }
  const legacyHead = await fetch(`${origin}/hosted/device?code=HEAD%2fCODE`, { method: "HEAD", redirect: "manual" })
  assert.equal(legacyHead.status, 302)
  assert.equal(legacyHead.headers.get("location"), "https://app.tryapparent.com/connect/device?code=HEAD%2fCODE")
  assert.equal(await legacyHead.text(), "")
  const legacyPost = await fetch(`${origin}/hosted/device?code=PRIVATE`, { method: "POST", redirect: "manual" })
  assert.equal(legacyPost.status, 405)
  assert.equal(legacyPost.headers.get("allow"), "GET, HEAD")
  for (const path of ["/hosted/other", "/hosted/device/extra", "/hosted/%64evice", "/hosted/private.json", "/hosted/source-pond-v5.png"]) {
    const response = await fetch(`${origin}${path}`)
    assert.equal(response.status, 404, path)
    assert.equal(response.headers.get("cache-control"), "no-store")
  }
  // Exercise hydration against the rewritten package through the real asset binding.
  // Every nonlocal request is intercepted; no live account or telemetry is used.
  const { chromium } = createRequire(resolve(packageRoot, "../../web/package.json"))("playwright-core")
  const browser = await chromium.launch({ headless: true, args: ["--no-sandbox"] })
  try {
    const page = await browser.newPage()
    const errors: string[] = [], unexpected: string[] = []
    page.on("pageerror", (error: Error) => errors.push(error.message))
    await page.route("**/*", async (route: any) => {
      const url = new URL(route.request().url())
      if (url.origin === origin) return route.continue()
      if (url.origin === "https://app.tryapparent.com" && url.pathname === "/connect/v1/session") {
        return route.fulfill({ status: 200, contentType: "application/json", headers: { "Access-Control-Allow-Origin": origin, "Access-Control-Allow-Credentials": "true" }, body: JSON.stringify({ authenticated: false }) })
      }
      unexpected.push(url.origin + url.pathname)
      return route.abort()
    })
    for (const route of ["connect", "connect/device", "connect/shares"]) {
      await page.goto(origin + "/" + route + "?code=RUNTIME-CODE")
      if (route === "connect") await page.getByRole("button", { name: "Connect", exact: true }).click()
      const signIn = page.getByRole("link", { name: "Continue with Google", exact: true })
      await signIn.waitFor()
      const target = new URL(await signIn.getAttribute("href"))
      assert.equal(target.origin, "https://app.tryapparent.com")
      assert.equal(target.searchParams.get("returnTo"), route.endsWith("/device") ? "/connect/device?code=RUNTIME-CODE" : "/connect")
      assert.equal(await page.locator("body").evaluate((body: HTMLElement) => body.scrollWidth > innerWidth), false)
    }
    assert.deepEqual(errors, [])
    assert.deepEqual(unexpected, [])
  } finally { await browser.close() }

} finally {
  const signalGroup = (signal: NodeJS.Signals) => {
    try { if (child.pid) process.kill(-child.pid, signal) }
    catch (error) { if ((error as NodeJS.ErrnoException).code !== "ESRCH") throw error }
  }
  signalGroup("SIGTERM")
  const stopped = await Promise.race([exited.then(() => true), delay(3000).then(() => false)])
  // Terminate any remaining runtime descendants before removing SQLite/WALs.
  signalGroup("SIGKILL")
  if (!stopped) await exited
  let groupExists = Boolean(child.pid)
  for (let attempt = 0; attempt < 80 && groupExists; attempt++) {
    try { process.kill(-child.pid!, 0); await delay(25) }
    catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ESRCH") throw error
      groupExists = false
    }
  }
  if (groupExists) throw new Error(`Wrangler group has not terminated; private state retained at ${runtimeRoot}`)
  await rm(runtimeRoot, { recursive: true, force: true })
}
