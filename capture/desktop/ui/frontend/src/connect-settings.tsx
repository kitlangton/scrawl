import { useEffect, useRef, useState } from 'react';
import { Button } from '@apparent/ui/button';
import type { ConnectAction, ConnectSettings, ConnectView } from './bridge';

export function ConnectSettingsCard() {
  const [result, setResult] = useState<ConnectSettings>({ status: 'disabled' });
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  const polling = useRef(false);
  async function request(action?: ConnectAction) {
    const current = ++generation.current;
    setBusy(true);
    try {
      const bridge = window.apparent;
      const handlers = bridge && {
        connect: bridge.connectSettings, allow: bridge.allowConnectUploads, cancel: bridge.cancelConnectSettings,
        check: bridge.checkConnectConnection, disable: bridge.disableConnectUploads,
        disconnect: bridge.disconnectConnectSettings, 'open-browser': bridge.openConnectBrowser,
      };
      const next = !bridge ? { status: 'disconnected' } as const : action
        ? await handlers![action]() : await bridge.readConnectSettings();
      if (current === generation.current) setResult(next);
    } catch {
      if (current === generation.current) setResult({ status: 'error' });
    } finally {
      if (current === generation.current) setBusy(false);
    }
  }
  useEffect(() => {
    void request();
    return () => { generation.current += 1; };
  }, []);
  useEffect(() => {
    if (result.status !== 'connected') return;
    // Local-only snapshots renew this window's controller lease and collect
    // background receipts. They never perform a health probe or authorize login.
    const timer = setInterval(() => {
      if (busy || polling.current) return;
      polling.current = true;
      void request().finally(() => { polling.current = false; });
    }, 1000);
    return () => clearInterval(timer);
  }, [result.status, busy]);

  if (result.status === 'disabled') return null;
  const view = result.status === 'connected' ? result.view : undefined;
  return <section aria-labelledby="connect-title" className="space-y-4 rounded-xl border border-border bg-white p-6">
    <div className="flex flex-wrap items-start justify-between gap-4">
      <div>
        <p className="text-xs font-medium text-primary">GENERAL / CONNECT · EXPERIMENTAL</p>
        <h2 id="connect-title" className="text-lg font-semibold">Apparent Connect</h2>
      </div>
      <Button variant="outline" disabled={busy} onClick={() => void request()}>Refresh Connect</Button>
    </div>
    <p className="text-sm leading-relaxed text-muted-foreground">
      Connecting this device requires explicit permission to upload new captures to your configured Connect server. It does not create public links or upload older captures.
    </p>
    <div role="status" aria-live="polite" className="space-y-2 rounded-lg bg-muted px-4 py-3 text-sm">
      {busy && <p>Checking local Connect settings…</p>}
      {view ? <ConnectDetails view={view} busy={busy} onAction={request} /> : result.status === 'disconnected'
        ? <p>Not connected to the running Rust app. Open Apparent and refresh; this window never starts it.</p>
        : result.status === 'error'
          ? <p>Connect state is unavailable. Check the running app and refresh. Private connection details were not returned.</p>
          : <p>Connect is unavailable.</p>}
    </div>
  </section>;
}

function ConnectDetails({ view, busy, onAction }: { view: ConnectView; busy: boolean; onAction: (action: ConnectAction) => void }) {
  return <>
    <strong>{view.status}</strong>
    <p>{view.detail}</p>
    {view.server && <p className="break-all text-muted-foreground">Server: {view.server}</p>}
    {view.userCode && <p className="font-mono text-lg font-semibold tracking-wider">{view.userCode}</p>}
    {view.verificationUri && <p className="break-all text-muted-foreground">Approve this device in your browser: {view.verificationUri}</p>}
    <p className="text-muted-foreground">{view.note}</p>
    <div className="flex flex-wrap gap-2 pt-2">
      {view.buttons.map(button => <Button key={button.command} variant={button.command === 'allow' ? 'default' : 'outline'}
        disabled={busy || !button.enabled} onClick={() => onAction(button.command)}>{button.title}</Button>)}
    </div>
  </>;
}
