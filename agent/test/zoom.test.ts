import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { PNG } from 'pngjs';
import { findPhraseMs, markWindow, parseZoomTime, runZoom, sampleTimes, zoomWindow } from '../dist/zoom/zoom.js';

function png(color: [number,number,number,number], width=20, height=10) {
  const image=new PNG({width,height});
  for(let p=0;p<image.data.length;p+=4) image.data.set(color,p);
  return PNG.sync.write(image,{colorType:6});
}
function manifest(file='frames/frame-01.png') {
  return {version:1,id:'zoom-test',createdAtMs:0,durationMs:10000,target:{mode:'device',displayId:0,frame:{x:0,y:0,width:100,height:100,scale:1}},
    transcript:{source:'test',text:'width, changes now',segments:[{t0:900,t1:2100,text:'The width changes now.'}],words:[{t0:1000,t1:1200,text:'Width,'},{t0:1500,t1:1900,text:'changes.'}]},
    strokes:[],marks:[{id:'mark-a',strokes:[],t0:1000,t1:1500,boxNorm:[0.25,0.2,0.75,0.8],shape:'circle',description:'circle',on:'button Save',frame:file}],
    events:[{kind:'click',t:1200,x:0.5,y:0.5,label:'Save',role:'button'}],
    keyframes:[{index:1,t:1000,reason:'mark',file,event:'mark',markId:'mark-a'},{index:2,t:4000,reason:'scene',file:'frames/frame-02.png'}],contactSheet:'contact-sheet.png'};
}
async function bundleFixture() {
  const parent=await mkdtemp(join(tmpdir(),'agent-zoom-')), root=join(parent,'capture.apparent');
  await mkdir(root);
  await mkdir(join(root,'frames'));
  await writeFile(join(root,'frames/frame-01.png'),png([255,0,0,255]));
  await writeFile(join(root,'frames/frame-02.png'),png([0,0,255,255]));
  await writeFile(join(root,'apparent.json'),JSON.stringify(manifest()));
  return root;
}

test('zoom time, phrase, legacy windows, inclusive sampling and limits',()=>{
  const bundle=manifest();
  assert.equal(parseZoomTime('0:12'),12000); assert.equal(parseZoomTime('12.5'),12500); assert.equal(parseZoomTime('12500ms'),12500); assert.equal(parseZoomTime('1.25e1s'),12500);
  assert.equal(findPhraseMs(bundle,'width changes'),1000); assert.equal(findPhraseMs(bundle,'changes now'),900);
  assert.equal(findPhraseMs({...bundle,transcript:{...bundle.transcript,words:[]}},'width changes'),900);
  assert.deepEqual(zoomWindow(10000,2000,25000),[9500,11500]);
  assert.deepEqual(markWindow(9322,10908,2000,13400),[8822,11408]);
  assert.deepEqual(sampleTimes(1000,2000,3),[1000,1500,2000]);
  assert.equal(sampleTimes(0,100,80).length,48);
  assert.deepEqual(sampleTimes(0,Number.MAX_SAFE_INTEGER,3),[0,4503599627370495,Number.MAX_SAFE_INTEGER]);
});

test('saved zoom composes real PNGs in row-major order and keeps semantic text in JSON',async()=>{
  const bundle=await bundleFixture(), out=join(await mkdtemp(join(tmpdir(),'zoom-out-parent-')),'fresh');
  try {
    const result=await runZoom({bundle,at:'width changes',span:2,frames:3,full:false,out,json:true});
    assert.equal(result.sourceKind,'saved-keyframes');
    assert.equal(result.cells[0].timestampMs,1000);
    assert.equal(result.cells[0].row,1); assert.equal(result.cells[0].column,1);
    assert.match(result.said,/Width, changes\./);
    assert.equal(result.cells[0].interactions[0].label,'Save');
    const sheet=PNG.sync.read(await readFile(result.sheet));
    assert.equal(sheet.width,492); assert.equal(sheet.height,332);
    assert.equal(result.mark,null);
    const sidecar=JSON.parse(await readFile(join(out,'zoom.json'),'utf8'));
    assert.deepEqual(sidecar.cells.map((cell:any)=>cell.time),result.cells.map((cell:any)=>cell.time));
  } finally { await rm(bundle,{recursive:true,force:true}); await rm(join(out,'..'),{recursive:true,force:true}); }
});

test('mark lookup, full-frame mapping and output collision protection',async()=>{
  const bundle=await bundleFixture();
  try {
    const value=manifest(); value.autocrop={label:'test',fullFrames:'frames-full',fullContactSheet:'contact-sheet-full.png',crops:[{index:1,x:0.25,y:0.2,width:0.5,height:0.6,kind:'focus',reason:'test'}]};
    await mkdir(join(bundle,'frames-full'));
    await writeFile(join(bundle,'frames-full/frame-01.png'),png([0,255,0,255],30,15));
    await writeFile(join(bundle,'apparent.json'),JSON.stringify(value));
    const outParent=await mkdtemp(join(tmpdir(),'zoom-out-parent-')), out=join(outParent,'fresh');
    const result=await runZoom({bundle,mark:'①',frame:1,span:2,frames:3,full:true,out,json:false});
    assert.equal(result.mark.id,'mark-a'); assert.equal(result.sourceKind,'saved-keyframes');
    assert.equal(result.cells[0].sourceFrameKind,'full-frame');
    assert.match(result.cells[0].framePath,/frames-full\/frame-01\.png$/);
    assert.match(result.said,/Width/);
    await assert.rejects(runZoom({bundle,at:'1',span:2,frames:1,full:false,out,json:false}),/EEXIST|already exists/);
    const cropped=await runZoom({bundle,mark:'mark-a',frame:1,span:2,frames:1,full:false,out:join(outParent,'cropped'),json:true});
    assert.equal(cropped.cells[0].sourceFrameKind,'full-frame'); assert.equal(cropped.cells[0].cropApplied,true);
    assert.deepEqual(cropped.cells[0].crop,[0.25,0.2,0.75,0.8]);
    await rm(outParent,{recursive:true,force:true});
  } finally { await rm(bundle,{recursive:true,force:true}); }
});

test('corrupt and oversized saved PNGs fail before any output is published',async()=>{
  const bundle=await bundleFixture(), parent=await mkdtemp(join(tmpdir(),'zoom-invalid-parent-')), out=join(parent,'owned-output');
  try {
    const invalid=Buffer.alloc(24); Buffer.from([137,80,78,71,13,10,26,10]).copy(invalid); invalid.writeUInt32BE(5000,16); invalid.writeUInt32BE(5000,20);
    await writeFile(join(bundle,'frames/frame-01.png'),invalid);
    await assert.rejects(runZoom({bundle,at:'1',span:2,frames:1,full:false,out,json:false}),/dimensions exceed safety limits/);
    await assert.rejects(import('node:fs/promises').then(fs=>fs.lstat(out)),{code:'ENOENT'});
  } finally { await rm(bundle,{recursive:true,force:true}); await rm(parent,{recursive:true,force:true}); }
});
