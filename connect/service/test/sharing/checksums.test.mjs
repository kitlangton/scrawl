import {test} from 'node:test';
import assert from 'node:assert/strict';
import {fixture,id,file,digest} from '../helpers.mjs';

test('uploads retain an R2-verified SHA256, retries validate bytes and preserve the original object',async()=>{
 const f=await fixture();try{
 const a=await f.identity('checksums'),p='/v1/captures/'+id(301);
 assert.equal((await f.call(p,{token:a.token,method:'POST',body:{files:[file()]}})).status,200);
 const put=raw=>f.call(p+'/files/summary.md',{token:a.token,method:'PUT',raw});
 assert.equal((await put('hello')).status,200);
 const original=(await f.db.prepare('SELECT object_key FROM files WHERE capture_id=?').bind(id(301)).first()).object_key;
 const head=await f.bucket.head(original);
 assert.equal(Buffer.from(head.checksums.sha256??[]).toString('hex'),digest('hello'));
 for(const [raw,code] of [['wrong','hash_mismatch'],['hi','size_mismatch'],['longer','size_mismatch']]){
  const response=await put(raw);assert.equal(response.status,400);assert.equal((await response.json()).error.code,code);
 }
 assert.equal((await put('hello')).status,200);
 assert.equal((await f.bucket.list()).objects.length,1);
 assert.equal((await f.bucket.get(original)).version,head.version);
 assert.equal(await(await f.bucket.get(original)).text(),'hello');
 assert.equal((await f.call(p+'/complete',{token:a.token,method:'POST'})).status,200);
 assert.equal((await put('wrong')).status,400);
 assert.equal((await put('hello')).status,200);
 assert.equal((await f.bucket.list()).objects.length,1);
 assert.equal((await f.bucket.head(original)).version,head.version);
 }finally{await f.close();}
});

test('completion rejects forged custom metadata without an R2 checksum',async()=>{
 const f=await fixture();try{
 const a=await f.identity('forged'),p='/v1/captures/'+id(302);
 await f.call(p,{token:a.token,method:'POST',body:{files:[file()]}});
 await f.bucket.put('forged','wrong',{customMetadata:{sha256:digest('hello')}});
 await f.db.prepare('UPDATE files SET object_key=? WHERE capture_id=?').bind('forged',id(302)).run();
 assert.equal((await f.call(p+'/complete',{token:a.token,method:'POST'})).status,409);
 }finally{await f.close();}
});
