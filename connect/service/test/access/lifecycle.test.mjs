import {test} from 'node:test';
import assert from 'node:assert/strict';
import {build} from 'esbuild';
import {setTimeout as delay} from 'node:timers/promises';
import {fixture,digest} from '../helpers.mjs';

// Faults surround real library SQL, never fabricated token responses. Two
// workerd isolates share D1; production has no fault headers or hooks.
const instrumented=await build({stdin:{resolveDir:process.cwd(),contents:`
 import worker from './src/worker.ts';
 import {identity} from './src/identity.ts';
 import {hash} from './src/common.ts';
 export default {async fetch(request,env){
  if(request.headers.has('X-Test-Origin')){const headers=new Headers(request.headers);headers.set('Origin',headers.get('X-Test-Origin'));request=new Request(request,{headers});}
  const fault=request.headers.get('X-Fault');
  const wrap=(statement,sql)=>new Proxy(statement,{get(target,key){
   if(key==='bind')return (...args)=>wrap(target.bind(...args),sql);
   if(key==='first'&&fault==='approval-read'&&sql.startsWith('SELECT * FROM auth_grants WHERE user_code='))return async (...args)=>{
    const row=await target.first(...args);await env.DB.prepare('UPDATE test_fault SET reached=reached+1').run();
    const deadline=Date.now()+15000;
    while((await env.DB.prepare('SELECT reached FROM test_fault').first()).reached<2){if(Date.now()>deadline)throw new Error('approval barrier timeout');await new Promise(r=>setTimeout(r,5));}
    return row;
   };
   if(key==='all')return async (...args)=>{
    const refresh=sql.includes('"oauthRefreshToken"');
    if(refresh&&((fault==='successor'&&/^insert /i.test(sql))||(fault==='replay'&&/^update /i.test(sql)&&sql.includes('"rotationReplayResponse"'))||(fault==='invalidation'&&/^delete /i.test(sql)))){
     await env.DB.prepare('UPDATE test_fault SET reached=reached+1').run();throw new Error('synthetic interrupted library write');
    }
    const result=await target.all(...args);
    if(fault==='invalidation-crash'&&sql.includes('"oauthAccessToken"')&&/^delete /i.test(sql)){
     await env.DB.prepare('UPDATE test_fault SET reached=reached+1').run();throw new Error('interruption after access invalidation, before refresh invalidation');
    }
    if(fault==='hold-cas'&&refresh&&/^update /i.test(sql)&&sql.includes('"rotatedAt"')){
     await env.DB.prepare('UPDATE test_fault SET reached=1').run();
     const deadline=Date.now()+15000;
     while(!(await env.DB.prepare('SELECT released FROM test_fault').first()).released){if(Date.now()>deadline)throw new Error('fault barrier timeout');await new Promise(r=>setTimeout(r,5));}
    }
    return result;
   };
   const value=target[key];return typeof value==='function'?value.bind(target):value;
  }});
  const DB=new Proxy(env.DB,{get(target,key){if(key==='prepare')return sql=>wrap(target.prepare(sql),sql);const value=target[key];return typeof value==='function'?value.bind(target):value;}});
  // Reach the real provider's invalidation boundary directly. In production a
  // stale preflight read can race into this same library path; raw routes stay private.
  if(fault==='invalidation-crash'){
   const {refreshToken}=await request.json(),g=await env.DB.prepare('SELECT clientId FROM oauthRefreshToken WHERE token=?').bind(await hash(refreshToken)).first();
   try{return await identity({...env,DB}).handler(new Request(env.API_ORIGIN+'/auth/oauth2/token',{method:'POST',headers:{'Content-Type':'application/x-www-form-urlencoded'},body:new URLSearchParams({grant_type:'refresh_token',client_id:g.clientId,refresh_token:refreshToken})}));}
   catch{return Response.json({error:{code:'auth_unavailable'}},{status:503});}
  }
  const response=await worker.fetch(request,{...env,DB});
  // The library catches a failed replay write and may still return its minted
  // credentials. Drop that response to model interruption before client receipt.
  if(fault==='replay')return Response.json({error:{code:'auth_unavailable'}},{status:503});
  return response;
 }};
`},bundle:true,format:'esm',platform:'browser',conditions:['workerd'],external:['node:*'],write:false});
async function setup(){const f=await fixture({}, {instances:true,scriptPath:undefined,script:instrumented.outputFiles[0].text});await f.db.exec('CREATE TABLE test_fault(reached INTEGER,released INTEGER); INSERT INTO test_fault VALUES(0,0)');return f;}
async function authorize(f,a,scope='upload'){
 const d=await(await f.call('/v1/device/start',{method:'POST',body:{scope}})).json();
 assert.equal((await f.call('/v1/device/approve',{...a,method:'POST',body:{userCode:d.userCode}})).status,200);
 const response=await f.call('/v1/device/poll',{method:'POST',body:{deviceCode:d.deviceCode}});assert.equal(response.status,200);
 const t=await response.json();assert.equal(t.status,'approved');assert.ok(t.refreshToken);return t;
}
const renew=(f,t,options={})=>f.call('/v1/token/refresh',{method:'POST',body:{refreshToken:t.refreshToken},...options});
const expireReplay=(f,t)=>f.db.prepare('UPDATE oauthRefreshToken SET rotationReplayExpiresAt=? WHERE token=?').bind(new Date(Date.now()-1000).toISOString(),digest(t.refreshToken)).run();

