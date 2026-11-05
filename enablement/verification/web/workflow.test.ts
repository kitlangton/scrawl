import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import test from 'node:test'

const root = new URL('../../../.github/workflows/', import.meta.url)
const reusable = readFileSync(new URL('value-stream-delivery.yml', root), 'utf8')
const workerCaller = readFileSync(new URL('web-delivery.yml', root), 'utf8')
const publicEntry = readFileSync(new URL('stream-public.yml', root), 'utf8')
const entry = (name: string) => readFileSync(new URL(`stream-${name}.yml`, root), 'utf8')
const paths = (source: string) => source.split(/\n\s+paths:\n/)[1]?.split(/\n\s{2,}[a-z_]+:/)[0]?.match(/^\s+- '([^']+)'/gm)?.map(value => value.trim().slice(3, -1)) ?? []
const eventPaths = (source: string, event: string) => {
  const section = source.split(`\n  ${event}:\n`)[1]?.split(/\n  [a-z_]+:\n|\npermissions:/)[0] ?? ''
  return section.match(/^      - '([^']+)'/gm)?.map(value => value.match(/'([^']+)'/)![1]) ?? []
}
const section = (source: string, name: string) => source.split(`\n  ${name}:\n`)[1]?.split(/\n  [a-z-]+:\n/)[0] ?? ''
const registrations: Record<string, string> = {
  'app-shell': 'composition/app/delivery.json', record: 'record/delivery.json',
  'connect-client': 'connect/delivery.json',
  engine: 'engine/delivery.json', 'engine-native': 'engine/delivery/native/delivery.json',
  'engine-media-native': 'engine/delivery/media/delivery.json',
  'platform-config': 'composition/config/native/delivery.json', onboarding: 'onboarding/delivery.json',
  agent: 'agent/delivery.json', public: 'public/delivery.json',
}

test('owner entrypoints select only their stream and shared delivery inputs', () => {
  const record = paths(entry('record'))
  const engine = paths(entry('engine'))
  assert(record.includes('record/**'))
  assert(record.includes('shared/ui/**'))
  assert(engine.includes('engine/**'))
  assert(record.includes('.github/workflows/value-stream-delivery.yml'))
  assert(engine.includes('.github/workflows/value-stream-delivery.yml'))
  assert(!record.includes('engine/**'))
  assert(!engine.includes('record/**'))
  for (const name of ['app-shell', 'record', 'connect-client', 'engine', 'engine-native', 'engine-media-native', 'platform-config', 'onboarding', 'agent', 'public']) {
    const source = entry(name)
    assert.match(source, new RegExp(`owner_id: '${name}'`))
    assert.match(source, new RegExp(`workflow_path: '\\.github/workflows/stream-${name}\\.yml'`))
    assert(paths(source).includes(`.github/workflows/stream-${name}.yml`))
    assert(paths(source).includes('composition/delivery/**'))
    assert(paths(source).includes('composition/architecture/**'))
    if (name === 'public') assert(paths(source).includes('enablement/verification/web/**'))
    const registration = JSON.parse(readFileSync(new URL(`../../../${registrations[name]}`, import.meta.url), 'utf8'))
    const matrix = JSON.parse(source.match(/^\s+targets: '(.+)'$/m)![1]).include
    assert.deepEqual(matrix.map((row: any) => row.id), matrix.map(() => name))
    assert.deepEqual([...matrix.map((row: any) => row.target)].sort(), [...registration.targets].sort())
    assert(matrix.every((row: any) => row.runner === (row.target === 'macos-x64' ? 'macos-26-intel' : 'ubuntu-24.04')))
    assert(matrix.every((row: any) => row.kind === registration.kind && row.stage === registration.stage && row.package === registration.package))
    for (const input of registration.inputs) assert(paths(source).includes(`${input.replace(/\/$/, '')}/**`), `${name} workflow misses declared input ${input}`)
    for (const input of registration.files ?? []) assert(paths(source).includes(input), `${name} workflow misses declared file input ${input}`)
  }
})

test('reusable execution receives one fixed owner matrix and never plans unrelated work', () => {
  assert.match(reusable, /workflow_call:/)
  assert.match(reusable, /matrix: \$\{\{ fromJSON\(inputs\.targets\) \}\}/)
  assert.doesNotMatch(reusable, /cli\.ts plan|jobs:\n  plan:/)
  assert.match(reusable, /APPARENT_DELIVERY_WORKFLOW_PATH: \$\{\{ inputs\.workflow_path \}\}/)
  assert.match(reusable, /name: Delivery contracts/)
  assert.match(reusable, /name: Workflow runtime budget/)
  assert.equal([...reusable.matchAll(/id: reuse\n\s+continue-on-error: true/g)].length, 2)
  assert.equal([...reusable.matchAll(/rm -rf "\$RUNNER_TEMP\/stream-output"/g)].length, 4)
  assert.match(reusable, /steps\.reuse\.outcome != 'success'/)
})

