import { test } from 'node:test';
import assert from 'node:assert/strict';
import { access, chmod, mkdtemp, mkdir, readFile, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { renameSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { PNG } from 'pngjs';
import { generateDemo } from '../dist/demo/generate.js';
import { readManifest } from '../dist/bundle.js';
import { runZoom } from '../dist/zoom/zoom.js';

test('demo writes private synthetic consumer bundle with annotated PNGs and CLI-readable semantics',async()=>{
  const parent=await mkdtemp(join(tmpdir(),'agent-demo-')),out=join(parent,'a path with spaces.apparent');
  try{
    const started=Date.now();
    const made=await generateDemo(out);const {value}=await readManifest(out);
    assert.equal(made.path,out);assert.equal(value.synthetic,true);assert.equal(value.provenance.kind,'synthetic-demo');
    assert.equal(value.keyframes[0].reason,'scene_change');
    assert.ok(value.createdAtMs>=started&&value.createdAtMs<=Date.now());
    const second=await generateDemo(join(parent,'second.apparent'));
    assert.notEqual((await readManifest(second.path)).value.id,value.id);
    assert.match(value.target.app,/synthetic/i);assert.equal(value.audio,undefined);assert.equal(value.video,undefined);
    assert.deepEqual(value.marks.map((m:any)=>m.strokes),[[0],[1,2],[3]]);
    assert.equal(value.transcript.words[0].t0,600);assert.equal(value.transcript.words.at(-1).t1,9400);
    assert.equal(value.keyframes.map((frame:any)=>frame.said).filter(Boolean).join(' '),value.transcript.text);
    for(const frame of value.keyframes){const image=PNG.sync.read(await readFile(join(out,frame.file)));assert.equal(image.width,960);assert.equal(image.height,600);assert.equal((await stat(join(out,frame.file))).mode&0o777,0o600);}
    assert.equal((await stat(out)).mode&0o777,0o700);for(const file of ['apparent.json','summary.md','contact-sheet.png'])assert.equal((await stat(join(out,file))).mode&0o777,0o600);
    const stored=await readFile(join(out,'frames/frame-03.png'));assert.deepEqual(stored.subarray(0,8),Buffer.from([137,80,78,71,13,10,26,10]));
    const result=await runZoom({bundle:out,mark:'demo-circle',span:2,frames:3,full:false,out:join(parent,'zoom'),json:true});assert.equal(result.sourceKind,'saved-keyframes');assert.equal(result.mark.id,'demo-circle');assert.equal(result.cells.length,1);
    const cli=new URL('../dist/cli.js',import.meta.url).pathname;
    const inspect=spawnSync(process.execPath,[cli,'inspect',out],{encoding:'utf8'});assert.equal(inspect.status,0,inspect.stderr);assert.match(inspect.stdout,/Synthetic demonstration only/);assert.doesNotMatch(inspect.stdout,/No microphone audio was saved/);
    const marks=spawnSync(process.execPath,[cli,'inspect',out,'--marks'],{encoding:'utf8'});assert.equal(marks.status,0,marks.stderr);assert.match(marks.stdout,/circled “Header band”/);assert.match(marks.stdout,/drew an arrow pointing to “Tool usage”/);
    const frames=spawnSync(process.execPath,[cli,'frames',out,'--from','0:02','--to','0:04','--json'],{encoding:'utf8'});assert.equal(frames.status,0,frames.stderr);assert.match(frames.stdout,/saved-keyframes/);
    const manifestRaw=await readFile(join(out,'apparent.json'),'utf8');assert.match(manifestRaw,/"synthetic": true/);
  }finally{await rm(parent,{recursive:true,force:true});}
});

test('demo default honors capturesDir without changing config and removes partial owned output on render failure',async()=>{
  const parent=await mkdtemp(join(tmpdir(),'agent-demo-default-')),home=join(parent,'home'),configured=join(parent,'configured captures');
  await mkdir(join(home,'.config','apparent'),{recursive:true});await mkdir(configured,{mode:0o700});
  const configPath=join(home,'.config','apparent','config.json'),config=JSON.stringify({capturesDir:configured,unrelatedSecret:'keep-private'});
  await writeFile(configPath,config);
  try{
    const cli=new URL('../dist/cli.js',import.meta.url).pathname;
    const generated=spawnSync(process.execPath,[cli,'demo','--json'],{encoding:'utf8',env:{...process.env,HOME:home,XDG_DATA_HOME:join(parent,'xdg'),APPARENT_CAPTURES_DIR:undefined}});
    assert.equal(generated.status,0,generated.stderr);const result=JSON.parse(generated.stdout);assert.ok(result.path.startsWith(configured+'/'));assert.equal(await readFile(configPath,'utf8'),config);
    const latest=spawnSync(process.execPath,[cli,'inspect','latest','--json'],{encoding:'utf8',env:{...process.env,HOME:home,XDG_DATA_HOME:join(parent,'xdg'),APPARENT_CAPTURES_DIR:undefined}});
    assert.equal(latest.status,0,latest.stderr);assert.equal(JSON.parse(latest.stdout).id,(await readManifest(result.path)).value.id);
    const original=PNG.sync.write;let calls=0;(PNG.sync as any).write=(...args:any[])=>{if(++calls===2)throw new Error('injected PNG encoder failure');return original.apply(PNG.sync,args);};
    const failed=join(parent,'partial.apparent');try{await assert.rejects(generateDemo(failed),/injected PNG encoder failure/);}finally{(PNG.sync as any).write=original;}
    await assert.rejects(access(failed));
  }finally{await rm(parent,{recursive:true,force:true});}
});

test('demo failure preserves a foreign replacement instead of deleting by pathname',async()=>{
  const parent=await mkdtemp(join(tmpdir(),'agent-demo-replacement-')),out=join(parent,'partial.apparent');
  const original=PNG.sync.write;let calls=0;
  (PNG.sync as any).write=(...args:any[])=>{
    if(++calls===2){
      const frame=join(out,'frames/frame-01.png');
      renameSync(frame,join(parent,'retained-original.png'));
      writeFileSync(frame,'foreign replacement',{flag:'wx'});
      throw new Error('injected failure after replacement');
    }
    return original.apply(PNG.sync,args);
  };
  try{
    await assert.rejects(generateDemo(out),/injected failure after replacement/);
    assert.equal(await readFile(join(out,'frames/frame-01.png'),'utf8'),'foreign replacement');
    await assert.rejects(access(join(out,'apparent.json')));
  }finally{(PNG.sync as any).write=original;await rm(parent,{recursive:true,force:true});}
});

test('demo refuses collisions, symlink parents, invalid output before creating anything, and preflights clipboard before output',async()=>{
  const parent=await mkdtemp(join(tmpdir(),'agent-demo-safe-'));
  try{
    const target=join(parent,'existing.apparent');await mkdir(target);await writeFile(join(target,'keep'),'untouched');
    await assert.rejects(generateDemo(target),/EEXIST|already exists/);assert.equal(await readFile(join(target,'keep'),'utf8'),'untouched');
    const link=join(parent,'link');await symlink(parent,link);await assert.rejects(generateDemo(join(link,'new.apparent')),/symlink/);
    const shared=join(parent,'shared');await mkdir(shared);await chmod(shared,0o777);
    await assert.rejects(generateDemo(join(shared,'unsafe.apparent')),/writable by other users/);
    await assert.rejects(access(join(shared,'unsafe.apparent')));
    const fifo=join(parent,'pipe.apparent');spawnSync('mkfifo',[fifo]);await assert.rejects(generateDemo(fifo),/EEXIST|already exists/);
    await assert.rejects(generateDemo('relative.apparent'),/canonical absolute/);
    const cli=new URL('../dist/cli.js',import.meta.url).pathname;
    const help=spawnSync(process.execPath,[cli,'demo','--help'],{encoding:'utf8'});assert.equal(help.status,0,help.stderr);assert.match(help.stdout,/Usage: apparent-agent demo/);assert.match(help.stdout,/compatible running Apparent GUI/);assert.match(help.stdout,/keeps the generated bundle/);
    const topHelp=spawnSync(process.execPath,[cli,'--help'],{encoding:'utf8'});
    assert.equal(topHelp.status,0);assert.equal((topHelp.stdout.match(/apparent-agent demo /g)??[]).length,1);assert.equal((topHelp.stdout.match(/apparent-agent zoom /g)??[]).length,1);
    const cliOut=join(parent,'cli output path.apparent');const cliDemo=spawnSync(process.execPath,[cli,'demo','--out',cliOut,'--json'],{encoding:'utf8'});assert.equal(cliDemo.status,0,cliDemo.stderr);const cliResult=JSON.parse(cliDemo.stdout);assert.match(cliResult.summary,/Synthetic demonstration only/);assert.equal(cliResult.images.length,6);assert.ok(cliResult.images.every((path:string)=>path.startsWith(cliOut+'/')));
    const clipboard=spawnSync(process.execPath,[cli,'demo','--out',join(parent,'clipboard.apparent'),'--clipboard'],{encoding:'utf8'});
    assert.equal(clipboard.status,2);assert.equal(clipboard.stdout,'');assert.match(clipboard.stderr,/compatible running Apparent GUI/);await assert.rejects(access(join(parent,'clipboard.apparent')));
    const duplicate=spawnSync(process.execPath,[cli,'demo','--json','--json'],{encoding:'utf8'});assert.equal(duplicate.status,2);assert.equal(duplicate.stdout,'');
  }finally{await rm(parent,{recursive:true,force:true});}
});
