import {test} from 'node:test';
import assert from 'node:assert/strict';
import {build} from 'esbuild';
import {fixture,id,digest} from '../helpers.mjs';

// Count JS reads of the actual incoming body, rather than timing the local R2
// emulator. Native piping must keep upload size out of the JS execution budget.
const instrumented=await build({stdin:{resolveDir:process.cwd(),contents:`
 import worker from './src/worker.ts';
 export default {async fetch(request,env){
  let reads=0;
  if(request.method==='PUT'&&request.body){
   const getReader=request.body.getReader.bind(request.body);
   request.body.getReader=(...args)=>{
    const reader=getReader(...args),read=reader.read.bind(reader);
    reader.read=(...args)=>{reads++;return read(...args);};return reader;
   };
  }
  const response=await worker.fetch(request,env);
  response.headers.set('X-Test-JS-Body-Reads',String(reads));return response;
 }};`},bundle:true,format:'esm',platform:'browser',conditions:['workerd'],external:['node:*'],write:false});

test('large uploads avoid per-chunk JavaScript body reads while retaining verified bytes',async()=>{
 const f=await fixture({}, {scriptPath:undefined,script:instrumented.outputFiles[0].text});
 try{
  const a=await f.identity('native-upload'),p='/v1/captures/'+id(322);
  const bytes=Buffer.alloc(20_821_791,19);
  const entry={path:'recording.mp4',size:bytes.length,sha256:digest(bytes)};
  assert.equal((await f.call(p,{token:a.token,method:'POST',body:{files:[entry]}})).status,200);
  const response=await f.call(p+'/files/recording.mp4',{token:a.token,method:'PUT',raw:bytes});
  assert.equal(response.status,200,await response.text());
  assert.equal(Number(response.headers.get('X-Test-JS-Body-Reads')),0);
  assert.equal((await f.call(p+'/complete',{token:a.token,method:'POST'})).status,200);
  const {objects}=await f.bucket.list();assert.equal(objects.length,1);
  const object=await f.bucket.get(objects[0].key);
  assert.equal(digest(Buffer.from(await object.arrayBuffer())),entry.sha256);
 }finally{await f.close();}
});
