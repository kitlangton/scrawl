import { spawnSync } from 'node:child_process'
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, writeFileSync, rmSync, symlinkSync, readdirSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createHash } from 'node:crypto'
import { admit, verify, unpack } from './mac-artifact-consumer.ts'
const repository = { full_name: 'tryapparent/apparent' }
const run = { id: 123, repository, head_repository: repository, path: '.github/workflows/verification-mac.yml', head_branch: 'main', event: 'workflow_run', status: 'completed', conclusion: 'success', run_attempt: 2 }
const artifact = { digest: 'sha256:' + 'e'.repeat(64), id: 456, expired: false, workflow_run: { id: 123 }, name: 'mac-dev-executables-123-2' }
const job = { name: 'verify / rust (macos-15-intel)', status: 'completed', conclusion: 'success', run_id: 123, run_attempt: 2 }
test('admits immutable successful trusted producer and exact attempt', () => {
  assert.deepEqual(admit(run, artifact, [job], '123', '456'), { kind: 'verified-main', run: '123', attempt: '2', artifact: '456', digest: artifact.digest })
})
for (const mutation of [{ conclusion: 'failure' }, { status: 'in_progress' }, { event: 'workflow_dispatch' }, { head_branch: 'feature' }, { repository: { full_name: 'other/repo' } }, { path: '.github/workflows/other.yml' }]) {
  test(`rejects producer ${JSON.stringify(mutation)}`, () => assert.throws(() => admit({ ...run, ...mutation }, artifact, [job], '123', '456')))
}
test('rejects failed jobs, expired/wrong artifacts and attempts', () => {
  for (const mutation of [{ digest: null }, { expired: true }, { id: 457 }, { workflow_run: { id: 124 } }, { name: 'mac-dev-executables-123-1' }]) assert.throws(() => admit(run, { ...artifact, ...mutation }, [job], '123', '456'))
  for (const jobs of [[], [job, job], [{ ...job, conclusion: 'failure' }], [{ ...job, run_attempt: 1 }]]) assert.throws(() => admit(run, artifact, jobs, '123', '456'))
})
function fixture(fn: (directory: string, manifest: any, expected: any, identity: any) => void) {
  const directory = mkdtempSync(join(tmpdir(), 'mac-consumer-'))
  const expected = { sha: 'a'.repeat(40), tree: 'b'.repeat(40), lock: 'c'.repeat(64), config: 'd'.repeat(64) }
  const files = ['apparent', 'apparent-helper'].map(name => {
    writeFileSync(join(directory, name), name, { mode: 0o644 })
    return { name, size: name.length, mode: 0o755, sha256: createHash('sha256').update(name).digest('hex') }
  })
  const manifest = { schema: 1, kind: 'mac-dev-executables', source: { sha: expected.sha, tree: expected.tree, clean: true }, producer: { repository: repository.full_name, run: '123', attempt: '2', job: 'rust', workflow: 'tryapparent/apparent/.github/workflows/verification-mac.yml@refs/heads/main' }, target: 'x86_64-apple-darwin', profile: 'dev', deployment_target: '15.0', cargo_lock_sha256: expected.lock, config_sha256: expected.config, files }
  writeFileSync(join(directory, 'manifest.json'), JSON.stringify(manifest))
  try { fn(directory, manifest, expected, admit(run, artifact, [job], '123', '456')) } finally { rmSync(directory, { recursive: true, force: true }) }
}
test('accepts exact fixed transport payload with recorded executable modes', () => fixture((...args) => verify(...args)))
test('rejects source, platform, hash and configuration mismatches', () => fixture((dir, m, expected, identity) => {
  for (const patch of [{ source: { ...m.source, sha: 'f'.repeat(40) } }, { source: { ...m.source, tree: 'f'.repeat(40) } }, { target: 'aarch64-apple-darwin' }, { cargo_lock_sha256: 'f'.repeat(64) }, { config_sha256: 'f'.repeat(64) }, { profile: 'release' }, { files: m.files.map((f: any) => ({ ...f, mode: 0o777 })) }, { files: m.files.map((f: any) => ({ ...f, sha256: 'f'.repeat(64) })) }, { files: [m.files[0], m.files[0]] }]) assert.throws(() => verify(dir, { ...m, ...patch }, expected, identity))
}))
test('rejects extra names, traversal names, missing helper and symlinks', () => fixture((dir, m, expected, identity) => {
  writeFileSync(join(dir, 'extra'), 'x'); assert.throws(() => verify(dir, m, expected, identity)); rmSync(join(dir, 'extra'))
  assert.throws(() => verify(dir, { ...m, files: [{ ...m.files[0], name: '../apparent' }, m.files[1]] }, expected, identity))
  rmSync(join(dir, 'apparent-helper')); assert.throws(() => verify(dir, m, expected, identity))
  symlinkSync(join(dir, 'apparent'), join(dir, 'apparent-helper')); assert.throws(() => verify(dir, m, expected, identity))
}))

