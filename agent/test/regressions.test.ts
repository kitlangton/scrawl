import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { mkdtemp, mkdir, writeFile, symlink, rm, stat, chmod, truncate } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { markEntries, markLine } from '../src/inspection.ts';
import { retainRequest, loadRequest } from '../src/ownership.ts';
import { recordWithEngine, type RecordingClient } from '../src/recording.ts';
import type { StartRequest } from '@apparent/engine-sdk';

const manifest = () => ({version:1,id:'fixture',createdAtMs:1,durationMs:2000,
  target:{mode:'screen',app:null,bundleId:null,windowTitle:null,windowId:null,displayId:1,frame:{x:0,y:0,width:800,height:600,scale:1}},
  transcript:{source:'local',text:'Inspect this control.',segments:[{t0:0,t1:2000,text:'Inspect this control.'}]},
  strokes:[],marks:[],events:[],keyframes:[],contactSheet:'contact-sheet.png'});
const cli = new URL('../dist/cli.js', import.meta.url).pathname;

test('legacy optional omissions and producer summary work; symlink/oversized summaries are refused', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-producer-'));
  try {
    const bundle = join(root,'fixture.apparent'); await mkdir(bundle);
    const data = manifest(); const raw=JSON.stringify(data,null,2);
    await writeFile(join(bundle,'apparent.json'),raw);
    await writeFile(join(root,'outside'),'private outside content');
    await symlink(join(root,'outside'),join(bundle,'summary.md'));
    let result=spawnSync(process.execPath,[cli,'inspect',bundle],{encoding:'utf8'});
    assert.notEqual(result.status,0); assert.equal(result.stdout,''); assert(!result.stderr.includes('private outside content'));
    await rm(join(bundle,'summary.md'));
    const summary='[apparent 0:02 · Screen · '+bundle+']\nOpen contact-sheet.png and frames/.\n';
    await writeFile(join(bundle,'summary.md'),summary);
    result=spawnSync(process.execPath,[cli,'inspect',bundle],{encoding:'utf8'});
    assert.equal(result.status,0); assert.equal(result.stdout,summary);
    result=spawnSync(process.execPath,[cli,'inspect',join(bundle,'apparent.json'),'--json'],{encoding:'utf8'});
    assert.equal(result.status,0); assert.equal(result.stdout,raw);
    await truncate(join(bundle,'summary.md'),64*1024*1024+1);
    result=spawnSync(process.execPath,[cli,'inspect',bundle],{encoding:'utf8'});
    assert.notEqual(result.status,0); assert.equal(result.stdout,'');
    await symlink(join(root,'outside'),join(bundle,'contact-sheet.png'));
    result=spawnSync(process.execPath,[cli,'inspect',bundle,'--images'],{encoding:'utf8'});
    assert.notEqual(result.status,0); assert.equal(result.stdout,'');
  } finally { await rm(root,{recursive:true,force:true}); }
});

test('mark entries match Rust shape/label/chronological and shared-frame numbering rules', () => {
  const a={id:'arrow',t0:300,t1:400,strokes:[0,1],shape:'arrow',description:'geometric description',on:' Button ',frame:'frames/1.png'};
  const b={id:'circle',t0:100,t1:200,strokes:[2],shape:'circle',description:'coordinates',on:'<button>',frame:'frames/1.png'};
  const flag={id:'flag',t0:500,t1:500,strokes:[],shape:'flag',description:'coordinates',frame:'frames/1.png'};
  const entries=markEntries({marks:[a,flag,b],keyframes:[{file:'frames/1.png',index:1,markId:'arrow'}]});
  assert.deepEqual(entries.map((e:any)=>[e.id,e.number,e.numbered,e.text]),[
    ['circle',1,true,'circled <button>'],['arrow',2,true,'drew an arrow pointing to “Button” (2 strokes)'],['flag',1,false,'flagged this moment']]);
  assert.equal(markLine(entries[1]),'#1 ✎② 0:00 drew an arrow pointing to “Button” (2 strokes) · arrow');
});

