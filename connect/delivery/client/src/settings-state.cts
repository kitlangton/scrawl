import type { LocalState, Snapshot } from './settings-controller.cjs';

type Grant = {
  server: string; scope: 'upload'|'read'; authRequired: boolean; hasRefresh: boolean;
  expiresAt: number|null; refreshExpiresAt: number|null; accountInitial: unknown;
};
type Facts = {
  cloud: { server: string; enabled: boolean; uploadConsent: boolean };
  hostedSync: boolean; revocationPending: boolean; upload: Grant|null; read: Grant|null;
  sharedLinksUrl: string|null;
};
const DEFAULT_SERVER = 'https://hosted.tryapparent.com';
const localFailure = (): LocalState => ({snapshot:null,error:'local-state',sharedLinksUrl:null});
const object = (v:unknown):v is Record<string,unknown> => v !== null && typeof v === 'object' && !Array.isArray(v);
const expires = (v:unknown):v is number|null => v === null || Number.isSafeInteger(v) && (v as number) >= 0;

function serverUrl(raw:string):string|null {
  const value=raw.replace(/\/+$/,'');
  if(value.length>2048 || /[^\x00-\x7f]/.test(value) || /[\x00-\x20\x7f"'\\?#@]/.test(value)) return null;
  const match=/^(https?):\/\/([^/]+)(?:\/(connect))?$/.exec(value);
  if(!match) return null;
  const [,scheme,authority]=match;
  if(!authority || (authority.match(/:/g)?.length??0)>1) return null;
  const [host,port]=authority.split(':');
  if(!host || !/^[a-zA-Z0-9.-]+$/.test(host)) return null;
  if(port!==undefined && (!/^\d+$/.test(port) || Number(port)<1 || Number(port)>65535)) return null;
  if(scheme==='http' && host!=='127.0.0.1' && host!=='localhost') return null;
  return value;
}

function safeLinks(value:string|null):string|null {
  if(value===null) return null;
  if(value.length===0 || Buffer.byteLength(value)>4096 || /[\x00-\x20\x7f%#\\"']/.test(value)) return null;
  try {
    const url=new URL(value);
    if(url.username || url.password || url.hash || url.search ||
      !(url.protocol==='https:' || url.protocol==='http:' && ['localhost','127.0.0.1'].includes(url.hostname)) ||
      !['/connect/shares','/hosted/shares'].includes(url.pathname)) return null;
    return value;
  } catch { return null; }
}

function parseGrant(value:unknown, expected:'upload'|'read'):Grant|null {
  if(value===null) return null;
  if(!object(value) || typeof value.server!=='string' || serverUrl(value.server)!==value.server || value.scope!==expected ||
    typeof value.authRequired!=='boolean' || typeof value.hasRefresh!=='boolean' ||
    !expires(value.expiresAt) || !expires(value.refreshExpiresAt)) throw new Error('malformed facts');
  if (value.hasRefresh ? !(typeof value.expiresAt === 'number' && value.expiresAt > 0 &&
      typeof value.refreshExpiresAt === 'number' && value.refreshExpiresAt > 0) : value.refreshExpiresAt !== null) throw new Error('malformed facts');
  return {server:serverUrl(value.server)!,scope:expected,authRequired:value.authRequired,
    hasRefresh:value.hasRefresh,expiresAt:value.expiresAt,refreshExpiresAt:value.refreshExpiresAt,
    accountInitial:value.accountInitial};
}

function parseFacts(value:unknown):Facts {
  if(!object(value) || !object(value.cloud) || typeof value.cloud.server!=='string' ||
    typeof value.cloud.enabled!=='boolean' || typeof value.cloud.uploadConsent!=='boolean' ||
    typeof value.hostedSync!=='boolean' || typeof value.revocationPending!=='boolean' ||
    !(value.sharedLinksUrl===null || typeof value.sharedLinksUrl==='string')) throw new Error('malformed facts');
  const upload=parseGrant(value.upload,'upload'), read=parseGrant(value.read,'read');
  return {cloud:{server:value.cloud.server,enabled:value.cloud.enabled,uploadConsent:value.cloud.uploadConsent},
    hostedSync:value.hostedSync,revocationPending:value.revocationPending,upload,read,
    sharedLinksUrl:value.sharedLinksUrl};
}

function usable(grant:Grant,now:number):boolean {
  return !grant.authRequired && (grant.refreshExpiresAt===null || grant.refreshExpiresAt>now) &&
    (grant.hasRefresh || grant.expiresAt===null || grant.expiresAt>now);
}
function current(grant:Grant,now:number):boolean {
  return usable(grant,now) && (grant.expiresAt===null || grant.expiresAt>now);
}
function initial(value:unknown):string|null {
  if(typeof value!=='string') return null;
  const chars=[...value];
  return chars.length>=1 && chars.length<=3 && chars.every(char=>/[\p{L}\p{N}]/u.test(char)) ? value : null;
}

/** Projects native-private account facts into the secret-free Settings state. */
export function deriveLocalState(value:unknown,nowSeconds=Math.floor(Date.now()/1000)):LocalState {
  let facts:Facts;
  try {
    if(!Number.isSafeInteger(nowSeconds) || nowSeconds<0) return localFailure();
    facts=parseFacts(value);
  } catch { return localFailure(); }
  if(facts.revocationPending) return localFailure();
  const configured=facts.cloud.server==='' ? null : serverUrl(facts.cloud.server);
  if(facts.cloud.server!=='' && configured===null) return {snapshot:null,error:'invalid-server',sharedLinksUrl:null};
  const server=configured;
  const origin=server??DEFAULT_SERVER;
  const upload=facts.upload?.server===origin ? facts.upload : null;
  const read=facts.read?.server===origin ? facts.read : null;
  const preferred=upload??read;
  const selectedUsable=preferred!==null && usable(preferred,nowSeconds);
  const selectedCurrent=preferred!==null && current(preferred,nowSeconds);
  const account=(upload??read);
  const accountInitial=account && usable(account,nowSeconds) ? initial(account.accountInitial) : null;
  const snapshot:Snapshot={server,connected:!!(upload&&current(upload,nowSeconds)||read&&current(read,nowSeconds)),
    uploadsEnabled:!!server && facts.cloud.enabled && facts.cloud.uploadConsent && facts.hostedSync && !!upload&&current(upload,nowSeconds),
    reauthorizationRequired:!!preferred&&!selectedUsable,
    renewalRequired:!!preferred&&selectedUsable&&!selectedCurrent,
    accountInitial};
  return {snapshot,error:null,sharedLinksUrl:safeLinks(facts.sharedLinksUrl)};
}
