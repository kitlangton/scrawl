import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, copyFileSync, writeFileSync, readFileSync, readdirSync, chmodSync, rmSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { admit, verify } from './mac-artifact-consumer.ts'
import { produce, audit, producerOptions } from './mac-artifact.ts'
const developer = '/Applications/Xcode_26.0.app/Contents/Developer'
const loads = 'cmd LC_BUILD_VERSION\ncmdsize 32\nplatform 1\nminos 15.0\nsdk 26.0\ncmd LC_LOAD_DYLIB\ncmdsize 56\nname /usr/lib/libSystem.B.dylib (offset 24)'
function fixture(fn: (run: (override?: Record<string, string>, os?: string, kind?: Parameters<typeof produce>[6]) => ReturnType<typeof produce>, root: string) => void) {
  const root = mkdtempSync(join(tmpdir(), 'mac-producer-'))
  mkdirSync(join(root, '.cargo')); copyFileSync('.cargo/config.toml', join(root, '.cargo/config.toml'))
  mkdirSync(join(root, 'target/debug'), { recursive: true })
  for (const name of ['apparent', 'apparent-helper']) { writeFileSync(join(root, 'target/debug', name), name); chmodSync(join(root, 'target/debug', name), 0o755) }
  writeFileSync(join(root, 'Cargo.lock'), 'synthetic lock')
  const env = { GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/verification-mac.yml@refs/heads/main', GITHUB_WORKFLOW_SHA: 'b'.repeat(40), GITHUB_RUN_ID: '123', GITHUB_RUN_ATTEMPT: '1', GITHUB_JOB: 'rust' }
  const probes: Record<string, string> = { 'git status --porcelain --untracked-files=normal': '', 'git rev-parse HEAD': 'a'.repeat(40), 'git rev-parse HEAD^{tree}': 'c'.repeat(40), 'rustc -vV': 'rustc 1.90.0\nhost: x86_64-apple-darwin\nrelease: 1.90.0', 'cargo -V': 'cargo 1.90.0 (abc)', 'xcode-select -p': developer, 'xcodebuild -version': 'Xcode 26.0\nBuild version 17A123', 'xcrun --sdk macosx --show-sdk-version': '26.0', 'xcrun --sdk macosx --show-sdk-path': `${developer}/Platforms/MacOSX.platform/Developer/SDKs/MacOSX26.0.sdk`, 'xcrun swiftc --version': 'Apple Swift version 6.2\nTarget: x86_64-apple-macosx15.0' }
  try { fn((override = {}, os = 'darwin', kind = 'verified-main') => produce(root, join(root, 'payload'), { ...env, ...Object.fromEntries(Object.entries(override).filter(([k]) => !k.includes(' '))) }, (c, args) => {
    const key = [c, ...args].join(' ')
    if (key in override) return override[key]
    if (c === 'lipo') return 'x86_64'
    if (c === 'otool') return override.loads ?? loads
    assert.ok(key in probes, key); return probes[key]
  }, os as 'darwin', 'x64', kind), root) } finally { rmSync(root, { recursive: true, force: true }) }
}
test('production creator emits exact fixed payload, checkout identity, hashes and modes', () => fixture((run, root) => {
  const m = run(); assert.equal(m.source.sha, 'a'.repeat(40)); assert.equal(m.producer.workflow_sha, 'b'.repeat(40))
  assert.equal(Object.hasOwn(m.producer, 'kind'), false, 'Legacy main identity stays unchanged')
  assert.deepEqual(readdirSync(join(root, 'payload')).sort(), ['apparent', 'apparent-helper', 'manifest.json'])
  for (const f of m.files) { assert.equal(f.mode, 0o755); assert.equal(f.sha256, createHash('sha256').update(readFileSync(join(root, 'payload', f.name))).digest('hex')) }
  assert.deepEqual(JSON.parse(readFileSync(join(root, 'payload/manifest.json'), 'utf8')), m)
}))
const focused = { GITHUB_EVENT_NAME: 'workflow_dispatch', GITHUB_REF: 'refs/heads/t3/candidate',
  GITHUB_SHA: 'a'.repeat(40), GITHUB_WORKFLOW_SHA: 'a'.repeat(40), GITHUB_JOB: 'settings-build',
  GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/verification-mac.yml@refs/heads/t3/candidate' }
test('focused creator emits its explicit kind with the same bounded payload contract', () => fixture((run, root) => {
  const m = run(focused, 'darwin', 'focused-settings-build')
  assert.equal(m.producer.kind, 'focused-settings-build'); assert.equal(m.producer.job, 'settings-build')
  assert.equal(m.producer.workflow, focused.GITHUB_WORKFLOW_REF)
  assert.equal(m.source.sha, focused.GITHUB_SHA); assert.equal(m.producer.workflow_sha, m.source.sha)
  assert.equal(m.kind, 'mac-dev-executables'); assert.equal(m.schema, 1)
  assert.deepEqual(readdirSync(join(root, 'payload')).sort(), ['apparent', 'apparent-helper', 'manifest.json'])
  assert.deepEqual(JSON.parse(readFileSync(join(root, 'payload/manifest.json'), 'utf8')), m)
}))
for (const [label, override] of Object.entries<Record<string, string>>({
  event: { GITHUB_EVENT_NAME: 'workflow_run' }, main: { GITHUB_REF: 'refs/heads/main' },
  tag: { GITHUB_REF: 'refs/tags/v1' }, traversal: { GITHUB_REF: 'refs/heads/../candidate' },
  whitespace: { GITHUB_REF: 'refs/heads/t3/candidate\n' }, repository: { GITHUB_REPOSITORY: 'fork/apparent' },
  job: { GITHUB_JOB: 'settings-native' }, jobWhitespace: { GITHUB_JOB: 'settings-build\n' },
  checkout: { GITHUB_SHA: 'b'.repeat(40) }, workflowSha: { GITHUB_WORKFLOW_SHA: 'b'.repeat(40) },
  workflow: { GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/verification.yml@refs/heads/t3/candidate' },
  branchMismatch: { GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/verification-mac.yml@refs/heads/t3/other' },
  environmentSelector: { APPARENT_PREBUILT_KIND: 'focused-settings-build' },
  flags: { CARGO_PROFILE_DEV_OPT_LEVEL: '0' },
})) {
  test(`focused producer rejects ${label} before writing`, () => fixture((run, root) => {
    assert.throws(() => run({ ...focused, ...override }, 'darwin', 'focused-settings-build'))
    assert.equal(existsSync(join(root, 'payload')), false)
  }))
}
test('focused context cannot enter the legacy main creator without its explicit kind', () => fixture(run => assert.throws(() => run(focused))))
test('producer CLI kind is explicit and malformed arguments cannot fall back to main', () => {
  assert.deepEqual(producerOptions(['/tmp/output']), { kind: 'verified-main', output: '/tmp/output' })
  assert.deepEqual(producerOptions(['--producer-kind', 'focused-settings-build', '/tmp/output']), { kind: 'focused-settings-build', output: '/tmp/output' })
  for (const args of [[], ['--producer-kind'], ['--producer-kind', 'other', '/tmp/output'],
    ['--producer-kind', 'focused-settings-build'], ['/tmp/output', 'extra'], ['--unknown', '/tmp/output']]) assert.throws(() => producerOptions(args))
})
for (const [label, override] of Object.entries<Record<string, string>>({ dirty: { 'git status --porcelain --untracked-files=normal': ' M build.rs' }, identity: { GITHUB_RUN_ID: '../123' }, config: { RUSTFLAGS: '-C opt-level=3' }, sdk: { 'xcrun --sdk macosx --show-sdk-version': '27.0' }, sha: { 'git rev-parse HEAD': 'main' }, dependency: { loads: loads.replace('/usr/lib/libSystem.B.dylib', '/tmp/evil.dylib') } })) {
  test(`rejects ${label}`, () => fixture(run => assert.throws(() => run(override))))
}
test('wrong platform', () => fixture(run => assert.throws(() => run({}, 'linux'))))
test('missing helper', () => fixture((run, root) => { rmSync(join(root, 'target/debug/apparent-helper')); assert.throws(() => run()) }))
test('unsafe mode', () => fixture((run, root) => { chmodSync(join(root, 'target/debug/apparent'), 0o777); assert.throws(() => run()) }))
test('rejects unreviewed config', () => fixture((run, root) => { writeFileSync(join(root, '.cargo/config.toml'), '[build]'); assert.throws(() => run()) }))
test('runtime closure rejects traversal, loader paths, unknown commands and Xcode-only Swift', () => {
  for (const path of ['/usr/lib/../evil', '@loader_path/evil', '@rpath/libswiftCore.dylib']) assert.throws(() => audit(loads.replace('/usr/lib/libSystem.B.dylib', path), developer, () => true))
  assert.throws(() => audit(loads.replace('LC_LOAD_DYLIB', 'LC_ID_DYLIB'), developer))
  assert.throws(() => audit(loads + '\ncmd LC_RPATH\ncmdsize 32\npath /tmp (offset 12)', developer))
  const swift = loads.replace('/usr/lib/libSystem.B.dylib', '@rpath/libswiftCore.dylib') + '\ncmd LC_RPATH\ncmdsize 32\npath /usr/lib/swift (offset 12)'
  assert.equal(audit(swift, developer, () => true).closure[0].provider, 'macOS')
  assert.throws(() => audit(swift, developer, () => false))
})
test('workflow retains success-only producer after existing evidence and before final job', () => {
  const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8')
  const segment = workflow.slice(workflow.indexOf('      - name: Retain verified Intel'))
  assert.match(segment, /if: success\(\) && runner.os == 'macOS'/)
  assert.match(segment, /if-no-files-found: error/)
  assert.ok(workflow.indexOf('name: verification-receipts-rust-') < workflow.indexOf('      - name: Retain verified Intel'))
  assert.match(segment, /needs: \[select, desktop-ui, connect-client, connect-web, rust\]/)
})

test('main Mac producer selects the tested compiler before cache restore without disabling cold builds or cache publication', () => {
  const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8')
  const rust = workflow.split('\n  rust:\n')[1].split('\n  all:')[0]
  const setup = rust.split('      - uses: Swatinem/rust-cache@v2')[0]
  assert.match(setup, /uses: dtolnay\/rust-toolchain@stable/)
  assert.match(setup, /uses: \.\/\.github\/actions\/mac-rust-cache/)
  assert.ok(setup.includes("toolchain: ${{ runner.os == 'macOS' && '1.99.1' || 'stable' }}"))
  assert.ok(setup.includes("test \"$(rustc -V)\" = 'rustc 1.99.1 (58cf8f9c3 2026-10-13)'"))
  assert.ok(setup.includes("if: runner.os == 'macOS' && (needs.select.outputs.root == 'true' || needs.select.outputs.engine == 'true')"))
  const cache = rust.split('      - uses: Swatinem/rust-cache@v2')[1].split('      - uses:')[0]
  assert.match(cache, /workspaces: \|\n            \. -> target\n            capture\/engine -> target\n            connect\/client -> target/)
  assert.doesNotMatch(cache, /save-if:|lookup-only:|add-rust-environment-hash-key:/)
  assert.doesNotMatch(rust, /cache-hit/)
  assert.match(rust, /verification\.py --base "\$BASE" --run --online --suite rust/)
})

const selectedCompiler = '1.99.1-x86_64-apple-darwin'
test('Rust workflow exports only the existing selected Xcode directory', () => {
  const root = mkdtempSync(join(tmpdir(), 'xcode-selection-'))
  try {
    const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8')
    const step = workflow.split('      - name: Pin the selected developer directory for Rust verification\n')[1].split('      - name:')[0]
    assert.match(step, /if: runner.os == 'macOS' && needs.select.outputs.root == 'true'/)
    const shell = step.split('        run: |\n')[1]
    for (const [selected, exists, probeStatus, expected] of [
      [developer, 'yes', '0', 0], ['/tmp/Xcode.app/Contents/Developer', 'yes', '0', 1],
      [developer, 'no', '0', 1], [developer, 'yes', '7', 7],
    ] as const) {
      const output = join(root, 'environment'); writeFileSync(output, '')
      const result = spawnSync('bash', ['-e', '-c', `
xcode-select() { printf '%s\\n' "$SELECTED"; return "$PROBE_STATUS"; }
test() { if [[ "$1" == '-d' ]]; then [[ "$2" == "$SELECTED" && "$DIRECTORY_EXISTS" == yes ]]; else builtin test "$@"; fi; }
${shell}`], { encoding: 'utf8', env: { ...process.env, GITHUB_ENV: output, SELECTED: selected,
        DIRECTORY_EXISTS: exists, PROBE_STATUS: probeStatus } })
      assert.equal(result.status, expected, result.stderr)
      assert.equal(readFileSync(output, 'utf8'), expected === 0 ? `DEVELOPER_DIR=${developer}\n` : '')
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})
for (const os of ['macOS', 'Linux']) for (const status of [0, 7]) test(`verifier trace scopes ${os} environment and preserves status ${status}`, () => {
  const root = mkdtempSync(join(tmpdir(), 'verifier-trace-'))
  try {
    const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8')
    const start = workflow.indexOf('          cargo_trace_env=()')
    assert.ok(start >= 0)
    const shell = workflow.slice(start, workflow.indexOf('\n          verification_complete=true', start))
    writeFileSync(join(root, 'python3'), '#!/bin/bash\nprintf "%s\\n" "$CARGO_LOG" "$*" >> "$TRACE_RECORD"\nexit "$TRACE_STATUS"\n', { mode: 0o755 })
    const record = join(root, 'calls')
    const result = spawnSync('bash', ['-e', '-c', shell + '\necho "$CARGO_LOG"'], { encoding: 'utf8',
      env: { ...process.env, PATH: root + ':' + process.env.PATH, CARGO_LOG: 'inherited',
        BASE: 'synthetic-base', RUNNER_TEMP: root, RUNNER_OS: os, TRACE_RECORD: record, TRACE_STATUS: String(status) } })
    assert.equal(result.status, status, result.stderr)
    assert.deepEqual(readFileSync(record, 'utf8').trim().split('\n'), [
      os === 'macOS' ? 'cargo::compiler::fingerprint=trace' : 'inherited',
      `enablement/verification.py --base synthetic-base --run --online --suite rust --parallel-contracts --cargo-timings ${os === 'macOS' ? '--overlap-root-build ' : ''}--receipt-dir ${root}/verification-receipts`,
    ])
    assert.equal(result.stdout.trim(), status === 0 ? 'inherited' : '')
  } finally { rmSync(root, { recursive: true, force: true }) }
})
for (const os of ['macOS', 'Linux']) for (const status of [0, 7]) test(`combined Cargo preflight preserves ${os} selection and status ${status}`, () => {
  const root = mkdtempSync(join(tmpdir(), 'cargo-preflight-'))
  try {
    const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8')
    const start = workflow.indexOf('            preflight_filters=(')
    const end = workflow.indexOf('\n          fi', start)
    assert.ok(start >= 0 && end > start)
    const shell = workflow.slice(start, end)
    const record = join(root, 'calls')
    writeFileSync(join(root, 'cargo'), '#!/bin/bash\nprintf "%s\\n" "$*" >> "$TRACE_RECORD"\nexit "$TRACE_STATUS"\n', { mode: 0o755 })
    const result = spawnSync('bash', ['-e', '-c', shell + '\necho reached-after-preflight'], { encoding: 'utf8', env: { ...process.env, PATH: root + ':' + process.env.PATH, RUNNER_OS: os, TRACE_RECORD: record, TRACE_STATUS: String(status) } })
    assert.equal(result.status, status, result.stderr)
    assert.equal(result.stdout.includes('reached-after-preflight'), status === 0)
    const calls = readFileSync(record, 'utf8').trim().split('\n')
    assert.equal(calls.length, 1)
    assert.match(calls[0], /^test -j 3 --locked --timings -- /)
    assert.match(calls[0], /the_command_line_row_.*competing_launches_cannot_remove_the_winning_control_socket/)
    assert.equal(calls[0].includes('hosted_security_darwin_acl_grants_fail_closed'), os === 'macOS')
    assert.match(calls[0], /--test-threads=1$/)
  } finally { rmSync(root, { recursive: true, force: true }) }
})
test('actual Cargo preflight filters run their union and propagate selected failures', () => {
  const root = mkdtempSync(join(tmpdir(), 'cargo-filter-union-'))
  try {
    mkdirSync(join(root, 'src'))
    writeFileSync(join(root, 'Cargo.toml'), '[package]\nname="preflight-fixture"\nversion="0.1.0"\nedition="2024"\n')
    writeFileSync(join(root, 'src/lib.rs'), `
mod capture { mod desktop {
mod settings { mod settings { mod tests {
#[test] fn the_command_line_row_first() {}
#[test] fn the_command_line_row_future_match() {}
}}}
mod workflow { mod agent_capture { mod tests {
#[test] fn competing_launches_cannot_remove_the_winning_control_socket() {
assert!(std::env::var_os("PREFLIGHT_FAIL").is_none());
}
}}}
}}
mod connect { mod hosted { mod hosted_tests {
#[test] fn hosted_security_darwin_acl_grants_fail_closed() {}
}}}
#[test] fn unrelated_must_be_excluded() { panic!("unselected"); }
`)
    const env = { ...process.env, CARGO_TARGET_DIR: join(root, 'target'), CARGO_NET_OFFLINE: 'true',
      CARGO_LOG: 'cargo::compiler::fingerprint=trace' }
    const lock = spawnSync('cargo', ['generate-lockfile', '--offline'], { cwd: root, env, encoding: 'utf8', timeout: 60000 })
    assert.equal(lock.status, 0, lock.stderr)
    const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8')
    const start = workflow.indexOf('            preflight_filters=(')
    const shell = workflow.slice(start, workflow.indexOf('\n          fi', start))
    for (const os of ['Linux', 'macOS']) for (const fail of [false, true]) {
      const result = spawnSync('bash', ['-e', '-c', shell], { cwd: root, encoding: 'utf8', timeout: 60000,
        env: { ...env, RUNNER_OS: os, ...(fail ? { PREFLIGHT_FAIL: '1' } : {}) } })
      assert.equal(result.status, fail ? 101 : 0, result.stdout + result.stderr)
      assert.match(result.stderr, /cargo::compiler::fingerprint:/)
      assert.match(result.stdout, /the_command_line_row_first \.\.\. ok/)
      assert.match(result.stdout, /the_command_line_row_future_match \.\.\. ok/)
      assert.match(result.stdout, new RegExp('competing_launches_cannot_remove_the_winning_control_socket \\.\\.\\. ' + (fail ? 'FAILED' : 'ok')))
      assert.equal(result.stdout.includes('hosted_security_darwin_acl_grants_fail_closed ... ok'), os === 'macOS')
      assert.ok(!result.stdout.includes('unrelated_must_be_excluded ...'))
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})
for (const outcome of ['success', 'failure', 'incomplete', 'signal']) test(`actual verification TMPDIR cleanup: ${outcome}`, () => {
  const root = mkdtempSync(join(tmpdir(), 'verification-cleanup-'))
  try {
    const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8')
    const step = workflow.split('      - name: Execute Rust changed-tree checks (not GUI acceptance)\n')[1]
    const prefix = step.split('        run: |\n')[1].split('          # Reproduce the CI-sensitive fixtures')[0]
      .split('\n').map(line => line.replace(/^          /, '')).join('\n')
    const finish = outcome === 'success' ? 'verification_complete=true; exit 0' : outcome === 'failure' ? 'exit 7' : outcome === 'signal' ? 'kill -TERM $$' : 'exit 0'
    const result = spawnSync('bash', ['-c', prefix + '\nprintf "%s\\n" "$TMPDIR"\n' + finish], {
      encoding: 'utf8', env: { ...process.env, HOME: root },
    })
    const owned = result.stdout.trim()
    assert.ok(owned.startsWith(root + '/.apparent-verification.'))
    assert.equal(existsSync(owned), outcome !== 'success')
    if (outcome === 'success') assert.equal(result.status, 0, result.stderr)
    else assert.ok(result.status !== 0 || result.signal !== null, 'unfinished verification must not succeed')
    if (outcome === 'failure') assert.equal(result.status, 7)
  } finally { rmSync(root, { recursive: true, force: true }) }
})

const legacyCompiler = 'stable-x86_64-apple-darwin'
const testedCompiler = 'release: 1.99.1\nhost: x86_64-apple-darwin\ncommit-hash: 58cf8f9c35b355199727627d07ae34a4e2846d67'
const oldCompiler = 'release: 1.98.0\nhost: x86_64-apple-darwin\ncommit-hash: 88d9e12ae178fab0fb5cc050a94da85685d449ea'
const inventoryCases = [
  { name: 'known unused stable is removed', pass: true, removed: true },
  { name: 'already normalized', pass: true, inventory: selectedCompiler },
  { name: 'same compiler alias retained', pass: true, stable: testedCompiler },
  { name: 'extra toolchain', inventory: `${selectedCompiler}\n${legacyCompiler}\nnightly-x86_64-apple-darwin` },
  { name: 'missing selected', inventory: legacyCompiler },
  { name: 'duplicate entry', inventory: `${selectedCompiler}\n${selectedCompiler}` },
  { name: 'malformed inventory', inventory: `${selectedCompiler}\n\n${legacyCompiler}` },
  { name: 'wrong active name', active: legacyCompiler },
  { name: 'wrong host', compiler: testedCompiler.replace('x86_64', 'aarch64') },
  { name: 'wrong active commit', compiler: testedCompiler.replace('58cf8f9c35b355199727627d07ae34a4e2846d67', 'bad') },
  { name: 'unexpected stable version', stable: oldCompiler.replace('1.98.0', '1.97.0') },
  { name: 'wrong stable commit', stable: oldCompiler.replace('88d9e12ae178fab0fb5cc050a94da85685d449ea', 'bad') },
  { name: 'self hosted runner', runner: 'self-hosted' },
  { name: 'wrong OS', os: 'Linux' },
  { name: 'wrong architecture', arch: 'ARM64' },
  { name: 'failed uninstall', mode: 'fail', removed: true },
  { name: 'partial uninstall', mode: 'partial', removed: true },
]
for (const scenario of inventoryCases) test(`actual Mac inventory shell: ${scenario.name}`, () => {
  const root = mkdtempSync(join(tmpdir(), 'mac-toolchains-'))
  try {
    const action = readFileSync('.github/actions/mac-rust-cache/action.yml', 'utf8')
    const shell = action.split('      run: |\n')[1].split('\n').map(line => line.replace(/^        /, '')).join('\n')
    writeFileSync(join(root, 'inventory'), scenario.inventory ?? `${selectedCompiler}\n${legacyCompiler}`)
    writeFileSync(join(root, 'rustc'), '#!/bin/bash\nprintf "%s\\n" "$FIXTURE_COMPILER"\n', { mode: 0o755 })
    writeFileSync(join(root, 'rustup'), `#!/bin/bash
set -eu
case "$*" in
  'show active-toolchain') printf '%s (default)\\n' "$FIXTURE_ACTIVE" ;;
  'toolchain list --quiet') cat "$FIXTURE_ROOT/inventory" ;;
  'run ${selectedCompiler} rustc -vV') printf '%s\\n' "$FIXTURE_COMPILER" ;;
  'run ${legacyCompiler} rustc -vV') printf '%s\\n' "$FIXTURE_STABLE" ;;
  'toolchain uninstall ${legacyCompiler}')
    printf '%s\\n' "$*" >> "$FIXTURE_ROOT/mutations"
    test "$FIXTURE_MODE" != fail
    if [ "$FIXTURE_MODE" != partial ]; then printf '%s' '${selectedCompiler}' > "$FIXTURE_ROOT/inventory"; fi ;;
  *) exit 91 ;;
esac
`, { mode: 0o755 })
    const result = spawnSync('bash', ['-c', shell], { encoding: 'utf8', env: {
      ...process.env, PATH: `${root}:${process.env.PATH}`, GITHUB_ACTIONS: 'true',
      RUNNER_ENVIRONMENT: scenario.runner ?? 'github-hosted', RUNNER_OS: scenario.os ?? 'macOS', RUNNER_ARCH: scenario.arch ?? 'X64',
      FIXTURE_ROOT: root, FIXTURE_ACTIVE: scenario.active ?? selectedCompiler,
      FIXTURE_COMPILER: scenario.compiler ?? testedCompiler, FIXTURE_STABLE: scenario.stable ?? oldCompiler,
      FIXTURE_MODE: scenario.mode ?? 'normal',
    } })
    assert.equal(result.status === 0, scenario.pass === true, result.stdout + result.stderr)
    assert.equal(existsSync(join(root, 'mutations')), scenario.removed === true)
    if (scenario.removed) assert.equal(readFileSync(join(root, 'mutations'), 'utf8'), `toolchain uninstall ${legacyCompiler}\n`)
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('share build kind is bound to its own workflow and job without changing Settings', () => fixture((run) => {
  const env = { ...focused, GITHUB_JOB: 'share-build', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/native-proof-mac.yml@refs/heads/t3/candidate' }
  assert.throws(() => run(env, 'darwin', 'focused-settings-build'))
  assert.throws(() => run(focused, 'darwin', 'focused-share-build'))
  const manifest = run(env, 'darwin', 'focused-share-build')
  assert.equal(manifest.producer.kind, 'focused-share-build')
  assert.equal(manifest.producer.job, 'share-build')
  assert.equal(manifest.producer.workflow, env.GITHUB_WORKFLOW_REF)
  assert.equal(producerOptions(['--producer-kind', 'focused-share-build', '/fresh']).kind, 'focused-share-build')
}))

for (const kind of ['focused-settings-build', 'focused-share-build'] as const) test(`${kind} emitted manifest passes the real consumer contract`, () => fixture((run, root) => {
  const sharing = kind === 'focused-share-build'
  const jobName = sharing ? 'share-build' : 'settings-build'
  const workflow = sharing ? 'native-proof-mac.yml' : 'verification-mac.yml'
  const env = { ...focused, GITHUB_JOB: jobName, GITHUB_WORKFLOW_REF: `tryapparent/apparent/.github/workflows/${workflow}@refs/heads/t3/candidate` }
  const manifest = run(env, 'darwin', kind)
  const repository = { full_name: 'tryapparent/apparent' }
  const upstream = { id: 123, run_attempt: 1, repository, head_repository: repository, path: `.github/workflows/${workflow}`, head_branch: 't3/candidate', head_sha: manifest.source.sha, event: 'workflow_dispatch', status: 'in_progress', conclusion: null }
  const artifact = { id: 456, name: 'mac-dev-executables-123-1', workflow_run: { id: 123 }, expired: false, digest: 'sha256:' + 'f'.repeat(64) }
  const job = { name: jobName, run_id: 123, run_attempt: 1, status: 'completed', conclusion: 'success' }
  const identity = admit(upstream, artifact, [job], '123', '456', { kind, run: '123', attempt: '1', sha: manifest.source.sha, head: manifest.source.sha, tree: manifest.source.tree, workflow: manifest.producer.workflow, workflowSha: manifest.source.sha, job: sharing ? 'connect-worker' : 'settings-native', event: 'workflow_dispatch' })
  verify(join(root, 'payload'), manifest, { sha: manifest.source.sha, tree: manifest.source.tree, lock: manifest.cargo_lock_sha256, config: manifest.config_sha256 }, identity)
}))

test('selected Xcode 26.0.1 retains its exact patch/build identity', () => fixture(run => {
  const selected = '/Applications/Xcode_26.0.1.app/Contents/Developer'
  const manifest = run({
    'xcodebuild -version': 'Xcode 26.0.1\nBuild version 17A400',
    'xcode-select -p': selected,
    'xcrun --sdk macosx --show-sdk-path': `${selected}/Platforms/MacOSX.platform/Developer/SDKs/MacOSX26.0.sdk`,
  })
  assert.equal(manifest.toolchain.xcode, 'Xcode 26.0.1\nBuild version 17A400')
  assert.equal(manifest.toolchain.developer, selected)
  assert.equal(manifest.toolchain.sdk, '26.0')
}))
for (const version of ['26.0.2', '26.1', '27.0', '26.0 beta']) test(`rejects unreviewed Xcode ${version}`, () => fixture(run => {
  assert.throws(() => run({ 'xcodebuild -version': `Xcode ${version}\nBuild version 17A400` }))
}))
