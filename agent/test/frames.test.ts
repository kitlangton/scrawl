import { mkdtemp, mkdir, chmod, writeFile, readFile, rm, symlink, appendFile, realpath } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { MediaClient, type MediaRequest, type MediaResponse, type MediaTransport } from '@apparent/engine-sdk/media';
import { ReferenceMediaPeer, type MediaTestScenario } from '@apparent/engine-sdk/media/testing';
import { decodeFrames, hasSourceVideo, parseClockMs, savedKeyframes, selectFrameTimes } from '../dist/frames.js';

const target = { mode:'display', app:null, bundleId:null, windowTitle:null, windowId:null, displayId:0, frame:{x:0,y:0,width:10,height:10,scale:1} };
const pngHeader = (w: number, h: number) => { const b=Buffer.alloc(24); Buffer.from('89504e470d0a1a0a','hex').copy(b); b.writeUInt32BE(w,16); b.writeUInt32BE(h,20); return b; };

class MaterializingPeer implements MediaTransport {
  readonly peer: ReferenceMediaPeer;
  readonly requests: MediaRequest[] = [];
  private readonly scenario: MediaTestScenario;
  constructor(scenario: MediaTestScenario = 'success') { this.scenario = scenario; this.peer = new ReferenceMediaPeer(scenario); }
  async request(request: MediaRequest, timeoutMs = 5000): Promise<MediaResponse> {
    this.requests.push(request);
    let response: MediaResponse;
    if (request.method === 'decode' && this.peer && (request as any).method === 'decode') {
      // The test peer's timeout scenario leaves the job alive but lets this RPC expire.
      response = await this.peer.request(request, this.scenario === 'timeout' ? Math.min(timeoutMs, 20) : timeoutMs);
    } else response = await this.peer.request(request, timeoutMs);
    const result: any = response.result;
    if (result?.state === 'complete' && Array.isArray(result.frames)) {
      for (const frame of result.frames) {
        const bytes = pngHeader(frame.width, frame.height);
        await mkdir(join(frame.path, '..'), { recursive: true });
        await writeFile(frame.path, bytes);
        frame.byteLength = bytes.length;
        frame.sha256 = createHash('sha256').update(bytes).digest('hex');
      }
    }
    return response;
  }
  close() { return this.peer.close(); }
}

async function fixture(t: (root: string, bundle: string, runtime: string, video: string) => Promise<void>) {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'agent-frames-')));
  try {
    const bundle = join(root, 'clip.apparent'), runtime = join(root, 'runtime'), video = join(bundle, 'recording.mov');
    await mkdir(bundle); await mkdir(runtime, { mode: 0o700 }); await chmod(runtime, 0o700);
    const manifest = { version:1,id:'clip',createdAtMs:1,durationMs:5000,target,transcript:{text:'',segments:[]},strokes:[],marks:[],events:[],keyframes:[],contactSheet:'contact.png',video:'recording.mov',videoOffsetMs:500 };
    await writeFile(join(bundle,'apparent.json'), JSON.stringify(manifest)); await writeFile(video, Buffer.from('synthetic video bytes'));
    await t(root,bundle,runtime,video);
  } finally { await rm(root,{recursive:true,force:true}); }
}

test('clock parser and range selection are integer bounded, ordered, and reject oversized ranges', () => {
  assert.equal(parseClockMs(' 1:02.345 '), 62345); assert.equal(parseClockMs('1.25s'),1250); assert.equal(parseClockMs('125ms'),125);
  assert.deepEqual(selectFrameTimes('0','1',4).requestedBundleMs,[0,250,500,750]);
  assert.throws(()=>selectFrameTimes('2','1',4),/ends before/);
  assert.throws(()=>selectFrameTimes('0','13',4),/48-frame/);
  assert.throws(()=>selectFrameTimes('0','1',1.5),/integer/);
  assert.throws(()=>parseClockMs('9007199254740992ms'),/invalid time/);
  assert.deepEqual(selectFrameTimes('1','1',4).requestedBundleMs,[1000]);
});

