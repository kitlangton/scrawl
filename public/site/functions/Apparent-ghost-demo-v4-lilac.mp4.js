import { movie, movieETag, chunkPath } from "../lib/pages-video-metadata.js"

// Only a single byte interval is served. Invalid/multipart ranges fail closed.
function interval(value) {
  const match = /^bytes=(\d*)-(\d*)$/.exec(value)
  if (!match || (!match[1] && !match[2])) return null
  const a = match[1] ? Number(match[1]) : null
  const b = match[2] ? Number(match[2]) : null
  if ((a !== null && !Number.isSafeInteger(a)) || (b !== null && !Number.isSafeInteger(b))) return null
  if (a === null) return b > 0 ? [Math.max(0, movie.size - b), movie.size - 1] : null
  if (a >= movie.size || (b !== null && b < a)) return null
  return [a, Math.min(b ?? movie.size - 1, movie.size - 1)]
}

export async function onRequest({ request, env }) {
  if (new URL(request.url).pathname !== movie.path) return new Response(null, { status: 404 })
  if (request.method !== "GET" && request.method !== "HEAD") {
    return new Response(null, { status: 405, headers: { Allow: "GET, HEAD" } })
  }
  const headers = new Headers({
    "Content-Type": "video/mp4",
    "Accept-Ranges": "bytes",
    ETag: movieETag,
    "Cache-Control": "public, max-age=3600, no-transform",
    "X-Content-Type-Options": "nosniff",
    "X-Apparent-Publisher": "cloudflare-pages",
  })
  const noneMatch = request.headers.get("If-None-Match")
  if (noneMatch?.split(",").some((tag) => tag.trim() === "*" || tag.trim().replace(/^W\//, "") === movieETag)) {
    return new Response(null, { status: 304, headers })
  }
  let start = 0
  let end = movie.size - 1
  let status = 200
  // Range applies to GET, not HEAD. Dates/weak validators cannot match this
  // immutable representation: If-Range accepts only its exact strong ETag.
  const range = request.method === "GET" && request.headers.get("Range")
  const ifRange = request.headers.get("If-Range")
  if (range && (!ifRange || ifRange === movieETag)) {
    const selected = interval(range)
    if (!selected) {
      headers.set("Content-Range", `bytes */${movie.size}`)
      headers.set("Content-Length", "0")
      return new Response(null, { status: 416, headers })
    }
    ;[start, end] = selected
    status = 206
    headers.set("Content-Range", `bytes ${start}-${end}/${movie.size}`)
  }
  headers.set("Content-Length", String(end - start + 1))
  if (request.method === "HEAD") return new Response(null, { headers })

  const abort = new AbortController()
  let reader
  let remaining = 0
  let assetRemaining = 0
  let skip = 0
  let position = start
  let stopped = false
  async function stop(reason) {
    stopped = true
    abort.abort(reason)
    if (reader) {
      try { await reader.cancel(reason) } catch { /* Preserve the original failure. */ }
      reader = null
    }
  }
  async function openChunk() {
    const index = Math.floor(position / movie.chunkSize)
    const chunk = movie.chunks[index]
    const from = position - index * movie.chunkSize
    const to = Math.min(chunk.size - 1, end - index * movie.chunkSize)
    // Fresh, same-origin request: never forward cookies, conditionals, or URLs
    // from the caller. ASSETS is the sole source; no next()/origin fallback.
    const url = new URL(chunkPath(chunk), request.url)
    const response = await env.ASSETS.fetch(new Request(url, {
      headers: { Range: `bytes=${from}-${to}`, "Accept-Encoding": "identity" },
      signal: abort.signal,
      redirect: "manual",
    }))
    const count = to - from + 1
    const valid = response.status === 206
      ? response.headers.get("Content-Range") === `bytes ${from}-${to}/${chunk.size}`
      : response.status === 200
    const expected = response.status === 206 ? count : chunk.size
    const length = response.headers.get("Content-Length")
    if (stopped || !valid || (length !== null && length !== String(expected))
      || (response.headers.get("Content-Encoding") && response.headers.get("Content-Encoding") !== "identity") || !response.body) {
      await response.body?.cancel()
      throw new Error("Invalid Pages video asset response")
    }
    reader = response.body.getReader()
    remaining = count
    assetRemaining = expected
    // Pages ASSETS returns full 200 streams without Content-Length, even with
    // Range. Slice that stream; never buffer a chunk or fetch another origin.
    skip = response.status === 200 ? from : 0
  }
  // An initial missing/invalid asset can still return a clean HTTP failure.
  try { await openChunk() } catch {
    await stop()
    return new Response(null, { status: 502, headers: { "Cache-Control": "no-store" } })
  }
  const body = new ReadableStream({
    async pull(controller) {
      try {
        while (!stopped) {
          if (!reader) await openChunk()
          const { value, done } = await reader.read()
          if (stopped) return
          if (done) {
            reader.releaseLock()
            reader = null
            if (remaining !== 0 || assetRemaining !== 0) throw new Error("Truncated Pages video asset")
            if (position > end) {
              stopped = true
              controller.close()
              return
            }
            continue
          }
          if (value.byteLength > assetRemaining) throw new Error("Oversized Pages video asset")
          assetRemaining -= value.byteLength
          const offset = Math.min(skip, value.byteLength)
          skip -= offset
          const selected = value.subarray(offset, offset + remaining)
          if (!selected.byteLength) continue
          remaining -= selected.byteLength
          position += selected.byteLength
          controller.enqueue(selected)
          if (remaining === 0 && position > end && assetRemaining > 0) {
            await stop()
            controller.close()
          }
          return
        }
      } catch (error) {
        await stop(error)
        controller.error(error)
      }
    },
    cancel: stop,
  }, { highWaterMark: 0 })
  // workerd ignores a manually set Content-Length on an ordinary stream.
  // Its fixed-length stream preserves the wire length, allowing Cloudflare's
  // custom-domain cache to serve byte ranges instead of a chunked full 200.
  return new Response(body.pipeThrough(new FixedLengthStream(end - start + 1)), { status, headers })
}
