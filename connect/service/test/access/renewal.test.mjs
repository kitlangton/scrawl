import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {Miniflare} from 'miniflare';
import {fixture,digest} from '../helpers.mjs';

async function authorize(f,a,scope='upload') {
 const start=await f.call('/v1/device/start',{method:'POST',body:{scope}});
 assert.equal(start.status,200);const d=await start.json();
 assert.equal((await f.call('/v1/device/approve',{...a,method:'POST',body:{userCode:d.userCode}})).status,200);
 const response=await f.call('/v1/device/poll',{method:'POST',body:{deviceCode:d.deviceCode}});
 assert.equal(response.status,200);const tokens=await response.json();
 assert.equal(tokens.status,'approved');assert.equal(typeof tokens.refreshToken,'string');
 assert.ok(tokens.expiresIn>0);assert.ok(tokens.refreshExpiresIn>tokens.expiresIn);assert.ok(tokens.grantId);
 return {...tokens,deviceCode:d.deviceCode};
}

test('device authorization renews immutable scope and grant; revocation isolates sibling grants',async()=>{
 const f=await fixture();try{
 const a=await f.identity('renewal'),one=await authorize(f,a,'read'),two=await authorize(f,a);
 const renew=()=>f.call('/v1/token/refresh',{method:'POST',body:{refreshToken:one.refreshToken,scope:'upload'}});
 const response=await renew();assert.equal(response.status,200);const next=await response.json();
 assert.equal(next.scope,'read');assert.equal(next.grantId,one.grantId);assert.notEqual(next.refreshToken,one.refreshToken);
 assert.equal((await f.call('/v1/account',{token:next.accessToken})).status,200);
 assert.equal((await f.call('/v1/token/revoke',{token:next.accessToken,method:'POST'})).status,200);
 for(const token of [one.accessToken,next.accessToken])assert.equal((await f.call('/v1/account',{token})).status,401);
 assert.equal((await f.call('/v1/token/refresh',{method:'POST',body:{refreshToken:next.refreshToken}})).status,401);
 assert.equal((await f.call('/v1/account',{token:two.accessToken})).status,200);
 assert.equal((await f.call('/v1/token/refresh',{method:'POST',body:{refreshToken:two.refreshToken}})).status,200);
 }finally{await f.close();}
});

test('auth cleanup removes expired replay material and terminal clients without revoking live devices or reseeding budgets',async()=>{
 const f=await fixture();try{
 const a=await f.identity('auth-cleanup'),t=await authorize(f,a),sibling=await authorize(f,a);
 const next=await(await f.call('/v1/token/refresh',{method:'POST',body:{refreshToken:t.refreshToken}})).json();
 const pending=await(await f.call('/v1/device/start',{method:'POST',body:{scope:'read'}})).json();
 await f.call('/v1/token/revoke',{token:sibling.accessToken,method:'POST'});
 await f.db.exec('UPDATE auth_grants SET expires_at=0');
 await f.db.prepare('UPDATE oauthRefreshToken SET rotationReplayExpiresAt=? WHERE token=?').bind(new Date(Date.now()-1000).toISOString(),digest(t.refreshToken)).run();
 await f.db.prepare('UPDATE oauthAccessToken SET expiresAt=? WHERE token=?').bind(new Date(Date.now()-1000).toISOString(),digest(t.accessToken)).run();
 const ledger=await f.db.prepare('SELECT * FROM r2_budget').first();
 assert.equal((await(await f.mf.getWorker()).scheduled()).outcome,'ok');
 assert.equal((await f.db.prepare('SELECT count(*) n FROM auth_grants WHERE response IS NOT NULL').first()).n,0);
 assert.equal((await f.db.prepare('SELECT rotationReplayResponse FROM oauthRefreshToken WHERE token=?').bind(digest(t.refreshToken)).first()).rotationReplayResponse,null);
 assert.equal(await f.db.prepare('SELECT id FROM oauthAccessToken WHERE token=?').bind(digest(t.accessToken)).first(),null,'ordinary expired access cleanup must leave renewal authorized');
 for(const clause of ['id=?','user_code=?']){
  const value=clause==='id=?'?sibling.grantId:pending.userCode;
  assert.equal((await f.db.prepare('SELECT count(*) n FROM oauthClient WHERE clientId=(SELECT client_id FROM auth_grants WHERE '+clause+')').bind(value).first()).n,0);
 }
 assert.equal((await f.call('/v1/account',{token:next.accessToken})).status,200);
 assert.equal((await f.call('/v1/token/refresh',{method:'POST',body:{refreshToken:next.refreshToken}})).status,200);
 assert.deepEqual(await f.db.prepare('SELECT * FROM r2_budget').first(),ledger);
 }finally{await f.close();}
});

