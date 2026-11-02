import {useEffect,useLayoutEffect,useRef,useState} from 'react';
import {createRoot} from 'react-dom/client';
import {Button} from '@apparent/ui/button';
import type {PillView} from './bridge';
import './style.css';

function Pill() {
  const [view,setView]=useState<PillView|null>(null);
  const [pending,setPending]=useState(false);
  const [error,setError]=useState(false);
  const current=useRef<PillView|null>(null);
  const busy=useRef(false);
  const pasteStart=useRef<[number,number]|null>(null);
  useEffect(()=>{
    const bridge=window.apparentPill;
    if(!bridge){setError(true);return;}
    let active=true;
    const apply=(next:PillView|null)=>{
      if(!active || !next || current.current && next.revision<current.current.revision)return;
      if(next.revision!==current.current?.revision){busy.current=false;setPending(false);setError(false);}
      current.current=next;setView(next);
    };
    const unsubscribe=bridge.subscribe(apply);
    void bridge.read().then(apply).catch(()=>{if(active)setError(true);});
    return ()=>{active=false;unsubscribe();};
  },[]);
  useLayoutEffect(()=>{if(view)void window.apparentPill?.layout(view.revision).catch(()=>{});},[view]);
  async function act(action:string,drop=false){
    const before=current.current;
    if(!before || busy.current)return;
    busy.current=true;setPending(true);setError(false);
    try{await window.apparentPill!.act(before.revision,action,drop);}
    catch{if(current.current?.revision===before.revision){busy.current=false;setPending(false);setError(true);}}
    // Successful dispatch unlocks on authoritative readback. Refusals allow a manual retry; never retry a mutation automatically.
  }
  useEffect(()=>{
    const key=(event:KeyboardEvent)=>{
      if(event.key==='Escape' && view?.actions.some(a=>a.id==='cancel' && a.enabled)){
        event.preventDefault();void act('cancel');
      }
    };
    window.addEventListener('keydown',key);return()=>window.removeEventListener('keydown',key);
  },[view]);
  if(!view)return <div role="status" className="pill connection">{error?'Recording controls unavailable':'Connecting…'}</div>;
  if(view.phase==='hidden')return null;
  return <main data-revision={view.revision} className="pill settings-shell" data-phase={view.phase} data-reduced={view.reducedMotion} data-warning={view.warning!==null}>
    <div className={`lead ${view.draggable?'movable':''}`} title={view.draggable?'Drag recording controls':undefined}>
      <span aria-hidden="true" className={`indicator ${view.phase}`}>{view.phase==='completed'?'✓':''}</span>
      <span className="clock" aria-label={view.phase==='recording'?`Recording ${view.clock}`:undefined}>
        {view.phase==='recording'?view.clock:view.label}
      </span>
    </div>
    {view.inputLevel!==null && view.phase==='recording' && <span className="meter" aria-label="Microphone input level">{[.55,.8,1,.75,.5].map((gain,index)=><i key={index} style={{height:`${3+19*gain*view.inputLevel!}px`}}/>)}</span>}
    {view.warning && view.warning!==view.label && <span className="warning" role="status">{view.warning}</span>}
    {view.actions.length>0 && <div className="controls" role="group" aria-label="Recording controls" aria-busy={pending}>
      {view.actions.map(action=><Button key={action.id} variant={action.primary?'default':'ghost'}
        data-primary={action.primary}
        aria-pressed={action.id.startsWith('select-')?action.selected:undefined}
        disabled={!action.enabled || pending}
        onPointerDown={event=>{if(action.id==='paste' && event.button===0 && event.isPrimary){pasteStart.current=[event.screenX,event.screenY];event.currentTarget.setPointerCapture(event.pointerId);void window.apparentPill?.beginPaste(view.revision).catch(()=>{pasteStart.current=null;});}}}
        onPointerUp={event=>{if(action.id==='paste' && event.button===0 && event.isPrimary && pasteStart.current){const [x,y]=pasteStart.current;pasteStart.current=null;void act('paste',Math.hypot(event.screenX-x,event.screenY-y)>=24);}}}
        onPointerCancel={()=>{pasteStart.current=null;void window.apparentPill?.cancelPaste();}}
        onClick={event=>{if(action.id!=='paste' || event.detail===0)void act(action.id);}}>{action.label}</Button>)}
    </div>}
    {view.notice && <span role="status" className="notice">{view.notice}</span>}
    {error && <span role="alert">Action not confirmed. Use the recording shortcut or tray menu.</span>}
  </main>;
}
createRoot(document.getElementById('root')!).render(<Pill/>);
