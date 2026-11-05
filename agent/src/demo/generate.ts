import { constants } from 'node:fs';
import { chmod, lstat, mkdir, open, realpath, rmdir, unlink, writeFile } from 'node:fs/promises';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { randomUUID } from 'node:crypto';
import { PNG } from 'pngjs';
import { resolveCaptures } from '../bundle.js';
import { semanticSummary } from '../summary.js';
import { annotateZoomFrame } from '../zoom/zoom.js';

const W=960,H=600,DURATION=9800;
const times=[400,1500,3600,6400,9200];
function rect(png:PNG,x:number,y:number,w:number,h:number,color:[number,number,number]) {
  for(let yy=Math.max(0,y);yy<Math.min(H,y+h);yy++)for(let xx=Math.max(0,x);xx<Math.min(W,x+w);xx++){const i=(yy*W+xx)*4;png.data[i]=color[0];png.data[i+1]=color[1];png.data[i+2]=color[2];png.data[i+3]=255;}
}
function scene(step:number) {
  const image=new PNG({width:W,height:H});
  for(let i=0;i<W*H;i++){image.data[i*4]=247;image.data[i*4+1]=248;image.data[i*4+2]=250;image.data[i*4+3]=255;}
  rect(image,0,0,W,46,[36,41,50]);rect(image,0,46,185,H-46,[235,238,242]);
  rect(image,22,78,130,12,[133,144,160]);rect(image,22,111,112,10,[174,183,194]);rect(image,22,147,140,10,[174,183,194]);
  rect(image,225,82,510,46,[255,255,255]);rect(image,225,145,510,130,[255,255,255]);rect(image,225,291,510,104,[255,255,255]);
  rect(image,250,101,200,10,step<2?[70,79,93]:[42,111,204]);
  rect(image,250,167,300,10,[111,122,137]);rect(image,250,193,420,8,[190,197,205]);rect(image,250,217,370,8,[190,197,205]);
  rect(image,250,318,260,10,[111,122,137]);rect(image,250,346,step<2?220:300,8,[190,197,205]);
  rect(image,750,440,180,112,[255,255,255]);rect(image,777,470,122,12,[150,158,170]);rect(image,810,510,86,28,[42,111,204]);
  return image;
}
function stroke(points:number[][],t0:number,t1:number,on:string,pin?:number) {
  const pts:number[]=[];for(const [i,p] of points.entries())pts.push(Math.round(p[0]*10000),Math.round(p[1]*10000),Math.round((t1-t0)*i/Math.max(1,points.length-1)));
  return {t0,t1,fw:W,fh:H,on,pin:pin??null,laser:false,pts};
}
function model() {
  const circle=Array.from({length:37},(_,i)=>[.51+.14*Math.cos(i*Math.PI*2/36),.175+.075*Math.sin(i*Math.PI*2/36)]);
  const strokes=[stroke(circle,2900,3548,'Header band'),stroke(Array.from({length:13},(_,i)=>[.22+.22*i/12,.48-.12*i/12]),5800,5992,'Tool usage'),stroke([[.41,.35],[.44,.36],[.42,.4]],6100,6160,'Tool usage'),stroke([[.88,.88]],8900,8900,'Send button',1)];
  const frame=(index:number,t:number,reason:string,event:string,markId:string|null)=>({index,t,reason,event,said:t<5000?'The header shifts when I select the tool.':'The spacing below the card feels too tight.',markId,file:`frames/frame-${String(index).padStart(2,'0')}.png`});
  const marks=[
    {id:'demo-circle',strokes:[0],t0:2900,t1:3548,boxNorm:[.37,.10,.65,.25],shape:'circle',description:'Circled the synthetic header',on:'Header band',frame:'frames/frame-03.png'},
    {id:'demo-arrow',strokes:[1,2],t0:5800,t1:6160,boxNorm:[.22,.35,.44,.48],shape:'arrow',description:'Arrow toward the synthetic tool card',on:'Tool usage',from:[.22,.48],to:[.44,.36],frame:'frames/frame-04.png'},
    {id:'demo-pin',strokes:[3],t0:8900,t1:8900,boxNorm:[.87,.87,.89,.89],shape:'pin',description:'Pinned the synthetic send button',on:'Send button',pin:1,frame:'frames/frame-05.png'}
  ];
  const transcript='When I select Tool usage the header shifts down slightly, and the spacing below the card feels too tight.';
  const words=[['When',600,850],['I',850,980],['select',980,1320],['Tool',1320,1650],['usage',1650,1900],['the',1900,2050],['header',2050,2400],['shifts',2400,2700],['down',2700,2950],['slightly,',2950,3400],['and',5400,5700],['the',5700,5900],['spacing',5900,6300],['below',6300,6700],['the',6700,6900],['card',6900,7200],['feels',7800,8200],['too',8200,8500],['tight.',8500,9400]].map(([text,t0,t1])=>({text:String(text),t0:Number(t0),t1:Number(t1)}));
  return {version:1,id:'synthetic-demo-v1',createdAtMs:1,durationMs:DURATION,synthetic:true,provenance:{kind:'synthetic-demo',notice:'Illustrative synthetic bundle; no screen, window, audio device, session, or capture was read.'},target:{mode:'window',app:'Apparent Demo (synthetic)',bundleId:null,windowTitle:'Illustrative interface — not a real window',windowId:null,displayId:0,frame:{x:0,y:0,width:W,height:H,scale:1}},context:{microphoneCapture:'off'},transcript:{source:'synthetic-demo',model:null,text:transcript,segments:[{t0:600,t1:4100,text:'When I select Tool usage the header shifts down slightly,'},{t0:5400,t1:9400,text:'and the spacing below the card feels too tight.'}],words,estimated:false,missing:null},strokes,marks,events:[{kind:'click',t:1250,x:.46,y:.36,button:'left',label:'Tool usage',role:'button'},{kind:'scroll',t:4800,x:.55,y:.55,dx:0,dy:-240,label:'Synthetic transcript'}],keyframes:times.map((t,i)=>frame(i+1,t,['scene','click','mark','mark','mark'][i],['synthetic interface','clicked Tool usage','circled Header band','drew arrow to Tool usage','pinned Send button'][i],[null,null,'demo-circle','demo-arrow','demo-pin'][i])),contactSheet:'contact-sheet.png',note:'Synthetic demonstration only. All interface imagery, interactions, narration, and timings are illustrative.',source:'synthetic-demo'};
}
async function createPrivateDirectory(path:string) {
  if(!isAbsolute(path)||resolve(path)!==path)throw new Error('Demo output must be a canonical absolute directory path');
  const parent=dirname(path);let cursor='/';
  for(const part of parent.split('/').filter(Boolean)){cursor=join(cursor,part);const info=await lstat(cursor).catch((e:NodeJS.ErrnoException)=>e.code==='ENOENT'?undefined:Promise.reject(e));if(info?.isSymbolicLink()||info&&!info.isDirectory())throw new Error('Demo output path contains a symlink or non-directory');}
  await mkdir(parent,{recursive:true,mode:0o700});if(await realpath(parent)!==parent)throw new Error('Demo output parent must be canonical');
  await mkdir(path,{mode:0o700});await chmod(path,0o700);
  const info=await lstat(path);if(!info.isDirectory()||info.isSymbolicLink()||(info.mode&0o077))throw new Error('Demo output directory must be private');
}
export async function generateDemo(out?:string) {
  const root=out??join(await resolveCaptures(),`demo-${Date.now()}-${randomUUID()}.apparent`), modelValue=model();
  await createPrivateDirectory(root);const owned:string[]=[];
  try {
    await mkdir(join(root,'frames'),{mode:0o700});owned.push('frames');
    const full=times.map((t,i)=>scene(i));
    for(let i=0;i<full.length;i++){
      const relative=`frames/frame-${String(i+1).padStart(2,'0')}.png`,path=join(root,relative);
      annotateZoomFrame(full[i],modelValue,times[i],[0,0,1,1]);
      owned.push(relative);await writeFile(path,PNG.sync.write(full[i]),{mode:0o600,flag:'wx'});
    }
    const sheet=new PNG({width:960,height:620});for(let i=0;i<sheet.data.length;i+=4){sheet.data[i]=31;sheet.data[i+1]=35;sheet.data[i+2]=42;sheet.data[i+3]=255;}
    for(let i=0;i<full.length;i++){const x=10+(i%2)*475,y=10+Math.floor(i/2)*202,scale=Math.min(460/W,192/H);for(let yy=0;yy<Math.floor(H*scale);yy++)for(let xx=0;xx<Math.floor(W*scale);xx++){const si=(Math.floor(yy/scale)*W+Math.floor(xx/scale))*4,di=((y+yy)*960+x+xx)*4;full[i].data.copy(sheet.data,di,si,si+4);}}
    owned.push('contact-sheet.png');await writeFile(join(root,'contact-sheet.png'),PNG.sync.write(sheet),{mode:0o600,flag:'wx'});
    const summary=semanticSummary(modelValue,root);
    owned.push('summary.md');await writeFile(join(root,'summary.md'),summary,{mode:0o600,flag:'wx'});
    // Manifest is the completion marker: publish only after every referenced asset is complete.
    const manifest=join(root,'apparent.json');owned.push('apparent.json');const file=await open(manifest,constants.O_WRONLY|constants.O_CREAT|constants.O_EXCL|constants.O_NOFOLLOW,0o600);
    try{await file.writeFile(`${JSON.stringify(modelValue,null,2)}\n`);await file.sync();}finally{await file.close();}
    return {path:root,summary,images:[join(root,'contact-sheet.png'),...modelValue.keyframes.map((k:any)=>join(root,k.file))]};
  } catch(error) {
    const rootInfo=await lstat(root).catch(()=>undefined);
    const ownedRoot=rootInfo?.isDirectory()&&!rootInfo.isSymbolicLink()&&(typeof process.getuid!=='function'||rootInfo.uid===process.getuid());
    if(ownedRoot)for(const relative of owned.reverse()){
      const path=join(root,relative);
      if(relative.includes('/')){const parentInfo=await lstat(dirname(path)).catch(()=>undefined);if(!parentInfo?.isDirectory()||parentInfo.isSymbolicLink())continue;}
      if(relative==='frames')await rmdir(path).catch(()=>undefined);
      else {const info=await lstat(path).catch(()=>undefined);if(info?.isFile()||info?.isSymbolicLink())await unlink(path).catch(()=>undefined);}
    }
    if(ownedRoot)await rmdir(root).catch(()=>undefined);throw error;
  }
}
