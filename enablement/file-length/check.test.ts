import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'

const source = fileURLToPath(new URL('../../', import.meta.url))
const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('GIT_')))
env.GIT_CONFIG_NOSYSTEM = '1'
env.GIT_CONFIG_GLOBAL = '/dev/null'

function fixture(t: test.TestContext) {
  const root = mkdtempSync(join(tmpdir(), 'file-length-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const git = (...args: string[]) => spawnSync('git', args, { cwd: root, env, encoding: 'utf8' })
  const ok = (...args: string[]) => {
    const result = git(...args)
    assert.equal(result.status, 0, result.stderr)
    return result.stdout
  }
  ok('init', '-q')
  ok('config', 'user.name', 'Hook Test')
  ok('config', 'user.email', 'hook@example.invalid')
  ok('config', 'core.hooksPath', '.githooks')
  for (const path of ['.githooks/pre-commit', 'enablement/verification.py', 'enablement/directory-width/check.py', 'enablement/file-length/check.ts']) {
    mkdirSync(dirname(join(root, path)), { recursive: true })
    copyFileSync(join(source, path), join(root, path))
  }
  const put = (path: string, content: string | Buffer) => {
    mkdirSync(dirname(join(root, path)), { recursive: true })
    writeFileSync(join(root, path), content)
  }
  const hook = () => spawnSync('sh', ['.githooks/pre-commit'], { cwd: root, env, encoding: 'utf8' })
  const pass = () => { const result = hook(); assert.equal(result.status, 0, result.stderr) }
  const reject = (path: string) => {
    const result = hook()
    assert.equal(result.status, 1, result.stderr)
    assert.ok(result.stderr.includes(JSON.stringify(path)), result.stderr)
    assert.match(result.stderr, /2001 lines \(limit 2000\)/)
  }
  const seed = () => ok('-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'fixture')
  return { root, git, ok, put, hook, pass, reject, seed }
}

test('initial commit: 2000 physical lines allowed, 2001 blocked; no index/worktree mutation', t => {
  const f = fixture(t)
  f.put('src/a.ts', '\n'.repeat(2000))
  f.ok('add', 'src/a.ts')
  f.pass()
  f.put('src/a.ts', '\n'.repeat(2001))
  f.ok('add', 'src/a.ts')
  const tree = f.ok('write-tree')
  f.reject('src/a.ts')
  assert.equal(f.ok('write-tree'), tree)
  assert.equal(readFileSync(join(f.root, 'src/a.ts'), 'utf8'), '\n'.repeat(2001))
})

test('unterminated last line counts, empty file does not', t => {
  const f = fixture(t)
  f.put('src/empty', '')
  f.put('src/a.ts', 'a\n'.repeat(1999) + 'a')
  f.ok('add', 'src')
  f.pass()
  f.put('src/a.ts', 'a\n'.repeat(2000) + 'a')
  f.ok('add', 'src/a.ts')
  f.reject('src/a.ts')
})

test('partial staging uses index in both directions', t => {
  const f = fixture(t)
  f.put('src/a.ts', 'a\n'.repeat(2001))
  f.ok('add', 'src/a.ts')
  f.put('src/a.ts', 'short\n')
  f.reject('src/a.ts')
  f.ok('add', 'src/a.ts')
  f.put('src/a.ts', 'a\n'.repeat(2001))
  f.pass()
})

test('untouched oversized files and deletion pass; modified oversized files fail even when shrinking', t => {
  const f = fixture(t)
  f.put('src/old.ts', 'a\n'.repeat(2002))
  f.ok('add', 'src/old.ts')
  f.seed()
  f.put('src/new.ts', 'new\n')
  f.ok('add', 'src/new.ts')
  f.pass()
  f.put('src/old.ts', 'a\n'.repeat(2001))
  f.ok('add', 'src/old.ts')
  f.reject('src/old.ts')
  f.ok('rm', '-f', 'src/old.ts')
  f.pass()
})

test('rename destination and filenames with whitespace, Unicode and newlines are checked', t => {
  const f = fixture(t)
  f.put('src/old.ts', 'a\n'.repeat(2001))
  f.ok('add', 'src/old.ts')
  f.seed()
  const dest = 'src/renamed ü\n file.ts'
  f.ok('mv', 'src/old.ts', dest)
  f.reject(dest)
})

test('lockfiles, binary blobs, symlinks, known generated and dependency paths are exempt', t => {
  const f = fixture(t)
  const large = 'a\n'.repeat(2001)
  for (const path of ['locks/Cargo.lock', 'locks/package-lock.json', 'locks/npm-shrinkwrap.json', 'locks/yarn.lock', 'locks/bun.lock', 'locks/bun.lockb', 'node_modules/pkg/file.ts', 'public/site/video-demo/output/file.txt', 'connect/web/dist/file.txt', 'connect/web/dist-site-check/file.txt']) f.put(path, large)
  f.put('assets/image.bin', Buffer.concat([Buffer.from([0]), Buffer.from(large)]))
  symlinkSync('absent', join(f.root, 'assets/link'))
  f.ok('add', '.')
  f.pass()
})

test('pnpm lockfile is exempt', t => {
  const f = fixture(t)
  f.put('pnpm-lock.yaml', 'a\n'.repeat(2001))
  f.ok('add', 'pnpm-lock.yaml')
  f.pass()
})

test('ordinary JSON, docs, SVG, extensionless and generated-looking first-party files are checked', t => {
  for (const name of ['data.json', 'notes.md', 'icon.svg', 'script', 'generated.ts', 'dist/custom.ts']) {
    const f = fixture(t)
    const path = `src/${name}`
    f.put(path, 'a\n'.repeat(2001))
    f.ok('add', path)
    f.reject(path)
  }
})

test('unmerged index fails closed', t => {
  const f = fixture(t)
  const oid = f.ok('hash-object', '-w', '--stdin').trim()
  const result = spawnSync('git', ['update-index', '--index-info'], { cwd: f.root, env, encoding: 'utf8', input: `100644 ${oid} 1\tsrc/conflict.ts\n100644 ${oid} 2\tsrc/conflict.ts\n100644 ${oid} 3\tsrc/conflict.ts\n` })
  assert.equal(result.status, 0, result.stderr)
  assert.notEqual(f.hook().status, 0)
})

test('actual git commit invokes the guard', t => {
  const f = fixture(t)
  f.put('src/a.ts', 'a\n'.repeat(2001))
  f.ok('add', 'src/a.ts')
  // copyFile preserves the executable mode of the installed hook.
  const blocked = f.git('commit', '-qm', 'blocked')
  assert.notEqual(blocked.status, 0)
  assert.match(blocked.stderr, /2001 lines/)
})
