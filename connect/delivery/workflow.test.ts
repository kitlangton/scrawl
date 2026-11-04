import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
test('producer verifies before sealing and never publishes',async()=>{
 const workflow=await readFile(new URL('../../.github/workflows/connect-delivery.yml',import.meta.url),'utf8');
 for(const requirement of ["branches: [main]","node-version: '22.23.0'","bun-version: '1.3.10'",'npm ci --include=dev','bun install --frozen-lockfile','install --with-deps chromium firefox','shared/analytics/config.test.ts','platform: web','connect-browser-${{ github.sha }}','connect-worker-${{ github.sha }}'])assert.ok(workflow.includes(requirement),requirement);
 assert.ok(workflow.indexOf('Verify browser and analytics')<workflow.indexOf('Seal verified'));
 const producer=workflow.split('  publish:')[0];
 assert.doesNotMatch(producer,/wrangler deploy|cargo |secrets\.|public\/site|needs:.*(?:linux|macos)/);
 assert.equal((workflow.match(/timeout-minutes: 10/g)||[]).length,6);
});

test('callable publisher admits independently with production controls',async()=>{
 const workflow=await readFile(new URL('../../.github/workflows/connect-delivery.yml',import.meta.url),'utf8');
 const publish=workflow.split('  publish:')[1].split('  runtime-budget:')[0];
 assert.match(workflow,/workflow_call:[\s\S]*producer-run:[\s\S]*required: true[\s\S]*type: string/);
 assert.ok(workflow.includes("inputs.producer-run == ''"));
 for(const value of ["inputs.producer-run != ''", "github.repository == 'tryapparent/apparent'", "vars.HOSTED_CD_ENABLED == 'true'",'name: hosted-production','group: web-delivery-production-eligible','cancel-in-progress: false','${{ inputs.producer-run }}','${{ secrets.CLOUDFLARE_API_TOKEN }}','Require atomic retirement'])assert.ok(publish.includes(value),value);
 assert.doesNotMatch(publish,/needs:|github.event_name|github.event.workflow_run.id/);
 assert.ok(workflow.includes('needs: [select, produce, publish, recipient, recipient-release]'));
});

test('durable recipient releases verify trusted provenance and preserve executability',async()=>{
 const workflow=await readFile(new URL('../../.github/workflows/connect-delivery.yml',import.meta.url),'utf8');
 const release=workflow.split('  recipient-release:')[1];
 for(const value of ['needs: recipient',"github.ref == 'refs/heads/main'",'connect/delivery/recipient/release.ts recipient-assets release-assets', 'pattern: recipient-${{ github.sha }}-${{ github.run_id }}-*','gh release create','--latest=false']) assert.ok(release.includes(value),value);
 assert.doesNotMatch(release,/--clobber|gh release (?:edit|delete|upload)|wrangler|HOSTED_CD_ENABLED/);
 assert.ok(release.indexOf('release.ts')<release.indexOf('gh release create'));
});
