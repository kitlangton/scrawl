import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
test('automatic main producers share one noncanceling pending lane; explicit candidates and publishers stay isolated',async()=>{
 const workflow=await readFile(new URL('../../.github/workflows/connect-delivery.yml',import.meta.url),'utf8');
 const concurrency=workflow.split('\nconcurrency:\n')[1].split('\npermissions:')[0];
 assert.match(concurrency,/cancel-in-progress: false/);
 assert.doesNotMatch(concurrency,/queue:/); // Pinned actionlint 1.7.12; default single pending slot.
 const group=concurrency.match(/^  group: (.+)$/m)![1];
 const key=(changes:Record<string,string|number>={})=>{
 const values={'github.event_name':'push','github.ref':'refs/heads/main','github.run_attempt':1,'github.run_id':123,'inputs.producer-run':'',...changes};
 return group.replace(/\$\{\{(.*?)\}\}/g,(_,expression:string)=>{
 const expanded=expression.replace(/github\.(?:event_name|ref|run_attempt|run_id)|inputs\.producer-run/g,field=>JSON.stringify(values[field as keyof typeof values]));
 return String(Function(`return (${expanded})`)());
 });
 };
 const automatic=key();assert.equal(automatic,key({'github.run_id':124}));
 for(const changes of [
 {'github.event_name':'workflow_dispatch'}, {'github.run_attempt':2}, {'github.ref':'refs/heads/feature'},
 // workflow_call inherits its caller's push context, not event_name=workflow_call.
 {'inputs.producer-run':'42'},
 ]) {
 assert.notEqual(automatic,key(changes));
 assert.notEqual(key(changes),key({...changes,'github.run_id':124}));
 }
 assert.notEqual(key({'inputs.producer-run':'42'}),key({'inputs.producer-run':'43'}));
 assert.notEqual(key({'inputs.producer-run':'42'}),'web-delivery-production-eligible');
 const recipient=workflow.split('\n  recipient:\n')[1].split('\n  recipient-release:')[0];
 assert.doesNotMatch(recipient,/concurrency:|cancel-in-progress:/,'workflow serialization must not interrupt a running matrix target');
});
test('producer verifies before sealing and never publishes',async()=>{
 const workflow=await readFile(new URL('../../.github/workflows/connect-delivery.yml',import.meta.url),'utf8');
 for(const requirement of ["branches: [main]","node-version: '22.23.0'","bun-version: '1.3.10'",'npm ci --include=dev','bun install --frozen-lockfile','install --with-deps chromium firefox','shared/analytics/config.test.ts','platform: web','connect-browser-${{ github.sha }}','connect-worker-${{ github.sha }}'])assert.ok(workflow.includes(requirement),requirement);
 assert.ok(workflow.indexOf('Verify browser and analytics')<workflow.indexOf('Seal verified'));
 const producer=workflow.split('  publish:')[0];
 assert.doesNotMatch(producer,/wrangler deploy|cargo |secrets\.|public\/site|needs:.*(?:linux|macos)/);
 assert.equal((workflow.match(/timeout-minutes: 10/g)||[]).length,7);
});

test('callable publisher admits independently with production controls',async()=>{
 const workflow=await readFile(new URL('../../.github/workflows/connect-delivery.yml',import.meta.url),'utf8');
 const publish=workflow.split('  publish:')[1].split('  runtime-budget:')[0];
 assert.match(workflow,/workflow_call:[\s\S]*producer-run:[\s\S]*required: true[\s\S]*type: string/);
 assert.ok(workflow.includes("inputs.producer-run == ''"));
 for(const value of ["inputs.producer-run != ''", "github.repository == 'tryapparent/apparent'", "vars.HOSTED_CD_ENABLED == 'true'",'name: hosted-production','group: web-delivery-production-eligible','cancel-in-progress: false','${{ inputs.producer-run }}','${{ secrets.CLOUDFLARE_API_TOKEN }}','Require atomic retirement'])assert.ok(publish.includes(value),value);
 assert.doesNotMatch(publish,/needs:|github.event_name|github.event.workflow_run.id/);
 assert.ok(workflow.includes('needs: [select, produce, publish, recipient-select, recipient, recipient-release]'));
});

test('durable recipient releases verify trusted provenance and preserve executability',async()=>{
 const workflow=await readFile(new URL('../../.github/workflows/connect-delivery.yml',import.meta.url),'utf8');
 const release=workflow.split('  recipient-release:')[1];
 for(const value of ['needs: recipient',"github.ref == 'refs/heads/main'",'connect/delivery/recipient/release.ts recipient-assets release-assets', 'pattern: recipient-${{ github.sha }}-${{ github.run_id }}-*','gh release create','--latest=false']) assert.ok(release.includes(value),value);
 assert.doesNotMatch(release,/--clobber|gh release (?:edit|delete|upload)|wrangler|HOSTED_CD_ENABLED/);
 assert.ok(release.indexOf('release.ts')<release.indexOf('gh release create'));
});

test('recipient selection is independent of web production and a skip cannot publish native evidence',async()=>{
 const workflow=await readFile(new URL('../../.github/workflows/connect-delivery.yml',import.meta.url),'utf8');
 const selection=workflow.split('  recipient-select:')[1].split('  recipient:')[0];
 const recipient=workflow.split('  recipient:')[1].split('  recipient-release:')[0];
 assert.match(selection,/baseline.ts recipient/);assert.match(selection,/fetch-depth: 0/);
 assert.doesNotMatch(selection,/needs:|upload-artifact|cargo|npm ci|bun install/);
 assert.match(recipient,/needs: recipient-select/);
 assert.match(recipient,/needs.recipient-select.outputs.required == 'true'/);
 assert.match(selection,/inputs.producer-run == ''/);
 const release=workflow.split('  recipient-release:')[1];assert.match(release,/needs: recipient/);
});
