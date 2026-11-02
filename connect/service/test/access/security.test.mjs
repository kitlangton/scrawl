import {test} from 'node:test';
import assert from 'node:assert/strict';
import {fixture,id,file,digest} from '../helpers.mjs';

test('identity refuses missing or weak configured secrets instead of library development defaults',async()=>{
 for(const secret of ['', 'short']){
  const f=await fixture({BETTER_AUTH_SECRET:secret});try{
   const response=await f.call('/auth/google/start');assert.equal(response.status,503);assert.equal((await response.json()).error.code,'auth_unavailable');
   assert.equal((await f.db.prepare('SELECT count(*) n FROM auth_verification').first()).n,0);
  }finally{await f.close();}
 }
});

test('private Better Auth management and alternate identity routes stay unreachable',async()=>{
 const f=await fixture();try{
 const a=await f.identity('private-management'),before=(await f.db.prepare('SELECT count(*) n FROM oauthClient').first()).n;
 for(const path of ['/auth/oauth2/register','/auth/oauth2/token','/auth/get-session','/auth/device/token','/auth/sign-up/email','/auth/callback/google']){
  for(const method of ['GET','POST'])assert.equal((await f.call(path,{...a,method,...method==='POST'?{body:{}}:{}})).status,404);
 }
 assert.equal((await f.db.prepare('SELECT count(*) n FROM oauthClient').first()).n,before);
 }finally{await f.close();}
});

test('capture reservation/upload/complete/download, retries, scopes and isolation',async()=>{
 const f=await fixture(); try {
 const a=await f.identity('alice'),b=await f.identity('bob'),p='/v1/captures/'+id(1),files=[file()];
 let r=await f.call(p,{token:a.token,method:'POST',body:{files}}); assert.equal(r.status,200); assert.equal((await r.json()).status,'pending');
 assert.equal((await f.call(p,{token:a.read,method:'POST',body:{files}})).status,403);
 for(const method of ['GET','DELETE','POST']) assert.equal((await f.call(p,{token:b.token,method,...(method==='POST'?{body:{files}}:{})})).status,404);
 assert.equal((await f.call(p+'/files/summary.md',{token:a.read})).status,409);
 assert.equal((await f.call(p+'/complete',{token:a.token,method:'POST'})).status,409);
 assert.equal((await f.call(p,{token:a.token,method:'POST',body:{files}})).status,200);
 assert.equal((await f.call(p,{token:a.token,method:'POST',body:{files:[file('summary.md','changed')]}})).status,409);
 assert.equal((await f.call(p+'/files/summary.md',{token:a.token,method:'PUT',raw:'bad'})).status,400);
 assert.equal((await f.call(p+'/files/summary.md',{token:a.token,method:'PUT',raw:'wrong'})).status,400);
 for(let n=0;n<2;n++) assert.equal((await f.call(p+'/files/summary.md',{token:a.token,method:'PUT',raw:'hello'})).status,200);
 assert.equal((await f.bucket.list()).objects.length,1,'idempotent retries must not inflate stored bytes');
 for(let n=0;n<2;n++) assert.equal((await f.call(p+'/complete',{token:a.token,method:'POST'})).status,200);
 assert.equal(await (await f.call(p+'/files/summary.md',{token:a.read})).text(),'hello');
 assert.equal((await f.call(p+'/files/summary.md',{token:b.read})).status,404);
 assert.equal((await f.call(p,{token:a.read,method:'DELETE'})).status,403);
 assert.equal((await f.call(p,{token:a.token,method:'DELETE'})).status,200);
 assert.equal((await f.call(p+'/complete',{token:a.token,method:'POST'})).status,404);
 assert.equal((await f.call(p+'/files/summary.md',{token:a.read})).status,404);
 assert.equal((await f.call(p,{token:a.token,method:'POST',body:{files}})).status,404);
 assert.equal((await (await f.call('/v1/account',{token:a.token})).json()).usedBytes,0);
 }finally{await f.close();}
});

test('expiry denies bytes and metadata before cleanup, pending reservation reclamation',async()=>{
 const f=await fixture();try{
 const a=await f.identity('a'),p='/v1/captures/'+id(2);
 await f.call(p,{token:a.token,method:'POST',body:{files:[file()]}});
 await f.call(p+'/files/summary.md',{token:a.token,method:'PUT',raw:'hello'});
 await f.call(p+'/complete',{token:a.token,method:'POST'});
 await f.db.prepare('UPDATE captures SET expires_at=?').bind(Date.now()-1).run();
 assert.equal((await f.call(p,{token:a.read})).status,404);
 assert.equal((await f.call(p+'/files/summary.md',{token:a.read})).status,404);
 assert.equal((await (await f.call('/v1/account',{token:a.read})).json()).usedBytes,0);
 const scheduler=await f.mf.getWorker(); await scheduler.scheduled();
 const objects=await f.bucket.list(); assert.equal(objects.objects.length,0);
 }finally{await f.close();}
});

