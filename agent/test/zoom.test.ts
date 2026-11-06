import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { deflateSync } from 'node:zlib';
import { createHash } from 'node:crypto';
import { PNG } from 'pngjs';
import { annotateZoomFrame, findPhraseMs, markWindow, parseZoomTime, runZoom, sampleTimes, zoomWindow, makeDecoder } from '../dist/zoom/zoom.js';

function png(color: [number,number,number,number], width=20, height=10) {
  const image=new PNG({width,height});
  for(let p=0;p<image.data.length;p+=4) image.data.set(color,p);
  return PNG.sync.write(image,{colorType:6});
}
function manifest(file='frames/frame-01.png') {
  return {version:1,id:'zoom-test',createdAtMs:0,durationMs:10000,target:{mode:'device',displayId:0,frame:{x:0,y:0,width:1000,height:800,scale:1}},
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
  assert.throws(()=>parseZoomTime('1:-1'),/invalid time/);
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
    const manifest=JSON.parse(await readFile(join(bundle,'apparent.json'),'utf8'));
    manifest.events.push({kind:'drag',t:900,t1:1900,x0:0.1,y0:0.1,x1:0.9,y1:0.9,button:'touch',path:[[0.1,0.1],[0.5,0.5],[0.9,0.9]]});
    await writeFile(join(bundle,'apparent.json'),JSON.stringify(manifest));
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
    const drag=sidecar.cells[0].interactions.find((event:any)=>event.kind==='drag');
    assert.equal(drag.t1,sidecar.cells[0].timestampMs);
    assert.deepEqual(drag.path,[[0.1,0.1]]);
  } finally { await rm(bundle,{recursive:true,force:true}); await rm(join(out,'..'),{recursive:true,force:true}); }
});

test('a PNG worker exiting during decode rejects promptly and clears its pending deadline', {timeout:2000}, async()=>{
  const decoder=makeDecoder();
  const pending=assert.rejects(decoder.decode(png([255,0,0,255]),[0,0,1,1],60_000),/PNG worker exited/);
  await decoder.worker.terminate();
  await pending;
});

test('mark lookup, full-frame mapping and output collision protection',async()=>{
  const bundle=await bundleFixture();
  try {
    const value=manifest(); value.autocrop={label:'test',fullFrames:'frames-full',fullContactSheet:'contact-sheet-full.png',crops:[{index:1,x:250,y:160,width:500,height:480,kind:'focus',reason:'test'}]};
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
    await assert.rejects(runZoom({bundle,mark:'mark-a',frame:2,span:2,frames:1,full:false,out:join(outParent,'wrong-frame'),json:true}),/No mark matches/);
    await rm(outParent,{recursive:true,force:true});
  } finally { await rm(bundle,{recursive:true,force:true}); }
});

test('same-timestamp full-frame counterparts follow their exact manifest image paths',async()=>{
  const bundle=await bundleFixture(),parent=await mkdtemp(join(tmpdir(),'zoom-duplicate-time-'));
  try{
    const value=manifest();value.keyframes[1].t=1000;value.keyframes[1].file='frames/frame-02.png';
    value.autocrop={label:'test',fullFrames:'frames-full',fullContactSheet:'contact.png',crops:[{index:1,x:250,y:160,width:500,height:480,kind:'focus',reason:'first'},{index:2,x:100,y:80,width:500,height:400,kind:'focus',reason:'second'}]};
    await mkdir(join(bundle,'frames-full'));await writeFile(join(bundle,'frames-full/frame-01.png'),png([0,255,0,255],30,15));await writeFile(join(bundle,'frames-full/frame-02.png'),png([255,255,0,255],30,15));await writeFile(join(bundle,'apparent.json'),JSON.stringify(value));
    const result=await runZoom({bundle,at:'1',span:2,frames:2,full:false,out:join(parent,'sheet'),json:true});
    assert.deepEqual(result.cells.map((cell:any)=>cell.framePath.slice(cell.framePath.lastIndexOf('frame-'))),['frame-01.png','frame-02.png']);
    assert.deepEqual(result.cells.map((cell:any)=>cell.crop),[[.25,.2,.75,.8],[.25,.2,.75,.8]]); // focused mark fixes its owning keyframe crop
  }finally{await rm(bundle,{recursive:true,force:true});await rm(parent,{recursive:true,force:true});}
});

