import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {setTimeout as delay} from 'node:timers/promises';
import {fixture,id,file} from '../helpers.mjs';

test('90MB real streaming upload fits v1 bound without whole-file Worker buffering',async()=>{
 const f=await fixture();try{
 const a=await f.identity('a'),p='/v1/captures/'+id(200),size=90000000,chunk=new Uint8Array(65536).fill(19),h=createHash('sha256');
 for(let n=0;n<size;n+=chunk.length)h.update(chunk.subarray(0,Math.min(chunk.length,size-n)));
 const entry={path:'recording.mp4',size,sha256:h.digest('hex')};
 assert.equal((await f.call(p,{token:a.token,method:'POST',body:{files:[entry]}})).status,200);
 let sent=0;const stream=new ReadableStream({pull(c){if(sent===size){c.close();return;}const v=chunk.slice(0,Math.min(chunk.length,size-sent));sent+=v.length;c.enqueue(v);}});
 const upload=await f.mf.dispatchFetch('https://api.example'+p+'/files/recording.mp4',{method:'PUT',headers:{Authorization:'Bearer '+a.token},body:stream,duplex:'half'});
 assert.equal(upload.status,200,await upload.text());
 assert.equal((await f.call(p+'/complete',{token:a.token,method:'POST'})).status,200);
 const object=await f.bucket.list();assert.equal(object.objects.length,1);assert.equal(object.objects[0].size,size);
 }finally{await f.close();}
});

test('deletion during a real in-flight upload forbids publication and scheduled cleanup removes staged bytes',async()=>{
 const f=await fixture();let controller;try{
 const a=await f.identity('a'),p='/v1/captures/'+id(201);
 await f.call(p,{token:a.token,method:'POST',body:{files:[file()]}});
 const stream=new ReadableStream({start(c){controller=c;c.enqueue(new TextEncoder().encode('he'));}});
 const upload=f.mf.dispatchFetch('https://api.example'+p+'/files/summary.md',{method:'PUT',headers:{Authorization:'Bearer '+a.token},body:stream,duplex:'half'});
 let attempts=0;for(let i=0;i<100;i++){attempts=(await f.db.prepare('SELECT count(*) AS n FROM attempts').first()).n;if(attempts)break;await delay(10);}assert.equal(attempts,1);
 assert.equal((await f.call(p,{token:a.token,method:'DELETE'})).status,200);
 controller.enqueue(new TextEncoder().encode('llo'));controller.close();controller=null;
 assert.equal((await upload).status,404);
 assert.equal((await f.call(p+'/complete',{token:a.token,method:'POST'})).status,404);
 assert.equal((await f.call(p+'/files/summary.md',{token:a.read})).status,404);
 await(await f.mf.getWorker()).scheduled();assert.equal((await f.bucket.list()).objects.length,0);
 }finally{if(controller)controller.close();await f.close();}
});

test('zero-byte file, concurrent identical manifests, rejection, device expiry and token expiry',async()=>{
 const f=await fixture();try{
 const a=await f.identity('a'),p='/v1/captures/'+id(202),files=[file('summary.md','')];
 const rs=await Promise.all([1,2,3].map(()=>f.call(p,{token:a.token,method:'POST',body:{files}})));assert.deepEqual(rs.map(r=>r.status),[200,200,200]);
 const upload=await f.call(p+'/files/summary.md',{token:a.token,method:'PUT',raw:''});assert.equal(upload.status,200,await upload.text());
 assert.equal((await f.call(p+'/complete',{token:a.token,method:'POST'})).status,200);
 for(const expired of [false,true]) {
  const d=await(await f.call('/v1/device/start',{method:'POST',body:{scope:'upload'}})).json();
  if(expired)await f.db.prepare('UPDATE auth_grants SET expires_at=? WHERE user_code=?').bind(Date.now()-1,d.userCode).run();
  else assert.equal((await f.call('/v1/device/reject',{...a,method:'POST',body:{userCode:d.userCode}})).status,200);
  assert.equal((await(await f.call('/v1/device/poll',{method:'POST',body:{deviceCode:d.deviceCode}})).json()).status,expired?'expired':'rejected');
  assert.equal((await f.call('/v1/device/approve',{...a,method:'POST',body:{userCode:d.userCode}})).status,expired?404:409);
 }
 await f.db.prepare('UPDATE oauthAccessToken SET expiresAt=?').bind(new Date(Date.now()-1).toISOString()).run();
 assert.equal((await f.call('/v1/account',{token:a.read})).status,401);
 }finally{await f.close();}
});
