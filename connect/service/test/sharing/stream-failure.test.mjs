import {test} from 'node:test';
import assert from 'node:assert/strict';
import {build} from 'esbuild';
import {fixture,id,file} from '../helpers.mjs';
const rejected=await build({stdin:{resolveDir:process.cwd(),contents:`
 import worker from './src/worker.ts';
 export default {fetch(request,env){
  if(request.headers.has('X-Test-Reject'))return worker.fetch(request,{...env,BUCKET:{
   put(){return Promise.reject(new Error('provider failure secret-token-do-not-expose'));},
   delete:key=>env.BUCKET.delete(key)
  }});
  return worker.fetch(request,env);
 }};`},bundle:true,format:'esm',platform:'browser',conditions:['workerd'],external:['node:*'],write:false});

test('early rejected R2 PUT cancels a stalled source and returns a sanitized error without hanging', {timeout:10000},async()=>{
 const f=await fixture({}, {scriptPath:undefined,script:rejected.outputFiles[0].text});try{
 const a=await f.identity('reject'),p='/v1/captures/'+id(320);
 await f.call(p,{token:a.token,method:'POST',body:{files:[file()]}});
 const stream=new ReadableStream({start(c){c.enqueue(new TextEncoder().encode('he'));}});
 const response=await f.mf.dispatchFetch('https://api.example'+p+'/files/summary.md',{method:'PUT',headers:{Authorization:'Bearer '+a.token,'X-Test-Reject':'1'},body:stream,duplex:'half'});
 assert.equal(response.status,500);
 assert.equal((await response.json()).error.message,'Request failed');
 assert.equal((await f.bucket.list()).objects.length,0);
 assert.equal((await f.db.prepare('SELECT object_key FROM files WHERE capture_id=?').bind(id(320)).first()).object_key,null);
 }finally{await f.close();}
});

test('a source stream failure cannot publish an object or complete the capture', {timeout:10000},async()=>{
 const f=await fixture();try{
 const a=await f.identity('source'),p='/v1/captures/'+id(321);
 await f.call(p,{token:a.token,method:'POST',body:{files:[file()]}});
 const stream=new ReadableStream({start(c){c.enqueue(new TextEncoder().encode('he'));},pull(c){c.error(new Error('source failed'));}});
 await f.mf.dispatchFetch('https://api.example'+p+'/files/summary.md',{method:'PUT',headers:{Authorization:'Bearer '+a.token},body:stream,duplex:'half'}).catch(()=>{});
 assert.equal((await f.call(p+'/complete',{token:a.token,method:'POST'})).status,409);
 assert.equal((await f.bucket.list()).objects.length,0);
 }finally{await f.close();}
});