test('partial full-frame retention preserves already-cropped saved pixels without claiming uncrop',async()=>{
  const bundle=await bundleFixture(),parent=await mkdtemp(join(tmpdir(),'zoom-partial-full-'));
  try{
    const value=manifest();value.autocrop={label:'focus',fullFrames:'frames-full',fullContactSheet:'contact.png',crops:[{index:1,x:250,y:160,width:500,height:480,kind:'focus',reason:'test'}]};
    await writeFile(join(bundle,'apparent.json'),JSON.stringify(value));
    const normal=await runZoom({bundle,at:'1',span:2,frames:1,full:false,out:join(parent,'normal'),json:true});
    assert.equal(normal.cells[0].sourceFrameKind,'autocropped-keyframe');assert.equal(normal.cells[0].fullFrameAvailable,false);
    assert.equal(normal.cells[0].cropApplied,false);assert.equal(normal.cells[0].annotations,'already baked into retained source');
    const full=await runZoom({bundle,at:'1',span:2,frames:1,full:true,out:join(parent,'full'),json:true});
    assert.equal(full.cells[0].sourceFrameKind,'autocropped-keyframe');assert.equal(full.cells[0].fullFrameAvailable,false);
    assert.equal(full.cells[0].cropApplied,false);assert.equal(full.cells[0].fullRequested,true);
  }finally{await rm(bundle,{recursive:true,force:true});await rm(parent,{recursive:true,force:true});}
});

test('configured provider paths still use saved mode for a missing video, but real decode setup errors stay authoritative',async()=>{
  const bundle=await bundleFixture(),parent=await mkdtemp(join(tmpdir(),'zoom-provider-fallback-'));
  const priorExecutable=process.env.APPARENT_ENGINE_FRAMES_EXECUTABLE,priorRuntime=process.env.APPARENT_ENGINE_RUNTIME;
  try{
    process.env.APPARENT_ENGINE_FRAMES_EXECUTABLE=join(parent,'missing-engine');process.env.APPARENT_ENGINE_RUNTIME=join(parent,'missing-runtime');
    const fallback=await runZoom({bundle,at:'1',span:2,frames:1,full:false,out:join(parent,'fallback'),json:true});
    assert.equal(fallback.sourceKind,'saved-keyframes');
    await writeFile(join(bundle,'recording.mp4'),'synthetic video source');
    await assert.rejects(runZoom({bundle,at:'1',span:2,frames:1,full:false,out:join(parent,'provider-error'),json:true}),/ENOENT|Frames Engine executable must be an executable regular file/);
  }finally{
    if(priorExecutable===undefined)delete process.env.APPARENT_ENGINE_FRAMES_EXECUTABLE;else process.env.APPARENT_ENGINE_FRAMES_EXECUTABLE=priorExecutable;
    if(priorRuntime===undefined)delete process.env.APPARENT_ENGINE_RUNTIME;else process.env.APPARENT_ENGINE_RUNTIME=priorRuntime;
    await rm(bundle,{recursive:true,force:true});await rm(parent,{recursive:true,force:true});
  }
});

test('top-level help lists zoom and invalid CLI combinations fail without output',async()=>{
  const bundle=await bundleFixture(),parent=await mkdtemp(join(tmpdir(),'zoom-cli-'));
  try{
    const cli=new URL('../dist/cli.js',import.meta.url).pathname,help=spawnSync(process.execPath,[cli,'--help'],{encoding:'utf8'});
    assert.match(help.stdout,/apparent-agent zoom/);
    const run=(...args:string[])=>spawnSync(process.execPath,[cli,'zoom',...args],{encoding:'utf8'});
    assert.equal(run(bundle,'--at','1','--mark','mark-a').status,2);
    assert.equal(run(bundle,'--at','1','--at','2').status,2);
    const rejected=run(bundle,'--mark','mark-a','--frame','2','--out',join(parent,'wrong-frame'));
    assert.equal(rejected.status,2);assert.match(rejected.stderr,/No mark matches/);
    await assert.rejects(import('node:fs/promises').then(fs=>fs.lstat(join(parent,'wrong-frame'))),{code:'ENOENT'});
  }finally{await rm(bundle,{recursive:true,force:true});await rm(parent,{recursive:true,force:true});}
});

