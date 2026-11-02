import { StrictMode, useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Button } from '@apparent/ui/button';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@apparent/ui/tabs';
import type { RuntimeState } from './bridge';
import { Storage } from './storage';
import './style.css';

// Visual fixtures only. These do not model or control real recording state.
const examples = [
  { id: 'idle', title: 'Ready when you are', detail: 'Point, talk and draw. The current app still owns all recording controls.' },
  { id: 'recording', title: 'Recording', detail: 'Your screen, voice and marks are being captured.' },
  { id: 'processing', title: 'Making the handoff', detail: 'The recording has stopped. Rust is preparing the bundle.' },
  { id: 'uploading', title: 'Uploading recording', detail: 'The link may be copied before the recording is ready to watch.' },
  { id: 'preparing', title: 'Link copied · preparing recording', detail: 'A copied link is not proof the video is ready. Recipients see a preparing state.' },
  { id: 'permission', title: 'Screen access unavailable', detail: 'Permission handling stays in the current app. This window cannot request access.' },
  { id: 'error', title: 'The handoff needs attention', detail: 'A synthetic error for reviewing layout and copy. No real capture has failed.' },
] as const;

function App() {
  const [runtime, setRuntime] = useState<RuntimeState>({ status: 'disconnected' });
  const [busy, setBusy] = useState(true);
  async function refresh() {
    setBusy(true);
    try {
      setRuntime(await window.apparent?.readRuntimeState() ?? { status: 'disconnected' });
    } catch {
      setRuntime({ status: 'error', message: 'The read-only bridge is unavailable. Try reopening this window.' });
    } finally { setBusy(false); }
  }
  useEffect(() => { void refresh(); }, []);
  return (
    <main className="mx-auto max-w-4xl space-y-8 px-8 py-10">
      <header className="space-y-3">
        <p className="text-sm font-medium text-primary">Apparent / UI foundation</p>
        <h1 className="text-3xl font-semibold tracking-tight">A small window into what’s next.</h1>
        <p className="max-w-2xl text-muted-foreground">An opt-in development surface. Recording and handoff stay in the current app; experimental Settings controls appear only when enabled.</p>
      </header>
      <section aria-labelledby="runtime-title" className="rounded-xl border border-border bg-white p-6">
        <div className="flex items-center justify-between gap-4">
          <div className="space-y-1">
            <h2 id="runtime-title" className="text-lg font-semibold">Live Rust state</h2>
            <p className="text-sm text-muted-foreground">One read on open or refresh. No background polling.</p>
          </div>
          <Button variant="outline" disabled={busy} onClick={() => void refresh()}>Refresh state</Button>
        </div>
        <div role="status" aria-live="polite" className="mt-5 rounded-lg bg-muted px-4 py-3 text-sm">
          {busy ? 'Reading local app…' : runtime.status === 'connected' ? <>
            <strong>Connected · {runtime.phase}</strong>
            <p className="mt-1 text-muted-foreground">Reported by the running Rust GUI. This window cannot change it.</p>
          </> : runtime.status === 'disconnected' ? <>
            <strong>Not connected</strong>
            <p className="mt-1 text-muted-foreground">{window.apparent ? 'Open the current Apparent app, then refresh. This window never starts it.' : 'Browser preview only. The Rust bridge is available in Electron.'}</p>
          </> : <><strong>Unable to read state</strong><p className="mt-1 text-muted-foreground">{runtime.message}</p></>}
        </div>
      </section>
      <Storage />
      <section aria-labelledby="preview-title" className="space-y-4">
        <div className="flex items-baseline justify-between gap-4">
          <h2 id="preview-title" className="text-lg font-semibold">State previews</h2>
          <span className="text-xs font-medium uppercase tracking-wider text-muted-foreground">Synthetic · not live</span>
        </div>
        <Tabs defaultValue="idle">
          <TabsList aria-label="Synthetic states" className="h-auto flex-wrap justify-start gap-1">
            {examples.map(({ id }) => <TabsTrigger key={id} value={id}>{id[0].toUpperCase() + id.slice(1)}</TabsTrigger>)}
          </TabsList>
          {examples.map(({ id, title, detail }) => <TabsContent key={id} value={id}>
            <div className="min-h-40 space-y-3 rounded-xl border border-border bg-white p-6">
              <p className="text-xs font-medium text-primary">SYNTHETIC EXAMPLE</p>
              <h3 className="text-xl font-semibold">{title}</h3>
              <p className="max-w-2xl text-sm leading-relaxed text-muted-foreground">{detail}</p>
            </div>
          </TabsContent>)}
        </Tabs>
      </section>
      <footer className="border-t border-border pt-4 text-xs leading-relaxed text-muted-foreground">
        React + shadcn/ui + Tailwind, hosted by Electron. No capture commands or permission prompts. Only the opt-in capture-location card can save a preference.
      </footer>
    </main>
  );
}

createRoot(document.getElementById('root')!).render(<StrictMode><App /></StrictMode>);
