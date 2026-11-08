// Actual Electron Node runtime -> production Rust listener/controller/client.
// Only the HTTP device endpoint is a labelled local pending-approval fixture.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const http = require('node:http');
const net = require('node:net');
const path = require('node:path');
const { ConnectSettings } = require('../electron/connect.cjs');
const [home, config] = process.argv.slice(2);
const address = path.join(home, '.config/apparent/agent-control/capture.sock');
const state = path.join(path.dirname(config), 'hosted');
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(read, accept, label) {
  const end = performance.now() + 5000;
  let last;
  while (performance.now() < end) { last = await read(); if (accept(last)) return last; await wait(30); }
  throw new Error(`${label}: ${JSON.stringify(last)}`);
}
async function cancellation() { return fs.readFile(path.join(state, 'cancellation'), 'utf8').catch(() => null); }
async function revoked(previous) {
  const raw = JSON.parse(await fs.readFile(config, 'utf8'));
  const files = await fs.readdir(state).catch(() => []);
  return (await cancellation()) !== previous && raw.cloud.enabled === false && raw.cloud.uploadConsent === false &&
    // An absent experiment is off; disable need not materialize a false key.
    raw.experiments.hostedSync !== true && !files.some(name => name.endsWith('.intent'));
}
async function raw(request, command) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(address);
    const timer = setTimeout(() => { socket.destroy(); reject(new Error('Raw request deadline')); }, 3500);
    let bytes = '', first = true;
    socket.on('error', reject);
    socket.on('connect', () => socket.write(JSON.stringify(request) + '\n'));
    socket.on('data', chunk => {
      bytes += chunk;
      if (!bytes.endsWith('\n')) return;
      const value = JSON.parse(bytes); bytes = '';
      if (first && command && value.connect) { first = false; socket.write(JSON.stringify(command) + '\n'); return; }
      clearTimeout(timer); socket.destroy(); resolve(value);
    });
  });
}
async function main() {
  const calls = [];
  const clients = new Set();
  let client;
  let origin;
  const server = http.createServer((request, response) => {
    calls.push(request.url);
    request.resume();
    response.setHeader('Content-Type', 'application/json');
    if (request.url === '/v1/device/start') return response.end(JSON.stringify({
      deviceCode: 'synthetic-private-device-secret', userCode: 'TEST-CODE', expiresIn: 60, interval: 1,
      verificationUri: origin + '/hosted/device?code=TEST-CODE',
    }));
    if (request.url === '/v1/device/poll') return response.end('{"status":"pending"}');
    response.writeHead(500); response.end('{"error":"unexpected_test_endpoint"}');
  });
  server.on('connection', socket => { clients.add(socket); socket.on('close', () => clients.delete(socket)); });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  origin = `http://127.0.0.1:${server.address().port}`;
  try {
    client = new ConnectSettings(home);
    assert.equal((await client.request()).status, 'connected');
    await client.request('allow'); // Default availability never supplies consent.
    assert.equal(calls.length, 0);
    await assert.rejects(fs.stat(config), { code: 'ENOENT' });
    await fs.writeFile(config, JSON.stringify({ experiments: { connectSettings: false } }), { mode: 0o600 });
    await until(() => client.request(), r => r.status === 'disabled', 'Explicit opt-out did not hide Connect');
    assert.equal(calls.length, 0);
    const original = { experiments: { unrelated: true }, cloud: { server: origin, privateNested: { keep: true } }, privateFixture: { keep: 'synthetic' } };
    await fs.writeFile(config, JSON.stringify(original), { mode: 0o600 });
    await until(() => client.request(), r => r.status === 'connected', 'Actual Rust opening did not become ready');
    const preserved = await fs.readFile(config, 'utf8');
    await client.request();
    await client.request('allow'); // Forged allow without preceding consent is inert.
    assert.equal(calls.length, 0);
    assert.equal(await fs.readFile(config, 'utf8'), preserved);
    for (const ending of ['cancel', 'close']) {
      const consent = await client.request('connect');
      assert.ok(consent.view.buttons.some(button => button.command === 'allow'));
      const beforeNetwork = calls.length;
      await client.request('allow');
      const approval = await until(() => client.request(), r => r.view?.userCode === 'TEST-CODE', 'Actual production client public approval missing');
      assert.equal(approval.view.verificationUri, origin + '/hosted/device?code=TEST-CODE');
      assert.ok(calls.length > beforeNetwork);
      assert.equal(JSON.stringify(approval).includes('synthetic-private-device-secret'), false);
      const previous = await cancellation();
      if (ending === 'cancel') {
        await client.request('cancel');
        await until(() => client.request(), r => r.view?.buttons.some(b => b.command === 'connect' && b.enabled), 'Cancel did not return to idle');
      } else client.close();
      await until(() => revoked(previous), Boolean, `${ending} did not durably revoke`);
      await wait(1200); // Cross one actual production device-poll/cancellation interval.
      const files = await fs.readdir(state);
      assert.equal(files.some(name => name.endsWith('.credential')), false);
      const saved = JSON.parse(await fs.readFile(config, 'utf8'));
      assert.deepEqual(saved.privateFixture, original.privateFixture);
      assert.deepEqual(saved.cloud.privateNested, original.cloud.privateNested);
      if (ending === 'close') await until(() => client.request(), r => r.status === 'connected', 'Closed stream did not reopen locally');
    }
    client.close(); await wait(80);
    assert.equal((await raw({ connect: 'settings', deadline: 1 })).error, 'admission_expired');
    assert.equal((await raw({ connect: 'settings', deadline: Number(process.hrtime.bigint()/1000000n)+3000, secret: 'synthetic' })).error, 'invalid_request');
    const count = calls.length;
    const expired = await raw({ connect: 'settings', deadline: Number(process.hrtime.bigint()/1000000n)+3000 },
      { connect: 'command', command: 'allow', deadline: 1 });
    assert.equal(expired.error, 'admission_expired');
    assert.equal(calls.length, count);
    assert.ok(calls.every(url => ['/v1/device/start', '/v1/device/poll'].includes(url)));
    console.log(JSON.stringify({ platform: process.platform, electron: process.versions.electron,
      bridge: 'real Rust listener/controller/production client', http: 'synthetic loopback pending-device fixture, no account approval',
      checks: 'disabled/local-only/no-consent/public-projection/cancel/EOF/durable-off/preserved-config/expired-frame', requests: calls.length }));
  } catch (error) {
    console.error(JSON.stringify({ fixtureEndpointsObserved: calls }));
    throw error;
  } finally {
    client?.close(); for (const socket of clients) socket.destroy();
    await new Promise(resolve => server.close(resolve));
  }
}
const watchdog = setTimeout(() => { console.error('Actual Rust Connect integration deadline'); process.exit(1); }, 20000);
main().catch(error => { console.error(error); process.exitCode = 1; }).finally(() => clearTimeout(watchdog));
