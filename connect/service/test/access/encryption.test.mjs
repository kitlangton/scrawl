import {test} from 'node:test';
import assert from 'node:assert/strict';
import {generateKeyPairSync} from 'node:crypto';
import {readFile} from 'node:fs/promises';
import {Miniflare} from 'miniflare';
import {fixture,id,file,digest} from '../helpers.mjs';

// Real endpoint public keys, encoded with the age X25519 Bech32 wire grammar.
function key() {
 const bytes=generateKeyPairSync('x25519').publicKey.export({format:'der',type:'spki'}).subarray(-32);
 const alphabet='qpzry9x8gf2tvdw0s3jn54khce6mua7l',words=[];let accumulator=0,bits=0;
 for(const byte of bytes){accumulator=(accumulator<<8)|byte;bits+=8;while(bits>=5){bits-=5;words.push((accumulator>>>bits)&31);}}
 if(bits)words.push((accumulator<<(5-bits))&31);
 let chk=1;for(const n of [3,3,3,0,1,7,5,...words,0,0,0,0,0,0]){const top=chk>>>25;chk=((chk&0x1ffffff)<<5)^n;[0x3b6a57b2,0x26508e6d,0x1ea119fa,0x3d4233dd,0x2a1462b3].forEach((g,i)=>{if((top>>>i)&1)chk^=g;});}chk^=1;
 return {version:1,recipient:'age1'+[...words,...Array.from({length:6},(_,i)=>(chk>>>(5*(5-i)))&31)].map(n=>alphabet[n]).join(''),signing:generateKeyPairSync('ed25519').publicKey.export({format:'der',type:'spki'}).subarray(-32).toString('hex')};
}
const peers=async(f,token)=>{const r=await f.call('/v1/encryption/peers',{token});assert.equal(r.status,200);return r.json();};
async function start(f,scope='read',encryptionKey=key()) {
 const r=await f.call('/v1/device/start',{method:'POST',body:{scope,encryptionKey}});assert.equal(r.status,200);return {...await r.json(),key:encryptionKey};
}
async function approve(f,a,d) {return f.call('/v1/device/approve',{...a,method:'POST',body:{userCode:d.userCode}});}
async function authorize(f,a,scope='read',descriptor=key()) {
 const d=await start(f,scope,descriptor);assert.equal((await approve(f,a,d)).status,200);
 const r=await f.call('/v1/device/poll',{method:'POST',body:{deviceCode:d.deviceCode}});assert.equal(r.status,200);return {...await r.json(),key:descriptor};
}
async function code(r,status,error) {assert.equal(r.status,status);assert.equal((await r.json()).error.code,error);}

