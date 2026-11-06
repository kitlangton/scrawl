import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import test from 'node:test'

const root = new URL('../../../.github/workflows/', import.meta.url)
const contracts = readFileSync(new URL('../actions/delivery-contracts/action.yml', root), 'utf8')
const component = readFileSync(new URL('../actions/delivery-component/action.yml', root), 'utf8')
const workerCaller = readFileSync(new URL('web-delivery.yml', root), 'utf8')
const entry = (name: string) => readFileSync(new URL(`stream-${name}.yml`, root), 'utf8')
const paths = (source: string) => source.split(/\n\s+paths:\n/)[1]?.split(/\n\s{2,}[a-z_]+:/)[0]?.match(/^\s+- '([^']+)'/gm)?.map(value => value.trim().slice(3, -1)) ?? []
const eventPaths = (source: string, event: string) => {
  const section = source.split(`\n  ${event}:\n`)[1]?.split(/\n  [a-z_]+:\n|\npermissions:/)[0] ?? ''
  return section.match(/^      - '([^']+)'/gm)?.map(value => value.match(/'([^']+)'/)![1]) ?? []
}
const section = (source: string, name: string) => source.split(`\n  ${name}:\n`)[1]?.split(/\n  [a-z-]+:\n/)[0] ?? ''
const registrations: Record<string, string> = {
  'app-shell': 'composition/app/delivery.json', record: 'record/delivery.json',
  'connect-browser': 'connect/tooling/hosting/delivery.json', 'connect-client': 'connect/delivery.json', engine: 'engine/delivery.json',
  'engine-native': 'engine/delivery/native/delivery.json', 'engine-capture-native': 'engine/delivery/capture/delivery.json', 'engine-media-native': 'engine/delivery/media/delivery.json',
  'platform-config': 'composition/config/native/delivery.json', 'platform-resources': 'shared/resources/delivery/delivery.json', onboarding: 'onboarding/delivery.json',
  agent: 'agent/delivery.json', public: 'public/delivery.json',
}
const owners = Object.keys(registrations)
const sharedActions = [
  '.github/actions/delivery-contracts/**', '.github/actions/delivery-component/**',
]

test('owner entrypoints select only their stream and shared delivery inputs', () => {
  const record = paths(entry('record'))
  const engine = paths(entry('engine'))
  assert(record.includes('record/**'))
  assert(record.includes('shared/ui/**'))
  assert(engine.includes('engine/**'))
  assert(record.includes(sharedActions[0]))
  assert(engine.includes(sharedActions[1]))
  assert(!record.includes('engine/**'))
  assert(!engine.includes('record/**'))
  for (const name of owners) {
    const source = entry(name)
    const sourcePaths = paths(source)
    assert(sourcePaths.includes(`.github/workflows/stream-${name}.yml`))
    assert(sourcePaths.includes('composition/delivery/**'))
    assert(sourcePaths.includes('composition/architecture/**'))
    for (const actionPath of sharedActions) assert(sourcePaths.includes(actionPath))
    if (name === 'public') assert(sourcePaths.includes('enablement/verification/web/**'))
    const registration = name === 'platform-resources'
      ? { id: name, kind: 'directory', stage: 'library', package: 'shared/resources/delivery', targets: ['linux-x64', 'macos-x64'], inputs: ['shared/resources/', '.cargo/'] }
      : JSON.parse(readFileSync(new URL(`../../../${registrations[name]}`, import.meta.url), 'utf8'))
    const matrix = JSON.parse(source.match(/^\s+matrix: (.+)$/m)![1]).include
    assert.deepEqual(matrix.map((row: any) => row.id), matrix.map(() => name))
    assert.deepEqual([...matrix.map((row: any) => row.target)].sort(), [...registration.targets].sort())
    assert(matrix.every((row: any) => row.runner === (row.target === 'macos-x64' ? 'macos-26-intel' : 'ubuntu-24.04')))
    assert(matrix.every((row: any) => row.kind === registration.kind && row.stage === registration.stage && row.package === registration.package))
    for (const input of registration.inputs) assert(sourcePaths.includes(`${input.replace(/\/$/, '')}/**`), `${name} workflow misses declared input ${input}`)
    for (const input of registration.files ?? []) assert(sourcePaths.includes(input), `${name} workflow misses declared file input ${input}`)
  }
})

