import { useEffect, useState } from 'react';
import availableFrog from '../../../../assets/settings/frog-jump.png';
import currentFrog from '../../../../assets/settings/frog-lounge.png';
import failedFrog from '../../../../assets/settings/frog-puzzled.png';
import { Button } from '@apparent/ui/button';

export interface AboutState {
  version: string;
  updates: boolean;
  linux: boolean;
  mainUpdates: boolean;
  updateChannel: string;
  updateCheck: { checking: boolean; result: 'available' | 'current' | 'failed' | null };
}
type Action = { action: string; key?: string; value?: string };
declare global {
  interface Window {
    apparentAbout: {
      read(): Promise<AboutState | undefined>;
      act(action: Action): Promise<void>;
      subscribe(callback: (state: AboutState) => void): () => void;
      link(link: 'website' | 'source'): Promise<void>;
    };
  }
}
export function About() {
  const [state, setState] = useState<AboutState>();
  const [error, setError] = useState('');
  useEffect(() => {
    let live = true;
    let received = false;
    const unsubscribe = window.apparentAbout.subscribe(value => { received = true; if (live) setState(value); });
    void window.apparentAbout.read().then(value => { if (live && !received && value) setState(value); })
      .catch(() => { if (live) setError('Unable to read update status. Reopen Settings to try again.'); });
    return () => { live = false; unsubscribe(); };
  }, []);
  async function act(action: Action) {
    setError('');
    try { await window.apparentAbout.act(action); }
    catch { setError('Unable to complete that action. Please try again.'); }
  }
  const result = state?.updateCheck.result;
  useEffect(() => {
    if (!result) return;
    const listener = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { event.preventDefault(); void act({ action: 'dismiss-update-check' }); }
      if (event.key === 'Enter' && !(event.target instanceof Element && event.target.closest('[role=dialog] button'))) {
        event.preventDefault();
        void act({ action: result === 'available' ? 'update-now' : 'dismiss-update-check' });
      }
    };
    window.addEventListener('keydown', listener);
    return () => window.removeEventListener('keydown', listener);
  }, [result]);
  return <section aria-label="About Apparent" className="space-y-4">
    <h2 className="text-xl font-semibold">Apparent</h2>
    {!state && <p role="status">Loading update status…</p>}
    {error && <p role="alert">{error}</p>}
    {state && <>
      <p>Version {state.version}</p>
      <Button variant="outline" className="h-11 text-lg" disabled={!state.updates || state.updateCheck.checking}
        onClick={() => void act({ action: 'check-for-updates' })}>
        {state.updateCheck.checking ? 'Checking…' : 'Check for Updates…'}
      </Button>
      {!state.updates && <p>{state.linux ? 'Update checks work when Apparent runs from an AppImage.' : 'Updates work in the installed Apparent.app.'}</p>}
      {state.linux && state.mainUpdates && <p><label>Channel <select value={state.updateChannel}
        onChange={event => void act({ action: 'set', key: 'updateChannel', value: event.target.value })}>
        <option value="stable">Stable releases</option><option value="main">Latest main</option>
      </select></label></p>}
      {result && <div className="space-y-4 border border-border p-4 text-lg" role="dialog" aria-modal="false" aria-label="Update result">
        <img src={result === 'available' ? availableFrog : result === 'current' ? currentFrog : failedFrog} alt="" width="160" />
        <p role="status">{result === 'available' ? 'Update available' : result === 'current' ? 'No update available' : 'Update check failed'}</p>
        {result === 'available' ? <>
          <Button variant="outline" className="mr-3 h-11 text-lg" onClick={() => void act({ action: 'update-later' })}>Update later</Button>
          <Button variant="outline" className="mr-3 h-11 text-lg" onClick={() => void act({ action: 'update-now' })}>Update now</Button>
        </> : <Button variant="outline" className="mr-3 h-11 text-lg" onClick={() => void act({ action: 'dismiss-update-check' })}>OK</Button>}
      </div>}
    </>}
    <p><Button variant="outline" className="mr-3 h-11 text-lg" onClick={() => void window.apparentAbout.link('source').catch(() => setError('Unable to open Source.'))}>Star on GitHub</Button></p>
    <p><Button variant="outline" className="mr-3 h-11 text-lg" onClick={() => void window.apparentAbout.link('website').catch(() => setError('Unable to open Website.'))}>Website</Button>
      <Button variant="outline" className="mr-3 h-11 text-lg" onClick={() => void window.apparentAbout.link('source').catch(() => setError('Unable to open Source.'))}>Source</Button>
      <Button variant="outline" className="mr-3 h-11 text-lg" onClick={() => void act({ action: 'tour' })}>Show Welcome Tour</Button></p>
  </section>;
}
