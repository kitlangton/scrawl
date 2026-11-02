import {test} from 'node:test';
import assert from 'node:assert/strict';
import {fixture,id,file,digest} from '../helpers.mjs';
import {readFile} from 'node:fs/promises';

async function setup(bindings={}){
 const f=await fixture({SHARING_ENABLED:'true',...bindings});
 // Baseline remains valid before this additive migration exists.
 try{await f.db.exec((await readFile('migrations/0003.sql','utf8')).replaceAll('\n',' '));}catch(e){if(e.code!=='ENOENT')throw e;}
 return f;
}
async function reserve(f,a,n=171){const p='/v1/captures/'+id(n);assert.equal((await f.call(p,{token:a.token,method:'POST',body:{files:[file()]}})).status,200);return p;}
async function ready(f,a,p){assert.equal((await f.call(p+'/files/summary.md',{token:a.token,method:'PUT',raw:'hello'})).status,200);assert.equal((await f.call(p+'/complete',{token:a.token,method:'POST'})).status,200);}

test('explicit capability reservation is pending, stable, single-capture and read-only',async()=>{
 const f=await setup();try{
  const a=await f.identity('alice'),b=await f.identity('bob'),p=await reserve(f,a);
  assert.equal((await f.call(p+'/share',{token:a.read,method:'POST'})).status,403);
  assert.equal((await f.call(p+'/share',{token:b.token,method:'POST'})).status,404);
  const r=await f.call(p+'/share',{token:a.token,method:'POST'});assert.equal(r.status,200,'explicit owner sharing must reserve a capability before upload completes');
  const s=await r.json();assert.equal(s.status,'pending');assert.match(s.url,/^https:\/\/api\.example\/s\/[a-f0-9]{64}$/);
  const path=new URL(s.url).pathname,secret=path.split('/').at(-1);
  assert.notEqual(secret,a.token);assert.ok(!secret.includes(id(171)));
  const repeats=await Promise.all(Array.from({length:5},()=>f.call(p+'/share',{token:a.token,method:'POST'}).then(r=>r.json())));assert.ok(repeats.every(x=>x.url===s.url));
  assert.equal((await f.db.prepare('SELECT count(*) AS n FROM shares WHERE revoked_at IS NULL').first()).n,1);
  const stored=await f.db.prepare('SELECT * FROM shares').first();assert.equal(stored.hash,digest(secret));assert.ok(!JSON.stringify(stored).includes(secret),'plaintext capability must not be stored');
  assert.equal((await (await f.call(path+'/manifest.json')).json()).status,'pending');
  assert.equal((await f.call(path+'/files/summary.md')).status,409);
  for(const suffix of ['', '/manifest.json','/files/summary.md'])for(const method of ['POST','PUT','DELETE'])assert.equal((await f.call(path+suffix,{method})).status,405);
  for(const suffix of ['/v1/account','/captures','/files/../secret','/files/secret.txt'])assert.equal((await f.call(path+suffix)).status,404);
  assert.equal((await f.call('/v1/account',{token:secret})).status,401);
  await ready(f,a,p);
  assert.equal(await (await f.call(path+'/files/summary.md')).text(),'hello');
  const m=await (await f.call(path+'/manifest.json')).json();assert.equal(m.status,'ready');assert.equal(m.files.length,1);assert.equal(m.files[0].url,s.url+'/files/summary.md');assert.ok(!JSON.stringify(m).includes('alice'));
  const html=await (await f.call(path)).text();assert.ok(html.includes('summary.md'));assert.ok(html.includes('manifest.json'));
  const list=await (await f.call('/v1/shares',{token:a.read})).json();assert.equal(list.shares.length,1);assert.equal(list.shares[0].url,s.url);
  assert.deepEqual((await (await f.call('/v1/shares',{token:b.read})).json()).shares,[]);
 }finally{await f.close();}
});