test('Public composition consumes only its verified Public and Connect artifacts', () => {
  assert(paths(publicEntry).includes('public/**'))
  assert(paths(publicEntry).includes('shared/analytics/**'))
  assert.match(publicEntry, /workflow_run:\n\s+workflows: \[Connect producer\]/)
  assert.match(publicEntry, /workflow_run\.conclusion == 'success'/)
  assert.match(publicEntry, /head_repository\.full_name == github\.repository/)
  assert.match(publicEntry, /secrets: inherit/)
  const deploy = section(reusable, 'deploy')
  assert.match(deploy, /needs: stream-public/)
  assert.match(deploy, /stream-artifact\.ts --current/)
  assert.match(deploy, /publish-pages\.ts seal/)
  assert.match(deploy, /WEBSITE_CD_ENABLED == 'true'/)
  assert.equal([...reusable.matchAll(/node public\/site\/scripts\/cd\/publish-pages\.ts/g)].length, 1)
  assert.match(workerCaller, /vars\.HOSTED_CD_ENABLED == 'true'/)
  assert.doesNotMatch(workerCaller, /workflow_run:/)
})

test('Connect browser/Worker producer retains explicit invocation and client path isolation', () => {
  const connect = readFileSync(new URL('connect-delivery.yml', root), 'utf8')
  assert.match(connect, /workflow_dispatch:/)
  assert.match(connect, /workflow_call:/)
  assert.doesNotMatch(connect.split('\nconcurrency:')[0], /^  push:/m)
  assert(paths(entry('connect-client')).includes('connect/delivery/client/**'))
  assert(paths(entry('connect-client')).includes('connect/delivery.json'))
  assert(!paths(entry('connect-client')).includes('connect/web/**'))
  assert.match(publicEntry, /workflows: \[Connect producer\]/)
  assert.doesNotMatch(workerCaller, /workflow_run:/)
  const streamWorkflows = readdirSync(new URL('../../../.github/workflows/', import.meta.url)).filter(name => name.endsWith('.yml') && name.startsWith('stream-'))
  assert.deepEqual(streamWorkflows.sort(), ['stream-agent.yml', 'stream-app-shell.yml', 'stream-connect-client.yml', 'stream-engine-media-native.yml', 'stream-engine-native.yml', 'stream-engine.yml', 'stream-onboarding.yml', 'stream-platform-config.yml', 'stream-public.yml', 'stream-record.yml'])
})

test('PR and main entrypoints share owner paths and fixed target matrices', () => {
  for (const name of ['app-shell', 'record', 'connect-client', 'engine', 'engine-native', 'engine-media-native', 'platform-config', 'onboarding', 'agent', 'public']) {
    const source = entry(name)
    assert(eventPaths(source, 'pull_request').length > 0, `${name} has PR paths`)
    assert.deepEqual(eventPaths(source, 'pull_request'), eventPaths(source, 'push'), `${name} PR paths match main paths`)
    assert.match(source, /verify-pr:\n    if: .*github\.event_name == 'pull_request'/)
    assert.match(source, /verify-pr:[\s\S]*?permissions:\n      contents: read\n      actions: read\n[\s\S]*?uses: \.\/\.github\/workflows\/value-stream-delivery\.yml/)
    assert.doesNotMatch(source.split('  verify-pr:')[1], /id-token: write|attestations: write|secrets: inherit/)
  }
})

test('PR execution is fixed-owner, fresh, synthetic and read-only', () => {
  // A skipped prerequisite silently skips the required PR builds too.
  assert.match(section(reusable, 'contracts'), /if: .*github\.event_name == 'pull_request'/)
  // All jobs in a called workflow must fit the read-only PR caller's ceiling,
  // even when publication jobs are excluded by their runtime condition.
  assert.doesNotMatch(reusable, /^permissions:|^      (?:id-token|attestations): write/m)
  assert.match(reusable, /on:\n  workflow_call:/)
  assert.doesNotMatch(reusable.split('\njobs:')[0], /pull_request:|push:|workflow_dispatch:|workflow_run:/)
  const pr = section(reusable, 'stream-pr')
  assert.match(pr, /needs: contracts/)
  assert.match(pr, /matrix: \$\{\{ fromJSON\(inputs\.targets\) \}\}/)
  assert.match(pr, /contents: read/)
  assert.match(pr, /cli\.ts build/)
  assert.match(pr, /phc_synthetic/)
  assert.match(pr, /npm run test:electron/)
  assert.doesNotMatch(pr, /GH_TOKEN|secrets\.|vars\.|cli\.ts (?:admit|seal|reuse)|attest-build-provenance|upload-artifact|id-token: write|attestations: write/)
  for (const job of ['stream', 'stream-public', 'deploy']) assert.match(section(reusable, job), /github\.ref == 'refs\/heads\/main'/)
  assert.match(section(reusable, 'runtime-budget'), /needs: \[contracts, stream, stream-public, deploy, stream-pr\]/)
})

test('all standalone workflows remain manual or reusable, and Public has one publisher', () => {
  const directory = new URL('../../../.github/workflows/', import.meta.url)
  for (const file of readdirSync(directory).filter(name => /\.ya?ml$/.test(name) && !name.startsWith('stream-'))) {
    const source = readFileSync(new URL(file, directory), 'utf8')
    const triggers = source.split('\non:\n')[1]?.split(/\n[^\s#]/)[0] ?? ''
    const events = [...triggers.matchAll(/^  ([a-z_]+):/gm)].map(match => match[1])
    assert(events.every(event => ['workflow_dispatch', 'workflow_call'].includes(event)), `${file} has no automatic event`)
  }
  const publishers = readdirSync(directory).filter(name => /\.ya?ml$/.test(name)).filter(name => readFileSync(new URL(name, directory), 'utf8').includes('publish-pages.ts'))
  assert.deepEqual(publishers, ['value-stream-delivery.yml'])
})