test('saved keyframe selection is inclusive, ordered, deduplicated, safe and keeps manifest timestamps', async () => fixture(async (_root,bundle,_runtime,video) => {
  const frames = join(bundle, 'frames'); await mkdir(frames);
  const values = [
    {t:1250,file:'frames/a space.png'}, {t:1500,file:'frames/dup.png'},
    {t:1000,file:'frames/dup.png'}, {t:2000,file:'frames/z.png'}, {t:2126,file:'frames/outside.png'}
  ];
  for (const value of values) await writeFile(join(bundle, value.file), pngHeader(2,2));
  const manifest = JSON.parse(await readFile(join(bundle,'apparent.json'),'utf8')); manifest.keyframes=values;
  await writeFile(join(bundle,'apparent.json'),JSON.stringify(manifest)); await rm(video);
  const result=await savedKeyframes(bundle,selectFrameTimes('1','2',4));
  assert.deepEqual(result.frames.map(frame=>frame.timestampMs),[1000,1250,2000]);
  assert.match(result.frames[1].path,/a space\.png$/);
}));

test('saved fallback rejects unsafe or corrupt references and empty selections', async () => fixture(async (_root,bundle,_runtime) => {
  const frames=join(bundle,'frames'); await mkdir(frames);
  const manifest=JSON.parse(await readFile(join(bundle,'apparent.json'),'utf8'));
  manifest.keyframes=[{t:100,file:'frames/good.png'},{t:100,file:'../outside.png'},{t:100,file:'frames/bad.png'},{t:100,file:'frames/missing.png'}];
  await writeFile(join(bundle,'apparent.json'),JSON.stringify(manifest));
  await writeFile(join(frames,'good.png'),pngHeader(1,1)); await writeFile(join(frames,'bad.png'),Buffer.alloc(24));
  const result=await savedKeyframes(bundle,selectFrameTimes('0.1','0.1',4));
  assert.equal(result.frames.length,1); assert.equal(result.frames[0].timestampMs,100);
  await writeFile(join(bundle,'.media-pending'),'');
  await assert.rejects(()=>savedKeyframes(bundle,selectFrameTimes('0.1','0.1',4)),/media is still pending/);
  await rm(join(bundle,'.media-pending'));
  const fifo=join(frames,'fifo.png'); assert.equal(spawnSync('mkfifo',[fifo]).status,0);
  manifest.keyframes.push({t:100,file:'frames/fifo.png'}); await writeFile(join(bundle,'apparent.json'),JSON.stringify(manifest));
  assert.equal((await savedKeyframes(bundle,selectFrameTimes('0.1','0.1',4))).frames.length,1);
  await symlink(join(frames,'good.png'),join(frames,'linked.png'));
  manifest.keyframes.push({t:100,file:'frames/linked.png'}); await writeFile(join(bundle,'apparent.json'),JSON.stringify(manifest));
  await assert.rejects(()=>savedKeyframes(bundle,selectFrameTimes('0.1','0.1',4)),/symlink/);
  await writeFile(join(bundle,'apparent.json'),'{');
  await assert.rejects(()=>savedKeyframes(bundle,selectFrameTimes('0.1','0.1',4)),/not valid JSON/);
}));

test('saved fallback rejects oversized matches instead of truncating and ignores non-PNG suffixes', async () => fixture(async (_root,bundle,_runtime) => {
  const frames=join(bundle,'frames'); await mkdir(frames);
  const manifest=JSON.parse(await readFile(join(bundle,'apparent.json'),'utf8'));
  manifest.keyframes=[];
  for(let i=0;i<49;i++) {
    const file=`frames/${String(i).padStart(2,'0')}.png`; manifest.keyframes.push({t:100,file}); await writeFile(join(bundle,file),pngHeader(1,1));
  }
  manifest.keyframes.push({t:100,file:'frames/not-png.dat'}); await writeFile(join(bundle,'frames/not-png.dat'),pngHeader(1,1));
  // A later unsafe reference must never be inspected once the valid limit is exceeded.
  manifest.keyframes.push({t:100,file:'frames/late-symlink.png'});
  await symlink(join(bundle,'frames/00.png'),join(bundle,'frames/late-symlink.png'));
  await writeFile(join(bundle,'apparent.json'),JSON.stringify(manifest));
  await assert.rejects(()=>savedKeyframes(bundle,selectFrameTimes('0.1','0.1',4)),/exceeds the 48-frame limit/);
}));

