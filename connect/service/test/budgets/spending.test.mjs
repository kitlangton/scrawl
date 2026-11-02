import {test} from 'node:test';
import assert from 'node:assert/strict';
import {build} from 'esbuild';
import {fixture,id,file} from '../helpers.mjs';

const instrumented=await build({stdin:{resolveDir:process.cwd(),contents:`
 import worker from './src/worker.ts';
 export default {async fetch(request,env){
  let calls=0;
  const BUCKET=new Proxy(env.BUCKET,{get(target,key){const value=target[key];return typeof value==='function'?(...args)=>{if(['put','get','head','list'].includes(key))calls++;return value.apply(target,args);}:value;}});
  const response=await worker.fetch(request,{...env,BUCKET});response.headers.set('X-Test-Metered',String(calls));return response;
 },scheduled:(controller,env)=>worker.scheduled(controller,env)};
`},bundle:true,format:'esm',platform:'browser',conditions:['workerd'],external:['node:*'],write:false});
const setup=()=>fixture({}, {scriptPath:undefined,script:instrumented.outputFiles[0].text});
const budget=f=>f.db.prepare('SELECT * FROM r2_budget WHERE id=1').first();
const reserve=(f,a,n,files=[file()])=>f.call('/v1/captures/'+id(n),{token:a.token,method:'POST',body:{files}});
const put=(f,a,n,raw='hello')=>f.call('/v1/captures/'+id(n)+'/files/summary.md',{token:a.token,method:'PUT',raw});
async function denied(response,code='budget_exhausted'){
 assert.equal(response.status,503);assert.equal((await response.json()).error.code,code);assert.equal(response.headers.get('X-Test-Metered'),'0');
}

test('missing durable budget fails closed before a real R2 PUT',async()=>{
 const f=await setup();try{
 const a=await f.identity('missing');await reserve(f,a,600);
 await f.db.exec('DROP TABLE IF EXISTS r2_budget');
 await denied(await put(f,a,600),'budget_unavailable');
 assert.equal((await f.bucket.list()).objects.length,0);
 assert.equal((await f.db.prepare('SELECT object_key FROM files').first()).object_key,null);
 }finally{await f.close();}
});

test('lifetime A exhaustion rejects a real PUT without publication or R2 work',async()=>{
 const f=await setup();try{
 const a=await f.identity('a-cap');await reserve(f,a,601);
 await f.db.prepare('UPDATE r2_budget SET a_cap=0').run();
 await denied(await put(f,a,601));
 assert.equal((await budget(f)).a_used,0);
 assert.equal((await f.db.prepare('SELECT count(*) AS n FROM attempts').first()).n,0);
 }finally{await f.close();}
});

test('512-file completion reserves all B before any HEAD and preserves pending on denial',async()=>{
 const f=await setup();try{
 const a=await f.identity('b-cap'),files=Array.from({length:512},(_,i)=>file(`frames/frame-${i}.png`,''));
 await reserve(f,a,602,files);
 await f.bucket.put('zero','',{sha256:files[0].sha256});
 await f.db.prepare("UPDATE files SET object_key='zero'").run();
 await f.db.prepare('UPDATE r2_budget SET b_cap=511').run();
 await denied(await f.call('/v1/captures/'+id(602)+'/complete',{token:a.token,method:'POST'}));
 assert.equal((await f.db.prepare('SELECT status FROM captures').first()).status,'pending');
 assert.equal((await budget(f)).b_used,0);
 }finally{await f.close();}
});

test('physical attempt cap rejects a real PUT even with logical quota available',async()=>{
 const f=await setup();try{
 const a=await f.identity('bytes');await reserve(f,a,603);
 await f.db.prepare('UPDATE r2_budget SET physical_cap=4').run();
 await denied(await put(f,a,603));
 assert.equal((await budget(f)).physical_used,0);
 assert.equal((await budget(f)).a_used,0);
 }finally{await f.close();}
});

test('settled failed and duplicate attempts reclaim bytes once, never refund A',async()=>{
 const f=await setup();try{
 const a=await f.identity('release');await reserve(f,a,604);
 assert.equal((await put(f,a,604)).status,200);
 for(const [raw,status] of [['wrong',400],['hi',400],['longer',400],['hello',200]]){
  assert.equal((await put(f,a,604,raw)).status,status);
  assert.equal((await budget(f)).physical_used,5,'terminal deleted temporary bytes must be released');
 }
 assert.equal((await budget(f)).a_used,5);
 assert.equal((await f.db.prepare('SELECT count(*) AS n FROM attempts WHERE terminal=1').first()).n,5);
 await f.call('/v1/captures/'+id(604),{token:a.token,method:'DELETE'});
 for(let i=0;i<2;i++)assert.equal((await(await f.mf.getWorker()).scheduled()).outcome,'ok');
 assert.equal((await budget(f)).physical_used,0);
 assert.equal((await budget(f)).a_used,5);
 assert.equal((await f.db.prepare('SELECT count(*) AS n FROM attempts WHERE released=1').first()).n,5);
 }finally{await f.close();}
});
