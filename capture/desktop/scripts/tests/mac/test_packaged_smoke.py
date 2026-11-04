"""Host regressions; these do not establish macOS package acceptance."""
from pathlib import Path
import subprocess
import unittest

SCRIPT = Path(__file__).resolve().parents[2] / 'mac/packaged-smoke.cjs'


class PackagedSmokeTests(unittest.TestCase):
    def node(self, source):
        result = subprocess.run(['node', '-e', source, str(SCRIPT)],
                                capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_personal_hosts_and_missing_opt_in_are_rejected(self):
        self.node("""
const assert = require('node:assert/strict');
const { admission } = require(process.argv[1]);
const env = { CI: 'true', GITHUB_ACTIONS: 'true', RUNNER_ENVIRONMENT: 'github-hosted',
  APPARENT_PACKAGED_SMOKE: '1', APPARENT_APP_DIR: '/owned/Apparent.app',
  APPARENT_SMOKE_OUTPUT: '/owned/evidence' };
admission(env, 'darwin');
assert.throws(() => admission(env, 'linux'));
for (const key of Object.keys(env)) {
  const invalid = { ...env }; delete invalid[key];
  assert.throws(() => admission(invalid, 'darwin'), key);
}
assert.throws(() => admission({ ...env, RUNNER_ENVIRONMENT: 'self-hosted' }, 'darwin'));
""")

    def test_readback_retries_only_explicit_worker_contention(self):
        self.node("""
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const net = require('node:net');
const { readLocation } = require(process.argv[1]);
const { captureLocation } = require(path.resolve(path.dirname(process.argv[1]), '../../ui/frontend/electron/runtime.cjs'));
(async () => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'ps-'));
  const directory = path.join(home, '.config/apparent/agent-control');
  await fs.mkdir(directory, { recursive: true, mode: 0o700 });
  const replies = [];
  let requests = 0;
  const server = net.createServer(socket => socket.once('data', () => {
    requests++;
    socket.end(JSON.stringify(requests === 1 ? {error:'app_busy'} : {storage: {
      savedPath:'/synthetic', effectivePath:'/synthetic', displayPath:'/synthetic',
      custom:true, source:'configured', revision:'a'.repeat(16) }}) + '\\n');
  }));
  try {
    const address = path.join(directory, 'capture.sock');
    await new Promise(resolve => server.listen(address, resolve));
    await fs.chmod(address, 0o600);
    const result = await readLocation(() => captureLocation(home), () => 1000, replies);
    assert.equal(result.status, 'connected');
    assert.equal(requests, 2);
    assert.deepEqual(replies[0], {status:'error',code:'app_busy'});
    let calls = 0;
    const invalid = await readLocation(async () => { calls++; return {status:'error',code:'invalid_response'}; }, () => 1000, []);
    assert.equal(invalid.code, 'invalid_response');
    assert.equal(calls, 1);
    await assert.rejects(readLocation(async () => ({status:'error',code:'app_busy'}), () => 1000, []), /eight bounded reads/);
  } finally {
    await new Promise(resolve => server.close(resolve));
    await fs.rm(home, {recursive:true});
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
""")
