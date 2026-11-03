import {test,before,after} from 'node:test';
import assert from 'node:assert/strict';
import {identity} from '../../src/identity.ts';
import {Miniflare} from 'miniflare';
import {readFile} from 'node:fs/promises';
import {serializeSignedCookie} from 'better-call';
import type {Env} from '../../src/types.ts';

let mf:Miniflare,db:Env['DB'],otherDb:Env['DB'];
before(async()=>{
 mf=new Miniflare({modules:[{type:'ESModule',path:'identity-fixture.js',contents:'export default {fetch(){return new Response(null)}}'}],compatibilityDate:'2026-02-05',d1Databases:['DB','OTHER']});
 db=await mf.getD1Database('DB') as unknown as Env['DB'];otherDb=await mf.getD1Database('OTHER') as unknown as Env['DB'];
 for(const target of [db,otherDb])for(const name of ['0001.sql','0002.sql','0004.sql','0005.sql'])await target.exec((await readFile('migrations/'+name,'utf8')).replace(/^--.*$/gm,'').replaceAll('\n',' '));
});
after(async()=>{await mf?.dispose();});
function environment():Env {
 return {DB:db,BUCKET:{} as Env['BUCKET'],
  API_ORIGIN:'https://api.example.invalid',SITE_ORIGIN:'https://app.example.invalid',
  GOOGLE_CLIENT_ID:'synthetic-client',GOOGLE_CLIENT_SECRET:'synthetic-provider-secret',
  BETTER_AUTH_SECRET:'synthetic-auth-secret-for-tests-at-least-32-characters',
  TOKEN_ENCRYPTION_KEY:Buffer.alloc(32,1).toString('base64')};
}

test('unchanged binding environment reuses static auth initialization without session cookie caching',async()=>{
 const env=environment(),first=identity(env);await first.$context;
 assert.equal(identity(env),first);
 assert.equal(first.options.session?.cookieCache?.enabled,false);
});

test('auth configuration changes cannot reuse previous database, secret or trusted origins',async()=>{
 const env=environment();let previous=identity(env);await previous.$context;
 for(const [key,value] of Object.entries({DB:otherDb,API_ORIGIN:'https://new-api.example.invalid',
  SITE_ORIGIN:'https://new-app.example.invalid',LEGACY_SITE_ORIGIN:'https://legacy.example.invalid',
  GOOGLE_CLIENT_ID:'other-synthetic-client',GOOGLE_CLIENT_SECRET:'other-provider-secret',
  BETTER_AUTH_SECRET:'other-synthetic-auth-secret-at-least-32-characters',
  OAUTH_DISCOVERY_URL:'https://provider.example.invalid/.well-known/openid-configuration'})) {
  Object.assign(env,{[key]:value});const next=identity(env);await next.$context;assert.notEqual(next,previous,key);previous=next;
 }
 assert.equal(previous.options.secret,env.BETTER_AUTH_SECRET);
 assert.equal(previous.options.database,env.DB);
 assert.deepEqual(previous.options.trustedOrigins,[env.SITE_ORIGIN,env.LEGACY_SITE_ORIGIN]);
 env.BETTER_AUTH_SECRET='invalid';assert.throws(()=>identity(env));
});

test('different binding objects never share an auth instance',async()=>{
 const first=environment(),second={...first},a=identity(first),b=identity(second);await Promise.all([a.$context,b.$context]);assert.notEqual(a,b);
});

test('failed initialization is evicted rather than poisoning later requests',async()=>{
 const env={...environment(),DB:{} as Env['DB']};
 const first=identity(env);await assert.rejects(first.$context);
 const second=identity(env);assert.notEqual(second,first);await assert.rejects(second.$context);
});

async function seedSession(target:Env['DB'],name:string,secret:string) {
 const now=new Date().toISOString(),expiry=new Date(Date.now()+3600000).toISOString();
 await target.prepare('INSERT INTO auth_user(id,name,email,emailVerified,createdAt,updatedAt) VALUES(?,?,?,1,?,?)').bind(name,name,name+'@example.invalid',now,now).run();
 await target.prepare('INSERT INTO auth_session(id,token,userId,expiresAt,createdAt,updatedAt) VALUES(?,?,?,?,?,?)').bind(name,name,name,expiry,now,now).run();
 return new Headers({Cookie:(await serializeSignedCookie('__Secure-apparent.session_token',name,secret,{path:'/'})).split(';')[0]});
}

test('cached static auth immediately rejects a session deleted after its first request',async()=>{
 const env=environment(),headers=await seedSession(db,'cache-revocation',env.BETTER_AUTH_SECRET),auth=identity(env);
 assert.equal((await auth.api.getSession({headers}))?.user.id,'cache-revocation');
 await db.prepare('DELETE FROM auth_session WHERE id=?').bind('cache-revocation').run();
 assert.equal(identity(env),auth);
 assert.equal(await identity(env).api.getSession({headers}),null);
});

test('same-wrapper database retargeting does not reuse previously read session data',async()=>{
 let current=db;
 const wrapper=new Proxy(db,{get(_target,key){const value=Reflect.get(current,key);return typeof value==='function'?(...args:unknown[])=>Reflect.apply(Reflect.get(current,key),current,args):value;}});
 const env={...environment(),DB:wrapper};
 const headers=await seedSession(db,'cache-rebinding',env.BETTER_AUTH_SECRET),auth=identity(env);
 assert.equal((await auth.api.getSession({headers}))?.user.id,'cache-rebinding');
 current=otherDb;
 assert.equal(identity(env),auth);
 assert.equal(await auth.api.getSession({headers}),null);
});
