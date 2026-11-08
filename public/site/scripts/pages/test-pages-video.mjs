import assert from "node:assert/strict"
import { after, before, mock, test } from "node:test"
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { createHash } from "node:crypto"
import { onRequest } from "../../functions/Apparent-ghost-demo-v4-lilac.mp4.js"
import { movie, movieETag, chunkPath } from "../../lib/pages-video-metadata.js"
import { preparePagesVideo } from "./prepare-pages-video.mjs"

let output
let original
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex")
before(() => {
  // Node lacks this Workers API. The separate workerd HTTP regression checks
  // the real implementation and transmitted length, not this unit adapter.
  globalThis.FixedLengthStream = class extends TransformStream {
    constructor(length) {
      let remaining = length
      super({
        transform(bytes, controller) {
          remaining -= bytes.byteLength
          if (remaining < 0) throw new Error("Oversized Pages video asset")
          controller.enqueue(bytes)
        },
        flush() {
          if (remaining !== 0) throw new Error("Truncated Pages video asset")
        },
      })
    }
  }
  mock.method(globalThis, "fetch", () => { throw new Error("Remote fetch forbidden") })
  output = mkdtempSync(join(tmpdir(), "apparent-pages-video-"))
  original = readFileSync(new URL("../../public/assets/media/Apparent-ghost-demo-v4-lilac.mp4", import.meta.url))
  writeFileSync(join(output, movie.path.slice(1)), original)
  writeFileSync(join(output, "index.html"), "preserve homepage")
  preparePagesVideo(output)
})
after(() => { rmSync(output, { recursive: true, force: true }); mock.restoreAll(); delete globalThis.FixedLengthStream })

// A local static-asset binding, not an HTTP server or a handler substitute.
// Its ranged bodies are the real split files, delivered in bounded pieces.
function binding(fault = null) {
  const calls = []
  let pulls = 0
  let cancellations = 0
  let didCancel
  const cancelled = new Promise((resolve) => { didCancel = resolve })
  return {
    calls,
    cancelled,
    get pulls() { return pulls },
    get cancellations() { return cancellations },
    async fetch(request) {
      calls.push(request)
      assert.equal(request.method, "GET")
      assert.equal(new URL(request.url).origin, "https://app.tryapparent.com")
      assert.equal(new URL(request.url).search, "")
      assert.deepEqual([...request.headers.keys()].sort(), ["accept-encoding", "range"])
      const chunk = movie.chunks.find((entry) => chunkPath(entry) === new URL(request.url).pathname)
      assert.ok(chunk, "only known local chunk paths may be read")
      if (fault === "throw" || (fault === "later" && calls.length === 2)) throw new Error("Asset unavailable")
      const bytes = readFileSync(join(output, chunkPath(chunk).slice(1)))
      const [, a, b] = /^bytes=(\d+)-(\d+)$/.exec(request.headers.get("Range"))
      const start = fault === "pages-full" ? 0 : Number(a)
      const end = fault === "pages-full" ? chunk.size - 1 : Number(b)
      let position = start
      const final = fault === "truncated" ? end - 1 : fault === "oversized" ? end + 1 : end
      const body = new ReadableStream({
        pull(controller) {
          pulls++
          if (fault === "body-error") { controller.error(new Error("Asset read failed")); return }
          if (position > final) { controller.close(); return }
          const next = Math.min(position + 65536, final + 1)
          controller.enqueue(bytes.subarray(position, next))
          position = next
        },
        cancel() { cancellations++; didCancel() },
      }, { highWaterMark: 0 })
      return new Response(body, {
        status: fault === "404" ? 404 : ["ignored-range", "pages-full"].includes(fault) ? 200 : 206,
        headers: fault === "pages-full" ? {} : {
          "Content-Length": String(end - start + (fault === "bad-length" ? 2 : 1)),
          "Content-Range": fault === "bad-range" ? "bytes 0-1/2" : `bytes ${start}-${end}/${chunk.size}`,
          ...(fault === "encoding" ? { "Content-Encoding": "gzip" } : {}),
        },
      })
    },
  }
}
function serve(headers = {}, method = "GET", assets = binding(), path = movie.path) {
  return onRequest({ request: new Request(`https://app.tryapparent.com${path}`, { method, headers }), env: { ASSETS: assets } })
}

