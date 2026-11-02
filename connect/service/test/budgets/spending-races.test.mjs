import {test} from 'node:test';
import assert from 'node:assert/strict';
import {build} from 'esbuild';
import {readFile,readdir} from 'node:fs/promises';
import {setTimeout as delay} from 'node:timers/promises';
import {fixture,id,file} from '../helpers.mjs';

const instrumented=await build({stdin:{resolveDir:process.cwd(),contents:`
 import worker from './src/worker.ts';
 export default {
  async fetch(request,env){
   let calls=0;
   const BUCKET=new Proxy(env.BUCKET,{get(target,key){const value=target[key];return typeof value==='function'?async(...args)=>{
    if(key==='delete'&&request.headers.has('X-Fail-Delete'))throw new Error('delete failed');
    if(['put','get','head','list'].includes(key)){calls++;if(request.headers.has('X-Hold-R2'))while(!(await env.DB.prepare('SELECT released FROM test_barrier').first()).released)await new Promise(resolve=>setTimeout(resolve,5));}
    return value.apply(target,args);
   }:value;}});
   const DB=request.headers.has('X-Crash-Grant')?{prepare:sql=>env.DB.prepare(sql),async batch(items){await env.DB.batch(items);throw new Error('crash after grant');}}:env.DB;
   const response=await worker.fetch(request,{...env,BUCKET,DB});response.headers.set('X-Metered',String(calls));return response;
  },async scheduled(controller,env){
   const BUCKET={async delete(keys){
    await env.BUCKET.delete(keys);
    if((await env.DB.prepare('SELECT hold_delete FROM test_barrier').first()).hold_delete){
     await env.DB.prepare('UPDATE test_barrier SET delete_done=1').run();
     while(!(await env.DB.prepare('SELECT released FROM test_barrier').first()).released)await new Promise(resolve=>setTimeout(resolve,5));
    }
   }};
   return worker.scheduled(controller,{...env,BUCKET});
  }
 };
`},bundle:true,format:'esm',platform:'browser',conditions:['workerd'],external:['node:*'],write:false});
const setup=async(instances=false)=>{const f=await fixture({}, {scriptPath:undefined,script:instrumented.outputFiles[0].text,instances});await f.db.exec('CREATE TABLE test_barrier(released INTEGER,hold_delete INTEGER,delete_done INTEGER); INSERT INTO test_barrier VALUES(0,0,0)');return f;};
const release=f=>f.db.exec('UPDATE test_barrier SET released=1');
const ledger=f=>f.db.prepare('SELECT * FROM r2_budget').first();
const path=n=>'/v1/captures/'+id(n);
const reserve=(f,a,n,files=[file()])=>f.call(path(n),{token:a.token,method:'POST',body:{files}});
const put=(f,a,n,headers={},raw='hello')=>f.call(path(n)+'/files/summary.md',{token:a.token,method:'PUT',raw,headers});
async function until(read,predicate){for(let i=0;i<200;i++){const value=await read();if(predicate(value))return value;await delay(5);}assert.fail('bounded barrier not reached');}
async function reject(r,code='budget_exhausted'){assert.equal(r.status,503);assert.equal((await r.json()).error.code,code);assert.equal(r.headers.get('X-Metered'),'0');}

for(const cap of ['a_cap','physical_cap'])test(cap+' race across accounts and Worker instances grants exactly one PUT',async()=>{
 const f=await setup(true);try{
 const a=await f.identity('one'),b=await f.identity('two');await reserve(f,a,700);await reserve(f,b,701);
 await f.db.prepare(`UPDATE r2_budget SET ${cap}=?`).bind(cap==='a_cap'?1:5).run();
 const first=put(f,a,700,{'X-Hold-R2':'1'});
 await until(()=>ledger(f),b=>b.a_used===1);
 const second=await(await f.mf.getWorker('second')).fetch('https://api.example'+path(701)+'/files/summary.md',{method:'PUT',headers:{Authorization:'Bearer '+b.token},body:'hello'});
 await reject(second);
 assert.equal((await ledger(f)).physical_used,5);
 assert.equal((await f.db.prepare('SELECT count(*) AS n FROM attempts').first()).n,1);
 await release(f);assert.equal((await first).status,200);
 assert.equal((await f.bucket.list()).objects.length,1);
 assert.equal((await ledger(f)).a_used,1);
 }finally{await f.close();}
});

