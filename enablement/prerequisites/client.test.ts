import {test} from 'node:test';
import assert from 'node:assert/strict';
import {execFileSync, spawnSync} from 'node:child_process';
import {existsSync, mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync} from 'node:fs';
import {join} from 'node:path';

const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8');
const job = (id: string) => workflow.split(`\n  ${id}:\n`)[1].split(/\n  [\w-]+:\n/)[0];
const client = job('connect-client');
const run = client.split('      - name: Independent Connect client build and tests\n')[1]
  .split('        run: |\n')[1].split('      - name:')[0]
  .split('\n').map(line => line.replace(/^          /, '')).join('\n');

function execute(t: any, failure = '', cleanupFailure = false) {
  const root = mkdtempSync('/tmp/apparent-dev-client-ci-');
  t.after(() => rmSync(root, {recursive: true, force: true}));
  const bin = join(root, 'bin'); mkdirSync(bin);
  const trace = join(root, 'trace'); const tempPath = join(root, 'temp-path');
  writeFileSync(join(bin, 'node'), `#!/bin/sh
set -eu
printf '%s\\n' "$*" >> "$TRACE"
printf '%s' "$TMPDIR" > "$TEMP_PATH"
case "$*" in *"$FAIL_FRAGMENT"*) if [ -n "$FAIL_FRAGMENT" ]; then exit 23; fi ;; esac
`, {mode: 0o700});
  if (cleanupFailure) writeFileSync(join(bin, 'rm'), '#!/bin/sh\nexit 29\n', {mode: 0o700});
  const result = spawnSync('bash', ['-e', '-c', run], {cwd: root, encoding: 'utf8', env: {
    ...process.env, HOME: root, PATH: bin + ':' + process.env.PATH,
    TRACE: trace, TEMP_PATH: tempPath, FAIL_FRAGMENT: failure,
  }});
  return {...result, commands: readFileSync(trace, 'utf8').trim().split('\n'),
    tempRemains: existsSync(readFileSync(tempPath, 'utf8'))};
}
test('client runs independently on both platforms with isolated cache and unchanged automatic/manual admission', () => {
  assert.match(client, /^    needs: select$/m);
  assert.equal(client.match(/^    if: (.+)$/m)?.[1], job('rust').match(/^    if: (.+)$/m)?.[1]);
  assert.match(client, /toolchain: \$\{\{ runner\.os == 'macOS' && '1\.99\.1' \|\| 'stable' \}\}/);
  assert.ok(client.indexOf('uses: ./.github/actions/mac-rust-cache') < client.indexOf('uses: Swatinem/rust-cache'));
  assert.match(client, /key: connect-client\n          workspaces: connect\/client -> target/);
  assert.doesNotMatch(client, /cache-on-failure:|continue-on-error:|setup-chrome|MetalToolchain/);
  assert.match(job('rust'), /\. -> target\n            capture\/engine -> target\n            connect\/client -> target/);
  assert.doesNotMatch(job('rust'), /--manifest-path connect\/client\/Cargo.toml/);
  assert.match(job('select'), /--execution portable/);
  assert.match(job('release-ready'), /needs: \[select, desktop-ui, connect-client, rust\]/);
});
test('the real client shell retains build, test compilation/execution, flags and private cleanup', t => {
  const result = execute(t);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.commands, [
    '--experimental-strip-types enablement/timing/command.ts client-compilation cargo build --manifest-path connect/client/Cargo.toml --locked --timings',
    '--experimental-strip-types enablement/timing/command.ts client-test-compilation cargo test --manifest-path connect/client/Cargo.toml --locked --timings --no-run',
    '--experimental-strip-types enablement/timing/command.ts client-test-execution cargo test --manifest-path connect/client/Cargo.toml --locked --timings -- --test-threads=2',
  ]);
  assert.equal(result.tempRemains, false);
});
test('failed compilation prevents test execution and retains its exit status through cleanup', t => {
  const result = execute(t, 'client-test-compilation');
  assert.equal(result.status, 23);
  assert.equal(result.commands.length, 2);
  assert.equal(result.tempRemains, false);
});
test('cleanup failure fails an otherwise successful client job and preserves an earlier failure', t => {
  assert.equal(execute(t, '', true).status, 1);
  assert.equal(execute(t, 'client-test-compilation', true).status, 23);
});
test('actual publication admission rejects absent, failed, cancelled, skipped or incomplete client verification', () => {
  execFileSync('python3', ['-c', String.raw`
import copy, importlib.util
spec=importlib.util.spec_from_file_location('admission_tests','enablement/tests/test_release_gate.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
case=module.AdmissionTests();case.setUp()
for platform,runner in [('linux','ubuntu-24.04'),('macos','macos-15-intel')]:
    name=f'verify / connect-client ({runner})'
    assert name in module.gate.REQUIRED[platform]
    original=copy.deepcopy(case.jobs[platform])
    for failure in ['missing','failure','cancelled','skipped','in_progress']:
        case.jobs[platform]=copy.deepcopy(original)
        if failure=='missing': case.jobs[platform]=[job for job in case.jobs[platform] if job['name']!=name]
        else:
            job=next(job for job in case.jobs[platform] if job['name']==name)
            if failure=='in_progress': job['status']=failure
            else: job['conclusion']=failure
        try: case.admit(platform)
        except ValueError: pass
        else: raise AssertionError(f'{platform}: admitted {failure} client verification')
    case.jobs[platform]=original
    assert case.admit(platform)==case.sha
`], {stdio: 'pipe'});
});
