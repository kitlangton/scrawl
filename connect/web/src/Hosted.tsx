import { useEffect, useRef, useState } from "react"
import * as api from "./api"
import "./hosted.css"
import { SharedLinks } from "./SharedLinks"

export function isHostedPath(path: string) { return /^\/(?:connect|hosted)(?:\/(?:device|shares))?\/?$/.test(path) }
export function Hosted({ devicePage = false, sharesPage = false }: { devicePage?: boolean; sharesPage?: boolean }) {
  return sharesPage ? <SharedLinks /> : <Account devicePage={devicePage} />
}
function Account({ devicePage }: { devicePage: boolean }) {
  const [account, setAccount] = useState<api.Session | null>(null)
  const [request, setRequest] = useState<api.Device | null>(null)
  const [code, setCode] = useState("")
  const [error, setError] = useState("")
  const [sessionUnavailable, setSessionUnavailable] = useState(false)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [interested, setInterested] = useState(false)
  const [tab, setTab] = useState<"computer" | "agent">("computer")
  const [consent, setConsent] = useState(false)
  const [now, setNow] = useState(0)
  const [dialogKind, setDialogKind] = useState<"setup" | "read">("setup")
  const dialog = useRef<HTMLDialogElement>(null)
  const guard = useRef(false)
  const generation = useRef(0)
  async function load() {
    const current = ++generation.current
    let sessionLoaded = false
    setLoading(true); setError(""); setSessionUnavailable(false); setAccount(null); setRequest(null); setConsent(false)
    const value = new URLSearchParams(window.location.search).get("code") ?? ""
    setCode(devicePage ? value : "")
    try {
      const result = await api.session()
      if (current !== generation.current) return
      sessionLoaded = true
      setAccount(result)
      if (result.authenticated) {
        setInterested(result.waitlist.interestedAtFiveDollars)
        if (devicePage && result.beta.access && value) {
          const preview = await api.device(value)
          if (current === generation.current) { setNow(Date.now()); setRequest(preview); setTab(preview.scope === "upload" ? "computer" : "agent"); setConsent(false) }
        }
      }
    } catch (e) { if (current === generation.current) { setSessionUnavailable(!sessionLoaded); setError(message(e)) } }
    finally { if (current === generation.current) setLoading(false) }
  }
  useEffect(() => { void load(); return () => { generation.current++ } }, [])
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 1000); return () => window.clearInterval(timer) }, [])
  async function act(action: () => Promise<void>) {
    if (guard.current) return
    guard.current = true; setBusy(true); setError("")
    try { await action() } catch (e) { setError(message(e)) }
    finally { guard.current = false; setBusy(false) }
  }
  const authenticated = account?.authenticated ? account : null
  const expired = request ? request.status === "expired" || (request.status === "pending" && request.expiresAt ? Date.parse(request.expiresAt) <= now : false) : false
  const computer = tab === "computer"
  const pending = request?.status === "pending" && !expired
  // A bad deployment config must reach the existing error UI, not crash render.
  let setupOrigin = ""
  try { setupOrigin = api.apiOrigin() } catch { /* Session/sign-in report configuration errors. */ }
  const heading = loading ? "Checking your account…" : error ? "Not confirmed" : authenticated && !authenticated.beta.access ? (authenticated.beta.enrolled >= authenticated.beta.capacity ? "The beta is full" : "Beta access is limited") : devicePage ? !authenticated ? "Sign in to continue" : !code ? "Start from the CLI" : request?.status === "approved" ? "Approval granted" : request?.status === "rejected" ? "Request rejected" : expired ? "That code expired" : request ? (request.scope === "upload" ? "Automatic uploads" : "Read-only permission") : "Request unavailable" : computer ? "Upload new captures" : "Read your captures"
  function openDialog(kind: "setup" | "read") { setDialogKind(kind); dialog.current?.showModal() }
  function selectTab(next: "computer" | "agent") {
    if (devicePage) return
    setTab(next)
  }
  const approve = () => {
    if (!authenticated || !request || !pending || (request.scope === "upload" && !consent)) return
    void act(async () => {
      if (request.expiresAt && Date.parse(request.expiresAt) <= Date.now()) throw new Error("This code has expired. Request a new code.")
      await api.approveDevice(authenticated.csrfToken, code)
      const next = await api.device(code)
      if (next.status !== "approved" || next.scope !== request.scope) throw new Error("Approval could not be confirmed. Reload to check its status.")
      setRequest(next)
    })
  }
  return <div className="pond-page hosted-page">
    <main className="hosted-main">
      <header className="hosted-header"><a href="/" aria-label="Apparent home"><img src="/apparent-wordmark.svg" width="119" height="28" alt="apparent" /></a><span>Connect</span></header>
      <div className="hosted-tabs" role="tablist" aria-label="Connect permissions" onKeyDown={event => {
        if (devicePage || !["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return
        event.preventDefault()
        const next = event.key === "Home" ? "computer" : event.key === "End" ? "agent" : computer ? "agent" : "computer"
        selectTab(next); document.getElementById(`hosted-${next}-tab`)?.focus()
      }}>
        {(["computer", "agent"] as const).map(value => <button key={value} id={`hosted-${value}-tab`} role="tab" aria-selected={tab === value} aria-controls="hosted-scope" tabIndex={tab === value ? 0 : -1} disabled={devicePage && tab !== value} onClick={() => selectTab(value)}>{value === "computer" ? "This computer" : "Your agent"}</button>)}
      </div>
      <section id="hosted-scope" role="tabpanel" aria-labelledby={`hosted-${tab}-tab`} className="hosted-panel" aria-busy={loading || busy}>
        <h1 role={loading ? "status" : undefined}>{heading}</h1>
        <div className={`hosted-scene${computer ? "" : " hosted-agent-view"}`}>
          <img className="hosted-source" src={computer ? "/hosted/source-pond-v4.png" : "/hosted/agent-reading-v4.png"} alt={computer ? "A frog beside a local laptop, holding a capture bundle" : "A frog reading an open capture beside a laptop"} />
          <div className="hosted-crossing"><button className="hosted-gate" aria-label="View separate agent read permission details" onClick={() => openDialog("read")}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 10V6a5 5 0 0 1 10 0v4M4 10h16v12H4ZM12 14v4" /></svg></button></div>
          {computer && <img className="hosted-agent" src="/hosted/agent-pond-v4.png" alt="An agent terminal waiting on a separate pond" />}
        </div>
        {loading && <p>Nothing is approved while we check.</p>}
        {error && <div role="alert" className="hosted-error">
          <p>{sessionUnavailable ? "We couldn’t check whether you’re signed in." : error}</p>
          {sessionUnavailable ? <>
            <p>Signing in grants no upload or read permission.</p>
            <div className="hosted-session-actions">
              <SignIn devicePage={devicePage} code={code} />
              <button disabled={busy || loading} onClick={() => void load()}>Try again</button>
            </div>
          </> : <button disabled={busy || loading} onClick={() => void load()}>Reload account</button>}
        </div>}
        {!loading && !error && <>
          {authenticated && !authenticated.beta.access ? <>
            <p>{authenticated.beta.enrolled} of {authenticated.beta.capacity} places occupied. {authenticated.waitlist.joined ? "You're on the waitlist." : "Join the waitlist for access updates."}</p>
            <form onSubmit={event => { event.preventDefault(); void act(async () => { await api.joinWaitlist(authenticated.csrfToken, interested); const next = await api.session(); if (!next.authenticated || !next.waitlist.joined || next.waitlist.interestedAtFiveDollars !== interested) throw new Error("Your waitlist preference could not be confirmed. Reload to check its status."); setAccount(next) }) }}>
              <label className="hosted-check"><input type="checkbox" checked={interested} disabled={busy} onChange={event => setInterested(event.target.checked)} /><span>I'd be interested at $5/month <span className="hosted-quiet">(optional)</span></span></label>
              <p className="hosted-quiet">Interest only. No payment details, subscription, or charge.</p>
              <button className="hosted-primary" disabled={busy} type="submit">{busy ? "Saving…" : authenticated.waitlist.joined ? "Update interest" : "Join waitlist"}</button>
            </form>
          </> : devicePage ? !authenticated ? <><p>Signing in grants no upload or read permission.</p><SignIn devicePage code={code} /></> : !code ? <p>No device code was supplied. Open the approval link from your app or agent. Do not paste access tokens here.</p> : request && <>
            <p className="hosted-tagline">{request.scope === "upload" ? "After CLI setup, new captures upload; older ones stay local." : "All your current and future uploads. Read-only."}</p>
            <div className="hosted-permission">
              <p>{request.scope === "upload" ? "This computer can add, read and delete uploaded captures. Agent read approval is separate." : "Cannot upload or delete. Upload permission is independent. Approval does not mean a capture has been read."}</p>
              <p>Captures can include screen images, recording, voice, transcript and window context.</p>
              {request.status === "approved" ? <p role="status">Approval granted. The CLI must finish connecting. This page cannot check the computer's upload setting or credential.</p> : request.status === "rejected" ? <p role="status">Nothing was approved. Start a new request in the CLI.</p> : expired ? <p role="status">Nothing was approved. Request a new code in the CLI.</p> : <>
                <p className="hosted-quiet">Only approve a request you just started on a trusted app or agent. {request.expiresAt ? `Expires ${new Date(request.expiresAt).toLocaleString()}.` : "Codes expire ten minutes after they are requested."}</p>
                {request.scope === "upload" && <label className="hosted-check"><input type="checkbox" checked={consent} disabled={busy} onChange={event => setConsent(event.target.checked)} /><span>I agree to share new capture contents with my Connect account.</span></label>}
                <button className="hosted-primary" disabled={busy || (request.scope === "upload" && !consent)} onClick={approve}>{busy ? "Confirming…" : request.scope === "upload" ? "Allow uploads" : "Allow agent read access"}</button>
              </>}
            </div>
          </> : <>
            <p className="hosted-tagline">{computer ? "New recordings upload; older ones stay local." : "Current and future uploads. Read-only."}</p>
            <button className="hosted-primary hosted-main-action" onClick={() => openDialog("setup")}>{computer ? "Connect" : "Review access"}</button>
          </>}
        </>}
        <details className="hosted-details" key={tab}><summary>{computer ? "Upload details" : "Read details"}</summary>
          <p>Captures can include screen images, recording, voice, transcript and window context.</p>
          <p>{computer ? "This computer’s grant can add, read and delete uploaded captures. Agent read approval is separate." : "All current and future uploads. Cannot upload or delete. Upload permission is independent; approval does not mean a capture has been read."}</p>
          <p>Google sign-in alone grants no upload or read permission. A capture ID is not a public sharing link.</p>
          <Limits />
        </details>
      </section>
      {authenticated && <div className="hosted-identity"><p>{authenticated.user.email}</p>{authenticated.beta.access && <a className="hosted-manage-link" href="/connect/shares">Manage shared links</a>}<button disabled={busy || loading} onClick={() => void act(async () => { await api.logout(authenticated.csrfToken); const next = await api.session(); if (next.authenticated) throw new Error("Sign-out could not be confirmed. Reload and try again."); setAccount(next); setRequest(null) })}>Log out</button></div>}
      <noscript><p>Enable JavaScript to sign in and review permissions. No approval happens without an explicit confirmation.</p></noscript>
    </main>
    <dialog ref={dialog} className="hosted-dialog" aria-labelledby="hosted-dialog-title">
      <button className="hosted-dialog-close" aria-label="Close dialog" onClick={() => dialog.current?.close()}>×</button>
      <h2 id="hosted-dialog-title">{dialogKind === "read" ? "Separate read approval" : !authenticated ? "Sign in" : computer ? "Start on this computer" : "Start with your agent"}</h2>
      {dialogKind === "read" ? <><p>Connecting this computer does not grant an agent read access.</p><p>A separate read grant covers all current and future uploads, including screen images, recording, voice, transcript and window context. It cannot upload or delete. This page cannot check an agent's credential or whether it has read a capture.</p></> : !authenticated ? <><p>Signing in grants no upload or read permission.</p><SignIn devicePage={devicePage} code={code} /></> : <>
        <p>Run this in the {computer ? "recording computer’s" : "agent’s"} terminal. Open its approval link, then let the CLI finish connecting.</p>
        <p>Update Apparent to use <code>apparent connect</code>. Older apps can use <code>apparent cloud</code>; it remains a compatibility alias.</p>
        <pre><code>{`apparent connect login --server ${setupOrigin || "https://YOUR-HOSTED-API-ORIGIN"} --scope ${computer ? "upload --consent-upload" : "read --no-browser"}`}</code></pre>
        {!setupOrigin && <p>Replace <code>https://YOUR-HOSTED-API-ORIGIN</code> with your deployed API origin, not the approval website.</p>}
        <p>{computer ? "Explicit upload consent is required. Only new captures upload automatically; existing captures stay local. This grant can add, read and delete uploaded captures." : "Approve all current and future uploads for read-only access. This grant cannot upload or delete; upload permission is independent."}</p>
        {computer && <><p>Check the computer’s actual local state:</p><pre><code>{"apparent connect status\napparent connect status --account"}</code></pre></>}
        <p>The website cannot inspect local settings or credentials, launch Apparent, or finish the CLI login for you.</p>
      </>}
      <Limits />
    </dialog>
  </div>
}
function Limits() {
  return <details className="hosted-limits"><summary>Retention &amp; limits</summary><ul>
    <li>Seven days maximum. The 500 MB (500,000,000-byte) account cap evicts oldest captures first, possibly earlier. Keep a local copy.</li>
    <li>Any file over 90 MB (90,000,000 bytes) fails the whole upload. Complete bundles only.</li>
    <li>Free during beta, with at most 20 accounts.</li>
    <li>Updated apps renew each approved device connection automatically while its renewal remains valid. Revoked or expired renewal requires CLI approval again. Older apps need an update and one new approval; website sign-in does not renew device access.</li>
  </ul></details>
}
function message(error: unknown) { return error instanceof Error ? error.message : "The request could not be confirmed. Please reload and retry." }
function SignIn({ devicePage, code }: { devicePage: boolean; code: string }) {
  try { return <a className="hosted-primary" href={api.signInUrl(devicePage, code)}>Continue with Google</a> }
  catch { return <p role="alert">Connect sign-in is not configured correctly. Please try again later.</p> }
}
