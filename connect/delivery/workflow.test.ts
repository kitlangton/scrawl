import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
test('producer verifies before sealing and never publishes',async()=>{
 const workflow=await readFile(new URL('../../.github/workflows/connect-delivery.yml',import.meta.url),'utf8');
 for(const requirement of ["branches: [main]","node-version: '22.23.0'","bun-version: '1.3.10'",'npm ci --include=dev','bun install --frozen-lockfile','install --with-deps chromium firefox','shared/analytics/config.test.ts','platform: web','connect-browser-${{ github.sha }}','connect-worker-${{ github.sha }}'])assert.ok(workflow.includes(requirement),requirement);
 assert.ok(workflow.indexOf('Verify browser and analytics')<workflow.indexOf('Seal verified'));
 assert.doesNotMatch(workflow,/wrangler deploy|cargo |secrets\.|public\/site|needs:.*(?:linux|macos)/);
 assert.equal((workflow.match(/timeout-minutes: 10/g)||[]).length,2);
});