test('platform resources workflow retains literal receipt names, full Cargo closure and only public targets', () => {
  const workflow = entry('platform-resources')
  const sourcePaths = paths(workflow)
  assert(sourcePaths.includes('shared/resources/**'))
  assert(sourcePaths.includes('.cargo/**'))
  assert.match(workflow, /name: deliver \/ Delivery contracts/)
  assert.match(workflow, /name: deliver \/ Workflow runtime budget/)
  assert.match(workflow, /name: deliver \/ stream \(\$\{\{ matrix\.id \}\}, \$\{\{ matrix\.target \}\}\)/)
  assert.match(workflow, /"id":"platform-resources","target":"linux-x64"/)
  assert.match(workflow, /"id":"platform-resources","target":"macos-x64"/)
  assert.doesNotMatch(workflow, /macos-arm64/)
  assert.match(workflow, /APPARENT_DELIVERY_WORKFLOW_PATH: '\.github\/workflows\/stream-platform-resources\.yml'/)
  assert.match(workflow, /id: \$\{\{ matrix\.id \}\}[\s\S]*?target: \$\{\{ matrix\.target \}\}[\s\S]*?publish: 'false'/)
})

test('shared composite actions hold steps while owner workflows own jobs and receipt identities', () => {
  assert.match(contracts, /node --experimental-strip-types --test composition\/delivery\/\*\.test\.ts enablement\/verification\/web\/workflow\.test\.ts/)
  assert.match(contracts, /composition\/architecture run check -- --owner shared --owner composition/)
  assert.match(contracts, /actionlint:1\.7\.12/)
  assert.match(component, /cli\.ts build/)
  assert.match(component, /inputs\.publish != 'true'/)
  assert.match(component, /cli\.ts admit/)
  assert.match(component, /cli\.ts seal/)
  assert.match(component, /attest-build-provenance/)
  for (const name of owners) {
    const source = entry(name)
    assert.match(source, /name: deliver \/ Delivery contracts/)
    assert.match(source, /name: deliver \/ stream \(/)
    assert.match(source, /name: verify-pr \/ pr-stream \(/)
    assert.match(source, /name: deliver \/ Workflow runtime budget/)
    assert.match(source, /APPARENT_DELIVERY_WORKFLOW_PATH: '\.github\/workflows\/stream-/)
    const budget = section(source, 'runtime-budget')
    const jobs = [...source.split('jobs:\n')[1].matchAll(/^  ([\w-]+):$/gm)].map(match => match[1]).filter(job => job !== 'runtime-budget')
    assert.ok(budget.includes(`needs: [${jobs.join(', ')}]`), `${name} runtime audit must wait for every owner job`)
  }
})

test('Public alone owns Pages deployment without Connect inputs or completion triggers', () => {
  const publicEntry = entry('public')
  assert(paths(publicEntry).includes('public/**'))
  assert.doesNotMatch(publicEntry, /workflow_run:|Connect producer|VITE_HOSTED_API_ORIGIN|CONNECT_BROWSER_ARTIFACT/)
  assert.match(publicEntry, /name: deploy \(public, web\)/)
  const deploy = section(publicEntry, 'deploy')
  assert.deepEqual([...deploy.matchAll(/^\s+needs: (.+)$/gm)].map(match => match[1]), ['stream'])
  assert.doesNotMatch(deploy, /scripts\/connect\/|CONNECT_BROWSER_ARTIFACT|COMPOSE_|connect-browser|VITE_HOSTED_API_ORIGIN/)
  assert.match(deploy, /scripts\/artifact\/verify\.ts/)
  assert.match(deploy, /node --experimental-strip-types scripts\/browser-setup\/install\.ts/)
  assert.doesNotMatch(deploy, /playwright-core install --with-deps/)
  assert.match(deploy, /scripts\/pages\/prepare-pages-video\.ts/)
  assert.match(deploy, /scripts\/pages\/test-public-homepage\.ts/)
  assert.match(section(publicEntry, 'deploy'), /permissions:\n      contents: read\n      actions: read/)
  assert.match(publicEntry, /stream-artifact\.ts --current/)
  assert.match(publicEntry, /publish-pages\.ts seal/)
  assert.match(publicEntry, /WEBSITE_CD_ENABLED == 'true'/)
  assert.deepEqual(readdirSync(root).filter(name => name.endsWith('.yml')).filter(name => readFileSync(new URL(name, root), 'utf8').includes('publish-pages.ts')), ['stream-public.yml'])
  assert.match(workerCaller, /vars\.HOSTED_CD_ENABLED == 'true'/)
  assert.doesNotMatch(workerCaller, /workflow_run:/)
})

test('Connect producer and client retain separate invocation and path ownership', () => {
  const connect = readFileSync(new URL('connect-delivery.yml', root), 'utf8')
  assert.match(connect, /workflow_dispatch:/)
  assert.match(connect, /workflow_call:/)
  assert.doesNotMatch(connect.split('\nconcurrency:')[0], /^  push:/m)
  assert(paths(entry('connect-client')).includes('connect/delivery/client/**'))
  assert(paths(entry('connect-client')).includes('connect/delivery.json'))
  assert(!paths(entry('connect-client')).includes('connect/web/**'))
  const streamWorkflows = readdirSync(root).filter(name => name.endsWith('.yml') && name.startsWith('stream-'))
  assert.deepEqual(streamWorkflows.sort(), owners.map(name => `stream-${name}.yml`).sort())
})

test('PR and main events share path filters, fixed target matrices and least privilege', () => {
  for (const name of owners) {
    const source = entry(name)
    assert(eventPaths(source, 'pull_request').length > 0, `${name} has PR paths`)
    assert.deepEqual(eventPaths(source, 'pull_request'), eventPaths(source, 'push'), `${name} PR paths match main paths`)
    assert.match(source, /verify-pr:\n[\s\S]*?if: github\.event_name == 'pull_request'/)
    const pr = section(source, 'verify-pr')
    assert.match(pr, /permissions:\n      contents: read\n      actions: read/)
    assert.match(pr, /publish: 'false'/)
    assert.doesNotMatch(pr, /id-token: write|attestations: write|secrets: inherit/)
    assert.match(pr, /PUBLIC_POSTHOG_TOKEN: phc_synthetic/)
  }
})

test('owner component checks are fresh on PRs; only main producer jobs admit, attest and upload', () => {
  assert.match(component, /if: inputs\.publish == 'true'/)
  assert.match(component, /if: inputs\.publish != 'true' \|\| steps\.admit\.outputs\.reuse != 'true'/)
  assert.match(component, /id: reuse\n\s+continue-on-error: true/)
  assert.match(component, /rm -rf "\$RUNNER_TEMP\/stream-output"/)
  for (const name of owners) {
    const source = entry(name)
    const producer = section(source, 'stream')
    const pr = section(source, 'verify-pr')
    assert.match(producer, /if: github\.ref == 'refs\/heads\/main'/)
    assert.match(producer, /publish: 'true'/)
    assert.match(pr, /contents: read/)
    assert.match(pr, /publish: 'false'/)
    assert.match(section(source, 'runtime-budget'), /needs: \[/)
  }
})

test('non-stream workflows remain manual or reusable', () => {
  const files = readdirSync(root).filter(name => /\.ya?ml$/.test(name))
  for (const file of files.filter(name => !name.startsWith('stream-'))) {
    const source = readFileSync(new URL(file, root), 'utf8')
    const triggers = source.split('\non:\n')[1]?.split(/\n[^\s#]/)[0] ?? ''
    const events = [...triggers.matchAll(/^  ([a-z_]+):/gm)].map(match => match[1])
    assert(events.every(event => ['workflow_dispatch', 'workflow_call'].includes(event)), `${file} has no automatic event`)
  }
})
