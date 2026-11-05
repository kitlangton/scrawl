import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import test from 'node:test'

const root = new URL('../../../.github/workflows/', import.meta.url)
const workflow = readFileSync(new URL('value-stream-delivery.yml', root), 'utf8')
const workerWorkflow = readFileSync(new URL('web-delivery.yml', root), 'utf8')
const section = (source: string, name: string) => source.split(`\n  ${name}:\n`)[1]?.split(/\n  [a-z-]+:\n/)[0] ?? ''

test('Public has its own static stream and deployment does not wait for unrelated streams', () => {
  const stream = section(workflow, 'stream-public')
  const deploy = section(workflow, 'deploy')
  assert.doesNotMatch(stream, /dtolnay\/rust-toolchain|cargo (?:build|test)|Linux verification|working-directory: connect\/(?:web|service)|cd .*connect\/(?:web|service)|npm run test:hosted|npm run test:analytics/)
  assert.match(workflow, /name: stream \(\$\{\{ matrix\.id \}\}, \$\{\{ matrix\.target \}\}\)/)
  assert.match(stream, /id: public\s+target: web/)
  assert.match(stream, /Public delivery and shared architecture contracts/)
  assert.match(stream, /enablement\/verification\/web\/candidate\.ts/)
  assert.match(stream, /composition\/delivery\/cli\.ts admit/)
  assert.match(stream, /composition\/delivery\/cli\.ts build/)
  assert.match(stream, /composition\/delivery\/cli\.ts seal/)
  assert.match(stream, /name: stream-\$\{\{ matrix\.id \}\}-\$\{\{ matrix\.target \}\}-/)
  assert.doesNotMatch(stream, /needs:|fromJSON|matrix: \$\{\{ fromJSON/)
  assert.match(deploy, /name: deploy \(public, web\)/)
  assert.match(deploy, /needs: stream-public/)
  assert.deepEqual([...deploy.matchAll(/^\s+needs: (.+)$/gm)].map((match) => match[1]), ['stream-public'])
  assert.match(deploy, /PUBLIC_COMPONENT_INPUT_DIGEST: \$\{\{ needs\.stream-public\.outputs\.input-digest \}\}/)
  assert.match(deploy, /stream-artifact\.ts --current \.build\/public-component \.build\/public-artifact/)
  assert.match(deploy, /scripts\/connect\/artifact\.ts/)
  assert.match(deploy, /scripts\/connect\/compose-connect\.ts/)
  assert.match(deploy, /scripts\/pages\/prepare-pages\.ts/)
  assert.match(deploy, /scripts\/pages\/prepare-pages-video\.ts/)
  assert.match(deploy, /scripts\/pages\/test-public-homepage\.ts/)
  assert.match(deploy, /scripts\/connect\/composed-browser\.test\.ts/)
  assert.match(deploy, /publish-pages\.ts seal \$\{\{ github\.sha \}\}/)
  assert.match(deploy, /node public\/site\/scripts\/cd\/publish-pages\.ts/)
  assert.equal([...workflow.matchAll(/node public\/site\/scripts\/cd\/publish-pages\.ts/g)].length, 1)
  assert.match(deploy, /vars\.WEBSITE_CD_ENABLED == 'true'/)
  assert.match(deploy, /group: website-production-eligible/)
  assert.match(deploy, /cancel-in-progress: false/)
  assert.match(deploy, /name: website-production/)
  assert.match(deploy, /ref: \$\{\{ github\.sha \}\}/)
  assert(!workflow.includes('\n  website:\n'), 'there must be a single Pages publisher')
  assert.doesNotMatch(workerWorkflow, /publish-pages\.ts|website-production|WEBSITE_CD_ENABLED/)
})

test('Public candidate workflow runs on push, manual dispatch, and successful Connect completion only', () => {
  assert.match(workflow, /workflow_run:\n\s+workflows: \[Connect producer\]/)
  assert.match(workflow, /types: \[completed\]/)
  const plan = section(workflow, 'plan')
  const contracts = section(workflow, 'contracts')
  assert.match(plan, /github\.event_name != 'workflow_run'/)
  assert.match(contracts, /github\.event_name != 'workflow_run'/)
  const stream = section(workflow, 'stream-public')
  assert.match(stream, /github\.event\.workflow_run\.conclusion == 'success'/)
  assert.match(stream, /github\.repository == 'tryapparent\/apparent'/)
  assert.match(stream, /github\.ref == 'refs\/heads\/main'/)
})

test('Connect completion keeps the independent Worker publisher and audit', () => {
  assert.match(workerWorkflow, /github\.event_name == 'workflow_run'/)
  assert.match(workerWorkflow, /workflow_run\.conclusion == 'success'/)
  assert.match(workerWorkflow, /workflow_run\.name == 'Connect producer'/)
  assert.match(workerWorkflow, /vars\.HOSTED_CD_ENABLED == 'true'/)
  assert.match(workerWorkflow, /uses: \.\/\.github\/workflows\/connect-delivery\.yml/)
  assert.match(workerWorkflow, /producer-run:.*github\.event\.workflow_run\.id/)
  assert.match(section(workerWorkflow, 'runtime-budget'), /needs: \[worker\]/)
  assert.doesNotMatch(section(workerWorkflow, 'worker'), /needs:|concurrency:/)
  assert(!workerWorkflow.split('jobs:')[0].includes('concurrency:'))
  assert.match(section(workflow, 'runtime-budget'), /needs: \[plan, contracts, stream, stream-public, deploy\]/)
})

test('Reusable worker caller grants the complete nested contents permission closure', () => {
  const callee = readFileSync(new URL('../../../.github/workflows/connect-delivery.yml', import.meta.url), 'utf8')
  const rank: Record<string, number> = { none: 0, read: 1, write: 2 }
  const caller = section(workerWorkflow, 'worker').match(/contents: (read|write|none)/)?.[1] ?? 'none'
  for (const requirement of callee.matchAll(/contents: (read|write|none)/g)) {
    assert(rank[caller] >= rank[requirement[1]], `worker contents:${caller} cannot grant nested contents:${requirement[1]}`)
  }
  assert.match(workerWorkflow.split('jobs:')[0], /contents: read/)
})

test('only Value-stream delivery has automatic workflow triggers', () => {
  const directory = new URL('../../../.github/workflows/', import.meta.url)
  for (const file of readdirSync(directory).filter(name => /\.ya?ml$/.test(name))) {
    const source = readFileSync(new URL(file, directory), 'utf8')
    const triggers = source.split('\non:\n')[1]?.split(/\n[^\s#]/)[0] ?? ''
    const events = [...triggers.matchAll(/^  ([a-z_]+):/gm)].map(match => match[1])
    assert(events.length > 0, file)
    assert.deepEqual(events.filter(event => !['workflow_dispatch', 'workflow_call'].includes(event)),
      file === 'value-stream-delivery.yml' ? ['push', 'workflow_run'] : [], file)
  }
})