test('corrupt and oversized saved PNGs fail before any output is published',async()=>{
  const bundle=await bundleFixture(), parent=await mkdtemp(join(tmpdir(),'zoom-invalid-parent-')), out=join(parent,'owned-output');
  try {
    await writeFile(join(bundle,'frames/frame-01.png'),png([0,0,0,255]));
    const usable=await readFile(join(bundle,'frames/frame-01.png'));usable.writeUInt32BE(5000,16);usable.writeUInt32BE(5000,20);
    // The dimension cap is checked before CRC/decompression.
    await writeFile(join(bundle,'frames/frame-01.png'),usable);
    await assert.rejects(runZoom({bundle,at:'1',span:2,frames:1,full:false,out,json:false}),/dimensions exceed safety limits/);
    await assert.rejects(import('node:fs/promises').then(fs=>fs.lstat(out)),{code:'ENOENT'});
  } finally { await rm(bundle,{recursive:true,force:true}); await rm(parent,{recursive:true,force:true}); }
});

test('producer-shaped circle ink, focused whole stroke, crop mapping, click and touch intent render without fake boxes',()=>{
  const image=new PNG({width:400,height:300});for(let p=0;p<image.data.length;p+=4)image.data.set([20,50,180,255],p);
  const pts:number[]=[];for(let i=0;i<=24;i++){const a=2*Math.PI*i/24;pts.push(Math.round((.5+.25*Math.cos(a))*10000),Math.round((.5+.25*Math.sin(a))*10000),i*25);}
  const bundle=manifest();bundle.strokes=[{t0:1000,t1:1600,fw:1000,fh:800,pin:null,laser:false,pts},{t0:900,t1:1200,fw:1000,fh:800,pin:null,laser:true,pts:[1000,6000,0,2000,6000,100,3000,6000,200]}];
  bundle.marks=[{...bundle.marks[0],strokes:[0],shape:'circle',t0:1000,t1:1600}];
  bundle.events=[{kind:'drag',t:1100,t1:1400,x0:.1,y0:.2,x1:.3,y1:.3,path:[[.1,.2],[.2,.25],[.3,.3]],button:'touch',label:'swipe'}, {kind:'click',t:1500,x:.8,y:.8,label:'Save'}];
  annotateZoomFrame(image,bundle,1300,[0,0,1,1],bundle.marks[0]);
  const pixel=(x:number,y:number)=>Array.from(image.data.subarray((y*image.width+x)*4,(y*image.width+x)*4+3));
  assert.notDeepEqual(pixel(328,240),[20,50,180]); // clicked point ripple
  assert.notDeepEqual(pixel(200,225),[20,50,180]); // real focused circle ink after t=1300
  assert.deepEqual(pixel(100,100),[20,50,180]); // no fabricated box edge
  assert.notDeepEqual(pixel(80,75),[20,50,180]); // touch trail
  assert.deepEqual(pixel(120,90),[20,50,180]); // drag points after the sample time stay hidden
  assert.notDeepEqual(pixel(100,180),[20,50,180]); // recent laser segment
  const pinImage=new PNG({width:400,height:300});for(let p=0;p<pinImage.data.length;p+=4)pinImage.data.set([20,50,180,255],p);
  bundle.strokes.push({t0:1000,t1:1000,fw:1000,fh:800,pin:3,laser:false,pts:[6000,5000,0]});
  const pinMark={...bundle.marks[0],id:'pin-mark',strokes:[2],shape:'pin',pin:3,t0:1000,t1:1000};bundle.marks.push(pinMark);
  annotateZoomFrame(pinImage,bundle,1000,[0,0,1,1],pinMark);
  assert.notDeepEqual(Array.from(pinImage.data.subarray((150*400+249)*4,(150*400+249)*4+3)),[20,50,180]);
  const recentImage=new PNG({width:400,height:300});for(let p=0;p<recentImage.data.length;p+=4)recentImage.data.set([20,50,180,255],p);
  bundle.strokes.push({t0:700,t1:1400,fw:1000,fh:800,pin:null,laser:false,pts:[1000,4000,0,2000,4000,100,9000,4000,600]});
  annotateZoomFrame(recentImage,bundle,900,[0,0,1,1],bundle.marks[0]);
  assert.notDeepEqual(Array.from(recentImage.data.subarray((120*400+60)*4,(120*400+60)*4+3)),[20,50,180]);
  assert.deepEqual(Array.from(recentImage.data.subarray((120*400+360)*4,(120*400+360)*4+3)),[20,50,180]); // recent unassociated ink is time-clipped
});

