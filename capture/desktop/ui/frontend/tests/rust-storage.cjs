// Invoked by the ignored Rust integration test with Electron in Node mode.
// Neither the Rust endpoint nor its clock/config policy is mocked.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const net = require('node:net');
const path = require('node:path');
const { captureLocation } = require('../electron/runtime.cjs');
const [home, config] = process.argv.slice(2);
async function main() {
  const selected = path.join(home, 'selected é captures');
  const enabled = { experiments: { electronCaptureLocation: true }, privateFixture: 'keep' };
  assert.deepEqual(await captureLocation(home), { status: 'disabled' });
  await fs.writeFile(config, JSON.stringify(enabled));
  const first = await captureLocation(home);
  assert.equal(first.status, 'connected');
  assert.equal(first.storage.source, 'builtin');
  assert.equal(first.storage.privateFixture, undefined);
  const saved = await captureLocation(home, 'set', { path: selected, expected: first.storage.revision });
  assert.equal(saved.status, 'connected');
  assert.equal(saved.storage.savedPath, selected);
  const raw = JSON.parse(await fs.readFile(config, 'utf8'));
  assert.equal(raw.privateFixture, 'keep');
  assert.equal(raw.capturesDir, selected);
  assert.deepEqual(await captureLocation(home, 'reset', { expected: first.storage.revision }), { status: 'error', code: 'conflict' });
  const expired = await new Promise((resolve, reject) => {
    const socket = net.createConnection(path.join(home, '.config/apparent/agent-control/capture.sock'));
    const timer = setTimeout(() => { socket.destroy(); reject(new Error('expired request stalled')); }, 4000);
    socket.on('error', reject);
    socket.once('connect', () => socket.write(JSON.stringify({ storage: 'reset', expected: saved.storage.revision,
      deadline: Number(process.hrtime.bigint() / 1000000n) - 1 }) + '\n'));
    let body = '';
    socket.on('data', data => { body += data; });
    socket.on('end', () => { clearTimeout(timer); resolve(JSON.parse(body)); });
  });
  assert.deepEqual(expired, { error: 'admission_expired' });
  assert.equal(JSON.parse(await fs.readFile(config, 'utf8')).capturesDir, selected);
  const reset = await captureLocation(home, 'reset', { expected: saved.storage.revision });
  assert.equal(reset.status, 'connected');
  assert.equal(reset.storage.savedPath, null);
  assert.equal(Object.hasOwn(JSON.parse(await fs.readFile(config, 'utf8')), 'capturesDir'), false);
  await fs.writeFile(config, 'malformed');
  assert.deepEqual(await captureLocation(home), { status: 'error', code: 'config_read_failed' });
  assert.deepEqual(await captureLocation(home, 'reset', { expected: reset.storage.revision }), { status: 'error', code: 'config_read_failed' });
  assert.equal(await fs.readFile(config, 'utf8'), 'malformed');
  console.log(JSON.stringify({ platform: process.platform, electron: process.versions.electron,
    bridge: 'real Rust socket', clock: 'actual Electron hrtime', checks: 'read/set/conflict/expired/reset/malformed' }));
}
const watchdog = setTimeout(() => { console.error('Rust/Electron integration deadline'); process.exit(1); }, 20000);
main().catch(error => { console.error(error); process.exitCode = 1; }).finally(() => clearTimeout(watchdog));