test('simultaneous approvals bind one immutable account before library consent',async()=>{
 const f=await setup();try{
 const a=await f.identity('approver-a'),b=await f.identity('approver-b');
 const d=await(await f.call('/v1/device/start',{method:'POST',body:{scope:'read'}})).json();
 const responses=await Promise.all([a,b].map((account,index)=>f.call('/v1/device/approve',{...account,origin:undefined,instance:index?'second':'first',method:'POST',body:{userCode:d.userCode},headers:{'X-Fault':'approval-read','X-Test-Origin':account.origin}})));
 const statuses=responses.map(r=>r.status).sort();assert.deepEqual(statuses,[200,409],JSON.stringify(await Promise.all(responses.map(r=>r.json()))));
 const winner=responses[0].status===200?a:b;
 assert.equal((await f.db.prepare('SELECT account_id FROM auth_grants WHERE user_code=?').bind(d.userCode).first()).account_id,winner.id);
 const token=await(await f.call('/v1/device/poll',{method:'POST',body:{deviceCode:d.deviceCode}})).json();assert.equal(token.status,'approved');
 assert.equal((await f.call('/v1/account',{token:token.accessToken})).status,200);
 }finally{await f.close();}
});

test('duplicate refresh across independent isolates has one successor and replays after restart',async()=>{
 const f=await setup();try{
 const a=await f.identity('parallel'),t=await authorize(f,a,'read');
 const responses=await Promise.all([renew(f,t),renew(f,t,{instance:'second'})]);
 assert.ok(responses.every(r=>[200,503].includes(r.status)));assert.ok(responses.some(r=>r.status===200));
 const success=await responses.find(r=>r.status===200).json();
 await f.reload();
 const replay=await renew(f,t,{instance:'second'});assert.equal(replay.status,200);const again=await replay.json();
 assert.equal(again.refreshToken,success.refreshToken);assert.equal(again.accessToken,success.accessToken);assert.equal(again.grantId,t.grantId);assert.equal(again.scope,'read');
 assert.equal((await f.db.prepare('SELECT count(*) n FROM oauthRefreshToken WHERE clientId=(SELECT client_id FROM auth_grants WHERE id=?) AND revoked IS NULL').bind(t.grantId).first()).n,1);
 }finally{await f.close();}
});

test('crashes after consumption and before replay persistence retry only within bounded grace',async()=>{
 const f=await setup();try{
 const a=await f.identity('crashes');
 for(const fault of ['successor','replay']){
  const t=await authorize(f,a);
  assert.equal((await renew(f,t,{headers:{'X-Fault':fault}})).status,503);
  const row=await f.db.prepare('SELECT * FROM oauthRefreshToken WHERE token=?').bind(digest(t.refreshToken)).first();
  assert.ok(row.revoked);assert.equal(row.rotationReplayResponse,null);
  assert.equal((await f.db.prepare('SELECT reached FROM test_fault').first()).reached,fault==='successor'?1:2);
  await f.reload();assert.equal((await renew(f,t,{instance:'second'})).status,503);
  await expireReplay(f,t);
  assert.equal((await renew(f,t,{instance:'second'})).status,401);
  assert.equal((await f.call('/v1/account',{token:t.accessToken})).status,401);
 }
 }finally{await f.close();}
});

test('expired replay denies authorization before fallible library family invalidation',async()=>{
 const f=await setup();try{
 const a=await f.identity('replay'),t=await authorize(f,a),sibling=await authorize(f,a);
 const rotated=await(await renew(f,t)).json();await expireReplay(f,t);
 const response=await renew(f,t,{headers:{'X-Fault':'invalidation'}});assert.equal(response.status,401);
 for(const token of [t.accessToken,rotated.accessToken])assert.equal((await f.call('/v1/account',{token})).status,401);
 assert.equal((await f.call('/v1/account',{token:sibling.accessToken})).status,200);
 assert.equal((await renew(f,sibling)).status,200);
 }finally{await f.close();}
});

