// Invoked by the ignored Rust integration test with Electron in Node mode.
// Neither the Rust endpoint nor its clock/config policy is mocked.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const net = require('node:net');
const path = require('node:path');
const { captureLocation, storageLocation, errorReporting, storagePreference } = require('../electron/runtime.cjs');
const { locationCandidate, resolveLocationCandidate } = require('../electron/storage-policy.cjs');
const [home, config] = process.argv.slice(2);
async function readLocation() {
  const facts = await storageLocation(home, 'facts');
  if (facts.status !== 'connected') return facts;
  const candidate = locationCandidate(facts.location);
  if (candidate.snapshot) return { status: 'connected', revision: candidate.revision, storage: candidate.snapshot };
  const probe = await storageLocation(home, 'probe', { path: candidate.probePath, expected: candidate.revision });
  if (probe.status !== 'connected') return probe;
  return { status: 'connected', revision: candidate.revision,
    storage: resolveLocationCandidate(facts.location, probe.available) };
}
async function main() {
  const selected = path.join(home, 'selected é captures');
  const enabled = { experiments: { electronCaptureLocation: true }, privateFixture: 'keep' };
  // Storage now defaults on; exercise the explicit opt-out before enabling it.
  assert.equal((await captureLocation(home)).status, 'connected');
  await fs.writeFile(config, JSON.stringify({ experiments: { electronCaptureLocation: false } }));
  assert.deepEqual(await captureLocation(home), { status: 'disabled' });
  await fs.writeFile(config, JSON.stringify(enabled));
  assert.deepEqual(await errorReporting(home), { status: 'connected', enabled: true });
  assert.deepEqual(await errorReporting(home, false), { status: 'connected', enabled: false });
  const revoked = JSON.parse(await fs.readFile(config, 'utf8'));
  assert.equal(revoked.errorReporting, false);
  assert.equal(revoked.privateFixture, 'keep');
  assert.deepEqual(await errorReporting(home), { status: 'connected', enabled: false });
  assert.deepEqual(await errorReporting(home, true), { status: 'connected', enabled: true });
  assert.notEqual(JSON.parse(await fs.readFile(config, 'utf8')).errorReportingGeneration, revoked.errorReportingGeneration);
  const first = await readLocation();
  assert.equal(first.status, 'connected');
  assert.equal(first.storage.source, 'builtin');
  assert.equal(first.storage.privateFixture, undefined);
  const savedFacts = await storageLocation(home, 'set', { path: selected, expected: first.revision });
  assert.equal(savedFacts.status, 'connected');
  const savedCandidate = locationCandidate(savedFacts.location);
  const savedProbe = await storageLocation(home, 'probe', { path: savedCandidate.probePath, expected: savedCandidate.revision });
  const saved = { status: 'connected', revision: savedCandidate.revision,
    storage: resolveLocationCandidate(savedFacts.location, savedProbe.available) };
  assert.equal(saved.status, 'connected');
  assert.equal(saved.storage.savedPath, selected);
  const raw = JSON.parse(await fs.readFile(config, 'utf8'));
  assert.equal(raw.privateFixture, 'keep');
  assert.equal(raw.capturesDir, selected);
  assert.deepEqual(await storageLocation(home, 'reset', { expected: first.revision }), { status: 'error', code: 'conflict' });
  const policy = await storagePreference(home, 'retention');
  assert.deepEqual(policy.policy, { mediaDays: { present: false, value: null }, bundleDays: { present: false, value: null } });
  const changedPolicy = await storagePreference(home, 'saveretention', { expected: policy.revision,
    policy: { mediaDays: 30, bundleDays: 90 } });
  assert.deepEqual(changedPolicy.policy, { mediaDays: { present: true, value: 30 }, bundleDays: { present: true, value: 90 } });
  assert.deepEqual(await storagePreference(home, 'retention'), changedPolicy);
  assert.deepEqual(await storagePreference(home, 'saveretention', { expected: policy.revision,
    policy: { mediaDays: null, bundleDays: null } }), { status: 'error', code: 'conflict' });
  await fs.writeFile(path.join(selected, 'synthetic.bin'), Buffer.alloc(1_000_000));
  assert.equal((await storagePreference(home, 'usage')).usage.formatted, '1 MB');
  const defaultPolicy = await storagePreference(home, 'resetretention', { expected: changedPolicy.revision });
  assert.deepEqual(defaultPolicy.policy, { mediaDays: { present: false, value: null }, bundleDays: { present: false, value: null } });
  assert.equal(defaultPolicy.custom, false);
  assert.equal(JSON.parse(await fs.readFile(config, 'utf8')).privateFixture, 'keep');
  assert.equal((await fs.stat(path.join(selected, 'synthetic.bin'))).size, 1_000_000);
  const expired = await new Promise((resolve, reject) => {
    const socket = net.createConnection(path.join(home, '.config/apparent/agent-control/capture.sock'));
    const timer = setTimeout(() => { socket.destroy(); reject(new Error('expired request stalled')); }, 4000);
    socket.on('error', reject);
    socket.once('connect', () => socket.write(JSON.stringify({ storage: 'resetlocation', expected: saved.revision,
      deadline: Number(process.hrtime.bigint() / 1000000n) - 1 }) + '\n'));
    let body = '';
    socket.on('data', data => { body += data; });
    socket.on('end', () => { clearTimeout(timer); resolve(JSON.parse(body)); });
  });
  assert.deepEqual(expired, { error: 'admission_expired' });
  assert.equal(JSON.parse(await fs.readFile(config, 'utf8')).capturesDir, selected);
  const resetFacts = await storageLocation(home, 'reset', { expected: saved.revision });
  const resetCandidate = locationCandidate(resetFacts.location);
  const reset = { status: 'connected', revision: resetCandidate.revision, storage: resetCandidate.snapshot };
  assert.equal(reset.status, 'connected');
  assert.equal(reset.storage.savedPath, null);
  assert.equal(Object.hasOwn(JSON.parse(await fs.readFile(config, 'utf8')), 'capturesDir'), false);
  await fs.writeFile(config, 'malformed');
  assert.deepEqual(await errorReporting(home), { status: 'error', code: 'config_read_failed' });
  assert.deepEqual(await storageLocation(home, 'facts'), { status: 'error', code: 'config_read_failed' });
  assert.deepEqual(await captureLocation(home, 'reset', { expected: resetFacts.location.revision }), { status: 'error', code: 'config_read_failed' });
  assert.equal(await fs.readFile(config, 'utf8'), 'malformed');
  console.log(JSON.stringify({ platform: process.platform, electron: process.versions.electron,
    bridge: 'real Rust socket', clock: 'actual Electron hrtime', checks: 'storage read/set/conflict/expired/reset/malformed; reporting default/read/optout/optin/generation/malformed' }));
}
const watchdog = setTimeout(() => { console.error('Rust/Electron integration deadline'); process.exit(1); }, 20000);
main().catch(error => { console.error(error); process.exitCode = 1; }).finally(() => clearTimeout(watchdog));
