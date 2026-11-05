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
 for(const requirement of ["node-version: '22.23.0'","bun-version: '1.3.10'",'npm ci --include=dev','bun install --frozen-lockfile','scripts/playwright-runtime.ts','shared/analytics/config.test.ts','platform: web','connect-browser-${{ github.sha }}','connect-worker-${{ github.sha }}'])assert.ok(workflow.includes(requirement),requirement);
 const triggers=workflow.split('\nconcurrency:')[0];
 assert.match(triggers,/^  workflow_dispatch:/m);
 assert.match(triggers,/^  workflow_call:/m);
 assert.doesNotMatch(triggers,/^  (?:push|pull_request):/m);
 const produce=workflow.split('\n  produce:\n')[1].split('\n  publish:')[0];
 assert.match(produce,/github.repository == 'tryapparent\/apparent'/);
 assert.match(produce,/github.ref == 'refs\/heads\/main'/);
 assert.doesNotMatch(workflow,/playwright-core install --with-deps/);
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
 assert.ok(workflow.includes('needs: [select, produce, publish, recipient-select, recipient, recipient-release]'));
});

test('durable recipient releases verify trusted provenance and preserve executability',async()=>{
 const workflow=await readFile(new URL('../../.github/workflows/connect-delivery.yml',import.meta.url),'utf8');
 const release=workflow.split('  recipient-release:')[1];
 for(const value of ["github.ref == 'refs/heads/main'",'connect/delivery/recipient/release.ts recipient-assets release-assets', 'pattern: recipient-${{ github.sha }}-${{ github.run_id }}-*','gh release create','--latest=false']) assert.ok(release.includes(value),value);
 assert.doesNotMatch(release,/--clobber|gh release (?:edit|delete|upload)|wrangler|HOSTED_CD_ENABLED/);
 assert.ok(release.indexOf('release.ts')<release.indexOf('gh release create'));
});

