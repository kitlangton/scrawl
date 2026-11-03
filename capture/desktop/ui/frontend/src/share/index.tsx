import {useEffect,useState} from 'react';
import {Button} from '@apparent/ui/button';
type Snapshot={capture:string;state:string};
declare global {interface Window {apparentShare?:{subscribe:(callback:(s:Snapshot)=>void)=>()=>void;confirm:()=>Promise<void>;cancel:()=>Promise<void>}}}
export function ShareCapture(){
 const [snapshot,setSnapshot]=useState<Snapshot>();const [submitted,setSubmitted]=useState(false);
 useEffect(()=>window.apparentShare?.subscribe(setSnapshot),[]);
 const state=snapshot?.state;
 return <main className="space-y-5 p-6"><h1 className="text-xl font-semibold">Share capture</h1>
 <p className="break-all text-sm text-muted-foreground">{snapshot?.capture??'Reading capture…'}</p>
 {state==='confirm'&&<><p>Anyone with this link can view this capture. Shared captures are not end-to-end encrypted.</p>
 <Button disabled={submitted} onClick={()=>{setSubmitted(true);void window.apparentShare?.confirm().catch(()=>setSnapshot(current=>current?{...current,state:"failed"}:current));}}>Create and copy public link</Button></>}
 <div role="status" aria-live="polite">{state==='preparing'?'Preparing public link…':state==='prepared'?'Return to this window to copy the link.':state==='copied'?'Link copied · upload may still be preparing. Open the link to check readiness.':state&&state!=='confirm'?'Public link was not confirmed copied. Local Paste remains available. Close and try again when connected.':''}</div>
 <Button variant="outline" onClick={()=>void window.apparentShare?.cancel().catch(()=>{})}>{state==='copied'?'Close':'Cancel'}</Button></main>;
}