test('approved registry is account/scope/grant-bound; renewable lifetime, refresh and revoke preserve immutable keys',async()=>{
 const f=await fixture();try{
 const a=await f.identity('keys'),b=await f.identity('foreign-keys'),pending=await start(f),rejected=await start(f);
 assert.equal((await f.call('/v1/device/reject',{...a,method:'POST',body:{userCode:rejected.userCode}})).status,200);
 const reader=await authorize(f,a),sender=await authorize(f,a,'upload'),foreign=await authorize(f,b);
 assert.deepEqual(await peers(f,b.read),{accountId:b.id,grantId:'foreign-keys-read',scope:'read',devices:[{grantId:foreign.grantId,scope:'read',key:foreign.key}]});
 const registry=await peers(f,reader.accessToken);assert.equal(registry.accountId,a.id);assert.equal(registry.grantId,reader.grantId);assert.equal(registry.scope,'read');
 assert.deepEqual(registry.devices.sort((x,y)=>x.scope.localeCompare(y.scope)),[{grantId:reader.grantId,scope:'read',key:reader.key},{grantId:sender.grantId,scope:'upload',key:sender.key}]);
 await code(await f.call('/v1/encryption/peers'),401,'unauthorized');
 await code(await f.call('/v1/encryption/peers',{method:'POST',token:a.token,body:{encryptionKey:key()}}),404,'not_found');
 // Grant approval code is expired, but the actual refresh authority remains live.
 await f.db.exec('UPDATE auth_grants SET expires_at=0');
 assert.equal((await peers(f,a.read)).devices.length,2);
 const response=await f.call('/v1/token/refresh',{method:'POST',body:{refreshToken:reader.refreshToken,encryptionKey:key(),scope:'upload'}});assert.equal(response.status,200);const renewed=await response.json();
 assert.equal(renewed.grantId,reader.grantId);assert.equal(renewed.scope,'read');assert.deepEqual((await peers(f,renewed.accessToken)).devices, (await peers(f,a.read)).devices);
 assert.deepEqual(await f.db.prepare('SELECT version,recipient,signing FROM encryption_keys WHERE grant_id=?').bind(reader.grantId).first(),reader.key);
 assert.equal((await(await f.mf.getWorker()).scheduled()).outcome,'ok');assert.equal((await peers(f,a.read)).devices.length,2);
 // Actual refresh expiry excludes a peer even while its access token is live.
 await f.db.prepare('UPDATE oauthRefreshToken SET expiresAt=? WHERE clientId=(SELECT client_id FROM auth_grants WHERE id=?)').bind(new Date(0).toISOString(),sender.grantId).run();
 assert.deepEqual((await peers(f,a.read)).devices.map(x=>x.grantId),[reader.grantId]);
 assert.equal((await f.call('/v1/token/revoke',{token:renewed.accessToken,method:'POST'})).status,200);
 assert.deepEqual((await peers(f,a.read)).devices,[]);
 await code(await f.call('/v1/encryption/peers',{token:renewed.accessToken}),401,'reauthorization_required');
 assert.equal((await f.db.prepare('SELECT count(*) n FROM encryption_keys').first()).n,5,'pending/rejected/revoked bindings remain immutable');
 assert.ok(pending.userCode);
 }finally{await f.close();}
});

test('keys require canonical checksum encoding; explicit approval cannot spoof or rebind descriptors',async()=>{
 const f=await fixture();try{
 const a=await f.identity('binding'),b=await f.identity('other-binding'),k=key();
 const mutations=[null,{...k,version:2},{...k,recipient:k.recipient.toUpperCase()},{...k,recipient:k.recipient.slice(0,-1)+(k.recipient.endsWith('q')?'p':'q')},{...k,signing:k.signing.toUpperCase()},{...k,signing:'0'.repeat(64)},{...k,extra:'untrusted'}];
 for(const encryptionKey of mutations)await code(await f.call('/v1/device/start',{method:'POST',body:{scope:'read',encryptionKey}}),400,'invalid_encryption_key');
 assert.equal((await f.db.prepare('SELECT count(*) n FROM encryption_keys').first()).n,0);
 const d=await start(f,'read',k),squatter=await start(f,'read',k);
 const denied=await start(f,'read',k);assert.equal((await f.call('/v1/device/reject',{...b,method:'POST',body:{userCode:denied.userCode}})).status,200,'denial is not approval and cannot squat a public key');
 await code(await f.call('/v1/device/approve',{...a,method:'POST',body:{userCode:d.userCode,encryptionKey:key()}}),400,'invalid_request');
 await code(await f.call('/v1/device/approve',{cookie:a.cookie,method:'POST',body:{userCode:d.userCode}}),403,'csrf_failed');
 assert.equal((await approve(f,a,d)).status,200,'pending unauthenticated duplicate must not squat a public key');
 await code(await approve(f,b,squatter),409,'encryption_key_conflict');
 await code(await approve(f,b,d),409,'device_unavailable');
 const upload=await start(f,'upload',k);await code(await approve(f,a,upload),409,'encryption_key_conflict');
 const half=await start(f,'read',{...key(),recipient:k.recipient});await code(await approve(f,a,half),409,'encryption_key_conflict');
 // Logout/reauthorization can retain the identity in the SAME account and scope.
 const repeat=await start(f,'read',k);assert.equal((await approve(f,a,repeat)).status,200);
 await assert.rejects(f.db.prepare('UPDATE encryption_keys SET signing=? WHERE recipient=?').bind(key().signing,k.recipient).run());
 const row=await f.db.prepare('SELECT g.account_id,g.scope,k.recipient,k.signing FROM auth_grants g JOIN encryption_keys k ON k.grant_id=g.id WHERE g.user_code=?').bind(d.userCode).first();
 assert.deepEqual(row,{account_id:a.id,scope:'read',recipient:k.recipient,signing:k.signing});
 const raceKey=key(),one=await start(f,'read',raceKey),two=await start(f,'read',raceKey);
 const responses=await Promise.all([approve(f,a,one),approve(f,b,two)]);assert.deepEqual(responses.map(r=>r.status).sort(),[200,409]);
 const granted=await f.db.prepare('SELECT count(*) n FROM auth_grants g JOIN encryption_keys k ON k.grant_id=g.id WHERE k.recipient=? AND g.approved_at IS NOT NULL').bind(raceKey.recipient).first();assert.equal(granted.n,1,'serialized binding must reject a simultaneous cross-account approval');
 }finally{await f.close();}
});

