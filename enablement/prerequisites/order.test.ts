import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync} from 'node:fs';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';

const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8');
const rust = workflow.split('\n  rust:\n')[1].split('\n  release-ready:\n')[0];
const preflight = 'CI-sensitive fixtures before native checks';
const packaging = 'Build and package the exact Mac candidate for native checks';
const storage = 'Native Mac Storage and error-reporting controls against the installed Rust GUI';
const copy = 'Native GPUI Copy capture ID against the same installed App';
const reference = 'Native Electron selected-reference control against the installed Rust App';
const macBridge = 'Early Mac Electron to Rust Settings integration (not native GUI proof)';
const full = 'Execute the same changed-tree checks (not GUI acceptance)';
const cache = 'Retain completed Mac dependency seeds after full verification';
const linuxBridge = 'Actual Electron to Rust Settings integration (not native GUI proof)';
const relevant = [preflight, packaging, storage, copy, reference, macBridge, full, cache, linuxBridge];
const earlyFilters = [
  'capture::desktop::settings::settings::tests::the_command_line_row_',
  'capture::desktop::workflow::agent_capture::tests::competing_launches_cannot_remove_the_winning_control_socket',
  'connect::hosted::hosted_tests::hosted_security_darwin_acl_grants_fail_closed',
];
const markers = [...rust.matchAll(/^      - name: (.+)$/gm)];
const steps = markers.map((match, i) => {
  const body = rust.slice(match.index!, markers[i + 1]?.index ?? rust.length);
  const multiline = body.match(/^        run: \|\n((?:          .*\n)+)/m)?.[1];
  const run = multiline ? multiline.split('\n').map(line => line.replace(/^          /, '')).join('\n')
    : body.match(/^        run: (.+)$/m)?.[1];
  return {name: match[1], body, run, condition: body.match(/^        if: (.+)$/m)?.[1]};
}).filter(step => relevant.includes(step.name));

// Execute the real workflow shell with command spies, never Cargo/native UI.
// This tests failure propagation and phase admission rather than a second plan.
function execute(t: any, os = 'macOS', failure = '', root = 'true') {
  const directory = mkdtempSync('/tmp/apparent-dev-ci-order-');
  t.after(() => rmSync(directory, {recursive: true, force: true}));
  const bin = join(directory, 'bin'); mkdirSync(bin);
  const trace = join(directory, 'trace'); const githubEnv = join(directory, 'env');
  writeFileSync(githubEnv, '');
  for (const executable of ['Electron.app/Contents/MacOS/Electron', 'electron']) {
    const target = join(directory, 'capture/desktop/ui/frontend/node_modules/electron/dist', executable);
    mkdirSync(join(target, '..'), {recursive: true});
    writeFileSync(target, '#!/bin/sh\nexit 0\n', {mode: 0o700});
  }
  for (const command of ['cargo', 'node', 'python3', 'npm', 'codesign', 'swiftc']) {
    writeFileSync(join(bin, command), `#!/bin/sh
set -eu
printf '%s|%s\\n' "$CURRENT_STEP" "$*" >> "$TRACE"
if [ -n "$FAIL_FRAGMENT" ]; then
  case "$*" in *"$FAIL_FRAGMENT"*) exit 31 ;; esac
fi
if [ "${command}" = node ]; then
  case "$*" in *'command.ts packaging '*) mkdir -p "$APPARENT_APP_DIR" ;; esac
fi
`, {mode: 0o700});
  }
  const executed: string[] = [];
  for (const step of steps) {
    const enabled = !step.condition || Function('runner', 'needs', `return (${step.condition})`)(
      {os}, {select: {outputs: {root, engine: 'true'}}});
    if (!enabled) continue;
    assert.ok(step.run, step.name);
    executed.push(step.name);
    const run = step.run.replaceAll('${{ needs.select.outputs.root }}', root);
    const result = spawnSync('bash', ['-e', '-c', run], {cwd: directory, encoding: 'utf8', env: {
      ...process.env, PATH: bin + ':' + process.env.PATH, HOME: directory, TMPDIR: directory,
      RUNNER_TEMP: directory, RUNNER_OS: os, GITHUB_ACTIONS: 'true', RUNNER_ENVIRONMENT: 'github-hosted',
      GITHUB_RUN_ID: 'fixture', GITHUB_RUN_ATTEMPT: '1', GITHUB_ENV: githubEnv,
      CURRENT_STEP: step.name, TRACE: trace, FAIL_FRAGMENT: failure,
    }});
    if (result.status !== 0) return {executed, status: result.status, stderr: result.stderr,
      trace: readFileSync(trace, 'utf8'), env: readFileSync(githubEnv, 'utf8')};
  }
  return {executed, status: 0, stderr: '', trace: readFileSync(trace, 'utf8'), env: readFileSync(githubEnv, 'utf8')};
}

