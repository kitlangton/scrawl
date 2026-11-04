import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { Button } from '@apparent/ui/button';
import { BuildIdentity } from '@apparent/ui/build-identity';
import './notes.css';

type Note = { pin: number; t: number; x: number; y: number; text: string };
type Snapshot = { mode: 'badge' | 'editor' | 'list'; current: number | null;
  notes: Note[]; drafts: Note[]; error: string | null;
  flight?: { id: number; start: [number, number]; delta: [number, number] } | null };
type Command = { action: 'save' | 'draft'; pin: number; text: string } |
  { action: 'edit' | 'delete'; pin: number } | { action: 'dismiss' | 'list' };
declare global {
  interface Window { captureNotes?: {
    onSnapshot: (callback: (state: Snapshot) => void) => () => void;
    command: (command: Command) => Promise<void>;
  } }
}
export function CaptureNotes() {
  const [state, setState] = useState<Snapshot>();
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const field = useRef<HTMLTextAreaElement>(null);
  const current = useRef<number | null>(null);
  const badge = location.hash === '#notes-badge';
  useEffect(() => window.captureNotes?.onSnapshot(next => {
    setState(next); setError(next.error ?? undefined);
    if (next.error || next.mode !== 'editor') setBusy(false);
    if (next.current !== current.current || next.mode !== 'editor') {
      current.current = next.current;
      setText(next.drafts.find(n => n.pin === next.current)?.text ?? '');
    }
  }), []);
  useEffect(() => {
    if (badge || state?.mode !== 'editor') return;
    const focus = () => field.current?.focus({ preventScroll: true });
    focus();
    // Native focus can arrive after the first state paint on XWayland/macOS.
    window.addEventListener('focus', focus);
    return () => window.removeEventListener('focus', focus);
  }, [badge, state?.current, state?.mode]);
  async function command(message: Command) {
    if (!window.captureNotes) { setError('Notes are not connected to the recording.'); return; }
    try { await window.captureNotes.command(message); }
    catch { setBusy(false); setError('Could not save this note. Your text is kept; try again.'); }
  }
  function save() {
    if (!state?.current || busy) return;
    setBusy(true); void command({ action: 'save', pin: state.current, text });
  }
  async function dismiss() {
    if (state?.mode === 'editor' && state.current) await command({ action: 'draft', pin: state.current, text });
    await command({ action: 'dismiss' });
  }
  if (!badge && state?.flight) {
    const { id, start, delta } = state.flight;
    return <main className="note-flight" key={id} aria-hidden="true" style={{
      left: start[0], top: start[1], '--flight-x': `${delta[0]}px`, '--flight-y': `${delta[1]}px`,
      '--flight-mid-x': `${delta[0] / 2}px`, '--flight-mid-y': `${delta[1] / 2 - 24}px`,
    } as CSSProperties}><svg viewBox="0 0 24 24" fill="none"><path d="M5 3h10l4 4v14H5V3Z" fill="currentColor"/><path d="M14 3v5h5M8 12h8M8 16h6" stroke="var(--color-primary-foreground)" strokeWidth="1.4" strokeLinecap="round"/></svg></main>;
  }
  if (!state) return <><div className="notes-unavailable" role="status">Connecting notes…</div><BuildIdentity /></>;
  return <main className={`capture-notes${badge ? ' notes-badge' : ''}`} onClick={e => {
    if (!badge && e.target === e.currentTarget) void dismiss();
  }} onKeyDown={e => {
    if (e.key === 'Escape' && !e.nativeEvent.isComposing) { e.preventDefault(); void dismiss(); }
  }}>
    {badge ? <><Button variant="ghost" className={`note-count${state.flight ? ' note-receiving' : ''}`} aria-label={`${state.notes.length} saved notes. Open notes`} onClick={() => void command({ action: 'list' })}>
      <svg viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="M3 2.5h10v11H3zM5 6h6M5 9h4" stroke="currentColor" strokeLinecap="round" /></svg>
      <span>Notes</span>{state.notes.length > 0 && <span className="note-total">{state.notes.length}</span>}
    </Button><div className="notes-build-slot"><BuildIdentity /></div></> : state.mode === 'list' ? <><section className="note-list" aria-label="Capture notes">
      <header><strong>Notes</strong><Button variant="ghost" size="sm" aria-label="Close notes" onClick={() => void dismiss()}>Close</Button></header>
      {state.drafts.filter(n => n.text.trim()).length === 0 ? <p>No notes yet. Shift-click to type.</p> :
        state.drafts.filter(n => n.text.trim()).map(n => <article key={n.pin}>
          <Button variant="ghost" className="note-row" onClick={() => void command({ action: 'edit', pin: n.pin })}>
            <span className="note-number">{n.pin}</span><span>{n.text}</span>
            {state.notes.find(saved => saved.pin === n.pin)?.text !== n.text && <small>Draft</small>}
          </Button>
          <Button variant="ghost" size="sm" aria-label={`Delete note ${n.pin}`} onClick={() => void command({ action: 'delete', pin: n.pin })}>Delete</Button>
        </article>)}
      {error && <p role="alert">{error}</p>}
    </section><div className="notes-build-slot"><BuildIdentity /></div></> : <><form className="note-popup" aria-label={`Note at pin ${state.current}`} onSubmit={e => { e.preventDefault(); save(); }}>
      <textarea ref={field} aria-label="Note text" placeholder="What should change?" value={text} rows={3} maxLength={8192}
        onChange={e => { const value = e.target.value; setText(value); if (state.current) void command({ action: 'draft', pin: state.current, text: value }); }}
        onKeyDown={e => {
          if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); save(); }
        }} />
      {error && <p role="alert">{error}</p>}
      <footer><Button type="submit" size="sm" disabled={busy}>{busy ? 'Saving…' : 'Attach'}<span aria-hidden="true">↵</span></Button></footer>
    </form><div className="notes-build-slot"><BuildIdentity /></div></>}
  </main>;
}