test('registry fails closed rather than truncating more than 64 live peers',async()=>{
 const f=await fixture();try{
 const a=await f.identity('peer-cap'),device=await authorize(f,a),base=await f.db.prepare('SELECT * FROM auth_grants WHERE id=?').bind(device.grantId).first(),refresh=await f.db.prepare('SELECT * FROM oauthRefreshToken WHERE clientId=?').bind(base.client_id).first();
 // Seed realistic issued credential rows to avoid a rate-limit bypass in production.
 for(let i=0;i<64;i++){
  const grant='cap-'+i,k=key();
  await f.db.prepare("INSERT INTO oauthClient(id,clientId,redirectUris) VALUES(?,?,'[]')").bind(grant,grant).run();
  await f.db.prepare('INSERT INTO auth_grants(id,client_id,device_hash,user_code,scope,account_id,user_id,expires_at,approved_at) VALUES(?,?,?,?,?,?,?,0,1)').bind(grant,grant,digest(grant),grant,'read',a.id,base.user_id).run();
  await f.db.prepare('INSERT INTO encryption_keys VALUES(?,1,?,?)').bind(grant,k.recipient,k.signing).run();
  await f.db.prepare('INSERT INTO oauthRefreshToken(id,token,clientId,userId,expiresAt,scopes) VALUES(?,?,?,?,?,?)').bind(grant,digest('refresh-'+grant),grant,base.user_id,refresh.expiresAt,refresh.scopes).run();
 }
 await code(await f.call('/v1/encryption/peers',{token:a.token}),409,'encryption_peer_limit');
 await f.db.prepare("UPDATE auth_grants SET revoked_at=? WHERE id='cap-0'").bind(Date.now()).run();
 assert.equal((await peers(f,a.token)).devices.length,64);
 }finally{await f.close();}
});

const opaque=()=>[file('envelope.json','opaque public descriptor'),file('sealed/part-000000.age','sealed bytes')];
const reserve=(f,a,n,files=opaque(),format='apparent-encrypted-v1')=>f.call('/v1/captures/'+id(n),{token:a.token,method:'POST',body:{files,format}});

