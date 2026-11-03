// Public, account-owned consent boundary: actual Worker, D1 and intercepted transport.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {fixture} from '../helpers.mjs';
const noticeVersion='product-analytics-v1';
async function setup(migrate=true){
 const sent=[];
 const f=await fixture({POSTHOG_ENABLED:'true',POSTHOG_PROJECT_TOKEN:'phc_synthetic',POSTHOG_HOST:'https://eu.i.posthog.com',POSTHOG_ENVIRONMENT:'synthetic'},{outboundService:async request=>{assert.equal(new URL(request.url).hostname,'eu.i.posthog.com');sent.push(await request.json());return Response.json({status:'Ok'});}});
 if(migrate){await f.db.exec((await readFile('migrations/0006.sql','utf8')).replaceAll('\n',' '));await f.db.exec('UPDATE analytics_control SET enabled=1');}
 return {...f,sent};
}
const read=(f,a)=>f.call('/v1/analytics/consent',a);
const choice=(f,a,consented,revision,version=noticeVersion)=>f.call('/v1/analytics/consent',{...a,method:'POST',body:{consented,revision,noticeVersion:version}});
async function device(f,a){const start=await(await f.call('/v1/device/start',{method:'POST',body:{scope:'upload'}})).json();assert.equal((await f.call('/v1/device/approve',{...a,method:'POST',body:{userCode:start.userCode}})).status,200);}

test('account consent API: explicit choice, account isolation, withdrawal and stale opt-in suppression',async()=>{
 const f=await setup();try{
  const a=await f.identity('consenting-account'),b=await f.identity('not-consenting-account');
  const initial=await read(f,a);assert.equal(initial.status,200);
  assert.deepEqual(await initial.json(),{available:true,consented:false,revision:null,noticeVersion});
  assert.equal((await f.db.prepare('SELECT count(*) n FROM analytics_consent').first()).n,0);
  assert.equal((await f.db.prepare('SELECT count(*) n FROM analytics_choices').first()).n,0,'reads never enroll or create a preference');
  assert.equal((await choice(f,a,true,null,'obsolete-notice')).status,400);
  const allowed=await choice(f,a,true,null);assert.equal(allowed.status,200);
  const on=await allowed.json();assert.equal(on.consented,true);assert.equal(typeof on.revision,'string');
  const consent=await f.db.prepare('SELECT * FROM analytics_consent WHERE account_id=?').bind(a.id).first();assert.equal(consent.notice_version,noticeVersion);
  await device(f,a);await device(f,b);
  assert.equal((await f.db.prepare('SELECT count(*) n FROM analytics_outbox').first()).n,1,'only opted-in authoritative action queues');
  const repeated=await choice(f,a,true,on.revision);assert.equal(repeated.status,200);
  assert.equal((await f.db.prepare('SELECT consent_id FROM analytics_consent WHERE account_id=?').bind(a.id).first()).consent_id,consent.consent_id);
  assert.equal((await f.db.prepare('SELECT count(*) n FROM analytics_outbox').first()).n,1,'an identical positive choice must not purge admitted events');
  const withdrawn=await choice(f,a,false,on.revision);assert.equal(withdrawn.status,200);
  const off=await withdrawn.json();assert.equal(off.consented,false);assert.notEqual(off.revision,on.revision);
  assert.equal((await f.db.prepare('SELECT count(*) n FROM analytics_outbox').first()).n,0,'withdrawal purges pending transport');
  assert.equal((await choice(f,a,true,null)).status,409,'a delayed first opt-in cannot overwrite withdrawal');
  assert.equal((await choice(f,a,true,on.revision)).status,409,'a stale saved choice cannot restore consent');
  await device(f,a);await(await f.mf.getWorker()).scheduled();assert.equal(f.sent.length,0,'post-withdrawal actions never reach transport');
  assert.equal((await read(f,b)).status,200);assert.equal((await(await read(f,b)).json()).consented,false);
  const renewed=await choice(f,a,true,off.revision);assert.equal(renewed.status,200);
  const next=await f.db.prepare('SELECT * FROM analytics_consent WHERE account_id=?').bind(a.id).first();assert.notEqual(next.consent_id,consent.consent_id);
  // Operator-recorded withdrawal also invalidates a stale browser decision.
  const current=await renewed.json();await f.db.prepare('DELETE FROM analytics_consent WHERE account_id=?').bind(a.id).run();
  assert.equal((await choice(f,a,true,current.revision)).status,409);
 }finally{await f.close();}
});

test('account consent API: cookie/CSRF ownership and migration/config failures stay fail-closed',async()=>{
 const f=await setup();try{
  const a=await f.identity('owner');
  assert.equal((await read(f,{})).status,401);
  assert.equal((await read(f,{token:a.token})).status,401,'bearers cannot manage human consent');
  assert.equal((await choice(f,{cookie:a.cookie},true,null)).status,403);
  assert.equal((await choice(f,{...a,origin:'https://evil.example'},true,null)).status,403);
  assert.equal((await f.call('/v1/analytics/consent',{...a,method:'POST',body:{consented:true,revision:null,noticeVersion,accountId:'another-account'}})).status,400);
  await f.db.exec('UPDATE analytics_control SET enabled=0');
  assert.equal((await choice(f,a,true,null)).status,503,'cannot grant new collection while globally unavailable');
  assert.equal((await choice(f,a,false,null)).status,200,'withdrawal remains available while delivery is disabled');
 }finally{await f.close();}
 const missing=await setup(false);try{const a=await missing.identity('missing-migration');assert.equal((await read(missing,a)).status,503);assert.equal((await choice(missing,a,true,null)).status,503);}finally{await missing.close();}
});
