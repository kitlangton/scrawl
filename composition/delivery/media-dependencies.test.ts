import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { installMediaDependencies, mediaPackages, installCaptureDependencies, capturePackages } from './media-dependencies.ts'

const environment = { GITHUB_ACTIONS: 'true', RUNNER_OS: 'Linux', ImageOS: 'ubuntu24' }
const osRelease = 'ID=ubuntu\nVERSION_CODENAME=noble\n'

test('uses isolated signed HTTPS sources and bounded process groups for the complete runtime', t => {
  const directory = mkdtempSync(join(tmpdir(), 'media-apt-contract-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const calls: { command: string; args: string[]; timeout: number }[] = []
  installMediaDependencies({ directory, environment, osRelease, root: false, run(command, args, options) {
    calls.push({ command, args, timeout: options.timeout })
    assert.equal(options.env.DEBIAN_FRONTEND, 'noninteractive')
  } })
  assert.equal(calls.length, 2)
  assert.deepEqual(calls.map(call => call.command), ['sudo', 'sudo'])
  const source = readFileSync(join(directory, 'ubuntu.sources'), 'utf8')
  assert.match(source, /URIs: https:\/\/archive\.ubuntu\.com\/ubuntu\//)
  assert.match(source, /URIs: https:\/\/security\.ubuntu\.com\/ubuntu\//)
  assert.equal((source.match(/Signed-By: \/usr\/share\/keyrings\/ubuntu-archive-keyring\.gpg/g) ?? []).length, 2)
  assert.doesNotMatch(source, /azure|trusted=yes|http:/)
  for (const call of calls) {
    assert.deepEqual(call.args.slice(0, 6), ['-n', 'env', 'DEBIAN_FRONTEND=noninteractive', 'timeout', '--signal=TERM', '--kill-after=10s'])
    assert(call.args.includes(`Dir::Etc::sourcelist=${join(directory, 'ubuntu.sources')}`))
    assert(call.args.includes('Dir::Etc::sourceparts=-'))
    assert(call.args.includes(`Dir::State::lists=${join(directory, 'lists')}`))
    assert(call.args.includes('APT::Update::Error-Mode=any'))
    assert(call.args.includes('Acquire::https::Timeout=20'))
    assert(!call.args.some(arg => /unauthenticated|AllowInsecure|AllowDowngrade/.test(arg)))
  }
  assert(calls[0].args.includes('60s'))
  assert(calls[1].args.includes('180s'))
  assert.equal(calls[0].args.at(-1), 'update')
  assert.deepEqual(calls[1].args.slice(-10), ['install', '--no-install-recommends', '-y', ...mediaPackages])
  assert.equal(calls[0].timeout + calls[1].timeout, 270_000)
})

test('update or installation failure propagates before native production can proceed', t => {
  for (const failedOperation of ['update', 'install']) {
    const directory = mkdtempSync(join(tmpdir(), 'media-apt-failure-'))
    t.after(() => rmSync(directory, { recursive: true, force: true }))
    const operations: string[] = []
    assert.throws(() => installMediaDependencies({ directory, environment, osRelease, root: true, run(command, args) {
      assert.equal(command, 'timeout')
      const operation = args.includes('update') ? 'update' : 'install'
      operations.push(operation)
      if (operation === failedOperation) throw new Error('synthetic repository timeout')
    } }), /synthetic repository timeout/)
    assert.deepEqual(operations, failedOperation === 'update' ? ['update'] : ['update', 'install'])
  }
})

test('rejects unsupported runners before issuing a privileged command', t => {
  const directory = mkdtempSync(join(tmpdir(), 'media-apt-runner-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  for (const override of [{ environment: {} }, { osRelease: 'ID=debian\nVERSION_CODENAME=bookworm\n' }]) {
    assert.throws(() => installMediaDependencies({ directory, environment, osRelease, root: true, ...override,
      run() { assert.fail('unsupported runner must not execute apt') } }), /declared Ubuntu/)
  }
})

test('capture dependency profile includes X11, PipeWire client closure and H264 without changing media setup', t => {
  const directory = mkdtempSync(join(tmpdir(), 'capture-apt-contract-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const calls: string[][] = []
  installCaptureDependencies({ directory, environment, osRelease, root: true, run(_command, args) { calls.push(args) } })
  assert.equal(calls.length, 2)
  assert.deepEqual(calls[1].slice(-capturePackages.length), [...capturePackages])
  for (const name of ['gstreamer1.0-x', 'gstreamer1.0-plugins-bad', 'gstreamer1.0-plugins-ugly']) assert(capturePackages.includes(name as typeof capturePackages[number]))
  assert(!mediaPackages.includes('gstreamer1.0-plugins-ugly' as typeof mediaPackages[number]))
  for (const name of ['gstreamer1.0-pipewire', 'libpipewire-0.3-modules', 'libspa-0.2-modules', 'pipewire-bin']) {
    assert(capturePackages.includes(name as typeof capturePackages[number]), `capture requires ${name}`)
    assert(!mediaPackages.includes(name as typeof mediaPackages[number]), `${name} stays capture-only`)
  }
})
