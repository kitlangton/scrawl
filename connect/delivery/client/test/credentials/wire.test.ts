import test from 'node:test';
import assert from 'node:assert/strict';
import {decodeStoredCredential,mergeRenewalReply} from '../../src/credentials/wire.ts';

const server='https://host.example/connect';
const legacy={server,scope:'upload',token:'token',generation:'a'.repeat(32)};
const bytes=(v:unknown)=>new TextEncoder().encode(JSON.stringify(v));
const read=(v:unknown,s='upload',url=server)=>decodeStoredCredential(bytes(v),s,url);
const renewable={...legacy,refreshToken:'refresh',expiresAt:200,refreshExpiresAt:500,grantId:'grant'};
const reply={scope:'upload',accessToken:'new-access',refreshToken:'new-refresh',grantId:'grant',expiresIn:60,refreshExpiresIn:300};

test('reads legacy fields with null/default semantics and retains wire encryption spelling',()=>{
 const c=read({...legacy,site:null,encryption_identity:'opaque',accountInitial:'AB',ignoredFutureField:true});
 assert.equal(c.site,null);assert.equal(c.authRequired,false);assert.equal(c.encryption_identity,'opaque');assert.equal(c.accountInitial,'AB');
});
test('allows explicit null optionals and requires complete renewable tuple',()=>{
 assert.equal(read({...legacy,refreshToken:null,expiresAt:null,refreshExpiresAt:null,grantId:null}).refreshToken,null);
 for(const partial of [{refreshToken:'r'},{refreshToken:'r',expiresAt:1},{expiresAt:1,refreshExpiresAt:2,grantId:'g'}])
   assert.throws(()=>read({...legacy,...partial}),/Invalid credential/);
 assert.equal(read(renewable).grantId,'grant');
});
test('pins exact scope and server (including root versus mounted path)',()=>{
 assert.throws(()=>read(legacy,'read'),/Invalid credential/);assert.throws(()=>read({...legacy,server:'https://host.example'}),/Invalid credential/);
 assert.throws(()=>read(legacy,'upload','https://host.example'),/Invalid credential/);
});
test('rejects malformed lifecycle, unsafe expiry, token bounds and generation',()=>{
 for(const v of [{...legacy,generation:'A'.repeat(32)},{...legacy,token:'x'.repeat(8193)},{...renewable,expiresAt:Number.MAX_SAFE_INTEGER+1},{...renewable,grantId:'g'.repeat(513)}])
 assert.throws(()=>read(v),/Invalid credential/);
});
test('merges reply immutably from original issue time and pins grant/scope/identity',()=>{
 const c=read(renewable),before={...c};const merged=mergeRenewalReply(c,reply,100);
 assert.deepEqual(c,before);assert.equal(merged.expiresAt,160);assert.equal(merged.refreshExpiresAt,400);assert.equal(merged.server,c.server);assert.equal(merged.generation,c.generation);
 for(const r of [{...reply,grantId:'other'},{...reply,scope:'read'},{...reply,expiresIn:Number.MAX_SAFE_INTEGER}]) assert.throws(()=>mergeRenewalReply(c,r,100),/Invalid credential/);
 assert.throws(()=>mergeRenewalReply(c,reply,Number.MAX_SAFE_INTEGER),/Invalid credential/);
});
test('preserves authRequired fail-closed behavior on stored state and clears on successful merge',()=>{
 const c=read({...renewable,authRequired:true});assert.equal(c.authRequired,true);assert.equal(mergeRenewalReply(c,reply,100).authRequired,false);
});
test('strict bounded decoder rejects duplicate keys, malformed JSON and oversized bytes without echoing input',()=>{
 for(const raw of ['{"server":"x","server":"secret"}', '{', ' '.repeat(32769)]) {
  assert.throws(()=>decodeStoredCredential(new TextEncoder().encode(raw),'upload',server),e=>String(e).includes('Invalid credential')&&!String(e).includes('secret'));
 }
});

test('a delayed response does not extend expiry beyond original issue time',()=>{
 const c=read(renewable), merged=mergeRenewalReply(c,reply,100), responseObservedAt=500;
 assert.equal(merged.expiresAt,160); assert.ok(merged.expiresAt!<responseObservedAt);
 assert.equal(merged.refreshExpiresAt,400); assert.ok(merged.refreshExpiresAt!<responseObservedAt);
});
test('a rotated reply scope is refused without mutating private credential facts',()=>{
 const c=read({...renewable,encryption_identity:'private-identity'}), before={...c};
 assert.throws(()=>mergeRenewalReply(c,{...reply,scope:'read'},100),/Invalid credential/);
 assert.deepEqual(c,before);
});