test('media adapter preserves requested order and duplicates and reports offset saturation', async () => fixture(async (_root,bundle,runtime) => {
  const transport = new MaterializingPeer(); const client = new MediaClient(transport,{allowTestDouble:true});
  const result = await decodeFrames(client,bundle,runtime,{fromMs:0,toMs:1400,fps:4,requestedBundleMs:[1200,0,700,1200]},()=>false,{allowTestDouble:true});
  const decode = transport.requests.find((r)=>r.method==='decode') as Extract<MediaRequest,{method:'decode'}>;
  assert.deepEqual(decode.requestedBundleMs,[1200,0,700,1200]);
  assert.deepEqual(result.frames.map(f=>f.requestedBundleMs),[1200,0,700,1200]);
  assert.equal(result.frames[1].effectiveVideoMs,0); assert.equal(result.frames[1].clampedToStart,true);
  assert.equal(result.frames[3].requestIndex,4);
  assert.equal((await readFile(result.frames[0].path)).length,result.frames[0].byteLength);
  await client.close();
}));

test('RPC timeout recovers complete inventory through inspect on the same connection', async () => fixture(async (_root,bundle,runtime) => {
  const peer = new MaterializingPeer('timeout');
  const transport: MediaTransport = { request: (req, timeoutMs) => peer.request(req, req.method === 'decode' ? 20 : timeoutMs), close: () => peer.close() };
  const client = new MediaClient(transport,{allowTestDouble:true});
  const result = await decodeFrames(client,bundle,runtime,{fromMs:0,toMs:500,fps:2,requestedBundleMs:[0]},()=>false,{allowTestDouble:true});
  assert.equal(result.frames.length,1); assert.ok(peer.requests.some((r)=>r.method==='inspect'));
  await client.close();
}));

test('adapter retains SDK-validated end-clamp and actual decoder position', async () => fixture(async (_root,bundle,runtime) => {
  const peer = new MaterializingPeer();
  const transport: MediaTransport = {
    async request(request, timeoutMs) {
      const response = await peer.request(request, timeoutMs);
      const result: any = response.result;
      if (result?.state === 'complete') {
        result.frames[0].clampedToEnd = true;
        result.frames[0].actualVideoMs = 1000;
      }
      return response;
    }, close: () => peer.close(),
  };
  const client = new MediaClient(transport, {allowTestDouble:true});
  try {
    const result = await decodeFrames(client,bundle,runtime,{fromMs:0,toMs:3000,fps:1,requestedBundleMs:[2500]},()=>false,{allowTestDouble:true});
    assert.equal(result.frames[0].effectiveVideoMs, 2000);
    assert.equal(result.frames[0].actualVideoMs, 1000);
    assert.equal(result.frames[0].clampedToEnd, true);
  } finally { await client.close(); }
}));

test('test double is rejected by default and explicitly accepted only in injected harness', async () => fixture(async (_root,bundle,runtime) => {
  const client = new MediaClient(new MaterializingPeer());
  await assert.rejects(()=>decodeFrames(client,bundle,runtime,{fromMs:0,toMs:1,fps:1,requestedBundleMs:[0]}),/explicit test-only opt-in/);
  await client.close();
}));

