// Aggregate-only, fail-open service activity. No request/body/token/content is
// retained. A primary success is never rolled back or awaited on these writes.
import type {Env} from '../types.ts';
import {DAY,stmt} from '../common.ts';
const HOUR=3600000;
export async function admitDailyActivity(env:Env,account:string,occurredAt:number) {
 if(!/^[a-f0-9]{64}$/.test(account)||!Number.isSafeInteger(occurredAt)||occurredAt<0)return;
 try {
  // One statement + metric-local trigger atomically deduplicates and increments.
  // The existing consent table is NOT populated by default eligibility. A saved
  // refusal/tombstone with no current affirmative row always excludes admission.
  await stmt(env,`INSERT OR IGNORE INTO aggregate_dau_dedup(day_utc,account_id,expires_at_ms)
   SELECT date(?/1000,'unixepoch'),a.id,? FROM accounts a CROSS JOIN aggregate_dau_state s
   WHERE a.id=? AND s.id=1 AND ?>=s.collection_started_at_ms
   AND (SELECT enabled FROM analytics_control WHERE id=1)=1
   AND ?<=CAST(unixepoch('subsec')*1000 AS INTEGER)
   AND ?>CAST(unixepoch('subsec')*1000 AS INTEGER)
   AND lower(a.email) NOT LIKE '%.invalid'
   AND NOT EXISTS(SELECT 1 FROM aggregate_dau_exclusions WHERE account_id=a.id)
   AND NOT EXISTS(SELECT 1 FROM analytics_choices WHERE account_id=a.id AND aggregate_objected=1)
   AND (NOT EXISTS(SELECT 1 FROM analytics_choices WHERE account_id=a.id)
    OR EXISTS(SELECT 1 FROM analytics_consent WHERE account_id=a.id AND consented_at<=?))
   AND ((SELECT count(*) FROM aggregate_dau_dedup)<10000
    OR EXISTS(SELECT 1 FROM aggregate_dau_dedup WHERE day_utc=date(?/1000,'unixepoch') AND account_id=a.id))`,
   occurredAt,(Math.floor(occurredAt/DAY)+1)*DAY+HOUR,account,occurredAt,occurredAt,
   (Math.floor(occurredAt/DAY)+1)*DAY+HOUR,occurredAt,occurredAt).run();
 }catch{
  // Missing/failed schema, cap or crash may undercount. Never log provider/request
  // data or claim full coverage. The view is permanently labelled a lower bound.
 }
}
export function queueDailyActivity(env:Env,context:ExecutionContext|undefined,request:Request,account:string,occurredAt:number) {
 if(!context||request.headers.get('Sec-GPC')==='1'||request.headers.get('DNT')==='1')return;
 try {context.waitUntil(admitDailyActivity(env,account,occurredAt));}catch{/* Primary service remains available. */}
}
export async function cleanupDailyActivity(env:Env) {
 try {
  await env.DB.batch([
   // Erasure/refusal/exclusion is reconciled before forgetting attribution. No
   // triggers are attached to primary product or existing consent-table writes.
   stmt(env,`DELETE FROM aggregate_dau_dedup WHERE expires_at_ms<=CAST(unixepoch('subsec')*1000 AS INTEGER)
    OR NOT EXISTS(SELECT 1 FROM accounts WHERE id=aggregate_dau_dedup.account_id)
    OR EXISTS(SELECT 1 FROM aggregate_dau_exclusions WHERE account_id=aggregate_dau_dedup.account_id)
    OR EXISTS(SELECT 1 FROM analytics_choices WHERE account_id=aggregate_dau_dedup.account_id AND aggregate_objected=1)
    OR (EXISTS(SELECT 1 FROM analytics_choices WHERE account_id=aggregate_dau_dedup.account_id)
     AND NOT EXISTS(SELECT 1 FROM analytics_consent WHERE account_id=aggregate_dau_dedup.account_id))`),
   stmt(env,"DELETE FROM aggregate_dau_days WHERE day_utc<date('now','-29 days')"),
   // Retain pre-registered fixture exclusions before their account exists.
   stmt(env,"DELETE FROM aggregate_dau_exclusions WHERE expires_at_ms<=CAST(unixepoch('subsec')*1000 AS INTEGER) AND NOT EXISTS(SELECT 1 FROM accounts WHERE id=aggregate_dau_exclusions.account_id)"),
   // A coarse global maintenance watermark, NEVER the time of an individual's
   // last action, and not a claim that all primary successes were observed.
   stmt(env,"UPDATE aggregate_dau_state SET as_of_ms=MAX(as_of_ms,CAST(unixepoch('subsec')/900 AS INTEGER)*900000) WHERE id=1")
  ]);
 }catch{/* No primary/individual-collector effect; failed maintenance stays stale. */}
}
