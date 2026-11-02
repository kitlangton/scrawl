import { Miniflare } from 'miniflare';
import { readFile } from 'node:fs/promises';
import { createHash, randomBytes } from 'node:crypto';
import {serializeSignedCookie} from 'better-call';
const authSecret='synthetic-better-auth-secret-only-for-tests-32bytes';
export const digest = s => createHash('sha256').update(s).digest('hex');
export async function fixture(bindings={},options={}) {
 const {instances=false,...workerOptions}=options;
 const config={modules:true,scriptPath:'dist/worker.js',compatibilityDate:'2026-02-05',compatibilityFlags:['nodejs_compat'],d1Databases:{DB:'shared-db'},r2Buckets:{BUCKET:'shared-bucket'},bindings:{SITE_ORIGIN:'https://site.example',API_ORIGIN:'https://api.example',BETTER_AUTH_SECRET:authSecret,GOOGLE_CLIENT_ID:'test-client',GOOGLE_CLIENT_SECRET:'controlled-secret',TOKEN_ENCRYPTION_KEY:Buffer.alloc(32,7).toString('base64'),...bindings},...workerOptions};
 config.modules=[{type:'ESModule',path:'dist/worker.js',contents:config.script??await readFile(config.scriptPath,'utf8')}];delete config.script;delete config.scriptPath;
 config.outboundService??=async request=>{
  if(request.url==='https://accounts.google.com/.well-known/openid-configuration')return Response.json({issuer:'https://accounts.google.com',authorization_endpoint:'https://accounts.google.com/o/oauth2/v2/auth',token_endpoint:'https://oauth2.googleapis.com/token',jwks_uri:'https://www.googleapis.com/oauth2/v3/certs',id_token_signing_alg_values_supported:['RS256']});
  if(new URL(request.url).hostname==='127.0.0.1')return fetch(request.url,{method:request.method,headers:Object.fromEntries(request.headers),...request.body?{body:await request.arrayBuffer()}:{}});
  return new Response(null,{status:503});
 };
 const runtime=instances?{workers:[{...config,name:'first'},{...config,name:'second'}]}:config;
 const mf = new Miniflare(runtime);
 let db=await mf.getD1Database('DB',instances?'first':undefined),bucket=await mf.getR2Bucket('BUCKET',instances?'first':undefined),generation=0;
 await db.exec((await readFile('migrations/0001.sql','utf8')).replaceAll('\n',' '));
 await db.exec((await readFile('migrations/0002.sql','utf8')).replaceAll('\n',' '));
 await db.exec((await readFile('migrations/0004.sql','utf8')).replace(/^--.*$/gm,'').replaceAll('\n',' '));
 const call=async (path,{token,cookie,csrf,origin,method='GET',body,raw,headers={},instance}={})=>{
  const target=instance?await mf.getWorker(instance):mf,fetch=instance?target.fetch.bind(target):target.dispatchFetch.bind(target);
  return fetch('https://api.example'+path,{method,redirect:'manual',headers:{...(token?{Authorization:'Bearer '+token}:{}),...(cookie?{Cookie:cookie}:{}),...(csrf?{'X-CSRF-Token':csrf}:{}),...(origin?{Origin:origin}:{}),...headers},...(body!==undefined?{body:JSON.stringify(body)}:raw!==undefined?{body:raw}:{})});
 };
 async function identity(id,beta='active') {
  const subject=id,account=digest('https://accounts.google.com/'+subject),token=randomBytes(32).toString('hex'),read=randomBytes(32).toString('hex'),session=randomBytes(32).toString('hex'),csrf=digest(session+authSecret),now=new Date().toISOString(),expiry=new Date(Date.now()+86400000).toISOString();
  await db.prepare('INSERT INTO accounts(id,email,beta,created_at) VALUES(?,?,?,?)').bind(account,id+'@example.com',beta,Date.now()).run();
  await db.prepare('INSERT INTO auth_user(id,name,email,emailVerified,createdAt,updatedAt) VALUES(?,?,?,1,?,?)').bind(id,id,id+'@example.com',now,now).run();
  await db.prepare("INSERT INTO auth_account(id,accountId,providerId,userId,createdAt,updatedAt) VALUES(?,?,'google',?,?,?)").bind(id,subject,id,now,now).run();
  for(const [t,s] of [[token,'upload'],[read,'read']]) {
   const client=id+'-'+s;
   await db.prepare('INSERT INTO oauthClient(id,clientId,redirectUris) VALUES(?,?,?)').bind(client,client,'[]').run();
   await db.prepare('INSERT INTO auth_grants(id,client_id,device_hash,user_code,scope,account_id,user_id,expires_at) VALUES(?,?,?,?,?,?,?,?)').bind(client,client,digest(client),client,s,account,id,Date.now()+86400000).run();
   await db.prepare('INSERT INTO oauthAccessToken(id,token,clientId,userId,scopes,expiresAt) VALUES(?,?,?,?,?,?)').bind(client,digest(t),client,id,JSON.stringify([s]),expiry).run();
  }
  await db.prepare('INSERT INTO auth_session(id,token,userId,expiresAt,createdAt,updatedAt) VALUES(?,?,?,?,?,?)').bind(session,session,id,expiry,now,now).run();
  return {id:account,token,read,cookie:(await serializeSignedCookie('__Secure-apparent.session_token',session,authSecret,{path:'/'})).split(';')[0],csrf,origin:'https://site.example'};
 }
 async function reload(){
  const suffix='\n// reloaded '+(++generation),modules=w=>w.modules.map(m=>({...m,contents:m.contents+suffix}));
  await mf.setOptions(instances?{workers:runtime.workers.map(w=>({...w,modules:modules(w)}))}:{...config,modules:modules(config)});
  db=await mf.getD1Database('DB',instances?'first':undefined);bucket=await mf.getR2Bucket('BUCKET',instances?'first':undefined);
 }
 return {mf,get db(){return db;},get bucket(){return bucket;},call,identity,reload,close:()=>mf.dispose()};
}
export const id = n => n.toString(16).padStart(32,'0');
export const file = (path='summary.md',text='hello') => ({path,size:Buffer.byteLength(text),sha256:digest(text)});
