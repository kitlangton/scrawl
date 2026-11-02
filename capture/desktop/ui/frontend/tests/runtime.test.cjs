const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const net = require('node:net');
const os = require('node:os');
const path = require('node:path');
const { readRuntimeState } = require('../electron/runtime.cjs');

async function fixture(t, reply, { mode = 0o700 } = {}) {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'ui-'));
  const directory = path.join(home, '.config/apparent/agent-control');
  await fs.mkdir(directory, { recursive: true, mode });
  await fs.chmod(directory, mode);
  const requests = [];
  const clients = new Set();
  const server = net.createServer((socket) => {
    clients.add(socket);
    socket.on('close', () => clients.delete(socket));
    socket.on('error', () => {});
    socket.once('data', (request) => {
      requests.push(request.toString());
      if (reply !== null) socket.end(reply);
    });
  });
  const address = path.join(directory, 'capture.sock');
  await new Promise((resolve) => server.listen(address, resolve));
  await fs.chmod(address, 0o600);
  t.after(async () => {
    for (const client of clients) client.destroy();
    await new Promise((resolve) => server.close(resolve));
    await fs.rm(home, { recursive: true, force: true });
  });
  return { home, directory, address, requests };
}

test('reads only state and strips capture identity and unexpected data', async (t) => {
  const f = await fixture(t, '{"state":{"phase":"recording","capture_id":"private-id"},"secret":"hidden"}\n');
  assert.deepEqual(await readRuntimeState(f.home), { status: 'connected', phase: 'recording' });
  assert.deepEqual(f.requests, ['state\n']);
});

test('missing GUI is disconnected and creates no config directory', async (t) => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'ui-'));
  t.after(() => fs.rm(home, { recursive: true, force: true }));
  assert.deepEqual(await readRuntimeState(home), { status: 'disconnected' });
  await assert.rejects(fs.stat(path.join(home, '.config')), { code: 'ENOENT' });
});

test('rejects public directories before contacting the socket', async (t) => {
  const f = await fixture(t, '{}\n', { mode: 0o755 });
  assert.equal((await readRuntimeState(f.home)).status, 'error');
  assert.deepEqual(f.requests, []);
});

test('rejects symlinked sockets and unsafe socket modes without a request', async (t) => {
  const f = await fixture(t, '{}\n');
  await fs.chmod(f.address, 0o666);
  assert.equal((await readRuntimeState(f.home)).status, 'error');
  await fs.rename(f.address, f.address + '.real');
  await fs.symlink(f.address + '.real', f.address);
  assert.equal((await readRuntimeState(f.home)).status, 'error');
  assert.deepEqual(f.requests, []);
});

for (const reply of ['not json\n', '{}\n', '{"state":{"phase":"new_phase"}}\n', '{"error":"app_busy"}\n', 'x'.repeat(4097), '{"state":{"phase":"idle"}}']) {
  test(`invalid, incomplete or oversized reply fails closed (${reply.slice(0, 22)})`, async (t) => {
    const f = await fixture(t, reply);
    assert.equal((await readRuntimeState(f.home)).status, 'error');
  });
}

test('absolute deadline bounds an unresponsive server', async (t) => {
  const f = await fixture(t, null);
  const before = performance.now();
  assert.equal((await readRuntimeState(f.home, 50)).status, 'error');
  assert.ok(performance.now() - before < 1000);
  assert.deepEqual(f.requests, ['state\n']);
});
