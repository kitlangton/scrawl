import '../runtime/exceptions';
import {useEffect,useState} from 'react';
import {createRoot} from 'react-dom/client';
import {Button} from '@apparent/ui/button';
import {BuildIdentityFrame} from '@apparent/ui/build-identity';
import '../style.css';
type Editor={label:string;externallyManaged:boolean;recording:boolean;error?:string};
type View={type:string;step?:string;ready?:boolean;index?:number;count?:number;label?:string;shortcut?:Editor;firstCaptureRequired?:boolean;shortcutChosen?:boolean;shortcutEditing?:boolean;rows?:{permission:string;status:string;requested:boolean;required:boolean;reopening:boolean;relaunch:boolean}[]};
type Action=string|{action:string;kind:string;keyCode:number;modifiers:number;key:string;reserved:boolean};
declare global {interface Window {onboarding:{action:(a:Action)=>Promise<void>;subscribe:(fn:(v:View)=>void)=>()=>void}}}
const macCodes:Record<string,number>={a:0,s:1,d:2,f:3,h:4,g:5,z:6,x:7,c:8,v:9,b:11,q:12,w:13,e:14,r:15,y:16,t:17,'1':18,'2':19,'3':20,'4':21,'6':22,'5':23,'9':25,'7':26,'8':28,'0':29,o:31,u:32,i:34,p:35,l:37,j:38,k:40,n:45,m:46};
const titles:Record<string,string>={welcome:'Show what you mean','how-it-works':'Point. Talk. Share.',permissions:'Allow capture on this computer',shortcut:'Choose your recording shortcut','try-it':'Make your first capture',done:'Ready when you are'};
function Onboarding(){
 const [view,setView]=useState<View>({type:'state'}),[shortcut,setShortcut]=useState('your shortcut'),[editor,setEditor]=useState<Editor>(),[error,setError]=useState('');
 useEffect(()=>window.onboarding.subscribe(v=>{if(v.type==='state')setView(v);if(v.type==='shortcut'&&v.label)setShortcut(v.label);if(v.type==='shortcut-editor')setEditor(v.shortcut)}),[]);
 const act=(a:Action)=>{setError('');void window.onboarding.action(a).catch(()=>setError('This action could not be sent. Please try again.'))};
 const editing=editor?.recording??view.shortcutEditing;
 return <main style={{padding:'40px 40px 76px',maxWidth:620,margin:'auto'}}><p>{view.step==='done'?'Apparent · Setup complete':`Apparent · ${(view.index??0)+1} / ${view.count??6}`}</p><h1>{titles[view.step??'welcome']??'Apparent'}</h1>
 {view.step==='welcome'&&<p>Give your agent the context you can see. Record your screen, explain aloud and point to what matters.</p>}
 {view.step==='how-it-works'&&<ol><li>Press your shortcut to record.</li><li>Talk, point and draw to explain your idea.</li><li>Press the shortcut again, then hand the capture to your agent.</li></ol>}
 {view.step==='permissions'&&<p>Apparent checks permission with your operating system. Return here after changing a permission in Settings.</p>}
 {view.step==='permissions'&&view.rows?.map(row=><section key={row.permission} style={{marginBlock:16}}><strong>{row.permission.replaceAll('-',' ')}</strong> · {row.status}{row.required?' (required)':''}<p>{row.relaunch?'A relaunch is needed for this permission.':row.status==='granted'?'':row.reopening?'Open system Settings to update access.':row.requested?'Waiting for the operating system to confirm access.':''}</p><Button disabled={row.status==='granted'&&!row.relaunch} onClick={()=>act('grant:'+row.permission)}>{row.relaunch?'Reopen Apparent':row.requested?'Open Settings':'Allow'}</Button></section>)}
 {view.step==='shortcut'&&<><p>Current shortcut: <strong>{editor?.label??shortcut}</strong></p>{editor?.externallyManaged?<><p>Your desktop manages this shortcut. Configure it in desktop Settings, then confirm.</p><Button onClick={()=>act('use-desktop-shortcut')}>Use desktop shortcut</Button></>:<><Button onClick={()=>act('use-current-shortcut')}>Use this shortcut</Button> <Button onClick={()=>act('change-shortcut')}>Change shortcut</Button> <Button onClick={()=>act('reset-shortcut')}>Use default shortcut</Button></>}
 {editing&&!editor?.externallyManaged&&<div tabIndex={0} role="group" aria-label="Shortcut recorder" onKeyDown={e=>{e.preventDefault();if(e.key==='Escape'){act('cancel-shortcut');return}const key=e.key.toLowerCase();if(!/^[a-z0-9]$/.test(key)||!(e.ctrlKey||e.altKey||e.metaKey)){setError('Use a letter or number with Control, Alt or Command.');return}const mac=/Mac/.test(navigator.platform);act({action:'shortcut',kind:'record',keyCode:mac?macCodes[key]:key.toUpperCase().charCodeAt(0),modifiers:(e.metaKey?256:0)|(e.shiftKey?512:0)|(e.altKey?2048:0)|(e.ctrlKey?4096:0),key,reserved:false})}}><p>Click here, then press your new shortcut.</p><Button onClick={()=>act('cancel-shortcut')}>Cancel</Button></div>}{editor?.error&&<p role="alert">{editor.error}</p>}{!view.shortcutChosen&&<p>Choose or confirm a shortcut to continue.</p>}</>}
 {view.step==='try-it'&&<><p>Press <strong>{shortcut}</strong> to start. Talk and point, then press it again to finish. Wait for Apparent to confirm the capture.</p>{!view.ready&&<p>Your first capture has not completed yet.</p>}<p>This practice stays on this computer and is not uploaded.</p>{!view.firstCaptureRequired&&<Button onClick={()=>act('continue')}>Skip practice</Button>}</>}
 {view.step==='done'&&<p>Use {shortcut} whenever you need to explain something on screen. Finish setup below.</p>}
 {error&&<p role="alert">{error}</p>}<footer style={{display:'flex',gap:12,marginTop:32}}><Button disabled={!view.index} onClick={()=>act('back')}>Back</Button><Button onClick={()=>act('later')}>Later</Button><Button disabled={!view.step||(view.step==='permissions'&&!view.ready)||(view.step==='shortcut'&&view.firstCaptureRequired&&!view.shortcutChosen)||(view.step==='try-it'&&view.firstCaptureRequired)} onClick={()=>act('continue')}>{view.step==='done'?'Finish':'Continue'}</Button></footer></main>
}
createRoot(document.getElementById('root')!).render(<BuildIdentityFrame><Onboarding/></BuildIdentityFrame>);
