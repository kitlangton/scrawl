import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { installPublicBrowsers } from './install.ts'
import { installUbuntuDependencies } from '../../../../composition/delivery/runtime/ubuntu-dependencies.ts'

const environment = { GITHUB_ACTIONS: 'true', RUNNER_OS: 'Linux', ImageOS: 'ubuntu24' }
const osRelease = 'ID=ubuntu\nVERSION_CODENAME=noble\n'

test('Pages deployment uses the same bounded browser installer as the Public producer', () => {
  const workflow = readFileSync(new URL('../../../../.github/workflows/stream-public.yml', import.meta.url), 'utf8')
  const deploy = workflow.split('\n  deploy:\n')[1].split('\n  runtime-budget:\n')[0]
  const setup = deploy.split('- name: Install locked website dependencies and browser\n')[1].split('\n      - name:')[0]
  assert.match(setup, /bun install --frozen-lockfile/)
  assert.match(setup, /node --experimental-strip-types scripts\/browser-setup\/install\.ts/)
  assert.doesNotMatch(setup, /playwright-core install --with-deps/)
  const producer = readFileSync(new URL('../site-build/verify.ts', import.meta.url), 'utf8')
  assert.match(producer, /await installPublicBrowsers\(/)
})

test('CI runs Playwright complete Chromium/Firefox dependency closure under bounded signed HTTPS apt', async t => {
  for (const root of [true, false]) {
    const directory = mkdtempSync(join(tmpdir(), 'public-apt-contract-'))
    t.after(() => rmSync(directory, { recursive: true, force: true }))
    const calls: { command: string; args: string[] }[] = []
    installUbuntuDependencies({ directory, environment, osRelease, root, run(command, args, options) {
      calls.push({ command, args })
      assert.equal(options.timeout, 255_000)
      assert.equal(options.env.DEBIAN_FRONTEND, 'noninteractive')
    } }, { command: '/node', args: ['/locked/playwright/cli.js', 'install-deps', 'chromium', 'firefox'] })
    assert.equal(calls.length, 1)
    const call = calls[0]
    assert.equal(call.command, root ? 'env' : 'sudo')
    assert.deepEqual(call.args.slice(-9), ['timeout', '--signal=TERM', '--kill-after=10s', '240s', '/node', '/locked/playwright/cli.js', 'install-deps', 'chromium', 'firefox'])
    assert(call.args.includes(`APT_CONFIG=${join(directory, 'apt.conf')}`))
    if (!root) assert.deepEqual(call.args.slice(0, 3), ['-n', 'env', 'DEBIAN_FRONTEND=noninteractive'])
    const config = readFileSync(join(directory, 'apt.conf'), 'utf8')
    for (const option of ['Dir::Etc::parts "-";', 'Dir::Etc::main "-";', 'Dir::Etc::sourceparts "-";', 'Acquire::https::Timeout "20";', 'Acquire::Retries "1";', 'APT::Update::Error-Mode "any";']) assert(config.includes(option), option)
    assert(config.includes(`Dir::State::lists "${join(directory, 'lists')}";`))
    const source = readFileSync(join(directory, 'ubuntu.sources'), 'utf8')
    assert.match(source, /URIs: https:\/\/archive\.ubuntu\.com\/ubuntu\//)
    assert.match(source, /URIs: https:\/\/security\.ubuntu\.com\/ubuntu\//)
    assert.equal(source.match(/Signed-By: \/usr\/share\/keyrings\/ubuntu-archive-keyring.gpg/g)?.length, 2)
    assert.doesNotMatch(source + config, /azure|http:\/\/|trusted=yes|AllowInsecure|unauthenticated/)
  }
})

test('both browsers remain required, custom Chrome still installs Firefox, and dependency failures stop downloads', async t => {
  const directory = mkdtempSync(join(tmpdir(), 'public-browser-contract-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const chromium = join(directory, 'chromium'), firefox = join(directory, 'firefox')
  writeFileSync(chromium, 'fixture'); writeFileSync(firefox, 'fixture')
  const browserPaths = async () => ({ chromium, firefox })
  for (const customChrome of [undefined, chromium]) {
    const order: string[] = []
    const chrome = await installPublicBrowsers({ environment: { ...environment, CHROME_PATH: customChrome }, osRelease, browserPaths,
      dependencies(setup, installer) {
        t.after(() => rmSync(setup.directory, { recursive: true, force: true }))
        order.push('dependencies')
        assert.deepEqual(installer.args.slice(-3), ['install-deps', 'chromium', 'firefox'])
      }, run(command, args) {
        order.push('download')
        assert.equal(command, process.execPath)
        assert.deepEqual(args.slice(1), ['install', ...(customChrome ? ['firefox'] : ['chromium', 'firefox'])])
        assert(!args.includes('--with-deps'))
      } })
    assert.equal(chrome, chromium)
    assert.deepEqual(order, ['dependencies', 'download'])
  }
  await assert.rejects(installPublicBrowsers({ environment, osRelease, browserPaths,
    dependencies(setup) {
      t.after(() => rmSync(setup.directory, { recursive: true, force: true }))
      throw new Error('synthetic apt timeout')
    }, run() { assert.fail('browser download/build must not follow dependency failure') } }), /synthetic apt timeout/)
  await assert.rejects(installPublicBrowsers({ environment: {}, browserPaths: async () => ({ chromium, firefox: join(directory, 'missing') }),
    run(_command, args) { assert(args.includes('--with-deps'), 'local installation keeps the supported Playwright path') } }), /Installed browser is missing/)
})
