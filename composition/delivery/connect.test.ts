import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import { after, test } from 'node:test'
import { catalog, inputDigest, makePlan, sourceDigest, type Owner } from './core.ts'

for (const name of execFileSync('git', ['rev-parse', '--local-env-vars'], { encoding: 'utf8' }).trim().split('\n')) delete process.env[name]
const temp = mkdtempSync(resolve(tmpdir(), 'connect-delivery-'))
after(() => rmSync(temp, { recursive: true, force: true }))
const owner: Owner = {
  schema: 1, id: 'connect-client', kind: 'npm', package: 'connect/delivery/client',
  inputs: ['connect/delivery/client/'], verify: ['typecheck', 'test'], build: 'build',
  targets: ['linux-x64'], stage: 'library', language: 'typescript',
}
function git(cwd: string, ...args: string[]) { execFileSync('git', args, { cwd, stdio: 'ignore' }) }
function repo(name: string) {
  const root = resolve(temp, name)
  for (const dir of ['connect/delivery/client/src', 'engine/sdk', 'composition/delivery', 'composition/architecture', '.github/workflows', 'enablement/verification']) mkdirSync(resolve(root, dir), { recursive: true })
  git(root, 'init', '-q'); git(root, 'config', 'user.email', 'delivery-test@example.invalid'); git(root, 'config', 'user.name', 'delivery test')
  writeFileSync(resolve(root, 'connect/delivery/client/package.json'), JSON.stringify({ name: '@apparent/connect-client' }))
  writeFileSync(resolve(root, 'connect/delivery/client/src/index.ts'), 'export const client = true\n')
  writeFileSync(resolve(root, 'connect/delivery.json'), JSON.stringify(owner))
  writeFileSync(resolve(root, 'engine/sdk/implementation.ts'), 'export const engine = true\n')
  writeFileSync(resolve(root, 'composition/delivery/core.ts'), 'delivery\n')
  writeFileSync(resolve(root, 'composition/architecture/guard.ts'), 'architecture\n')
  mkdirSync(resolve(root, '.github/actions/delivery-component'), { recursive: true }); writeFileSync(resolve(root, '.github/actions/delivery-component/action.yml'), 'component steps\n')
  writeFileSync(resolve(root, 'enablement/verification/workflow-budget.ts'), 'budget\n')
  writeFileSync(resolve(root, 'LICENSE'), 'license\n')
  git(root, 'add', '.'); git(root, 'commit', '-qm', 'initial')
  return root
}

test('connect client is a bounded Linux npm library lane with architecture enforcement', () => {
  const root = resolve(process.cwd())
  const registration = catalog(root)
  assert.deepEqual(registration.blocked, [])
  const lane = makePlan(root).matrix.include.filter(row => row.id === 'connect-client')
  assert.deepEqual(lane.map(({ target, kind, stage, package: pkg, verify, build, architecture_owner }) => ({ target, kind, stage, package: pkg, verify, build, architecture_owner })), [
    { target: 'linux-x64', kind: 'npm', stage: 'library', package: 'connect/delivery/client', verify: ['typecheck', 'test'], build: 'build', architecture_owner: 'connect' },
  ])
})

test('connect source digest tracks its client and registration, but not Engine implementation', () => {
  const root = repo('digest')
  const initial = sourceDigest(root, owner)
  writeFileSync(resolve(root, 'engine/sdk/implementation.ts'), 'changed engine implementation\n')
  git(root, 'add', '.'); git(root, 'commit', '-qm', 'engine change')
  assert.equal(sourceDigest(root, owner), initial)
  writeFileSync(resolve(root, 'connect/delivery.json'), JSON.stringify({ ...owner, verify: ['typecheck', 'test', 'test:package'] }))
  git(root, 'add', '.'); git(root, 'commit', '-qm', 'registration change')
  const afterRegistration = sourceDigest(root, owner)
  assert.notEqual(afterRegistration, initial)
  writeFileSync(resolve(root, 'connect/delivery/client/src/index.ts'), 'export const client = false\n')
  git(root, 'add', '.'); git(root, 'commit', '-qm', 'client change')
  assert.notEqual(sourceDigest(root, owner), afterRegistration)
})

