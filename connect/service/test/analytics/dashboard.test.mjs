import {test} from 'node:test';
import assert from 'node:assert/strict';
import {usageDashboardPlan} from '../../scripts/usage-dashboard.mjs';

test('separate private dashboard discloses missing coverage and login limitation',()=>{
 const plan=usageDashboardPlan();
 assert.equal(plan.dashboard.is_shared,false);
 assert.match(plan.notice.body,/does not mean there are no signed-in accounts/);
 assert.match(plan.notice.body,/Login alone is not instrumented/);
 assert.equal(plan.insights.length,4);
 assert.deepEqual(plan.insights.map(i=>i.name),['Pageview visitors · 30 days','Connect action accounts · 24h','Connect action accounts · 7 days','Connect action accounts · 30 days']);
 assert.equal(plan.notice.type,'text');
 assert.ok(plan.insights.every(i=>i.query.kind==='DataVisualizationNode'&&i.query.display==='BoldNumber')); 
 assert.ok(plan.insights.every(i=>!i.name.includes('Active users')));
});

test('account metrics count identities across the entire window and qualifying events only',()=>{
 const plan=usageDashboardPlan();
 for(const [index,days] of [1,7,30].entries()) {
  const query=plan.insights[index+1].query.source.query;
  assert.match(query,/uniqExact\(distinct_id\)/);
  assert.ok(query.includes(`INTERVAL ${days} DAY`));
  assert.match(query,/properties.environment = 'production'/);
  assert.match(query,/timestamp <= now\(\)/);
  const events=[...query.matchAll(/'([^']+)'/g)].map(m=>m[1]);
  assert.deepEqual(events,['production','connect_account_created','connect_device_connected','connect_upload_completed']);
  assert.doesNotMatch(query,/GROUP BY|sum\(|connect_storage_snapshot|\$pageview/);
 }
 assert.match(plan.insights[0].query.source.query,/event IN \('\$pageview'\)/);
});
