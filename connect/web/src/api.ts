// The only browser/backend boundary. Never persist session or device credentials.
export type Session = { authenticated: false } | {
  authenticated: true
  user: { email: string }
  csrfToken: string
  beta: { access: boolean; capacity: number; enrolled: number }
  waitlist: { joined: boolean; interestedAtFiveDollars: boolean }
}
export type Device = { scope: "upload" | "read"; expiresAt?: string; status: "pending" | "approved" | "rejected" | "expired" }

export function apiOrigin(config = import.meta.env.VITE_HOSTED_API_ORIGIN as string | undefined): string {
  if (!config) return ""
  const url = new URL(config)
  const local = ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)
  if ((url.protocol !== "https:" && !(local && url.protocol === "http:")) || url.username || url.password || url.pathname !== "/" || url.search || url.hash) throw new Error("Invalid Connect API origin configuration.")
  return url.origin
}
export function signInUrl(device: boolean, userCode: string): string {
  // Only fixed local destinations; never forward a URL supplied by the visitor.
  const returnTo = device ? `/connect/device?code=${encodeURIComponent(userCode)}` : "/connect"
  return `${apiOrigin()}/auth/google/start?returnTo=${encodeURIComponent(returnTo)}`
}
async function request(path: string, csrfToken?: string, body?: object, method?: string): Promise<unknown> {
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(), 12000)
  try {
    const response = await fetch(`${apiOrigin()}${path}`, {
      method: method ?? (csrfToken ? "POST" : "GET"), credentials: "include", cache: "no-store", redirect: "error", signal: controller.signal,
      headers: csrfToken ? { "Content-Type": "application/json", "X-CSRF-Token": csrfToken } : { Accept: "application/json" },
      body: csrfToken ? JSON.stringify(body ?? {}) : undefined,
    })
    if (!response.ok) {
      if (response.status === 401) throw new Error("Your session has ended. Sign in again, then retry.")
      if (response.status === 403) throw new Error("This request was not authorized. Reload the page before trying again.")
      if (response.status === 404 || response.status === 410) throw new Error("This device code was not found or has expired. Request a new code in your app or agent.")
      throw new Error("Connect is unavailable right now. Nothing was confirmed. Please retry.")
    }
    if (response.status === 204) return null
    if (!response.headers.get("content-type")?.includes("application/json")) throw new Error("Connect returned an unexpected response. Please retry later.")
    return await response.json()
  } catch (error) {
    if (error instanceof TypeError || (error instanceof DOMException && error.name === "AbortError")) throw new Error("Could not reach Connect. Check your connection and retry.")
    throw error
  } finally { clearTimeout(timeout) }
}
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object") throw new Error("Connect returned an invalid response.")
  return value as Record<string, unknown>
}
export async function session(): Promise<Session> {
  const data = record(await request("/v1/session"))
  if (data.authenticated === false) return { authenticated: false }
  const user = record(data.account), beta = record(data.slots), waitlist = record(user.waitlist)
  if (data.authenticated !== true || typeof user.email !== "string" || typeof data.csrfToken !== "string" || !data.csrfToken || !["active", "waitlist"].includes(user.beta as string) || beta.limit !== 20 || !Number.isInteger(beta.used) || (beta.used as number) < 0 || (beta.used as number) > 20 || typeof waitlist.interestedAtFiveDollars !== "boolean") throw new Error("Connect returned an invalid account response.")
  // Pick only needed fields. Extra server fields (especially credentials) never reach UI state.
  return { authenticated: true, user: { email: user.email }, csrfToken: data.csrfToken, beta: { access: user.beta === "active", capacity: 20, enrolled: beta.used as number }, waitlist: { joined: user.beta === "waitlist", interestedAtFiveDollars: waitlist.interestedAtFiveDollars } }
}
export async function device(userCode: string): Promise<Device> {
  const data = record(await request(`/v1/device?userCode=${encodeURIComponent(userCode)}`))
  if (!["upload", "read"].includes(data.scope as string) || !["pending", "approved", "rejected", "expired"].includes(data.status as string) || (data.expiresAt !== undefined && (typeof data.expiresAt !== "string" || !Number.isFinite(Date.parse(data.expiresAt))))) throw new Error("This device request has an unknown permission or invalid expiry. Nothing was approved.")
  return { scope: data.scope as Device["scope"], expiresAt: data.expiresAt as string | undefined, status: data.status as Device["status"] }
}
export const joinWaitlist = (csrf: string, interestedAtFiveDollars: boolean) => request("/v1/waitlist", csrf, { interestedAtFiveDollars })
export const approveDevice = (csrf: string, userCode: string) => request("/v1/device/approve", csrf, { userCode })
export const logout = (csrf: string) => request("/auth/logout", csrf)

export type Share = { id: string; captureId: string; title: string; status: string; createdAt: string; expiresAt: string; url?: string }
export async function shares(): Promise<Share[]> {
  // Return only a complete list. In particular, absence on page one is not
  // evidence that a later-page link was revoked.
  const rows: Share[] = [], cursors = new Set<string>(), ids = new Set<string>()
  let before: string | null = null
  do {
    const data = record(await request(`/v1/shares${before ? `?before=${encodeURIComponent(before)}` : ""}`))
    if (!Array.isArray(data.shares) || data.shares.length > 100 || !(data.nextCursor === null || (typeof data.nextCursor === "string" && /^\d+:[a-f0-9]{32}$/.test(data.nextCursor) && Number.isSafeInteger(Number(data.nextCursor.split(":")[0]))))) throw new Error("Shared links returned invalid pagination data. Reload to check the complete list.")
    for (const value of data.shares) {
      const row = share(value)
      if (ids.has(row.id)) throw new Error("Shared links changed while loading. Reload to check the complete list.")
      ids.add(row.id); rows.push(row)
    }
    before = data.nextCursor as string | null
    if (before !== null) {
      if (cursors.has(before)) throw new Error("Shared links returned a repeated page. Reload to check the complete list.")
      cursors.add(before)
    }
  } while (before !== null)
  return rows
}
function share(value: unknown): Share {
    const row = record(value)
    if (!["id", "captureId", "title", "status", "createdAt", "expiresAt"].every(key => typeof row[key] === "string") || !row.id || !["pending", "ready", "paused", "failed", "expired", "revoked", "deleted"].includes(row.status as string) || !Number.isFinite(Date.parse(row.createdAt as string)) || !Number.isFinite(Date.parse(row.expiresAt as string))) throw new Error("Shared links returned invalid data.")
    const result: Share = { id: row.id as string, captureId: row.captureId as string, title: row.title as string, status: row.status as string, createdAt: row.createdAt as string, expiresAt: row.expiresAt as string }
    if (typeof row.url === "string" && ["pending", "ready", "paused", "failed"].includes(result.status) && Date.parse(result.expiresAt) > Date.now()) {
      const url = new URL(row.url)
      if (url.protocol !== "https:" || url.username || url.password) throw new Error("Shared link returned an unsafe URL.")
      result.url = url.href
    }
    return result
}
export const stopSharing = (csrf: string, id: string) => request(`/v1/shares/${encodeURIComponent(id)}`, csrf, undefined, "DELETE")
