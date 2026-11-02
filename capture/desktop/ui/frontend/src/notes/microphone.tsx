import { useEffect, useState } from 'react';
import { Button } from '@apparent/ui/button';
import type { ErrorReporting } from '../bridge';
export function RecordingMicrophone() {
  const [state, setState] = useState<ErrorReporting>({ status: 'disconnected' });
  const [busy, setBusy] = useState(false);
  async function request(enabled?: boolean) {
    const bridge = window.apparent;
    if (!bridge || busy) return;
    setBusy(true);
    try { setState(await (enabled === undefined ? bridge.readMicrophoneEnabled() : bridge.setMicrophoneEnabled(enabled))); }
    catch { setState({ status: 'error', code: 'bridge_failed' }); }
    finally { setBusy(false); }
  }
  useEffect(() => { void request(); }, []);
  return <section aria-labelledby="recording-microphone-title" className="space-y-3 rounded-xl border border-border bg-surface p-6">
    <div className="flex items-center justify-between gap-4"><h2 id="recording-microphone-title" className="text-lg font-semibold">Recording microphone</h2>
      <Button variant="outline" disabled={busy} onClick={() => void request()}>Refresh microphone setting</Button></div>
    <label className="flex items-center gap-3 text-sm font-medium"><input type="checkbox" aria-label="Record microphone" disabled={busy || state.status !== 'connected'}
      checked={state.status === 'connected' && state.enabled} onChange={e => void request(e.target.checked)} />Record microphone</label>
    <p className="text-sm text-muted-foreground">Applies to your next screen capture and stays saved. Turn off for silent feedback; Shift-click to type a note. Active recordings keep their microphone state.</p>
    {state.status === 'connected' ? <p role="status" className="text-sm">Microphone {state.enabled ? 'on' : 'off'} for your next screen capture.</p> :
      <p role="status" className="text-sm">{state.status === 'error' ? 'Could not confirm the setting. Refresh before trying again.' : 'Connect to the running Apparent app to change this setting.'}</p>}
  </section>;
}
