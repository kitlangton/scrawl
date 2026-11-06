import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { constants } from 'node:fs';
import { chmod, lstat, mkdir, mkdtemp, open, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { tmpdir } from 'node:os';
import { test } from 'node:test';
import { createInstalledNativeResources } from '../dist/installed.js';

const executable = process.env.APPARENT_RESOURCE_HOST_BIN;
assert(executable, 'Installed native proof requires the exact staged producer helper');

test('compiled installed provider preserves native flock/publication and its protected lifetime', { timeout: 20_000 }, async () => {
  const artifactDirectory = await realpath(dirname(executable));
  const manifestBytes = await readFile(join(artifactDirectory, 'manifest.json'));
  const manifest = JSON.parse(manifestBytes.toString('utf8'));
  const resources = await createInstalledNativeResources({ artifactDirectory,
    trustedManifestSha256: createHash('sha256').update(manifestBytes).digest('hex'),
    expectedSourceSha: manifest.sourceSha, target: manifest.target });
  const root = await realpath(await mkdtemp(join(tmpdir(), 'installed-resources-native-')));
  let lease: Awaited<ReturnType<typeof resources.lock>> | undefined;
  try {
    await chmod(root, 0o700);
    const lockPath = join(root, 'caller.lock');
    await writeFile(lockPath, '', { mode: 0o600 });
    const held = await open(lockPath, constants.O_RDWR | constants.O_NOFOLLOW);
    lease = await resources.lock(held, { timeoutMs: 0 });
    assert.equal(held.fd, -1); assert.equal(lease.state, 'acquired');
    await assert.rejects(resources.close(), { category: 'invalid_request' });
    assert.deepEqual(await resources.lock(await open(lockPath, constants.O_RDWR | constants.O_NOFOLLOW),
      { timeoutMs: 25 }), { state: 'busy' });
    assert.equal((await lstat(resources.executable)).mode & 0o777, 0o755);
    if (lease.state !== 'acquired') assert.fail('Native lease was not acquired');
    await lease.lease.release(); lease = undefined;
    await mkdir(join(root, 'staged'), { mode: 0o700 });
    await writeFile(join(root, 'staged/value'), 'synthetic-installed-resource', { mode: 0o600 });
    const parent = await open(root, constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW);
    assert.deepEqual(await resources.publish(parent, 'staged', 'published'), { state: 'published' });
    assert.equal(parent.fd, -1);
    assert.equal(await readFile(join(root, 'published/value'), 'utf8'), 'synthetic-installed-resource');
    await resources.close();
    await assert.rejects(lstat(resources.executable), { code: 'ENOENT' });
    const after = await open(lockPath, constants.O_RDWR | constants.O_NOFOLLOW);
    await assert.rejects(resources.lock(after, { timeoutMs: 0 }), { category: 'provider_unavailable' });
    assert.equal(after.fd, -1);
  } finally {
    if (lease?.state === 'acquired') await lease.lease.release();
    await resources.close(); await rm(root, { recursive: true, force: true });
  }
});
