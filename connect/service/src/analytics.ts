// Explicit server-side product events only. Never pass requests, exceptions,
// credentials, paths, manifests, recording content or arbitrary properties here.
import type {Env} from './types.ts';
import {DAY,fail,hash,random,stmt} from './common.ts';
interface Activity {
 id: string; account_id: string; consent_id: string; event: string; occurred_at: number;
 bytes: number; captures: number; pending_bytes: number; physical_bytes: number;
}
const hosts=new Set(['https://eu.i.posthog.com']);
const events=new Set(['connect_account_created','connect_device_connected','connect_upload_completed','connect_share_created','connect_storage_snapshot']);
const actor=(event: string)=>['connect_upload_completed','connect_share_created'].includes(event)?'device':event==='connect_storage_snapshot'?'system':'browser';
const host=(env:Env)=>env.POSTHOG_HOST??'https://eu.i.posthog.com';
const environment=(env:Env)=>env.POSTHOG_ENVIRONMENT??'production';
const configured=(env:Env)=>(env.POSTHOG_ENABLED===undefined||env.POSTHOG_ENABLED==='true')&&!!env.POSTHOG_PROJECT_TOKEN&&hosts.has(host(env))&&['production','synthetic','development'].includes(environment(env));
const exclusions=(env:Env)=>new Set((env.POSTHOG_EXCLUDED_ACCOUNT_IDS||'').split(',').map(s=>s.trim()).filter(Boolean));
export const analyticsNoticeVersion='product-analytics-v1';
export async function analyticsChoice(env:Env,account:string) {
 try {
  const state=await stmt(env,`SELECT enabled,
   EXISTS(SELECT 1 FROM analytics_consent WHERE account_id=? AND consented_at<=?) AS consented,
   (SELECT revision FROM analytics_choices WHERE account_id=?) AS revision
   FROM analytics_control WHERE id=1`,account,Date.now(),account).first<{enabled:number;consented:number;revision:string|null}>();
  if(!state)fail('analytics_unavailable',503);
  return {available:configured(env)&&!exclusions(env).has(account)&&state.enabled===1,consented:!!state.consented,revision:state.revision,noticeVersion:analyticsNoticeVersion};
 }catch{fail('analytics_unavailable',503);}
}
export async function setAnalyticsChoice(env:Env,account:string,consented:boolean,revision:string|null) {
 if(consented&&!(await analyticsChoice(env,account)).available)fail('analytics_unavailable',503);
 const next=random(),now=Date.now();
 // Withdrawal always wins. A new opt-in must own the revision it observed;
 // an old in-flight true cannot overwrite an intervening false or operator erase.
 const condition=consented?'(? IS NULL AND NOT EXISTS(SELECT 1 FROM analytics_choices WHERE account_id=?)) OR EXISTS(SELECT 1 FROM analytics_choices WHERE account_id=? AND revision=?)':'1';
 let results;
 try {
  results=await env.DB.batch([
   stmt(env,`INSERT INTO analytics_choices(account_id,revision,decided_at) SELECT ?,?,? WHERE ${condition}
    ON CONFLICT(account_id) DO UPDATE SET revision=excluded.revision,decided_at=excluded.decided_at`,account,next,now,...consented?[revision,account,account,revision]:[]),
   stmt(env,`DELETE FROM analytics_consent WHERE account_id=? AND ?=0 AND EXISTS
    (SELECT 1 FROM analytics_choices WHERE account_id=? AND revision=?)`,account,consented?1:0,account,next),
   stmt(env,`INSERT INTO analytics_consent(account_id,notice_version,consented_at) SELECT ?,?,?
    WHERE ?=1 AND EXISTS(SELECT 1 FROM analytics_choices WHERE account_id=? AND revision=?)
    AND NOT EXISTS(SELECT 1 FROM analytics_consent WHERE account_id=?)`,account,analyticsNoticeVersion,now,consented?1:0,account,next,account)
  ]);
 }catch{fail('analytics_unavailable',503);}
 if(!results[0].meta.changes)fail('consent_changed',409);
 return analyticsChoice(env,account);
}
export async function analyticsIdentity(env:Env,account:string) {
 if(!configured(env)||exclusions(env).has(account))return null;
 try {
  const control=await stmt(env,`SELECT enabled FROM analytics_control WHERE id=1 AND EXISTS
   (SELECT 1 FROM analytics_consent WHERE account_id=? AND consented_at<=?)`,account,Date.now()).first<{enabled:number}>();
  return control?.enabled?'account:'+account:null;
 }catch{return null;}
}
export async function flushAnalytics(env: Env) {
 if(!configured(env))return;
 try {
  const control=await stmt(env,'SELECT enabled,dropped FROM analytics_control WHERE id=1').first<{enabled:number;dropped:number}>();
  if(!control?.enabled)return;
  const now=Date.now(),day=Math.floor(now/DAY);
  // One UTC-day snapshot per account. The checkpoint and rows commit together,
  // including across cron retries. Current storage is NOT cumulative uploads.
  await env.DB.batch([
   // Refuse legacy, withdrawn or pre-consent events even if rows were restored.
   stmt(env,`DELETE FROM analytics_outbox WHERE NOT EXISTS (
    SELECT 1 FROM analytics_consent c WHERE c.account_id=analytics_outbox.account_id
     AND c.consent_id=analytics_outbox.consent_id AND c.consented_at<=analytics_outbox.occurred_at AND c.consented_at<=?)`,now),
   stmt(env,`INSERT OR IGNORE INTO analytics_outbox(id,account_id,event,occurred_at,bytes,captures,pending_bytes,physical_bytes,consent_id)
    SELECT 'storage:'||a.id||':'||?,a.id,'connect_storage_snapshot',?,
     COALESCE((SELECT sum(bytes) FROM captures WHERE account_id=a.id AND status='ready' AND expires_at>?),0),
     (SELECT count(*) FROM captures WHERE account_id=a.id AND status='ready' AND expires_at>?),
     COALESCE((SELECT sum(bytes) FROM captures WHERE account_id=a.id AND status='pending' AND expires_at>? AND pending_until>?),0),
     COALESCE((SELECT sum(t.bytes) FROM attempts t JOIN captures c ON c.id=t.capture_id WHERE c.account_id=a.id AND t.released=0),0),consent.consent_id
    FROM accounts a JOIN analytics_consent consent ON consent.account_id=a.id
    WHERE consent.consented_at<=? AND (SELECT snapshot_day FROM analytics_control WHERE id=1)<?
    LIMIT MAX(0,10000-(SELECT count(*) FROM analytics_outbox))`,day,now,now,now,now,now,now,day),
   stmt(env,`UPDATE analytics_control SET
    dropped=dropped+CASE WHEN snapshot_day<? THEN (SELECT count(*) FROM accounts a JOIN analytics_consent consent ON consent.account_id=a.id
     WHERE consent.consented_at<=? AND NOT EXISTS(SELECT 1 FROM analytics_outbox o WHERE o.id='storage:'||a.id||':'||?)) ELSE 0 END,
    snapshot_day=? WHERE id=1`,day,now,day,day),
   stmt(env,`UPDATE analytics_control SET dropped=dropped+(SELECT count(*) FROM analytics_outbox WHERE occurred_at<?) WHERE id=1`,now-90*DAY),
   stmt(env,'DELETE FROM analytics_outbox WHERE occurred_at<?',now-90*DAY)
  ]);
  const {results}=await stmt(env,`SELECT o.* FROM analytics_outbox o JOIN analytics_consent c ON c.account_id=o.account_id
   WHERE c.consent_id=o.consent_id AND c.consented_at<=o.occurred_at AND c.consented_at<=? ORDER BY o.occurred_at,o.id LIMIT 100`,Date.now()).all<Activity>();
  if(!results.length)return;
  const excluded=exclusions(env);
  const safe=results.filter(row=>!excluded.has(row.account_id)&&events.has(row.event)&&/^[a-f0-9]{64}$/.test(row.account_id)&&[row.bytes,row.captures,row.pending_bytes,row.physical_bytes,row.occurred_at].every(n=>Number.isSafeInteger(n)&&n>=0));
  const batch=await Promise.all(safe.map(async row=>{
   const digest=await hash('apparent-analytics:'+row.consent_id+':'+row.id);
   const uuid=`${digest.slice(0,8)}-${digest.slice(8,12)}-5${digest.slice(13,16)}-a${digest.slice(17,20)}-${digest.slice(20,32)}`;
   return {uuid,event:row.event,timestamp:new Date(row.occurred_at).toISOString(),properties:{
    distinct_id:'account:'+row.account_id,environment:environment(env),actor:actor(row.event),
    outcome:'success',$geoip_disable:true,$ip:null,
    ...(row.event==='connect_upload_completed'?{recording_bytes:row.bytes}:{}),
    ...(row.event==='connect_storage_snapshot'?{retained_bytes:row.bytes,retained_recordings:row.captures,pending_reserved_bytes:row.pending_bytes,physical_reserved_bytes:row.physical_bytes}:{}),
    collection_dropped:control.dropped
   }};
  }));
  if(batch.length){
   // This final database authorization is the export admission point. A batch
   // admitted here may finish after withdrawal, even before this read returns.
   // Withdrawal before admission purges rows; later permission cannot revive them.
   const eligible=await stmt(env,`SELECT o.id,o.consent_id FROM analytics_outbox o JOIN analytics_consent c ON c.account_id=o.account_id
    WHERE o.id IN (SELECT value FROM json_each(?)) AND c.consent_id=o.consent_id AND c.consented_at<=o.occurred_at AND c.consented_at<=?
     AND (SELECT enabled FROM analytics_control WHERE id=1)=1`,JSON.stringify(safe.map(row=>row.id)),Date.now()).all<{id:string;consent_id:string}>();
   const allowed=new Map(eligible.results.map(row=>[row.id,row.consent_id]));
   const current=batch.filter((_,i)=>allowed.get(safe[i].id)===safe[i].consent_id);
   if(!current.length)return;
   const response=await fetch(host(env)+'/batch/',{method:'POST',redirect:'manual',headers:{'Content-Type':'application/json'},body:JSON.stringify({api_key:env.POSTHOG_PROJECT_TOKEN,batch:current}),signal:AbortSignal.timeout(5000)});
   if(!response.ok)return;
   const result=await response.json() as {status?:number|string};if(result.status!==1&&result.status!=='Ok')return;
  }
  // A failed send/ambiguous acknowledgement leaves rows for retry with the same
  // UUID and timestamp. Exclusions never go to the analytics provider.
  await stmt(env,`DELETE FROM analytics_outbox WHERE EXISTS (
   SELECT 1 FROM json_each(?) sent WHERE json_extract(sent.value,'$.id')=analytics_outbox.id
    AND json_extract(sent.value,'$.consent_id')=analytics_outbox.consent_id
  )`,JSON.stringify(results.map(({id,consent_id})=>({id,consent_id})))).run();
 }catch{
  // External analytics outages/config/migration gaps never fail product work.
  // Do not log the exception: provider text may include the submitted payload.
 }
}