test('successful recipient targets publish signed independent receipts before the four-target aggregate',async()=>{
 const workflow=await readFile(new URL('../../.github/workflows/connect-delivery.yml',import.meta.url),'utf8');
 const releaseSource=await readFile(new URL('./recipient/release.ts',import.meta.url),'utf8');
 const recipient=workflow.split('\n  recipient:\n')[1].split('\n  recipient-release:')[0];
 const aggregate=workflow.split('\n  recipient-release:\n')[1];
 assert.match(recipient,/contents: read[\s\S]*id-token: write[\s\S]*attestations: write/);
 assert.ok(recipient.indexOf('Test and build component only')<recipient.indexOf('Seal executable bytes'));
 assert.ok(recipient.includes('cp -R shared/config-native/. "$RUNNER_TEMP/recipient-source/shared/config-native/"'));
 assert.ok(recipient.includes('cp -R connect/client/. "$RUNNER_TEMP/recipient-source/connect/client/"'));
 assert.ok(recipient.includes('working-directory: ${{ runner.temp }}/recipient-source/connect/client'));
 assert.ok(recipient.includes("if [ '${{ matrix.runner }}' = 'macos-15-intel' ]; then"));
 assert.ok(recipient.includes('build_jobs="$(sysctl -n hw.ncpu)"'));
 assert.ok(recipient.includes('test "$build_jobs" -eq 4'));
 assert.ok(recipient.includes("cargo test --locked --target '${{ matrix.target }}' -j \"$build_jobs\""));
 assert.ok(recipient.includes("cargo build --locked --release --bin apparent-connect-recipient --target '${{ matrix.target }}' -j \"$build_jobs\""));
 assert.doesNotMatch(recipient,/gh release create|release\.ts (?:prepare-target|publish-targets?)/);
 assert.match(aggregate,/needs: \[select, produce, publish, recipient-select, recipient\]/);
 assert.match(aggregate,/timeout-minutes: 2/);
 assert.match(aggregate,/contents: write[\s\S]*id-token: write[\s\S]*attestations: write/);
 assert.ok(aggregate.indexOf('--publication-preflight')<aggregate.indexOf('actions/download-artifact@v4'));
 assert.doesNotMatch(aggregate.slice(0,aggregate.indexOf('--publication-preflight')),/actions\/attest-build-provenance|publish-targets|gh release create/);
 assert.ok(aggregate.indexOf('actions/download-artifact@v4')<aggregate.indexOf('release.ts prepare-targets'));
 assert.ok(aggregate.indexOf('release.ts prepare-targets')<aggregate.indexOf('actions/attest-build-provenance@v2'));
 assert.ok(aggregate.indexOf('actions/attest-build-provenance@v2')<aggregate.indexOf('release.ts publish-targets'));
 assert.match(aggregate,/WORKFLOW_BUDGET_RESERVE_SECONDS: '120'/);
 const expected=aggregate.match(/WORKFLOW_BUDGET_EXPECTED_JOB_NAMES: '([^']+)'/)?.[1];
 assert.ok(expected,'publisher must pin the complete job inventory for this workflow attempt');
 assert.deepEqual(JSON.parse(expected!),['select','produce','publish','recipient-select',
  'recipient (ubuntu-24.04, x86_64-unknown-linux-gnu)','recipient (ubuntu-24.04-arm, aarch64-unknown-linux-gnu)',
  'recipient (macos-15, aarch64-apple-darwin)','recipient (macos-15-intel, x86_64-apple-darwin)',
  'Recipient target release and aggregate']);
 assert.match(workflow,/runtime-budget:[\s\S]*needs: \[select, produce, publish, recipient-select, recipient, recipient-release\]/,
  'the terminal audit is downstream of the publisher and must not be expected in its live job inventory');
 assert.match(aggregate,/if: always\(\) && !cancelled\(\)/);
 assert.doesNotMatch(aggregate,/continue-on-error: true/);
 assert.match(aggregate,/if: needs\.recipient\.result == 'success'/);
 assert.match(aggregate,/gh release create/);
 assert.match(aggregate,/--latest=false/);
 for(const value of ['--source-ref','--source-digest',"execute('gh',['release','create'"])assert.ok(releaseSource.includes(value),value);
 assert.match(workflow,/needs: \[select, produce, publish, recipient-select, recipient, recipient-release\]/);
 assert.match(releaseSource,/No successful recipient targets to publish/);
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
 const release=workflow.split('  recipient-release:')[1];assert.match(release,/needs: \[[^\]]*recipient\]/);
});

test('explicit web-only dispatch retains trusted web production and cannot select, build or release recipients', async () => {
 const workflow=await readFile(new URL('../../.github/workflows/connect-delivery.yml',import.meta.url),'utf8');
 assert.match(workflow,/workflow_dispatch:\n    inputs:\n      web-only:[\s\S]*?type: boolean\n        required: false\n        default: false/);
 for(const job of ['recipient-select','recipient','recipient-release']) {
  const condition=workflow.split('\n  '+job+':\n')[1].match(/^    if: (.+)$/m)![1];
  assert.ok(condition.endsWith(' && inputs.web-only != true'),`${job} must fail closed for a web-only dispatch`);
  // Conditions retain their original gates as well as the explicit mode guard.
  assert.match(condition,/github.repository == 'tryapparent\/apparent'/);
  assert.match(condition,/github.ref == 'refs\/heads\/main'/);
  assert.match(condition,/inputs.producer-run == ''/);
 }
 const produce=workflow.split('\n  produce:\n')[1].split('\n  publish:')[0];
 assert.doesNotMatch(produce,/web-only/);
 assert.match(produce,/connect\/delivery\/artifacts.ts validate/);
 assert.match(produce,/name: connect-browser-\$\{\{ github.sha \}\}/);
 const publish=workflow.split('\n  publish:\n')[1].split('\n  runtime-budget:')[0];
 assert.match(publish,/inputs.producer-run != ''/);
 assert.doesNotMatch(publish,/web-only/);
});