test('recorded stroke RGB reaches plain ink, pin fills and timed laser ink while absent RGB keeps legacy colors',()=>{
  const image=new PNG({width:400,height:300});
  const bundle=manifest();
  bundle.events=[];bundle.marks=[];
  bundle.strokes=[
    {t0:0,t1:200,fw:1000,fh:800,pin:null,laser:false,rgb:[17,203,81],pts:[2000,3000,0,8000,3000,200]},
    {t0:1000,t1:1200,fw:1000,fh:800,pin:1,laser:false,rgb:[0,0,255],pts:[5000,5000,0]},
    {t0:800,t1:1200,fw:1000,fh:800,pin:null,laser:true,rgb:[255,255,0],pts:[2000,7500,0,8000,7500,200]},
  ];
  annotateZoomFrame(image,bundle,1200,[0,0,1,1]);
  const pixel=(x:number,y:number)=>Array.from(image.data.subarray((y*image.width+x)*4,(y*image.width+x)*4+4));
  assert.deepEqual(pixel(200,90),[17,203,81,230]);
  assert.deepEqual(pixel(207,150),[0,0,255,255]);
  assert.deepEqual(pixel(200,225),[255,255,0,150]);

  // Frozen pre-RGB renderer output for all three styles, including pin glyphs.
  const legacyAll = structuredClone(bundle);
  for (const stroke of legacyAll.strokes) delete stroke.rgb;
  const legacyAllImage = new PNG({width:400,height:300});
  annotateZoomFrame(legacyAllImage,legacyAll,1200,[0,0,1,1]);
  assert.equal(createHash('sha256').update(legacyAllImage.data).digest('hex'),
    '18b07e378e430dffda1a31a2ca6721e8ec860dd1984e71c2d2ff6450875f748b');

  const legacy=new PNG({width:400,height:300});
  const defaults=manifest();defaults.events=[];defaults.marks=[];
  defaults.strokes=[{t0:0,t1:100,fw:1000,fh:800,pin:null,laser:false,pts:[2000,3000,0,8000,3000,100]}];
  annotateZoomFrame(legacy,defaults,100,[0,0,1,1]);
  assert.deepEqual(Array.from(legacy.data.subarray((90*400+200)*4,(90*400+200)*4+4)),[238,48,68,230]);

  const black=manifest();black.events=[];black.marks=[];
  black.strokes=[
    {t0:0,t1:200,fw:1000,fh:800,pin:null,laser:false,rgb:[0,0,0],pts:[2000,2000,0,8000,2000,200]},
    {t0:1000,t1:1200,fw:1000,fh:800,pin:1,laser:false,rgb:[0,0,0],pts:[5000,5000,0]},
    {t0:800,t1:1200,fw:1000,fh:800,pin:null,laser:true,rgb:[0,0,0],pts:[2000,8000,0,8000,8000,200]},
  ];
  const blackImage=new PNG({width:400,height:300});annotateZoomFrame(blackImage,black,1200,[0,0,1,1]);
  const blackPixel=(x:number,y:number)=>Array.from(blackImage.data.subarray((y*blackImage.width+x)*4,(y*blackImage.width+x)*4+4));
  assert.deepEqual(blackPixel(200,60),[0,0,0,230]);
  assert.deepEqual(blackPixel(207,150),[0,0,0,255]);
  assert.deepEqual(blackPixel(200,240),[0,0,0,150]);
  const faded=new PNG({width:400,height:300});annotateZoomFrame(faded,{...black,strokes:[black.strokes[2]]},1500,[0,0,1,1]);
  assert.deepEqual(Array.from(faded.data.subarray((240*400+200)*4,(240*400+200)*4+4)),[0,0,0,0]);
});

test('present malformed stroke RGB is rejected before zoom output creation',async()=>{
  const bundle=await bundleFixture(),parent=await mkdtemp(join(tmpdir(),'zoom-rgb-invalid-'));
  try{
    for(const [index,rgb] of [[0,null],[1,[0,0]],[2,[0,0,256]],[3,[0,1.5,0]],[4,['1',2,3]]] as const){
      const value=manifest();value.strokes=[{t0:0,t1:100,fw:100,fh:100,rgb,pts:[0,0,0,10000,10000,100]}];
      await writeFile(join(bundle,'apparent.json'),JSON.stringify(value));
      const out=join(parent,`invalid-${index}`);
      await assert.rejects(runZoom({bundle,at:'1',span:2,frames:1,full:false,out,json:false}),/Malformed stroke RGB/);
      await assert.rejects(import('node:fs/promises').then(fs=>fs.lstat(out)),{code:'ENOENT'});
    }
  }finally{await rm(bundle,{recursive:true,force:true});await rm(parent,{recursive:true,force:true});}
});

