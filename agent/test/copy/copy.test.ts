import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {mkdtemp,mkdir,writeFile,readFile,rm,readdir,chmod} from 'node:fs/promises';
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
    await mkdir(join(root,'.config','apparent'),{recursive:true});const configPath=join(root,'.config','apparent','config.json');await writeFile(configPath,JSON.stringify({secret:'untouched',experiments:{agentContext:false}}));const disabled=await localCopyText(bundle);assert.equal(disabled,text);assert((await readFile(configPath,'utf8')).includes('untouched'));await writeFile(configPath,JSON.stringify({secret:'untouched',experiments:{agentContext:{bad:true}}}));assert.equal(await localCopyText(bundle),disabled);
    await writeFile(configPath,JSON.stringify({secret:'untouched',experiments:{agentContext:true}}));const enabled=await localCopyText(bundle);assert.notEqual(enabled,disabled);assert(enabled.includes('- #1 (0:01) “Read this.”'));assert(enabled.includes('clicked button “Save” “Then click.”'));assert(!enabled.includes('PRIVATE WINDOW'));assert((await readFile(configPath,'utf8')).includes('untouched'));
    const noFrames={...manifest,keyframes:[]};await writeFile(join(bundle,'apparent.json'),JSON.stringify(noFrames));await writeFile(configPath,JSON.stringify({secret:'untouched',experiments:{agentContext:false}}));const withoutRich=await localCopyText(bundle);await writeFile(configPath,JSON.stringify({secret:'untouched',experiments:{agentContext:true}}));assert.equal(await localCopyText(bundle),withoutRich);
    manifest.keyframes[0].file='../outside.png';await writeFile(join(bundle,'apparent.json'),JSON.stringify(manifest));await assert.rejects(localCopyText(bundle),/unsafe/);
  }finally{if(originalHome===undefined)delete process.env.HOME;else process.env.HOME=originalHome;await rm(root,{recursive:true,force:true});}
});

test('configured Agent context copy sends ordered keyframe text to Record without duplicate timeline text',async()=>{
  const root=await mkdtemp(join(tmpdir(),'agent-copy-rich-'));const bundle=join(root,'semantic.apparent');await mkdir(bundle);await writeFile(join(bundle,'apparent.json'),await readFile(new URL('../fixtures/semantic.json',import.meta.url)));
  const config=join(root,'.config','apparent');await mkdir(config,{recursive:true});await writeFile(join(config,'config.json'),JSON.stringify({secret:'preserved',experiments:{agentContext:true}}));const previous=process.env.HOME;process.env.HOME=root;
  try{const text=await localCopyText(bundle);const golden=await readFile(new URL('../fixtures/semantic.agent.md',import.meta.url),'utf8');assert.equal(text.slice(text.indexOf('- #1')),golden.slice(golden.indexOf('- #1')));assert(text.includes('- #1 (0:02) “Open the menu,”'));assert(text.includes('✎① 0:02 pin 1'));assert(text.includes('clicked button “Menu”'));assert(text.includes('Typed note · pin 1 · frames/one.png'));assert(text.includes('Keep this'));assert(text.includes('tab → Preview'));assert(text.includes('tab cleared'));assert(!text.includes('Private title'));assert(!text.includes('private-session'));assert.equal((text.match(/“Open the menu,”/gu)??[]).length,1);assert.equal((text.match(/tab → Preview/gu)??[]).length,1);assert.equal((text.match(/tab cleared/gu)??[]).length,1);assert.equal(JSON.parse(await readFile(join(config,'config.json'),'utf8')).secret,'preserved');}
  finally{if(previous===undefined)delete process.env.HOME;else process.env.HOME=previous;await rm(root,{recursive:true,force:true});}
});