test('manifest and streaming input bounds reject paths, types, hashes and large files',async()=>{
 const f=await fixture();try{
 const a=await f.identity('a');let n=30;
 for(const path of ['../summary.md','/summary.md','frames//frame-1.png','frames/./frame-1.png','frames/../summary.md','frames\\frame-1.png','secret.txt','frames/thing.svg','', 'frames/'+ 'a'.repeat(140)+'.png']) {
  assert.equal((await f.call('/v1/captures/'+id(n++),{token:a.token,method:'POST',body:{files:[file(path)]}})).status,400,path);
 }
 for(const entry of [{...file(),size:-1},{...file(),sha256:'A'.repeat(64)},{...file(),size:90000001}]) assert.equal((await f.call('/v1/captures/'+id(n++),{token:a.token,method:'POST',body:{files:[entry]}})).status,400);
 assert.equal((await f.call('/v1/captures/'+id(n++),{token:a.token,method:'POST',body:{files:[file(),file()]}})).status,400);
 assert.equal((await f.call('/v1/captures/'+id(n++),{token:a.token,method:'POST',body:{files:Array.from({length:513},(_,i)=>file('frames/frame-'+i+'.png'))}})).status,400);
 assert.equal((await f.call('/v1/device/start',{method:'POST',raw:' '.repeat(131073)})).status,413);
 }finally{await f.close();}
});

test('concurrent quota admission is atomic and full reservation is counted',async()=>{
 const f=await fixture();try{
 const a=await f.identity('a');const files=Array.from({length:3},(_,i)=>({...file('frames/frame-'+i+'.png'),size:90000000}));
 const responses=await Promise.all([60,61].map(n=>f.call('/v1/captures/'+id(n),{token:a.token,method:'POST',body:{files}})));
 assert.ok(responses.every(r=>[200,404].includes(r.status)),'older reservation may already be evicted before response');
 const {results:captures}=await f.db.prepare('SELECT status FROM captures ORDER BY reservation_seq').all();
 assert.deepEqual(captures.map(c=>c.status),['deleted','pending']);
 assert.equal((await (await f.call('/v1/account',{token:a.read})).json()).usedBytes,270000000);
 await f.db.prepare("UPDATE captures SET pending_until=? WHERE status='pending'").bind(Date.now()-1).run();
 assert.equal((await (await f.call('/v1/account',{token:a.read})).json()).usedBytes,0);
 assert.equal((await f.call('/v1/captures/'+id(62),{token:a.token,method:'POST',body:{files}})).status,200);
 }finally{await f.close();}
});

test('device preview, CSRF, explicit scope approval, durable poll and revocation',async()=>{
 const f=await fixture();try{
 const a=await f.identity('a'),start=await(await f.call('/v1/device/start',{method:'POST',body:{scope:'read'}})).json();
 const {userCode,deviceCode}=start;
 assert.equal((await f.call('/v1/device?userCode='+userCode)).status,401);
 const preview=await(await f.call('/v1/device?userCode='+userCode,a)).json();assert.equal(preview.scope,'read');assert.equal(preview.status,'pending');assert.equal(new Date(preview.expiresAt).toISOString(),preview.expiresAt);
 assert.deepEqual(await(await f.call('/v1/device/poll',{method:'POST',body:{deviceCode}})).json(),{status:'pending'});
 assert.equal((await f.call('/v1/device/poll',{method:'POST',body:{deviceCode}})).status,429);
 assert.equal((await f.call('/v1/device/approve',{cookie:a.cookie,method:'POST',body:{userCode}})).status,403);
 assert.equal((await f.call('/v1/device/approve',{...a,origin:'https://evil.example',method:'POST',body:{userCode}})).status,403);
 assert.equal((await f.call('/v1/device/approve',{...a,method:'POST',body:{userCode}})).status,200);
 await f.db.prepare('UPDATE auth_grants SET last_poll=0').run();await f.db.prepare('UPDATE auth_device SET lastPolledAt=NULL').run();
 const approved=await(await f.call('/v1/device/poll',{method:'POST',body:{deviceCode}})).json();
 assert.equal(approved.status,'approved');assert.equal(approved.scope,'read');assert.ok(approved.accessToken.length>=32);
 await f.db.prepare('UPDATE auth_grants SET last_poll=0').run();
 assert.deepEqual(await(await f.call('/v1/device/poll',{method:'POST',body:{deviceCode}})).json(),approved);
 assert.equal((await f.call('/v1/captures/'+id(70),{token:approved.accessToken,method:'POST',body:{files:[file()]}})).status,403);
 assert.equal((await f.call('/v1/token/revoke',{token:approved.accessToken,method:'POST'})).status,200);
 assert.equal((await f.call('/v1/account',{token:approved.accessToken})).status,401);
 await f.db.prepare('UPDATE auth_grants SET last_poll=0').run();
 assert.deepEqual(await(await f.call('/v1/device/poll',{method:'POST',body:{deviceCode}})).json(),{status:'rejected'});
 const session=await(await f.call('/v1/session',{cookie:a.cookie})).json();assert.equal(session.csrfToken,a.csrf);assert.equal(session.account.beta,'active');assert.equal(session.slots.limit,20);
 assert.equal((await f.call('/auth/logout',{...a,method:'POST'})).status,200);
 assert.equal((await(await f.call('/v1/session',{cookie:a.cookie})).json()).authenticated,false);
 }finally{await f.close();}
});