test('PNG interlace, malformed geometry and aggregate source pixels fail before output',async()=>{
  const bundle=await bundleFixture(),parent=await mkdtemp(join(tmpdir(),'zoom-bounds-'));
  try{
    const chunk=(name:string,data:Buffer)=>{const head=Buffer.alloc(8);head.writeUInt32BE(data.length,0);head.write(name,4,'ascii');const crc=Buffer.alloc(4);return Buffer.concat([head,data,crc]);};
    const ihdr=Buffer.alloc(13);ihdr.writeUInt32BE(1,0);ihdr.writeUInt32BE(1,4);ihdr[8]=8;ihdr[9]=6;ihdr[12]=1;
    const bomb=Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',ihdr),chunk('IDAT',deflateSync(Buffer.alloc(64*1024*1024))),chunk('IEND',Buffer.alloc(0))]);
    assert.ok(bomb.length<1024*1024);await writeFile(join(bundle,'frames/frame-01.png'),bomb);
    await assert.rejects(runZoom({bundle,at:'1',span:2,frames:1,full:false,out:join(parent,'interlace'),json:false}),/Interlaced PNG is unsupported/);
    const value=manifest();value.strokes=[{t0:0,t1:100,fw:100,fh:100,pts:[0,0,0,1e9,1,1]}];await writeFile(join(bundle,'apparent.json'),JSON.stringify(value));
    await assert.rejects(runZoom({bundle,at:'1',span:2,frames:1,full:false,out:join(parent,'geometry'),json:false}),/Malformed stroke point/);
    await writeFile(join(bundle,'frames/frame-01.png'),png([0,0,0,255]));
    const tiny=manifest();tiny.autocrop={label:'tiny',fullFrames:'frames-full',fullContactSheet:'contact.png',crops:[{index:1,x:1,y:1,width:0.001,height:0.001,kind:'focus',reason:'invalid'}]};await writeFile(join(bundle,'apparent.json'),JSON.stringify(tiny));
    await assert.rejects(runZoom({bundle,at:'1',span:2,frames:1,full:false,out:join(parent,'tiny-crop'),json:false}),/Invalid capture-point autocrop bounds/);
    const extreme=manifest();extreme.marks[0].boxNorm[3]=1e99;await writeFile(join(bundle,'apparent.json'),JSON.stringify(extreme));
    await assert.rejects(runZoom({bundle,at:'1',span:2,frames:1,full:false,out:join(parent,'extreme-mark'),json:false}),/Malformed mark/);
    const manyPoints=manifest();manyPoints.strokes=[0,1].map(()=>({t0:0,t1:100,fw:100,fh:100,pts:new Array(4100*3).fill(0)}));await writeFile(join(bundle,'apparent.json'),JSON.stringify(manyPoints));
    await assert.rejects(runZoom({bundle,at:'1',span:2,frames:1,full:false,out:join(parent,'point-count'),json:false}),/Bundle stroke points exceed zoom render limits/);
    const many=manifest();many.durationMs=11000;many.keyframes=[];
    await mkdir(join(bundle,'frames'),{recursive:true});
    for(let i=0;i<11;i++){const f=`frames/large-${i}.png`;many.keyframes.push({index:i+1,t:i*1000,reason:'scene',file:f});const b=png([0,0,0,255]);b.writeUInt32BE(3000,16);b.writeUInt32BE(2000,20);await writeFile(join(bundle,f),b);}
    await writeFile(join(bundle,'apparent.json'),JSON.stringify(many));
    await assert.rejects(runZoom({bundle,at:'5.5',span:11,frames:11,full:false,out:join(parent,'aggregate'),json:false}),/aggregate pixel limit/);
    await assert.rejects(import('node:fs/promises').then(fs=>fs.lstat(join(parent,'aggregate'))),{code:'ENOENT'});
  }finally{await rm(bundle,{recursive:true,force:true});await rm(parent,{recursive:true,force:true});}
});