test('relative config is skipped; Linux XDG default is respected without changing storage', async () => {
  const root=await mkdtemp(join(tmpdir(),'agent-storage-'));
  try {
    const data=join(root,'data'); const bundle=join(data,'apparent','captures','fixture.apparent');
    await mkdir(bundle,{recursive:true}); await mkdir(join(root,'.config','apparent'),{recursive:true});
    await writeFile(join(root,'.config','apparent','config.json'),JSON.stringify({capturesDir:'relative',secret:'untouched'}));
    await writeFile(join(bundle,'apparent.json'),JSON.stringify(manifest())); await writeFile(join(bundle,'summary.md'),'default');
    await writeFile(join(bundle,'contact-sheet.png'),'synthetic image');
    const env={...process.env,HOME:root,XDG_DATA_HOME:data}; delete env.APPARENT_CAPTURES_DIR;
    if(process.platform==='linux') {
      const result=spawnSync(process.execPath,[cli,'inspect','latest'],{env,encoding:'utf8',cwd:root});
      assert.equal(result.status,0,result.stderr); assert.equal(result.stdout,'default\n');
    }
  } finally {await rm(root,{recursive:true,force:true});}
});

test('private ownership is durable before start, never overwritten, and rejects exposed records', async () => {
  const root=await mkdtemp(join(tmpdir(),'agent-ownership-')); await chmod(root,0o700);
  try {
    const request={protocolVersion:1,method:'start',clientId:'c'.repeat(32),rpcId:'d'.repeat(32),captureId:'a'.repeat(32),ownerToken:'b'.repeat(64),bundleId:'fixture',bundlePath:'/tmp/fixture.apparent',options:{screen:true}} as StartRequest;
    const file=await retainRequest(root,request);
    assert.equal((await stat(file)).mode & 0o777,0o600);
    assert.deepEqual(await loadRequest(root,request.captureId),request);
    await assert.rejects(retainRequest(root,request));
    await chmod(file,0o644); await assert.rejects(loadRequest(root,request.captureId),/private recovery/);
    await assert.rejects(loadRequest(root,'../outside'),/capture ID/);
  } finally {await rm(root,{recursive:true,force:true});}
});

test('startup delay does not consume requested duration and request persistence precedes ambiguous start', async () => {
  const calls:string[]=[]; let readyAt=0; let stoppedAt=0; let stopping=false;
  const ready={protocolVersion:1,implementation:'native',readiness:'ready',capabilities:{screenCapture:true,audioCapture:false,windowCapture:false}};
  const client={async handshake(){return ready;},prepareStart(){return {captureId:'a'.repeat(32),ownerToken:'b'.repeat(64)};},
    async start(){calls.push('start');return {captureId:'a'.repeat(32),ownerToken:'b'.repeat(64),receipt:{state:'starting'}};},
    async control(method:string){
      if(method==='stop'){stopping=true;stoppedAt=performance.now();return {state:'stopping'};}
      if(stopping)return {state:'saved',bundlePath:'/tmp/fixture.apparent',artifactVersion:1,artifactProvenance:'native'};
      readyAt ||= performance.now(); return {state:'recording'};
    }} as unknown as RecordingClient;
  assert.equal(await recordWithEngine(client,'/tmp/fixture.apparent',0.05,()=>false,async()=>{calls.push('persist');}),'/tmp/fixture.apparent');
  assert.deepEqual(calls,['persist','start']); assert(stoppedAt-readyAt>=45);
  const broken={...client,async start(){assert.equal(calls.at(-1),'persist');throw new Error('ambiguous start');}} as RecordingClient;
  await assert.rejects(recordWithEngine(broken,'/tmp/fixture.apparent',1,()=>false,async()=>{calls.push('persist');}),/ambiguous start/);
});

test('existing Rust inspection fixtures preserve current/legacy raw JSON, summaries and mark semantics',async()=>{
  const {readFile,copyFile}=await import('node:fs/promises');
  const root=await mkdtemp(join(tmpdir(),'agent-compatibility-'));
  try {
    for(const name of ['current','legacy']) {
      const bundle=join(root,`${name}.apparent`);await mkdir(bundle);
      const fixture=(file:string)=>new URL(`./fixtures/${file}`,import.meta.url);
      await copyFile(fixture(`${name}.json`),join(bundle,'apparent.json'));
      await copyFile(fixture(`${name}.md`),join(bundle,'summary.md'));
      const run=(args:string[])=>spawnSync(process.execPath,[cli,'inspect',bundle,...args],{encoding:'utf8'});
      const raw=run(['--json']);assert.equal(raw.status,0,raw.stderr);assert.equal(raw.stdout,await readFile(fixture(`${name}.json`),'utf8'));
      const text=run([]);assert.equal(text.status,0,text.stderr);assert.equal(text.stdout,await readFile(fixture(`${name}.md`),'utf8'));
      if(name==='current') {
        const marks=run(['--marks','--json']);assert.equal(marks.status,0,marks.stderr);
        assert.deepEqual(JSON.parse(marks.stdout),JSON.parse(await readFile(fixture('current.marks.json'),'utf8')));
      }
    }
  }finally{await rm(root,{recursive:true,force:true});}
});