test("real approved source splits, reconstructs exactly, and prepares repeatably without widening routes", () => {
  assert.equal(original.length, 30999148)
  assert.equal(hash(original), movie.sha256)
  assert.equal(existsSync(join(output, movie.path.slice(1))), false)
  const chunks = movie.chunks.map((chunk) => {
    const bytes = readFileSync(join(output, chunkPath(chunk).slice(1)))
    assert.ok(bytes.length < 25 * 1024 * 1024)
    assert.equal(hash(bytes), chunk.sha256)
    return bytes
  })
  assert.deepEqual(Buffer.concat(chunks), original)
  assert.deepEqual(JSON.parse(readFileSync(join(output, "_routes.json"))), { version: 1, include: [movie.path], exclude: [] })
  assert.equal(readFileSync(join(output, "index.html"), "utf8"), "preserve homepage")
  assert.deepEqual(preparePagesVideo(output), { size: movie.size, chunks: 4, sha256: movie.sha256 })
  assert.deepEqual(readdirSync(output).sort(), ["_routes.json", "assets", "index.html"])
  assert.deepEqual(readdirSync(join(output, "assets")).sort(), movie.chunks.map((chunk) => chunkPath(chunk).split("/").at(-1)).sort())
})

test("preparation rejects wrong source and corrupt existing chunks, retaining original", () => {
  const fixture = mkdtempSync(join(tmpdir(), "apparent-pages-invalid-"))
  try {
    const source = join(fixture, movie.path.slice(1))
    writeFileSync(source, "not the approved movie")
    assert.throws(() => preparePagesVideo(fixture), /identity mismatch/)
    assert.equal(readFileSync(source, "utf8"), "not the approved movie")
    assert.equal(existsSync(join(fixture, "assets")), false)
    writeFileSync(source, original)
    mkdirSync(join(fixture, "assets"))
    const destination = join(fixture, chunkPath(movie.chunks[0]).slice(1))
    writeFileSync(destination, "corrupt chunk")
    assert.throws(() => preparePagesVideo(fixture), /identity mismatch/)
    assert.equal(hash(readFileSync(source)), movie.sha256)
    assert.equal(readFileSync(destination, "utf8"), "corrupt chunk")
    assert.equal(existsSync(join(fixture, "_routes.json")), false)
    rmSync(source)
    assert.throws(() => preparePagesVideo(fixture), /identity mismatch/)
  } finally { rmSync(fixture, { recursive: true, force: true }) }
})

test("actual handler streams the exact full movie from ASSETS only", async () => {
  const assets = binding()
  const response = await serve({ Cookie: "never-forward", Authorization: "never-forward" }, "GET", assets)
  assert.equal(response.status, 200)
  assert.equal(response.headers.get("Content-Type"), "video/mp4")
  assert.equal(response.headers.get("Content-Length"), String(original.length))
  assert.equal(response.headers.get("Accept-Ranges"), "bytes")
  assert.equal(response.headers.get("ETag"), movieETag)
  assert.equal(assets.calls.length, 1)
  assert.ok(assets.pulls <= 1, "the fixed-length transform buffers at most one bounded source block")
  assert.equal(hash(Buffer.from(await response.arrayBuffer())), movie.sha256)
  assert.equal(assets.calls.length, 4)
})

test("single, suffix, open-ended and cross-chunk ranges return exact source bytes", async () => {
  for (const [range, start, end] of [
    ["bytes=0-0", 0, 0],
    ["bytes=8388607-8388609", 8388607, 8388609],
    ["bytes=8388608-16777216", 8388608, 16777216],
    ["bytes=-128", movie.size - 128, movie.size - 1],
    ["bytes=30999140-", 30999140, movie.size - 1],
    ["bytes=30999140-99999999", 30999140, movie.size - 1],
    ["bytes=-99999999", 0, movie.size - 1],
  ]) {
    const response = await serve({ Range: range })
    assert.equal(response.status, 206, range)
    assert.equal(response.headers.get("Content-Range"), `bytes ${start}-${end}/${movie.size}`)
    assert.equal(response.headers.get("Content-Length"), String(end - start + 1))
    assert.deepEqual(Buffer.from(await response.arrayBuffer()), original.subarray(start, end + 1), range)
  }
})

test("HEAD, invalid ranges, unsupported methods and validators avoid all asset reads", async () => {
  const assets = binding()
  for (const headers of [{}, { Range: "bytes=0-1" }, { Range: "bytes=invalid" }]) {
    const response = await serve(headers, "HEAD", assets)
    assert.equal(response.status, 200)
    assert.equal(response.headers.get("Content-Length"), String(movie.size))
    assert.equal(response.body, null)
  }
  for (const range of ["bytes=0-1,3-4", "bytes=30999148-", "bytes=2-1", "bytes=-0", "bytes=-", "bytes=", "bytes=9007199254740992-", "items=0-1"]) {
    const response = await serve({ Range: range }, "GET", assets)
    assert.equal(response.status, 416, range)
    assert.equal(response.headers.get("Content-Range"), `bytes */${movie.size}`)
    assert.equal(response.body, null)
  }
  for (const method of ["POST", "PUT", "OPTIONS", "DELETE"]) {
    const response = await serve({}, method, assets)
    assert.equal(response.status, 405)
    assert.equal(response.headers.get("Allow"), "GET, HEAD")
  }
  for (const validator of [movieETag, `W/${movieETag}`, `"other", ${movieETag}`, "*"]) {
    const response = await serve({ "If-None-Match": validator }, "GET", assets)
    assert.equal(response.status, 304)
    assert.equal(response.body, null)
  }
  assert.equal((await serve({}, "GET", assets, "/unrelated")).status, 404)
  assert.equal(assets.calls.length, 0)
})

