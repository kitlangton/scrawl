import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import test from 'node:test'

const root = new URL('../../../.github/workflows/', import.meta.url)
const reusable = readFileSync(new URL('value-stream-delivery.yml', root), 'utf8')
const workerCaller = readFileSync(new URL('web-delivery.yml', root), 'utf8')
const publicEntry = readFileSync(new URL('streams/public.yml', root), 'utf8')
const entry = (name: string) => readFileSync(new URL(`streams/${name}.yml`, root), 'utf8')
const paths = (source: string) => source.split(/\n\s+paths:\n/)[1]?.split(/\n\s{2,}[a-z_]+:/)[0]?.match(/^\s+- '([^']+)'/gm)?.map(value => value.trim().slice(3, -1)) ?? []
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
    assert.match(source, new RegExp(`workflow_path: '\\.github/workflows/streams/${name}\\.yml'`))
    assert(paths(source).includes(`.github/workflows/streams/${name}.yml`))
    assert(paths(source).includes('composition/delivery/**'))
    assert(paths(source).includes('composition/architecture/**'))
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
  assert.match(workerCaller, /workflow_run:\n\s+workflows: \[Connect producer\]/)
  assert.match(workerCaller, /vars\.HOSTED_CD_ENABLED == 'true'/)
})

test('Connect browser/Worker producer retains explicit invocation and client path isolation', () => {
  const connect = readFileSync(new URL('connect-delivery.yml', root), 'utf8')
  assert.match(connect, /workflow_dispatch:/)
  assert.match(connect, /workflow_call:/)
  assert.doesNotMatch(connect.split('\nconcurrency:')[0], /^  push:/m)
  assert(paths(entry('connect-client')).includes('connect/delivery/client/**'))
  assert(paths(entry('connect-client')).includes('connect/delivery.json'))
  assert(!paths(entry('connect-client')).includes('connect/web/**'))
  for (const name of ['public.yml', 'web-delivery.yml']) {
    const source = name === 'public.yml' ? publicEntry : workerCaller
    assert.match(source, /workflows: \[Connect producer\]/)
  }
  const streamWorkflows = readdirSync(new URL('../../../.github/workflows/streams/', import.meta.url)).filter(name => name.endsWith('.yml'))
  assert.deepEqual(streamWorkflows.sort(), ['agent.yml', 'app-shell.yml', 'connect-client.yml', 'engine-media-native.yml', 'engine-native.yml', 'engine.yml', 'onboarding.yml', 'platform-config.yml', 'public.yml', 'record.yml'])
})
