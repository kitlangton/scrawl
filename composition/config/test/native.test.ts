import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, realpath, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createConfigBackend, createRecordPenColorConfig, type ConfigBackend } from '../src/client.ts';

// This suite is mandatory in the native producer; the normal TS loop never builds Rust.
const executable = process.env.APPARENT_CONFIG_HOST_BIN;
if (!executable) throw new Error('APPARENT_CONFIG_HOST_BIN must name the exact staged helper');
const executableSha256 = createHash('sha256').update(await readFile(executable)).digest('hex');
async function fixture() {
  const directory = await realpath(await mkdtemp(join(tmpdir(), 'config-native-client-')));
  const configPath = join(directory, 'config.json');
  const peers: ConfigBackend[] = [];
  return { directory, configPath,
    async peer(ownerId = 'onboarding') {
      const peer = await createConfigBackend({ executable: executable!, executableSha256, configPath, ownerId });
      peers.push(peer);
      return peer;
    },
    async close() { await Promise.all(peers.map(peer => peer.close())); await rm(directory, { recursive: true }); },
  };
}

test('public onboarding bootstrap retires the legacy writer atomically and existing-owner retirement is explicit', async () => {
  const f = await fixture();
  try {
    await writeFile(f.configPath, JSON.stringify({ onboarding: { resume: 'permissions' }, secret: 'synthetic' }), { mode: 0o600 });
    const backend = await f.peer();
    const snapshot = await backend.readForAdoption('onboarding');
    assert.deepEqual(await backend.adoptLegacy('onboarding', snapshot.legacy, { stage: 'permissions' }), { status: 'saved', revision: 1 });
    const saved = JSON.parse(await readFile(f.configPath, 'utf8'));
    assert.deepEqual(saved.onboardingLegacyWriter, { schema: 1, state: 'retired' });
    assert.equal(saved.secret, 'synthetic');
    assert.deepEqual(saved.onboarding, { resume: 'permissions' });
    assert.deepEqual(await backend.retireOnboardingLegacyWriter(), { status: 'retired' });
    const other = await f.peer('record');
    await assert.rejects(other.retireOnboardingLegacyWriter(), /onboarding owner/i);
  } finally { await f.close(); }
});

test('real helper CAS has one cross-process winner, preserves other owners and survives restart', async () => {
  const f = await fixture();
  try {
    await writeFile(f.configPath, JSON.stringify({ secret: 'synthetic-only', cloud: { enabled: false } }), { mode: 0o600 });
    const a = await f.peer();
    const b = await f.peer();
    assert.deepEqual(await a.read('onboarding'), { revision: 0, value: null });
    const results = await Promise.all([a.compareAndSwap('onboarding', 0, { stage: 'welcome' }), b.compareAndSwap('onboarding', 0, { stage: 'permission' })]);
    assert.equal(results.filter(result => result.status === 'saved').length, 1);
    const conflict = results.find(result => result.status === 'conflict');
    assert(conflict?.status === 'conflict');
    assert.equal(conflict.revision, 1);
    const expected = await a.read('onboarding');
    assert.deepEqual(conflict.current, expected.value);
    const agent = await f.peer('agent');
    assert.deepEqual(await agent.compareAndSwap('agent', 0, { verbose: false }), { status: 'saved', revision: 1 });
    await a.close();
    await b.close();
    const resumed = await f.peer();
    assert.deepEqual(await resumed.read('onboarding'), expected);
    const raw = JSON.parse(await readFile(f.configPath, 'utf8'));
    assert.equal(raw.secret, 'synthetic-only');
    assert.deepEqual(raw.cloud, { enabled: false });
    assert.equal((await stat(f.configPath)).mode & 0o777, 0o600);
  } finally { await f.close(); }
});

test('real helper rejects malformed existing config and never overwrites its bytes', async () => {
  for (const contents of ['{bad', '[]', '{"streamState":[]}']) {
    const f = await fixture();
    try {
      await writeFile(f.configPath, contents, { mode: 0o600 });
      const peer = await f.peer();
      assert.equal((await peer.compareAndSwap('onboarding', 0, {})).status, 'unknown');
      await assert.rejects(peer.read('onboarding'));
      assert.equal(await readFile(f.configPath, 'utf8'), contents);
    } finally { await f.close(); }
  }
});


