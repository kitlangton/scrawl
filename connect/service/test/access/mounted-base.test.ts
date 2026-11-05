import {test} from 'node:test';
import assert from 'node:assert/strict';
import {fixture} from '../helpers.mjs';
import {trustedSiteOrigin} from '../../src/common.ts';
import type {Env} from '../../src/types.ts';

test('mounted /connect Worker base preserves exact routing and origin security',async()=>{
 const f=await fixture({API_ORIGIN:'https://api.example/connect',SITE_ORIGIN:'https://app.example'});
 try {
  assert.equal((await f.call('/v1/session')).status,200);
  for(const path of ['/connected/v1/session','/private/v1/session','/connect/v1/session']) {
   const response=await f.call(path);assert.equal(response.status,404,path);
  }
  assert.equal((await f.call('/v1/session',{origin:'https://app.example'})).headers.get('Access-Control-Allow-Origin'),'https://app.example');
  for(const origin of ['https://evil.example','https://app.example/connect','null']) {
   const response=await f.call('/v1/session',{origin});assert.equal(response.headers.get('Access-Control-Allow-Origin'),null,origin);
   assert.equal((await f.call('/',{origin,method:'OPTIONS'})).status,403,origin);
  }
  for(const path of ['/v1/session','/connected/v1/session','/connectivity/v1/session']) assert.equal((await f.mf.dispatchFetch('https://api.example'+path)).status,404,path);
  const auth=await f.call('/auth/google/start?returnTo=%2Fconnect');
  assert.equal(auth.status,302);
  assert.equal(new URL(auth.headers.get('Location')!).searchParams.get('redirect_uri'),'https://api.example/connect/auth/google/callback');
 } finally {await f.close();}
});

test('configured site origins remain exact allowlist entries, never truncated URL bases',()=>{
 const env={SITE_ORIGIN:'https://app.example/connect',LEGACY_SITE_ORIGIN:'https://legacy.example/path'} as Env;
 assert.equal(trustedSiteOrigin('https://app.example',env),false);
 assert.equal(trustedSiteOrigin('https://legacy.example',env),false);
});
