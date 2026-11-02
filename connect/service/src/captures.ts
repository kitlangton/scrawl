import type {Env, CaptureRow, FileRow, ManifestFile, AttemptSnapshot} from './types.ts';
import {DAY,hex,json,fail,stmt,body,bearer,usage,isRecord} from './common.ts';
import {checkedHeads,getObject,uploadAttempt,settledAttempt,deleteAttempts} from './storage.ts';
const live="status!='deleted' AND expires_at>? AND (status='ready' OR pending_until>?)";
export function validPath(path: unknown): path is string {
 return typeof path==='string'&&path.length<=128&&/^(apparent\.json|summary\.md|contact-sheet(?:-full)?\.png|audio\.wav|recording\.(mp4|mov)|finish-trace\.json|timeline\.json|frames(?:-full)?\/frame-[0-9]+\.png)$/.test(path);
}
function manifest(input: unknown) {
 if(!Array.isArray(input)||input.length<1||input.length>512)fail('invalid_manifest');
 const seen=new Set();let bytes=0;
 const files=input.map((f: unknown)=>{
  if(!isRecord(f)||!validPath(f.path))fail('invalid_path');
  if(seen.has(f.path)||typeof f.size!=='number'||!Number.isSafeInteger(f.size)||f.size<0||typeof f.sha256!=='string'||! /^[a-f0-9]{64}$/.test(f.sha256))fail('invalid_manifest');
  if(f.size>90000000)fail('file_too_large');
  seen.add(f.path);bytes+=f.size;return {path:f.path,size:f.size,sha256:f.sha256};
 }).sort((a,b)=>a.path.localeCompare(b.path));
 if(bytes>500000000)fail('quota_exceeded',409);
 return {files,bytes,text:JSON.stringify(files)};
}
async function capture(env: Env,id: string,account: string) {
 const row=await stmt(env,`SELECT * FROM captures WHERE id=? AND account_id=? AND ${live}`,id,account,Date.now(),Date.now()).first<CaptureRow>();
 if(!row)fail('not_found',404);return row;
}
const info = (c: CaptureRow) => ({id:c.id,status:c.status,expiresAt:new Date(c.expires_at).toISOString()});
export async function captureRoutes(request: Request,env: Env,url: URL) {
 if(url.pathname==='/v1/account'&&request.method==='GET') {const a=await bearer(request,env);return json({beta:a.beta,quotaBytes:500000000,usedBytes:await usage(env,a.account_id)});}
 const match=/^\/v1\/captures\/([a-f0-9]{32})(?:\/(complete|files\/(.+)))?$/.exec(url.pathname);if(!match)return null;
 const [,id,action,encoded]=match,method=request.method;
 const a=await bearer(request,env,method!=='GET');
 if(!action&&method==='POST') {
  // Validate the complete manifest before any database write/eviction.
  const m=manifest((await body(request)).files);
  let existing=await stmt(env,'SELECT * FROM captures WHERE id=?',id).first<CaptureRow>();
  if(!existing) {
   const now=Date.now();
   try {
    // The insert trigger tombstones the oldest quota overflow in this same
    // transaction. A duplicate ID cannot run it, including concurrent retries.
    await env.DB.batch([
     stmt(env,'INSERT INTO captures(id,account_id,manifest,bytes,expires_at,pending_until) VALUES(?,?,?,?,?,?)',id,a.account_id,m.text,m.bytes,now+7*DAY,now+DAY),
     // One set insertion, not one SQL statement per file. JSON keeps the full
     // 512-file manifest below D1's 100-bound-parameter limit as well.
     stmt(env,"INSERT INTO files(capture_id,path,size,sha256) SELECT ?,json_extract(value,'$.path'),json_extract(value,'$.size'),json_extract(value,'$.sha256') FROM json_each(?)",id,m.text)
    ]);
   }catch(error){
    existing=await stmt(env,'SELECT * FROM captures WHERE id=?',id).first<CaptureRow>();
    if(!existing)throw error;
   }
  }
  const c=await capture(env,id,a.account_id);
  if(c.manifest!==m.text)fail('manifest_conflict',409);
  return json(info(c));
 }
 if(!action&&method==='DELETE') {
  const c=await stmt(env,'SELECT account_id FROM captures WHERE id=?',id).first();if(!c||c.account_id!==a.account_id)fail('not_found',404);
  await stmt(env,"UPDATE captures SET status='deleted' WHERE id=? AND account_id=?",id,a.account_id).run();
  // Tombstone is authoritative; cron retries any failed or late object removal.
  return json({ok:true});
 }
 const c=await capture(env,id,a.account_id);
 if(!action&&method==='GET')return json({...info(c),files:JSON.parse(c.manifest)});
 if(action==='complete'&&method==='POST') {
  if(c.status==='ready')return json(info(c));
  const {results:files}=await stmt(env,'SELECT * FROM files WHERE capture_id=?',id).all<FileRow>();
  await checkedHeads(env,files,verified);
  await stmt(env,`UPDATE captures SET status='ready' WHERE id=? AND account_id=? AND status='pending' AND ${live}`,id,a.account_id,Date.now(),Date.now()).run();
  const ready=await capture(env,id,a.account_id);if(ready.status!=='ready')fail('not_ready',409);return json(info(ready));
 }
 if(encoded) {
  let path;try {path=encoded.split('/').map(segment=>{const decoded=decodeURIComponent(segment);if(decoded.includes('/')||decoded.includes('\\'))fail('invalid_path');return decoded;}).join('/');}catch{fail('invalid_path');}
  if(!validPath(path))fail('invalid_path');
  const f=await stmt(env,'SELECT * FROM files WHERE capture_id=? AND path=?',id,path).first<FileRow>();if(!f)fail('not_found',404);
  if(method==='GET') {
   if(c.status!=='ready')fail('not_ready',409);
   const object=await getObject(env,f.object_key);if(!object)fail('not_found',404);
   await capture(env,id,a.account_id);
   return new Response(object.body,{headers:{'Content-Type':mime(path),'Content-Length':String(f.size),'Content-Disposition':'attachment; filename="'+path.split('/').at(-1)+'"','X-Content-Type-Options':'nosniff'}});
  }
  if(method==='PUT') {
   const attempt=await uploadAttempt(env,id,a.account_id,f),{key}=attempt;
   let terminal=0;
   try {
    // R2 verifies SHA256 natively, including retries. Each attempt uses a new key;
    // a retry can never replace the original object, even before validation ends.
    const fixed=new FixedLengthStream(f.size),writer=fixed.writable.getWriter();
    const reader=request.body?.getReader();
    const cancel=(e: unknown)=>{void writer.abort(e).catch(()=>{});if(reader)void reader.cancel(e).catch(()=>{});void fixed.readable.cancel(e).catch(()=>{});};
    const write=receive(reader,f,writer).catch(e=>{cancel(e);throw e;});
    const put=Promise.resolve().then(()=>attempt.put(fixed.readable)).catch(e=>{cancel(e);throw e;});
    // allSettled keeps both promises observed when either stream end fails.
    const results=await Promise.allSettled([write,put]);
    await settledAttempt(env,key);terminal=1;
    if(results[0].status==='rejected')throw results[0].reason;if(results[1].status==='rejected'){
     // Provider text is used only for classification, never returned or logged.
     if(/(?:sha-?256|checksum|digest).*(?:match|invalid)|(?:match|invalid).*(?:sha-?256|checksum|digest)/i.test(String(results[1].reason?.message)))fail('hash_mismatch');
     throw results[1].reason;
    }
    if(!verified(results[1].value,f))fail('hash_mismatch');
    if(f.object_key||c.status==='ready') {await capture(env,id,a.account_id);await deleteAttempts(env,[{key,terminal}]);return json({ok:true});}
    const changed=await stmt(env,`UPDATE files SET object_key=? WHERE capture_id=? AND path=? AND object_key IS NULL AND EXISTS(SELECT 1 FROM captures WHERE id=? AND status='pending' AND ${live})`,key,id,path,id,Date.now(),Date.now()).run();
    if(!changed.meta.changes){await deleteAttempts(env,[{key,terminal}]);await capture(env,id,a.account_id);}
    return json({ok:true});
   }catch(e){await deleteAttempts(env,[{key,terminal}]).catch(()=>{});throw e;}
  }
 }
 fail('not_found',404);
}
function verified(object: R2Object | null,file: ManifestFile) {return object?.size===file.size&&object.checksums?.sha256&&hex(new Uint8Array(object.checksums.sha256))===file.sha256;}
async function receive(reader: ReadableStreamDefaultReader<Uint8Array> | undefined,file: ManifestFile,writer: WritableStreamDefaultWriter<Uint8Array>) {
 let count=0;
 try{
  if(reader)for(;;){const {done,value}=await reader.read();if(done)break;count+=value.length;if(count>file.size)fail('size_mismatch');await writer.write(value);}
  if(count!==file.size)fail('size_mismatch');
  await writer.close();
 }catch(e){void writer.abort(e).catch(()=>{});if(reader)void reader.cancel(e).catch(()=>{});throw e;}finally{reader?.releaseLock();}
}
function mime(path: string){if(path.endsWith('.png'))return 'image/png';if(path.endsWith('.mp4'))return 'video/mp4';if(path.endsWith('.mov'))return 'video/quicktime';if(path.endsWith('.wav'))return 'audio/wav';if(path.endsWith('.json'))return 'application/json';return 'text/plain; charset=utf-8';}
export async function cleanup(env: Env) {
 const now=Date.now();
 await stmt(env,"UPDATE captures SET status='deleted' WHERE status!='deleted' AND (expires_at<=? OR (status='pending' AND pending_until<=?))",now,now).run();
 // Keep attempt records: a PUT already in flight can finish after this sweep.
 // Repeated deletion of tombstoned keys makes that orphan harmless and reclaimable.
 const {results}=await stmt(env,"SELECT a.key,a.terminal FROM attempts a JOIN captures c ON c.id=a.capture_id LEFT JOIN files f ON f.object_key=a.key WHERE c.status='deleted' OR (f.object_key IS NULL AND a.created_at<?) ORDER BY a.cleaned_at,a.key LIMIT 1000",now-DAY).all<AttemptSnapshot>();
 for(let i=0;i<results.length;i+=100){
  const page=results.slice(i,i+100);
  await deleteAttempts(env,page,now);
 }
 const iso=new Date(now).toISOString();
 await env.DB.batch([
  stmt(env,'DELETE FROM auth_session WHERE expiresAt<=?',iso),
  stmt(env,'DELETE FROM oauthAccessToken WHERE expiresAt<=?',iso),
  stmt(env,'DELETE FROM auth_verification WHERE expiresAt<=?',iso),
  stmt(env,'DELETE FROM auth_device WHERE expiresAt<=?',iso),
  stmt(env,'DELETE FROM rate_limits WHERE window<?',Math.floor(now/3600000)-1),
  stmt(env,'UPDATE auth_grants SET response=NULL WHERE response IS NOT NULL AND (expires_at<=? OR revoked_at IS NOT NULL)',now),
  stmt(env,'UPDATE oauthRefreshToken SET rotationReplayResponse=NULL WHERE rotationReplayExpiresAt<=? AND rotationReplayResponse IS NOT NULL',iso),
  // Keep live independently approved devices when the short device code expires.
  // Terminal per-grant clients cascade only their own provider credential rows.
  stmt(env,'DELETE FROM oauthClient WHERE clientId IN (SELECT g.client_id FROM auth_grants g WHERE g.expires_at<=? AND (g.revoked_at IS NOT NULL OR (NOT EXISTS(SELECT 1 FROM oauthRefreshToken r WHERE r.clientId=g.client_id AND r.expiresAt>?) AND NOT EXISTS(SELECT 1 FROM oauthAccessToken t WHERE t.clientId=g.client_id AND t.expiresAt>?))))',now,iso,iso),
 ]);
}
