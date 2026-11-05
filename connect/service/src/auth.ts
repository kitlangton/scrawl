import type {Env,Scope} from './types.ts';
import {analyticsChoice,analyticsIdentity,analyticsNoticeVersion,setAnalyticsChoice} from './analytics.ts';
import {identity,DEVICE_CODE_GRANT_TYPE} from './identity.ts';
import {random,hash,json,fail,stmt,body,bounded,session,csrf,usage,slots,rate,envelope,bearer,isRecord,apiBase} from './common.ts';

// Validate canonical age X25519 Bech32 encoding, including checksum and padding.
// This proves encoding, not possession or cryptographic suitability of a key.
function encryptionKey(value:unknown) {
 if(!isRecord(value)||Object.keys(value).sort().join(',')!=='recipient,signing,version'||value.version!==1||typeof value.recipient!=='string'||!/^age1[023456789acdefghjklmnpqrstuvwxyz]{58}$/.test(value.recipient)||typeof value.signing!=='string'||! /^(?!0{64}$)[a-f0-9]{64}$/.test(value.signing))fail('invalid_encryption_key');
 const alphabet='qpzry9x8gf2tvdw0s3jn54khce6mua7l',words=[...value.recipient.slice(4)].map(c=>alphabet.indexOf(c));
 const generators=[0x3b6a57b2,0x26508e6d,0x1ea119fa,0x3d4233dd,0x2a1462b3];let checksum=1;
 for(const word of [3,3,3,0,1,7,5,...words]) {
  const top=checksum>>>25;checksum=((checksum&0x1ffffff)<<5)^word;
  for(let i=0;i<5;i++)if((top>>>i)&1)checksum^=generators[i];
 }
 if(checksum!==1||(words[51]&15)!==0||words.slice(0,52).every(w=>w===0))fail('invalid_encryption_key');
 return {version:1,recipient:value.recipient,signing:value.signing};
}

