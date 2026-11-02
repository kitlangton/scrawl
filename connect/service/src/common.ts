import type {Env, SessionRow, TokenRow} from './types.ts';
import {identity} from './identity.ts';
export const DAY = 86400000;
export const random = (n=32) => [...crypto.getRandomValues(new Uint8Array(n))].map(x=>x.toString(16).padStart(2,'0')).join('');
export const hex = (bytes: Uint8Array) => [...bytes].map(x=>x.toString(16).padStart(2,'0')).join('');
export const hash = async (value: string) => hex(new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(value))));
export const json = (value: unknown) => Response.json(value);
export class ApiError extends Error { declare code: string; declare status: number; constructor(code: string,status=400,message=code.replaceAll('_',' ')) {super(message);this.code=code;this.status=status;} }
export function fail(code: string,status=400): never {throw new ApiError(code,status);}
export const stmt = (env: Env,sql: string,...args: unknown[]) => env.DB.prepare(sql).bind(...args);
export function isRecord(value: unknown): value is Record<string, unknown> {return !!value && typeof value==='object' && !Array.isArray(value);}
export async function body(request: Request | Response,limit=131072): Promise<Record<string, unknown>> {
 const bytes=await bounded(request,limit);
 try { const v: unknown=JSON.parse(new TextDecoder().decode(bytes));if(!isRecord(v))fail('invalid_request'); return v; } catch { fail('invalid_request'); }
}
export async function bounded(request: Request | Response,limit: number) {
 if(Number(request.headers.get('Content-Length'))>limit)fail('body_too_large',413);
 const reader=request.body?.getReader();if(!reader)return new Uint8Array();
 const chunks=[];let length=0;
 try {for(;;){const {done,value}=await reader.read();if(done)break;length+=value.length;if(length>limit){await reader.cancel();fail('body_too_large',413);}chunks.push(value);}}finally{reader.releaseLock();}
 const out=new Uint8Array(length);let at=0;for(const c of chunks){out.set(c,at);at+=c.length;}return out;
}

export function session(request: Request,env: Env,required: false): Promise<SessionRow | null>;
export function session(request: Request,env: Env,required?: true): Promise<SessionRow>;
export async function session(request: Request,env: Env,required=true) {
 const found=await identity(env).api.getSession({headers:request.headers});
 if(!found||!found.user.emailVerified){if(required)fail('unauthorized',401);return null;}
 const account=await admit(env,found.user.id);
 if(!account){if(required)fail('unauthorized',401);return null;}
 const row=await stmt(env,'SELECT id AS account_id,email,beta,interest FROM accounts WHERE id=?',account).first<SessionRow>();
 return {...row!,hash:found.session.id,user_id:found.user.id,csrf:await hash(found.session.token+env.BETTER_AUTH_SECRET),expires_at:new Date(found.session.expiresAt).getTime()};
}
export async function admit(env:Env,userId:string) {
 const user=await stmt(env,"SELECT p.accountId,u.email FROM auth_account p JOIN auth_user u ON u.id=p.userId WHERE p.userId=? AND p.providerId='google' AND u.emailVerified=1",userId).first<{accountId:string;email:string}>();
 if(!user)return null;
 // Preserve the incumbent issuer/subject account ID. Never remap by email.
 const account=await hash('https://accounts.google.com/'+user.accountId);
 await stmt(env,"INSERT INTO accounts(id,email,beta,created_at) SELECT ?,?,CASE WHEN (SELECT count(*) FROM accounts WHERE beta='active')<20 THEN 'active' ELSE 'waitlist' END,? ON CONFLICT(id) DO UPDATE SET email=excluded.email",account,user.email,Date.now()).run();
 return account;
}
// CORS and cookie-authenticated mutations must agree on this exact allowlist.
export function trustedSiteOrigin(origin: string | null,env: Env): origin is string {
 if(!origin||origin==='null')return false;
 try {if(new URL(origin).origin!==origin)return false;}catch{return false;}
 return origin===env.SITE_ORIGIN||origin===env.LEGACY_SITE_ORIGIN;
}
export function csrf(request: Request,env: Env,s: SessionRow) {if(!trustedSiteOrigin(request.headers.get('Origin'),env) || request.headers.get('X-CSRF-Token')!==s.csrf)fail('csrf_failed',403);}
export async function bearer(request: Request,env: Env,write=false) {
 const token=/^Bearer ([A-Za-z0-9._~-]{20,2048})$/.exec(request.headers.get('Authorization')||'')?.[1];if(!token)fail('unauthorized',401);
 const row=await stmt(env,'SELECT t.token AS hash,g.id AS grant_id,g.account_id,g.scope,t.expiresAt AS expires_at,a.beta FROM oauthAccessToken t JOIN auth_grants g ON g.client_id=t.clientId AND g.user_id=t.userId JOIN accounts a ON a.id=g.account_id WHERE t.token=? AND t.expiresAt>? AND t.revoked IS NULL AND g.revoked_at IS NULL',await hash(token),new Date().toISOString()).first<TokenRow>();
 if(!row)fail('reauthorization_required',401);if(row.beta!=='active')fail('beta_required',403);if(write&&row.scope!=='upload')fail('forbidden',403);return row;
}
export async function usage(env: Env,account: string) {return (await stmt(env,"SELECT COALESCE(sum(bytes),0) AS bytes FROM captures WHERE account_id=? AND status!='deleted' AND expires_at>? AND (status='ready' OR pending_until>?)",account,Date.now(),Date.now()).first<{bytes: number}>())!.bytes;}
export async function slots(env: Env) {const {used}=(await stmt(env,"SELECT count(*) AS used FROM accounts WHERE beta='active'").first<{used: number}>())!;return {used,limit:20,remaining:Math.max(0,20-used)};}
export async function rate(env: Env,key: string,max=30) {
 const window=Math.floor(Date.now()/3600000);
 const row=await stmt(env,'INSERT INTO rate_limits(key,window,count) VALUES(?,?,1) ON CONFLICT(key) DO UPDATE SET count=CASE WHEN window=excluded.window THEN count+1 ELSE 1 END,window=excluded.window RETURNING count',key,window).first<{count: number}>();
 if(row!.count>max)fail('slow_down',429);
}
export async function envelope(env: Env,value: string,decrypt=false) {
 const bytes=Uint8Array.from(atob(env.TOKEN_ENCRYPTION_KEY||''),c=>c.charCodeAt(0));if(bytes.length!==32)throw new Error('key configuration');
 const key=await crypto.subtle.importKey('raw',bytes,'AES-GCM',false,['encrypt','decrypt']);
 if(decrypt){const b=Uint8Array.from(atob(value),c=>c.charCodeAt(0));return new TextDecoder().decode(await crypto.subtle.decrypt({name:'AES-GCM',iv:b.slice(0,12)},key,b.slice(12)));}
 const iv=crypto.getRandomValues(new Uint8Array(12)),cipher=new Uint8Array(await crypto.subtle.encrypt({name:'AES-GCM',iv},key,new TextEncoder().encode(value)));
 return btoa(String.fromCharCode(...iv,...cipher));
}
