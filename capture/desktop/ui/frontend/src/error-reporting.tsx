import { useEffect, useRef, useState } from 'react';
import { Button } from '@apparent/ui/button';
import type { ErrorReporting as Snapshot } from './bridge';

const messages: Record<string, string> = {
  timeout: 'The read timed out. Refresh to check the saved setting.',
  outcome_unknown: 'The save timed out; its outcome is unknown. Refresh before trying again.',
  admission_expired: 'The request expired before Rust applied it. Refresh and try again.',
  config_read_failed: 'Rust could not safely read the settings file.',
  config_write_failed: 'Rust could not safely save the setting.',
  app_busy: 'Rust is handling another settings request. Refresh shortly.',
  invalid_response: 'Rust returned an unsupported settings response.',
  bridge_failed: 'The local settings bridge failed. Refresh to check the saved setting.',
};

export function ErrorReporting() {
  const [result, setResult] = useState<Snapshot>({ status: 'disconnected' });
  const [busy, setBusy] = useState(false);
  const order = useRef(0);
  async function request(enabled?: boolean) {
    const token = ++order.current;
    setBusy(true);
    try {
      const bridge = window.apparent;
      let next = !bridge ? { status: 'disconnected' as const } : enabled === undefined
        ? await bridge.readErrorReporting() : await bridge.setErrorReporting(enabled);
      // The single Rust settings worker may reject simultaneous mount reads.
      // Allow a bounded startup burst (2.3 seconds total backoff). Only retry
      // reads explicitly rejected as busy; never repeat a mutation or timeout.
      for (const delay of [100, 200, 400, 800, 800]) {
        if (!bridge || enabled !== undefined || next.status !== 'error' || next.code !== 'app_busy') break;
        await new Promise(resolve => setTimeout(resolve, delay));
        if (token !== order.current) return;
        next = await bridge.readErrorReporting();
      }
      if (token === order.current) setResult(next);
    } catch {
      if (token === order.current) setResult({ status: 'error', code: 'bridge_failed' });
    } finally { if (token === order.current) setBusy(false); }
  }
  useEffect(() => { void request(); return () => { order.current += 1; }; }, []);
  return <section aria-labelledby="reporting-title" className="space-y-4 rounded-xl border border-border bg-surface p-6">
    <div className="flex flex-wrap items-center justify-between gap-4">
      <div><p className="text-xs font-medium text-primary">GENERAL / TECHNICAL ERRORS</p>
        <h2 id="reporting-title" className="text-lg font-semibold">Send technical error reports</h2></div>
      <Button variant="outline" disabled={busy} onClick={() => void request()}>Refresh reporting setting</Button>
    </div>
    <div role="status" aria-live="polite" className="space-y-2 text-sm">
      {busy && <p>Checking Rust settings…</p>}
      {result.status === 'connected' && <p>Technical error reports are <strong>{result.enabled ? 'on' : 'off'}</strong>. Rust saves this choice immediately.</p>}
      {result.status === 'disconnected' && <p>Not connected to Rust. Open the current Apparent app and refresh.</p>}
      {result.status === 'incompatible' && <p>This Rust app does not support the technical error reporting setting.</p>}
      {result.status === 'error' && <p>{messages[result.code] ?? messages.invalid_response}</p>}
    </div>
    <label className="flex items-center gap-3 text-sm font-medium">
      <input aria-label="Send technical error reports" type="checkbox" checked={result.status === 'connected' && result.enabled}
        disabled={busy || result.status !== 'connected'} onChange={(event) => void request(event.target.checked)} />
      Send technical error reports
    </label>
    <p className="text-sm text-muted-foreground">On by default. Reports contain only reviewed technical failure details; they never include recordings, logs or raw error text. Requires a packaged PostHog token to send.</p>
  </section>;
}
