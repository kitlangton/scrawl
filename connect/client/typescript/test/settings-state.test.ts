import assert from 'node:assert/strict';
import { test } from 'node:test';
import { deriveLocalState } from '../dist/settings-state.cjs';

const origin='https://hosted.tryapparent.com';
const grant=(scope:'upload'|'read',patch:Record<string,unknown>={})=>({server:origin,scope,
  authRequired:false,hasRefresh:true,expiresAt:2000,refreshExpiresAt:4000,accountInitial:'AB',
  token:'private-access-secret',refreshToken:'private-refresh-secret',unknown:'private-unknown-secret',...patch});
const facts=(patch:Record<string,unknown>={})=>({cloud:{server:origin,enabled:true,uploadConsent:true},hostedSync:true,
  revocationPending:false,upload:grant('upload'),read:null,sharedLinksUrl:`${origin}/hosted/shares`,
  privateFact:'private-fact-secret',...patch});

test('upload permission requires every independent gate while retaining a current connection',()=>{
  const baseline=deriveLocalState(facts(),1000);
  assert.deepEqual(baseline.snapshot,{server:origin,connected:true,uploadsEnabled:true,
    reauthorizationRequired:false,renewalRequired:false,accountInitial:'AB'});
  for(const patch of [
    {cloud:{server:origin,enabled:false,uploadConsent:true}},
    {cloud:{server:origin,enabled:true,uploadConsent:false}},
    {hostedSync:false},
  ]) {
    const state=deriveLocalState(facts(patch),1000);
    assert.equal(state.snapshot?.connected,true);
    assert.equal(state.snapshot?.uploadsEnabled,false);
  }
  const defaultServer=deriveLocalState(facts({cloud:{server:'',enabled:true,uploadConsent:true},upload:grant('upload',{server:origin})}),1000);
  assert.equal(defaultServer.snapshot?.server,null);
  assert.equal(defaultServer.snapshot?.connected,true);
  assert.equal(defaultServer.snapshot?.uploadsEnabled,false);
});

test('read grant is connected without enabling uploads, and only matching grants count',()=>{
  const readOnly=deriveLocalState(facts({upload:null,read:grant('read')}),1000);
  assert.equal(readOnly.snapshot?.connected,true);
  assert.equal(readOnly.snapshot?.uploadsEnabled,false);
  assert.equal(readOnly.snapshot?.accountInitial,'AB');
  const mismatched=deriveLocalState(facts({upload:grant('upload',{server:'https://other.example'}),read:null}),1000);
  assert.equal(mismatched.snapshot?.connected,false);
  assert.equal(mismatched.snapshot?.reauthorizationRequired,false);
  assert.equal(mismatched.snapshot?.accountInitial,null);
});

test('expired renewable access requests renewal; expired legacy and auth-required grants require reauthorization',()=>{
  const renewable=deriveLocalState(facts({upload:grant('upload',{expiresAt:900,refreshExpiresAt:4000})}),1000);
  assert.equal(renewable.snapshot?.connected,false);
  assert.equal(renewable.snapshot?.renewalRequired,true);
  assert.equal(renewable.snapshot?.reauthorizationRequired,false);
  const legacy=deriveLocalState(facts({upload:grant('upload',{hasRefresh:false,expiresAt:900,refreshExpiresAt:null})}),1000);
  assert.equal(legacy.snapshot?.reauthorizationRequired,true);
  assert.equal(legacy.snapshot?.renewalRequired,false);
  const revoked=deriveLocalState(facts({upload:grant('upload',{authRequired:true})}),1000);
  assert.equal(revoked.snapshot?.reauthorizationRequired,true);
  assert.equal(revoked.snapshot?.uploadsEnabled,false);
});

test('server normalization follows the native origin validator and rejects invalid configuration',()=>{
  const trailing=deriveLocalState(facts({cloud:{server:`${origin}///`,enabled:true,uploadConsent:true}}),1000);
  assert.equal(trailing.snapshot?.server,origin);
  assert.equal(trailing.snapshot?.connected,true);
  for(const server of ['http://public.example','https://host/path','https://host?x=1','https://user@host',
    'https://host:0','https://host:65536','https://[::1]']) {
    const state=deriveLocalState(facts({cloud:{server,enabled:true,uploadConsent:true}}),1000);
    assert.equal(state.error,'invalid-server',server);
    assert.equal(state.snapshot,null);
    assert.equal(state.sharedLinksUrl,null);
  }
  assert.equal(deriveLocalState(facts({cloud:{server:'http://localhost:8787',enabled:true,uploadConsent:true},
    upload:grant('upload',{server:'http://localhost:8787'})}),1000).snapshot?.connected,true);
});

test('malformed optional account initial drops alone and private or unknown fields never project',()=>{
  const state=deriveLocalState(facts({upload:grant('upload',{accountInitial:'private@example.test'})}),1000);
  assert.equal(state.error,null);
  assert.equal(state.snapshot?.connected,true);
  assert.equal(state.snapshot?.accountInitial,null);
  const json=JSON.stringify(state);
  for(const secret of ['private-access-secret','private-refresh-secret','private-unknown-secret',
    'private-fact-secret','private@example.test']) assert.equal(json.includes(secret),false,secret);
  assert.equal(state.sharedLinksUrl,`${origin}/hosted/shares`);
});

test('malformed facts and pending revocation fail closed; invalid shared links are omitted',()=>{
  assert.deepEqual(deriveLocalState(null,1000),{snapshot:null,error:'local-state',sharedLinksUrl:null});
  assert.equal(deriveLocalState(facts({revocationPending:true}),1000).error,'local-state');
  for(const link of ['file:///private','https://user:pass@hosted.tryapparent.com/hosted/shares',
    `${origin}/hosted/shares?token=private`,`${origin}/hosted/shares#fragment`]) {
    assert.equal(deriveLocalState(facts({sharedLinksUrl:link}),1000).sharedLinksUrl,null,link);
  }
  assert.equal(deriveLocalState(facts({upload:{...grant('upload'),expiresAt:'bad'}}),1000).error,'local-state');
});
