import {test} from 'node:test';
import assert from 'node:assert/strict';
import {fixture,id,file} from '../helpers.mjs';

const files=()=>['evidence/envelope.json','evidence/sealed/part-000000.age',
 'media/envelope.json','media/sealed/part-000000.age'].map(path=>file(path,path));
const body=()=>({format:'apparent-encrypted-v1',delivery:'evidence-first-v1',files:files()});

test('grouped delivery requires explicit matching admission and complete contiguous groups before writes',async()=>{
 const f=await fixture();try {
  const a=await f.identity('grouped-admission'),p='/v1/captures/'+id(165);
  const invalid=[{...body(),delivery:undefined},{...body(),delivery:'unknown'},
   {...body(),format:'plaintext-v1'},{...body(),files:files().slice(0,2)},
   {...body(),files:[...files(),file('envelope.json')]},
   {...body(),files:files().map(x=>x.path==='media/sealed/part-000000.age'?{...x,path:'media/sealed/part-000001.age'}:x)}];
  const ledger=await f.db.prepare('SELECT * FROM r2_budget').first();
  for(const input of invalid)assert.equal((await f.call(p,{token:a.token,method:'POST',body:input})).status,400);
  assert.equal((await f.db.prepare('SELECT count(*) n FROM captures').first()).n,0);
  assert.deepEqual(await f.db.prepare('SELECT * FROM r2_budget').first(),ledger);
  const result=await f.call(p,{token:a.token,method:'POST',body:body()});
  assert.equal(result.status,200);assert.equal((await result.json()).delivery,'evidence-first-v1');
  assert.equal((await f.call(p,{token:a.token,method:'POST',body:body()})).status,200);
  assert.equal((await f.call(p,{token:a.token,method:'POST',body:{...body(),files:files().map(x=>({...x,size:x.size+1}))}})).status,409);
 }finally {await f.close();}
});

test('validated evidence is readable during pending media; auth, full readiness, expiry and accounting remain enforced',async()=>{
 const f=await fixture();try {
  const a=await f.identity('grouped-reader'),other=await f.identity('grouped-foreign'),p='/v1/captures/'+id(166);
  assert.equal((await f.call(p,{token:a.token,method:'POST',body:body()})).status,200);
  const metadata=async()=>{const r=await f.call(p,{token:a.read});assert.equal(r.status,200);return r.json();};
  assert.equal((await metadata()).evidenceReady,false);
  assert.equal((await f.call(p+'/files/evidence/envelope.json',{token:a.read})).status,409);
  const put=async(path:string,raw=path)=>f.call(p+'/files/'+path,{token:a.token,method:'PUT',raw});
  assert.equal((await put('evidence/envelope.json')).status,200);
  assert.equal((await f.call(p+'/files/evidence/envelope.json',{token:a.read})).status,409);
  assert.equal((await put('evidence/sealed/part-000000.age','invalid')).status,400);
  assert.equal((await metadata()).evidenceReady,false);
  assert.equal((await put('evidence/sealed/part-000000.age')).status,200);
  const partial=await metadata();assert.equal(partial.evidenceReady,true);assert.equal(partial.status,'pending');
  assert.equal((await f.call(p+'/files/evidence/envelope.json')).status,401);
  assert.equal((await f.call(p+'/files/evidence/envelope.json',{token:other.read})).status,404);
  assert.equal((await f.call(p+'/files/evidence/envelope.json',{token:a.read})).status,200);
  assert.equal((await f.call(p+'/files/media/envelope.json',{token:a.read})).status,409);
  assert.equal((await f.call(p+'/complete',{token:a.token,method:'POST'})).status,409);
  for(const asset of files().filter(x=>x.path.startsWith('media/')))assert.equal((await put(asset.path)).status,200);
  assert.equal((await f.call(p+'/complete',{token:a.token,method:'POST'})).status,200);
  assert.equal((await metadata()).status,'ready');assert.equal((await metadata()).evidenceReady,true);
  assert.equal((await f.call(p+'/files/media/envelope.json',{token:a.read})).status,200);
  assert.equal((await f.call(p+'/share',{token:a.token,method:'POST'})).status,409);
  // Deletion and expiry remain authoritative even for previously ready evidence.
  assert.equal((await f.call(p,{token:a.token,method:'DELETE'})).status,200);
  assert.equal((await f.call(p+'/files/evidence/envelope.json',{token:a.read})).status,404);
  await (await f.mf.getWorker()).scheduled();
  assert.equal((await f.db.prepare('SELECT physical_used FROM r2_budget').first()).physical_used,0);
  const q='/v1/captures/'+id(167);
  assert.equal((await f.call(q,{token:a.token,method:'POST',body:body()})).status,200);
  await f.db.prepare('UPDATE captures SET pending_until=0 WHERE id=?').bind(id(167)).run();
  assert.equal((await f.call(q,{token:a.read})).status,404);
 }finally {await f.close();}
});
