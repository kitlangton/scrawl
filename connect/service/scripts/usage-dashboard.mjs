// Offline PostHog payloads only. Never reads credentials, fetches or enables collection.
import {pathToFileURL} from 'node:url';

const notice = `## Measurement coverage\n\nProduction collection and consent coverage have not been verified. A zero below means no matching events were received; it does not mean there are no signed-in accounts. Do not use this dashboard as a complete account census.\n\nConnect actions are account creation, first explicit device connection and completed upload. Login alone is not instrumented by the current analytics candidate. Storage snapshots, polling, retries and anonymous visits are excluded. Only opted-in accounts are eligible; an account is not necessarily one person.\n\nAll windows are rolling UTC windows ending at query time. Production only; synthetic/development events are excluded. Keep this notice until the operator verifies rollout and describes its actual start date and consent coverage.`;

export function usageDashboardPlan() {
 const actionEvents = ['connect_account_created','connect_device_connected','connect_upload_completed'];
 const metrics = [
  {name:'Pageview visitors · 30 days', days:30, events:['$pageview'], description:'Distinct browser identities with a recorded pageview. Not authenticated accounts, unique humans or capture-app users.'},
  ...[1,7,30].map(days=>({name:`Connect action accounts · ${days===1?'24h':days+' days'}`,days,events:actionEvents,description:'Distinct pseudonymous account identities with an account-created, device-connected or upload-completed event. Excludes login-only activity, system snapshots and browser visits. Limited to opted-in collection coverage.'})),
 ];
 return {
  dashboard:{name:'Apparent visitors and Connect actions',description:'Pageview visitors and accounts with recorded Connect actions, with explicit collection coverage. Does not measure all signed-in users.',is_shared:false},
  notice:{type:'text',body:notice},
  insights:metrics.map(({name,days,events,description})=>({
   name,description,
   query:{kind:'DataVisualizationNode',display:'BoldNumber',source:{kind:'HogQLQuery',query:`SELECT uniqExact(distinct_id) AS count FROM events WHERE timestamp >= now() - INTERVAL ${days} DAY AND timestamp <= now() AND properties.environment = 'production' AND event IN (${events.map(event=>`'${event}'`).join(', ')})`}},
  })),
 };
}

if(process.argv[1]&&import.meta.url===pathToFileURL(process.argv[1]).href) {
 if(process.argv.length!==2) {process.stderr.write('Usage: node scripts/usage-dashboard.mjs (offline JSON output only)\n');process.exitCode=2;}
 else process.stdout.write(JSON.stringify(usageDashboardPlan(),null,2)+'\n');
}
