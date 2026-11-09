import {createRoot} from 'react-dom/client';
import {useEffect, useState} from 'react';
import {Button} from '@apparent/ui/button';
import '../style.css';

type Command = 'record' | 'choose' | 'latest' | 'settings';
declare global {
  interface Window {
    apparentControls: {command(action: Command): Promise<void>; mounted(): Promise<{busy: boolean; status: string | null}>; onBusy(listener: (busy: boolean) => void): () => void; onStatus(listener: (status: string | null) => void): () => void};
  }
}

function Controls() {
  const [busy, setBusy] = useState(true);
  const [status, setStatus] = useState<string | null>(null);
  useEffect(() => {
    const unsubscribe = window.apparentControls.onBusy(setBusy);
    const unsubscribeStatus = window.apparentControls.onStatus(setStatus);
    void window.apparentControls.mounted().then(state => {setBusy(state.busy); setStatus(state.status);});
    return () => {unsubscribe(); unsubscribeStatus();};
  }, []);
  return <main className="mx-auto flex max-w-sm flex-col gap-4 p-6">
    <h1 className="text-xl font-semibold">Apparent</h1>
    <p>Record, open your latest capture, or change Settings.</p>
    {status && <p role="status">{status}</p>}
    <Button disabled={busy} onClick={() => void window.apparentControls.command('record')}>Record screen</Button>
    <Button disabled={busy} variant="outline" onClick={() => void window.apparentControls.command('choose')}>Choose what to record…</Button>
    <Button disabled={busy} variant="outline" onClick={() => void window.apparentControls.command('latest')}>Open latest capture</Button>
    <Button disabled={busy} variant="outline" onClick={() => void window.apparentControls.command('settings')}>Settings…</Button>
    {busy && <p role="status">Capture is busy. Use the recording controls or shortcut.</p>}
    <p className="text-sm">You can also use your recording shortcut. Open Apparent again to return here.</p>
  </main>;
}

createRoot(document.getElementById('root')!).render(<Controls/>);