test('source and output failures retain diagnostic job output and do not publish paths', async () => fixture(async (_root,bundle,runtime,video) => {
  const client = new MediaClient(new MaterializingPeer('source-mismatch'),{allowTestDouble:true});
  await assert.rejects(()=>decodeFrames(client,bundle,runtime,{fromMs:0,toMs:1,fps:1,requestedBundleMs:[0]},()=>false,{allowTestDouble:true}),/ended failed/);
  await client.close();
  await rm(video);
  const missing = new MediaClient(new MaterializingPeer(),{allowTestDouble:true});
  await assert.rejects(()=>decodeFrames(missing,bundle,runtime,{fromMs:0,toMs:1,fps:1,requestedBundleMs:[0]},()=>false,{allowTestDouble:true}),/source video is missing/);
  await missing.close();
}));

test('partial failure, cancellation, and output collision never return frame paths', async () => fixture(async (_root,bundle,runtime) => {
  for (const scenario of ['partial-decode-failure','cancelled','output-exists'] as const) {
    const client = new MediaClient(new MaterializingPeer(scenario),{allowTestDouble:true});
    await assert.rejects(()=>decodeFrames(client,bundle,runtime,{fromMs:0,toMs:1,fps:1,requestedBundleMs:[0]},()=>false,{allowTestDouble:true}),/ended (failed|cancelled)/);
    await client.close();
  }
}));

test('media-pending, evidence-only, symlinked media, and changed source are refused', async () => fixture(async (root,bundle,runtime,video) => {
  const options={fromMs:0,toMs:1,fps:1,requestedBundleMs:[0]};
  const makeClient=()=>new MediaClient(new MaterializingPeer(),{allowTestDouble:true});
  await writeFile(join(bundle,'.media-pending'),'');
  let client=makeClient(); await assert.rejects(()=>decodeFrames(client,bundle,runtime,options,()=>false,{allowTestDouble:true}),/media is still pending/); await client.close();
  await rm(join(bundle,'.media-pending'));
  await writeFile(join(bundle,'.evidence-only.json'),'{}');
  client=makeClient(); await assert.rejects(()=>decodeFrames(client,bundle,runtime,options,()=>false,{allowTestDouble:true}),/Evidence-only/); await client.close();
  await rm(join(bundle,'.evidence-only.json'));
  const outside=join(root,'outside.mov'); await writeFile(outside,'outside'); await rm(video); await symlink(outside,video);
  client=makeClient(); await assert.rejects(()=>decodeFrames(client,bundle,runtime,options,()=>false,{allowTestDouble:true}),/symlink/); await client.close();
  await rm(video); await writeFile(video,'synthetic video bytes');
  const peer=new MaterializingPeer();
  const transport: MediaTransport={request:async(req,timeoutMs)=>{const response=await peer.request(req,timeoutMs); if(req.method==='decode') await appendFile(video,' changed'); return response;},close:()=>peer.close()};
  client=new MediaClient(transport,{allowTestDouble:true});
  await assert.rejects(()=>decodeFrames(client,bundle,runtime,options,()=>false,{allowTestDouble:true}),/identity changed/);
  await client.close();
}));

test('valid evidence snapshot is never decoded, even when an unexpected video copy exists', async () => fixture(async (_root,bundle,runtime,video) => {
  await writeFile(join(bundle,'.evidence-only.json'), JSON.stringify({version:1,captureId:'a'.repeat(32),server:'https://connect.example',media:'not downloaded',guidance:'Use apparent fetch <id> --media --out <new-directory> for the complete recording'}));
  assert.equal(await hasSourceVideo(bundle), false);
  const client=new MediaClient(new MaterializingPeer(),{allowTestDouble:true});
  await assert.rejects(()=>decodeFrames(client,bundle,runtime,{fromMs:0,toMs:1,fps:1,requestedBundleMs:[0]},()=>false,{allowTestDouble:true}),/Evidence-only bundle cannot be decoded/);
  await client.close();
  await writeFile(video,'unexpected copy');
  assert.equal(await hasSourceVideo(bundle), false);
  for (const server of ['http://localhost:8787/connect', 'http://127.0.0.1:8080']) {
    await writeFile(join(bundle,'.evidence-only.json'),JSON.stringify({version:1,captureId:'a'.repeat(32),server,media:'not downloaded',guidance:'Use apparent fetch <id> --media --out <new-directory> for the complete recording'}));
    assert.equal(await hasSourceVideo(bundle),false);
  }
  const malformed=join(bundle,'.evidence-only.json'); await writeFile(malformed,'{}');
  await assert.rejects(()=>hasSourceVideo(bundle),/published snapshot format/);
}));