test('revocation, re-share and read-time expiry guard HTML, previews, manifests and ranges',async()=>{
 const f=await setup();try{
  const a=await f.identity('lifecycle'),b=await f.identity('foreign'),p=await reserve(f,a);await ready(f,a,p);
  const share=async()=>{const r=await f.call(p+'/share',{token:a.token,method:'POST'});assert.equal(r.status,200);return r.json();};
  const s=await share(),path=new URL(s.url).pathname;
  const list=await (await f.call('/v1/shares',a)).json(),management=list.shares[0].id;
  assert.equal((await f.call('/v1/shares/'+management,{...b,method:'DELETE'})).status,404);
  assert.equal((await f.call('/v1/shares/'+management,{cookie:a.cookie,method:'DELETE'})).status,403);
  assert.equal((await f.call('/v1/shares/'+management,{cookie:a.cookie,csrf:a.csrf,origin:'https://evil.example',method:'DELETE'})).status,403);
  const part=await f.call(path+'/files/summary.md',{headers:{Range:'bytes=1-3'}});assert.equal(part.status,206);assert.equal(part.headers.get('Content-Range'),'bytes 1-3/5');assert.equal(await part.text(),'ell');
  assert.equal((await f.call(path+'/files/summary.md',{headers:{Range:'bytes=8-9'}})).status,416);
  assert.equal((await f.call('/v1/shares/'+management,{...a,method:'DELETE'})).status,200);
  for(const suffix of ['', '/manifest.json','/thumbnail.png','/files/summary.md'])for(const method of ['GET','HEAD']){const r=await f.call(path+suffix,{method,headers:{Range:'bytes=0-1'}});assert.equal(r.status,410);assert.equal(r.headers.get('Cache-Control'),'no-store');assert.equal(r.headers.get('Referrer-Policy'),'no-referrer');assert.match(r.headers.get('X-Robots-Tag'),/noindex/);const text=await r.text();assert.ok(!text.includes('hello'));assert.ok(!text.includes('og:image'));}
  assert.equal(await (await f.call(p+'/files/summary.md',{token:a.read})).text(),'hello','revoking public access preserves the private original');
  const second=await share();assert.notEqual(second.url,s.url);const next=new URL(second.url).pathname;
  await f.db.prepare('UPDATE captures SET expires_at=? WHERE id=?').bind(Date.now()-1,id(171)).run();
  for(const suffix of ['', '/manifest.json','/thumbnail.png','/files/summary.md'])assert.equal((await f.call(next+suffix)).status,410);
 }finally{await f.close();}
});

test('Connect and explicit legacy origins share management CSRF but never grant foreign authority',async()=>{
 const canonical='https://app.tryapparent.com',legacy='https://hosted.tryapparent.com';
 const f=await setup({SITE_ORIGIN:canonical,LEGACY_SITE_ORIGIN:legacy});try{
  const a=await f.identity('connect-sharing'),p=await reserve(f,a);await ready(f,a,p);
  for(const origin of [canonical,legacy]){
   const created=await f.call(p+'/share',{token:a.token,method:'POST'});assert.equal(created.status,200);
   const share=await created.json(),path=new URL(share.url).pathname;
   const list=await f.call('/v1/shares',{cookie:a.cookie,origin});assert.equal(list.status,200);assert.equal(list.headers.get('Access-Control-Allow-Origin'),origin);
   const management=(await list.json()).shares.find(s=>s.url===share.url).id;
   for(const rejected of ['https://evil.example','null',undefined,canonical+'/',canonical+'.evil.example']){
    const denied=await f.call('/v1/shares/'+management,{cookie:a.cookie,csrf:a.csrf,origin:rejected,method:'DELETE'});
    assert.equal(denied.status,403);assert.equal(denied.headers.get('Access-Control-Allow-Origin'),null);
    assert.equal((await f.call(path+'/manifest.json')).status,200,'denied mutation must preserve capability');
   }
   assert.equal((await f.call('/v1/shares/'+management,{cookie:a.cookie,csrf:'wrong',origin,method:'DELETE'})).status,403);
   const stopped=await f.call('/v1/shares/'+management,{cookie:a.cookie,csrf:a.csrf,origin,method:'DELETE'});
   assert.equal(stopped.status,200);assert.equal(stopped.headers.get('Access-Control-Allow-Origin'),origin);
   assert.equal((await f.call(path+'/manifest.json')).status,410);
   assert.equal(await(await f.call(p+'/files/summary.md',{token:a.read})).text(),'hello');
  }
 }finally{await f.close();}
});

