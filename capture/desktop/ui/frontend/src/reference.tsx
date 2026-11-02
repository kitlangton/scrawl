import { useEffect, useRef, useState } from 'react';
import {PublicShare} from './public-share';
import { Button } from '@apparent/ui/button';
import type { CaptureReference as Result } from './bridge';

const errors: Record<string, string> = {
  conflict: 'The selected capture changed. Refresh before copying again.',
  refresh_required: 'Refresh the selected capture before copying.',
  no_selection: 'No saved capture is selected. Select one in the current app, then refresh.',
  clipboard_failed: 'The clipboard could not be updated. Refresh before trying again.',
  outcome_unknown: 'Copy timed out. Its outcome is unknown. Check the clipboard and refresh; nothing is retried automatically.',
  admission_expired: 'Copy expired before it could run. Refresh before trying again.',
  app_busy: 'A reference request is already running. Refresh when it finishes.',
};

export function CaptureReference() {
  const [result, setResult] = useState<Result>({ status: 'disabled' });
  const [busy, setBusy] = useState(false);
  const order = useRef(0);
  async function request(operation: 'read' | 'copy') {
    const token = ++order.current;
    setBusy(true);
    try {
      const bridge = window.apparent;
      const next: Result = !bridge ? { status: 'disabled' } : await (
        operation === 'read' ? bridge.readCaptureReference() : bridge.copyCaptureReference());
      if (token === order.current) setResult(next);
    } catch {
      if (token === order.current) setResult({ status: 'error', code: 'bridge_failed' });
    } finally { if (token === order.current) setBusy(false); }
  }
  useEffect(() => { void request('read'); return () => { order.current += 1; }; }, []);
  if (result.status === 'disabled') return null;
  const id = result.status === 'connected' ? result.id : null;
  return <section aria-labelledby="reference-title" className="space-y-4 rounded-xl border border-border bg-white p-6">
    <div className="flex flex-wrap items-center justify-between gap-4">
      <h2 id="reference-title" className="text-lg font-semibold">Selected capture</h2>
      <Button variant="outline" disabled={busy} onClick={() => void request('read')}>Refresh capture reference</Button>
    </div>
    <p className="text-sm text-muted-foreground">The saved capture selected in the current Rust app. Public sharing applies only to this selected capture after confirmation.</p>
    <div role="status" aria-live="polite" className="space-y-2 text-sm">
      {busy && <p>Waiting for the selected capture…</p>}
      {!busy && id && <p className="break-all font-medium">{id}</p>}
      {result.status === 'connected' && !id && <p>No saved capture is selected. Select one in the current app, then refresh.</p>}
      {!busy && result.status === 'connected' && result.copied && <p>Capture ID copied</p>}
      {result.status === 'cancelled' && <p>The read was cancelled. Refresh capture reference before copying.</p>}
      {result.status === 'disconnected' && <p>Open the current Apparent app and select a saved capture, then refresh. This window never starts it.</p>}
      {result.status === 'incompatible' && <p>This Rust backend cannot provide a selected capture reference. Update it before copying.</p>}
      {result.status === 'error' && <p>{errors[result.code] ?? 'Unable to read the capture reference. Refresh when the Rust app is responsive.'}</p>}
    </div>
    {id && <PublicShare capture={id}/>}
    <Button disabled={busy || !id} onClick={() => void request('copy')}>Copy capture ID</Button>
  </section>;
}
