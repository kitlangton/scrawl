import assert from 'node:assert/strict';
import { test } from 'node:test';
import { chmod, mkdtemp, readFile, rm, stat, symlink } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createConfigBackend, createRecordPenColorConfig } from '../src/client.ts';

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


test('adoption stays owner-bound and validates presence, conflict and one-time revision', async () => {
  for (const mode of ['ok', 'conflict', 'invalid-adoption']) {
    const backend = await createConfigBackend(await options(mode));
    try {
      const snapshot = await backend.readForAdoption('onboarding');
      assert.deepEqual(snapshot, { owner: { present: false, revision: 0, value: null }, legacy: { present: true, value: { seen: true } } });
      await assert.rejects(backend.readForAdoption('agent'), /owner/i);
      await assert.rejects(backend.adoptLegacy('agent', snapshot.legacy, {}), /owner/i);
      await assert.rejects(backend.adoptLegacy('onboarding', { present: false, value: {} }, {}), /snapshot/i);
      await assert.rejects(backend.adoptLegacy('onboarding', snapshot.legacy, { invalid: Infinity }), /JSON/i);
      const result = await backend.adoptLegacy('onboarding', snapshot.legacy, { stage: 'resume' });
      if (mode === 'ok') assert.deepEqual(result, { status: 'saved', revision: 1 });
      if (mode === 'conflict') assert.deepEqual(result, { status: 'conflict', snapshot: { owner: { present: true, revision: 2, value: null }, legacy: { present: false, value: null } } });
      if (mode === 'invalid-adoption') {
        assert.equal(result.status, 'unknown');
        await assert.rejects(backend.readForAdoption('onboarding'), /unavailable/i);
      }
    } finally { await backend.close(); }
  }
});

test('adoption loses no safety on unknown publication or transport loss', async () => {
  for (const mode of ['unknown', 'exit', 'wrong-id', 'oversized']) {
    const backend = await createConfigBackend(await options(mode));
    try {
      assert.equal((await backend.adoptLegacy('onboarding', { present: false, value: null }, {})).status, 'unknown');
      if (mode === 'unknown') assert.deepEqual(await backend.read('onboarding'), { revision: 0, value: null });
      else await assert.rejects(backend.read('onboarding'), /unavailable/i);
    } finally { await backend.close(); }
  }
});

test('Record facade fixes owner and key, preserves absent versus null, and reports fresh CAS conflict', async () => {
  const base = await options();
  const { ownerId: _ownerId, ...recordOptions } = base;
  const record = await createRecordPenColorConfig(recordOptions);
  try {
    assert.deepEqual(await record.read(), { present: true, value: null });
    assert.deepEqual(await record.compareAndSwap({ present: true, value: null }, { present: true, value: 'blue' }), { status: 'saved' });
  } finally { await record.close(); }

  const conflictOptions = await options('conflict');
  const { ownerId: _conflictOwner, ...conflictRecordOptions } = conflictOptions;
  const conflictRecord = await createRecordPenColorConfig(conflictRecordOptions);
  try {
    assert.deepEqual(await conflictRecord.compareAndSwap({ present: false, value: null }, { present: false, value: null }), { status: 'conflict', current: { present: true, value: null } });
  } finally { await conflictRecord.close(); }

  const onboarding = await createConfigBackend(base);
  try { await assert.rejects(onboarding.readPenColor(), /record owner/i); }
  finally { await onboarding.close(); }

  const unknown = await createRecordPenColorConfig({ ...recordOptions, configPath: join(tmpdir(), 'unknown.json') });
  try {
    assert.equal((await unknown.compareAndSwap({ present: false, value: null }, { present: true, value: 'red' })).status, 'unknown');
  } finally { await unknown.close(); }
});
