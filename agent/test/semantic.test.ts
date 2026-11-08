import { mkdtemp, mkdir, rm, writeFile, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { hasMicrophoneSamples } from '../dist/bundle.js';
import { semanticSummary } from '../dist/summary.js';

const fixture = JSON.parse(await (await import('node:fs/promises')).readFile(new URL('./fixtures/semantic.json', import.meta.url),'utf8'));
function pcm16(sample=0):Buffer {const b=Buffer.alloc(46);b.write('RIFF',0);b.writeUInt32LE(38,4);b.write('WAVEfmt ',8);b.writeUInt32LE(16,16);b.writeUInt16LE(1,20);b.writeUInt16LE(1,22);b.writeUInt32LE(16000,24);b.writeUInt32LE(32000,28);b.writeUInt16LE(2,32);b.writeUInt16LE(16,34);b.write('data',36);b.writeUInt32LE(2,40);b.writeInt16LE(sample,44);return b;}

test('WAV probe requires a readable PCM16 sample and refuses unsafe paths',async()=>{
  const root=await mkdtemp(join(tmpdir(),'agent-audio-probe-'));
  try {
    assert.equal(await hasMicrophoneSamples(root,'audio.wav'),false);
    await writeFile(join(root,'audio.wav'),Buffer.alloc(44));assert.equal(await hasMicrophoneSamples(root,'audio.wav'),false);
    await writeFile(join(root,'audio.wav'),Buffer.from('corrupt audio'));assert.equal(await hasMicrophoneSamples(root,'audio.wav'),false);
    await writeFile(join(root,'audio.wav'),pcm16(0));assert.equal(await hasMicrophoneSamples(root,'audio.wav'),true);
    await symlink('/etc/passwd',join(root,'linked.wav'));assert.equal(await hasMicrophoneSamples(root,'linked.wav'),false);
    assert.equal(await hasMicrophoneSamples(root,'../outside.wav'),false);
  } finally {await rm(root,{recursive:true,force:true});}
});

test('audio warnings distinguish missing samples, evidence snapshots, iPhone, and disabled capture',()=>{
  const bundle={...fixture,keyframes:[],marks:[],events:[],screens:[],windows:[],contextChanges:[],audio:'audio.wav',transcript:{text:'',segments:[],missing:{reason:'Recognition unavailable.'}}};
  const output=(value:any,evidence=false,audio=false)=>semanticSummary(value,'/synthetic',false,evidence,audio);
  assert.match(output({...bundle,audio:null}),/No microphone audio was saved/);
  assert.doesNotMatch(output({...bundle,audio:null}),/No transcript/);
  assert.doesNotMatch(output(bundle,true),/No microphone audio was saved/);
  assert.match(output(bundle,true),/No transcript: Recognition unavailable\./);
  assert.doesNotMatch(output({...bundle,source:'iphone'},false,false),/microphone audio/);
  assert.doesNotMatch(output({...bundle,context:{microphoneCapture:'off'}},false,false),/microphone audio/);
  assert.match(output(bundle,false,true),/No transcript: Recognition unavailable\./);
});

test('video-only telemetry reports unavailable interactions without inventing capture origin',async()=>{
  const root=await mkdtemp(join(tmpdir(),'agent-video-only-origin-'));
  try {
    const bundle=join(root,'app-output.apparent');await mkdir(bundle);
    const manifest=JSON.stringify({...fixture,interactionTelemetry:'videoOnly',keyframes:[],marks:[],events:[]});
    await writeFile(join(bundle,'apparent.json'),manifest);
    const cli=new URL('../dist/cli.js',import.meta.url).pathname;
    const run=()=>spawnSync(process.execPath,[cli,'inspect',bundle],{encoding:'utf8',env:{...process.env,HOME:root,PATH:''}});
    const result=run();assert.equal(result.status,0,result.stderr);
    assert.match(result.stdout,/Interaction context: click telemetry was not collected\./);
    assert.match(result.stdout,/will not appear in events, marks or click keyframes/);
    assert.doesNotMatch(result.stdout,/timed CLI|recording shortcut/);
    assert.equal(await (await import('node:fs/promises')).readFile(join(bundle,'apparent.json'),'utf8'),manifest);
    await writeFile(join(bundle,'apparent.json'),JSON.stringify({...fixture,interactionTelemetry:undefined}));
    assert.doesNotMatch(run().stdout,/Interaction context: click telemetry/);
  } finally {await rm(root,{recursive:true,force:true});}
});

test('unavailable click telemetry does not establish absent ink or accepted-history state',()=>{
  const strokes=[
    {t0:40,t1:80,fw:16,fh:16,pts:[1000,2000,0,3000,4000,40],rgb:[15,127,240]},
    {t0:100,t1:130,fw:16,fh:16,pts:[5000,5000,0,7000,6000,30]},
  ];
  for(const annotations of [strokes,[]]) {
    const bundle={...fixture,strokes:annotations,interactionTelemetry:'videoOnly'};
    const before=JSON.stringify(bundle);
    const text=semanticSummary(bundle,'/synthetic');
    assert.match(text,/click telemetry was not collected/);
    assert.doesNotMatch(text,/Shift ink were not collected|ink was not collected|Accepted strokes:/);
    assert.equal(JSON.stringify(bundle),before);
  }
});

test('config agentContext is read-only, defaults off, and explicit experiment enables it',async()=>{
  const root=await mkdtemp(join(tmpdir(),'agent-experiment-config-'));
  try {
    const bundle=join(root,'capture.apparent'),config=join(root,'.config','apparent');await mkdir(bundle);await mkdir(config,{recursive:true});
    await writeFile(join(bundle,'apparent.json'),JSON.stringify(fixture));await writeFile(join(bundle,'summary.md'),'stored');
    const manifest=await (await import('node:fs/promises')).readFile(join(bundle,'apparent.json'));
    const cli=new URL('../dist/cli.js',import.meta.url).pathname;
    const run=(...args:string[])=>spawnSync(process.execPath,[cli,'inspect',bundle,...args],{encoding:'utf8',env:{...process.env,HOME:root}});
    await writeFile(join(config,'config.json'),JSON.stringify({experiments:{agentContext:false,unrelated:{bad:true}},keep:'value'}));
    const off=run();assert.equal(off.status,0,off.stderr);assert.match(off.stdout,/#1 \(0:02\) “Open the menu, then choose”/);
    const explicit=run('--experiment','agentContext');assert.equal(explicit.status,0,explicit.stderr);assert.match(explicit.stdout,/✎① 0:02 pin 1/);
    await writeFile(join(config,'config.json'),JSON.stringify({experiments:{agentContext:true,unrelated:'unknown'},keep:'value'}));
    const on=run();assert.equal(on.status,0,on.stderr);assert.match(on.stdout,/✎① 0:02 pin 1/);
    assert.deepEqual(await (await import('node:fs/promises')).readFile(join(bundle,'apparent.json')),manifest);
    assert.equal(run('--stored-summary','--experiment','agentContext').status,2);
  } finally {await rm(root,{recursive:true,force:true});}
});