function zip(kind: string) {
  const result = spawnSync('python3', ['-c', `import io,sys,zipfile,stat
output=io.BytesIO()
with zipfile.ZipFile(output, 'w') as z:
    for name in ['apparent', 'apparent-helper', 'manifest.json']:
        if sys.argv[1]=='traversal' and name=='apparent': name='../apparent'
        info=zipfile.ZipInfo(name)
        info.external_attr=(stat.S_IFLNK | 0o777) << 16 if sys.argv[1]=='symlink' else (stat.S_IFREG | 0o644) << 16
        z.writestr(info, 'x' * (1024*1024+1) if sys.argv[1]=='large' and name=='manifest.json' else name)
    if sys.argv[1]=='duplicate': z.writestr('apparent', 'replacement')
sys.stdout.buffer.write(output.getvalue())`, kind], { maxBuffer: 2 * 1024 * 1024 })
  assert.equal(result.status, 0)
  return result.stdout
}
test('download owner authenticates bytes before extracting only fixed regular files', () => {
  const root = mkdtempSync(join(tmpdir(), 'mac-archive-'))
  const directory = join(root, 'payload')
  try {
    const good = zip('good')
    const digest = 'sha256:' + createHash('sha256').update(good).digest('hex')
    // Regression: legitimate run/artifact IDs plus a forged local manifest and
    // replacement binaries cannot authenticate an unrelated archive.
    assert.throws(() => unpack(zip('traversal'), digest, directory))
    assert.equal(existsSync(directory), false)
    for (const kind of ['traversal', 'symlink', 'duplicate', 'large']) {
      const bytes = zip(kind)
      assert.throws(() => unpack(bytes, 'sha256:' + createHash('sha256').update(bytes).digest('hex'), directory))
      assert.equal(existsSync(directory), false)
    }
    unpack(good, digest, directory)
    assert.deepEqual(readdirSync(directory).sort(), ['apparent', 'apparent-helper', 'manifest.json'])
    assert.throws(() => unpack(good, digest, directory))
  } finally { rmSync(root, { recursive: true, force: true }) }
})

const focusedRun = { ...run, head_branch: 'candidate', head_sha: 'a'.repeat(40), event: 'workflow_dispatch', status: 'in_progress', conclusion: null }
const focusedJob = { ...job, name: 'settings-build' }
const focusedContext = { kind: 'focused-settings-build', run: '123', attempt: '2', sha: 'a'.repeat(40), head: 'a'.repeat(40), tree: 'b'.repeat(40), workflow: 'tryapparent/apparent/.github/workflows/verification-mac.yml@refs/heads/candidate', workflowSha: 'a'.repeat(40), job: 'settings-native', event: 'workflow_dispatch' }
test('focused build accepts only same active native candidate after successful build', () => fixture((dir, m, expected) => {
  const identity = admit(focusedRun, artifact, [focusedJob], '123', '456', focusedContext)
  const focused = { ...m, producer: { ...m.producer, kind: 'focused-settings-build', job: 'settings-build', workflow: focusedContext.workflow, workflow_sha: focusedContext.sha } }
  verify(dir, focused, expected, identity)
  assert.throws(() => verify(dir, m, expected, identity), 'main manifest cannot impersonate focused producer')
  assert.throws(() => verify(dir, focused, expected, admit(run, artifact, [job], '123', '456')), 'focused manifest cannot become full verification')
  assert.throws(() => verify(dir, { ...focused, producer: { ...focused.producer, workflow_sha: 'f'.repeat(40) } }, expected, identity))
  assert.throws(() => verify(dir, focused, { ...expected, tree: 'f'.repeat(40) }, identity))
}))
test('focused admission rejects cross-run, attempts, refs, source and non-native callers', () => {
  for (const patch of [{ run: '124' }, { attempt: '1' }, { sha: 'f'.repeat(40) }, { head: 'f'.repeat(40) }, { tree: 'invalid' }, { workflowSha: 'f'.repeat(40) }, { workflow: focusedContext.workflow + '-other' }, { job: 'settings-build' }, { event: 'push' }, { kind: 'unknown' }]) assert.throws(() => admit(focusedRun, artifact, [focusedJob], '123', '456', { ...focusedContext, ...patch }))
  for (const patch of [{ status: 'completed', conclusion: 'success' }, { conclusion: 'failure' }, { head_sha: 'f'.repeat(40) }, { head_branch: 'main' }, { event: 'workflow_run' }]) assert.throws(() => admit({ ...focusedRun, ...patch }, artifact, [focusedJob], '123', '456', focusedContext))
  for (const jobs of [[job], [{ ...focusedJob, conclusion: 'failure' }], [{ ...focusedJob, run_attempt: 1 }]]) assert.throws(() => admit(focusedRun, artifact, jobs, '123', '456', focusedContext))
  assert.throws(() => admit(focusedRun, artifact, [focusedJob], '123', '456'), 'default main kind must still require final automatic main success')
})