// Independent browser hosting delivery contracts.
{

for (const name of execFileSync('git', ['rev-parse', '--local-env-vars'], { encoding: 'utf8' }).trim().split('\n')) delete process.env[name]
process.env.ImageOS ??= 'ubuntu24'
process.env.ImageVersion ??= '2026.01.01'
const repoRoot = execFileSync('git', ['rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim()
const temp = mkdtempSync(resolve(tmpdir(), 'connect-host-delivery-'))
after(() => rmSync(temp, { recursive: true, force: true }))
const files = [
  'public/site/public/assets/hosted/source-pond-v4.png',
  'public/site/public/assets/hosted/agent-pond-v4.png',
  'public/site/public/assets/hosted/agent-reading-v4.png',
  'public/site/public/assets/fonts/HostGrotesk-Variable.ttf',
  'public/site/public/assets/brand/apparent-wordmark.svg',
]
const owner: Owner = {
  schema: 1, id: 'connect-browser', kind: 'directory', package: 'connect/tooling/hosting',
  inputs: ['connect/tooling/hosting/', 'connect/service/', 'connect/web/', 'shared/ui/', 'shared/analytics/'],
  verify: [], build: 'verify:artifact', targets: ['web'], stage: 'website', language: 'typescript',
  artifactPath: '.output', packageManager: 'npm', files,
  environment: ['PUBLIC_POSTHOG_ENABLED', 'PUBLIC_POSTHOG_TOKEN', 'PUBLIC_POSTHOG_HOST', 'PUBLIC_POSTHOG_ENVIRONMENT', 'VITE_HOSTED_API_ORIGIN', 'API_ORIGIN', 'SITE_ORIGIN'],
}
function git(cwd: string, ...args: string[]) { execFileSync('git', args, { cwd, stdio: 'ignore' }) }
function makeRepo(name: string) {
  const root = resolve(temp, name)
  for (const dir of [...owner.inputs.map(input => input.slice(0, -1)), 'composition/delivery', 'composition/architecture', '.github/workflows', 'enablement/verification', 'connect/delivery/client', ...files.map(file => file.slice(0, file.lastIndexOf('/')))]) mkdirSync(resolve(root, dir), { recursive: true })
  git(root, 'init', '-q'); git(root, 'config', 'user.email', 'delivery-test@example.invalid'); git(root, 'config', 'user.name', 'delivery test')
  writeFileSync(resolve(root, owner.package, 'delivery.json'), JSON.stringify(owner))
  for (const input of owner.inputs) writeFileSync(resolve(root, input, 'input.ts'), `${input}\n`)
  for (const file of files) writeFileSync(resolve(root, file), `asset ${file}\n`)
  writeFileSync(resolve(root, 'connect/delivery/client/client.ts'), 'client\n')
  writeFileSync(resolve(root, 'composition/delivery/core.ts'), 'delivery\n')
  writeFileSync(resolve(root, 'composition/architecture/guard.ts'), 'architecture\n')
  writeFileSync(resolve(root, '.github/workflows/stream-connect-browser.yml'), readFileSync(resolve(repoRoot, '.github/workflows/stream-connect-browser.yml')))
  writeFileSync(resolve(root, 'enablement/verification/workflow-budget.ts'), 'budget\n')
  writeFileSync(resolve(root, 'LICENSE'), 'license\n')
  git(root, 'add', '.'); git(root, 'commit', '-qm', 'initial')
  return root
}

test('Connect browser registers as the independent TypeScript web artifact lane', () => {
  const registration = catalog(repoRoot)
  assert.deepEqual(registration.blocked, [])
  const lane = makePlan(repoRoot).matrix.include.filter(row => row.id === 'connect-browser')
  assert.deepEqual(lane.map(({ target, kind, stage, package: pkg, verify, build, packageManager, architecture_owner }) => ({ target, kind, stage, package: pkg, verify, build, packageManager, architecture_owner })), [
    { target: 'web', kind: 'directory', stage: 'website', package: 'connect/tooling/hosting', verify: [], build: 'verify:artifact', packageManager: 'npm', architecture_owner: 'connect' },
  ])
  const registered = registration.owners.find(item => item.id === 'connect-browser')
  assert.deepEqual(registered?.inputs, owner.inputs)
  assert.deepEqual(registered?.files, files)
  assert.deepEqual(registered?.environment, owner.environment)
})

test('Connect browser digest covers its declared source, assets, and build environment', () => {
  const root = makeRepo('digest')
  const initial = inputDigest(root, owner)
  const change = (path: string, label: string) => {
    mkdirSync(resolve(root, path.slice(0, path.lastIndexOf('/'))), { recursive: true })
    writeFileSync(resolve(root, path), `changed ${label}\n`)
    git(root, 'add', '-A'); git(root, 'commit', '-qm', `change ${label}`)
  }
  change('connect/delivery/client/client.ts', 'unrelated client')
  assert.equal(inputDigest(root, owner), initial, 'client-only changes do not invalidate browser artifact reuse')
  for (const path of ['.github/workflows/stream-connect-browser.yml', 'connect/tooling/hosting/src/index.ts', 'connect/service/src/worker.ts', 'connect/web/browser.ts', 'shared/ui/button.ts', 'shared/analytics/client.ts', ...files]) {
    change(path, path)
    assert.notEqual(inputDigest(root, owner), initial, `${path} must invalidate reuse`)
  }
  const beforeEnv = inputDigest(root, owner)
  const oldOrigin = process.env.VITE_HOSTED_API_ORIGIN
  const oldApiOrigin = process.env.API_ORIGIN
  try {
    process.env.VITE_HOSTED_API_ORIGIN = 'https://app.tryapparent.com/connect'
    process.env.API_ORIGIN = 'https://changed.example.invalid/connect'
    assert.notEqual(inputDigest(root, owner), beforeEnv, 'build environment changes invalidate reuse')
  } finally {
    if (oldOrigin === undefined) delete process.env.VITE_HOSTED_API_ORIGIN
    else process.env.VITE_HOSTED_API_ORIGIN = oldOrigin
    if (oldApiOrigin === undefined) delete process.env.API_ORIGIN
    else process.env.API_ORIGIN = oldApiOrigin
  }
})

test('Connect browser installs pinned Bun through the shared action on fresh main and PR builds', () => {
  const workflow = readFileSync(resolve(repoRoot, '.github/workflows/stream-connect-browser.yml'), 'utf8')
  const component = readFileSync(resolve(repoRoot, '.github/actions/delivery-component/action.yml'), 'utf8')
  for (const job of ['stream', 'verify-pr']) {
    const lane = workflow.split('\n  ' + job + ':\n')[1].split(/\n  [a-z-]+:\n/)[0]
    assert.match(lane, /uses: \.\/\.github\/actions\/delivery-component/)
    assert.match(lane, /id: \$\{\{ matrix.id \}\}/)
  }
  assert.match(component, /uses: oven-sh\/setup-bun@v2\n\s+if: [^\n]*inputs.id == 'connect-browser'[\s\S]*?bun-version: '1\.3\.10'/)
  assert.match(readFileSync(resolve(repoRoot, '.github/actions/delivery-contracts/action.yml'), 'utf8'), /stream-connect-browser\.yml/)
})

}
