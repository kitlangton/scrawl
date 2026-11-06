import { constants } from 'node:fs';
import { lstat, mkdir, open, realpath, rmdir, unlink } from 'node:fs/promises';
import { Worker } from 'node:worker_threads';
import { homedir } from 'node:os';
import { dirname, isAbsolute, join, resolve, sep } from 'node:path';
import { createHash, randomBytes } from 'node:crypto';
import { PNG } from 'pngjs';
import { readManifest, resolveBundle, safeReference } from '../bundle.js';
import { decodeFrameTimes, hasSourceVideo, savedKeyframes, verifyFramesExecutable, verifyPrivateRuntime } from '../frames.js';
import { markEntries } from '../inspection.js';

const MAX_FRAMES = 48, MAX_FILE_BYTES = 16 * 1024 * 1024, MAX_IMAGE_PIXELS = 9_000_000;
const MAX_TOTAL_INPUT_BYTES = 64 * 1024 * 1024, MAX_TOTAL_PIXELS = 64_000_000;
const MAX_RENDER_MS = 60_000, MAX_STROKES = 4096, MAX_POINTS = 8192, MAX_EVENTS = 8192, MAX_MARKS = 4096;
const CELL_W = 480, CELL_H = 320, GAP = 6;
type RasterBudget={steps:number};
const signature = Buffer.from([137,80,78,71,13,10,26,10]);
const normalize = (text: string) => text.replace(/[^\p{L}\p{N}\s]/gu, '').toLowerCase();
export const clock = (ms: number) => `${Math.floor(ms / 60000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, '0')}.${String(ms % 1000).padStart(3, '0')}`;

export function findPhraseMs(bundle: any, phrase: string): number | undefined {
  const needle = normalize(phrase).trim().split(/\s+/).filter(Boolean);
  if (!needle.length) return;
  const words: any[] = bundle.transcript.words ?? [];
  const tokens = words.map(word => normalize(word.text).replace(/\s/g, ''));
  for (let i = 0; i < tokens.length; i++) {
    if (needle.every((token, j) => tokens[i + j] === token)) return words[i].t0;
  }
  const joined = needle.join(' ');
  return bundle.transcript.segments.find((segment: any) => normalize(segment.text).trim().split(/\s+/).join(' ').includes(joined))?.t0;
}

export function resolveMoment(bundle: any, at: string): number {
  try { return parseZoomTime(at); } catch { /* phrase lookup follows legacy time-first resolution */ }
  const found = findPhraseMs(bundle, at);
  if (found === undefined) throw new Error(`“${at}” is neither a time nor a phrase in the narration`);
  return found;
}

