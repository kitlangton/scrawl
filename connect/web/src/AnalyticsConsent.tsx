import { useEffect, useRef, useState } from "react"
import * as api from "./api"
import { invalidateAnalyticsAccount, syncAnalyticsAccount } from "../../../shared/analytics/browser"

export function AnalyticsConsent({ csrf }: { csrf: string }) {
  const [choice, setChoice] = useState<api.AnalyticsChoice | null>(null)
  const [pending, setPending] = useState(false)
  const [desired, setDesired] = useState<boolean | null>(null)
  const [status, setStatus] = useState("")
  const [error, setError] = useState("")
  const generation = useRef(0)
  useEffect(() => {
    const current = ++generation.current
    void api.analyticsChoice().then(next => { if (current === generation.current) setChoice(next) })
      .catch(() => { if (current === generation.current) setError("Your analytics choice could not be loaded. No choice was changed.") })
    return () => { generation.current++ }
  }, [csrf])
  async function choose(consented: boolean) {
    const current = ++generation.current
    const linkEpoch = invalidateAnalyticsAccount()
    setPending(true); setDesired(consented); setError(""); setStatus("")
    try {
      const saved = await api.setAnalyticsChoice(csrf, consented, choice?.revision ?? null)
      const next = await api.analyticsChoice()
      if (current !== generation.current) return
      setChoice(next)
      if (saved.consented !== consented || next.consented !== consented || saved.revision !== next.revision) throw new Error("unconfirmed")
      setStatus(consented ? "Account analytics is on." : "Account analytics is off.")
      if (!consented) syncAnalyticsAccount(null, linkEpoch)
      else {
        const session = await api.session()
        if (current === generation.current) syncAnalyticsAccount(session.authenticated ? session.analyticsId : null, linkEpoch)
      }
    } catch {
      if (current === generation.current) { setChoice(null); setError("Your analytics choice could not be confirmed. Reload to check, or stop account analytics below.") }
    } finally { if (current === generation.current) { setPending(false); setDesired(null) } }
  }
  return <details className="hosted-details">
    <summary>Optional account analytics</summary>
    <p>Simon van Laak uses EU-hosted PostHog to measure account creation, device connections, completed uploads, creation of share links and storage quantities. Daily active Connect accounts counts accounts with a completed upload or a share link created that UTC day; it does not count local-only recordings. No recording contents, filenames, email, keys or capability links are sent.</p>
    <p>This is separate from website analytics, Google sign-in, upload consent and agent read access. Saying no does not disable Connect. Your choice stays until you change it or delete the account.</p>
    {choice && <label className="hosted-check"><input type="checkbox" checked={desired ?? choice.consented} disabled={pending || !choice.available} onChange={event => void choose(event.target.checked)} /><span>Allow account analytics</span></label>}
    {choice && !choice.available && <p className="hosted-quiet">Analytics delivery is currently unavailable. An existing choice is retained; you can still stop account analytics.</p>}
    {(pending || !choice || choice.consented) && <button type="button" onClick={() => void choose(false)}>Stop account analytics</button>}
    {pending && <p role="status">Confirming your choice…</p>}
    {status && <p role="status">{status}</p>}
    {error && <p role="alert">{error}</p>}
    <p>Processing is based on your consent. Stopping blocks new export admissions and clears pending events; already-authorized sends may finish. Withdrawal is not completed provider erasure. PostHog’s existing-plan retention defaults apply, not a promised physical deletion deadline.</p>
    <p>For access, correction, erasure, restriction, portability or consent questions, contact <a href="mailto:simon@tryapparent.com">simon@tryapparent.com</a>. You may complain to your data-protection authority.</p>
  </details>
}
