import { useEffect, useRef, useState } from 'react';
import { Button } from '@apparent/ui/button';
import type { CaptureLocation, LocationSnapshot } from './bridge';
import { Retention } from './retention';
import { changeCaptureLocation, readCaptureLocation, resetCaptureLocation } from './storage/transport';

const errors: Record<string, string> = {
  conflict: 'The saved location changed while the picker was open. Refresh before choosing again.',
  outcome_unknown: 'The save outcome is unknown. Refresh to read the saved location; do not repeat it blindly.',
  admission_expired: 'The request expired before the save. Refresh and choose again.',
  config_write_failed: 'The preference could not be saved. The config may be unavailable or unsafe.',
  config_read_failed: 'The Rust app could not read the saved preference.',
  app_busy: 'Another location request is still running. Refresh when it finishes.',
  invalid_path: 'That folder was not accepted. Choose an absolute folder path.',
};

export function Storage({ settings = false }: { settings?: boolean }) {
  const [result, setResult] = useState<CaptureLocation>({ status: 'disabled' });
  const [snapshot, setSnapshot] = useState<LocationSnapshot>();
  const [busy, setBusy] = useState(true);
  const order = useRef(0);
  async function request(operation: 'read' | 'change' | 'reset') {
    const token = ++order.current;
    setBusy(true);
    try {
      const next: CaptureLocation = operation === 'read' ? await readCaptureLocation() :
        operation === 'change' ? await changeCaptureLocation() : await resetCaptureLocation();
      if (token !== order.current) return;
      if (next.status !== 'cancelled') {
        setResult(next);
        if (next.status === 'connected') setSnapshot(next.storage);
        else setSnapshot(undefined); // No stale path masquerading as an authoritative read.
      }
    } catch {
      if (token === order.current) {
        setSnapshot(undefined);
        setResult({ status: 'error', code: 'bridge_failed' });
      }
    }
    finally { if (token === order.current) setBusy(false); }
  }
  useEffect(() => { void request('read'); return () => { order.current += 1; }; }, []);
  if (result.status === 'disabled' && !settings) return null;
  return <section aria-labelledby="storage-title" className="space-y-4 rounded-xl border border-border bg-surface p-6">
    <div className="flex flex-wrap items-center justify-between gap-4">
      <div>{!settings && <p className="text-xs font-medium text-primary">EXPERIMENTAL / STORAGE</p>}
        <h2 id="storage-title" className="text-lg font-semibold">Capture location</h2></div>
      <Button variant="outline" disabled={busy} onClick={() => void request('read')}>Refresh location</Button>
    </div>
    <div role="status" aria-live="polite" className="space-y-2 text-sm">
      {busy && <p>Waiting for capture location…</p>}
      {snapshot && <>
        <p className="text-muted-foreground">Effective location</p>
        <p className="break-all font-medium">{snapshot.displayPath || '(empty environment override)'}</p>
        {snapshot.savedPath !== null && snapshot.savedPath !== snapshot.effectivePath &&
          <p className="break-all text-muted-foreground">Saved preference: {snapshot.savedPath}</p>}
        {snapshot.source === 'environment' && <p>The running Rust app uses APPARENT_CAPTURES_DIR. Change and Reset save a preference but cannot override that environment setting.</p>}
        {snapshot.source === 'fallback' && <p>The saved folder cannot currently be used. Rust is using its built-in location instead.</p>}
      </>}
      {result.status === 'disconnected' && <p>Not connected to Rust. Open the current Apparent app and refresh. This window never starts it.</p>}
      {result.status === 'incompatible' && <p>This Rust backend does not support capture location. Update it before using this card.</p>}
      {!busy && result.status === 'disabled' && <p>Storage is disabled in your configuration. Close this window and reopen Settings to use the fallback.</p>}
      {result.status === 'error' && <p>{errors[result.code] ?? 'Unable to read capture location. Refresh when the Rust app is responsive.'}</p>}
    </div>
    <p className="text-sm text-muted-foreground">New captures use this location. Existing captures stay where they are. Nothing is moved.</p>
    <div className="flex gap-3">
      <Button disabled={busy || result.status !== 'connected'} onClick={() => void request('change')}>Change…</Button>
      <Button variant="outline" disabled={busy || result.status !== 'connected' || !snapshot?.custom} onClick={() => void request('reset')}>Reset</Button>
      {settings && <Button variant="outline" disabled={busy || result.status !== 'connected'} onClick={() => {
        void window.apparent?.showCaptureFolder().catch(() => setResult({ status: 'error', code: 'bridge_failed' }));
      }}>Show folder</Button>}
    </div>
    {snapshot && !busy && <Retention key={snapshot.effectivePath} effectivePath={snapshot.effectivePath} settings={settings} />}
  </section>;
}
