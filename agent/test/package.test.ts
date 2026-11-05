import {test} from 'node:test';
import {strict as assert} from 'node:assert';
import {mkdtemp,rm,mkdir,writeFile,readFile,access} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {PNG} from 'pngjs';

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
    run([npm,'install','--offline','--ignore-scripts','--no-audit','--no-fund','--prefix',root,join(root,metadata[0].filename)]);
    const cli=resolve(root,'node_modules/@apparent/agent/dist/cli.js');
    assert.equal(run(['--input-type=module','-e',"await import('@apparent/agent')"]),'');
    assert.match(run([cli,'--help']),/Local Apparent/);assert.equal(run([cli,'--version']),'0.1.0\n');
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
