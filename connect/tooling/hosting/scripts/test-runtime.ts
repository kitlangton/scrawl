import { spawn } from "node:child_process"
import { createServer } from "node:net"
import { readdir } from "node:fs/promises"
import assert from "node:assert/strict"
import { setTimeout as delay } from "node:timers/promises"
import { fileURLToPath } from "node:url"
import { dirname, resolve } from "node:path"

const here = dirname(fileURLToPath(import.meta.url))
const packageRoot = resolve(here, "..")
const socket = createServer()
await new Promise<void>(r => socket.listen(0, "127.0.0.1", r))
const address = socket.address()
if (!address || typeof address === "string") throw new Error("could not reserve local test port")
const port = address.port
await new Promise<void>((resolveClose, reject) => socket.close(error => error ? reject(error) : resolveClose()))
const child = spawn(resolve(packageRoot, "node_modules/.bin/wrangler"), ["dev", "--config", "wrangler.template.jsonc", "--local", "--ip", "127.0.0.1", "--port", String(port)], { cwd: packageRoot, stdio: "ignore", env: { ...process.env, NO_COLOR: "1" } })
const origin = `http://127.0.0.1:${port}`
try {
  let ready = false
  for (let attempt = 0; attempt < 80 && !ready; attempt++) {
    if (child.exitCode !== null) throw new Error(`wrangler dev exited ${child.exitCode}`)
    try { ready = (await fetch(`${origin}/connect`)).status === 200 } catch { await delay(250) }
  }
  assert(ready, "Workers local runtime did not become ready")
  for (const route of ["connect", "connect/device", "connect/shares", "hosted", "hosted/device", "hosted/shares"]) for (const slash of ["", "/"]) {
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
  for (const path of ["/", "/unknown", "/connect/other", "/hosted/device/extra", "/connectivity", "/connect%2Fdevice", "/connect/assets/%2e%2e%2findex.js", "/connect/assets/.env", "/connect/assets/manifest.json", "/hosted/private.json", "/connect/assets/missing.js"]) {
    const response = await fetch(`${origin}${path}`)
    assert.equal(response.status, 404, path)
    assert.equal(response.headers.get("cache-control"), "no-store")
  }
  const browserAssets = await readdir(resolve(packageRoot, ".output/dist/connect/assets"))
  const script = browserAssets.find(name => name.endsWith(".js"))
  const stylesheet = browserAssets.find(name => name.endsWith(".css"))
  assert(script && stylesheet, "packaged JS and CSS exist under the owned asset namespace")
  for (const [path, mediaType, cacheDirective] of [["/connect/assets/apparent-wordmark.svg", "image/svg+xml", "no-cache"], ["/connect/assets/HostGrotesk-Variable.ttf", "font/ttf", "no-cache"], ["/hosted/source-pond-v4.png", "image/png", "no-cache"], [`/connect/assets/${script}`, "text/javascript", "immutable"], [`/connect/assets/${stylesheet}`, "text/css", "immutable"]]) {
    const response = await fetch(`${origin}${path}`)
    assert.equal(response.status, 200, path)
    assert.match(response.headers.get("content-type") ?? "", new RegExp(mediaType.replace("+", "\\+")), path)
    assert.match(response.headers.get("cache-control") ?? "", new RegExp(cacheDirective), path)
  }
  const noCookieEffect = await fetch(`${origin}/connect`, { headers: { Cookie: "synthetic=private", Authorization: "Bearer synthetic" } })
  assert.equal(noCookieEffect.status, 200)
  assert.equal(noCookieEffect.headers.has("set-cookie"), false)
} finally {
  child.kill("SIGTERM")
  await Promise.race([new Promise<void>(resolveExit => child.once("exit", () => resolveExit())), delay(3000)])
}
