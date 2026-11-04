import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
const workflow = readFileSync(new URL('../../../.github/workflows/web-delivery.yml', import.meta.url), 'utf8')
const section = (name: string) => workflow.split(`\n  ${name}:\n`)[1]?.split(/\n  [a-z-]+:\n/)[0] ?? ''
test('Public consumes immutable Connect output without native or Connect product builds', () => {
  assert.doesNotMatch(workflow, /cargo|rust-toolchain|Linux verification|working-directory: connect\/(?:web|service)|cd .*connect\/(?:web|service)|npm run test:hosted|npm run test:analytics/)
  const verify = section('verify')
  assert(verify.indexOf('scripts/connect/artifact.ts') < verify.indexOf('node scripts/connect/compose-connect.mjs'))
  assert.match(verify, /CONNECT_BROWSER_ARTIFACT:/)
  assert.match(verify, /fetch-depth: 0/)
  assert.match(verify, /scripts\/connect\/composed-browser.test.ts/)
  assert.match(verify, /scripts\/connect\/analytics-smoke.ts/)
  assert.match(verify, /seal \$\{\{ needs.admit.outputs.sha \}\}/)
})
test('Public release gate handles explicit not-applicable checks and preserves publication controls', () => {
  assert.match(section('admit'), /selection.ts/)
  assert.match(section('verify'), /needs.admit.outputs.required == 'true'/)
  assert.match(section('release'), /if: always\(\)/)
  assert.match(section('release'), /WEB_REQUIRED: \$\{\{ needs.admit.outputs.required \}\}/)
  assert.match(section('release'), /WEB_RESULT: \$\{\{ needs.verify.result \}\}/)
  assert.match(section('release'), /selection.ts gate/)
  assert.match(section('website'), /needs: \[admit, verify, release\]/)
  assert.match(section('website'), /vars.WEBSITE_CD_ENABLED == 'true'/)
  assert.match(section('website'), /group: website-production-eligible/)
  assert.match(section('website'), /cancel-in-progress: false/)
  assert.match(section('website'), /name: website-\$\{\{ needs.admit.outputs.sha \}\}/)
})
test('Worker invokes owner reusable publication independently with a separate lock owner', () => {
  const worker = section('worker')
  assert.match(worker, /github.event_name == 'workflow_run'/)
  assert.match(worker, /workflow_run.conclusion == 'success'/)
  assert.match(worker, /vars.HOSTED_CD_ENABLED == 'true'/)
  assert.match(worker, /uses: .\/.github\/workflows\/connect-delivery.yml/)
  assert.match(worker, /producer-run:.*github.event.workflow_run.id/)
  assert.doesNotMatch(worker, /needs:|concurrency:/)
  assert(!workflow.includes('\n  hosted:\n'))
  assert(!workflow.split('jobs:')[0].includes('concurrency:'))
})
