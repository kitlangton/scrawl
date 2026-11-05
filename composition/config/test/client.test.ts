import assert from 'node:assert/strict';
import { test } from 'node:test';
import { chmod, mkdtemp, readFile, rm, stat, symlink } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createConfigBackend } from '../src/client.ts';

const peer = fileURLToPath(new URL('./peer.ts', import.meta.url));
async function options(mode = 'ok') {
  await chmod(peer, 0o755);
  return { executable: peer, executableSha256: createHash('sha256').update(await readFile(peer)).digest('hex'),
    configPath: join(tmpdir(), `${mode}.json`), ownerId: 'onboarding', timeoutMs: 1_000 };
}

test('binds one owner, validates requests before dispatch, and closes deterministically', async () => {
  const backend = await createConfigBackend(await options());
  try {
    assert.deepEqual(await backend.read('onboarding'), { revision: 0, value: null });
    await assert.rejects(backend.read('agent'), /owner/i);
    await assert.rejects(backend.compareAndSwap('onboarding', -1, {}), /revision/i);
    await assert.rejects(backend.compareAndSwap('onboarding', 0, { bad: Number.NaN }), /JSON/i);
    assert.deepEqual(await backend.compareAndSwap('onboarding', 0, {}), { status: 'saved', revision: 1 });
  } finally { await backend.close(); }
  await backend.close();
  await assert.rejects(backend.read('onboarding'), /unavailable/i);
});

test('transport loss, timeout and malformed replies never become a saved write', async () => {
  for (const mode of ['hang', 'exit', 'malformed', 'oversized', 'wrong-id']) {
    const backend = await createConfigBackend(await options(mode));
    try {
      assert.deepEqual(await backend.compareAndSwap('onboarding', 0, { stage: 'welcome' }),
        { status: 'unknown', message: 'Configuration outcome unknown; reload before retrying' }, mode);
      await assert.rejects(backend.read('onboarding'), /unavailable/i);
    } finally { await backend.close(); }
  }
});

test('explicit publication uncertainty preserves the transport for reconciliation', async () => {
  const backend = await createConfigBackend(await options('unknown'));
  try {
    assert.equal((await backend.compareAndSwap('onboarding', 0, {})).status, 'unknown');
    assert.deepEqual(await backend.read('onboarding'), { revision: 0, value: null });
  } finally { await backend.close(); }
});

test('runs verified bytes in a private directory and removes only that staged executable', async () => {
  const backend = await createConfigBackend(await options('identity'));
  let staged: string;
  try {
    const { value } = await backend.read('onboarding');
    assert.equal(typeof value, 'string');
    staged = value as string;
    assert.notEqual(staged, peer);
    assert.equal((await stat(dirname(staged))).mode & 0o777, 0o700);
    assert.deepEqual(await readFile(staged), await readFile(peer));
  } finally { await backend.close(); }
  await assert.rejects(stat(staged!), { code: 'ENOENT' });
  assert((await stat(peer)).isFile());
});

test('rejects unverified executables and unsafe caller binding before launch', async () => {
  const base = await options();
  await assert.rejects(createConfigBackend({ ...base, executableSha256: '0'.repeat(64) }), /artifact/i);
  await assert.rejects(createConfigBackend({ ...base, executable: 'node' }), /absolute/i);
  await assert.rejects(createConfigBackend({ ...base, executable: '/dev/zero' }), /artifact/i);
  await assert.rejects(createConfigBackend({ ...base, ownerId: '../onboarding' }), /owner/i);
  await assert.rejects(createConfigBackend({ ...base, configPath: 'config.json' }), /absolute/i);
  const root = await mkdtemp(join(tmpdir(), 'config-peer-link-'));
  try {
    const alias = join(root, 'peer');
    await symlink(peer, alias);
    await assert.rejects(createConfigBackend({ ...base, executable: alias }), /artifact/i);
  } finally { await rm(root, { recursive: true }); }
});