test('public reads spend the durable allowance, reject unshared IDs and fail closed',async()=>{
 const f=await setup();try{
  const a=await f.identity('budget'),p=await reserve(f,a);await ready(f,a,p);
  assert.equal((await f.call('/s/'+a.token+'/manifest.json')).status,404);assert.equal((await f.call('/s/'+id(171))).status,404);
  const s=await (await f.call(p+'/share',{token:a.token,method:'POST'})).json(),path=new URL(s.url).pathname;
  const before=(await f.db.prepare('SELECT b_used FROM r2_budget').first()).b_used;
  assert.equal(await (await f.call(path+'/files/summary.md')).text(),'hello');assert.equal((await f.db.prepare('SELECT b_used FROM r2_budget').first()).b_used,before+1);
  await f.db.prepare('UPDATE r2_budget SET b_cap=b_used').run();assert.equal((await f.call(path+'/manifest.json')).status,200,'metadata does not call R2');
  const denied=await f.call(path+'/files/summary.md',{headers:{Range:'bytes=0-1'}});assert.equal(denied.status,503);assert.equal((await denied.json()).error.code,'budget_exhausted');
  await f.db.prepare('DELETE FROM r2_budget').run();assert.equal((await f.call(path+'/files/summary.md')).status,503);
  await f.call(p,{token:a.token,method:'DELETE'});assert.equal((await f.call(path)).status,410);
 }finally{await f.close();}
});

test('deleted ready captures report deletion after their obsolete upload deadline',async()=>{
 const f=await setup();try{
  const a=await f.identity('deleted'),p=await reserve(f,a);await ready(f,a,p);
  const s=await (await f.call(p+'/share',{token:a.token,method:'POST'})).json(),path=new URL(s.url).pathname;
  await f.db.prepare('UPDATE captures SET pending_until=? WHERE id=?').bind(Date.now()-1,id(171)).run();
  assert.equal((await f.call(p,{token:a.token,method:'DELETE'})).status,200);
  const response=await f.call(path);assert.equal(response.status,410);assert.match(await response.text(),/<h1[^>]*>Recording deleted<\/h1>/);
  const list=await (await f.call('/v1/shares',{cookie:a.cookie})).json();assert.equal(list.shares[0].status,'deleted');
 }finally{await f.close();}
});
test('pending progress reflects confirmed files, failed state is not ready, default-off grant gate',async()=>{
 const f=await setup();try{
  const a=await f.identity('progress'),p=await reserve(f,a),s=await (await f.call(p+'/share',{token:a.token,method:'POST'})).json(),path=new URL(s.url).pathname;
  for(const status of ['paused','failed','uploading']){
   assert.equal((await f.call(p+'/share/progress',{token:a.token,method:'POST',body:{state:status,uploadedBytes:999999,totalBytes:999999}})).status,200);
   const m=await (await f.call(path+'/manifest.json')).json();assert.equal(m.status,status==='uploading'?'pending':status);assert.equal(m.uploadedBytes,0);assert.equal(m.totalBytes,5);assert.equal(m.files.length,0);
  }
  await f.call(p+'/files/summary.md',{token:a.token,method:'PUT',raw:'hello'});
  const m=await (await f.call(path+'/manifest.json')).json();assert.equal(m.uploadedBytes,5);assert.equal(m.status,'pending');
  const html=await (await f.call(path)).text();assert.ok(!html.includes('og:image'));assert.ok(html.includes('Preparing recording'));assert.ok(!html.includes('<video'));
 }finally{await f.close();}
 const disabled=await fixture();try{const a=await disabled.identity('off'),p=await reserve(disabled,a);const r=await disabled.call(p+'/share',{token:a.token,method:'POST'});assert.equal(r.status,403);assert.equal((await r.json()).error.code,'sharing_disabled');}finally{await disabled.close();}
});

test('owner management paginates all links deterministically without borrowing a foreign cursor',async()=>{
 const f=await setup();try{
  const a=await f.identity('pages'),p=await reserve(f,a);await ready(f,a,p);
  const s=await (await f.call(p+'/share',{token:a.token,method:'POST'})).json();
  const original=await f.db.prepare('SELECT * FROM shares').first();await f.db.prepare('UPDATE shares SET revoked_at=1').run();
  const inserts=Array.from({length:105},(_,n)=>f.db.prepare('INSERT INTO shares(id,capture_id,account_id,hash,envelope,created_at,revoked_at) VALUES(?,?,?,?,?,?,1)').bind(id(n+1000),id(171),original.account_id,id(n+2000),original.envelope,original.created_at));
  await f.db.batch(inserts);
  let before=null;const found=[];do{const r=await f.call('/v1/shares'+(before?'?before='+encodeURIComponent(before):''),a);assert.equal(r.status,200);const page=await r.json();assert.ok(page.shares.length<=100);found.push(...page.shares.map(x=>x.id));before=page.nextCursor;assert.ok(before===null||/^\d+:[a-f0-9]{32}$/.test(before));}while(before);
  assert.equal(found.length,106);assert.equal(new Set(found).size,106);
  assert.equal((await f.call('/v1/shares?before=garbage',a)).status,400);
 }finally{await f.close();}
});
