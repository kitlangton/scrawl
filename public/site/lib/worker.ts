// Every request comes through here first (run_worker_first in wrangler.jsonc):
// other hosts and plain HTTP redirect to https://app.tryapparent.com, and
// hashed build assets get a long cache. Release files and the appcast are
// returned exactly as uploaded, on any host, so shipped apps keep updating.
// `wrangler dev --var ANY_HOST:1` turns the redirect off for local testing.
type Env = { ASSETS: { fetch(request: Request): Promise<Response> }; ANY_HOST?: string }

const HOST = "app.tryapparent.com"

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url)
    if (url.pathname === "/appcast.xml" || url.pathname.startsWith("/releases/")) return env.ASSETS.fetch(request)

    if (!env.ANY_HOST && (url.hostname !== HOST || url.protocol !== "https:")) {
      url.protocol = "https:"
      url.hostname = HOST
      url.port = ""
      return Response.redirect(url.toString(), 301)
    }

    const response = await env.ASSETS.fetch(request)
    const headers = new Headers(response.headers)
    if (url.pathname.startsWith("/assets/")) headers.set("Cache-Control", "public, max-age=31536000, immutable")
    if (url.protocol === "https:") headers.set("Strict-Transport-Security", "max-age=31536000")
    headers.set("X-Content-Type-Options", "nosniff")
    headers.set("Referrer-Policy", "strict-origin-when-cross-origin")
    if (url.pathname.endsWith(".mp4")) return ranged(request, request.method === "HEAD" ? await env.ASSETS.fetch(new Request(request.url, { headers: request.headers })) : response, headers)
    return new Response(response.body, { status: response.status, statusText: response.statusText, headers })
  },
}

// Static assets ignore Range, but Safari and AVFoundation need byte ranges to play video.
// A HEAD is answered from the full GET so its headers match.
async function ranged(request: Request, response: Response, headers: Headers): Promise<Response> {
  headers.set("Accept-Ranges", "bytes")
  const match = /^bytes=(\d*)-(\d*)$/.exec(request.headers.get("Range") ?? "")
  if (response.status !== 200 || !match || (!match[1] && !match[2])) {
    return new Response(response.body, { status: response.status, statusText: response.statusText, headers })
  }
  const body = await response.arrayBuffer()
  const size = body.byteLength
  const start = match[1] ? Number(match[1]) : Math.max(0, size - Number(match[2]))
  const end = match[1] && match[2] ? Math.min(Number(match[2]), size - 1) : size - 1
  if (start > end) {
    headers.set("Content-Range", `bytes */${size}`)
    return new Response(null, { status: 416, headers })
  }
  headers.set("Content-Range", `bytes ${start}-${end}/${size}`)
  headers.set("Content-Length", String(end - start + 1))
  return new Response(request.method === "HEAD" ? null : body.slice(start, end + 1), { status: 206, headers })
}
