import {execFileSync} from 'node:child_process';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';
import assert from 'node:assert/strict';

const probe = String.raw`
import importlib.util, json, subprocess, sys
from unittest.mock import patch
spec = importlib.util.spec_from_file_location("verification", "enablement/verification.py")
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)
selection = {"root": True, "engine": True, "browser": True}
commands = []
def execute(command, **kwargs):
    commands.append(command)
    if sys.argv[2] == "fail" and command[0] == "bun":
        raise subprocess.CalledProcessError(7, command)
    return subprocess.CompletedProcess(command, 0)
try:
    with patch.object(v, "check_temp_environment"), patch.object(v.subprocess, "run", side_effect=execute):
        if sys.argv[1] == "default":
            v.run_checks(selection)
        else:
            v.run_checks(selection, execution=sys.argv[1])
except subprocess.CalledProcessError:
    sys.exit(7)
print(json.dumps(commands))
`;
function inventory(scope: string) {
  return JSON.parse(execFileSync('python3', ['-c', probe, scope, 'ok'], {encoding: 'utf8'})) as string[][];
}
test('explicit routing partitions the unchanged local full inventory', () => {
  const full = inventory('default');
  const portable = inventory('portable');
  const platform = inventory('platform');
  assert.deepEqual(inventory('full'), full);
  assert.deepEqual([...portable, ...platform].map(JSON.stringify).sort(), full.map(JSON.stringify).sort());
  assert.ok(portable.some(c => c[0] === 'bun'));
  assert.ok(portable.some(c => c.includes('scripts/test-browser.mjs')));
  assert.ok(!portable.some(c => c[0] === 'cargo' || c.includes('test_temp_preflight.py')));
  assert.ok(platform.some(c => c.includes('capture/engine/Cargo.toml')));
  assert.ok(platform.some(c => c.includes('test_temp_preflight.py')));
  assert.ok(!platform.some(c => c[0] === 'bun' || c.includes('test_verification.py')));
});
test('portable browser failures propagate rather than admitting platform work', () => {
  const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8');
  const job = (id: string) => {
    const body = workflow.split(`\n  ${id}:\n`)[1];
    assert.ok(body, `Missing ${id} job`);
    return body.split(/\n  [\w-]+:\n/)[0];
  };
  const select = job('select');
  assert.match(select, /^    runs-on: ubuntu-24\.04$/m);
  assert.match(select, /python3 enablement\/verification\.py .*--execution portable/);
  for (const id of ['rust', 'desktop-ui']) {
    assert.match(job(id), /^    needs: select$/m);
  }
  assert.throws(() => execFileSync('python3', ['-c', probe, 'portable', 'fail']), {status: 7});
});