test('interrupted library family deletion cannot leave a renewable orphan authorization',async()=>{
 const f=await setup();try{
 const a=await f.identity('family-crash'),t=await authorize(f,a),sibling=await authorize(f,a);
 const next=await(await renew(f,t)).json();await expireReplay(f,t);
 assert.equal((await renew(f,t,{headers:{'X-Fault':'invalidation-crash'}})).status,503);
 assert.equal((await f.db.prepare('SELECT reached FROM test_fault').first()).reached,1,'actual access deletion preceded the fault');
 assert.equal((await renew(f,next,{instance:'second'})).status,401,'surviving refresh rows must not mint an authorized orphan');
 assert.equal((await f.call('/v1/account',{token:sibling.accessToken})).status,200);assert.equal((await renew(f,sibling)).status,200);
 }finally{await f.close();}
});

test('revocation dominates an in-flight library successor in another isolate',async()=>{
 const f=await setup();let pending;try{
 const a=await f.identity('revoke-race'),t=await authorize(f,a),sibling=await authorize(f,a);
 pending=renew(f,t,{headers:{'X-Fault':'hold-cas'}});
 let reached=false;for(let i=0;i<500;i++){if((await f.db.prepare('SELECT reached FROM test_fault').first()).reached){reached=true;break;}await delay(10);}assert.ok(reached,'real rotation CAS must reach the barrier');
 assert.equal((await f.call('/v1/token/revoke',{instance:'second',token:t.accessToken,method:'POST'})).status,200);
 assert.equal((await f.call('/v1/account',{instance:'second',token:t.accessToken})).status,401);
 await f.db.exec('UPDATE test_fault SET released=1');assert.equal((await pending).status,401);pending=null;
 assert.equal((await renew(f,t)).status,401);
 assert.equal((await f.call('/v1/account',{token:sibling.accessToken})).status,200);
 }finally{await f.db.exec('UPDATE test_fault SET released=1');if(pending)await pending;await f.close();}
});

test('access expiry permits refresh; logout and mismatched revocation cannot revoke sibling grants',async()=>{
 const f=await setup();try{
 const a=await f.identity('expiry'),t=await authorize(f,a),sibling=await authorize(f,a);
 assert.equal((await f.call('/v1/token/revoke',{token:t.accessToken,method:'POST',body:{refreshToken:sibling.refreshToken}})).status,401);
 assert.equal((await f.call('/auth/logout',{...a,method:'POST'})).status,200);
 assert.equal((await f.call('/v1/account',{token:t.accessToken})).status,200);
 await f.db.prepare('UPDATE oauthAccessToken SET expiresAt=? WHERE token=?').bind(new Date(Date.now()-1000).toISOString(),digest(t.accessToken)).run();
 assert.equal((await f.call('/v1/account',{token:t.accessToken})).status,401);
 const response=await renew(f,t);assert.equal(response.status,200);const next=await response.json();assert.equal(next.grantId,t.grantId);
 assert.equal((await f.call('/v1/token/revoke',{token:t.accessToken,method:'POST',body:{refreshToken:next.refreshToken}})).status,200);
 assert.equal((await f.call('/v1/account',{token:next.accessToken})).status,401);assert.equal((await renew(f,sibling)).status,200);
 assert.equal((await f.call('/v1/token/revoke',{method:'POST',body:{refreshToken:sibling.refreshToken}})).status,200);
 assert.equal((await f.call('/v1/account',{token:sibling.accessToken})).status,401);
 }finally{await f.close();}
});

test('provider-revoked access and expired renewal are denied; legacy credentials require reapproval',async()=>{
 const f=await setup();try{
 const a=await f.identity('expired-refresh'),t=await authorize(f,a);
 await f.db.prepare('UPDATE oauthAccessToken SET revoked=? WHERE token=?').bind(new Date().toISOString(),digest(t.accessToken)).run();
 assert.equal((await f.call('/v1/account',{token:t.accessToken})).status,401);
 await f.db.prepare('UPDATE oauthRefreshToken SET expiresAt=? WHERE token=?').bind(new Date(Date.now()-1000).toISOString(),digest(t.refreshToken)).run();
 const expired=await renew(f,t);assert.equal(expired.status,401);assert.equal((await expired.json()).error.code,'reauthorization_required');
 const legacy=await f.call('/v1/account',{token:digest('retired-pre-renewal-token')});assert.equal(legacy.status,401);assert.equal((await legacy.json()).error.code,'reauthorization_required');
 }finally{await f.close();}
});
