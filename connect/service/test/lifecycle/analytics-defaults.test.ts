// Default capability is on; every account still needs its own affirmative choice.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {fixture} from '../helpers.mjs';
const noticeVersion='product-analytics-v1';
async function setup(bindings={}) {
 const sent=[];
 const f=await fixture({POSTHOG_PROJECT_TOKEN:'phc_synthetic',...bindings},{outboundService:async request=>{
  assert.equal(request.url,'https://eu.i.posthog.com/batch/');sent.push(await request.json());return Response.json({status:'Ok'});
 }});
 await f.db.exec((await readFile('migrations/0006.sql','utf8')).replaceAll('\n',' '));
 return {...f,sent};
}
const read=async(f,a)=>await(await f.call('/v1/analytics/consent',a)).json();
const choose=(f,a,consented,revision)=>f.call('/v1/analytics/consent',{...a,method:'POST',body:{consented,revision,noticeVersion}});
async function device(f,a){const start=await(await f.call('/v1/device/start',{method:'POST',body:{scope:'upload'}})).json();assert.equal((await f.call('/v1/device/approve',{...a,method:'POST',body:{userCode:start.userCode}})).status,200);}
test('configured collector defaults enabled, production and EU without implicitly enrolling accounts',async()=>{
 const f=await setup();try{
  await f.db.exec((await readFile('migrations/0007.sql','utf8')).replaceAll('\n',' '));
  const a=await f.identity('default-consenting'),b=await f.identity('default-opted-out');
  assert.deepEqual(await read(f,a),{available:true,consented:false,revision:null,noticeVersion});
  await device(f,b);await(await f.mf.getWorker()).scheduled();assert.deepEqual(f.sent,[]);
  assert.equal((await choose(f,a,true,null)).status,200);
  await device(f,a);await device(f,b);await(await f.mf.getWorker()).scheduled();
  assert.equal(f.sent.length,1);assert.ok(f.sent[0].batch.some(e=>e.event==='connect_device_connected'));
  assert.ok(f.sent[0].batch.every(e=>e.properties.distinct_id==='account:'+a.id&&e.properties.environment==='production'));
  const state=await read(f,a);assert.equal((await choose(f,a,false,state.revision)).status,200);
  await device(f,a);await(await f.mf.getWorker()).scheduled();assert.equal(f.sent.length,1);
  assert.equal((await f.db.prepare('SELECT count(*) n FROM analytics_outbox').first()).n,0);
 }finally{await f.close();}
});
test('additive default migration preserves decisions, withdrawal tombstones and spent budgets',async()=>{
 const f=await setup();try{
  await f.db.exec('UPDATE analytics_control SET enabled=1');
  const a=await f.identity('retained-choice'),b=await f.identity('withdrawn-choice');
  await choose(f,a,true,null);await choose(f,b,false,null);
  await device(f,a);
  const queued=(await f.db.prepare('SELECT * FROM analytics_outbox ORDER BY id').all()).results;
  assert.equal(queued.length,1);
  const before=(await f.db.prepare('SELECT * FROM analytics_choices ORDER BY account_id').all()).results;
  const consents=(await f.db.prepare('SELECT * FROM analytics_consent').all()).results;
  await f.db.exec('UPDATE r2_budget SET a_used=17,b_used=23,physical_used=1234 WHERE id=1');
  const budget=(await f.db.prepare('SELECT * FROM r2_budget').all()).results;
  await f.db.exec('UPDATE analytics_control SET enabled=0,dropped=7');
  await f.db.exec((await readFile('migrations/0007.sql','utf8')).replaceAll('\n',' '));
  assert.deepEqual(await f.db.prepare('SELECT enabled,dropped FROM analytics_control').first(),{enabled:1,dropped:7});
  assert.deepEqual((await f.db.prepare('SELECT * FROM analytics_choices ORDER BY account_id').all()).results,before);
  assert.deepEqual((await f.db.prepare('SELECT * FROM analytics_consent').all()).results,consents);
  assert.deepEqual((await f.db.prepare('SELECT * FROM analytics_outbox ORDER BY id').all()).results,queued);
  assert.deepEqual((await f.db.prepare('SELECT * FROM r2_budget').all()).results,budget);
  assert.equal((await choose(f,b,true,null)).status,409);
 }finally{await f.close();}
});
test('explicit kill switch and malformed configuration still fail closed',async()=>{
 for(const bindings of [{POSTHOG_ENABLED:'false'},{POSTHOG_ENABLED:'invalid'},{POSTHOG_HOST:'https://other.example'},{POSTHOG_ENVIRONMENT:'invalid'},{POSTHOG_PROJECT_TOKEN:''}]){
  const f=await setup(bindings);try{
   await f.db.exec('UPDATE analytics_control SET enabled=1');const a=await f.identity('explicit-off');
   assert.equal((await read(f,a)).available,false);assert.equal((await choose(f,a,true,null)).status,503);
   assert.equal((await choose(f,a,false,null)).status,200);await device(f,a);await(await f.mf.getWorker()).scheduled();assert.deepEqual(f.sent,[]);
  }finally{await f.close();}
 }
});