test('opaque reservation/upload/resume/download uses existing hash, ownership, tombstone and budget accounting',async()=>{
 const f=await fixture();try{
 const a=await f.identity('opaque'),b=await f.identity('opaque-other'),p='/v1/captures/'+id(189),files=opaque();
 assert.equal((await reserve(f,a,189)).status,200);assert.equal((await reserve(f,a,189)).status,200);
 await code(await reserve(f,a,189,[...files,file('sealed/part-000001.age')]),409,'manifest_conflict');
 await code(await f.call(p,{token:b.read}),404,'not_found');
 await code(await f.call(p+'/files/sealed/part-000000.age',{token:a.read,method:'PUT',raw:'sealed bytes'}),403,'forbidden');
 await code(await f.call(p+'/files/sealed/part-000000.age',{token:a.token,method:'PUT',raw:'wrong length'}),400,'hash_mismatch');
 await f.db.exec('UPDATE r2_budget SET a_cap=a_used');
 await code(await f.call(p+'/files/sealed/part-000000.age',{token:a.token,method:'PUT',raw:'sealed bytes'}),503,'budget_exhausted');
 await f.db.exec('UPDATE r2_budget SET a_cap=500000');
 for(const [path,text] of [['envelope.json','opaque public descriptor'],['sealed/part-000000.age','sealed bytes']]){
  assert.equal((await f.call(p+'/files/'+path,{token:a.token,method:'PUT',raw:text})).status,200);
  assert.equal((await f.call(p+'/files/'+path,{token:a.token,method:'PUT',raw:text})).status,200);
 }
 assert.equal((await f.call(p+'/complete',{token:a.token,method:'POST'})).status,200);
 const manifest=await(await f.call(p,{token:a.read})).json();assert.equal(manifest.format,'apparent-encrypted-v1');assert.deepEqual(manifest.files,files);
 const download=await f.call(p+'/files/sealed/part-000000.age',{token:a.read});assert.equal(download.headers.get('Content-Type'),'application/octet-stream');assert.equal(await download.text(),'sealed bytes');
 const stored=await f.db.prepare('SELECT * FROM captures WHERE id=?').bind(id(189)).first();assert.equal(stored.bytes,files.reduce((n,x)=>n+x.size,0));assert.equal(stored.manifest,JSON.stringify(files));
 const ledger=await f.db.prepare('SELECT * FROM r2_budget').first();assert.equal(ledger.physical_used,stored.bytes);assert.equal((await f.bucket.list()).objects.length,2);
 assert.equal((await f.call(p,{token:a.token,method:'DELETE'})).status,200);
 for(let i=0;i<2;i++)assert.equal((await(await f.mf.getWorker()).scheduled()).outcome,'ok');
 assert.equal((await f.db.prepare('SELECT physical_used FROM r2_budget').first()).physical_used,0);assert.equal((await f.bucket.list()).objects.length,0);
 await code(await reserve(f,a,189),404,'not_found');
 assert.equal((await f.db.prepare('SELECT a_used FROM r2_budget').first()).a_used,ledger.a_used,'cleanup must never refund lifetime spending');
 assert.equal((await reserve(f,a,190)).status,200,'fresh reservation can reuse reclaimed physical capacity');
 }finally{await f.close();}
});

test('format discriminator rejects unknown/mixed manifests before writes, preserving legacy and existing bounds',async()=>{
 const f=await fixture();try{
 const a=await f.identity('formats');
 const cases=[['future-v2',opaque(),'unsupported_format'],[null,opaque(),'unsupported_format'],['plaintext-v1',opaque(),'invalid_path'],['apparent-encrypted-v1',[...opaque(),file()],'invalid_path'],['apparent-encrypted-v1',[file('envelope.json')],'invalid_manifest'],['apparent-encrypted-v1',[file('sealed/part-000000.age')],'invalid_manifest'],['apparent-encrypted-v1',[file('envelope.json'),file('sealed/part-0.age')],'invalid_path'],['apparent-encrypted-v1',[...opaque(),file('sealed/part-000000.age')],'invalid_manifest'],['apparent-encrypted-v1',[file('envelope.json'),{...file('sealed/part-000000.age'),size:90000001}],'file_too_large'],['apparent-encrypted-v1',[file('envelope.json'),...Array.from({length:512},(_,i)=>file(`sealed/part-${String(i).padStart(6,'0')}.age`))],'invalid_manifest'],['apparent-encrypted-v1',[file('envelope.json'),...Array.from({length:6},(_,i)=>({...file(`sealed/part-${String(i).padStart(6,'0')}.age`),size:90000000}))],'quota_exceeded']];
 const before=await f.db.prepare('SELECT * FROM r2_budget').first();
 for(const [format,files,error] of cases)await code(await reserve(f,a,191,files,format),error==='quota_exceeded'?409:400,error);
 assert.equal((await f.db.prepare('SELECT count(*) n FROM captures').first()).n,0);assert.deepEqual(await f.db.prepare('SELECT * FROM r2_budget').first(),before);
 const r=await f.call('/v1/captures/'+id(192),{token:a.token,method:'POST',body:{files:[file()]}});assert.equal(r.status,200);
 assert.equal((await f.db.prepare('SELECT format FROM captures').first()).format,'plaintext-v1');assert.equal((await(await f.call('/v1/captures/'+id(192),{token:a.read})).json()).format,undefined);
 }finally{await f.close();}
});