test('global B race across instances grants one GET',async()=>{
 const f=await setup(true);try{
 const a=await f.identity('read-one'),b=await f.identity('read-two');
 for(const [owner,n] of [[a,702],[b,703]]){await reserve(f,owner,n);await put(f,owner,n);assert.equal((await f.call(path(n)+'/complete',{token:owner.token,method:'POST'})).status,200);}
 const before=(await ledger(f)).b_used;await f.db.prepare('UPDATE r2_budget SET b_cap=b_used+1').run();
 const first=f.call(path(702)+'/files/summary.md',{token:a.read,headers:{'X-Hold-R2':'1'}});
 await until(()=>ledger(f),b=>b.b_used===before+1);
 const second=await(await f.mf.getWorker('second')).fetch('https://api.example'+path(703)+'/files/summary.md',{headers:{Authorization:'Bearer '+b.read}});
 await reject(second);
 await release(f);assert.equal(await(await first).text(),'hello');
 await reject(await f.call(path(702)+'/files/summary.md',{token:a.read}));
 }finally{await f.close();}
});

test('early missing completion consumes the whole bulk grant, ready retry consumes none',async()=>{
 const f=await setup();try{
 const a=await f.identity('heads'),files=[file('frames/frame-0.png',''),file('frames/frame-1.png','')];await reserve(f,a,704,files);
 await f.db.prepare("UPDATE files SET object_key='absent'").run();
 for(let n=1;n<=2;n++){const r=await f.call(path(704)+'/complete',{token:a.token,method:'POST'});assert.equal(r.status,409);assert.equal(r.headers.get('X-Metered'),'1');assert.equal((await ledger(f)).b_used,n*2);}
 await f.bucket.put('absent','',{sha256:files[0].sha256});
 assert.equal((await f.call(path(704)+'/complete',{token:a.token,method:'POST'})).status,200);
 await f.db.prepare('UPDATE r2_budget SET b_cap=b_used').run();const before=await ledger(f);
 const ready=await f.call(path(704)+'/complete',{token:a.token,method:'POST'});assert.equal(ready.status,200);assert.equal(ready.headers.get('X-Metered'),'0');assert.deepEqual(await ledger(f),before);
 }finally{await f.close();}
});

test('metadata, invalid, foreign, read-only, waitlist, revoked and expired routes do not drain budget',async()=>{
 const f=await setup();try{
 const a=await f.identity('owner'),b=await f.identity('foreign'),w=await f.identity('wait','waitlist');await reserve(f,a,705);const before=await ledger(f);
 for(const [route,options,status] of [
  [path(705),{token:a.read},200],['/v1/account',{token:a.read},200],
  [path(705)+'/files/summary.md',{method:'PUT',token:a.read,raw:'hello'},403],
  [path(705)+'/files/summary.md',{method:'PUT',token:b.token,raw:'hello'},404],
  [path(705)+'/files/summary.md',{method:'PUT',token:w.token,raw:'hello'},403],
  [path(705)+'/files/secret.txt',{method:'PUT',token:a.token,raw:'hello'},400],
  [path(705)+'/files/summary.md',{method:'PUT',raw:'hello'},401],
  [path(705)+'/files/summary.md',{token:a.read},409]
 ]){const r=await f.call(route,options);assert.equal(r.status,status);assert.equal(r.headers.get('X-Metered'),'0');}
 await f.call('/v1/token/revoke',{token:b.token,method:'POST'});
 assert.equal((await put(f,b,705)).status,401);
 await f.db.prepare('UPDATE captures SET expires_at=0').run();assert.equal((await put(f,a,705)).status,404);
 assert.deepEqual(await ledger(f),before);
 }finally{await f.close();}
});

test('missing row, invalid ledger and reservation DB errors fail closed for PUT/HEAD/GET',async()=>{
 const f=await setup();try{
 const a=await f.identity('bad-state');await reserve(f,a,706);await put(f,a,706);await f.call(path(706)+'/complete',{token:a.token,method:'POST'});await reserve(f,a,707);
 const saved=await ledger(f);
 for(const damage of ["UPDATE r2_budget SET a_cap='invalid'",'DELETE FROM r2_budget',"CREATE TRIGGER budget_fault BEFORE UPDATE ON r2_budget BEGIN SELECT RAISE(ABORT,'injected database error'); END"]){
  await f.db.exec(damage);
  await reject(await put(f,a,706),'budget_unavailable');
  await reject(await f.call(path(706)+'/files/summary.md',{token:a.read}),'budget_unavailable');
  await reject(await f.call(path(707)+'/complete',{token:a.token,method:'POST'}),'budget_unavailable');
  await f.db.exec('DROP TRIGGER IF EXISTS budget_fault');
  await f.db.prepare('INSERT OR REPLACE INTO r2_budget VALUES(?,?,?,?,?,?,?)').bind(...Object.values(saved)).run();
 }
 }finally{await f.close();}
});

