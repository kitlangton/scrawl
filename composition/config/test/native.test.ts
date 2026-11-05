import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, realpath, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createConfigBackend, type ConfigBackend } from '../src/client.ts';

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
