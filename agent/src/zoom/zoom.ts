import { constants } from 'node:fs';
import { lstat, mkdir, open, realpath, rmdir, unlink } from 'node:fs/promises';
import { homedir } from 'node:os';
import { basename, dirname, isAbsolute, join, resolve, sep } from 'node:path';
import { randomBytes } from 'node:crypto';
import { PNG } from 'pngjs';
import { readManifest, resolveBundle, safeReference } from '../bundle.js';
import { decodeFrameTimes, hasSourceVideo, savedKeyframes, verifyFramesExecutable, verifyPrivateRuntime } from '../frames.js';
import { markEntries } from '../inspection.js';

const MAX_FRAMES = 48, MAX_FILE_BYTES = 16 * 1024 * 1024, MAX_IMAGE_PIXELS = 16_000_000;
const MAX_TOTAL_INPUT_BYTES = 64 * 1024 * 1024, MAX_TOTAL_PIXELS = 64_000_000;
const CELL_W = 480, CELL_H = 320, GAP = 6;
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
    if (parts.length !== 2 || !/^\d+$/.test(parts[0]) || !/^[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?$/.test(parts[1])) throw new Error('invalid time');
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
  const matches = entries.filter((entry: any) => entry.id === query || numeric !== undefined && entry.number === numeric &&
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

async function readPng(path: string, bundlePath: string, providerOutput = false) {
  await assertNoSymlinkPath(path);
  const canonical = await realpath(path);
  const framesRoot = `${join(bundlePath, 'frames')}${sep}`;
  if (!providerOutput && !canonical.startsWith(framesRoot) && !canonical.startsWith(`${join(bundlePath, 'frames-full')}${sep}`)) throw new Error('Saved frame escaped bundle image directories');
  const file = await open(canonical, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOFOLLOW);
  try {
    const before = await file.stat({ bigint: true });
    if (!before.isFile() || before.size < 24n || before.size > BigInt(MAX_FILE_BYTES)) throw new Error('Saved PNG exceeds file size limits');
    const bytes = Buffer.alloc(Number(before.size));
    let offset = 0;
    while (offset < bytes.length) { const read = await file.read(bytes, offset, bytes.length - offset, offset); if (!read.bytesRead) throw new Error('Saved PNG changed while reading'); offset += read.bytesRead; }
    const after = await file.stat({ bigint: true });
    if (before.dev !== after.dev || before.ino !== after.ino || before.size !== after.size || before.mtimeNs !== after.mtimeNs) throw new Error('Saved PNG changed while reading');
    if (!bytes.subarray(0, 8).equals(signature)) throw new Error('Saved frame is not a PNG');
    const width = bytes.readUInt32BE(16), height = bytes.readUInt32BE(20);
    if (!width || !height || width * height > MAX_IMAGE_PIXELS) throw new Error('Saved PNG dimensions exceed safety limits');
    return { image: PNG.sync.read(bytes, { checkCRC: true }), bytes: bytes.length, path: canonical };
  } finally { await file.close(); }
}

type Crop = [number, number, number, number];
function cropFor(bundle: any, frame: any, at: number, full: boolean, focusedMark?: any): Crop {
  if (full || !bundle.autocrop) return [0,0,1,1];
  if (!bundle.keyframes.length) return [0,0,1,1];
  let index = focusedMark?.frame ? bundle.keyframes.findIndex((item: any) => item.file === focusedMark.frame) : -1;
  if (index < 0 && focusedMark?.keyframe) index = bundle.keyframes.findIndex((item: any) => item.index === focusedMark.keyframe);
  if (index < 0) index = bundle.keyframes.reduce((best: number, item: any, i: number) => Math.abs((item.t ?? 0) - at) < Math.abs((bundle.keyframes[best]?.t ?? 0) - at) ? i : best, 0);
  const crop = bundle.autocrop.crops[index];
  return crop && crop.kind !== 'full' ? [Math.max(0,crop.x), Math.max(0,crop.y), Math.min(1,crop.x+crop.width), Math.min(1,crop.y+crop.height)] : [0,0,1,1];
}

function cropImage(source: any, rect: Crop) {
  const left = Math.max(0, Math.min(source.width - 1, Math.floor(rect[0] * source.width)));
  const top = Math.max(0, Math.min(source.height - 1, Math.floor(rect[1] * source.height)));
  const right = Math.max(left + 1, Math.min(source.width, Math.ceil(rect[2] * source.width)));
  const bottom = Math.max(top + 1, Math.min(source.height, Math.ceil(rect[3] * source.height)));
  const width = right - left, height = bottom - top;
  const scale = Math.min(1, 1280 / Math.max(width, height));
  const outW = Math.max(1, Math.round(width * scale)), outH = Math.max(1, Math.round(height * scale));
  const out = new PNG({ width: outW, height: outH });
  for (let y = 0; y < outH; y++) for (let x = 0; x < outW; x++) {
    const sx = left + Math.min(width - 1, Math.floor(x / scale)), sy = top + Math.min(height - 1, Math.floor(y / scale));
    const si = (sy * source.width + sx) * 4, di = (y * outW + x) * 4;
    source.data.copy(out.data, di, si, si + 4);
  }
  return out;
}

function paintPixel(image: any, x: number, y: number, color = [238, 48, 68, 220]) {
  if (x < 0 || y < 0 || x >= image.width || y >= image.height) return;
  const p = (Math.floor(y) * image.width + Math.floor(x)) * 4; image.data.set(color, p);
}
function line(image: any, x0: number, y0: number, x1: number, y1: number, radius = 2, color=[238,48,68,220]) {
  const n = Math.max(1, Math.ceil(Math.hypot(x1 - x0, y1 - y0)));
  for (let i = 0; i <= n; i++) { const x = x0 + (x1-x0)*i/n, y = y0 + (y1-y0)*i/n;
    for (let dy = -radius; dy <= radius; dy++) for (let dx = -radius; dx <= radius; dx++) if (dx*dx+dy*dy <= radius*radius) paintPixel(image,x+dx,y+dy,color); }
}
function annotate(image: any, bundle: any, at: number, rect: Crop, mark?: any) {
  const point = (x: number, y: number) => [((x-rect[0])/(rect[2]-rect[0]))*image.width, ((y-rect[1])/(rect[3]-rect[1]))*image.height];
  const marks = bundle.marks.filter((m: any) => at >= m.t0 && (m.t0 <= at || m.id === mark?.id));
  for(const m of marks) for(const strokeIndex of m.strokes??[]) {
    const stroke=bundle.strokes[strokeIndex]; if(!stroke||!Array.isArray(stroke.pts)||stroke.pts.length<6) continue;
    const pts:number[][]=[];
    for(let i=0;i+2<stroke.pts.length;i+=3) {
      const t=stroke.t0+stroke.pts[i+2]; if(t>at) break;
      pts.push(point(stroke.pts[i]/10000,stroke.pts[i+1]/10000));
    }
    for(let i=1;i<pts.length;i++) line(image,pts[i-1][0],pts[i-1][1],pts[i][0],pts[i][1],2,[238,48,68,230]);
  }
  for (const m of marks) {
    const [x0,y0,x1,y1] = m.boxNorm.map(Number); const a=point(x0,y0), b=point(x1,y1);
    line(image,a[0],a[1],b[0],a[1]); line(image,b[0],a[1],b[0],b[1]); line(image,b[0],b[1],a[0],b[1]); line(image,a[0],b[1],a[0],a[1]);
  }
  for (const event of bundle.events) if (event.kind === 'click' && event.t <= at && at-event.t <= 350) {
    const [cx,cy] = point(event.x,event.y); const radius=8;
    for (let deg=0;deg<360;deg+=2) { const rad=deg*Math.PI/180; paintPixel(image,cx+Math.cos(rad)*radius,cy+Math.sin(rad)*radius,[238,48,68,190]); }
  }
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
  const info = await (await import('node:fs/promises')).lstat(root);
  if (!info.isDirectory() || info.isSymbolicLink() || (typeof process.getuid === 'function' && info.uid !== process.getuid()) || (info.mode & 0o077)) throw new Error('Zoom output must be a private owner directory');
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
  const executable=process.env.APPARENT_ENGINE_FRAMES_EXECUTABLE, runtime=process.env.APPARENT_ENGINE_RUNTIME;
  let refs: Array<{timestampMs:number;path:string}>, sourceKind: 'decoded-video'|'saved-keyframes'; let jobId: string|undefined;
  if (!executable && !runtime) {
    const saved=await savedKeyframes(bundlePath,{fromMs:bounds[0],toMs:bounds[1],fps:4,requestedBundleMs:times});
    refs=saved.frames; sourceKind='saved-keyframes';
  } else {
    if (!executable || !runtime || !isAbsolute(executable) || resolve(executable)!==executable || !isAbsolute(runtime) || resolve(runtime)!==runtime)
      throw new Error('Set canonical absolute APPARENT_ENGINE_FRAMES_EXECUTABLE and APPARENT_ENGINE_RUNTIME paths');
    await verifyFramesExecutable(executable); await verifyPrivateRuntime(runtime);
    if (!await hasSourceVideo(bundlePath)) throw new Error('Bundle source video is missing; configured provider errors are authoritative');
    const { MediaClient, launchMediaWorker } = await import('@apparent/engine-sdk/media');
    const client=new MediaClient(launchMediaWorker(executable,runtime,3000));
    try {
      const hello=await client.handshake();
      if (hello.implementation === 'test-double') throw new Error('Production zoom refuses the Engine SDK test double');
      if (!hello.capabilities.decodeFrames) {
        const saved=await savedKeyframes(bundlePath,{fromMs:bounds[0],toMs:bounds[1],fps:4,requestedBundleMs:times}); refs=saved.frames; sourceKind='saved-keyframes';
      } else {
        const result=await decodeFrameTimes(client,bundlePath,runtime,times);
        refs=result.frames.map(f=>({timestampMs:f.requestedBundleMs,path:f.path})); jobId=result.jobId; sourceKind='decoded-video';
      }
    } finally {
      const closed=await client.close();
      if (!['already-exited','eof-exited'].includes(closed)) throw new Error(`Media worker shutdown uncertain (${closed}); retain provider job output for diagnosis`);
    }
  }
  if (!refs.length) throw new Error(sourceKind === 'saved-keyframes' ? 'No saved keyframes match this window; video decoding is unavailable' : 'No frames returned');
  if (sourceKind === 'saved-keyframes' && bundle.autocrop) {
    const mapped=[] as typeof refs;
    for(const ref of refs) {
      const frame=bundle.keyframes.find((k:any)=>k.t===ref.timestampMs&&typeof k.file==='string'&&k.file.startsWith('frames/'));
      if(!frame) { mapped.push(ref); continue; }
      const candidate=join(bundlePath,bundle.autocrop.fullFrames,basename(frame.file));
      if(!safeReference(`${bundle.autocrop.fullFrames}/${basename(frame.file)}`)) throw new Error('Bundle contains an unsafe full-frame reference');
      const info=await lstat(candidate).catch((e:NodeJS.ErrnoException)=>e.code==='ENOENT'?null:Promise.reject(e));
      if(info?.isSymbolicLink()) throw new Error('Full-frame reference follows a symlink');
      mapped.push(info?.isFile()?{...ref,path:candidate}:ref);
    }
    refs=mapped;
  }
  if (refs.length > MAX_FRAMES) throw new Error(`Zoom frame inventory exceeds ${MAX_FRAMES}`);
  let totalBytes=0,totalPixels=0; const images=[]; const cellMappings:Array<{crop:Crop;appliedCrop:Crop;cropApplied:boolean;fullRequested:boolean;fullFrameAvailable:boolean;sourceFrameKind:string}> = [];
  for (const [index,ref] of refs.entries()) {
    totalBytes += (await (await import('node:fs/promises')).stat(ref.path)).size;
    if(totalBytes>MAX_TOTAL_INPUT_BYTES) throw new Error('Zoom source PNGs exceed aggregate byte limit');
    const decoded=await readPng(ref.path,bundlePath,sourceKind==='decoded-video'); totalPixels+=decoded.image.width*decoded.image.height;
    if(totalPixels>MAX_TOTAL_PIXELS) throw new Error('Zoom source PNGs exceed aggregate pixel limit');
    const nearest=bundle.keyframes.length?bundle.keyframes.reduce((best:any,k:any)=>Math.abs(k.t-ref.timestampMs)<Math.abs(best.t-ref.timestampMs)?k:best,bundle.keyframes[0]):undefined;
    const cropMapping=cropFor(bundle,nearest,ref.timestampMs,false,mark);
    const fullFrame=sourceKind==='decoded-video'||refs[index].path.includes(`${sep}frames-full${sep}`)||!bundle.autocrop;
    const rect=options.full||sourceKind==='saved-keyframes'&&!fullFrame?[0,0,1,1] as Crop:cropMapping;
    const annotationRect=sourceKind==='saved-keyframes'&&!fullFrame?cropMapping:rect;
    const image=cropImage(decoded.image,rect); annotate(image,bundle,ref.timestampMs,annotationRect,mark);
    images.push({image,at:ref.timestampMs});
    const isFullPath=refs[index].path.includes(`${sep}frames-full${sep}`);
    cellMappings.push({crop:cropMapping,appliedCrop:rect, cropApplied:rect.some((v:number,i:number)=>v!==[0,0,1,1][i]),fullRequested:options.full,
      fullFrameAvailable:sourceKind==='decoded-video'||isFullPath||!bundle.autocrop,
      sourceFrameKind:sourceKind==='decoded-video'?'decoded-video-frame':isFullPath?'full-frame':bundle.autocrop?'autocropped-keyframe':'saved-keyframe'});
  }
  const {sheet,columns,rows}=composite(images);
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
    const metadata={bundle:bundlePath,sheet:sheetPath,sourceKind,source:sourceKind==='decoded-video'?'engine-media-sdk':'bundle-manifest', ...(jobId?{jobId}:{}),window:{fromMs:bounds[0],toMs:bounds[1],from:clock(bounds[0]),to:clock(bounds[1])},layout:{order:'row-major',columns,rows,cellWidth:CELL_W,cellHeight:CELL_H},cells:images.map((image,index)=>({index:index+1,row:Math.floor(index/columns)+1,column:index%columns+1,timestampMs:image.at,time:clock(image.at),framePath:refs[index].path,...cellMappings[index],
      marks:bundle.marks.filter((m:any)=>m.t0<=image.at).map((m:any)=>({id:m.id,t0:m.t0,t1:m.t1,shape:m.shape,description:m.description,on:m.on??null})),
      interactions:bundle.events.filter((e:any)=>(e.t??e.t0)<=image.at+350&&(e.t1??e.t??e.t0)>=image.at-350).map((e:any)=>e)})),said,narration:{words:(bundle.transcript.words??[]).filter((w:any)=>w.t1>=bounds[0]&&w.t0<=bounds[1]),segments:bundle.transcript.segments.filter((s:any)=>s.t1>=bounds[0]&&s.t0<=bounds[1])},mark:mark??null};
    const json=Buffer.from(JSON.stringify(metadata,null,2)+'\n');
    const side=await open(sidecarPath,constants.O_WRONLY|constants.O_CREAT|constants.O_EXCL|constants.O_NOFOLLOW,0o600);
    sidecarMade=true;
    try { await side.writeFile(json); await side.sync(); } finally { await side.close(); }
    return metadata;
  } catch(error) {
    if(sheetMade) await unlink(sheetPath).catch(()=>undefined); if(sidecarMade) await unlink(sidecarPath).catch(()=>undefined);
    await rmdir(output).catch(()=>undefined);
    throw new Error(`Zoom composition failed; owned output directory ${output} was cleaned when empty: ${error instanceof Error?error.message:String(error)}`);
  }
}
