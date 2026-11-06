import { decodeStrictJson } from '../recipient/json.ts';

const opaque=(v:unknown,max=8192):v is string=>typeof v==='string'&&v.length>0&&new TextEncoder().encode(v).length<=max&&[...new TextEncoder().encode(v)].every(b=>b>=0x21&&b<=0x7e);
const obj=(v:unknown):v is Record<string,unknown>=>v!==null&&typeof v==='object'&&!Array.isArray(v);
const bad=():never=>{throw new Error('Invalid credential')};
const own=(o:Record<string,unknown>,k:string)=>Object.hasOwn(o,k);
const optional=(o:Record<string,unknown>,k:string):unknown=>own(o,k)?o[k]:null;

/** Private credential representation matching the native stored wire format. */
export type StoredCredential=Readonly<{server:string;site:string|null;scope:string;token:string;generation:string;
 refreshToken:string|null;expiresAt:number|null;refreshExpiresAt:number|null;grantId:string|null;authRequired:boolean;
 encryption_identity:string|null;accountInitial:string|null}>;
function validate(c:StoredCredential,expectedScope:string,expectedServer:string):StoredCredential {
 if(c.scope!==expectedScope||c.server!==expectedServer||!opaque(c.token)||!/^[0-9a-f]{32}$/.test(c.generation))bad();
 const legacy=c.refreshToken===null&&c.refreshExpiresAt===null&&c.grantId===null;
 const renewable=c.refreshToken!==null&&Number.isSafeInteger(c.expiresAt)&&c.expiresAt!>0&&Number.isSafeInteger(c.refreshExpiresAt)&&c.refreshExpiresAt!>0&&opaque(c.refreshToken)&&opaque(c.grantId)&&c.grantId!.length<=512;
 if(!legacy&&!renewable)bad();
 if(c.site!==null&&typeof c.site!=='string'||c.encryption_identity!==null&&typeof c.encryption_identity!=='string'||c.accountInitial!==null&&typeof c.accountInitial!=='string'||typeof c.authRequired!=='boolean')bad();
 return Object.freeze({...c});
}
/** Decode bounded stored JSON. expectedServer must already have passed the caller's origin approval. */
export function decodeStoredCredential(bytes:Uint8Array,expectedScope:string,expectedServer:string):StoredCredential {
 try {
 const raw=decodeStrictJson(bytes,32768);if(!obj(raw)||typeof raw.server!=='string'||typeof raw.scope!=='string'||typeof raw.token!=='string'||typeof raw.generation!=='string')bad(); const v=raw as Record<string,unknown>;
 const opt=(k:string)=>optional(v,k);
 const n=(k:string):number|null=>{const x=opt(k);return x===null||x===undefined?null:typeof x==='number'&&Number.isSafeInteger(x)&&x>=0?x:bad()};
 const str=(k:string):string|null=>{const x=opt(k);return x===null||x===undefined?null:typeof x==='string'?x:bad()};
 return validate({server:v.server as string,scope:v.scope as string,token:v.token as string,generation:v.generation as string,site:str('site'),refreshToken:str('refreshToken'),expiresAt:n('expiresAt'),refreshExpiresAt:n('refreshExpiresAt'),grantId:str('grantId'),authRequired:v.authRequired===undefined?false:v.authRequired as boolean,encryption_identity:str('encryption_identity'),accountInitial:str('accountInitial')},expectedScope,expectedServer);
 } catch { return bad(); }
}
/** Pure merge of a renewal reply. `issuedAt` is the original request issue time. */
export function mergeRenewalReply(current:StoredCredential,reply:unknown,issuedAt:number):StoredCredential {
 try {
 if(!Number.isSafeInteger(issuedAt)||issuedAt<0||!obj(reply)||reply.scope!==current.scope)bad(); const r=reply as Record<string,unknown>;
 const access=r.accessToken,refresh=r.refreshToken,grant=r.grantId,life=r.expiresIn,refreshLife=r.refreshExpiresIn;
 if(!opaque(access)||!opaque(refresh)||!opaque(grant,512)||grant!==current.grantId||!Number.isSafeInteger(life)||(life as number)<=0||!Number.isSafeInteger(refreshLife)||(refreshLife as number)<=0)bad();
 const expiresAt=issuedAt+(life as number),refreshExpiresAt=issuedAt+(refreshLife as number);if(!Number.isSafeInteger(expiresAt)||!Number.isSafeInteger(refreshExpiresAt))bad();
 const account=r.accountInitial;
 const accountInitial=typeof account==='string'&&[...account].length>=1&&[...account].length<=3&&/^([\p{L}\p{N}]+)$/u.test(account)?account:null;
 return validate({...current,token:access as string,refreshToken:refresh as string,expiresAt,refreshExpiresAt,accountInitial,authRequired:false},current.scope,current.server);
 } catch { return bad(); }
}
