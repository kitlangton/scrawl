import service from '../../../service/src/worker.ts'
import type { Env as ServiceEnv } from '../../../service/src/types.ts'

interface Env extends ServiceEnv { ASSETS: Fetcher }

const routeDocuments = new Set([
  "/connect", "/connect/", "/connect/device", "/connect/device/", "/connect/shares", "/connect/shares/",
])
const art = new Set([
  "/connect/assets/source-pond-v4.png", "/connect/assets/agent-pond-v4.png", "/connect/assets/agent-reading-v4.png",
])
const notFoundHeaders = {
  "Cache-Control": "no-store",
  "Referrer-Policy": "no-referrer",
  "X-Content-Type-Options": "nosniff",
  "X-Robots-Tag": "noindex, nofollow, noarchive",
}

function routeAssetPath(path: string): string | undefined {
  if (routeDocuments.has(path)) return `${path.replace(/\/$/, "")}/index.html`.replace(/^\/\//, "/")
  if (art.has(path)) return path
  if (/^\/connect\/assets\/[A-Za-z0-9_-][A-Za-z0-9._-]*$/.test(path)
    && !/\.(?:map|json)$/i.test(path)
    && !/(?:^|\/)(?:node_modules|functions|backend)(?:\/|$)/i.test(path)) return path
  return undefined
}

export default {
  async fetch(request: Request, env: Env, context?: ExecutionContext): Promise<Response> {
    const url = new URL(request.url)
    const path = url.pathname
    // All API, OAuth, capability and recipient asset behavior is owned by the
    // existing service handler. Its API_ORIGIN is configured with /connect.
    if (path.startsWith('/connect/v1/') || path === '/connect/v1'
      || path.startsWith('/connect/auth/') || path === '/connect/auth'
      || path.startsWith('/connect/s/')
      || ['/connect/share-view.js', '/connect/share-view.css', '/connect/share-wordmark.svg', '/connect/share-font.ttf'].includes(path)) {
      return service.fetch(request, env, context)
    }
    if (request.method !== "GET" && request.method !== "HEAD") {
      return new Response(null, { status: 405, headers: { ...notFoundHeaders, Allow: "GET, HEAD" } })
    }
    // Encoded path bytes and backslashes are not part of the published route contract.
    if (path.includes("%") || path.includes("\\") || path.includes("//")) {
      return new Response(null, { status: 404, headers: notFoundHeaders })
    }
    const assetPath = routeAssetPath(path)
    if (!assetPath) return new Response(null, { status: 404, headers: notFoundHeaders })

    const assetUrl = new URL(request.url)
    assetUrl.pathname = assetPath
    assetUrl.search = ""
    const headers = new Headers()
    for (const name of ["accept", "accept-encoding", "if-none-match", "if-modified-since", "range", "if-range"]) {
      const value = request.headers.get(name)
      if (value !== null) headers.set(name, value)
    }
    const assetRequest = new Request(assetUrl, { method: request.method, headers })
    const response = await env.ASSETS.fetch(assetRequest)
    if (response.status === 404) return new Response(null, { status: 404, headers: notFoundHeaders })
    const outputHeaders = new Headers(response.headers)
    outputHeaders.set("X-Apparent-Publisher", "connect-worker")
    outputHeaders.set("X-Content-Type-Options", "nosniff")
    outputHeaders.set("Referrer-Policy", "no-referrer")
    outputHeaders.set("X-Robots-Tag", "noindex, nofollow, noarchive")
    if (assetPath.endsWith("/index.html")) outputHeaders.set("Cache-Control", "no-store")
    else outputHeaders.set("Cache-Control", "no-cache, no-transform")
    outputHeaders.delete("Set-Cookie")
    return new Response(request.method === "HEAD" ? null : response.body, { status: response.status, headers: outputHeaders })
  },
} satisfies ExportedHandler<Env>
