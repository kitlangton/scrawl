import {useEffect,useRef,useState} from 'react';
import {Button} from '@apparent/ui/button';
import type {PublicShareResult} from './bridge';
export function PublicShare({capture}:{capture:string}) {
 const [result,setResult]=useState<PublicShareResult>();
 const [confirmed,setConfirmed]=useState(false);
 const version=useRef(0);
 useEffect(()=>{setConfirmed(false);setResult(undefined);return ()=>{version.current++;void window.apparent?.cancelPublicShare();};},[capture]);
 useEffect(()=>{
   if(result?.status!=='connected'||result.state!=='preparing')return;
   const token=version.current;
   const timer=setTimeout(()=>{void window.apparent?.readPublicShare().then(next=>{if(token===version.current)setResult(next);}).catch(()=>{if(token===version.current)setResult({status:'error',code:'outcome_unknown'});});},500);
   return ()=>clearTimeout(timer);
 },[result]);
 const preparing=result?.status==='connected'&&result.state==='preparing';
 async function start(){const token=++version.current;setConfirmed(false);setResult({status:'connected',state:'preparing'});try{const next=await window.apparent?.startPublicShare();if(token===version.current)setResult(next);}catch{if(token===version.current)setResult({status:'error',code:'outcome_unknown'});}}
 return <div className="space-y-3 border-t pt-4">
  <Button variant="outline" disabled={preparing} onClick={()=>setConfirmed(true)}>Copy public link</Button>
  {confirmed&&<div role="group" aria-label="Confirm public sharing" className="space-y-3">
   <p>Anyone with this link can view this capture. Shared captures are not end-to-end encrypted.</p>
   <Button onClick={()=>void start()}>Create and copy public link</Button>{' '}
   <Button variant="outline" onClick={()=>setConfirmed(false)}>Cancel</Button>
  </div>}
  <div role="status" aria-live="polite">
   {preparing&&<><p>Preparing public link…</p><Button variant="outline" onClick={()=>{version.current++;void window.apparent?.cancelPublicShare().then(setResult);}}>Cancel sharing</Button></>}
   {result?.status==='connected'&&result.state==='copied'&&<p>Link copied · upload may still be preparing. Open the link to check readiness.</p>}
   {result&&!(result.status==='connected'&&['preparing','copied'].includes(result.state))&&<p>Public link was not confirmed copied. Check your clipboard and refresh before trying again. Local Paste remains available.</p>}
  </div>
 </div>;
}
