import '../runtime/exceptions';
import {useCallback,useEffect,useLayoutEffect,useRef,useState} from 'react';
import {createRoot} from 'react-dom/client';
import {Button} from '@apparent/ui/button';
import {StickyNote} from '@apparent/ui/icons';
import type {PillView} from './bridge';
import './style.css';
import {UploadPocket} from './UploadPocket';
import {Check,Circle,ClipboardPaste,Link,LoaderCircle,Monitor,PenLine,Scan,ScanEye,Smartphone,Square,X,type LucideIcon} from 'lucide-react';

const actionIcons:Record<string,LucideIcon>={
 'select-area':Scan,'switch-area':Scan,'select-screen':Monitor,'switch-screen':Monitor,
 'select-iphone':Smartphone,'switch-iphone':Smartphone,record:Circle,whiteboard:PenLine,
 finish:Square,paste:ClipboardPaste,'copy-link':Link,inspect:ScanEye,cancel:X,dismiss:X,
};

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
  const layout=useCallback(()=>{const revision=current.current?.revision;if(revision!==undefined)void window.apparentPill?.layout(revision).catch(()=>{});},[]);
  if(!view)return <div role="status" className="pill connection">{error?'Recording controls unavailable':'Connecting…'}</div>;
  if(view.phase==='hidden')return null;
  return <main data-revision={view.revision} className="pill settings-shell" aria-label={view.phase==='selecting'?view.label:undefined} data-phase={view.phase} data-reduced={view.reducedMotion} data-warning={view.warning!==null}>
    <UploadPocket notice={error?null:view.notice} pocket={error?{tone:'failure',title:'Action not confirmed',description:'Use the recording shortcut or tray menu.',actionIds:[]}:view.pocket??(view.warning?{tone:'warning',title:'Recording notice',description:view.warning,actionIds:[]}:view.notice?{tone:'info',title:'Control changed',description:'',actionIds:[]}:null)} onLayout={layout}
      actions={view.pocket?.actionIds.map(id=>{const action=view.actions.find(a=>a.id===id);return action && <Button key={id} variant="outline" disabled={!action.enabled || pending} onClick={()=>void act(id)}>{action.label}</Button>;})}>
    {trigger=><div className={`pill-surface${view.noteCount>0?' has-notes':''}`}>
    {view.noteCount>0 && <div className="pill-notes-layer"><Button className="pill-notes" variant="ghost"
      aria-label={`${view.noteCount} saved notes. Open notes`} disabled={pending || !view.actions.some(a=>a.id==='notes' && a.enabled)}
      onClick={()=>void act('notes')}><StickyNote aria-hidden="true"/><span>{view.noteCount>99?'99+':view.noteCount}</span></Button></div>}
    <div className="pill-row">
    {(view.phase!=='selecting' || trigger) && <div className={`lead ${view.draggable?'movable':''} ${window.apparentPill?.manualDrag?'manual-drag':''}`} title={view.draggable?'Drag recording controls':undefined}
      onPointerDown={event=>{if(view.draggable && !(event.target as HTMLElement).closest('button') && window.apparentPill?.manualDrag && event.button===0 && event.isPrimary)void window.apparentPill.beginDrag(view.revision).catch(()=>{});}}>
      <span className={`indicator-slot ${view.phase}`}>
        {trigger || (view.phase==='completed'?<Check aria-hidden="true" className="indicator completed"/>:view.phase==='processing'?<LoaderCircle aria-hidden="true" className="indicator processing"/>:<span aria-hidden="true" className={`indicator ${view.phase}`}/>)}
      </span>
      {view.phase!=='selecting' && <span className="clock" aria-label={view.phase==='recording'?`Recording ${view.clock}`:undefined}>
        {view.phase==='recording'?view.clock:view.phase==='processing'?'Processing…':view.label}
      </span>}
    </div>}
    {view.inputLevel!==null && view.phase==='recording' && <span className="meter" aria-label="Microphone input level">{[.55,.8,1,.75,.5].map((gain,index)=><i key={index} style={{height:`${3+19*gain*view.inputLevel!}px`}}/>)}</span>}
    {view.actions.length>0 && <div className="controls" role="group" aria-label="Recording controls" aria-busy={pending}>
      {view.actions.filter(action=>action.id!=='notes').map(action=>{const Icon=actionIcons[action.id];const iconOnly=action.id==='cancel'||action.id==='dismiss';return <Button key={action.id} variant={action.primary?'default':action.selected?'secondary':'ghost'}
        size={iconOnly?'icon':'default'} data-icon-only={iconOnly} aria-label={action.label} title={iconOnly?action.label:undefined}
        data-primary={action.primary}
        aria-pressed={action.id.startsWith('select-')?action.selected:undefined}
        disabled={!action.enabled || pending}
        onPointerDown={event=>{if(action.id==='paste' && event.button===0 && event.isPrimary){pasteStart.current=[event.screenX,event.screenY];event.currentTarget.setPointerCapture(event.pointerId);void window.apparentPill?.beginPaste(view.revision).catch(()=>{pasteStart.current=null;});}}}
        onPointerUp={event=>{if(action.id==='paste' && event.button===0 && event.isPrimary && pasteStart.current){const [x,y]=pasteStart.current;pasteStart.current=null;void act('paste',Math.hypot(event.screenX-x,event.screenY-y)>=24);}}}
        onPointerCancel={()=>{pasteStart.current=null;void window.apparentPill?.cancelPaste();}}
        onClick={event=>{if(action.id!=='paste' || event.detail===0)void act(action.id);}}>{Icon && <Icon aria-hidden="true"/>}{!iconOnly && <span>{action.label}</span>}</Button>;})}
    </div>}
    </div>
    </div>}
    </UploadPocket>
  </main>;
}
createRoot(document.getElementById('root')!).render(<Pill/>);
