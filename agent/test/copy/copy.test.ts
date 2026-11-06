import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {mkdtemp,mkdir,writeFile,readFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {writeCopyText,localCopyText} from '../../dist/handoff/copy.js';
import type {NativeHostSnapshot} from '@apparent/app-contracts/native';

const snapshot = {processEpoch:'test-epoch',revision:7} as NativeHostSnapshot;
test('copy preflights exact UTF8 before opening Host; no truncation or surrogate substitution',async()=>{
  let calls=0; const ports=()=>{calls++;throw new Error('must not open Host');};
  assert.deepEqual(await writeCopyText('x'.repeat(21801),ports),{status:'refused',reason:'too-large'});
  for(const value of ['nul\0text','\ud800','\udfff']) assert.deepEqual(await writeCopyText(value,ports),{status:'refused',reason:'invalid-text'});
  assert.equal(calls,0);
});
test('copy binds a fresh snapshot and confirms exact digest and UTF8 byte length',async()=>{
  const text='Exact 🖊️ text\n';const expected={digest:createHash('sha256').update(text).digest('hex'),byteLength:Buffer.byteLength(text)};const calls:string[]=[];
  const result=await writeCopyText(text,()=>({host:{readSnapshot:async()=>{calls.push('snapshot');return snapshot;}},clipboard:{writeText:async(value,observed)=>{calls.push('write');assert.equal(value,text);assert.deepEqual(observed,{processEpoch:'test-epoch',revision:7});return{status:'applied',...expected};}}}));
  assert.deepEqual(calls,['snapshot','write']);assert.deepEqual(result,{status:'applied',...expected});
});
test('copy never retries an ambiguous write or a refusal',async()=>{
  for(const status of ['lost','digest','length','stale','busy','unavailable','outcome-unknown'] as const){
    let writes=0;const result=await writeCopyText('text',()=>({host:{readSnapshot:async()=>snapshot},clipboard:{writeText:async()=>{writes++;if(status==='lost')throw new Error('reply lost');if(status==='digest'||status==='length')return{status:'applied',digest:status==='digest'?'0'.repeat(64):createHash('sha256').update('text').digest('hex'),byteLength:status==='length'?3:4};return status==='outcome-unknown'?{status}:{status:'refused',reason:status};}}}));
    assert.equal(writes,1);assert.equal(result.status,['stale','busy','unavailable'].includes(status)?'refused':'outcome-unknown');
  }
  let writes=0;assert.deepEqual(await writeCopyText('text',()=>({host:{readSnapshot:async()=>{throw new Error('no GUI');}},clipboard:{writeText:async()=>{writes++;return{status:'outcome-unknown'};}}})),{status:'refused',reason:'unavailable'});assert.equal(writes,0);
});
test('local copy reuses Record semantics and Agent inspection command without session forwarding or input writes',async()=>{
  const root=await mkdtemp(join(tmpdir(),'agent-copy-'));const bundle=join(root,"a 'quoted'.apparent");await mkdir(bundle);const manifest=JSON.parse(await readFile(new URL('../fixtures/current.json',import.meta.url),'utf8'));
  manifest.destination={session:'private-session',via:'local',title:'DO NOT FORWARD',url:'https://example.invalid/private'};manifest.target.windowTitle='PRIVATE WINDOW';
  const raw=JSON.stringify(manifest);await writeFile(join(bundle,'apparent.json'),raw);const originalHome=process.env.HOME;process.env.HOME=root;
  try{const text=await localCopyText(bundle);assert(text.startsWith("apparent-agent inspect '"));assert(text.includes("'\\''quoted'\\''"));assert(text.includes('Read this. Then click.'));assert(text.includes('Keep semantic text.'));assert(text.includes('Use make-it-apparent'));assert(!text.includes('private-session'));assert(!text.includes('DO NOT FORWARD'));assert(!text.includes('PRIVATE WINDOW'));assert.equal(await readFile(join(bundle,'apparent.json'),'utf8'),raw);
    await mkdir(join(root,'.config','apparent'),{recursive:true});await writeFile(join(root,'.config','apparent','config.json'),JSON.stringify({secret:'untouched',experiments:{agentDemoVideo:false}}));assert(!(await localCopyText(bundle)).includes('Use make-it-apparent'));assert((await readFile(join(root,'.config','apparent','config.json'),'utf8')).includes('untouched'));
    manifest.keyframes[0].file='../outside.png';await writeFile(join(bundle,'apparent.json'),JSON.stringify(manifest));await assert.rejects(localCopyText(bundle),/unsafe/);
  }finally{if(originalHome===undefined)delete process.env.HOME;else process.env.HOME=originalHome;await rm(root,{recursive:true,force:true});}
});

test('compiled CLI uses the public Host port; lost reply stays unknown with a single write',async()=>{
  const {createServer}=await import('node:net');const {chmod}=await import('node:fs/promises');const {once}=await import('node:events');const {execFile}=await import('node:child_process');const {promisify}=await import('node:util');
  const facts={api:1,processEpoch:'epoch-a',revision:4,scope:'running-desktop-process',freshness:'cached-gui-observation',permissions:[],shortcuts:{record:{configured:null,effective:null,registration:'unknown',canValidate:true,canSave:false},paste:{configured:null,effective:null,registration:'unknown',canValidate:true,canSave:false}}};
  const root=await mkdtemp(join(tmpdir(),'agent-copy-cli-'));const bundle=join(root,'fixture.apparent');await mkdir(bundle,{mode:0o700});await writeFile(join(bundle,'apparent.json'),await readFile(new URL('../fixtures/current.json',import.meta.url)));
  const dir=join(root,'.config','apparent','agent-control');await mkdir(dir,{recursive:true,mode:0o700});await chmod(join(root,'.config'),0o700);await chmod(join(root,'.config','apparent'),0o700);const socketPath=join(dir,'capture.sock');let lose=false;let writes=0;
  const sockets=new Set<import('node:net').Socket>();const server=createServer(socket=>{sockets.add(socket);socket.once('close',()=>sockets.delete(socket));let buffer='';socket.on('data',chunk=>{buffer+=chunk.toString();if(!buffer.includes('\n'))return;const request=JSON.parse(buffer.trim());buffer='';let result:unknown;
    if(request.operation==='readSnapshot')result=facts;
    else {assert.equal(request.operation,'writeClipboardText');writes++;assert.deepEqual(request.observed,{processEpoch:'epoch-a',revision:4});assert(request.text.startsWith('apparent-agent inspect '));if(lose){socket.end();return;}result={status:'applied',digest:createHash('sha256').update(request.text).digest('hex'),byteLength:Buffer.byteLength(request.text)};}
    socket.end(JSON.stringify({protocol:1,id:request.id,result})+'\n');});});server.listen(socketPath);await once(server,'listening');await chmod(socketPath,0o600);
  const run=promisify(execFile);const cli=new URL('../../dist/cli.js',import.meta.url).pathname;
  try{const applied=await run(process.execPath,[cli,'copy',bundle],{env:{...process.env,HOME:root},timeout:10000});assert.match(applied.stdout,/Copied bundle text/);assert.match(applied.stdout,/Images were not copied/);assert.equal(writes,1);lose=true;
    await assert.rejects(run(process.execPath,[cli,'copy',bundle],{env:{...process.env,HOME:root},timeout:10000}),(error:any)=>{assert.equal(error.code,3);assert.equal(error.stdout,'');assert.match(error.stderr,/outcome is unknown/);return true;});assert.equal(writes,2);
  }finally{for(const socket of sockets)socket.destroy();await new Promise<void>(resolve=>server.close(()=>resolve()));await rm(root,{recursive:true,force:true});}
});

test('copy refuses unfinished bundles and preserves canonical uncropped references',async()=>{
  const root=await mkdtemp(join(tmpdir(),'agent-copy-crop-'));const bundle=join(root,'fixture.apparent');await mkdir(bundle);const manifest=JSON.parse(await readFile(new URL('../fixtures/current.json',import.meta.url),'utf8'));
  manifest.autocrop={label:'focused window',fullFrames:'full-frames',fullContactSheet:'full-contact.png',crops:[]};await writeFile(join(bundle,'apparent.json'),JSON.stringify(manifest));
  try{assert((await localCopyText(bundle)).includes(join(bundle,'full-frames')));await writeFile(join(bundle,'unfinished.json'),'{}');await assert.rejects(localCopyText(bundle),/unfinished/);}finally{await rm(root,{recursive:true,force:true});}
});