test('compiled CLI uses the public Host port; lost reply stays unknown with a single write',async()=>{
  const {createServer}=await import('node:net');const {access,chmod}=await import('node:fs/promises');const {once}=await import('node:events');const {execFile}=await import('node:child_process');const {promisify}=await import('node:util');
  const facts={api:1,processEpoch:'epoch-a',revision:4,scope:'running-desktop-process',freshness:'cached-gui-observation',permissions:[],shortcuts:{record:{configured:null,effective:null,registration:'unknown',canValidate:true,canSave:false},paste:{configured:null,effective:null,registration:'unknown',canValidate:true,canSave:false}}};
  const root=await mkdtemp(join(tmpdir(),'agent-copy-cli-'));const bundle=join(root,'fixture.apparent');await mkdir(bundle,{mode:0o700});await writeFile(join(bundle,'apparent.json'),await readFile(new URL('../fixtures/semantic.json',import.meta.url)));
  const config=join(root,'.config','apparent');await mkdir(config,{recursive:true,mode:0o700});await writeFile(join(config,'config.json'),JSON.stringify({secret:'preserved',experiments:{agentContext:true}}));const dir=join(config,'agent-control');await mkdir(dir,{recursive:true,mode:0o700});await chmod(join(root,'.config'),0o700);await chmod(config,0o700);const socketPath=join(dir,'capture.sock');let lose=false;let writes=0;
  const sockets=new Set<import('node:net').Socket>();const server=createServer(socket=>{sockets.add(socket);socket.once('close',()=>sockets.delete(socket));let buffer='';socket.on('data',chunk=>{buffer+=chunk.toString();if(!buffer.includes('\n'))return;const request=JSON.parse(buffer.trim());buffer='';let result:unknown;
    if(request.operation==='readSnapshot')result=facts;
    else {assert.equal(request.operation,'writeClipboardText');writes++;assert.deepEqual(request.observed,{processEpoch:'epoch-a',revision:4});assert(request.text.startsWith('apparent-agent inspect '));assert(request.text.includes('- #1 (0:02) “Open the menu,”'));assert(request.text.includes('Typed note · pin 1 · frames/one.png'));assert(request.text.includes('tab → Preview'));assert.equal((request.text.match(/tab → Preview/gu)??[]).length,1);assert.equal((request.text.match(/tab cleared/gu)??[]).length,1);assert(!request.text.includes('Private title'));if(lose){socket.end();return;}result={status:'applied',digest:createHash('sha256').update(request.text).digest('hex'),byteLength:Buffer.byteLength(request.text)};}
    socket.end(JSON.stringify({protocol:1,id:request.id,result})+'\n');});});server.listen(socketPath);await once(server,'listening');await chmod(socketPath,0o600);
  const run=promisify(execFile);const cli=process.env.AGENT_CLI_UNDER_TEST??new URL('../../dist/cli.js',import.meta.url).pathname;
  try{const applied=await run(process.execPath,[cli,'copy',bundle],{env:{...process.env,HOME:root},timeout:10000});assert.match(applied.stdout,/Copied bundle text/);assert.match(applied.stdout,/Images were not copied/);assert.equal(writes,1);lose=true;
    await assert.rejects(run(process.execPath,[cli,'copy',bundle],{env:{...process.env,HOME:root},timeout:10000}),(error:any)=>{assert.equal(error.code,3);assert.equal(error.stdout,'');assert.match(error.stderr,/outcome is unknown/);return true;});assert.equal(writes,2);
  }finally{for(const socket of sockets)socket.destroy();await new Promise<void>(resolve=>server.close(()=>resolve()));await rm(root,{recursive:true,force:true});}
});

