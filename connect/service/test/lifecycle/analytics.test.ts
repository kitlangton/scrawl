import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {build} from 'esbuild';
import {Miniflare} from 'miniflare';
import {fixture,digest,id,file} from '../helpers.mjs';
const settings={POSTHOG_ENABLED:'true',POSTHOG_PROJECT_TOKEN:'phc_synthetic',POSTHOG_HOST:'https://eu.i.posthog.com',POSTHOG_ENVIRONMENT:'synthetic'};
// Generated public-only wire fixture; no endpoint private key is retained.
const encryptionKey={version:1,recipient:'age1jyg59kq02njlzz0h99thxywpltpp2ywtsgef3h8sawepwnglyq3qqz5957',signing:'e9a2f3b2c655bc429dda6c8e5486f3f776ad3d13ea41662ae089176bbb1604ba'};
async function setup(bindings={},withoutAnalytics=false,runtime={}) {
 const provider={status:200,ack:{status:'Ok'},bodies:[]};
 const f=await fixture({...settings,...bindings},{...runtime,outboundService:async request=>{
  if(request.url==='https://accounts.google.com/.well-known/openid-configuration')return Response.json({issuer:'https://accounts.google.com',authorization_endpoint:'https://accounts.google.com/o/oauth2/v2/auth',token_endpoint:'https://oauth2.googleapis.com/token',jwks_uri:'https://www.googleapis.com/oauth2/v3/certs',id_token_signing_alg_values_supported:['RS256']});
  assert.equal(request.url,'https://eu.i.posthog.com/batch/');
  provider.bodies.push(await request.json());
  if(provider.beforeResponse)await provider.beforeResponse();
  return Response.json(provider.ack,{status:provider.status});
 }});
 if(!withoutAnalytics)await f.db.exec((await readFile('migrations/0006.sql','utf8')).replaceAll('\n',' '));
 return {...f,provider};
}
const enroll=async(f,account,at=Date.now())=>f.db.prepare('INSERT INTO analytics_consent(account_id,consented_at) VALUES(?,?)').bind(account,at).run();
const accountId=subject=>digest('https://accounts.google.com/'+subject);
const queued=async f=>(await f.db.prepare('SELECT * FROM analytics_outbox ORDER BY id').all()).results;
const sessionId=async(f,a)=>(await(await f.call('/v1/session',{cookie:a.cookie})).json()).account.analyticsId;
const sweep=async f=>assert.equal((await(await f.mf.getWorker()).scheduled()).outcome,'ok');
test('account pilot consent: global enablement alone never collects transitions or snapshots',async()=>{
 const f=await setup();try{
  await f.db.exec('UPDATE analytics_control SET enabled=1');
  const a=await f.identity(digest('not enrolled'));
  const device=await(await f.call('/v1/device/start',{method:'POST',body:{scope:'upload'}})).json();
  assert.equal((await f.call('/v1/device/approve',{cookie:a.cookie,csrf:a.csrf,origin:a.origin,method:'POST',body:{userCode:device.userCode}})).status,200);
  const path=await upload(f,a,90);
  assert.equal((await f.call(path+'/complete',{token:a.token,method:'POST'})).status,200);
  assert.deepEqual(await queued(f),[],'no signup, device or upload event without account consent');
  assert.equal(await sessionId(f,a),null);
  await sweep(f);
  assert.deepEqual(await queued(f),[],'no storage snapshot without account consent');
  assert.equal(f.provider.bodies.length,0);
  assert.equal((await f.db.prepare('SELECT dropped FROM analytics_control').first()).dropped,0);
 }finally{await f.close();}
});
async function upload(f,a,n,text='private evidence') {
 const path='/v1/captures/'+id(n);
 assert.equal((await f.call(path,{token:a.token,method:'POST',body:{files:[file('summary.md',text)]}})).status,200);
 assert.equal((await f.call(path+'/files/summary.md',{token:a.token,method:'PUT',raw:text})).status,200);
 return path;
}
test('account pilot consent: withdrawal purges pending events, blocks retries and re-enrolment cannot restore old consent',async()=>{
 const f=await setup();try{
  await f.db.exec('UPDATE analytics_control SET enabled=1');
  const a=await f.identity(digest('withdraw'));
  await enroll(f,a.id);assert.deepEqual(await queued(f),[],'enrolment does not backfill signup');
  const path=await upload(f,a,91);
  assert.equal((await f.call(path+'/complete',{token:a.token,method:'POST'})).status,200);
  f.provider.status=503;await sweep(f);
  const old=await queued(f);assert.equal(old.length,2,'one upload and one snapshot');
  assert.equal(await sessionId(f,a),'account:'+a.id);
  await f.db.prepare('DELETE FROM analytics_consent WHERE account_id=?').bind(a.id).run();
  assert.deepEqual(await queued(f),[],'withdrawal removes the queue in the same DB operation');
  assert.equal(await sessionId(f,a),null);
  const other=await upload(f,a,92);
  assert.equal((await f.call(other+'/complete',{token:a.token,method:'POST'})).status,200);
  await f.db.exec('UPDATE analytics_control SET snapshot_day=-1');
  await sweep(f);assert.deepEqual(await queued(f),[]);assert.equal(f.provider.bodies.length,1);
  await enroll(f,a.id);
  assert.equal(await sessionId(f,a),'account:'+a.id);
  // Simulate a stale queue restore. Even a later opt-in must not authorize it.
  const row=old.find(r=>r.event==='connect_upload_completed');
  await f.db.prepare('INSERT INTO analytics_outbox(id,account_id,event,occurred_at,bytes,captures,consent_id) VALUES(?,?,?,?,?,?,?)').bind(row.id,row.account_id,row.event,row.occurred_at,row.bytes,row.captures,row.consent_id).run();
  f.provider.status=200;await sweep(f);
  assert.deepEqual(await queued(f),[]);assert.equal(f.provider.bodies.length,1,'old consent is not revived');
  const fresh=await upload(f,a,93);
  assert.equal((await f.call(fresh+'/complete',{token:a.token,method:'POST'})).status,200);
  await sweep(f);
  assert.deepEqual(f.provider.bodies.at(-1).batch.map(e=>e.event),['connect_upload_completed']);
 }finally{await f.close();}
});
test('account pilot consent: account deletion purges only its consent and queue without authorizing a recreated account',async()=>{
 const f=await setup();try{
  await f.db.exec('UPDATE analytics_control SET enabled=1');
  const removed=digest('deleted pilot account'),peer=digest('retained pilot account');
  const create=account=>f.db.prepare('INSERT INTO accounts(id,email,beta,created_at) VALUES(?,?,?,?)').bind(account,'synthetic@example.com','active',Date.now()).run();
  for(const account of [removed,peer]){await enroll(f,account);await create(account);}
  f.provider.status=503;await sweep(f);
  const before=await queued(f),peerRows=before.filter(row=>row.account_id===peer);
  assert.equal(before.filter(row=>row.account_id===removed).length,2,'signup and storage were actually queued');
  await f.db.prepare('DELETE FROM accounts WHERE id=?').bind(removed).run();
  assert.equal(await f.db.prepare('SELECT account_id FROM analytics_consent WHERE account_id=?').bind(removed).first(),null);
  assert.ok(await f.db.prepare('SELECT account_id FROM analytics_consent WHERE account_id=?').bind(peer).first());
  assert.deepEqual(await queued(f),peerRows,'deletion preserves the other account and its queue');
  await create(removed);
  assert.deepEqual(await queued(f),peerRows,'a recreated account must obtain fresh opt-in');
  f.provider.status=200;await sweep(f);
  assert.ok(f.provider.bodies.at(-1).batch.length>0);
  assert.ok(f.provider.bodies.at(-1).batch.every(event=>event.properties.distinct_id==='account:'+peer));
  assert.deepEqual(await queued(f),[]);
 }finally{await f.close();}
});
test('account pilot consent: invalid and future choices do not authorize collection',async()=>{
 const f=await setup();try{
  await f.db.exec('UPDATE analytics_control SET enabled=1');
  const a=await f.identity(digest('future consent'));
  for(const at of [0,-1,1.5])await assert.rejects(enroll(f,a.id,at));
  await enroll(f,a.id,Date.now()+86400000);
  await assert.rejects(f.db.prepare('UPDATE analytics_consent SET consented_at=1 WHERE account_id=?').bind(a.id).run());
  assert.equal(await sessionId(f,a),null);
  const path=await upload(f,a,94);
  assert.equal((await f.call(path+'/complete',{token:a.token,method:'POST'})).status,200);
  await sweep(f);assert.deepEqual(await queued(f),[]);assert.equal(f.provider.bodies.length,0);
 }finally{await f.close();}
});
test('analytics migration: fresh ordered schema includes main auth and disabled consent-gated analytics',async()=>{
 const mf=new Miniflare({modules:true,script:'export default {fetch(){return new Response("synthetic");}}',compatibilityDate:'2026-02-05',d1Databases:{DB:'fresh'}});try{
  const db=await mf.getD1Database('DB');
  for(const name of ['0001','0002','0003','0004','0005','0006'])await db.exec((await readFile('migrations/'+name+'.sql','utf8')).replace(/^--.*$/gm,'').replaceAll('\n',' '));
  assert.ok(await db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='auth_grants'").first());
  assert.ok(await db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='shares'").first());
  assert.ok(await db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='encryption_keys'").first());
  assert.equal((await db.prepare('SELECT enabled FROM analytics_control').first()).enabled,0);
  assert.equal((await db.prepare('SELECT count(*) n FROM analytics_consent').first()).n,0);
  const budget=await db.prepare('SELECT * FROM r2_budget').first();
  await assert.rejects(db.exec((await readFile('migrations/0006.sql','utf8')).replaceAll('\n',' ')));
  assert.deepEqual(await db.prepare('SELECT * FROM r2_budget').first(),budget,'rerun cannot reseed the budget');
 }finally{await mf.dispose();}
});
test('analytics migration: upgrade from main preserves auth, accounts, captures and spent budget without backfill',async()=>{
 const f=await setup({SHARING_ENABLED:'true'},true);try{
  await f.db.exec((await readFile('migrations/0003.sql','utf8')).replaceAll('\n',' '));
  const a=await f.identity('main upgrade'),path=await upload(f,a,95);
  assert.equal((await f.call(path+'/complete',{token:a.token,method:'POST'})).status,200);
  assert.equal((await f.call(path+'/share',{token:a.token,method:'POST'})).status,200);
  const device=await(await f.call('/v1/device/start',{method:'POST',body:{scope:'upload'}})).json();
  assert.equal((await f.call('/v1/device/approve',{...a,method:'POST',body:{userCode:device.userCode}})).status,200);
  const issued=await(await f.call('/v1/device/poll',{method:'POST',body:{deviceCode:device.deviceCode}})).json();
  assert.equal(issued.status,'approved');assert.equal(typeof issued.refreshToken,'string');
  await f.db.prepare('INSERT INTO encryption_keys(grant_id,version,recipient,signing) VALUES(?,1,?,?)').bind(issued.grantId,encryptionKey.recipient,encryptionKey.signing).run();
  await f.db.exec('UPDATE r2_budget SET a_used=a_used+7,b_used=b_used+11');
  const tables=['accounts','captures','shares','r2_budget','auth_user','auth_account','auth_session','auth_grants','oauthClient','oauthAccessToken','oauthRefreshToken','encryption_keys'];
  const rows=()=>Promise.all(tables.map(async table=>(await f.db.prepare('SELECT * FROM '+table).all()).results));
  assert.equal(await sessionId(f,a),null,'new runtime fails closed before the analytics migration');
  await sweep(f);assert.equal(f.provider.bodies.length,0);
  const before=await rows();
  await f.db.exec((await readFile('migrations/0006.sql','utf8')).replaceAll('\n',' '));
  assert.equal((await f.db.prepare('SELECT enabled FROM analytics_control').first()).enabled,0);
  assert.deepEqual(await queued(f),[]);
  assert.equal((await f.db.prepare('SELECT count(*) AS n FROM analytics_consent').first()).n,0);
  assert.deepEqual(await rows(),before);
  await f.db.exec('UPDATE analytics_control SET enabled=1');
  await sweep(f);assert.equal(f.provider.bodies.length,0);
  await enroll(f,a.id);
  assert.equal(await sessionId(f,a),'account:'+a.id);
  assert.deepEqual(await queued(f),[],'upgrading and enrolment do not invent signup/device/upload history');
  assert.equal((await f.call('/v1/account',{token:a.token})).status,200,'main bearer remains valid');
  const refreshed=await(await f.call('/v1/token/refresh',{method:'POST',body:{refreshToken:issued.refreshToken}})).json();
  assert.equal(refreshed.grantId,issued.grantId);assert.equal(refreshed.scope,'upload');
  assert.equal((await f.call('/v1/account',{token:refreshed.accessToken})).status,200);
  assert.deepEqual(await queued(f),[],'refreshing an existing device does not invent a connection');
 }finally{await f.close();}
});
test('account pilot consent: final database authorization is the export cut-off for withdrawal, re-enrolment and disable',async()=>{
 // Wrap the real worker/D1, as in the budget tests. Mutate consent only after
 // the original rows have been selected, not in a fake exporter or hash stub.
 const instrumented=await build({stdin:{resolveDir:process.cwd(),contents:`
  import worker from './src/worker.ts';
  export default {...worker,async scheduled(controller,env){
   const DB={prepare(sql){const wrap=s=>({raw:s,bind(...args){return wrap(s.bind(...args));},first(...args){return s.first(...args);},run(...args){return s.run(...args);},async all(...args){
    const result=await s.all(...args);
    const boundary=env.TEST_CONSENT_PHASE==='before'?'SELECT o.*':'SELECT o.id,o.consent_id';
    if(sql.startsWith(boundary)&&result.results.length){
     if(env.TEST_CONSENT_RACE==='disable')await env.DB.prepare('UPDATE analytics_control SET enabled=0').run();
     else {
      await env.DB.prepare('DELETE FROM analytics_consent').run();
      if(env.TEST_CONSENT_RACE==='reenroll')await env.DB.prepare('INSERT INTO analytics_consent(account_id,consented_at) SELECT id,? FROM accounts').bind(Date.now()).run();
     }
    }
    return result;
   }});return wrap(env.DB.prepare(sql));},batch(items){return env.DB.batch(items.map(s=>s.raw));}};
   return worker.scheduled(controller,{...env,DB});
  }};`},bundle:true,format:'esm',platform:'browser',conditions:['workerd'],external:['node:*'],write:false});
 for(const phase of ['before','after'])for(const mode of ['withdraw','reenroll','disable']){
  const f=await setup({TEST_CONSENT_RACE:mode,TEST_CONSENT_PHASE:phase},false,{scriptPath:undefined,script:instrumented.outputFiles[0].text});try{
   await f.db.exec('UPDATE analytics_control SET enabled=1');
   const a=await f.identity(digest('prepared '+mode));await enroll(f,a.id);
   await sweep(f);
   const expected=phase==='after'?1:0;
   assert.equal(f.provider.bodies.length,expected,phase+' final authorization: '+mode);
   if(expected)assert.deepEqual(f.provider.bodies[0].batch.map(e=>e.event),['connect_storage_snapshot'],'only the previously authorized snapshot finishes');
   if(mode==='disable'&&phase==='before')assert.equal((await queued(f)).length,1,'disabled delivery retains its queue');
   else assert.deepEqual(await queued(f),[],'withdrawal purged the selected row');
   assert.equal(await sessionId(f,a),mode==='reenroll'?'account:'+a.id:null);
   await sweep(f);assert.equal(f.provider.bodies.length,expected,'no later export revives the old batch');
  }finally{await f.close();}
 }
});
test('account pilot consent: an in-flight acknowledgement cannot delete re-enrolled queued snapshots',async()=>{
 const f=await setup();let release,flight,timer;try{
  await f.db.exec('UPDATE analytics_control SET enabled=1');
  const a=await f.identity(digest('inflight'));
  await enroll(f,a.id);
  const held=new Promise(resolve=>{release=resolve;});
  let reached;const entered=new Promise(resolve=>{reached=resolve;});
  f.provider.beforeResponse=async()=>{reached();await held;};
  flight=sweep(f);
  await Promise.race([entered,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error('synthetic provider was not reached')),3000);})]);
  clearTimeout(timer);
  const old=await queued(f);assert.equal(old.length,1);
  await f.db.prepare('DELETE FROM analytics_consent WHERE account_id=?').bind(a.id).run();
  assert.deepEqual(await queued(f),[]);
  await enroll(f,a.id);
  // The second sweep gets a refused acknowledgement and must retain its new row.
  f.provider.beforeResponse=null;f.provider.status=503;
  await f.db.exec('UPDATE analytics_control SET snapshot_day=-1');
  await sweep(f);const current=await queued(f);assert.equal(current.length,1);
  assert.notEqual(current[0].consent_id,old[0].consent_id);
  assert.notEqual(f.provider.bodies[0].batch[0].uuid,f.provider.bodies[1].batch[0].uuid);
  f.provider.status=200;release();await flight;
  assert.deepEqual(await queued(f),current,'old acknowledgement cannot erase a new consent generation');
  await sweep(f);assert.deepEqual(await queued(f),[]);
 }finally{clearTimeout(timer);release?.();if(flight)await flight;await f.close();}
});
test('optional analytics: encrypted device approval, atomic distinct accounts, concurrent/idempotent completion and allowlisted delivery',async()=>{
 const f=await setup();try{
 assert.equal((await f.db.prepare('SELECT enabled FROM analytics_control').first()).enabled,0);
 const excluded=await f.identity(digest('collection disabled'));
 assert.equal(await sessionId(f,excluded),null);assert.deepEqual(await queued(f),[]);
 await f.db.exec('UPDATE analytics_control SET enabled=1');
 await enroll(f,accountId(digest('first')));await enroll(f,accountId(digest('second')));
 const a=await f.identity(digest('first')),b=await f.identity(digest('second'));
 assert.equal(await sessionId(f,a),'account:'+a.id);
 const device=await(await f.call('/v1/device/start',{method:'POST',body:{scope:'upload',encryptionKey}})).json();
 assert.equal((await f.call('/v1/device/approve',{...a,method:'POST',body:{userCode:device.userCode}})).status,200);
 assert.ok([200,409].includes((await f.call('/v1/device/approve',{...a,method:'POST',body:{userCode:device.userCode}})).status));
 const issued=await(await f.call('/v1/device/poll',{method:'POST',body:{deviceCode:device.deviceCode}})).json();
 assert.equal(issued.status,'approved');
 assert.deepEqual(await f.db.prepare('SELECT version,recipient,signing FROM encryption_keys WHERE grant_id=?').bind(issued.grantId).first(),encryptionKey);
 assert.ok((await f.db.prepare('SELECT approved_at FROM auth_grants WHERE id=?').bind(issued.grantId).first()).approved_at>0);
 const renew=()=>f.call('/v1/token/refresh',{method:'POST',body:{refreshToken:issued.refreshToken}});
 const refreshed=await(await renew()).json(),replay=await(await renew()).json();
 assert.equal(refreshed.grantId,issued.grantId);assert.equal(refreshed.scope,'upload');
 assert.equal(replay.accessToken,refreshed.accessToken);assert.equal(replay.refreshToken,refreshed.refreshToken);
 assert.equal((await f.call('/v1/token/revoke',{token:refreshed.accessToken,method:'POST'})).status,200);
 assert.equal((await f.call('/v1/account',{token:refreshed.accessToken})).status,401);
 const denied=await(await f.call('/v1/device/start',{method:'POST',body:{scope:'read'}})).json();
 assert.equal((await f.call('/v1/device/reject',{...a,method:'POST',body:{userCode:denied.userCode}})).status,200);
 const paths=await Promise.all([upload(f,a,1),upload(f,b,2,'a second private recording')]);
 await Promise.all(Array.from({length:4},()=>f.call(paths[0]+'/complete',{token:a.token,method:'POST'})));
 assert.equal((await f.call(paths[1]+'/complete',{token:b.token,method:'POST'})).status,200);
 assert.equal((await f.call(paths[0]+'/complete',{token:a.token,method:'POST'})).status,200);
 const before=await queued(f);assert.equal(before.filter(r=>r.event==='connect_upload_completed').length,2);
 assert.equal(before.filter(r=>r.event==='connect_account_created').length,2);
 assert.equal(before.filter(r=>r.event==='connect_device_connected').length,1);
 await sweep(f);
 const sent=f.provider.bodies[0];assert.ok(sent,'the real worker must contact the synthetic provider');
 assert.equal((await queued(f)).length,0);
 const uploads=sent.batch.filter(e=>e.event==='connect_upload_completed');
 assert.equal(uploads.length,2);assert.equal(new Set(uploads.map(e=>e.properties.distinct_id)).size,2);
 for(const event of uploads){assert.equal(event.properties.actor,'device');assert.equal(event.properties.environment,'synthetic');assert.match(event.uuid,/^[a-f0-9-]{36}$/);}
 assert.equal(sent.batch.filter(e=>e.event==='connect_storage_snapshot').length,2);
 const snapshot=sent.batch.find(e=>e.event==='connect_storage_snapshot'&&e.properties.distinct_id==='account:'+a.id);
 assert.equal(snapshot.properties.retained_bytes,Buffer.byteLength('private evidence'));assert.equal(snapshot.properties.retained_recordings,1);
 const serialized=JSON.stringify(sent);for(const secret of ['private evidence','a second private recording',a.token,a.cookie,'@example.com','summary.md',id(1),'https://api.example',encryptionKey.recipient,encryptionKey.signing])assert.ok(!serialized.includes(secret),secret);
 await sweep(f);assert.deepEqual(await queued(f),[]);assert.equal(f.provider.bodies.length,1);
 }finally{await f.close();}
});
test('provider acknowledgement: observed Ok and legacy numeric success drain the queue; ambiguous bodies do not',async()=>{
 const f=await setup();try{
  await f.db.exec('UPDATE analytics_control SET enabled=1');
  const account=digest('provider acknowledgement');await enroll(f,accountId(account));await f.identity(account);
  for(const ack of [{},{status:0},{status:'ok'},{status:'success'},{status:'Error'},null]){
   f.provider.ack=ack;await sweep(f);assert.equal((await queued(f)).length,2,'ambiguous acknowledgement must retain signup and snapshot');
  }
  f.provider.ack={status:'Ok'};await sweep(f);assert.deepEqual(await queued(f),[],'observed PostHog batch acknowledgement drains the outbox');
  await f.db.exec('UPDATE analytics_control SET snapshot_day=-1');
  f.provider.ack={status:1};await sweep(f);assert.deepEqual(await queued(f),[],'legacy numeric success remains accepted');
 }finally{await f.close();}
});
test('provider failures preserve identical event identity; retries, deletion and expired reservations do not inflate storage',async()=>{
 const f=await setup();try{
 await f.db.exec('UPDATE analytics_control SET enabled=1');
 await enroll(f,accountId(digest('retry')));
 const a=await f.identity(digest('retry')),path=await upload(f,a,3,'hello');
 await f.call(path+'/complete',{token:a.token,method:'POST'});
 f.provider.status=503;await sweep(f);assert.ok((await queued(f)).length>0);
 await upload(f,a,4,'pending');await f.db.prepare('UPDATE captures SET pending_until=0 WHERE id=?').bind(id(4)).run();
 assert.equal((await f.call('/v1/account',{token:a.token})).status,200);
 f.provider.status=200;await sweep(f);assert.equal(f.provider.bodies.length,2);assert.deepEqual(f.provider.bodies[1],f.provider.bodies[0]);assert.deepEqual(await queued(f),[]);
 await f.call(path,{token:a.token,method:'DELETE'});
 await f.db.exec('UPDATE analytics_control SET snapshot_day=-1');
 await sweep(f);const snapshot=f.provider.bodies.at(-1).batch.find(e=>e.event==='connect_storage_snapshot');
 assert.equal(snapshot.properties.retained_bytes,0);assert.equal(snapshot.properties.pending_reserved_bytes,0);
 }finally{await f.close();}
});
test('collection remains bounded during outages and overflow is explicit, not a product failure',async()=>{
 const f=await setup();try{
 await f.db.exec("UPDATE analytics_control SET enabled=1; WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000) INSERT INTO analytics_outbox(id,account_id,event,occurred_at) SELECT 'fill:'||x,'synthetic','connect_account_created',1 FROM n;");
 await enroll(f,accountId(digest('overflow')));const a=await f.identity(digest('overflow'));
 assert.equal((await queued(f)).length,10000);assert.equal((await f.db.prepare('SELECT dropped FROM analytics_control').first()).dropped,1);
 assert.equal((await f.call('/v1/account',{token:a.token})).status,200);
 }finally{await f.close();}
});
test('excluded accounts cannot be linked or exported; disabled/invalid delivery never contacts provider',async()=>{
 const subject=digest('excluded'),excluded=accountId(subject);
 for(const bindings of [{POSTHOG_EXCLUDED_ACCOUNT_IDS:excluded},{POSTHOG_ENABLED:'false'},{POSTHOG_HOST:'https://us.i.posthog.com'}]){
  const f=await setup(bindings);try{
   await f.db.exec('UPDATE analytics_control SET enabled=1');await enroll(f,excluded);const a=await f.identity(subject);
   assert.equal(await sessionId(f,a),null);await sweep(f);assert.equal(f.provider.bodies.length,0);
  }finally{await f.close();}
 }
});