export function parseZoomTime(input: string): number {
  const text = input.trim();
  let value: number;
  if (text.includes(':')) {
    const parts = text.split(':');
    if (parts.length !== 2 || !/^\d+$/.test(parts[0]) || !/^(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?$/.test(parts[1])) throw new Error('invalid time');
    value = Number(parts[0]) * 60_000 + Math.round(Number(parts[1]) * 1000);
  } else if (text.endsWith('ms')) {
    const milliseconds=text.slice(0,-2).trim();
    if(!/^\+?\d+$/.test(milliseconds)) throw new Error('invalid time');
    value=Number(milliseconds);
  } else {
    const seconds = text.endsWith('s') ? text.slice(0, -1) : text;
    if (!/^[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?$/.test(seconds)) throw new Error('invalid time');
    value = Math.round(Number(seconds) * 1000);
  }
  if (!Number.isSafeInteger(value) || value < 0) throw new Error('invalid time');
  return value;
}

export function zoomWindow(at: number, span: number, duration: number): [number, number] {
  const size = Math.max(200, Math.min(span, Math.max(duration, 200)));
  let from = Math.max(0, at - Math.floor(size / 4)), to = from + size;
  if (from > duration-size || to > duration) { to = duration; from = Math.max(0, to - size); }
  return [from, to];
}
export function markWindow(t0: number, t1: number, span: number, duration: number): [number, number] {
  const size = Math.max(200, Math.min(Math.max(span, t1 - t0 + 1000), Math.max(duration, 200)));
  const midpoint=Number((BigInt(t0)+BigInt(t1))/2n);
  let from = Math.max(0, midpoint - Math.floor(size / 2)), to = from + size;
  if (from > duration-size || to > duration) { to = duration; from = Math.max(0, to - size); }
  return [from, to];
}
export function sampleTimes(from: number, to: number, count: number): number[] {
  const n = Math.max(1, Math.min(MAX_FRAMES, count));
  if (n === 1 || to <= from) return [from];
  const delta=BigInt(to-from), divisor=BigInt(n-1);
  return Array.from({ length: n }, (_, i) => from + Number(delta*BigInt(i)/divisor));
}

function pickMark(bundle: any, query: string, frame?: number) {
  query=query.trim();
  const entries = markEntries(bundle);
  if (frame !== undefined && (!Number.isSafeInteger(frame) || frame < 1)) throw new Error('--frame must be a positive keyframe number');
  const circled='①②③④⑤⑥⑦⑧⑨⑩⑪⑫⑬⑭⑮⑯⑰⑱⑲⑳';
  const circledIndex=circled.indexOf(query);
  const numeric = /^\d+$/.test(query) ? Number(query) : circledIndex>=0?circledIndex+1:undefined;
  const matches = entries.filter((entry: any) => (entry.id === query || numeric !== undefined && entry.number === numeric) &&
    (frame === undefined || entry.keyframe === frame));
  if (!matches.length) throw new Error(`No mark matches ${query}${frame === undefined ? '' : ` on keyframe #${frame}`}`);
  if (matches.length > 1 && frame === undefined) throw new Error(`Mark ${query} appears on multiple keyframes; specify --frame`);
  return matches[0];
}

async function assertNoSymlinkPath(path: string) {
  const parts = resolve(path).split(sep).filter(Boolean); let current: string = sep;
  for (const part of parts) {
    current = join(current, part);
    const info = await import('node:fs/promises').then(fs => fs.lstat(current)).catch((e: NodeJS.ErrnoException) => e.code === 'ENOENT' ? null : Promise.reject(e));
    if (info?.isSymbolicLink()) throw new Error('Zoom path contains a symlink');
  }
}

type PngInfo = { width:number;height:number;bytes:number;hash:string;canonical:string;dev:bigint;ino:bigint;size:bigint;mtimeNs:bigint };
function preflightPng(bytes: Buffer): {width:number;height:number} {
  if(bytes.length<45 || !bytes.subarray(0,8).equals(signature)) throw new Error('Saved frame is not a complete PNG');
  let offset=8, seenHeader=false, seenData=false, endedData=false, seenEnd=false, chunks=0;
  let width=0,height=0;
  while(offset<bytes.length) {
    if(++chunks>4096 || offset+12>bytes.length) throw new Error('Malformed PNG chunk bounds');
    const length=bytes.readUInt32BE(offset), type=bytes.toString('ascii',offset+4,offset+8), end=offset+12+length;
    if(end>bytes.length || end<offset) throw new Error('Malformed PNG chunk bounds');
    if(!seenHeader && type!=='IHDR') throw new Error('PNG must begin with IHDR');
    if(type==='IHDR') {
      if(seenHeader || offset!==8 || length!==13) throw new Error('PNG must contain exactly one valid IHDR');
      seenHeader=true;width=bytes.readUInt32BE(offset+8);height=bytes.readUInt32BE(offset+12);
      const bitDepth=bytes[offset+16],colorType=bytes[offset+17],interlace=bytes[offset+20];
      if(!width||!height||width*height>MAX_IMAGE_PIXELS||width>100_000||height>100_000) throw new Error('Saved PNG dimensions exceed safety limits');
      if(interlace!==0) throw new Error('Interlaced PNG is unsupported by bounded zoom decoding');
      if(![0,2,4,6].includes(colorType)||![1,2,4,8].includes(bitDepth)) throw new Error('Unsupported PNG format or bit depth');
    } else if(type==='IDAT') { if(endedData) throw new Error('PNG IDAT chunks must be contiguous'); seenData=true; }
    else if(seenData && type!=='IEND') endedData=true;
    if(type==='acTL'||type==='fcTL'||type==='fdAT') throw new Error('Animated PNG is unsupported');
    if(type==='IEND') { if(length!==0 || seenEnd || end!==bytes.length) throw new Error('Malformed PNG IEND'); seenEnd=true; break; }
    offset=end;
  }
  if(!seenHeader||!seenData||!seenEnd) throw new Error('PNG is missing required chunks');
  return {width,height};
}

async function inspectPng(path: string, bundlePath: string, providerOutput = false): Promise<PngInfo> {
  await assertNoSymlinkPath(path);
  const initial=await lstat(path,{bigint:true});
  if(!initial.isFile()||initial.isSymbolicLink()||initial.size<45n||initial.size>BigInt(MAX_FILE_BYTES)) throw new Error('Saved PNG exceeds file size limits');
  const canonical = await realpath(path);
  const framesRoot = `${join(bundlePath, 'frames')}${sep}`;
  if (!providerOutput && !canonical.startsWith(framesRoot) && !canonical.startsWith(`${join(bundlePath, 'frames-full')}${sep}`)) throw new Error('Saved frame escaped bundle image directories');
  if(canonical!==resolve(path)) throw new Error('PNG path changed during validation');
  const file = await open(canonical, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOFOLLOW);
  try {
    const before = await file.stat({ bigint: true });
    if (!before.isFile() || before.dev!==initial.dev || before.ino!==initial.ino || before.size!==initial.size || before.size > BigInt(MAX_FILE_BYTES)) throw new Error('PNG source identity changed before reading');
    const bytes = Buffer.alloc(Number(before.size));
    let offset = 0;
    while (offset < bytes.length) { const read = await file.read(bytes, offset, bytes.length - offset, offset); if (!read.bytesRead) throw new Error('Saved PNG changed while reading'); offset += read.bytesRead; }
    const after = await file.stat({ bigint: true });
    const pathAfter=await lstat(path,{bigint:true}), canonicalAfter=await realpath(path);
    if (before.dev !== after.dev || before.ino !== after.ino || before.size !== after.size || before.mtimeNs !== after.mtimeNs || pathAfter.dev!==before.dev || pathAfter.ino!==before.ino || canonicalAfter!==canonical) throw new Error('PNG source identity changed while reading');
    const dimensions=preflightPng(bytes);
    return { ...dimensions,bytes:bytes.length,hash:createHash('sha256').update(bytes).digest('hex'),canonical,dev:before.dev,ino:before.ino,size:before.size,mtimeNs:before.mtimeNs };
  } finally { await file.close(); }
}

export function makeDecoder() {
  const worker=new Worker(new URL('./png-worker.js',import.meta.url),{resourceLimits:{maxOldGenerationSizeMb:96,maxYoungGenerationSizeMb:24,stackSizeMb:4}});
  type Decoded={width:number;height:number;data:ArrayBuffer};
  const pending=new Map<number,{resolve:(value:Decoded)=>void;reject:(error:Error)=>void;timer:NodeJS.Timeout}>();
  let serial=0, failure:Error|undefined;
  const fail=(error:Error)=>{
    failure=error;
    for(const request of pending.values()){clearTimeout(request.timer);request.reject(error);}
    pending.clear();
  };
  worker.on('error',()=>fail(new Error('PNG worker failed within its resource limits')));
  worker.on('exit',()=>fail(new Error('PNG worker exited before decode completed')));
  worker.on('message',(message:any)=>{
    const request=pending.get(message.id);if(!request)return;
    pending.delete(message.id);clearTimeout(request.timer);
    message.error?request.reject(new Error(message.error)):request.resolve(message);
  });
  return {worker,decode(bytes:Buffer,crop:Crop,timeoutMs:number):Promise<Decoded> {
    if(failure)return Promise.reject(failure);
    const id=++serial;
    return new Promise((resolve,reject)=>{
      const timer=setTimeout(()=>{
        fail(new Error('PNG worker exceeded zoom deadline'));
        void worker.terminate();
      },Math.max(1,timeoutMs));
      pending.set(id,{resolve,reject,timer});
      try{worker.postMessage({id,bytes:bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength),crop,maxWidth:CELL_W,maxHeight:CELL_H});}
      catch{fail(new Error('PNG worker could not accept decode input'));}
    });
  }};
}

function relativeBundlePath(bundlePath:string,path:string):string|undefined { const prefix=`${bundlePath}${sep}`; return path.startsWith(prefix)?path.slice(prefix.length).split(sep).join('/'):undefined; }
async function readVerifiedBytes(path:string,bundlePath:string,provider:boolean,expected:PngInfo):Promise<Buffer>{
  await assertNoSymlinkPath(path);const canonical=await realpath(path);if(canonical!==expected.canonical||canonical!==resolve(path))throw new Error('PNG source identity changed before decode');
  if(!provider&&!canonical.startsWith(`${join(bundlePath,'frames')}${sep}`)&&!canonical.startsWith(`${join(bundlePath,'frames-full')}${sep}`))throw new Error('Saved frame escaped bundle image directories');
  const initial=await lstat(path,{bigint:true});if(initial.dev!==expected.dev||initial.ino!==expected.ino||initial.size!==expected.size)throw new Error('PNG source identity changed before decode');
  const file=await open(canonical,constants.O_RDONLY|constants.O_NONBLOCK|constants.O_NOFOLLOW);
  try{const before=await file.stat({bigint:true});if(before.dev!==expected.dev||before.ino!==expected.ino||before.size!==expected.size||before.mtimeNs!==expected.mtimeNs)throw new Error('PNG source identity changed before decode');
    const bytes=Buffer.alloc(Number(before.size));let offset=0;while(offset<bytes.length){const result=await file.read(bytes,offset,bytes.length-offset,offset);if(!result.bytesRead)throw new Error('PNG source shortened during decode');offset+=result.bytesRead;}
    const after=await file.stat({bigint:true}),pathAfter=await lstat(path,{bigint:true});if(after.dev!==expected.dev||after.ino!==expected.ino||after.size!==expected.size||after.mtimeNs!==expected.mtimeNs||pathAfter.dev!==expected.dev||pathAfter.ino!==expected.ino||await realpath(path)!==canonical)throw new Error('PNG source identity changed while reading');
    const dimensions=preflightPng(bytes);if(dimensions.width!==expected.width||dimensions.height!==expected.height||createHash('sha256').update(bytes).digest('hex')!==expected.hash)throw new Error('PNG content changed after preflight');return bytes;
  }finally{await file.close();}
}

type Crop = [number, number, number, number];
function cropFor(bundle: any, frame: any, at: number, full: boolean, focusedMark?: any): Crop {
  if (full || !bundle.autocrop) return [0,0,1,1];
  if (!bundle.keyframes.length) return [0,0,1,1];
  let index = focusedMark?.frame ? bundle.keyframes.findIndex((item: any) => item.file === focusedMark.frame) : -1;
  if (index < 0 && focusedMark?.keyframe) index = bundle.keyframes.findIndex((item: any) => item.index === focusedMark.keyframe);
  if (index < 0) index = bundle.keyframes.reduce((best: number, item: any, i: number) => Math.abs((item.t ?? 0) - at) < Math.abs((bundle.keyframes[best]?.t ?? 0) - at) ? i : best, 0);
  const crop = bundle.autocrop.crops[index];
  const bounds=bundle.target?.frame, fw=Number(bounds?.width),fh=Number(bounds?.height);
  if(!Number.isFinite(fw)||!Number.isFinite(fh)||fw<=0||fh<=0||fw>100_000||fh>100_000) throw new Error('Invalid capture dimensions for crop mapping');
  if(!crop||crop.kind==='full') return [0,0,1,1];
  const values=[crop.x,crop.y,crop.width,crop.height].map(Number);
  if(values.some(v=>!Number.isFinite(v))||values[2]<1||values[3]<1||values[0]<0||values[1]<0||values[0]+values[2]>fw+1||values[1]+values[3]>fh+1) throw new Error('Invalid capture-point autocrop bounds');
  return [Math.max(0,values[0]/fw),Math.max(0,values[1]/fh),Math.min(1,(values[0]+values[2])/fw),Math.min(1,(values[1]+values[3])/fh)];
}
function contextualMarks(bundle:any,at:number,selected:any,frame:any){
  const entries=new Map<string,any>(markEntries(bundle).map((entry:any)=>[entry.id,entry]));
  const selectedEntry=selected?entries.get(selected.id):undefined,selectedFrame=selected?.frame??bundle.keyframes.find((k:any)=>k.index===selectedEntry?.keyframe)?.file;
  return bundle.marks.filter((m:any)=>selected?m.id===selected.id||selectedFrame&&m.frame===selectedFrame:(m.t0<=at&&at-m.t1<=1200||frame?.file&&m.frame===frame.file))
    .map((m:any)=>{const entry:any=entries.get(m.id);return{id:m.id,t0:m.t0,t1:m.t1,shape:m.shape,description:m.description,on:m.on??null,frame:m.frame??null,keyframe:entry?.keyframe??null,number:entry?.number??null,numbered:entry?.numbered??false,pin:m.pin??null};});
}

function paintPixel(image: any, x: number, y: number, color = [238, 48, 68, 220]) {
  if (x < 0 || y < 0 || x >= image.width || y >= image.height) return;
  const p = (Math.floor(y) * image.width + Math.floor(x)) * 4; image.data.set(color, p);
}
function disk(image:any,cx:number,cy:number,radius:number,color:number[],budget:RasterBudget){
  const r=Math.ceil(radius),work=(2*r+1)**2;budget.steps+=work;if(budget.steps>2_000_000)throw new Error('Zoom annotation raster work limit exceeded');
  for(let y=-r;y<=r;y++)for(let x=-r;x<=r;x++)if(x*x+y*y<=radius*radius)paintPixel(image,cx+x,cy+y,color);
}
const DIGITS:Record<string,string[]>={
  '0':['111','101','101','101','111'],'1':['010','110','010','010','111'],'2':['111','001','111','100','111'],
  '3':['111','001','111','001','111'],'4':['101','101','111','001','001'],'5':['111','100','111','001','111'],
  '6':['111','100','111','101','111'],'7':['111','001','010','010','010'],'8':['111','101','111','101','111'],'9':['111','101','111','001','111']};
function pinMarker(image:any,x:number,y:number,number:number,budget:RasterBudget,color=[238,48,68]){
  const radius=9.75;disk(image,x,y,11.625,[255,255,255,255],budget);disk(image,x,y,radius,[...color,255],budget);
  const digits=String(number),scale=Math.min(1,14/(digits.length*4-1)),width=(digits.length*4-1)*scale,height=5*scale;
  budget.steps+=digits.length*15;if(budget.steps>2_000_000)throw new Error('Zoom annotation raster work limit exceeded');
  for(let d=0;d<digits.length;d++){const glyph=DIGITS[digits[d]];for(let row=0;row<5;row++)for(let col=0;col<3;col++)if(glyph[row][col]==='1'){
    const left=Math.round(x-width/2+(d*4+col)*scale),top=Math.round(y-height/2+row*scale);
    for(let py=0;py<Math.max(1,Math.round(scale));py++)for(let px=0;px<Math.max(1,Math.round(scale));px++)paintPixel(image,left+px,top+py,[255,255,255,255]);
  }}
}
function eventsAt(events:any[],t:number){return events.map(e=>{
  if(e.kind!=='drag'||!(e.t<=t&&t<e.t1)||!Array.isArray(e.path)||e.path.length<2)return e;
  const n=e.path.length,at=(i:number)=>e.t+Math.floor((e.t1-e.t)*i/(n-1));
  const done=Array.from({length:n},(_,i)=>i).filter(i=>at(i)<=t),last=done[done.length-1]??0;
  let kept=done.filter(i=>at(i)+350>=t).map(i=>e.path[i]);if(!kept.length)kept=[e.path[last]];
  return {...e,t:Math.max(e.t,t-350),t1:t,x0:kept[0][0],y0:kept[0][1],x1:kept[kept.length-1][0],y1:kept[kept.length-1][1],path:kept};
});}
function line(image: any, x0: number, y0: number, x1: number, y1: number, radius = 2, color=[238,48,68,220],budget:RasterBudget={steps:0}) {
  if (![x0,y0,x1,y1].every(Number.isFinite)) throw new Error('Invalid annotation geometry');
  // Liang–Barsky clipping prevents hostile far-away coordinates from creating unbounded loops.
  const dx=x1-x0,dy=y1-y0,p=[-dx,dx,-dy,dy],q=[x0,image.width-1-x0,y0,image.height-1-y0];let lo=0,hi=1;
  for(let i=0;i<4;i++){if(p[i]===0){if(q[i]<0)return;continue;}const t=q[i]/p[i];if(p[i]<0)lo=Math.max(lo,t);else hi=Math.min(hi,t);if(lo>hi)return;}
  const ax=x0+lo*dx,ay=y0+lo*dy,bx=x0+hi*dx,by=y0+hi*dy;
  x0=ax;y0=ay;x1=bx;y1=by;
  const n = Math.max(1, Math.min(2048, Math.ceil(Math.hypot(x1 - x0, y1 - y0))));
  budget.steps+=n;if(budget.steps>2_000_000)throw new Error('Zoom annotation raster work limit exceeded');
  for (let i = 0; i <= n; i++) { const x = x0 + (x1-x0)*i/n, y = y0 + (y1-y0)*i/n;
    for (let dy = -radius; dy <= radius; dy++) for (let dx = -radius; dx <= radius; dx++) if (dx*dx+dy*dy <= radius*radius) paintPixel(image,x+dx,y+dy,color); }
}
export function annotateZoomFrame(image: any, bundle: any, at: number, rect: Crop, mark?: any,budget:RasterBudget={steps:0}) {
  const point = (x: number, y: number) => [((x-rect[0])/(rect[2]-rect[0]))*image.width, ((y-rect[1])/(rect[3]-rect[1]))*image.height];
  const selected=mark&&bundle.marks.find((item:any)=>item.id===mark.id),focused=selected&&at>=selected.t0?selected:undefined, owned=new Set<number>(focused?.strokes??[]);
  const active=bundle.strokes.map((s:any,i:number)=>({s,i})).filter(({s,i}:any)=>!s.laser&&(owned.has(i)||(!focused&&s.t0<=at&&at-s.t1<=1200)));
  for(const {s,i} of active){
    const color: number[] = s.rgb ?? [238,48,68];
    const pts:number[][]=[]; for(let j=0;j<s.pts.length;j+=3){const t=s.t0+s.pts[j+2];if(!owned.has(i)&&t>at)break;pts.push(point(s.pts[j]/10000,s.pts[j+1]/10000));}
    if(s.pin!==undefined&&s.pin!==null&&pts.length){const [x,y]=pts[0];pinMarker(image,x,y,s.pin,budget,color);}
    else for(let j=1;j<pts.length;j++)line(image,pts[j-1][0],pts[j-1][1],pts[j][0],pts[j][1],2,[...color,230],budget);
  }
  for(const s of bundle.strokes.filter((stroke:any)=>stroke.laser)){
    const color: number[] = s.rgb ?? [255,32,50];
    const pts:number[][]=[];for(let j=0;j<s.pts.length;j+=3){const t=s.t0+s.pts[j+2];if(t>at||at-t>420)continue;pts.push(point(s.pts[j]/10000,s.pts[j+1]/10000));}
    for(let j=1;j<pts.length;j++)line(image,pts[j-1][0],pts[j-1][1],pts[j][0],pts[j][1],1,[...color,150],budget);
  }
  const events=eventsAt(bundle.events,at),clicks=events.map((e:any)=>e.kind==='click'&&Math.abs(e.t-at)<=600?{delta:Math.abs(e.t-at),point:[e.x,e.y]}:e.kind==='drag'&&(e.label==='long press'||e.label?.startsWith('long press:'))&&((e.t<=at&&at-e.t1<=600)||Math.abs(e.t-at)<=600)?{delta:Math.abs(e.t1-at),point:[e.x0,e.y0]}:undefined).filter(Boolean).sort((a:any,b:any)=>a.delta-b.delta);
  for(const e of events){if(e.kind==='drag'&&e.button==='touch'&&e.t<=at&&at-e.t1<=900&&!(e.label==='long press'||e.label?.startsWith('long press:'))){const path=e.path?.length>=2?e.path:[[e.x0,e.y0],[e.x1,e.y1]];let prev:number[]|undefined;for(const p of path){const q=point(p[0],p[1]);if(prev)line(image,prev[0],prev[1],q[0],q[1],2,[45,142,255,220],budget);prev=q;}}}
  if(clicks.length){const [x,y]=point(clicks[0]!.point[0],clicks[0]!.point[1]);for(let d=0;d<360;d+=4){const r=d*Math.PI/180;paintPixel(image,x+Math.cos(r)*8,y+Math.sin(r)*8,[45,142,255,220]);}}
}

function composite(images: Array<{ image: any; at: number }>) {
  const columns = Math.min(4, images.length), rows = Math.ceil(images.length / columns);
  const width = columns * CELL_W + (columns + 1) * GAP, height = rows * CELL_H + (rows + 1) * GAP;
  if (width * height > MAX_TOTAL_PIXELS) throw new Error('Zoom sheet exceeds aggregate pixel limit');
  const sheet = new PNG({ width, height });
  for (let i = 0; i < width * height; i++) { const p = i*4; sheet.data[p]=25; sheet.data[p+1]=27; sheet.data[p+2]=31; sheet.data[p+3]=255; }
  for (const [i, item] of images.entries()) {
    const scale = Math.min(CELL_W/item.image.width, CELL_H/item.image.height), w=Math.max(1,Math.round(item.image.width*scale)), h=Math.max(1,Math.round(item.image.height*scale));
    const ox=GAP+(i%columns)*(CELL_W+GAP)+Math.floor((CELL_W-w)/2), oy=GAP+Math.floor(i/columns)*(CELL_H+GAP)+Math.floor((CELL_H-h)/2);
    for(let y=0;y<h;y++) for(let x=0;x<w;x++) {
      const sx=Math.min(item.image.width-1,Math.floor(x/scale)), sy=Math.min(item.image.height-1,Math.floor(y/scale));
      const si=(sy*item.image.width+sx)*4, di=((oy+y)*width+ox+x)*4; item.image.data.copy(sheet.data,di,si,si+4);
    }
  }
  return { sheet, columns, rows };
}

async function outputDirectory(requested?: string) {
  const requestedRoot = requested ? resolve(requested) : join(homedir(), '.cache', 'apparent', 'agent', 'zoom');
  if (requested && (!isAbsolute(requested) || requestedRoot !== requested)) throw new Error('--out must be a canonical absolute directory path');
  await assertNoSymlinkPath(dirname(requestedRoot));
  await mkdir(dirname(requestedRoot), { recursive: true, mode: 0o700 });
  const parent = await realpath(dirname(requestedRoot));
  if (parent !== dirname(requestedRoot)) throw new Error('Zoom output parent must be canonical');
  if (!requested) await mkdir(requestedRoot, { recursive: true, mode: 0o700 });
  const root = requested ? requestedRoot : join(requestedRoot, `zoom-${Date.now()}-${randomBytes(6).toString('hex')}`);
  await mkdir(root, { mode: 0o700 });
  try{
    const info = await (await import('node:fs/promises')).lstat(root);
    if (!info.isDirectory() || info.isSymbolicLink() || (typeof process.getuid === 'function' && info.uid !== process.getuid()) || (info.mode & 0o077)) throw new Error('Zoom output must be a private owner directory');
  }catch(error){await rmdir(root).catch(()=>undefined);throw error;}
  return root;
}

export async function runZoom(options: { bundle: string; at?: string; mark?: string; frame?: number; span: number; frames: number; full: boolean; out?: string; json: boolean }) {
  const span = options.span;
  if (!Number.isFinite(span) || span <= 0 || !Number.isSafeInteger(Math.floor(span*1000))) throw new Error('--span must be positive and within the safe timestamp range');
  if (!Number.isSafeInteger(options.frames) || options.frames < 0) throw new Error('--frames must be a nonnegative safe integer');
  if ((options.at === undefined) === (options.mark === undefined)) throw new Error('Pass exactly one of --at or --mark');
  if (options.frame !== undefined && options.mark === undefined) throw new Error('--frame requires --mark');
  const bundlePath = await resolveBundle(options.bundle);
  await assertNoSymlinkPath(bundlePath);
  const { value: bundle } = await readManifest(bundlePath);
  const mark = options.mark === undefined ? undefined : pickMark(bundle, options.mark, options.frame);
  let at: number;
  let bounds: [number,number];
  if (mark) { at = Math.floor((mark.t0+mark.t1)/2); bounds = markWindow(mark.t0,mark.t1,Math.floor(span*1000),bundle.durationMs); }
  else { at = resolveMoment(bundle, options.at!); if (at > bundle.durationMs) throw new Error(`${clock(at)} is past the end (${clock(bundle.durationMs)})`); bounds=zoomWindow(at,Math.floor(span*1000),bundle.durationMs); }
  const times = sampleTimes(bounds[0],bounds[1],options.frames);
  if(bundle.strokes.length>MAX_STROKES||bundle.marks.length>MAX_MARKS||bundle.events.length>MAX_EVENTS) throw new Error('Bundle annotation inventory exceeds zoom limits');
  let pointCount=0;
  for(const s of bundle.strokes){if(Object.prototype.hasOwnProperty.call(s,'rgb')&&(!Array.isArray(s.rgb)||s.rgb.length!==3||s.rgb.some((channel:number)=>!Number.isInteger(channel)||channel<0||channel>255)))throw new Error('Malformed stroke RGB in zoom bundle');if(!Array.isArray(s.pts)||s.pts.length%3!==0||s.pts.length>MAX_POINTS*3||!Number.isSafeInteger(s.fw)||!Number.isSafeInteger(s.fh)||s.fw<=0||s.fh<=0||s.fw>100_000||s.fh>100_000||!Number.isSafeInteger(s.t0)||!Number.isSafeInteger(s.t1)||s.t1<s.t0||s.pin!==undefined&&s.pin!==null&&(!Number.isSafeInteger(s.pin)||s.pin<1||s.pin>0xffffffff))throw new Error('Malformed stroke in zoom bundle');pointCount+=s.pts.length/3;if(pointCount>MAX_POINTS)throw new Error('Bundle stroke points exceed zoom render limits');let prior=-1;for(let i=0;i<s.pts.length;i+=3){if(!Number.isSafeInteger(s.pts[i])||s.pts[i]<0||s.pts[i]>10000||!Number.isSafeInteger(s.pts[i+1])||s.pts[i+1]<0||s.pts[i+1]>10000||!Number.isSafeInteger(s.pts[i+2])||s.pts[i+2]<prior||s.pts[i+2]>s.t1-s.t0+1000)throw new Error('Malformed stroke point in zoom bundle');prior=s.pts[i+2];}}
  for(const m of bundle.marks)if(!Array.isArray(m.strokes)||m.strokes.some((i:number)=>!Number.isSafeInteger(i)||i<0||i>=bundle.strokes.length)||!Number.isSafeInteger(m.t0)||!Number.isSafeInteger(m.t1)||m.t1<m.t0||m.pin!==undefined&&m.pin!==null&&(!Number.isSafeInteger(m.pin)||m.pin<1||m.pin>0xffffffff)||!Array.isArray(m.boxNorm)||m.boxNorm.length!==4||m.boxNorm.some((v:number)=>!Number.isFinite(v)||v<0||v>1)||[m.from,m.to].filter(Boolean).some((p:any)=>!Array.isArray(p)||p.length!==2||p.some((v:number)=>!Number.isFinite(v)||v<0||v>1)))throw new Error('Malformed mark in zoom bundle');
  let eventPoints=0;for(const e of bundle.events){if(e.kind==='drag'&&e.path!==undefined&&(!Array.isArray(e.path)||e.path.length>MAX_POINTS||e.path.some((p:any)=>!Array.isArray(p)||p.length!==2))||e.kind!=='drag'&&e.path!==undefined)throw new Error('Malformed interaction path in zoom bundle');eventPoints+=e.kind==='drag'?(e.path?.length??0):0;if(eventPoints>MAX_POINTS)throw new Error('Interaction paths exceed zoom render limits');const coords=e.kind==='drag'?[e.x0,e.y0,e.x1,e.y1,...(e.path??[]).flat()]:['click','point','scroll','note'].includes(e.kind)?[e.x,e.y]:[];if(coords.some((v:any)=>typeof v!=='number'||!Number.isFinite(v)||v<0||v>1))throw new Error('Malformed interaction coordinates in zoom bundle');if(e.t!==undefined&&!Number.isSafeInteger(e.t)||e.t1!==undefined&&!Number.isSafeInteger(e.t1)||Number.isSafeInteger(e.t1)&&Number.isSafeInteger(e.t)&&e.t1<e.t)throw new Error('Malformed interaction time range in zoom bundle');}
  const executable=process.env.APPARENT_ENGINE_FRAMES_EXECUTABLE, runtime=process.env.APPARENT_ENGINE_RUNTIME;
  let refs: Array<{timestampMs:number;path:string;effectiveVideoMs?:number;actualVideoMs?:number;clampedToStart?:boolean;clampedToEnd?:boolean}>, sourceKind: 'decoded-video'|'saved-keyframes'; let jobId: string|undefined;
  if (!executable && !runtime) {
    const saved=await savedKeyframes(bundlePath,{fromMs:bounds[0],toMs:bounds[1],fps:4,requestedBundleMs:times});
    refs=saved.frames; sourceKind='saved-keyframes';
  } else {
    if (!executable || !runtime || !isAbsolute(executable) || resolve(executable)!==executable || !isAbsolute(runtime) || resolve(runtime)!==runtime)
      throw new Error('Set canonical absolute APPARENT_ENGINE_FRAMES_EXECUTABLE and APPARENT_ENGINE_RUNTIME paths');
    const videoAvailable=await hasSourceVideo(bundlePath);
    if (!videoAvailable) {
      const saved=await savedKeyframes(bundlePath,{fromMs:bounds[0],toMs:bounds[1],fps:4,requestedBundleMs:times}); refs=saved.frames; sourceKind='saved-keyframes';
    } else {
    await verifyFramesExecutable(executable); await verifyPrivateRuntime(runtime);
    const { MediaClient, launchMediaWorker } = await import('@apparent/engine-sdk/media');
    const client=new MediaClient(launchMediaWorker(executable,runtime,3000));
    let primary:unknown;
    let cancelled=false;const cancel=()=>{cancelled=true;};
    process.on('SIGINT',cancel);process.on('SIGTERM',cancel);
    try {
      const hello=await client.handshake();
      if (hello.implementation === 'test-double') throw new Error('Production zoom refuses the Engine SDK test double');
      if (!hello.capabilities.decodeFrames) {
        const saved=await savedKeyframes(bundlePath,{fromMs:bounds[0],toMs:bounds[1],fps:4,requestedBundleMs:times}); refs=saved.frames; sourceKind='saved-keyframes';
      } else {
        const result=await decodeFrameTimes(client,bundlePath,runtime,times,()=>cancelled);
        refs=result.frames.map(f=>({timestampMs:f.requestedBundleMs,path:f.path,effectiveVideoMs:f.effectiveVideoMs,actualVideoMs:f.actualVideoMs,clampedToStart:f.clampedToStart,clampedToEnd:f.clampedToEnd})); jobId=result.jobId; sourceKind='decoded-video';
      }
    } catch(error){primary=error;throw error;} finally {
      process.off('SIGINT',cancel);process.off('SIGTERM',cancel);
      const closed=await client.close();
      if (!['already-exited','eof-exited'].includes(closed)&&!primary) throw new Error(`Media worker shutdown uncertain (${closed}); retain provider job output for diagnosis`);
    }
    }
  }
  if (!refs.length) throw new Error(sourceKind === 'saved-keyframes' ? 'No saved keyframes match this window; video decoding is unavailable' : 'No frames returned');
  if (sourceKind === 'saved-keyframes' && bundle.autocrop) {
    const mapped=[] as typeof refs;
    for(const ref of refs) {
      const frame=bundle.keyframes.find((k:any)=>k.t===ref.timestampMs&&k.file===relativeBundlePath(bundlePath,ref.path));
      if(!frame) { mapped.push(ref); continue; }
      const fullRef=`${bundle.autocrop.fullFrames}/${frame.file.slice('frames/'.length)}`;
      const candidate=join(bundlePath,fullRef);
      if(!safeReference(fullRef)) throw new Error('Bundle contains an unsafe full-frame reference');
      const info=await lstat(candidate).catch((e:NodeJS.ErrnoException)=>e.code==='ENOENT'?null:Promise.reject(e));
      if(info?.isSymbolicLink()) throw new Error('Full-frame reference follows a symlink');
      mapped.push(info?.isFile()?{...ref,path:candidate}:ref);
    }
    refs=mapped;
  }
  if (refs.length > MAX_FRAMES) throw new Error(`Zoom frame inventory exceeds ${MAX_FRAMES}`);
  const deadline=Date.now()+MAX_RENDER_MS,infos:PngInfo[]=[];let totalBytes=0,totalPixels=0;
  for(const ref of refs){const info=await inspectPng(ref.path,bundlePath,sourceKind==='decoded-video');totalBytes+=info.bytes;totalPixels+=info.width*info.height;if(totalBytes>MAX_TOTAL_INPUT_BYTES)throw new Error('Zoom source PNGs exceed aggregate byte limit');if(totalPixels>MAX_TOTAL_PIXELS)throw new Error('Zoom source PNGs exceed aggregate pixel limit');infos.push(info);}
  const images:any[]=[],cellMappings:Array<{crop:Crop;appliedCrop:Crop;cropApplied:boolean;fullRequested:boolean;fullFrameAvailable:boolean;sourceFrameKind:string;annotations:string}> = [],rasterBudget:RasterBudget={steps:0};
  const decoder=makeDecoder();
  try{for(const [index,ref] of refs.entries()){
    if(Date.now()>deadline)throw new Error('Zoom composition exceeded its 60 second deadline');
    const rel=relativeBundlePath(bundlePath,ref.path),isFullPath=!!rel?.startsWith(`${bundle.autocrop?.fullFrames}/`);
    const frameRel=isFullPath?`frames/${rel!.slice(`${bundle.autocrop.fullFrames}/`.length)}`:rel;
    const exactFrame=bundle.keyframes.find((k:any)=>k.file===frameRel);
    const nearest=exactFrame??(bundle.keyframes.length?bundle.keyframes.reduce((best:any,k:any)=>Math.abs(k.t-ref.timestampMs)<Math.abs(best.t-ref.timestampMs)?k:best,bundle.keyframes[0]):undefined);
    const cropMapping=cropFor(bundle,nearest,ref.timestampMs,false,mark);
    const fullFrame=sourceKind==='decoded-video'||isFullPath||!bundle.autocrop;
    const rect:Crop=sourceKind==='saved-keyframes'&&!fullFrame?[0,0,1,1]:options.full?[0,0,1,1]:cropMapping;
    const bytes=await readVerifiedBytes(ref.path,bundlePath,sourceKind==='decoded-video',infos[index]);
    const decoded=await decoder.decode(bytes,rect,deadline-Date.now());if(Date.now()>deadline)throw new Error('Zoom composition exceeded its 60 second deadline');
    const image=new PNG({width:decoded.width,height:decoded.height});image.data=Buffer.from(decoded.data);
    const annotationRect=sourceKind==='decoded-video'?rect:cropMapping;
    if(sourceKind==='decoded-video')annotateZoomFrame(image,bundle,ref.timestampMs,annotationRect,mark,rasterBudget);
    images.push({image,at:ref.timestampMs});
    cellMappings.push({crop:cropMapping,appliedCrop:rect,cropApplied:rect.some((v:number,i:number)=>v!==[0,0,1,1][i]),fullRequested:options.full,fullFrameAvailable:fullFrame,
      sourceFrameKind:sourceKind==='decoded-video'?'decoded-video-frame':isFullPath?'full-frame':bundle.autocrop?'autocropped-keyframe':'saved-keyframe',annotations:sourceKind==='saved-keyframes'?'already baked into retained source':'rendered from bundle strokes and interactions'});
  }}finally{await decoder.worker.terminate();}
  const {sheet,columns,rows}=composite(images);
  if(Date.now()>deadline)throw new Error('Zoom composition exceeded its 60 second deadline');
  const output=await outputDirectory(options.out);
  const sheetPath=join(output,'zoom.png'), sidecarPath=join(output,'zoom.json');
  let sheetMade=false,sidecarMade=false;
  try {
    const png=PNG.sync.write(sheet,{colorType:6});
    const fd=await open(sheetPath,constants.O_WRONLY|constants.O_CREAT|constants.O_EXCL|constants.O_NOFOLLOW,0o600);
    sheetMade=true;
    try { await fd.writeFile(png); await fd.sync(); } finally { await fd.close(); }
    const words=(bundle.transcript.words??[]).filter((w:any)=>w.t1>=bounds[0]&&w.t0<=bounds[1]).map((w:any)=>w.text);
    const said=words.length?words.join(' '):bundle.transcript.segments.filter((s:any)=>s.t1>=bounds[0]&&s.t0<=bounds[1]).map((s:any)=>s.text.trim()).join(' ');
    const metadata={bundle:bundlePath,sheet:sheetPath,sourceKind,source:sourceKind==='decoded-video'?'engine-media-sdk':'bundle-manifest', ...(jobId?{jobId}:{}),window:{fromMs:bounds[0],toMs:bounds[1],from:clock(bounds[0]),to:clock(bounds[1])},layout:{order:'row-major',columns,rows,cellWidth:CELL_W,cellHeight:CELL_H},cells:images.map((image,index)=>{
      const relative=relativeBundlePath(bundlePath,refs[index].path),full=relative?.startsWith(`${bundle.autocrop?.fullFrames}/`),framePath=full?`frames/${relative!.slice(`${bundle.autocrop.fullFrames}/`.length)}`:relative;
      const owning=bundle.keyframes.find((k:any)=>k.file===framePath)??bundle.keyframes.reduce((best:any,k:any)=>!best||Math.abs(k.t-image.at)<Math.abs(best.t-image.at)?k:best,undefined);
      return{index:index+1,row:Math.floor(index/columns)+1,column:index%columns+1,timestampMs:image.at,time:clock(image.at),framePath:refs[index].path,...(sourceKind==='decoded-video'?{decode:{effectiveVideoMs:refs[index].effectiveVideoMs,actualVideoMs:refs[index].actualVideoMs,clampedToStart:refs[index].clampedToStart,clampedToEnd:refs[index].clampedToEnd}}:{}),...cellMappings[index],
        marks:contextualMarks(bundle,image.at,mark,owning),interactions:eventsAt(bundle.events,image.at).filter((e:any)=>e.kind==='click'?Math.abs(e.t-image.at)<=600:e.kind==='drag'?e.t<=image.at&&image.at-e.t1<=900||Math.abs(e.t-image.at)<=600:false).map((e:any)=>e)};
    }),said,narration:{words:(bundle.transcript.words??[]).filter((w:any)=>w.t1>=bounds[0]&&w.t0<=bounds[1]),segments:bundle.transcript.segments.filter((s:any)=>s.t1>=bounds[0]&&s.t0<=bounds[1])},mark:mark??null};
    const json=Buffer.from(JSON.stringify(metadata,null,2)+'\n');if(json.length>16*1024*1024)throw new Error('Zoom semantic sidecar exceeds size limit');
    const side=await open(sidecarPath,constants.O_WRONLY|constants.O_CREAT|constants.O_EXCL|constants.O_NOFOLLOW,0o600);
    sidecarMade=true;
    try { await side.writeFile(json); await side.sync(); } finally { await side.close(); }
    return metadata;
  } catch(error) {
    const failures:string[]=[];
    if(sheetMade)await unlink(sheetPath).catch(()=>failures.push('zoom.png'));
    if(sidecarMade)await unlink(sidecarPath).catch(()=>failures.push('zoom.json'));
    await rmdir(output).catch(()=>failures.push('directory not empty or no longer owned'));
    const state=failures.length?`owned output retained at ${output} (${failures.join(', ')})`:`owned output ${output} cleaned`;
    throw new Error(`Zoom composition failed; ${state}: ${error instanceof Error?error.message:String(error)}`);
  }
}