test('copy --preview prints exact local handoff text without Host access or input mutation',async()=>{
  const {createServer}=await import('node:net');const {once}=await import('node:events');const {execFile}=await import('node:child_process');const {promisify}=await import('node:util');
  const root=await mkdtemp(join(tmpdir(),'agent-copy-preview-'));const bundle=join(root,'-Evidence with spaces 👋');await mkdir(bundle,{mode:0o700});
  const manifest=JSON.parse(await readFile(new URL('../fixtures/semantic.json',import.meta.url),'utf8'));manifest.note='Long local note '.repeat(1800);
  const manifestBytes=Buffer.from(JSON.stringify(manifest));await writeFile(join(bundle,'apparent.json'),manifestBytes);
  const receipt=Buffer.from(JSON.stringify({version:1,captureId:'c'.repeat(32),server:'https://connect.example',media:'not downloaded',guidance:'Use apparent fetch <id> --media --out <new-directory> for the complete recording'}));await writeFile(join(bundle,'.evidence-only.json'),receipt);
  const configDir=join(root,'.config','apparent');await mkdir(configDir,{recursive:true,mode:0o700});await chmod(join(root,'.config'),0o700);await chmod(configDir,0o700);
  const configPath=join(configDir,'config.json');const configBytes=Buffer.from(JSON.stringify({secret:'must remain untouched',experiments:{agentContext:true,agentDemoVideo:true}}));await writeFile(configPath,configBytes);
  const control=join(configDir,'agent-control');await mkdir(control,{mode:0o700});const socketPath=join(control,'capture.sock');let connections=0;const server=createServer(socket=>{connections++;socket.destroy();});server.listen(socketPath);await once(server,'listening');await chmod(socketPath,0o600);
  const originalHome=process.env.HOME;process.env.HOME=root;const cli=new URL('../../dist/cli.js',import.meta.url).pathname;const run=promisify(execFile);
  try{
    const expected=await localCopyText(bundle);assert(Buffer.byteLength(expected)>21800);const before=await readdir(bundle);
    const preview=await run(process.execPath,[cli,'copy',bundle,'--preview'],{env:{...process.env,HOME:root,PATH:'/nonexistent',APPARENT_ENGINE_FRAMES_EXECUTABLE:'/missing/provider',APPARENT_ENGINE_RUNTIME:'/missing/runtime'},timeout:15000});
    assert.equal(preview.stdout,expected);assert.match(preview.stderr,/Preview only; clipboard unchanged/);assert.match(preview.stderr,/video\/audio not downloaded/);assert.equal(connections,0);
    assert.deepEqual(await readdir(bundle),before);assert.deepEqual(await readFile(join(bundle,'apparent.json')),manifestBytes);assert.deepEqual(await readFile(join(bundle,'.evidence-only.json')),receipt);assert.deepEqual(await readFile(configPath),configBytes);
    const plainPreview=await run(process.execPath,[cli,'copy','--preview','--','-Evidence with spaces 👋'],{env:{...process.env,HOME:root,PATH:'/nonexistent'},cwd:root,timeout:15000});assert.equal(plainPreview.stdout,expected);assert.equal(connections,0);
    await writeFile(join(bundle,'.media-pending'),'pending');
    const pending=await run(process.execPath,[cli,'copy',bundle,'--preview'],{env:{...process.env,HOME:root},timeout:10000}).then(()=>null,(error:any)=>error);assert.equal(pending?.code,2);assert.equal(pending?.stdout,'');assert.match(pending?.stderr??'',/media is pending/);assert.equal(connections,0);
    await rm(join(bundle,'.media-pending'));
    const badArgs=[['/does/not/exist','--preview','--preview'],['/does/not/exist','--unknown'],['--'],['/does/not/exist','--',bundle,'extra'],['/does/not/exist','another']];
    for(const args of badArgs){const result=await run(process.execPath,[cli,'copy',...args],{env:{...process.env,HOME:root},timeout:10000}).then(()=>null,(error:any)=>error);assert.equal(result?.code,2);assert.equal(result?.stdout,'');assert.doesNotMatch(result?.stderr??'',/Local bundle directory not found/);}
    assert.equal(connections,0);assert.deepEqual(await readFile(configPath),configBytes);
  }finally{if(originalHome===undefined)delete process.env.HOME;else process.env.HOME=originalHome;await new Promise<void>(resolve=>server.close(()=>resolve()));await rm(root,{recursive:true,force:true});}
});

