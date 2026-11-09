import { useEffect, useState } from 'react';
import { Button } from '@apparent/ui/button';
import type { View } from './protocol';
declare global { interface Window { hermesRecording?: { read(): Promise<View>; start(): Promise<View> } } }
export function HermesRecording() {
  const [view, setView] = useState<View>({ phase: 'confirm' });
  useEffect(() => { document.title = 'Record for Hermes'; }, []);
  useEffect(() => { const timer = setInterval(() => { void window.hermesRecording?.read().then(setView); }, 500); return () => clearInterval(timer); }, []);
  const captions = { confirm: 'Continue only if you started this from Hermes. Record your screen, voice and marks. Finish returns the recording to Hermes for the original conversation. Keep Hermes open.',
    starting: 'Starting Apparent. Complete any permission prompts in the app.', recording: 'Talk, point and draw. Finish or cancel using Apparent’s recording controls.',
    processing: 'Preparing your recording…', saved: 'Recording saved. Waiting for Hermes to accept it. Keep Hermes open.', cancelled: 'Recording cancelled. Nothing was sent.', error: view.message ?? 'Unable to record.' };
  return <main className="space-y-4 bg-background p-6 text-foreground"><h1 className="text-xl font-semibold">Record for Hermes</h1>
    <p role="status" className="text-sm text-muted-foreground">{captions[view.phase]}</p>
    {view.phase === 'confirm' && <Button onClick={() => { setView({ ...view, phase: 'starting' }); void window.hermesRecording?.start().then(setView); }}>Record and send when finished</Button>}
    {view.source && <p className="break-all text-xs text-muted-foreground">Saved recording: {view.source}</p>}
    {view.phase === 'error' && <p className="text-sm">Any recording already started remains in Apparent. Nothing is retried automatically.</p>}
  </main>;
}