interface Grant {id:string;client_id:string;device_hash:string;user_code:string;scope:Scope;account_id:string|null;user_id:string|null;expires_at:number;revoked_at:number|null;response:string|null;last_poll:number}
function returnTo(value:string|null) {
 const match=/^\/(?:connect|hosted)(\/device)?\/?(?:\?([^#]*))?$/.exec(value??'/connect');if(!match)fail('invalid_request');
 const query=new URLSearchParams(match[2]);if([...query.keys()].some(k=>k!=='code')||query.getAll('code').length>1)fail('invalid_request');
 return '/connect'+(match[1]||'')+(query.has('code')?'?'+new URLSearchParams({code:query.get('code')!}):'');
}
// Deliberately private dispatch: no raw Better Auth administration, session-token
// device endpoint, registration or OAuth token route is reachable from HTTP.
async function call(env:Env,path:string,data?:Record<string,unknown>,headers?:Headers) {
 const h=new Headers(headers);h.delete('Origin');h.delete('Authorization');
 if(h.has('Cookie'))h.set('Origin',env.SITE_ORIGIN);
 const form=path==='/oauth2/token'||path==='/oauth2/revoke';
 if(data)h.set('Content-Type',form?'application/x-www-form-urlencoded':'application/json');
 try{return await identity(env).handler(new Request(apiBase(env)+'/auth'+path,{method:data?'POST':'GET',headers:h,...data?{body:form?new URLSearchParams(data as Record<string,string>):JSON.stringify(data)}:{}}));}catch{fail('auth_unavailable',503);}
}
async function tokens(env:Env,g:Grant,response:Response) {
 if(response.status>=500||response.status===429)return {error:'server_error',status:response.status};
 const b=await body(response);
 if(!response.ok)return {error:typeof b.error==='string'?b.error:'server_error',status:response.status};
 if(typeof b.access_token!=='string'||typeof b.refresh_token!=='string'||typeof b.expires_in!=='number')fail('internal_error',500);
 return issued(env,g,b.access_token,b.refresh_token);
}
async function issued(env:Env,g:Grant,accessToken:string,refreshToken:string){
 const expiry=await stmt(env,'SELECT a.expiresAt AS access_expiry,r.expiresAt AS refresh_expiry FROM oauthAccessToken a JOIN oauthRefreshToken r ON a.clientId=r.clientId AND a.userId=r.userId WHERE a.token=? AND r.token=? AND a.clientId=? AND a.userId=? AND a.revoked IS NULL',await hash(accessToken),await hash(refreshToken),g.client_id,g.user_id).first<{access_expiry:string;refresh_expiry:string}>();
 if(!expiry)fail('auth_unavailable',503);
 const user=await stmt(env,'SELECT email FROM auth_user WHERE id=? AND emailVerified=1',g.user_id).first<{email:string}>();
 const live=await stmt(env,'SELECT id FROM auth_grants WHERE id=? AND revoked_at IS NULL',g.id).first();if(!live)fail('reauthorization_required',401);
 const seconds=(date:string)=>Math.max(0,Math.floor((Date.parse(date)-Date.now())/1000));
 const first=user?.email.trim()[Symbol.iterator]().next().value as string|undefined;
 const accountInitial=first&&/[\p{L}\p{N}]/u.test(first)?first.toLocaleUpperCase('und'):undefined;
 return {accessToken,refreshToken,expiresIn:seconds(expiry.access_expiry),refreshExpiresIn:seconds(expiry.refresh_expiry),scope:g.scope,grantId:g.id,...(accountInitial?{accountInitial}: {})};
}
export async function authRoutes(request:Request,env:Env,url:URL) {
 const p=url.pathname,m=request.method;
 if(p==='/v1/analytics/consent'&&(m==='GET'||m==='POST')) {
  const s=await session(request,env);
  if(m==='GET')return json(await analyticsChoice(env,s.account_id));
  csrf(request,env,s);const b=await body(request);
  if(Object.keys(b).some(k=>!['consented','revision','noticeVersion'].includes(k))||typeof b.consented!=='boolean'||
   !(b.revision===null||typeof b.revision==='string'&&/^[a-f0-9]{64}$/.test(b.revision))||
   b.consented&&b.noticeVersion!==analyticsNoticeVersion)fail('invalid_request');
  return json(await setAnalyticsChoice(env,s.account_id,b.consented,b.revision));
 }
 if(p==='/v1/encryption/peers'&&m==='GET') {
  const a=await bearer(request,env);
  // The ten-minute approval code expiry is deliberately NOT a device lifetime.
  // EXISTS avoids duplicate peers after refresh rotation and never trusts a
  // caller's account/scope/key. Only issued, still-renewable grants qualify.
  const {results}=await stmt(env,`SELECT g.id AS grantId,g.scope,k.version,k.recipient,k.signing FROM auth_grants g JOIN encryption_keys k ON k.grant_id=g.id WHERE g.account_id=? AND g.approved_at IS NOT NULL AND g.user_id IS NOT NULL AND g.revoked_at IS NULL AND EXISTS(SELECT 1 FROM oauthRefreshToken r WHERE r.clientId=g.client_id AND r.userId=g.user_id AND r.revoked IS NULL AND r.expiresAt>?) ORDER BY g.id LIMIT 65`,a.account_id,new Date().toISOString()).all<{grantId:string;scope:Scope;version:number;recipient:string;signing:string}>();
  if(results.length>64)fail('encryption_peer_limit',409);
  return json({accountId:a.account_id,grantId:a.grant_id,scope:a.scope,devices:results.map(({grantId,scope,...key})=>({grantId,scope,key:encryptionKey(key)}))});
 }
 if(p==='/auth/google/start'&&m==='GET') {
  await rate(env,'oauth:'+request.headers.get('CF-Connecting-IP'));
  const result=await identity(env).api.signInSocial({body:{provider:'google',callbackURL:env.SITE_ORIGIN+returnTo(url.searchParams.get('returnTo')),errorCallbackURL:apiBase(env)+'/auth/error'},asResponse:true});
  const b=await body(result);if(typeof b.url!=='string')fail('oauth_failed',401);
  const headers=new Headers(result.headers);headers.set('Location',b.url);return new Response(null,{status:302,headers});
 }
 if(p==='/auth/google/callback'&&m==='GET') {
  const target=new URL(request.url);target.pathname=new URL(env.API_ORIGIN).pathname.replace(/\/$/,'')+'/auth/callback/google';
  const response=await identity(env).handler(new Request(target,request));
  const location=response.headers.get('Location');
  if(!location||new URL(location).origin!==env.SITE_ORIGIN)fail('oauth_failed',401);
  const destination=new URL(location);let path;
  try{path=returnTo(location.slice(env.SITE_ORIGIN.length));}catch{fail('oauth_failed',401);}
  if(destination.hash)fail('oauth_failed',401);
  const headers=new Headers(response.headers);headers.set('Location',env.SITE_ORIGIN+path);return new Response(null,{status:302,headers});
 }
 if(p==='/v1/session'&&m==='GET') {
  const s=await session(request,env,false);
  return json({authenticated:!!s,csrfToken:s?.csrf||null,account:s?{email:s.email,beta:s.beta,quotaBytes:500000000,usedBytes:await usage(env,s.account_id),analyticsId:await analyticsIdentity(env,s.account_id),waitlist:{interestedAtFiveDollars:!!s.interest}}:null,slots:await slots(env)});
 }
 if(p==='/auth/logout'&&m==='POST') {
  const s=await session(request,env);csrf(request,env,s);
  const response=await call(env,'/sign-out',{},request.headers);if(!response.ok)fail('auth_unavailable',503);
  return new Response(JSON.stringify({ok:true}),{headers:response.headers});
 }
 if(p==='/v1/waitlist'&&m==='POST') {
  const s=await session(request,env);csrf(request,env,s);const b=await body(request);if(typeof b.interestedAtFiveDollars!=='boolean')fail('invalid_request');
  await stmt(env,'UPDATE accounts SET interest=? WHERE id=?',b.interestedAtFiveDollars?1:0,s.account_id).run();return json({ok:true});
 }
 if(p==='/v1/device/start'&&m==='POST') {
  await rate(env,'device:'+request.headers.get('CF-Connecting-IP'));
  const b=await body(request);if(b.scope!=='upload'&&b.scope!=='read')fail('invalid_request');
  const key=b.encryptionKey===undefined?null:encryptionKey(b.encryptionKey);

  // One library client per authorization also isolates library family invalidation.
  const registered=await call(env,'/oauth2/register',{client_name:'Apparent device',application_type:'native',token_endpoint_auth_method:'none',grant_types:[DEVICE_CODE_GRANT_TYPE,'refresh_token'],scope:b.scope+' offline_access'});
  const client=await body(registered);if(!registered.ok||typeof client.client_id!=='string')fail('auth_unavailable',503);
  const response=await call(env,'/device/code',{client_id:client.client_id,scope:b.scope+' offline_access'}),d=await body(response);
  if(!response.ok||typeof d.device_code!=='string'||typeof d.user_code!=='string')fail('auth_unavailable',503);
  const grant=random(),insert=stmt(env,'INSERT INTO auth_grants(id,client_id,device_hash,user_code,scope,expires_at) VALUES(?,?,?,?,?,?)',grant,client.client_id,await hash(d.device_code),d.user_code,b.scope,Date.now()+600000);
  if(key)await env.DB.batch([insert,stmt(env,'INSERT INTO encryption_keys(grant_id,version,recipient,signing) VALUES(?,?,?,?)',grant,key.version,key.recipient,key.signing)]);else await insert.run();
  return json({deviceCode:d.device_code,userCode:d.user_code,verificationUri:env.SITE_ORIGIN+'/connect/device?code='+d.user_code,expiresIn:600,interval:5});
 }
 if(p==='/v1/device/poll'&&m==='POST') {
  const {deviceCode}=await body(request);if(typeof deviceCode!=='string'||!/^[a-f0-9]{64}$/.test(deviceCode))fail('invalid_request');
  const g=await stmt(env,'SELECT * FROM auth_grants WHERE device_hash=?',await hash(deviceCode)).first<Grant>();if(!g||g.expires_at<=Date.now())return json({status:'expired'});
  if(g.revoked_at!==null)return json({status:'rejected'});
  const changed=await stmt(env,'UPDATE auth_grants SET last_poll=? WHERE id=? AND last_poll<=?',Date.now(),g.id,Date.now()-5000).run();if(!changed.meta.changes)fail('slow_down',429);
  // Do not consume a library approval until its application account binding
  // has completed; a crash in that gap requires a new explicit approval.
  if(!g.user_id||!g.account_id)return json({status:'pending'});
  if(g.response){const saved=JSON.parse(await envelope(env,g.response,true));return json({status:'approved',...await issued(env,g,saved.accessToken,saved.refreshToken)});}
  const result=await tokens(env,g,await call(env,'/oauth2/token',{grant_type:DEVICE_CODE_GRANT_TYPE,client_id:g.client_id,device_code:deviceCode}));
  if('error' in result){if(result.error==='authorization_pending')return json({status:'pending'});if(result.error==='slow_down'||result.status===429)fail('slow_down',429);if(result.status>=500)fail('auth_unavailable',503);return json({status:result.error==='expired_token'?'expired':'rejected'});}
  await stmt(env,'UPDATE auth_grants SET response=? WHERE id=? AND revoked_at IS NULL',await envelope(env,JSON.stringify(result)),g.id).run();
  return json({status:'approved',...result});
 }
 if(p==='/v1/device'&&m==='GET') {
  const s=await session(request,env);await rate(env,'preview:'+s.account_id,120);
  const code=url.searchParams.get('userCode');if(!code||!/^[A-F0-9]{12}$/.test(code))fail('invalid_request');
  const g=await stmt(env,'SELECT * FROM auth_grants WHERE user_code=?',code).first<Grant>();if(!g)fail('device_unavailable',404);
  const d=await stmt(env,'SELECT status FROM auth_device WHERE userCode=?',code).first<{status:string}>();
  return json({scope:g.scope,status:g.expires_at<=Date.now()?'expired':g.revoked_at!==null?'rejected':g.response?'approved':d?.status==='denied'?'rejected':d?.status||'pending',expiresAt:new Date(g.expires_at).toISOString()});
 }
 if(['/v1/device/approve','/v1/device/reject'].includes(p)&&m==='POST') {
  const s=await session(request,env);csrf(request,env,s);await rate(env,'approve:'+s.account_id);
  const input=await body(request),{userCode}=input;if(input.encryptionKey!==undefined||typeof userCode!=='string'||!/^[A-F0-9]{12}$/.test(userCode))fail('invalid_request');
  const g=await stmt(env,'SELECT * FROM auth_grants WHERE user_code=?',userCode).first<Grant>();if(!g||g.expires_at<=Date.now())fail('device_unavailable',404);
  if(g.revoked_at!==null)fail('device_unavailable',409);
  if(g.account_id&&g.account_id!==s.account_id)fail('device_unavailable',409);
  if(p.endsWith('approve')&&s.beta!=='active')fail('beta_required',403);
  const collision=()=>stmt(env,'SELECT 1 FROM encryption_keys k JOIN encryption_keys other ON other.grant_id!=k.grant_id JOIN auth_grants g ON g.id=other.grant_id WHERE k.grant_id=? AND (k.recipient=other.recipient OR k.signing=other.signing) AND g.approved_at IS NOT NULL AND (g.account_id!=? OR g.scope!=? OR k.recipient!=other.recipient OR k.signing!=other.signing)',g.id,s.account_id,g.scope).first();
  if(p.endsWith('approve')&&await collision())fail('encryption_key_conflict',409);
  const preview=await call(env,'/device?user_code='+encodeURIComponent(userCode),undefined,request.headers);if(!preview.ok)fail('device_unavailable',409);
  const result=await call(env,p.endsWith('approve')?'/device/approve':'/device/deny',{userCode},request.headers);
  if(!result.ok)fail('device_unavailable',409);
  try{await stmt(env,'UPDATE auth_grants SET account_id=?,user_id=?,revoked_at=COALESCE(revoked_at,?),approved_at=COALESCE(approved_at,?) WHERE id=?',s.account_id,s.user_id,p.endsWith('reject')?Date.now():null,p.endsWith('approve')?Date.now():null,g.id).run();}catch(error){if(await collision())fail('encryption_key_conflict',409);throw error;}
  return json({ok:true});
 }
 if(p==='/v1/token/refresh'&&m==='POST') {
  const {refreshToken}=await body(request);if(typeof refreshToken!=='string'||refreshToken.length>2048)fail('invalid_request');
  const r=await stmt(env,'SELECT g.*,r.revoked,r.rotatedAt,r.rotationReplayExpiresAt,r.rotationReplayResponse FROM oauthRefreshToken r JOIN auth_grants g ON g.client_id=r.clientId AND g.user_id=r.userId WHERE r.token=?',await hash(refreshToken)).first<Grant&{revoked:string|null;rotatedAt:string|null;rotationReplayExpiresAt:string|null;rotationReplayResponse:string|null}>();
  if(!r||r.revoked_at!==null)fail('reauthorization_required',401);
  // Tombstone BEFORE any fallible/non-atomic library family invalidation.
  // A concurrent mint can leave rows behind, but cannot regain authorization.
  if(r.revoked&&(!r.rotationReplayExpiresAt||Date.parse(r.rotationReplayExpiresAt)<=Date.now())){
   await stmt(env,'UPDATE auth_grants SET revoked_at=COALESCE(revoked_at,?) WHERE id=?',Date.now(),r.id).run();fail('reauthorization_required',401);
  }
  // Do not let an in-progress library rotation with no saved replay response
  // trigger family invalidation. Wait only within the library's bounded grace.
  if(r.revoked&&r.rotatedAt&&!r.rotationReplayResponse&&r.rotationReplayExpiresAt&&Date.parse(r.rotationReplayExpiresAt)>Date.now())fail('auth_unavailable',503);
  const result=await tokens(env,r,await call(env,'/oauth2/token',{grant_type:'refresh_token',client_id:r.client_id,refresh_token:refreshToken}));
  if('error' in result){
   if(result.status===429)fail('slow_down',429);
   if(result.status>=500)fail('auth_unavailable',503);
   const latest=await stmt(env,'SELECT rotationReplayExpiresAt FROM oauthRefreshToken WHERE token=?',await hash(refreshToken)).first<{rotationReplayExpiresAt:string|null}>();
   if(latest?.rotationReplayExpiresAt&&Date.parse(latest.rotationReplayExpiresAt)>Date.now())fail('auth_unavailable',503);
   await stmt(env,'UPDATE auth_grants SET revoked_at=? WHERE id=?',Date.now(),r.id).run();fail('reauthorization_required',401);
  }
  return json(result);
 }
 if(p==='/v1/token/revoke'&&m==='POST') {
  const raw=await bounded(request,131072),b=raw.length?await body(new Response(raw)):{};let g:Grant|null=null;
  if(typeof b.refreshToken==='string')g=await stmt(env,'SELECT g.* FROM oauthRefreshToken r JOIN auth_grants g ON g.client_id=r.clientId AND g.user_id=r.userId WHERE r.token=?',await hash(b.refreshToken)).first<Grant>();
  if(b.refreshToken!==undefined&&(!g||typeof b.refreshToken!=='string'))fail('unauthorized',401);
  if(request.headers.has('Authorization')){
   if(g){
    // Renewal proves authority even after access expiry. A supplied bearer
    // must still identify this SAME device, never another account or grant.
    const token=/^Bearer ([A-Za-z0-9._~-]{20,2048})$/.exec(request.headers.get('Authorization')!)?.[1];
    if(!token||!await stmt(env,'SELECT id FROM oauthAccessToken WHERE token=? AND clientId=? AND userId=?',await hash(token),g.client_id,g.user_id).first())fail('unauthorized',401);
   }else{const a=await bearer(request,env);g=await stmt(env,'SELECT * FROM auth_grants WHERE id=?',a.grant_id).first<Grant>();}
  }
  if(!g)fail('unauthorized',401);
  // This durable application authorization tombstone dominates any concurrent
  // library mint, so even a surviving row cannot access captures or renew.
  await stmt(env,'UPDATE auth_grants SET revoked_at=COALESCE(revoked_at,?),response=NULL WHERE id=?',Date.now(),g.id).run();
  const token=typeof b.refreshToken==='string'?b.refreshToken:request.headers.get('Authorization')!.slice(7);
  await call(env,'/oauth2/revoke',{client_id:g.client_id,token,token_type_hint:typeof b.refreshToken==='string'?'refresh_token':'access_token'});
  return json({ok:true});
 }
 return null;
}
