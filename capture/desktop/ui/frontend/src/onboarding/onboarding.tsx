import { useEffect, useRef, useState } from 'react';
import { Button } from '@apparent/ui/button';
import icon from '../../../../../../shared/assets/brand/apparent-app-icon.svg';
import frog from '../../../../assets/onboarding/frog.png';
import './onboarding.css';
import { GestureGuide } from './gesture-guide';
import { ControlGuide } from './control-guide';
type Row={permission:string;status:string;required:boolean;reopening:boolean;relaunch:boolean};
type Action=string|{action:'shortcut';kind:'record';keyCode:number;modifiers:number;key:string;reserved:false};
type Editor={recording:boolean;externallyManaged:boolean;error:string|null};
type State={tutorial:'skipped'|'watched'|'practiced'|null;platform:'macos'|'linux';shortcutEditor:Editor|null;step:string;ready:boolean;fix:boolean;rows:Row[];index:number;count:number;shortcut:string;shortcutEditing:boolean;shortcutChosen:boolean;shortcutTested:boolean;practice:string|null;firstCaptureRequired:boolean;penPicker:boolean;penSelected:boolean;penError:boolean;penRgb:number[];testInstance:boolean;coachArrow?:number;coachPointsUp?:boolean};
declare global {interface Window {nativeOnboarding?:{readState:()=>Promise<State|null>;action:(action:Action)=>Promise<boolean>;onState:(listener:(state:State)=>void)=>()=>void}}}
const permissionNames:Record<string,string>={'screen-recording':'Screen & System Audio Recording',microphone:'Microphone',accessibility:'Accessibility','speech-recognition':'Speech Recognition',camera:'Camera'};
export function NativeOnboarding(){
  const [wideCoach,setWideCoach]=useState(window.innerWidth>=700);
  useEffect(()=>{const resize=()=>setWideCoach(window.innerWidth>=700);window.addEventListener('resize',resize);return()=>window.removeEventListener('resize',resize)},[]);
  const [state,setState]=useState<State|null>(null),[error,setError]=useState('');
  const [penPhase,setPenPhase]=useState<'idle'|'preparing'|'hopping'>('idle');
  const penLanding=penPhase==='hopping',penBusy=penPhase!=='idle';
  const penFrog=useRef<HTMLImageElement|null>(null),penRun=useRef(0),penActive=useRef(false);
  const penTimer=useRef<ReturnType<typeof setTimeout>|null>(null),penCommitted=useRef(false);
  useEffect(()=>()=>{penRun.current++;if(penTimer.current)clearTimeout(penTimer.current)},[]);
  useEffect(()=>{if(state?.step!=='welcome'||!state?.penPicker||state?.penError){penRun.current++;penActive.current=false;if(penTimer.current)clearTimeout(penTimer.current);penTimer.current=null;setPenPhase('idle');penCommitted.current=false}},[state?.step,state?.penPicker,state?.penError]);
  document.documentElement.classList.add('native-onboarding-shell');
  useEffect(()=>{const bridge=window.nativeOnboarding;const off=bridge?.onState(setState);void bridge?.readState().then(setState);return()=>off?.()},[]);
  async function action(value:Action){try{setError('');await window.nativeOnboarding?.action(value)}catch{setError('The app connection was interrupted. Close setup and reopen it from apparent.')}}
  const shortcutAdvance=useRef<string|null>(null);
  useEffect(()=>{
    const ready=state?.step==='shortcut'&&state.firstCaptureRequired&&state.shortcutChosen&&state.shortcutTested&&!state.shortcutEditing&&!state.shortcutEditor?.recording;
    if(!ready){shortcutAdvance.current=null;return;}
    if(shortcutAdvance.current===state.shortcut)return;
    shortcutAdvance.current=state.shortcut;
    void action('continue');
  },[state?.step,state?.firstCaptureRequired,state?.shortcutChosen,state?.shortcutTested,state?.shortcutEditing,state?.shortcutEditor?.recording,state?.shortcut]);
  function finishPenHop(){
    if(penCommitted.current)return;
    penCommitted.current=true;
    if(penTimer.current)clearTimeout(penTimer.current);
    penTimer.current=null;
    void action('continue');
  }
  async function confirmPen(){
    if(!state?.penSelected||state.penError||penBusy||penActive.current)return;
    penActive.current=true;const run=++penRun.current;
    penCommitted.current=false;setError('');setPenPhase('preparing');
    if(window.matchMedia('(prefers-reduced-motion: reduce)').matches){finishPenHop();return;}
    const painted=()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve())));
    try {
      const image=penFrog.current;if(!image)throw Error('Missing frog');
      await Promise.all([image.decode(),document.fonts.ready]);
      await painted();if(run!==penRun.current)return;
      setPenPhase('hopping');await painted();if(run!==penRun.current)return;
      const hop=image.getAnimations().find(animation=>(animation as CSSAnimation).animationName==='native-pen-hop');
      if(!hop)throw Error('Hop did not start');
      await hop.finished;if(run!==penRun.current)return;
      // Let the landing remain visible. Advance from rendered animation completion, never a cold-start wall timer.
      await new Promise<void>(resolve=>{penTimer.current=setTimeout(resolve,180)});
      if(run===penRun.current)finishPenHop();
    } catch {
      if(run!==penRun.current)return;
      if(window.matchMedia('(prefers-reduced-motion: reduce)').matches){finishPenHop();return;}
      penActive.current=false;setPenPhase('idle');setError('The frog could not finish its jump. Try again.');
    }
  }
  const shortcutRecorder=useRef<HTMLDivElement|null>(null);
  useEffect(()=>{if(state?.shortcutEditor?.recording)shortcutRecorder.current?.focus()},[state?.shortcutEditor?.recording]);
  if(!state)return <main className="native-setup"><p>Connecting to apparent…</p></main>;
  const goAgain=state.step==='try-it'&&(!state.practice||state.practice==='start');
  const practice=state.step==='try-it'&&!goAgain,penPage=state.step==='welcome'&&state.penPicker;
  const boardName='Whiteboard';
  const trackpadHint=state.practice==='screen-ink';
  const helper=<aside className="native-trackpad-helper" aria-label="Trackpad drawing hint"><div className="native-trackpad-helper-content"><strong>Pressing down on your trackpad?</strong><p>No need. Just hold Shift and glide.</p></div></aside>;
  document.title=state.testInstance?'apparent onboarding preview 9':'apparent setup';
  if(practice){
    const content:Record<string,{title:string;text:string;button?:string;action?:string}>={
      start:{title:'Go again.',text:''},
      screen:{title:'Choose Screen.',text:'Choose Screen in the capture controls. Whiteboard becomes available once recording starts.'},
      whiteboard:{title:`Open ${boardName}.`,text:'Click Screen, then choose Whiteboard.'},
      'board-ink':{title:'Draw on the whiteboard.',text:'Try a circle or an arrow. The guide moves on when you finish your mark.'},
      note:{title:'Add a note.',text:'Hold Shift and click a spot.'},
      'screen-ink':{title:'Draw on your screen.',text:'Hold Shift and circle something in your window.'},
      stop:{title:'Finish your capture.',text:`Press ${state.shortcut} again. Your sketch and screen marks stay together.`},
      preparing:{title:'Preparing your capture…',text:'Recording has stopped. Your marks and context are being brought together.'},
      copied:{title:'Copied. Try pasting it.',text:'Your capture is copied. Paste it into your agent.',button:'Show paste steps',action:'practice-paste'},
      paste:{title:'Paste your capture.',text:`In your agent, press ${state.platform==='linux'?'Ctrl':'⌘'} V. Confirm when it appears.`,button:'Confirm paste',action:'practice-pasted'},
      locate:{title:'apparent lives here.',text:`Click the a in your ${state.platform==='linux'?'system tray':'menu bar'}, then Open Latest Capture.`},
      complete:{title:'Ready for your next idea.',text:'You can record, draw, add notes, paste, and find your captures. Your setup is complete.',button:'Finish setup',action:'practice-finish'},
      retry:{title:'Try another capture.',text:`Press ${state.shortcut} to start again, then follow the drawing and note prompts. Your previous capture is saved.`},
      'copy-failed':{title:'Saved, but not copied.',text:'Your capture is saved. Find it in apparent’s menu.',button:'Find saved capture',action:'practice-locate'},
    };
    const c=content[state.practice??'start']??content.start;
    return <><main className="native-coach" data-paired={trackpadHint&&wideCoach} data-stage={state.practice} data-arrow={state.coachPointsUp} style={{'--coach-arrow':`${(state.coachArrow??190)-24}px`} as React.CSSProperties}><img key={state.practice} src={frog} alt="Frog guide"/><div className="native-coach-content"><div className="native-coach-label">Your first capture</div><h1>{c.title}</h1>{c.text&&<p>{c.text}</p>}{(['start','retry','screen-ink','note','copied','paste'].includes(state.practice??'start'))&&<GestureGuide key={state.practice} kind={state.practice==='note'?'note':state.practice==='screen-ink'?'draw':state.practice==='copied'||state.practice==='paste'?'paste':'record'} shortcut={state.shortcut} platform={state.platform}/>}{['screen','whiteboard','stop','locate'].includes(state.practice??'')&&<ControlGuide key={state.practice} kind={state.practice==='stop'?'finish':state.practice==='locate'?'locate':'whiteboard'} screenOnly={state.practice==='screen'}/>} {c.button&&<Button onClick={()=>void action(c.action!)}>{c.button}</Button>}{trackpadHint&&!wideCoach&&helper}{error&&<p role="alert">{error}</p>}<div className="native-coach-actions">{!state.firstCaptureRequired&&<Button variant="outline" onClick={()=>void action('continue')}>Skip practice</Button>}<Button variant="ghost" className="native-later" onClick={()=>void action('later')}>Try later</Button></div></div></main>{trackpadHint&&wideCoach&&helper}</>;
  }
  return <main className="native-setup" data-shortcut-state={state.step==='shortcut'||goAgain}><header><span>{state.testInstance?'apparent · onboarding preview 9':'apparent · setup'}</span><Button variant="ghost" size="icon" aria-label="Close setup" onClick={()=>void action('close')}>×</Button></header><section>
    {state.step==='welcome'&&!state.penPicker&&<div className="native-welcome"><h1>Welcome.</h1><img src={icon} alt="apparent"/><Button disabled>Sign in with Google</Button><p>Set up locally. Sign-in is not available yet.</p></div>}
    {penPage&&<div className="native-pen-picker"><h1>Pick your pen.</h1><div className="native-pencils">{([['coral',[217,76,68]],['purple',[122,88,199]],['blue',[59,121,189]],['green',[57,123,97]]] as const).map(([name,rgb])=><Button variant="ghost" key={name} aria-label={`${name} pen`} disabled={penBusy} aria-pressed={state.penSelected&&state.penRgb.every((component,index)=>component===rgb[index])} style={{'--pencil':`var(--apparent-pen-${name})`} as React.CSSProperties} onClick={()=>void action(`pen:${name}`)}><span aria-hidden="true"/></Button>)}</div><div className="native-pond" data-landing={penLanding} style={{'--pond-colour':state.penSelected?`rgb(${state.penRgb.join(',')})`:'var(--apparent-muted)'} as React.CSSProperties}><svg className="native-puddle" viewBox="0 0 300 100" role="img" aria-label="Puddle in your selected pen colour"><path d="M22 49C9 21 65 11 99 17C133 1 169 18 192 12C229 4 249 28 275 31C310 37 295 69 263 73C244 94 208 86 180 90C144 102 122 84 91 90C48 93 6 78 22 49Z"/><ellipse className="native-puddle-ring" cx="150" cy="49" rx="48" ry="15"/></svg><img ref={penFrog} className="native-pond-frog" src={frog} alt="Frog beside the puddle"/></div>{state.penError&&<p role="alert">The colour could not be saved. Try choosing it again.</p>}</div>}
    {state.step==='permissions'&&<><h1>{state.fix?'Allow access.':'We need three things.'}</h1><div className="native-permissions">{state.rows.map(row=><div key={row.permission}><div><strong>{permissionNames[row.permission]}</strong>{!row.required&&<small>Optional</small>}</div>{row.reopening?<span>Reopening apparent…</span>:row.status==='granted'&&!row.relaunch?<span className="native-granted">✓ Enabled</span>:<Button variant="outline" onClick={()=>void action(`grant:${row.permission}`)}>{row.relaunch?'Reopen apparent':'Open Settings'}</Button>}</div>)}</div></>}
    {state.step==='shortcut'&&<><h1>{!state.shortcutChosen?'Choose your shortcut.':state.shortcutTested?'Go again.':'Try your shortcut.'}</h1><div className="native-shortcut">
      {state.shortcutEditor?.recording?<div ref={shortcutRecorder} tabIndex={0} role="group" aria-label="Shortcut recorder" onKeyDown={event=>{
      if(event.key==='Tab'||event.target!==event.currentTarget)return;
      event.preventDefault();if(event.key==='Escape'){void action('cancel-shortcut');return;}
      const physical=/^(Key[A-Z]|Digit[0-9])$/.test(event.code)?event.code.replace(/^(Key|Digit)/,'').toLowerCase():event.key.toLowerCase();
      const key=state.platform==='macos'?physical:event.key.toLowerCase();if(!/^[a-z0-9]$/.test(key)||!(event.ctrlKey||event.altKey||event.metaKey)){setError('Use a letter or number with Control, Option or Command.');return;}
      const codes:Record<string,number>={a:0,s:1,d:2,f:3,h:4,g:5,z:6,x:7,c:8,v:9,b:11,q:12,w:13,e:14,r:15,y:16,t:17,'1':18,'2':19,'3':20,'4':21,'6':22,'5':23,'9':25,'7':26,'8':28,'0':29,o:31,u:32,i:34,p:35,l:37,j:38,k:40,n:45,m:46};
      void action({action:'shortcut',kind:'record',keyCode:state.platform==='macos'?codes[key]:key.toUpperCase().charCodeAt(0),modifiers:(event.metaKey?256:0)|(event.shiftKey?512:0)|(event.altKey?2048:0)|(event.ctrlKey?4096:0),key,reserved:false});
    }}><p>Press your new shortcut.</p><Button variant="outline" onClick={()=>void action('cancel-shortcut')}>Cancel</Button></div>:<>
        {state.shortcutChosen?<kbd>{state.shortcut}</kbd>:<p>No shortcut selected.</p>}
        {state.shortcutEditor?.externallyManaged?<Button onClick={()=>void action('use-desktop-shortcut')}>Use desktop shortcut</Button>:<Button variant="outline" onClick={()=>void action('change-shortcut')}>{state.shortcutChosen?'Change shortcut':'Choose shortcut'}</Button>}
      </>}{state.shortcutEditor?.error&&<p role="alert">{state.shortcutEditor.error}</p>}</div></>}
    {goAgain&&<><h1>Go again.</h1><div className="native-shortcut"><kbd>{state.shortcut}</kbd></div></>}
    {state.step==='done'&&<><h1>Ready for your next idea.</h1><div className="native-explain"><img src={frog} alt=""/></div><p>apparent stays in the menu bar, ready when you are.</p></>}
    {error&&<p role="alert">{error}</p>}
    </section><footer className={penPage?'native-pen-footer':undefined}><Button variant="ghost" onClick={()=>void action('back')} disabled={state.index===0||penBusy}>Back</Button>{!penPage&&<span>{state.index+1} / {state.count}</span>}{!goAgain&&<Button aria-busy={penPage&&penBusy} onClick={()=>penPage?confirmPen():void action('continue')} disabled={(penPage&&(!state.penSelected||penBusy||state.penError))||(state.step==='permissions'&&!state.ready)||(state.step==='shortcut'&&state.firstCaptureRequired&&(!state.shortcutChosen||!state.shortcutTested||state.shortcutEditor?.recording))}>{state.step==='welcome'?(state.penPicker?'Use this pen':'Choose a pen'):state.step==='done'?'Finish setup':state.step==='permissions'?(state.fix?'Finish permission setup':'Choose recording shortcut'):state.step==='shortcut'?'Continue':'Try a capture'}</Button>}</footer></main>;
}