test('security fixtures and required packaging precede serial native checks; full suites and cache completion remain required', t => {
  assert.deepEqual(steps.map(step => step.name), relevant);
  assert.equal((rust.match(/bash capture\/desktop\/scripts\/packaging\/install-app\.sh/g) ?? []).length, 1);
  const result = execute(t);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.executed, relevant.filter(name => name !== linuxBridge));
  const early = result.trace.split('\n').filter(line => line.startsWith(preflight + '|'));
  assert.equal(early.length, 1, 'Mac fixtures should share one Cargo freshness/build check');
  assert.equal(early[0], preflight + '|test -j 3 --locked --timings -- --test-threads=2 ' + earlyFilters.join(' '));
  assert.match(result.trace, /verification\.py --base [a-f0-9]+ --run --online --execution platform/);
  assert.match(result.env, /CACHE_ON_FAILURE=true/);
  for (const name of [storage, copy, reference]) {
    assert.equal(steps.find(step => step.name === name)!.body.includes('continue-on-error'), false);
  }
});
test('early Cargo failure prevents packaging, native assertions and cache completion', t => {
  const result = execute(t, 'macOS', 'test -j 3');
  assert.equal(result.status, 31);
  assert.deepEqual(result.executed, [preflight]);
  assert.doesNotMatch(result.env, /CACHE_ON_FAILURE=true/);
});
test('packaging failure cannot enter GUI assertions, ordinary suites or publish partial failure seeds', t => {
  const result = execute(t, 'macOS', 'command.ts packaging ');
  assert.equal(result.status, 31);
  assert.deepEqual(result.executed, [preflight, packaging]);
  assert.doesNotMatch(result.env, /CACHE_ON_FAILURE=true/);
});
test('native failure arrives before full suites without arming incomplete dependency seeds', t => {
  const result = execute(t, 'macOS', 'native-storage-assertions');
  assert.equal(result.status, 31);
  assert.deepEqual(result.executed, [preflight, packaging, storage]);
  assert.doesNotMatch(result.env, /CACHE_ON_FAILURE=true/);
});
test('Linux retains its required full-build-before-bridge order and never admits Mac GUI operations', t => {
  const result = execute(t, 'Linux');
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.executed, [preflight, full, linuxBridge]);
  assert.deepEqual(result.trace.split('\n').filter(line => line.startsWith(preflight + '|')), [
    preflight + '|test -j 3 --locked ' + earlyFilters[0] + ' -- --test-threads=2',
    preflight + '|test -j 3 --locked ' + earlyFilters[1] + ' -- --exact',
  ]);
  assert.doesNotMatch(result.trace, /native-(storage|copy|reference)-assertions|command.ts packaging/);
});

// Explicit opt-in: cheap contracts must not need Rust or compile the app.
// Run through t3-build on Clanker. This uses real Cargo/libtest with the actual
// workflow shell, a cold target and no dependencies/native helpers.
const cargoFixture = process.env.APPARENT_EARLY_CARGO_FIXTURE === '1' ? test : test.skip;
cargoFixture('real Cargo ORs every early Mac filter and propagates each assertion failure', t => {
  const directory = mkdtempSync('/tmp/apparent-dev-early-cargo-');
  t.after(() => rmSync(directory, {recursive: true, force: true}));
  const names = [earlyFilters[0] + 'abbreviates_only_paths_inside_home',
    earlyFilters[0] + 'installs_uninstalls_and_says_why_not', ...earlyFilters.slice(1)].sort();
  // Merge common namespaces so libtest sees the actual fully qualified names.
  const tree: any = {};
  for (const name of names) {
    const parts = name.split('::'); const leaf = parts.pop()!;
    let branch = tree;
    for (const part of parts) branch = branch[part] ??= {};
    branch[leaf] = `#[test] fn ${leaf}() { assert_ne!(std::env::var("EARLY_FAIL").unwrap_or_default(), "${name}"); }`;
  }
  const render = (branch: any): string => Object.entries(branch).map(([key, value]) =>
    typeof value === 'string' ? value : `mod ${key} { ${render(value)} }`).join('\n');
  writeFileSync(join(directory, 'Cargo.toml'), '[package]\nname="early-fixture"\nversion="0.0.0"\nedition="2024"\n[lib]\npath="lib.rs"\n');
  writeFileSync(join(directory, 'Cargo.lock'), 'version = 4\n[[package]]\nname="early-fixture"\nversion="0.0.0"\n');
  writeFileSync(join(directory, 'lib.rs'), render(tree) + '\n#[test] fn unselected() { panic!("unselected test ran"); }\n');
  const shell = steps.find(step => step.name === preflight)!.run!
    .replaceAll('${{ needs.select.outputs.root }}', 'true');
  for (const failure of ['', ...names]) {
    const result = spawnSync('bash', ['-e', '-c', shell], {cwd: directory, encoding: 'utf8', timeout: 30_000,
      env: {...process.env, RUNNER_OS: 'macOS', EARLY_FAIL: failure, CARGO_TARGET_DIR: join(directory, 'target'),
        CARGO_NET_OFFLINE: 'true', RUSTC_WRAPPER: ''}});
    assert.equal(result.status, failure ? 101 : 0, result.stdout + result.stderr);
    const executed = [...result.stdout.matchAll(/^test (\S+) \.\.\. (?:ok|FAILED)$/gm)].map(match => match[1]).sort();
    assert.deepEqual(executed, names, result.stdout);
    if (failure) assert.match(result.stdout, /test result: FAILED\. 3 passed; 1 failed;/);
    else assert.match(result.stdout, /test result: ok\. 4 passed; 0 failed;/);
  }
});
test('Engine-only selections do not launch the application or native controls', t => {
  const result = execute(t, 'macOS', '', 'false');
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.executed, [full, cache]);
  assert.doesNotMatch(result.trace, /install-app|_electron_rust_roundtrip/);
});