test('encrypted sharing refuses mutations and every public read without R2 work or share creation',async()=>{
 const f=await fixture({SHARING_ENABLED:'true'});try{
 await f.db.exec((await readFile('migrations/0003.sql','utf8')).replaceAll('\n',' '));
 const a=await f.identity('no-share'),p='/v1/captures/'+id(193);assert.equal((await reserve(f,a,193)).status,200);
 const ledger=await f.db.prepare('SELECT * FROM r2_budget').first();
 for(const suffix of ['/share','/share/progress'])await code(await f.call(p+suffix,{token:a.token,method:'POST',body:{state:'uploading'}}),409,'encrypted_sharing_unsupported');
 assert.equal((await f.db.prepare('SELECT count(*) n FROM shares').first()).n,0);
 // Defensive legacy/future rogue share row must not bypass the read/render guard.
 const secret='a'.repeat(64);await f.db.prepare('INSERT INTO shares(id,capture_id,account_id,hash,envelope,created_at) VALUES(?,?,?,?,?,?)').bind(id(194),id(193),a.id,digest(secret),'not a decryptable capability',Date.now()).run();
 for(const suffix of ['','/manifest.json','/thumbnail.png','/files/envelope.json','/files/sealed/part-000000.age'])for(const method of ['GET','HEAD']){
  const r=await f.call('/s/'+secret+suffix,{method});assert.equal(r.status,409);if(method==='GET')assert.equal((await r.json()).error.code,'encrypted_sharing_unsupported');
 }
 await code(await f.call('/v1/shares',{token:a.read}),409,'encrypted_sharing_unsupported');
 await code(await f.call('/v1/shares/'+id(194),{token:a.token,method:'DELETE'}),409,'encrypted_sharing_unsupported');
 assert.equal((await f.db.prepare('SELECT revoked_at FROM shares').first()).revoked_at,null);
 assert.deepEqual(await f.db.prepare('SELECT * FROM r2_budget').first(),ledger);
 }finally{await f.close();}
});

test('0005 preserves existing capture/grant/ledger and replay errors without resetting spent budget',async()=>{
 const mf=new Miniflare({modules:[{type:'ESModule',path:'migration.mjs',contents:'export default {}'}],d1Databases:['DB']});try{
 const db=await mf.getD1Database('DB'),migrate=async n=>db.exec((await readFile('migrations/'+n+'.sql','utf8')).replace(/^--.*$/gm,'').replaceAll('\n',' '));
 for(const n of ['0001','0002','0003','0004'])await migrate(n);
 await db.prepare("INSERT INTO accounts VALUES(?,?,'active',1,123)").bind('synthetic','synthetic@example.com').run();
 await db.prepare("INSERT INTO captures(id,account_id,manifest,bytes,expires_at,pending_until) VALUES(?,'synthetic','[]',0,9999999999999,9999999999999)").bind(id(195)).run();
 await db.exec('UPDATE r2_budget SET a_used=7,b_used=9');const before=await db.prepare('SELECT * FROM r2_budget').first(),capture=await db.prepare('SELECT * FROM captures').first();
 await migrate('0005');assert.deepEqual(await db.prepare('SELECT * FROM captures').first(),{...capture,format:'plaintext-v1'});assert.deepEqual(await db.prepare('SELECT * FROM r2_budget').first(),before);
 await assert.rejects(migrate('0005'));assert.deepEqual(await db.prepare('SELECT * FROM r2_budget').first(),before);
 }finally{await mf.dispose();}
});