test('crash after durable grant before PUT leaks capacity safely and survives reload/migration rerun',async()=>{
 const f=await setup();try{
 const a=await f.identity('crash-grant');await reserve(f,a,708);
 await reject(await put(f,a,708,{'X-Crash-Grant':'1'}),'budget_unavailable');
 assert.equal((await ledger(f)).physical_used,5);assert.equal((await ledger(f)).a_used,1);
 assert.equal((await f.bucket.list()).objects.length,0);
 await f.call(path(708),{token:a.token,method:'DELETE'});
 for(let i=0;i<2;i++)assert.equal((await(await f.mf.getWorker()).scheduled()).outcome,'ok');
 assert.equal((await ledger(f)).physical_used,5,'absent but uncertain key cannot free capacity');
 const before=await ledger(f);await f.reload();assert.deepEqual(await ledger(f),before);
 await assert.rejects(f.db.exec((await readFile('migrations/0002.sql','utf8')).replaceAll('\n',' ')));
 assert.deepEqual(await ledger(f),before);
 await f.db.prepare('UPDATE r2_budget SET a_cap=a_used,physical_cap=physical_used').run();await reserve(f,a,709);
 await reject(await f.call(path(709)+'/files/summary.md?period=2099-01&reset=true',{token:a.token,method:'PUT',raw:'hello',headers:{'X-Budget-Period':'2099-01'}}));
 }finally{await f.close();}
});

test('uncertain PUT finishes after cleanup delete: bytes remain charged even after absent-key sweeps',async()=>{
 const f=await setup();let controller;try{
 const a=await f.identity('late'),p=path(710);await reserve(f,a,710);
 await f.db.exec("CREATE TRIGGER terminal_fault BEFORE UPDATE OF terminal ON attempts BEGIN SELECT RAISE(ABORT,'crash before terminal marker'); END");
 const stream=new ReadableStream({start(c){controller=c;c.enqueue(new TextEncoder().encode('he'));}});
 const upload=f.mf.dispatchFetch('https://api.example'+p+'/files/summary.md',{method:'PUT',headers:{Authorization:'Bearer '+a.token,'X-Fail-Delete':'1'},body:stream,duplex:'half'});
 await until(()=>f.db.prepare('SELECT * FROM attempts').first(),Boolean);
 await f.call(p,{token:a.token,method:'DELETE'});
 assert.equal((await(await f.mf.getWorker()).scheduled()).outcome,'ok');
 assert.equal((await ledger(f)).physical_used,5);assert.equal((await f.bucket.list()).objects.length,0);
 controller.enqueue(new TextEncoder().encode('llo'));controller.close();controller=null;
 assert.equal((await upload).status,500);
 assert.equal((await f.bucket.list()).objects.length,1,'real R2 PUT finished after deletion');
 assert.equal((await f.db.prepare('SELECT terminal,released FROM attempts').first()).terminal,0);
 for(let i=0;i<2;i++)assert.equal((await(await f.mf.getWorker()).scheduled()).outcome,'ok');
 assert.equal((await f.bucket.list()).objects.length,0);assert.equal((await ledger(f)).physical_used,5);
 }finally{if(controller)controller.close();await f.close();}
});

test('terminal deleted objects release once after delete/stamp failures, even at all caps',async()=>{
 const f=await setup();try{
 const a=await f.identity('terminal');await reserve(f,a,711);await put(f,a,711);
 await f.db.prepare('UPDATE r2_budget SET a_cap=a_used,b_cap=b_used,physical_cap=physical_used').run();
 await reject(await put(f,a,711));await f.call(path(711),{token:a.token,method:'DELETE'});
 await f.db.exec("CREATE TRIGGER stamp_fault BEFORE UPDATE OF cleaned_at ON attempts BEGIN SELECT RAISE(ABORT,'stamp failed'); END");
 assert.equal((await(await f.mf.getWorker()).scheduled()).outcome,'exception');
 assert.equal((await f.bucket.list()).objects.length,0);assert.equal((await ledger(f)).physical_used,5);assert.equal((await f.db.prepare('SELECT released FROM attempts').first()).released,0);
 await f.db.exec('DROP TRIGGER stamp_fault');
 for(let i=0;i<2;i++)assert.equal((await(await f.mf.getWorker()).scheduled()).outcome,'ok');
 assert.equal((await ledger(f)).physical_used,0);assert.equal((await ledger(f)).a_used,1);
 }finally{await f.close();}
});

