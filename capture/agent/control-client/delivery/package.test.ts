import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { chmodSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import { pack, stage } from './package.ts';

const hash = (bytes: Buffer | string) => createHash('sha256').update(bytes).digest('hex');
const run = (command: string, args: string[]) => {
  const result = spawnSync(command, args, { encoding: 'utf8' });
  assert.equal(result.status, 0, `${command}: ${result.stderr}`);
};

test('real standalone artifact roundtrip and fail-closed staging', () => {
  const executable = process.env.APPARENT_CAPTURE_CLIENT;
  assert(executable, 'APPARENT_CAPTURE_CLIENT must name the exact standalone binary built from this clean source');
  const head = spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' });
  assert.equal(head.status, 0);
  const source = head.stdout.trim();
  const target = `${process.arch === 'x64' ? 'x86_64' : 'aarch64'}-${process.platform === 'darwin' ? 'apple-darwin' : 'unknown-linux-gnu'}`;
  const root = mkdtempSync(join(tmpdir(), 'capture-client-proof-'));
  const previousMask = process.umask(0o077);
  try {
    chmodSync(root, 0o700);
    const produced = pack(executable, source, target, join(root, 'producer'));
    const destination = join(root, 'package');
    const manifest = stage(produced.archive, produced.digest, source, target, destination);
    assert.equal(manifest.sourceSha, source);
    assert.equal(manifest.protocol, 1);
    assert.equal(hash(readFileSync(join(destination, 'apparent-capture'))), manifest.executableSha256);
    assert.equal(lstatSync(join(destination, 'apparent-capture')).mode & 0o777, 0o755);
    assert.equal(lstatSync(join(destination, 'manifest.json')).mode & 0o777, 0o644);
    assert.deepEqual(requireNames(destination), ['apparent-capture', 'manifest.json']);

    const retained = join(root, 'retained'); mkdirSync(retained); writeFileSync(join(retained, 'sentinel'), 'keep');
    assert.throws(() => stage(produced.archive, produced.digest, source, target, retained), /already exists/);
    assert.equal(readFileSync(join(retained, 'sentinel'), 'utf8'), 'keep');
    assert.throws(() => stage(produced.archive, '0'.repeat(64), source, target, join(root, 'bad-digest')), /archive digest mismatch/);
    const wrongTarget = target === 'x86_64-unknown-linux-gnu' ? 'aarch64-unknown-linux-gnu' : 'x86_64-unknown-linux-gnu';
    assert.throws(() => stage(produced.archive, produced.digest, source, wrongTarget, join(root, 'wrong-target')),
      /target mismatch|manifest/);

    const tamper = join(root, 'tamper'); mkdirSync(tamper);
    const executableBytes = readFileSync(executable);
    writeFileSync(join(tamper, 'apparent-capture'), executableBytes);
    const changedManifest = { ...manifest, protocol: 2 };
    writeFileSync(join(tamper, 'manifest.json'), JSON.stringify(changedManifest));
    const tamperedArchive = join(root, 'tampered.tar.gz');
    run('tar', ['-czf', tamperedArchive, '-C', tamper, 'apparent-capture', 'manifest.json']);
    assert.throws(() => stage(tamperedArchive, hash(readFileSync(tamperedArchive)), source, target, join(root, 'bad-protocol')), /manifest/);

    writeFileSync(join(tamper, 'manifest.json'), JSON.stringify({ ...manifest, sourceSha: '0'.repeat(40) }));
    const changedSourceArchive = join(root, 'changed-source.tar.gz');
    run('tar', ['-czf', changedSourceArchive, '-C', tamper, 'apparent-capture', 'manifest.json']);
    assert.throws(() => stage(changedSourceArchive, hash(readFileSync(changedSourceArchive)), source, target, join(root, 'bad-source')), /source identity mismatch/);

    const linked = join(root, 'linked'); mkdirSync(linked);
    writeFileSync(join(linked, 'manifest.json'), JSON.stringify(manifest));
    symlinkSync(executable, join(linked, 'apparent-capture'));
    const linkedArchive = join(root, 'linked.tar.gz');
    run('tar', ['-czf', linkedArchive, '-C', linked, 'apparent-capture', 'manifest.json']);
    assert.throws(() => stage(linkedArchive, hash(readFileSync(linkedArchive)), source, target, join(root, 'bad-link')), /regular files/);
  } finally {
    process.umask(previousMask);
    rmSync(root, { recursive: true, force: true });
  }
});

function requireNames(path: string) {
  return ['apparent-capture', 'manifest.json'].filter(name => {
    try { lstatSync(join(path, name)); return true; } catch { return false; }
  }).sort();
}
