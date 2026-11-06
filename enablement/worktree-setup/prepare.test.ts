import assert from 'node:assert/strict';
import { cpSync, existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';
import { parseArgs, prepare, versionAtLeast } from './prepare.ts';

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