test('physical reservations cover logical eviction and parallel temporary retries',async()=>{
 const f=await setup();try{
 const a=await f.identity('evict'),b=await f.identity('other');await reserve(f,a,712);await put(f,a,712);
 // A new logical full-account reservation tombstones the old object, not its physical charge.
 const full=Array.from({length:6},(_,i)=>({...file(`frames/frame-${i}.png`),size:i===5?50000000:90000000}));
 await reserve(f,a,713,full);assert.equal((await f.db.prepare('SELECT status FROM captures WHERE id=?').bind(id(712)).first()).status,'deleted');
 assert.equal((await ledger(f)).physical_used,5);
 await reserve(f,b,714);await put(f,b,714);await f.db.prepare('UPDATE r2_budget SET physical_cap=15').run();
 const retry=put(f,b,714,{'X-Hold-R2':'1'});await until(()=>ledger(f),b=>b.physical_used===15);
 await reject(await put(f,b,714));assert.equal((await ledger(f)).physical_used,15);
 await release(f);assert.equal((await retry).status,200);assert.equal((await ledger(f)).physical_used,10);
 assert.equal((await(await f.mf.getWorker()).scheduled()).outcome,'ok');assert.equal((await ledger(f)).physical_used,5);
 }finally{await f.close();}
});

test('metered R2 source inventory has no raw bypass or LIST path',async()=>{
 const sources=(await readdir('src')).filter(name=>/\.(?:ts|js)$/.test(name)&&name!=='types.ts');
 assert.ok(sources.length>0,'runtime source inventory must not be empty');
 assert.ok(sources.includes('storage.ts'),'inventory must include the storage guard');
 for(const name of sources)if(name!=='storage.ts')assert.doesNotMatch(await readFile('src/'+name,'utf8'),/\bBUCKET\b/,name+' must use the storage guard');
 const source=await readFile('src/storage.ts','utf8');
 assert.deepEqual([...source.matchAll(/env\.BUCKET\.(\w+)\(/g)].map(m=>m[1]).sort(),['delete','get','head','put']);
 assert.doesNotMatch(source,/\b(?:month|period|reset)\s*[:=(]/i);
});

test('cleanup cannot release a PUT that becomes terminal after its deletion snapshot',async()=>{
 const f=await setup();let controller;try{
 const a=await f.identity('snapshot');await reserve(f,a,715);
 const stream=new ReadableStream({start(c){controller=c;c.enqueue(new TextEncoder().encode('he'));}});
 const upload=f.mf.dispatchFetch('https://api.example'+path(715)+'/files/summary.md',{method:'PUT',headers:{Authorization:'Bearer '+a.token,'X-Fail-Delete':'1'},body:stream,duplex:'half'});
 await until(()=>f.db.prepare('SELECT * FROM attempts').first(),Boolean);
 await f.call(path(715),{token:a.token,method:'DELETE'});
 await f.db.exec('UPDATE test_barrier SET hold_delete=1');
 const sweep=(await f.mf.getWorker()).scheduled();
 await until(()=>f.db.prepare('SELECT delete_done FROM test_barrier').first(),b=>b.delete_done===1);
 controller.enqueue(new TextEncoder().encode('llo'));controller.close();controller=null;
 assert.equal((await upload).status,500); // injected deletion failure after lost publication
 assert.equal((await f.db.prepare('SELECT terminal FROM attempts').first()).terminal,1);
 assert.equal((await f.bucket.list()).objects.length,1);
 await release(f);assert.equal((await sweep).outcome,'ok');
 assert.equal((await ledger(f)).physical_used,5,'delete preceded PUT completion, so its bytes are still charged');
 assert.equal((await f.db.prepare('SELECT released FROM attempts').first()).released,0);
 assert.equal((await(await f.mf.getWorker()).scheduled()).outcome,'ok');
 assert.equal((await ledger(f)).physical_used,0);assert.equal((await f.bucket.list()).objects.length,0);
 }finally{if(controller)controller.close();await f.close();}
});

test('terminal deletion failure keeps physical charge until cleanup succeeds',async()=>{
 const f=await setup();try{
 const a=await f.identity('delete-failure');await reserve(f,a,716);await put(f,a,716);
 const retry=await put(f,a,716,{'X-Fail-Delete':'1'});assert.equal(retry.status,500);
 assert.equal((await ledger(f)).physical_used,10);assert.equal((await f.bucket.list()).objects.length,2);
 await f.db.exec('UPDATE attempts SET created_at=0');
 for(let i=0;i<2;i++)assert.equal((await(await f.mf.getWorker()).scheduled()).outcome,'ok');
 assert.equal((await ledger(f)).physical_used,5);assert.equal((await ledger(f)).a_used,2);
 assert.equal((await f.bucket.list()).objects.length,1);
 }finally{await f.close();}
});
