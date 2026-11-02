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

    def test_cleanup_stops_owned_supervisor_and_descendant(self):
        self.node("""
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { supervisor, stopBackend } = require(process.argv[1]);
(async () => {
  const child = spawn('/bin/bash', ['-c', supervisor, 'smoke-cleanup-test',
    '/bin/bash', '-c', 'echo $$; exec sleep 60'], { detached: true, stdio: ['pipe', 'pipe', 'inherit'] });
  try {
    const [line] = await once(child.stdout, 'data');
    const descendant = Number(line.toString().trim());
    assert.ok(descendant > 0 && descendant !== child.pid);
    process.kill(descendant, 0);
    await stopBackend(child);
    assert.throws(() => process.kill(-child.pid, 0), { code: 'ESRCH' });
    assert.throws(() => process.kill(descendant, 0), { code: 'ESRCH' });
  } finally { if (child.exitCode === null && child.signalCode === null) await stopBackend(child); }
})().catch(error => { console.error(error); process.exitCode = 1; });
""")

    def test_timed_out_electron_close_terminates_owned_child_and_fails(self):
        self.node("""
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { stopElectron, within } = require(process.argv[1]);
(async () => {
  const child = spawn('/bin/sleep', ['60']);
  await once(child, 'spawn');
  try {
    await assert.rejects(within(new Promise(() => {}), 10), /timed out/);
  } finally {
    await assert.rejects(stopElectron({ close: () => new Promise(() => {}), process: () => child }, 10), /timed out/);
    assert.equal(child.signalCode, 'SIGKILL');
    assert.throws(() => process.kill(child.pid, 0), { code: 'ESRCH' });
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
""")
