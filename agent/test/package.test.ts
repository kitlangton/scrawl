import {test} from 'node:test';
import {strict as assert} from 'node:assert';
import {mkdtemp,rm,mkdir,writeFile,readFile,access,chmod} from 'node:fs/promises';
import {cpSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {PNG} from 'pngjs';
import {createHash} from 'node:crypto';

// The isolated run executes this suite too; only its own source-copy driver is omitted.
if (process.env.APPARENT_AGENT_ISOLATED_TEST !== '1') test('Agent installs, builds and tests from an isolated owner source copy',async()=>{
  const root=await mkdtemp(join(tmpdir(),'agent-source-isolation-'));
  try {
    const npm=process.env.npm_execpath!; assert(npm);
    const source=fileURLToPath(new URL('..',import.meta.url));
    const isolated=join(root,'agent');
    cpSync(source,isolated,{recursive:true,filter:path=>!path.split('/').some(part=>part==='node_modules'||part==='dist')});
    const env={...process.env,HOME:root,APPARENT_AGENT_ISOLATED_TEST:'1'};
    const run=(args:string[])=>{
      const result=spawnSync(process.execPath,args,{cwd:isolated,encoding:'utf8',timeout:120000,env});
      assert.equal(result.status,0,result.stderr);return result.stdout;
    };
    run([npm,'ci','--no-audit','--no-fund']);
    run([npm,'run','build']);
    run([npm,'run','typecheck']);
    run([npm,'test']);
    const packed=JSON.parse(run([npm,'pack','--json','--pack-destination',root]));
    assert.equal(packed[0].name,'@apparent/agent');
  }finally{await rm(root,{recursive:true,force:true});}
});

test('packed CLI installs offline outside checkout and prints its standalone skill without config, storage, or Engine',async()=>{
  const root=await mkdtemp(join(tmpdir(),'agent-package-'));
  try {
    const npm=process.env.npm_execpath!; assert(npm);
    const env={...process.env,HOME:root,XDG_DATA_HOME:join(root,'data')};
    delete env.APPARENT_ENGINE_EXECUTABLE; delete env.APPARENT_ENGINE_FRAMES_EXECUTABLE; delete env.APPARENT_ENGINE_RUNTIME; delete env.APPARENT_CAPTURES_DIR;
    const runRaw=(args:string[],cwd=root)=>spawnSync(process.execPath,args,{cwd,encoding:'utf8',timeout:30000,env});
    const run=(args:string[],cwd=root)=>{
      const result=runRaw(args,cwd);
      assert.equal(result.status,0,result.stderr);return result.stdout;
    };
    const source=fileURLToPath(new URL('..',import.meta.url));
    const metadata=JSON.parse(run([npm,'pack','--ignore-scripts','--json','--pack-destination',root],source));
    run([npm,'install','--offline','--no-audit','--no-fund','--prefix',root,join(root,metadata[0].filename)]);
    const cli=resolve(root,'node_modules/@apparent/agent/dist/cli.js');
    assert.equal(run(['--input-type=module','-e',"await import('@apparent/agent')"]),'');
    const packageRoot=resolve(root,'node_modules/@apparent/agent');
    const recordConsumer=join(packageRoot,'.record-consumer.mts');
    await writeFile(recordConsumer,[
      "import * as record from '@apparent/record';",
      "import * as host from '@apparent/record/host';",
      "import * as capture2 from '@apparent/record/host/capture2';",
      "import * as publication from '@apparent/record/publication';",
      "import * as processing from '@apparent/record/processing';",
      "import * as mediaConsumer from '@apparent/record/processing/media-consumer';",
      "import * as media from '@apparent/engine-sdk/media';",
      "import * as sdk from '@apparent/engine-sdk';",
      "import * as sdkTesting from '@apparent/engine-sdk/testing';",
      "import * as mediaTesting from '@apparent/engine-sdk/media/testing';",
      "import * as captureSdk from '@apparent/engine-sdk/capture';",
      "import * as captureTesting from '@apparent/engine-sdk/capture/testing';",
      "import type { Capture2Phase } from '@apparent/record/host/capture2';",
      '// @ts-expect-error Capture2Host has no Agent-visible Saved phase.',
      'const savedPhase: Capture2Phase = "saved";',
      'void [record, host, capture2, publication, processing, mediaConsumer, media, sdk, sdkTesting, mediaTesting, captureSdk, captureTesting];',
    ].join('\n'));
    try {
      const tsc=resolve(source,'node_modules/typescript/bin/tsc');
      run([tsc,'--noEmit','--strict','--skipLibCheck','false','--module','NodeNext','--moduleResolution','NodeNext',
        '--target','ES2022','--types','node','--typeRoots',resolve(source,'node_modules/@types'),recordConsumer],packageRoot);
      const imports=run(['--input-type=module','-e',[
        "for(const [name,key] of [['@apparent/record','RecordWorkflow'],['@apparent/record/host','createRecordHostRuntime'],",
        "['@apparent/record/host/capture2','createCapture2Host'],['@apparent/record/publication','prepareRecordPublication'],",
        "['@apparent/record/processing','assembleProcessingOutput'],['@apparent/record/processing/media-consumer','processCompletedBundle'],",
        "['@apparent/engine-sdk/media','MediaClient']]){const value=await import(name);if(!(key in value))throw Error(name+' missing '+key);console.log(name)}",
        "await import('@apparent/agent');",
      ].join('')],packageRoot);
      assert.deepEqual(imports.trim().split('\n'),[
        '@apparent/record','@apparent/record/host','@apparent/record/host/capture2','@apparent/record/publication',
        '@apparent/record/processing','@apparent/record/processing/media-consumer','@apparent/engine-sdk/media',
      ]);
      run(['--input-type=module','-e',[
        "import assert from 'node:assert/strict';import {createRequire} from 'node:module';",
        "const require=createRequire(new URL('./package.json',import.meta.url));",
        "for(const name of ['react','react-dom','electron','@apparent/ui'])assert.throws(()=>require.resolve(name),{code:'MODULE_NOT_FOUND'});",
      ].join('')],packageRoot);
      run(['--input-type=module','-e',[
        "import assert from 'node:assert/strict';",
        "import {CaptureClient} from '@apparent/engine-sdk/capture';",
        "import {ReferenceCapturePeer} from '@apparent/engine-sdk/capture/testing';",
        "const peer=new ReferenceCapturePeer();",
        "assert.equal(peer.handshake.implementation,'test-double');assert.equal(peer.handshake.readiness,'unavailable');",
        "assert.equal(Object.values(peer.handshake.capabilities).every(value=>value===false),true);",
        "await assert.rejects(()=>new CaptureClient(peer).handshake(),error=>error.message.includes('explicit /capture/testing opt-in'));",
        "await peer.close();",
      ].join('')],packageRoot);
    } finally { await rm(recordConsumer,{force:true}); }
    assert.match(run([cli,'--help']),/Local Apparent/);assert.equal(run([cli,'--version']),'0.1.0\n');
    const hostDir=join(root,'.config','apparent','agent-control');await mkdir(hostDir,{recursive:true,mode:0o700});await chmod(join(root,'.config'),0o700);await chmod(join(root,'.config','apparent'),0o700);
    const socketPath=join(hostDir,'capture.sock');const {createServer}=await import('node:net');const {once}=await import('node:events');let snapshots=0,writes=0,clipboardText='';
    const host=createServer(socket=>{let buffer='';socket.on('error',()=>{});socket.on('data',chunk=>{buffer+=chunk.toString();if(!buffer.includes('\n'))return;const request=JSON.parse(buffer.trim());if(request.operation==='readSnapshot'){snapshots++;socket.end(JSON.stringify({protocol:1,id:request.id,result:{api:1,processEpoch:'packed-demo',revision:snapshots,scope:'running-desktop-process',freshness:'cached-gui-observation',permissions:[],shortcuts:{record:{configured:null,effective:null,registration:'unknown',canValidate:true,canSave:false},paste:{configured:null,effective:null,registration:'unknown',canValidate:true,canSave:false}}}})+'\n');return;}assert.equal(request.operation,'writeClipboardText');writes++;clipboardText=request.text;assert.deepEqual(request.observed,{processEpoch:'packed-demo',revision:2});socket.end(JSON.stringify({protocol:1,id:request.id,result:{status:'applied',digest:createHash('sha256').update(request.text).digest('hex'),byteLength:Buffer.byteLength(request.text)}})+'\n');});});
    host.listen(socketPath);await once(host,'listening');await chmod(socketPath,0o600);
    try {const syntheticOut=join(root,'packed-demo.apparent');const {execFile}=await import('node:child_process');const {promisify}=await import('node:util');const copied=await promisify(execFile)(process.execPath,[cli,'demo','--out',syntheticOut,'--clipboard','--json'],{cwd:root,encoding:'utf8',timeout:30000,env});const demoResult=JSON.parse(copied.stdout),demoManifest=JSON.parse(await readFile(join(syntheticOut,'apparent.json'),'utf8'));assert.equal(demoResult.path,syntheticOut);assert.equal(demoManifest.synthetic,true);assert.equal(demoManifest.provenance.kind,'synthetic-demo');assert.equal(demoResult.clipboardResult.status,'applied');assert.equal(writes,1);assert.equal(snapshots,2);assert(clipboardText.startsWith('apparent-agent inspect '));assert.match(clipboardText,/Synthetic demonstration only/);assert.match(clipboardText,/Use make-it-apparent/);assert.doesNotMatch(clipboardText,/Images were not copied/);} finally {await new Promise<void>(resolve=>host.close(()=>resolve()));}
    const captureV1=runRaw([cli,'capture','inspect','fixture-id']);
    assert.equal(captureV1.status,2);assert.equal(captureV1.stdout,'');assert.match(captureV1.stderr,/Set explicit absolute Engine executable and runtime paths/);
    for(const protocolArgs of [['--protocol','capture-v9','inspect','fixture-id'],['--protocol','capture2','--protocol','capture2','inspect','fixture-id']]) {
      const invalid=runRaw([cli,'capture',...protocolArgs]);
      assert.equal(invalid.status,2);assert.equal(invalid.stdout,'');assert.match(invalid.stderr,/one --protocol capture2/);
    }
    for (const command of ['inspect', 'record', 'capture', 'frames', 'zoom']) assert.match(run([cli,command,'--help']),/Usage: apparent-agent/);
    for(const args of [['--at','1','--mark','m1'],['--at','1','--frame','2'],['--mark','m1','--frames','-1']]) {
      const invalid=runRaw([cli,'zoom',...args]); assert.equal(invalid.status,2); assert.equal(invalid.stdout,'');
    }
    const unavailable = runRaw([cli,'frames','./clip.apparent','--from','0','--to','1']);
    assert.equal(unavailable.status,2); assert.equal(unavailable.stdout,'');
    assert.match(unavailable.stderr,/Local bundle directory not found/);
    const noHelpers=join(root,'no-helpers');await mkdir(noHelpers);
    const issueEnv={...env,PATH:noHelpers};
    const issueHelp=spawnSync(process.execPath,[cli,'issue','--help'],{cwd:root,encoding:'utf8',timeout:30000,env:issueEnv});
    assert.equal(issueHelp.status,0,issueHelp.stderr);assert.match(issueHelp.stdout,/--submit/);
    const issuePreviewResult=spawnSync(process.execPath,[cli,'issue','--title','Offline 🐸','--body','Multiline\ntext'],{cwd:root,encoding:'utf8',timeout:30000,env:issueEnv});
    assert.equal(issuePreviewResult.status,0,issuePreviewResult.stderr);
    const issuePreview=JSON.parse(issuePreviewResult.stdout);
    assert.deepEqual(issuePreview,{repository:'tryapparent/apparent',title:'Offline 🐸',body:'Multiline\ntext',submitted:false});
    const issueSkill=await readFile(resolve(source,'skills/apparent-issue/SKILL.md'));
    assert.deepEqual(Buffer.from(run(['--input-type=module','-e',`import{readFileSync}from'node:fs';process.stdout.write(readFileSync(${JSON.stringify(resolve(root,'node_modules/@apparent/agent/skills/apparent-issue/SKILL.md'))}))`])),issueSkill);
    const sourceGuide=await readFile(resolve(source,'skills/apparent/SKILL.md'));
    assert.deepEqual(Buffer.from(run([cli,'skill'])),sourceGuide);
    for(const operand of ['unexpected','--help','--unknown']){
      const invalid=runRaw([cli,'skill',operand]);
      assert.equal(invalid.status,2);assert.equal(invalid.stdout,'');
      assert.match(invalid.stderr,/skill takes no operands or options/);
    }
    const exists=async(path:string)=>access(path).then(()=>true,()=>false);
    assert.equal(await exists(join(root,'.config','apparent','config.json')),false);
    assert.equal(await exists(join(root,'data','apparent','captures')),false);
    const bundle=join(root,'fixture.apparent');await mkdir(bundle);
    const raw=JSON.stringify({version:1,id:'fixture',createdAtMs:1,durationMs:1,target:{mode:'display',app:null,bundleId:null,windowTitle:null,windowId:null,displayId:0,frame:{x:0,y:0,width:800,height:600,scale:1}},transcript:{text:'hello',segments:[]},strokes:[],marks:[],events:[],keyframes:[],contactSheet:'sheet.png'});
    await writeFile(join(bundle,'apparent.json'),raw);await writeFile(join(bundle,'summary.md'),'hello\n');
    assert.match(run([cli,'inspect',bundle]),/“hello”/);assert.equal(run([cli,'inspect',bundle,'--stored-summary']),'hello\n');assert.equal(run([cli,'inspect',bundle,'--json']),raw);
    assert.equal(await readFile(join(bundle,'apparent.json'),'utf8'),raw);
    const zoomBundle=join(root,'zoom.apparent');await mkdir(join(zoomBundle,'frames'),{recursive:true});
    const image=new PNG({width:12,height:8}); for(let p=0;p<image.data.length;p+=4) image.data.set([40,160,220,255],p);
    await writeFile(join(zoomBundle,'frames/frame-01.png'),PNG.sync.write(image,{colorType:6}));
    const zoomManifest={version:1,id:'packed-zoom',createdAtMs:1,durationMs:3000,target:{mode:'display',app:null,bundleId:null,windowTitle:null,windowId:null,displayId:0,frame:{x:0,y:0,width:12,height:8,scale:1}},transcript:{text:'look here',segments:[{t0:900,t1:1100,text:'look here'}],words:[{t0:900,t1:1000,text:'look'},{t0:1000,t1:1100,text:'here'}]},strokes:[],marks:[],events:[],keyframes:[{index:1,t:1000,file:'frames/frame-01.png'}],contactSheet:'missing.png'};
    const zoomRaw=JSON.stringify(zoomManifest); await writeFile(join(zoomBundle,'apparent.json'),zoomRaw);
    const zoomOutput=join(root,'packed-zoom-output');
    const zoomResult=JSON.parse(run([cli,'zoom',zoomBundle,'--at','look here','--frames','1','--out',zoomOutput,'--json']));
    assert.equal(zoomResult.sourceKind,'saved-keyframes');assert.equal(zoomResult.cells[0].timestampMs,1000);
    assert.ok((await readFile(zoomResult.sheet)).length>24);assert.equal(await readFile(join(zoomBundle,'apparent.json'),'utf8'),zoomRaw);
    const defaultZoom=JSON.parse(run([cli,'zoom',zoomBundle,'--at','look here','--frames','1','--json']));
    assert.ok(defaultZoom.sheet.startsWith(join(root,'.cache','apparent','agent','zoom')));
  }finally{await rm(root,{recursive:true,force:true});}
});
