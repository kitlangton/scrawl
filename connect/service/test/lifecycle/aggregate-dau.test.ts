import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {DatabaseSync} from 'node:sqlite';
import {fixture,id,file} from '../helpers.mjs';
import {admitDailyActivity,cleanupDailyActivity} from '../../src/reporting/aggregate-dau.ts';
import {setAnalyticsChoice} from '../../src/analytics.ts';
import type {Env} from '../../src/types.ts';
const DAY=86400000,HOUR=3600000;
async function setup(aggregate=true){const f=await fixture({SHARING_ENABLED:'true',POSTHOG_ENABLED:'false'});for(const n of ['0003','0006','0007','0008',...aggregate?['0009','0010']:[]])await f.db.exec((await readFile('migrations/'+n+'.sql','utf8')).replaceAll('\n',' '));return f;}
async function upload(f,a,n,headers={}){const path='/v1/captures/'+id(n);assert.equal((await f.call(path,{token:a.token,method:'POST',body:{files:[file()]}})).status,200);assert.equal((await f.call(path+'/files/summary.md',{token:a.token,method:'PUT',raw:'hello'})).status,200);assert.equal((await f.call(path+'/complete',{token:a.token,method:'POST',headers})).status,200);return path;}
async function observed(f,n){for(let i=0;i<100;i++){const rows=(await f.db.prepare('SELECT * FROM reporting_daily_active_accounts').all()).results;if(rows.reduce((sum,r)=>sum+r.active_accounts,0)===n)return rows;await new Promise(resolve=>setTimeout(resolve,10));}assert.fail('Actual Worker background aggregation did not converge within bounded local test wait');}
test('aggregate DAU real Worker: successes, shared account dedup, concurrency/retries, no signup or PostHog consent fabrication',async()=>{
 const f=await setup();try{
  const a=await f.identity('aggregate-a'),b=await f.identity('aggregate-b');const empty=(await f.db.prepare('SELECT * FROM reporting_daily_active_accounts').all()).results;assert.equal(empty.length,1);assert.equal(empty[0].active_accounts,null);assert.equal(empty[0].coverage,'unknown');
  const [pa,pb]=await Promise.all([upload(f,a,901),upload(f,b,902)]);
  await Promise.all([f.call(pa+'/share',{token:a.token,method:'POST'}),f.call(pa+'/share',{token:a.token,method:'POST'}),f.call(pb+'/complete',{token:b.token,method:'POST'})]);
  const rows=await observed(f,2);assert.equal(rows.length,1);assert.equal(rows[0].coverage,'observed_lower_bound');assert.deepEqual(Object.keys(rows[0]).sort(),['active_accounts','as_of_ms','collection_started_at_ms','coverage','day_utc']);
  assert.equal((await f.db.prepare('SELECT count(*) n FROM aggregate_dau_dedup').first()).n,2);
  for(const table of ['analytics_consent','analytics_choices','analytics_outbox'])assert.equal((await f.db.prepare('SELECT count(*) n FROM '+table).first()).n,0);
 }finally{await f.close();}
});
test('aggregate DAU real Worker: prior refusal, fixture exclusion and GPC/DNT cannot be bypassed by unflagged retries',async()=>{
 const f=await setup();try{
  const refused=await f.identity('refused'),excluded=await f.identity('excluded'),gpc=await f.identity('gpc'),dnt=await f.identity('dnt');
  assert.equal((await f.call('/v1/analytics/consent',{...refused,method:'POST',body:{consented:false,revision:null,noticeVersion:'product-analytics-v1'}})).status,200);
  await f.db.prepare('INSERT INTO aggregate_dau_exclusions(account_id) VALUES(?)').bind(excluded.id).run();
  for(const [n,a,headers] of [[903,refused,{}],[904,excluded,{}],[905,gpc,{'Sec-GPC':'1'}],[906,dnt,{DNT:'1'}]]){const path=await upload(f,a,n,headers);assert.equal((await f.call(path+'/complete',{token:a.token,method:'POST'})).status,200);}
  await(await f.mf.getWorker()).scheduled();assert.deepEqual((await f.db.prepare("SELECT * FROM reporting_daily_active_accounts WHERE coverage='observed_lower_bound'").all()).results,[]);
  const path='/v1/captures/'+id(905);assert.equal((await f.call(path+'/share',{token:gpc.token,method:'POST',headers:{'Sec-GPC':'1'}})).status,200);assert.equal((await f.call(path+'/share',{token:gpc.token,method:'POST'})).status,200);
  await(await f.mf.getWorker()).scheduled();assert.equal((await f.db.prepare('SELECT count(*) n FROM aggregate_dau_dedup').first()).n,0);
 }finally{await f.close();}
});
test('local workerd D1 upgrade preserves positive choices, refusal revisions, pending individual events and spent budgets',async()=>{
 const f=await setup(false);try{
  const a=await f.identity('upgrade-allowed'),b=await f.identity('upgrade-refused');
  const env={DB:f.db,POSTHOG_ENABLED:'true',POSTHOG_PROJECT_TOKEN:'synthetic-inert-no-transport-called'} as unknown as Env;
  await setAnalyticsChoice(env,a.id,true,null);await setAnalyticsChoice(env,b.id,false,null);
  await f.db.prepare("INSERT INTO analytics_outbox(id,account_id,event,occurred_at,consent_id) SELECT 'synthetic-pending',account_id,'connect_upload_completed',consented_at,consent_id FROM analytics_consent WHERE account_id=?").bind(a.id).run();
  await f.db.exec('UPDATE r2_budget SET a_used=17,b_used=23,physical_used=1234 WHERE id=1; UPDATE analytics_control SET enabled=0,dropped=7');
  const snapshot=async()=>Promise.all(['SELECT * FROM r2_budget','SELECT * FROM analytics_control','SELECT account_id,revision,decided_at FROM analytics_choices ORDER BY account_id','SELECT * FROM analytics_consent','SELECT * FROM analytics_outbox'].map(async sql=>(await f.db.prepare(sql).all()).results));
  const before=await snapshot();await f.db.exec((await readFile('migrations/0009.sql','utf8')).replaceAll('\n',' '));assert.deepEqual(await snapshot(),before);
  assert.equal((await f.db.prepare('SELECT aggregate_objected FROM analytics_choices WHERE account_id=?').bind(b.id).first()).aggregate_objected,1);
  assert.equal((await f.db.prepare('SELECT aggregate_objected FROM analytics_choices WHERE account_id=?').bind(a.id).first()).aggregate_objected,0);
 }finally{await f.close();}
});
test('local workerd D1: an aggregate objection survives later individual re-consent and failed maintenance rolls back',async()=>{
 const f=await setup();try{
  const a=await f.identity('separate-objection');
  const env={DB:f.db,POSTHOG_ENABLED:'true',POSTHOG_PROJECT_TOKEN:'synthetic-inert-no-transport-called'} as unknown as Env;
  const stopped=await setAnalyticsChoice(env,a.id,false,null);await setAnalyticsChoice(env,a.id,true,stopped.revision);
  assert.equal((await f.db.prepare('SELECT aggregate_objected FROM analytics_choices WHERE account_id=?').bind(a.id).first()).aggregate_objected,1);
  assert.equal((await f.db.prepare('SELECT count(*) n FROM analytics_consent WHERE account_id=?').bind(a.id).first()).n,1,'individual affirmative consent is still permitted, separately');
  await upload(f,a,908);await(await f.mf.getWorker()).scheduled();assert.equal((await f.db.prepare('SELECT count(*) n FROM aggregate_dau_dedup').first()).n,0);
  const b=await f.identity('maintenance-rollback');await upload(f,b,909);await observed(f,1);
  await f.db.exec("UPDATE aggregate_dau_dedup SET expires_at_ms=0; CREATE TRIGGER maintenance_failure BEFORE UPDATE ON aggregate_dau_state BEGIN SELECT RAISE(ABORT,'synthetic failure'); END");
  const before=await f.db.prepare('SELECT as_of_ms FROM aggregate_dau_state').first();await(await f.mf.getWorker()).scheduled();
  assert.deepEqual(await f.db.prepare('SELECT as_of_ms FROM aggregate_dau_state').first(),before);assert.equal((await f.db.prepare('SELECT count(*) n FROM aggregate_dau_dedup').first()).n,1,'real D1 batch rolled back preceding cleanup when watermark write failed');
 }finally{await f.close();}
});
test('aggregate failures never roll back real successful upload/share or mutate legacy preferences',async()=>{
 const f=await setup();try{const a=await f.identity('metric-failure');await f.db.exec('DROP TABLE aggregate_dau_days');const path=await upload(f,a,907);assert.equal((await f.call(path+'/share',{token:a.token,method:'POST'})).status,200);await(await f.mf.getWorker()).scheduled();assert.equal((await f.db.prepare('SELECT status FROM captures WHERE id=?').bind(id(907)).first()).status,'ready');assert.equal((await f.db.prepare('SELECT count(*) n FROM shares').first()).n,1);assert.equal((await f.db.prepare('SELECT count(*) n FROM aggregate_dau_dedup').first()).n,0,'metric statement/trigger rolls back atomically, not product mutation');assert.equal((await f.db.prepare('SELECT count(*) n FROM analytics_choices').first()).n,0);await assert.rejects(f.db.prepare('SELECT * FROM reporting_daily_active_accounts').all());}finally{await f.close();}
});
async function sqlite(bootstrap=true){
 const db=new DatabaseSync(':memory:');let now=Math.floor(Date.now()/DAY)*DAY-1;db.function('unixepoch',{varargs:true},(...args)=>args[0]==='subsec'?now/1000:Math.floor(now/1000));
 for(const n of ['0001','0002','0004','0005','0003','0006','0007','0008'])db.exec(await readFile('migrations/'+n+'.sql','utf8'));
 db.exec('UPDATE r2_budget SET a_used=17,b_used=23,physical_used=1234 WHERE id=1');
 const before={budget:db.prepare('SELECT * FROM r2_budget').all(),control:db.prepare('SELECT * FROM analytics_control').all()};db.exec(await readFile('migrations/0009.sql','utf8'));assert.deepEqual(db.prepare('SELECT * FROM r2_budget').all(),before.budget);assert.deepEqual(db.prepare('SELECT * FROM analytics_control').all(),before.control);
 const prepare=(sql,params=[])=>({bind:(...values)=>prepare(sql,values),run:async()=>({meta:{changes:Number(db.prepare(sql).run(...params).changes)}}),sql,params});
 const env={DB:{prepare,batch:async items=>{db.exec('BEGIN');try{const result=[];for(const item of items)result.push(await item.run());db.exec('COMMIT');return result;}catch(error){db.exec('ROLLBACK');throw error;}}}};
 if(bootstrap)db.exec(await readFile('migrations/0010.sql','utf8'));
 const account='a'.repeat(64);db.prepare("INSERT INTO accounts(id,email,beta,created_at) VALUES(?,'synthetic@example.com','active',?)").run(account,now);
 return {db,env,account,get now(){return now;},advance:n=>{now=n;},close:()=>db.close()};
}
test('actual SQL: additive bootstrap preserves populated activity, immutable start, privacy choices and original views',async()=>{
 const f=await sqlite(false);try{
  await admitDailyActivity(f.env,f.account,f.now);
  f.db.prepare('INSERT INTO analytics_choices(account_id,revision,decided_at,aggregate_objected) VALUES(?,?,?,1)').run(f.account,'c'.repeat(64),f.now);
  f.db.prepare('INSERT INTO analytics_consent(account_id,consented_at) VALUES(?,?)').run(f.account,f.now);
  f.db.exec('UPDATE analytics_control SET enabled=0,dropped=7');
  const snapshot=()=>Object.fromEntries(['aggregate_dau_days','aggregate_dau_dedup','aggregate_dau_exclusions','analytics_control','analytics_choices','analytics_consent','analytics_outbox','r2_budget'].map(table=>[table,f.db.prepare('SELECT * FROM '+table).all()]));
  const before=snapshot(),state=f.db.prepare('SELECT id,collection_started_at_ms,as_of_ms FROM aggregate_dau_state').all();
  const views=f.db.prepare("SELECT name,sql FROM sqlite_schema WHERE type='view' AND name!='reporting_daily_active_accounts' ORDER BY name").all();
  const rows=f.db.prepare('SELECT * FROM reporting_daily_active_accounts').all();
  f.db.exec(await readFile('migrations/0010.sql','utf8'));
  assert.deepEqual(snapshot(),before);assert.deepEqual(f.db.prepare('SELECT id,collection_started_at_ms,as_of_ms FROM aggregate_dau_state').all(),state);
  assert.deepEqual(f.db.prepare("SELECT name,sql FROM sqlite_schema WHERE type='view' AND name!='reporting_daily_active_accounts' ORDER BY name").all(),views);
  assert.deepEqual(f.db.prepare('SELECT * FROM reporting_daily_active_accounts').all(),rows,'an existing observed total remains unchanged');
 }finally{f.close();}
});
test('actual SQL: empty reporting has typed current-day unknown, never invented activity, and yields to real observed totals',async()=>{
 const f=await sqlite();try{
  const start=f.db.prepare('SELECT collection_started_at_ms FROM aggregate_dau_state').get().collection_started_at_ms;
  const rows=()=>f.db.prepare('SELECT * FROM reporting_daily_active_accounts ORDER BY day_utc').all();
  let result=rows();assert.equal(result.length,1,'an empty source still needs a non-activity row for native schema bootstrap');
  assert.equal(result[0].active_accounts,null);assert.equal(result[0].coverage,'unknown');assert.equal(result[0].day_utc,new Date(f.now).toISOString().slice(0,10));assert.equal(result[0].collection_started_at_ms,start);
  assert.deepEqual(f.db.prepare('PRAGMA table_info(reporting_daily_active_accounts)').all().map(r=>[r.name,r.type]),[['day_utc','TEXT'],['active_accounts','INTEGER'],['as_of_ms','INTEGER'],['collection_started_at_ms','INTEGER'],['coverage','TEXT']]);
  for(const value of [0,1])assert.throws(()=>f.db.prepare('UPDATE aggregate_dau_state SET reporting_unknown_accounts=?').run(value));
  assert.equal(f.db.prepare('SELECT count(*) n FROM aggregate_dau_days').get().n,0,'status metadata cannot create an activity day');
  assert.equal(f.db.prepare('SELECT count(*) n FROM aggregate_dau_dedup').get().n,0,'no identity is needed for bootstrap');
  await admitDailyActivity(f.env,f.account,f.now);result=rows();assert.equal(result.length,1);assert.equal(result[0].active_accounts,1);assert.equal(result[0].coverage,'observed_lower_bound');
  f.advance(f.now+1);result=rows();assert.equal(result.length,2);assert.equal(result[0].active_accounts,1);assert.equal(result[1].active_accounts,null);assert.equal(result[1].coverage,'unknown');assert.equal(result[1].collection_started_at_ms,start);
  await admitDailyActivity(f.env,f.account,f.now);result=rows();assert.equal(result.length,2);assert.ok(result.every(r=>r.active_accounts===1&&r.coverage==='observed_lower_bound'));
 }finally{f.close();}
});
test('actual SQL: original UTC boundary, idempotent count, logical TTL, late retry refusal and immutable aggregate after forgetting',async()=>{
 const f=await sqlite();try{await Promise.all(Array.from({length:8},()=>admitDailyActivity(f.env,f.account,f.now)));assert.equal(f.db.prepare('SELECT active_accounts n FROM aggregate_dau_days').get().n,1);const first=f.now;f.advance(first+1);await admitDailyActivity(f.env,f.account,f.now);assert.equal(f.db.prepare('SELECT count(*) n FROM aggregate_dau_days').get().n,2);f.advance(first+1+HOUR);await cleanupDailyActivity(f.env);assert.equal(f.db.prepare('SELECT count(*) n FROM aggregate_dau_dedup').get().n,1);assert.equal(f.db.prepare('SELECT sum(active_accounts) n FROM aggregate_dau_days').get().n,2);await admitDailyActivity(f.env,f.account,first);assert.equal(f.db.prepare('SELECT sum(active_accounts) n FROM aggregate_dau_days').get().n,2);assert.ok(f.db.prepare('SELECT expires_at_ms FROM aggregate_dau_dedup').get().expires_at_ms-f.now<=25*HOUR);}finally{f.close();}
});
test('actual SQL: global stop, bounded capacity, pre-registered fixture exclusion survival and rolling retention',async()=>{
 const f=await sqlite();try{
  f.db.exec('UPDATE analytics_control SET enabled=0');await admitDailyActivity(f.env,f.account,f.now);assert.equal(f.db.prepare('SELECT count(*) n FROM aggregate_dau_dedup').get().n,0);f.db.exec('UPDATE analytics_control SET enabled=1');
  f.db.prepare('INSERT INTO aggregate_dau_exclusions(account_id) VALUES(?)').run('b'.repeat(64));await cleanupDailyActivity(f.env);assert.equal(f.db.prepare('SELECT count(*) n FROM aggregate_dau_exclusions').get().n,1,'cleanup retains fixture pre-registration before its account exists');
  const day=new Date(f.now).toISOString().slice(0,10),add=f.db.prepare('INSERT INTO aggregate_dau_dedup VALUES(?,?,?)');for(let i=0;i<10000;i++)add.run(day,i.toString(16).padStart(64,'0'),f.now+25*HOUR);
  await admitDailyActivity(f.env,f.account,f.now);assert.equal(f.db.prepare('SELECT count(*) n FROM aggregate_dau_dedup WHERE account_id=?').get(f.account).n,0,'cap overflow cannot invent full coverage or a true zero');
  f.db.exec("INSERT INTO aggregate_dau_days VALUES('2000-01-01',1)");await cleanupDailyActivity(f.env);assert.equal(f.db.prepare("SELECT count(*) n FROM aggregate_dau_days WHERE day_utc='2000-01-01'").get().n,0);
  assert.deepEqual(f.db.prepare('PRAGMA table_info(reporting_daily_active_accounts)').all().map(r=>r.name),['day_utc','active_accounts','as_of_ms','collection_started_at_ms','coverage']);
 }finally{f.close();}
});
test('actual SQL: prospective start, reserved .invalid fixture, explicit objections, erasure and no misleading zeros',async()=>{
 const f=await sqlite();try{
  await admitDailyActivity(f.env,f.account,f.now-1);assert.equal(f.db.prepare('SELECT count(*) n FROM aggregate_dau_dedup').get().n,0);
  f.db.prepare("UPDATE accounts SET email='fixture@example.invalid' WHERE id=?").run(f.account);await admitDailyActivity(f.env,f.account,f.now);assert.equal(f.db.prepare('SELECT count(*) n FROM aggregate_dau_dedup').get().n,0);
  f.db.prepare("UPDATE accounts SET email='test@example.com' WHERE id=?").run(f.account);await admitDailyActivity(f.env,f.account,f.now);
  f.db.prepare('INSERT INTO analytics_choices(account_id,revision,decided_at) VALUES(?,?,?)').run(f.account,'f'.repeat(64),f.now);await cleanupDailyActivity(f.env);
  assert.equal(f.db.prepare('SELECT count(*) n FROM aggregate_dau_dedup').get().n,0);assert.equal(f.db.prepare("SELECT count(*) n FROM reporting_daily_active_accounts WHERE coverage='observed_lower_bound'").get().n,0);
  await admitDailyActivity(f.env,f.account,f.now);assert.equal(f.db.prepare('SELECT count(*) n FROM aggregate_dau_dedup').get().n,0);
  f.db.prepare('DELETE FROM analytics_choices WHERE account_id=?').run(f.account);await admitDailyActivity(f.env,f.account,f.now);
  f.db.prepare('DELETE FROM accounts WHERE id=?').run(f.account);await cleanupDailyActivity(f.env);
  assert.equal(f.db.prepare('SELECT count(*) n FROM aggregate_dau_dedup').get().n,0);assert.equal(f.db.prepare('SELECT sum(active_accounts) n FROM aggregate_dau_days').get().n,0);
 }finally{f.close();}
});