test('corrupt video metadata and exposed runtime directories fail closed', async () => fixture(async (_root,bundle,runtime) => {
  const options={fromMs:0,toMs:1,fps:1,requestedBundleMs:[0]};
  const raw=JSON.parse(await readFile(join(bundle,'apparent.json'),'utf8'));
  raw.videoOffsetMs='500'; await writeFile(join(bundle,'apparent.json'),JSON.stringify(raw));
  let client=new MediaClient(new MaterializingPeer(),{allowTestDouble:true});
  await assert.rejects(()=>decodeFrames(client,bundle,runtime,options,()=>false,{allowTestDouble:true}),/malformed video clock offset/); await client.close();
  raw.videoOffsetMs=500; raw.video='../outside.mov'; await writeFile(join(bundle,'apparent.json'),JSON.stringify(raw));
  client=new MediaClient(new MaterializingPeer(),{allowTestDouble:true});
  await assert.rejects(()=>decodeFrames(client,bundle,runtime,options,()=>false,{allowTestDouble:true}),/unsafe video reference/); await client.close();
  raw.video='recording.mov'; await writeFile(join(bundle,'apparent.json'),JSON.stringify(raw)); await chmod(runtime,0o755);
  client=new MediaClient(new MaterializingPeer(),{allowTestDouble:true});
  await assert.rejects(()=>decodeFrames(client,bundle,runtime,options,()=>false,{allowTestDouble:true}),/owner-only private directory/); await client.close();
}));

test('cancelling acknowledgment remains nonterminal and close uncertainty is reported as a request', async () => fixture(async (root,bundle,runtime) => {
  let jobId='';
  const transport: MediaTransport={
    async request(request) {
      if(request.method==='handshake') return {channel:'media',protocolVersion:1,rpcId:request.rpcId,result:{channel:'media',protocolVersion:1,implementation:'native',capabilities:{decodeFrames:true}}};
      if(request.method==='decode') { jobId=request.jobId; throw new Error('synthetic RPC timeout'); }
      if(request.method==='inspect') return {channel:'media',protocolVersion:1,rpcId:request.rpcId,result:{jobId:request.jobId,state:'cancelling'}};
      return {channel:'media',protocolVersion:1,rpcId:request.rpcId,result:{jobId:request.jobId,state:'cancelling'}};
    },
    async close(){return 'term-requested-after-timeout';}
  };
  const client=new MediaClient(transport);
  await assert.rejects(()=>decodeFrames(client,bundle,runtime,{fromMs:0,toMs:1,fps:1,requestedBundleMs:[0]},()=>true),/remains cancelling.*diagnostic output/);
  assert.match(jobId,/^[0-9a-f]{32}$/);
  assert.equal(await client.close(),'term-requested-after-timeout');
}));

test('CLI with no bundle reports the local bundle error before looking for providers', () => {
  const cli = new URL('../dist/cli.js', import.meta.url).pathname;
  const result = spawnSync(process.execPath,[cli,'frames','./clip.apparent','--from','0','--to','1'],{encoding:'utf8',env:{...process.env,APPARENT_ENGINE_FRAMES_EXECUTABLE:'',APPARENT_ENGINE_RUNTIME:''}});
  assert.equal(result.status,2); assert.equal(result.stdout,''); assert.match(result.stderr,/Local bundle directory not found/);
});

