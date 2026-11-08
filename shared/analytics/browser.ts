// Explicit events only. Shared by Public and Connect; no SDK or capability routes.
export interface AnalyticsConfig { enabled: boolean; host: string; token: string; environment: string }
const KEY = "apparent.product-analytics.v1"
const NOTICE = "product-analytics-v1"
const MAX_AGE = 180 * 86400000
const ROUTES = new Set(["/", "/connect", "/hosted"])
const SOURCES = new Set(["google", "bing", "duckduckgo", "github", "linkedin", "newsletter", "reddit", "x"])
const MEDIUMS = new Set(["organic", "cpc", "email", "social", "referral"])
let epoch = 0
let knownAccount: {known: boolean; id: string | null} = {known: false, id: null}
const listeners = new Set<(state: typeof knownAccount) => void>()
let accountReader: (() => Promise<string | null>) | null = null
export function setAnalyticsAccountReader(reader: typeof accountReader) { accountReader = reader }
export function invalidateAnalyticsAccount() {
  epoch++; knownAccount = {known: false, id: null}
  for (const listener of listeners) listener(knownAccount)
  return epoch
}
export function syncAnalyticsAccount(id: string | null, expected: number) {
  if (expected !== epoch) return
  knownAccount = {known: true, id: typeof id === "string" && /^account:[a-f0-9]{64}$/.test(id) ? id : null}
  for (const listener of listeners) listener(knownAccount)
}
export function startAnalytics(config: AnalyticsConfig) {
  if (typeof window === "undefined" || typeof globalThis.crypto?.randomUUID !== "function" || config.enabled !== true || config.host !== "https://eu.i.posthog.com" || !/^phc_[a-zA-Z0-9_]+$/.test(config.token) || !["production", "synthetic", "development"].includes(config.environment)) return
  const route = location.pathname.replace(/\/$/, "") || "/"
  const suppressed = () => (navigator as Navigator & {globalPrivacyControl?: boolean}).globalPrivacyControl === true || navigator.doNotTrack === "1"
  if (!ROUTES.has(route) || suppressed() || document.querySelector('aside[aria-label="Analytics preferences"]')) return
  let choice: "allow" | "deny" | null = null, anonymous: string | null = null, account: string | null = null, identified: string | null = null, linkedAccount: string | null = null, lastPage: string | null = null, consentAt: number | null = null
  let sessionKnown = route === "/" || knownAccount.known
  let decision = 0
  let storageAvailable = true, raw: string | null = null
  try { raw = localStorage.getItem(KEY) } catch { storageAvailable = false }
  try {
    const stored = JSON.parse(raw || "null")
    if (stored?.noticeVersion === NOTICE && ["allow", "deny"].includes(stored.choice) && Number.isFinite(stored.at) && stored.at <= Date.now() && Date.now() - stored.at < MAX_AGE) {
      choice = stored.choice; consentAt = stored.at
      if (/^anon:[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/.test(stored.id || "")) anonymous = stored.id
      if (/^account:[a-f0-9]{64}$/.test(stored.account || "")) linkedAccount = stored.account
    }
  } catch { /* A malformed preference is not permission; a fresh choice may replace it. */ }
  function save() {
    if (!storageAvailable) { choice = "deny"; anonymous = identified = linkedAccount = null; return false }
    try { localStorage.setItem(KEY, JSON.stringify({choice, at: consentAt, noticeVersion: NOTICE, ...(choice === "allow" ? {id: anonymous, account: linkedAccount} : {})})); return true }
    catch {
      storageAvailable = false; choice = "deny"; anonymous = identified = linkedAccount = null
      try { if (JSON.parse(localStorage.getItem(KEY) || "null")?.choice === "allow") localStorage.removeItem(KEY) } catch { /* Read-only storage may also refuse removal. */ }
      return false
    }
  }
  // Do not reuse a saved grant if a new refusal could not be persisted now.
  if (choice === "allow") save()
  function permitted() {
    if (!storageAvailable || choice !== "allow") return false
    const now = Date.now()
    if (suppressed() || consentAt === null || !Number.isFinite(consentAt) || consentAt > now || now - consentAt >= MAX_AGE) {
      choice = suppressed() ? "deny" : null; decision++
      anonymous = identified = linkedAccount = lastPage = null; consentAt = null; render()
      return false
    }
    return true
  }
  async function send(event: string, properties: Record<string, string | boolean | null> = {}, distinct = identified || anonymous) {
    if (!permitted() || !distinct) return
    const occurredAt = new Date().toISOString()
    const contextEpoch = epoch, choiceEpoch = decision
    if (distinct.startsWith("account:")) {
      let current: string | null
      try { current = accountReader ? await accountReader() : null } catch { return }
      if (!permitted() || contextEpoch !== epoch || choiceEpoch !== decision) return
      if (current !== distinct) {
        // An out-of-band withdrawal/account change cannot retain the old link.
        invalidateAnalyticsAccount()
        return
      }
    }
    const body = JSON.stringify({api_key: config.token, event, uuid: crypto.randomUUID(), timestamp: occurredAt, properties: {
      distinct_id: distinct, environment: config.environment, actor: "browser", $geoip_disable: true, $ip: null, $process_person_profile: !!identified, ...properties,
    }})
    fetch(config.host + "/capture/", {method: "POST", body, headers: {"Content-Type": "text/plain"}, credentials: "omit", referrerPolicy: "no-referrer", redirect: "error", keepalive: true}).catch(() => {})
  }
  function identify() {
    if (!permitted() || !account || account === identified) return
    if (!anonymous) anonymous = "anon:" + crypto.randomUUID()
    if (linkedAccount && linkedAccount !== account) anonymous = "anon:" + crypto.randomUUID()
    linkedAccount = account
    if (!save()) return
    identified = account
    send("$identify", {$anon_distinct_id: anonymous, $process_person_profile: true}, account)
  }
  function pageview() {
    if (lastPage === route || !sessionKnown || !permitted()) return
    if (!anonymous) { anonymous = "anon:" + crypto.randomUUID(); if (!save()) return }
    identify()
    const params = new URLSearchParams(location.search)
    let source = "direct"
    try { const domain = new URL(document.referrer).hostname.replace(/^www\./, ""); if (domain !== location.hostname) source = domain.endsWith(".google.com") || domain === "google.com" ? "google" : [...SOURCES].find(s => domain === s + ".com" || domain === s + ".org") || "other" } catch { /* No referrer. */ }
    if (params.has("utm_source")) source = SOURCES.has(params.get("utm_source") || "") ? params.get("utm_source")! : "other"
    const medium = MEDIUMS.has(params.get("utm_medium") || "") ? params.get("utm_medium")! : "unspecified"
    send("$pageview", {page: route, acquisition_source: source, acquisition_medium: medium}); lastPage = route
  }
  const host = document.createElement("aside")
  host.setAttribute("aria-label", "Analytics preferences")
  host.style.cssText = "position:fixed;left:16px;bottom:16px;z-index:1000;max-width:calc(100vw - 32px)"
  const shadow = host.attachShadow({mode: "open"})
  shadow.innerHTML = `<style>:host{font:14px/1.5 system-ui,sans-serif;color:var(--apparent-foreground)}section{box-sizing:border-box;width:min(420px,calc(100vw - 32px));max-height:calc(100dvh - 32px);overflow:auto;padding:20px;border:1px solid var(--apparent-border);border-radius:12px;background:var(--apparent-surface)}h2{font-size:16px;margin:0 0 8px}p{margin:0 0 12px}nav{display:flex;gap:8px;flex-wrap:wrap;position:sticky;bottom:0;padding-block:8px;background:var(--apparent-surface)}button{font:inherit;min-height:40px;padding:8px 14px;background:var(--apparent-secondary);color:var(--apparent-secondary-foreground);border:1px solid var(--apparent-border);border-radius:6px;cursor:pointer}button:focus-visible,a:focus-visible{outline:3px solid var(--apparent-ring);outline-offset:3px}a{color:inherit;text-decoration:underline}.settings{min-height:32px;padding:4px 8px;font-size:12px;background:var(--apparent-surface)}</style><div></div>`
  const panel = shadow.querySelector("div")!
  let previousFocus: Element | null = null
  function render(open = choice === null || !storageAvailable) {
    if (!open) {
      panel.innerHTML = '<button class="settings" type="button">Analytics settings</button>'
      panel.querySelector("button")!.onclick = () => { previousFocus = shadow.activeElement ?? document.activeElement; render(true); panel.querySelector("button")!.focus() }
      return
    }
    panel.innerHTML = '<section role="region" aria-label="Optional website analytics"><h2>Optional website analytics</h2><p>Simon van Laak uses EU-hosted PostHog to count visits and download clicks. With your separate Connect analytics permission, visits can be linked to your account. No recordings, filenames, email or share links are sent. PostHog receives network metadata such as your IP address and browser.</p><p>Your website choice lasts 180 days. Account permission is separate.</p><details><summary>Details, retention and your rights</summary><p>Processing is based on your consent. We send a random browser identifier, allowlisted page/acquisition/platform labels and event times; no URL queries or fragments. Account linking requires both permissions. GPC and Do Not Track suppress this browser stream.</p><p>Change your website choice through Analytics settings. Refusing or withdrawing stops new browser sends, but is not completed provider erasure. PostHog’s existing-plan retention defaults apply; no physical deletion deadline is promised.</p><p>For access, correction, erasure, restriction, portability or consent questions, contact the controller, Simon van Laak, at <a href="mailto:simon@tryapparent.com">simon@tryapparent.com</a>. You may complain to your data-protection authority.</p></details><nav aria-label="Analytics choice"><button type="button" data-choice="deny">No thanks</button><button type="button" data-choice="allow">Allow analytics</button></nav></section>'
    if (!storageAvailable) {
      const warning = document.createElement("p")
      warning.setAttribute("role", "status"); warning.textContent = "Your browser cannot save analytics preferences. Analytics stays off; you can still read this notice and refuse for this visit."
      panel.querySelector("nav")!.before(warning)
      const allow = panel.querySelector<HTMLButtonElement>('[data-choice="allow"]')!
      allow.disabled = true; allow.style.opacity = "0.6"; allow.style.cursor = "not-allowed"
    }
    panel.querySelectorAll<HTMLButtonElement>("[data-choice]").forEach(button => { button.onclick = () => {
      choice = button.dataset.choice === "allow" ? "allow" : "deny"; consentAt = Date.now(); decision++
      if (choice === "deny") { anonymous = identified = linkedAccount = lastPage = null }
      else anonymous ||= "anon:" + crypto.randomUUID()
      const wasStorageAvailable = storageAvailable, saved = save()
      render(!saved && (button.dataset.choice === "allow" || wasStorageAvailable)); pageview()
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus(); else panel.querySelector("button")!.focus()
    } })
  }
  function accountChanged(state: typeof knownAccount) {
    const next = state.id
    if ((account && account !== next || !next && linkedAccount) && choice === "allow") { anonymous = "anon:" + crypto.randomUUID(); identified = linkedAccount = null; save() }
    account = next; sessionKnown = route === "/" || state.known
    identify(); pageview()
  }
  listeners.add(accountChanged)
  document.body.append(host); render(); accountChanged(knownAccount); pageview()
  document.addEventListener("click", event => {
    const link = event.target instanceof Element ? event.target.closest("a") : null
    if (!link || choice !== "allow") return
    let url: URL; try { url = new URL(link.href) } catch { return }
    if (url.origin !== "https://github.com" || !/^\/tryapparent\/apparent\/releases(?:\/|$)/.test(url.pathname)) return
    const platform = ["mac", "linux"].includes(link.dataset.downloadPlatform || "") ? link.dataset.downloadPlatform! : /\.dmg$/.test(url.pathname) ? "mac" : /\.AppImage$/.test(url.pathname) ? "linux" : "unspecified"
    send("app_download_clicked", {platform, download_target: /\/download\//.test(url.pathname) ? "installer" : "release_listing"})
  }, true)
  window.addEventListener("storage", event => { if (event.key !== KEY && event.key !== null) return; choice = "deny"; decision++; anonymous = identified = linkedAccount = lastPage = null; render(false) })
}
