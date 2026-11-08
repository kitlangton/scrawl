import {useEffect, useRef, useState} from 'react';
import {createRoot} from 'react-dom/client';
import {Button} from '@apparent/ui/button';
import icon from '../../../../../../shared/assets/brand/apparent-app-icon.svg';
import frog from '../../../../assets/onboarding/frog.png';
import '../style.css';
import './style.css';
type State = {needsReplacement?: boolean; version?: string; installed?: boolean; destination?: string; previousApp?: string};
type Phase = 'ready' | 'confirm' | 'preparing' | 'hopping' | 'installing';
declare global {interface Window {apparentInstaller: {action: (action: string, replace?: boolean) => Promise<State>}}}
function Installer() {
  const [state, setState] = useState<State>(), [error, setError] = useState(''), [phase, setPhase] = useState<Phase>('ready');
  const busy = phase === 'preparing' || phase === 'hopping' || phase === 'installing', hop = phase === 'hopping', confirm = phase === 'confirm';
  const replacement = useRef<HTMLButtonElement>(null);
  useEffect(() => {if (confirm) replacement.current?.focus();}, [confirm]);
  const cargo = useRef<HTMLButtonElement>(null), active = useRef(false), generation = useRef(0);
  useEffect(() => { void refresh(); return () => {generation.current++;}; }, []);
  async function refresh() {try {setState(await window.apparentInstaller.action('status')); setError('');} catch (e) {setError(e instanceof Error ? e.message : 'The installer could not check Applications.');}}
  async function install(replace = false) {
    if (!state || active.current || state.installed) return;
    if (state?.needsReplacement && !replace) {setPhase('confirm'); return;}
    active.current = true; setPhase('preparing'); setError('');
    const run = ++generation.current;
    try {
      if (!matchMedia('(prefers-reduced-motion: reduce)').matches) {
        await Promise.all(Array.from(cargo.current?.querySelectorAll('img') ?? []).map(image => image.decode())); await document.fonts.ready;
        await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
        if (run !== generation.current) return;
        const journey = cargo.current?.closest('.installer-journey');
        const destination = journey?.querySelector('.installer-applications svg')?.getBoundingClientRect();
        const origin = cargo.current?.getBoundingClientRect();
        if (destination && origin) {
          (journey as HTMLElement).style.setProperty('--installer-hop-x', `${destination.x + destination.width / 2 - origin.x - origin.width / 2}px`);
          (journey as HTMLElement).style.setProperty('--installer-hop-y', `${destination.y + destination.height / 2 - origin.y - origin.height / 2}px`);
        }
        setPhase('hopping');
        await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
        const animation = cargo.current?.getAnimations().find(a => (a as CSSAnimation).animationName === 'installer-hop');
        if (animation) await animation.finished;
        await new Promise(resolve => setTimeout(resolve, 180));
      }
      if (run !== generation.current) return;
      setPhase('installing');
      const installed = await window.apparentInstaller.action('install', replace);
      if (run === generation.current) setState(installed);
    } catch (e) { if (run === generation.current) setError(e instanceof Error ? e.message : 'Installation did not finish. Your existing app was preserved.'); }
    finally {if (run === generation.current) {active.current = false; setPhase('ready');}}
  }
  async function open() {try {await window.apparentInstaller.action('open');} catch(e) {setError(e instanceof Error ? e.message : 'Open apparent from Applications.');}}
  return <main className="installer"><header>apparent · installer</header><h1>{state?.installed ? 'Ready to make it apparent.' : 'A little hop to Applications.'}</h1>
    <p>{state?.installed ? 'apparent is installed in Applications.' : 'Click or drag apparent into Applications.'}</p>
    <div className="installer-journey" data-hopping={hop} data-phase={phase} data-installed={state?.installed === true}>
      <Button ref={cargo} className="installer-app" variant="ghost" disabled={busy || !state || state.installed} aria-label="Install apparent in Applications" draggable={!busy && !!state && !state.installed} onDragStart={e => {e.dataTransfer.effectAllowed='copy'; e.dataTransfer.setData('application/x-apparent-install','app');}} onClick={() => void install()}><span className="installer-cargo"><img className="installer-frog" src={frog} alt=""/><img className="installer-app-icon" src={icon} alt="apparent app"/></span><span>apparent</span></Button>
      <div className="installer-applications" role="group" aria-label="Applications install drop target" onDragOver={e => {if(e.dataTransfer.types.includes('application/x-apparent-install')) e.preventDefault();}} onDrop={e => {e.preventDefault(); if(e.dataTransfer.getData('application/x-apparent-install')==='app') void install();}}><svg viewBox="0 0 80 64" aria-hidden="true"><path d="M4 12Q4 6 10 6H30L38 16H70Q76 16 76 22V54Q76 60 70 60H10Q4 60 4 54Z"/><path className="installer-folder-front" d="M4 26H76V54Q76 60 70 60H10Q4 60 4 54Z"/><path className="installer-folder-mark" d="M31 44L40 28L49 44M34 39H46M28 45H52"/></svg><strong>Applications</strong></div>
    </div>
    {confirm && <section role="alertdialog" aria-label="Replace existing apparent" aria-describedby="replace-description"><p id="replace-description">Replace the installed Apparent Preview? Finish recordings and quit it first. Its previous app bundle will be preserved; recordings and settings stay in their existing folders.</p><div className="installer-actions"><Button ref={replacement} onClick={() => void install(true)}>Replace app</Button><Button variant="outline" onClick={() => setPhase('ready')}>Keep existing app</Button></div></section>}
    {error && <p role="alert">{error}</p>}
    <footer>{state?.installed ? <><Button onClick={() => void open()}>Open apparent</Button>{state.previousApp && <p>Your previous app was kept at {state.previousApp}. To restore it in Finder, quit apparent, move the current app aside, then rename the backup to Apparent Preview.app.</p>}</> : <>{busy && <span role="status">Installing…</span>}{error && <Button variant="outline" disabled={busy} onClick={() => void refresh()}>Check again</Button>}</>}<small>Opening this installer does not change your recording permissions.</small></footer>
  </main>;
}
createRoot(document.getElementById('root')!).render(<Installer/>);