test("If-Range matches only the strong movie ETag; mismatches safely return full 200", async () => {
  for (const validator of [movieETag, `W/${movieETag}`, '"old"', "Wed, 21 Oct 2015 07:28:00 GMT"]) {
    const response = await serve({ Range: "bytes=3-7", "If-Range": validator })
    assert.equal(response.status, validator === movieETag ? 206 : 200)
    assert.equal(hash(Buffer.from(await response.arrayBuffer())), validator === movieETag ? hash(original.subarray(3, 8)) : movie.sha256)
  }
})

test("invalid initial assets fail 502 with no origin fallback", async () => {
  for (const fault of ["throw", "404", "bad-range", "bad-length", "ignored-range", "encoding"]) {
    const assets = binding(fault)
    const response = await serve({ Range: "bytes=2-5" }, "GET", assets)
    assert.equal(response.status, 502, fault)
    assert.equal(response.headers.get("Cache-Control"), "no-store")
    assert.equal(assets.calls.length, 1)
    assert.equal(response.body, null)
  }
})

test("truncated, oversized, failed and later-missing bodies error rather than silently altering bytes", async () => {
  for (const fault of ["truncated", "oversized", "body-error", "later"]) {
    const assets = binding(fault)
    const response = await serve({ Range: fault === "later" ? "bytes=8388607-8388609" : "bytes=2-5" }, "GET", assets)
    assert.equal(response.status, 206)
    await assert.rejects(response.arrayBuffer(), /Pages video asset|Asset/)
    assert.ok(assets.calls.length <= 2)
  }
})

test("consumer cancellation stops the reader and never fetches subsequent chunks", async () => {
  const assets = binding()
  const response = await serve({}, "GET", assets)
  const reader = response.body.getReader()
  const first = await reader.read()
  assert.equal(first.value.byteLength, 65536)
  assert.equal(assets.pulls, 1)
  await reader.cancel("player closed")
  await assets.cancelled
  assert.equal(assets.cancellations, 1)
  assert.equal(assets.calls.length, 1)
  assert.equal(assets.calls[0].signal.aborted, true)
  assert.equal(globalThis.fetch.mock.callCount(), 0)
})

test("cancellation aborts an in-flight next-chunk request without leaking its body", async () => {
  const assets = binding()
  const originalFetch = assets.fetch.bind(assets)
  let nextRequest
  let reachedNext
  const nextStarted = new Promise((resolve) => { reachedNext = resolve })
  assets.fetch = async (request) => {
    if (assets.calls.length === 1) {
      nextRequest = request
      reachedNext()
      await new Promise((resolve, reject) => {
        request.signal.addEventListener("abort", () => reject(new Error("Asset request aborted")), { once: true })
      })
    }
    return originalFetch(request)
  }
  const response = await serve({ Range: "bytes=8388607-8388609" }, "GET", assets)
  const reader = response.body.getReader()
  assert.equal((await reader.read()).value.byteLength, 1)
  const pending = reader.read()
  await nextStarted
  await reader.cancel()
  assert.equal(nextRequest.signal.aborted, true)
  assert.equal((await pending).done, true)
})

test("an ASSETS full-chunk 200 supports an exact whole-chunk range", async () => {
  const response = await serve({ Range: "bytes=0-8388607" }, "GET", binding("ignored-range"))
  assert.equal(response.status, 206)
  assert.deepEqual(Buffer.from(await response.arrayBuffer()), original.subarray(0, 8388608))
})

test("real Pages ASSETS full 200 streams without length support exact full and seeking responses", async () => {
  for (const [range, start, end] of [
    [null, 0, movie.size - 1], ["bytes=2-5", 2, 5],
    ["bytes=8388607-8388609", 8388607, 8388609],
    ["bytes=-128", movie.size - 128, movie.size - 1],
    ["bytes=30999140-", 30999140, movie.size - 1],
  ]) {
    const assets = binding("pages-full")
    const response = await serve(range ? { Range: range } : {}, "GET", assets)
    assert.equal(response.status, range ? 206 : 200)
    assert.deepEqual(Buffer.from(await response.arrayBuffer()), original.subarray(start, end + 1))
    if (end < movie.chunkSize - 1) assert.equal(assets.cancellations, 1, "cancel unused suffix")
  }
})
