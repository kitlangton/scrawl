import {useEffect,useRef,useState,type ReactNode} from 'react';
import {Collapsible,CollapsibleTrigger,CollapsibleContent} from '@apparent/ui/collapsible';
import {Alert,AlertTitle,AlertDescription} from '@apparent/ui/alert';
import {Button} from '@apparent/ui/button';
import {ChevronDown,ChevronUp,CircleAlert,Info,TriangleAlert} from 'lucide-react';
import type {PocketView} from './bridge';

/** Disclosure is local presentation state; all mutations use existing Rust actions. */
export function UploadPocket({pocket,notice,children,actions,onLayout}: {
  pocket:PocketView|null;notice:string|null;children:(trigger:ReactNode)=>ReactNode;actions:ReactNode;onLayout:()=>void;
}) {
  const StatusIcon=pocket?.tone==='failure'?CircleAlert:pocket?.tone==='warning'?TriangleAlert:Info;
  const triggerRef=useRef<HTMLButtonElement>(null);
  const identity=pocket?JSON.stringify({pocket,notice}):'';
  const [closed,setClosed]=useState('');
  const open=!!pocket && closed!==identity;
  useEffect(()=>{onLayout();},[open,identity,onLayout]);
  useEffect(()=>{
    if(!open)return;
    const key=(event:KeyboardEvent)=>{if(event.key==='Escape'){
      event.preventDefault();event.stopImmediatePropagation();setClosed(identity);triggerRef.current?.focus();
    }};
    window.addEventListener('keydown',key,true);return()=>window.removeEventListener('keydown',key,true);
  },[open,identity]);
  return <Collapsible className="pocket-shell" open={open} onOpenChange={value=>setClosed(value?'':identity)}
    onKeyDownCapture={event=>{if(event.key==='Escape' && open){event.preventDefault();event.stopPropagation();setClosed(identity);}}}>
    {children(pocket && <CollapsibleTrigger asChild><Button ref={triggerRef} variant="ghost" className="pocket-trigger" aria-label={open?'Hide notification':'Show notification'}>{open?<ChevronUp aria-hidden="true"/>:<ChevronDown aria-hidden="true"/>}</Button></CollapsibleTrigger>)}
    <CollapsibleContent className="upload-pocket">
      {pocket && <Alert className="pocket-alert" variant={pocket.tone==='failure'?'destructive':'default'} data-tone={pocket.tone}>
        <AlertTitle className="pocket-title"><span className="pocket-status-icon"><StatusIcon aria-hidden="true"/></span>{pocket.title}</AlertTitle>
        {notice && <AlertDescription>{notice}</AlertDescription>}
        {pocket.description && <AlertDescription>{pocket.description}</AlertDescription>}
        {actions && <div className="pocket-actions">{actions}</div>}
      </Alert>}
    </CollapsibleContent>
  </Collapsible>;
}
