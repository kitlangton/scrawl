import service from '../../../service/src/worker.ts'
import type { Env as ServiceEnv } from '../../../service/src/types.ts'

interface Env extends ServiceEnv { ASSETS: Fetcher }

// The legacy API used a root API URL but already trusted the configured site
// origins. Preserve that allowlist. Reuse one profile per runtime environment and
// replace it whenever any service input changes (especially bindings/secrets).
const legacyProfiles = new WeakMap<object, { inputs: unknown[]; env: ServiceEnv }>()
function legacyProfile(env: Env): ServiceEnv {
  const inputs = [env.DB, env.BUCKET, env.API_ORIGIN, env.SITE_ORIGIN, env.LEGACY_SITE_ORIGIN,
    env.GOOGLE_CLIENT_ID, env.GOOGLE_CLIENT_SECRET,
    env.TOKEN_ENCRYPTION_KEY, env.BETTER_AUTH_SECRET, env.OAUTH_DISCOVERY_URL,
    env.SHARING_ENABLED, env.POSTHOG_ENABLED, env.POSTHOG_PROJECT_TOKEN, env.POSTHOG_HOST,
    env.POSTHOG_ENVIRONMENT, env.POSTHOG_EXCLUDED_ACCOUNT_IDS]
  const cached = legacyProfiles.get(env)
  if (cached && inputs.every((value, index) => value === cached.inputs[index])) return cached.env
  const profile = { ...env, API_ORIGIN: "https://hosted.tryapparent.com" }
  legacyProfiles.set(env, { inputs, env: profile })
  return profile
}

const routeDocuments = new Set([
  "/connect", "/connect/", "/connect/device", "/connect/device/", "/connect/shares", "/connect/shares/",
])
const art = new Set([
  "/connect/assets/source-pond-v4.png", "/connect/assets/agent-pond-v4.png", "/connect/assets/agent-reading-v4.png",
])
const hostedRedirects = new Map([
  ["/hosted", "/connect"], ["/hosted/", "/connect/"],
  ["/hosted/device", "/connect/device"], ["/hosted/device/", "/connect/device/"],
  ["/hosted/shares", "/connect/shares"], ["/hosted/shares/", "/connect/shares/"],
  ["/hosted/source-pond-v4.png", "/connect/assets/source-pond-v4.png"],
  ["/hosted/agent-pond-v4.png", "/connect/assets/agent-pond-v4.png"],
  ["/hosted/agent-reading-v4.png", "/connect/assets/agent-reading-v4.png"],
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
    // Legacy recording documents have one canonical web address. Metadata and
    // file endpoints retain their original transport contract for old clients.
    if (url.origin === "https://hosted.tryapparent.com"
      && /^\/s\/[a-f0-9]{64}$/.test(path)
      && (request.method === "GET" || request.method === "HEAD")) {
      if (env.API_ORIGIN !== "https://app.tryapparent.com/connect") {
        return new Response(null, { status: 404, headers: notFoundHeaders })
      }
      return new Response(null, { status: 302, headers: {
        ...notFoundHeaders, Location: env.API_ORIGIN + path + url.search,
      } })
    }
    // Keep the old API contract on its exact historical host. The original
    // Request (including method, body, cookies, Origin and auth headers) goes
    // directly to the existing service handler under its legacy origin profile.
    if (url.origin === "https://hosted.tryapparent.com" && (
      path === "/v1" || path.startsWith("/v1/") || path === "/auth" || path.startsWith("/auth/")
      || path.startsWith("/s/") || ["/share-view.js", "/share-view.css", "/share-wordmark.svg", "/share-font.ttf"].includes(path)
    )) return service.fetch(request, legacyProfile(env), context)
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
    const canonicalPath = hostedRedirects.get(path)
    if (canonicalPath) {
      // SITE_ORIGIN is deployment controlled. Keep redirects pinned to the
      // accepted canonical profile; never derive a destination from Host or
      // returnTo input.
      let canonicalOrigin: string
      try {
        const configured = new URL(env.SITE_ORIGIN)
        if (configured.origin !== "https://app.tryapparent.com" || configured.pathname !== "/"
          || configured.search || configured.hash || configured.username || configured.password) {
          return new Response(null, { status: 404, headers: notFoundHeaders })
        }
        canonicalOrigin = configured.origin
      } catch {
        return new Response(null, { status: 404, headers: notFoundHeaders })
      }
      const query = request.url.indexOf("?")
      const location = canonicalOrigin + canonicalPath + (query < 0 ? "" : request.url.slice(query))
      return new Response(null, { status: 302, headers: {
        ...notFoundHeaders,
        "Cache-Control": "no-store",
        Location: location,
      } })
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
  async scheduled(controller: ScheduledController, env: Env): Promise<void> {
    return service.scheduled(controller, env)
  },
} satisfies ExportedHandler<Env>
