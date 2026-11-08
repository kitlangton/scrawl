import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createAPI, decodeResponse, ReporterHalt, type Response} from './api.ts';
const response = (patch: Partial<Response> = {}): Response => ({status:200,headers:{},data:{ok:true},...patch});
test('response decoding retains quota/request headers across redirects without exposing request data', () => {
 const parsed=decodeResponse('HTTP/2.0 302 Found\r\nlocation: ignored\r\n\r\nHTTP/2.0 403 Forbidden\r\nX-RateLimit-Remaining: 0\r\nX-RateLimit-Reset: 2000\r\nX-GitHub-Request-Id: request-id\r\n\r\n{"message":"API rate limit exceeded"}');
 assert.equal(parsed.status,403);assert.equal(parsed.headers['x-ratelimit-reset'],'2000');
 assert.equal(parsed.data.message,'API rate limit exceeded');
 assert.throws(()=>decodeResponse('HTTP/2.0 200 OK\nmissing-boundary'));
 assert.throws(()=>decodeResponse('HTTP/2.0 200 OK\n\nprivate-response-fragment'),{message:'Invalid GitHub API JSON response'});
});
test('primary quota failure stops every later request and exposes reset time', () => {
 let calls=0;
 const api=createAPI({transport:()=>{calls++;return response({status:403,headers:{'x-ratelimit-remaining':'0','x-ratelimit-reset':'2000','x-github-request-id':'safe-id'},data:{message:'API rate limit exceeded'}});}});
 for(let i=0;i<3;i++) assert.throws(()=>api('repos/example/status'),(e:any)=>e instanceof ReporterHalt && e.retryAt===2_000_000 && e.message.includes('safe-id'));
 assert.equal(calls,1);
});
test('secondary limit honors Retry-After and cannot loop', () => {
 let calls=0;const api=createAPI({now:()=>1000,transport:()=>{calls++;return response({status:429,headers:{'retry-after':'90'}});}});
 assert.throws(()=>api('first'),(e:any)=>e.retryAt===91_000);
 assert.throws(()=>api('second'),ReporterHalt);assert.equal(calls,1);
});
test('budget and quota reserve halt before another HTTP request', () => {
 let calls=0;const api=createAPI({maximum:2,transport:()=>{calls++;return response();}});
 api('one');api('two');assert.throws(()=>api('three'),ReporterHalt);assert.equal(calls,2);
 const reserve=createAPI({transport:()=>response({headers:{'x-ratelimit-remaining':'5','x-ratelimit-reset':'2000'}})});
 reserve('one');assert.throws(()=>reserve('two'),ReporterHalt);assert.equal(reserve.count(),1);
});
test('same-credential preflight rejects exhausted quota without status writes', () => {
 const paths:string[]=[];const api=createAPI({maximum:60,transport:path=>{paths.push(path);return response({data:{resources:{core:{remaining:69,reset:2000}}}});}});
 assert.throws(()=>api.preflight(),ReporterHalt);assert.throws(()=>api('status',{}),ReporterHalt);
 assert.deepEqual(paths,['rate_limit']);
});
test('only immutable source reads are reused; authority, live refs and evidence refresh', () => {
 const paths:string[]=[];const api=createAPI({pause:()=>{},transport:path=>{paths.push(path);return response();}});
 const source='repos/example/contents/workflow.yml?ref='+'a'.repeat(40);
 for(let i=0;i<2;i++) {api(source);api('repos/example/git/ref/heads/main');api('repos/example/collaborators/writer/permission');api('repos/example/actions/workflows/workflow.yml/runs');api('repos/example/status',{});}
 assert.equal(paths.filter(path=>path===source).length,1);assert.equal(paths.length,9);
});


test('mutating requests are spaced without retrying or caching writes',()=>{
 let now=1000;const delays:number[]=[];let writes=0;
 const api=createAPI({now:()=>now,pause:ms=>{delays.push(ms);now+=ms;},transport:()=>{writes++;return response();}});
 api('status',{});now+=100;api('status',{});api('status',{});
 assert.deepEqual(delays,[900,1000]);assert.equal(writes,3);
});
test('malformed quota headers halt and unknown secondary reset requires at least a minute',()=>{
 for(const headers of [{'x-ratelimit-remaining':'unknown'},{'x-ratelimit-reset':'NaN'}]) {
  const api=createAPI({transport:()=>response({headers})});
  assert.throws(()=>api('one'),ReporterHalt);assert.throws(()=>api('two'),ReporterHalt);assert.equal(api.count(),1);
 }
 const api=createAPI({now:()=>1000,transport:()=>response({status:403,data:{message:'secondary rate limit'}})});
 assert.throws(()=>api('one'),(e:any)=>e.retryAt===61_000);
});