test('copy refuses unfinished bundles and preserves canonical uncropped references',async()=>{
  const root=await mkdtemp(join(tmpdir(),'agent-copy-crop-'));const bundle=join(root,'fixture.apparent');await mkdir(bundle);const manifest=JSON.parse(await readFile(new URL('../fixtures/current.json',import.meta.url),'utf8'));
  manifest.autocrop={label:'focused window',fullFrames:'full-frames',fullContactSheet:'full-contact.png',crops:[]};await writeFile(join(bundle,'apparent.json'),JSON.stringify(manifest));
  try{assert((await localCopyText(bundle)).includes(join(bundle,'full-frames')));await writeFile(join(bundle,'unfinished.json'),'{}');await assert.rejects(localCopyText(bundle),/unfinished/);}finally{await rm(root,{recursive:true,force:true});}
});

test('compiled demo clipboard preflights, writes exact Record text once with a fresh snapshot, and retains synthetic output on failure',async()=>{
  const {createServer}=await import('node:net');const {access,chmod}=await import('node:fs/promises');const {once}=await import('node:events');const {execFile}=await import('node:child_process');const {promisify}=await import('node:util');
  const root=await mkdtemp(join(tmpdir(),'agent-demo-copy-cli-'));const control=join(root,'.config','apparent','agent-control');await mkdir(control,{recursive:true,mode:0o700});await chmod(join(root,'.config'),0o700);await chmod(join(root,'.config','apparent'),0o700);const socketPath=join(control,'capture.sock');
  let snapshots=0,writes=0,mode:'applied'|'refused'|'lost'='applied';let writtenText='';
  const facts=(revision:number)=>({api:1,processEpoch:'epoch-demo',revision,scope:'running-desktop-process',freshness:'cached-gui-observation',permissions:[],shortcuts:{record:{configured:null,effective:null,registration:'unknown',canValidate:true,canSave:false},paste:{configured:null,effective:null,registration:'unknown',canValidate:true,canSave:false}}});
  const server=createServer(socket=>{let buffer='';socket.on('data',chunk=>{buffer+=chunk.toString();if(!buffer.includes('\n'))return;const request=JSON.parse(buffer.trim());buffer='';if(request.operation==='readSnapshot'){snapshots++;socket.end(JSON.stringify({protocol:1,id:request.id,result:facts(snapshots===1?4:5)})+'\n');return;}
    assert.equal(request.operation,'writeClipboardText');writes++;writtenText=request.text;assert.deepEqual(request.observed,{processEpoch:'epoch-demo',revision:5});
    if(mode==='lost'){socket.end();return;}const result=mode==='refused'?{status:'refused',reason:'busy'}:{status:'applied',digest:createHash('sha256').update(request.text).digest('hex'),byteLength:Buffer.byteLength(request.text)};socket.end(JSON.stringify({protocol:1,id:request.id,result})+'\n');});});
  server.listen(socketPath);await once(server,'listening');await chmod(socketPath,0o600);const run=promisify(execFile);const cli=new URL('../../dist/cli.js',import.meta.url).pathname;
  try{
    const malformed=await run(process.execPath,[cli,'demo','--clipboard','--out',join(root,'bad.apparent'),'--json','--json'],{env:{...process.env,HOME:root}}).then(()=>null,(error:any)=>error);assert.equal(malformed.code,2);assert.equal(malformed.stdout,'');assert.equal(snapshots,0);await assert.rejects(access(join(root,'bad.apparent')));
    for(const badOut of [`${root}/nested/../bad.apparent`]) {
      const invalid=await run(process.execPath,[cli,'demo','--clipboard','--out',badOut],{env:{...process.env,HOME:root}}).then(()=>null,(error:any)=>error);
      assert.equal(invalid.code,2);assert.match(invalid.stderr,/canonical absolute/);assert.equal(snapshots,0);
    }
    const unavailableRoot=join(root,'unavailable-home'),unavailableOut=join(root,'unavailable.apparent');const unavailable=await run(process.execPath,[cli,'demo','--out',unavailableOut,'--clipboard'],{env:{...process.env,HOME:unavailableRoot}}).then(()=>null,(error:any)=>error);assert.equal(unavailable.code,2);assert.equal(unavailable.stdout,'');assert.match(unavailable.stderr,/no bundle was generated/);await assert.rejects(access(unavailableOut));
    const out=join(root,'applied.apparent');const applied=await run(process.execPath,[cli,'demo','--out',out,'--clipboard','--json'],{env:{...process.env,HOME:root},timeout:15000});const payload=JSON.parse(applied.stdout);const originalHome=process.env.HOME;process.env.HOME=root;let expectedText:string;try{expectedText=await localCopyText(out);}finally{if(originalHome===undefined)delete process.env.HOME;else process.env.HOME=originalHome;}assert.equal(payload.path,out);assert.equal(writtenText,expectedText);assert.deepEqual(payload.clipboardResult,{status:'applied',digest:createHash('sha256').update(expectedText).digest('hex'),byteLength:Buffer.byteLength(expectedText)});assert.match(writtenText,/Synthetic demonstration only/);assert.equal(snapshots,2);assert.equal(writes,1);assert.match(await readFile(join(out,'apparent.json'),'utf8'),/"synthetic": true/);
    const plainOut=join(root,'plain.apparent');const plain=await run(process.execPath,[cli,'demo','--out',plainOut,'--clipboard'],{env:{...process.env,HOME:root},timeout:15000});assert.match(plain.stdout,/Synthetic demo bundle:/);assert.match(plain.stdout,new RegExp(plainOut.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')));assert.match(plain.stdout,/Copied demo text to clipboard/);assert.match(plain.stdout,new RegExp(createHash('sha256').update(writtenText).digest('hex')));assert.equal(writes,2);assert.equal(snapshots,4);
    for(const failure of ['refused','lost'] as const){mode=failure;snapshots=0;const failedOut=join(root,`${failure}.apparent`);const error=await run(process.execPath,[cli,'demo','--out',failedOut,'--clipboard',...(failure==='lost'?['--json']:[])],{env:{...process.env,HOME:root},timeout:15000}).then(()=>null,(caught:any)=>caught);assert.equal(error.code,failure==='lost'?3:2);assert.equal(snapshots,2);assert.equal(writes,failure==='lost'?4:3);if(failure==='lost'){const json=JSON.parse(error.stdout);assert.equal(json.path,failedOut);assert.deepEqual(json.clipboardResult,{status:'outcome-unknown'});}else{assert.match(error.stdout,/Synthetic demo bundle:/);assert(error.stdout.includes(failedOut));}assert.doesNotMatch(error.stdout,/Copied demo text to clipboard/);assert.match(error.stderr,/retained at/);const manifest=JSON.parse(await readFile(join(failedOut,'apparent.json'),'utf8'));assert.equal(manifest.synthetic,true);assert.equal(manifest.provenance.kind,'synthetic-demo');assert.deepEqual((await readFile(join(failedOut,'contact-sheet.png'))).subarray(0,8),Buffer.from([137,80,78,71,13,10,26,10]));}
  }finally{await new Promise<void>(resolve=>server.close(()=>resolve()));await rm(root,{recursive:true,force:true});}
});

test('local copy preserves legacy context clear values at the typed Record boundary', async () => {
  const root=await mkdtemp(join(tmpdir(),'agent-copy-context-'));
  const bundle=join(root,'context.apparent');await mkdir(bundle);
  const raw=await readFile(new URL('../fixtures/semantic.json',import.meta.url),'utf8');
  await writeFile(join(bundle,'apparent.json'),raw);
  try {
    const text=await localCopyText(bundle);
    assert.match(text,/tab → Preview/);assert.match(text,/tab cleared/);
    assert.equal(await readFile(join(bundle,'apparent.json'),'utf8'),raw);
  } finally { await rm(root,{recursive:true,force:true}); }
});
