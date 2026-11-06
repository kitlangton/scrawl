import assert from 'node:assert/strict';
import { cpSync, existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';
import { parseArgs, prepare, versionAtLeast, withPreparationLock } from './prepare.ts';

test('defaults to the shared baseline and deduplicates explicit owners', () => {
  assert.deepEqual(parseArgs([]), { owners: [], help: false });
  assert.deepEqual(parseArgs(['--owner', 'agent', '--owner', 'agent']), { owners: ['agent'], help: false });
});

test('rejects unknown and incomplete owner arguments', () => {
  assert.throws(() => parseArgs(['--owner', 'everything']), /Unknown owner/);
  assert.throws(() => parseArgs(['--owner']), /Unknown owner/);
  assert.throws(() => parseArgs(['--install-all']), /Unknown argument/);
});

test('help does not enable broad install selection', () => {
  assert.deepEqual(parseArgs(['--help']), { owners: [], help: true });
});

test('toolchain comparison handles minimums and rejects older versions', () => {
  assert.equal(versionAtLeast('v22.23.4', '22.18.0'), true);
  assert.equal(versionAtLeast('22.6.0', '22.18.0'), false);
  assert.equal(versionAtLeast('3.9.0', '3.9'), true);
});

test('overlapping callers serialize and release their worktree lock', async () => {
  const root = mkdtempSync(join(tmpdir(), 'apparent-prepare-lock-'));
  try {
    const script = join(root, 'caller.ts');
    writeFileSync(script, `import { withPreparationLock } from ${JSON.stringify(new URL('./prepare.ts', import.meta.url).href)};
import { existsSync, writeFileSync, unlinkSync, appendFileSync } from 'node:fs';
withPreparationLock(${JSON.stringify(root)}, () => {
  const marker = ${JSON.stringify(join(root, 'active'))};
  if (existsSync(marker)) throw new Error('overlapping preparation');
  writeFileSync(marker, 'active');
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 150);
  appendFileSync(${JSON.stringify(join(root, 'completed'))}, 'done\\n');
  unlinkSync(marker);
});`);
    const caller = () => new Promise<void>((resolve, reject) => {
      const child = spawn(process.execPath, ['--experimental-strip-types', script], { stdio: 'inherit' });
      child.on('error', reject);
      child.on('exit', code => code === 0 ? resolve() : reject(new Error(`caller exit ${code}`)));
    });
    await Promise.all([caller(), caller()]);
    assert.equal(readFileSync(join(root, 'completed'), 'utf8'), 'done\ndone\n');
    assert.equal(existsSync(join(root, '.apparent-worktree-prepare.lock')), false);
    assert.throws(() => withPreparationLock(root, () => { throw new Error('owned failure'); }), /owned failure/);
    assert.equal(existsSync(join(root, '.apparent-worktree-prepare.lock')), false);
    mkdirSync(join(root, '.apparent-worktree-prepare.lock'));
    assert.throws(() => withPreparationLock(root, () => assert.fail('must not enter'), 0), /already locked/);
    assert.ok(existsSync(join(root, '.apparent-worktree-prepare.lock')), 'never remove a foreign lock');
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('cold isolated checkout prepares locked baseline and a warm rerun is idempotent', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'apparent-worktree-prepare-'));
  try {
    const repo = resolve('.');
    mkdirSync(join(fixture, 'composition/architecture'), { recursive: true });
    cpSync(join(repo, 'composition/architecture/package.json'), join(fixture, 'composition/architecture/package.json'));
    cpSync(join(repo, 'composition/architecture/package-lock.json'), join(fixture, 'composition/architecture/package-lock.json'));
    mkdirSync(join(fixture, 'enablement'));
    cpSync(join(repo, 'enablement/verification.py'), join(fixture, 'enablement/verification.py'));
    prepare(fixture, []);
    const lockBytes = readFileSync(join(fixture, 'composition/architecture/package-lock.json'));
    const installed = join(fixture, 'composition/architecture/node_modules/typescript/package.json');
    assert.ok(existsSync(installed), 'real TypeScript package was installed');
    const installedMtime = statSync(installed).mtimeMs;
    prepare(fixture, []);
    assert.ok(readFileSync(join(fixture, 'composition/architecture/package-lock.json')).equals(lockBytes), 'preparation preserves lockfile bytes');
    assert.equal(statSync(installed).mtimeMs, installedMtime, 'warm rerun must not reinstall the verified package');
    // Browser preflight only checks controls and Bun availability. A real missing
    // install must reach Bun, even when those checks already succeed.
    const bin = join(fixture, 'bin');
    mkdirSync(bin);
    const browser = join(fixture, 'connect/web');
    mkdirSync(browser, { recursive: true });
    writeFileSync(join(browser, 'package.json'), '{"type":"module"}');
    writeFileSync(join(browser, 'bun.lock'), '{}');
    writeFileSync(join(bin, 'bun'), `#!/bin/sh
if [ "$1" = "--version" ]; then echo 1.3.10; exit 0; fi
[ "$1" = "install" ] && [ "$2" = "--frozen-lockfile" ] && [ "$3" = "--prefer-offline" ] || exit 17
mkdir -p node_modules/vite
printf '%s' '{"name":"vite","type":"module","exports":"./index.js"}' > node_modules/vite/package.json
printf '%s' 'export const prepared = true;' > node_modules/vite/index.js
echo install >> installs
`, { mode: 0o700 });
    const oldPath = process.env.PATH;
    process.env.PATH = `${bin}:${oldPath}`;
    try {
      prepare(fixture, ['browser']);
      assert.equal(readFileSync(join(browser, 'installs'), 'utf8'), 'install\n');
    } finally { process.env.PATH = oldPath; }
    const staleStarted = performance.now();
    const staleManifest = JSON.parse(readFileSync(installed, 'utf8')) as { version: string };
    staleManifest.version = '0.0.0-stale-fixture';
    writeFileSync(installed, JSON.stringify(staleManifest));
    prepare(fixture, []);
    const expectedVersion = JSON.parse(lockBytes.toString('utf8')).packages['node_modules/typescript'].version as string;
    assert.equal(JSON.parse(readFileSync(installed, 'utf8')).version, expectedVersion, 'npm ci refreshes stale installed contents from the lock');
    console.log(`[measurement] stale refresh elapsed ${(performance.now() - staleStarted).toFixed(0)}ms`);
    const lock = join(fixture, 'composition/architecture/package-lock.json');
    writeFileSync(lock, '{invalid json');
    assert.throws(() => prepare(fixture, []), /failed with|invalid|EJSONPARSE/i, 'invalid lock must fail closed');
  } finally {
    rmSync(fixture, { recursive: true, force: true });
  }
});