test('approved poll retries preserve credentials but report remaining rather than initial lifetimes',async()=>{
 const f=await fixture();try{
 const a=await f.identity('poll-lifetime'),t=await authorize(f,a);
 await f.db.prepare('UPDATE oauthAccessToken SET expiresAt=? WHERE token=?').bind(new Date(Date.now()+50000).toISOString(),digest(t.accessToken)).run();
 await f.db.prepare('UPDATE oauthRefreshToken SET expiresAt=? WHERE token=?').bind(new Date(Date.now()+90000).toISOString(),digest(t.refreshToken)).run();
 await f.db.prepare('UPDATE auth_grants SET last_poll=0 WHERE id=?').bind(t.grantId).run();
 const response=await f.call('/v1/device/poll',{method:'POST',body:{deviceCode:t.deviceCode}});assert.equal(response.status,200);const retry=await response.json();
 assert.equal(retry.accessToken,t.accessToken);assert.equal(retry.refreshToken,t.refreshToken);assert.equal(retry.grantId,t.grantId);
 assert.ok(retry.expiresIn>0&&retry.expiresIn<=50);assert.ok(retry.refreshExpiresIn>0&&retry.refreshExpiresIn<=90);
 }finally{await f.close();}
});

test('migration retires only old credentials and preserves incumbent metadata and spent ledger',async()=>{
 const mf=new Miniflare({modules:[{type:'ESModule',path:'migration-worker.mjs',contents:'export default {}'}],d1Databases:['DB']});try{
 const db=await mf.getD1Database('DB');
 const migrate=async n=>db.exec((await readFile('migrations/'+n+'.sql','utf8')).replace(/^--.*$/gm,'').replaceAll('\n',' '));
 for(const n of ['0001','0002','0003'])await migrate(n);
 const account=digest('https://accounts.google.com/legacy'),capture='a'.repeat(32);
 await db.prepare("INSERT INTO accounts VALUES(?,?,'active',1,123)").bind(account,'legacy@example.com').run();
 await db.prepare("INSERT INTO captures(id,account_id,manifest,bytes,expires_at,pending_until) VALUES(?,?,'[]',0,9999999999999,9999999999999)").bind(capture,account).run();
 await db.prepare("INSERT INTO tokens VALUES(?,?,'read',9999999999999)").bind(digest('legacy-token'),account).run();
 await db.exec('UPDATE r2_budget SET a_used=5,b_used=7');
 const before=await db.prepare('SELECT * FROM r2_budget').first(),old=await db.prepare('SELECT * FROM accounts').first(),recording=await db.prepare('SELECT * FROM captures').first();
 await migrate('0004');
 assert.deepEqual(await db.prepare('SELECT * FROM r2_budget').first(),before);assert.deepEqual(await db.prepare('SELECT * FROM accounts').first(),old);assert.deepEqual(await db.prepare('SELECT * FROM captures').first(),recording);
 for(const table of ['tokens','devices','sessions','oauth_states'])assert.equal(await db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name=?").bind(table).first(),null);
 await assert.rejects(migrate('0004'));assert.deepEqual(await db.prepare('SELECT * FROM r2_budget').first(),before);
 }finally{await mf.dispose();}
});
