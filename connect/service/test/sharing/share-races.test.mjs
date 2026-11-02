import {test} from 'node:test';
import assert from 'node:assert/strict';
import {build} from 'esbuild';
import {readFile} from 'node:fs/promises';
import {setTimeout as delay} from 'node:timers/promises';
import {fixture,id,file} from '../helpers.mjs';

const instrumented=await build({stdin:{resolveDir:process.cwd(),contents:`
 import worker from './src/worker.ts';
 export default {async fetch(request,env){
  let calls=0;
  const BUCKET=new Proxy(env.BUCKET,{get(target,key){const value=target[key];return typeof value==='function'?async(...args)=>{
   if(['put','get','head','list'].includes(key)){calls++;if(request.headers.has('X-Hold-R2')){await env.DB.prepare('UPDATE test_barrier SET entered=1').run();while(!(await env.DB.prepare('SELECT released FROM test_barrier').first()).released)await new Promise(r=>setTimeout(r,5));}}
   return value.apply(target,args);
  }:value;}});
  const DB=request.headers.has('X-Hold-Manifest')?{prepare(sql){const statement=env.DB.prepare(sql);if(!sql.startsWith('SELECT path,size,sha256,object_key FROM files'))return statement;return {bind(...args){const bound=statement.bind(...args);return {async all(){const result=await bound.all();await env.DB.prepare('UPDATE test_barrier SET entered=1').run();while(!(await env.DB.prepare('SELECT released FROM test_barrier').first()).released)await new Promise(r=>setTimeout(r,5));return result;}}}};}}:env.DB;
  const response=await worker.fetch(request,{...env,DB,BUCKET});response.headers.set('X-Metered',String(calls));return response;
 }};
`},bundle:true,format:'esm',platform:'browser',conditions:['workerd'],external:['node:*'],write:false});
async function setup(){
 const f=await fixture({SHARING_ENABLED:'true'},{scriptPath:undefined,script:instrumented.outputFiles[0].text,instances:true});
 await f.db.exec((await readFile('migrations/0003.sql','utf8')).replaceAll('\n',' '));
 await f.db.exec('CREATE TABLE test_barrier(released INTEGER,entered INTEGER); INSERT INTO test_barrier VALUES(0,0)');
 const a=await f.identity('race'),p='/v1/captures/'+id(171);
 assert.equal((await f.call(p,{token:a.token,method:'POST',body:{files:[file()]}})).status,200);
 assert.equal((await f.call(p+'/files/summary.md',{token:a.token,method:'PUT',raw:'hello'})).status,200);
 assert.equal((await f.call(p+'/complete',{token:a.token,method:'POST'})).status,200);
 const grants=await Promise.all(Array.from({length:5},()=>f.call(p+'/share',{token:a.token,method:'POST'})));
 const links=await Promise.all(grants.map(async r=>{assert.equal(r.status,200);return (await r.json()).url;}));assert.equal(new Set(links).size,1);
 return {...f,a,path:new URL(links[0]).pathname,management:(await f.db.prepare('SELECT id FROM shares').first()).id};
}
async function entered(f){for(let n=0;n<200;n++){if((await f.db.prepare('SELECT entered FROM test_barrier').first()).entered)return;await delay(5);}assert.fail('bounded barrier not reached');}
const release=f=>f.db.exec('UPDATE test_barrier SET released=1');

for(const [suffix,header] of [['/files/summary.md','X-Hold-R2'],['/thumbnail.png','X-Hold-R2'],['','X-Hold-R2'],['/manifest.json','X-Hold-Manifest']])test('revocation wins a blocked public read '+(suffix||'HTML'),async()=>{
 const f=await setup();let pending;
 try{
  // Give thumbnail its own real stored bytes without a second app fixture.
  if(suffix==='/thumbnail.png')await f.db.prepare("UPDATE files SET path='contact-sheet.png'").run();
  pending=f.call(f.path+suffix,{headers:{[header]:'1',...(suffix.startsWith('/files/')?{Range:'bytes=0-2'}:{})}});
  await entered(f);
  assert.equal((await f.call('/v1/shares/'+f.management,{token:f.a.token,method:'DELETE'})).status,200);
  await release(f);const r=await pending;assert.ok([409,410].includes(r.status));assert.ok(!(await r.text()).includes('hello'));
 }finally{await release(f);if(pending)await pending;await f.close();}
});

test('public capability GETs race across worker instances within the same durable B allowance',async()=>{
 const f=await setup();let pending;
 try{
  const before=(await f.db.prepare('SELECT b_used FROM r2_budget').first()).b_used;
  await f.db.prepare('UPDATE r2_budget SET b_cap=b_used+1').run();
  pending=f.call(f.path+'/files/summary.md',{headers:{'X-Hold-R2':'1'}});await entered(f);
  const other=await (await f.mf.getWorker('second')).fetch('https://api.example'+f.path+'/files/summary.md');assert.equal(other.status,503);assert.equal(other.headers.get('X-Metered'),'0');
  assert.equal((await f.db.prepare('SELECT b_used FROM r2_budget').first()).b_used,before+1);
  await release(f);assert.equal(await (await pending).text(),'hello');
 }finally{await release(f);if(pending)await pending;await f.close();}
});