test('real helper adopts owner-interpreted legacy progress once without losing original keys', async () => {
  const f = await fixture();
  try {
    const legacy = { seen: true, deferred: true, resume: 'permissions', granted: ['microphone'], relaunchAt: 123, reopenFix: true };
    await writeFile(f.configPath, JSON.stringify({ onboarding: legacy, secret: 'synthetic-only', streamState: { agent: { revision: 9, value: 'opaque' } } }), { mode: 0o600 });
    const left = await f.peer();
    const right = await f.peer();
    const snapshot = await left.readForAdoption('onboarding');
    assert.deepEqual(snapshot, { owner: { present: false, revision: 0, value: null }, legacy: { present: true, value: legacy } });
    const before = await readFile(f.configPath, 'utf8');
    await right.readForAdoption('onboarding');
    assert.equal(await readFile(f.configPath, 'utf8'), before);
    // These encoded owner values represent owner codec output, not host journey policy.
    const results = await Promise.all([
      left.adoptLegacy('onboarding', snapshot.legacy, { schema: 1, stage: 'permissions', imported: true }),
      right.adoptLegacy('onboarding', snapshot.legacy, { schema: 1, stage: 'welcome', imported: true }),
    ]);
    assert.equal(results.filter(result => result.status === 'saved').length, 1);
    const conflict = results.find(result => result.status === 'conflict');
    assert(conflict?.status === 'conflict');
    assert.deepEqual(conflict.snapshot, await left.readForAdoption('onboarding'));
    assert.equal(conflict.snapshot.owner.present, true);
    assert.equal(conflict.snapshot.owner.revision, 1);
    const raw = JSON.parse(await readFile(f.configPath, 'utf8'));
    assert.deepEqual(raw.onboarding, legacy);
    assert.equal(raw.secret, 'synthetic-only');
    assert.deepEqual(raw.streamState.agent, { revision: 9, value: 'opaque' });
    await left.close();
    const resumed = await f.peer();
    assert.deepEqual(await resumed.readForAdoption('onboarding'), conflict.snapshot);
    assert.equal((await resumed.adoptLegacy('onboarding', snapshot.legacy, null)).status, 'conflict');
  } finally { await f.close(); }
});

test('staged native helper performs Record-only penColor presence CAS with fresh concurrent conflicts', async () => {
  const f = await fixture();
  try {
    await writeFile(f.configPath, JSON.stringify({ secret: 'synthetic-only', streamState: { agent: { revision: 8, value: { keep: true } } } }), { mode: 0o600 });
    const options = { executable: executable!, executableSha256, configPath: f.configPath };
    const left = await createRecordPenColorConfig(options);
    const right = await createRecordPenColorConfig(options);
    try {
      const absent = { present: false, value: null } as const;
      assert.deepEqual(await left.read(), absent);
      const results = await Promise.all([
        left.compareAndSwap(absent, { present: true, value: '#123456' }),
        right.compareAndSwap(absent, { present: true, value: '#abcdef' }),
      ]);
      assert.equal(results.filter(result => result.status === 'saved').length, 1);
      const conflict = results.find(result => result.status === 'conflict');
      assert(conflict?.status === 'conflict');
      assert.deepEqual(conflict.current, await left.read());
      const current = await left.read();
      assert.deepEqual(await left.compareAndSwap(current, { present: true, value: null }), { status: 'saved' });
      assert.deepEqual(await left.read(), { present: true, value: null });
      assert.deepEqual(await left.compareAndSwap({ present: true, value: null }, absent), { status: 'saved' });
      assert.deepEqual(await left.read(), absent);
    } finally { await Promise.all([left.close(), right.close()]); }
    const raw = JSON.parse(await readFile(f.configPath, 'utf8'));
    assert.equal(Object.hasOwn(raw, 'penColor'), false);
    assert.equal(raw.secret, 'synthetic-only');
    assert.deepEqual(raw.streamState.agent, { revision: 8, value: { keep: true } });
  } finally { await f.close(); }
});