test('installed CLI uses saved keyframes offline when video is removed and marks JSON source', async () => fixture(async (_root,bundle,_runtime,video) => {
  const frames=join(bundle,'frames'); await mkdir(frames);
  await writeFile(join(frames,'saved frame.png'),pngHeader(3,2));
  const manifest=JSON.parse(await readFile(join(bundle,'apparent.json'),'utf8'));
  manifest.keyframes=[{t:1750,file:'frames/saved frame.png'}]; await writeFile(join(bundle,'apparent.json'),JSON.stringify(manifest)); await rm(video);
  const cli=new URL('../dist/cli.js',import.meta.url).pathname;
  const result=spawnSync(process.execPath,[cli,'frames',bundle,'--from','1.7','--to','1.7','--json'],{encoding:'utf8',env:{...process.env,APPARENT_ENGINE_FRAMES_EXECUTABLE:'',APPARENT_ENGINE_RUNTIME:''}});
  assert.equal(result.status,0,result.stderr); assert.equal(result.stderr,'Using saved keyframes; no video was decoded.\n');
  const output=JSON.parse(result.stdout); assert.equal(output.mode,'saved-keyframes'); assert.equal(output.frames[0].timestampMs,1750); assert.match(output.frames[0].path,/saved frame\.png$/);
}));

test('installed CLI uses saved images when the source video is missing even with provider paths configured', async () => fixture(async (_root,bundle,_runtime,video) => {
  const frames=join(bundle,'frames'); await mkdir(frames); await writeFile(join(frames,'saved.png'),pngHeader(2,2));
  const manifest=JSON.parse(await readFile(join(bundle,'apparent.json'),'utf8')); manifest.keyframes=[{t:500,file:'frames/saved.png'}];
  await writeFile(join(bundle,'apparent.json'),JSON.stringify(manifest)); await rm(video);
  const cli=new URL('../dist/cli.js',import.meta.url).pathname;
  const result=spawnSync(process.execPath,[cli,'frames',bundle,'--from','0','--to','0.5'],{encoding:'utf8',env:{...process.env,APPARENT_ENGINE_FRAMES_EXECUTABLE:'/missing/engine',APPARENT_ENGINE_RUNTIME:'/missing/runtime'}});
  assert.equal(result.status,0,result.stderr); assert.match(result.stdout,/saved\.png/); assert.match(result.stderr,/no video was decoded/);
}));

test('installed CLI does not claim extraction when no saved image matches', async () => fixture(async (_root,bundle,_runtime,video) => {
  await rm(video);
  const cli=new URL('../dist/cli.js',import.meta.url).pathname;
  const result=spawnSync(process.execPath,[cli,'frames',bundle,'--from','1','--to','2'],{encoding:'utf8',env:{...process.env,APPARENT_ENGINE_FRAMES_EXECUTABLE:'',APPARENT_ENGINE_RUNTIME:''}});
  assert.equal(result.status,2); assert.equal(result.stdout,''); assert.match(result.stderr,/No saved keyframes match.*video extraction is unavailable/);
}));

test('a frame replaced with a FIFO is rejected without waiting for a writer', {timeout: 2000}, async () => fixture(async (_root,bundle,runtime) => {
  const peer = new MaterializingPeer();
  const transport: MediaTransport = {
    async request(request, timeoutMs) {
      const response = await peer.request(request, timeoutMs);
      const result: any = response.result;
      if (request.method === 'decode' && result?.state === 'complete') {
        const path = result.frames[0].path;
        await rm(path);
        assert.equal(spawnSync('mkfifo', [path]).status, 0);
      }
      return response;
    }, close: () => peer.close(),
  };
  const client = new MediaClient(transport, {allowTestDouble:true});
  try {
    await assert.rejects(() => decodeFrames(client,bundle,runtime,{fromMs:0,toMs:1,fps:1,requestedBundleMs:[0]},()=>false,{allowTestDouble:true}), /bounded returned inventory/);
  } finally { await client.close(); }
}));
