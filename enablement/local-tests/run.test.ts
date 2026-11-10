import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, copyFileSync, rmSync, chmodSync } from 'node:fs';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { select, selection, owners, changedPaths, captureToolTests, commands, contractTests, execute, root } from './run.ts';

test('owner selection retains consumers and unrelated packages stay out', () => {
  assert.deepEqual(select(['enablement/local-tests/run.ts']), ['tooling']);
  assert.deepEqual(select(['capture/desktop/ui/frontend/src/Settings.tsx']), ['capture', 'desktop']);
  assert.deepEqual(select(['connect/client/src/config.rs']), ['capture', 'client']);
  assert.deepEqual(select(['shared/ui/src/button.tsx']), ['capture', 'desktop', 'service']);
  assert.deepEqual(select(['public/site/src/App.tsx']), ['public']);
  assert.deepEqual(select(['connect/web/src/api.ts']), ['web', 'public']);
  assert.deepEqual(select(['hermes/inspect.py']), ['hermes']);
  assert.deepEqual(select(['capture/desktop/scripts/packaging/install-app.sh']), ['capture', 'engine', 'capture-tools']);
  assert.deepEqual(select(['README.md']), []);
});

test('unknown, missing history and empty deltas cannot silently skip coverage', () => {
  for (const paths of [[], ['Cargo.lock'], ['new-owner/file.ts'], ['shared/schema.ts'], ['.github/workflows/checks.yml']]) {
    assert.deepEqual(select(paths), [...owners]);
  }
  assert.deepEqual(select(['README.md'], false), [...owners]);
});

test('prerequisite, CI routing, selector and timing regressions are mandatory even on docs pushes', () => {
  const plan = commands([], 'base', 'head');
  const args = [...plan, ...contractTests()].flatMap(command => command.argv);
  for (const path of ['enablement/prerequisites/native.test.ts',
    'enablement/timing/command.test.ts', 'enablement/timing/mac-cache.test.ts', 'enablement/local-tests/run.test.ts']) {
    assert.ok(args.includes(path), path);
  }
  assert.ok(args.includes('--pr'));
  assert.ok(args.includes('enablement/ci/'));
  assert.ok(!args.includes('cargo'));
  assert.ok(!args.some(arg => arg.includes('verification-status')), 'retired reporter stays absent from local gates');
});

test('package gates execute complete canonical suites including browser and Electron smoke', () => {
  const plan = commands([...owners], 'base', 'head', 'linux');
  for (const [cwd, last] of [['connect/web', 'test'], ['connect/service', 'test:recipient'],
    ['capture/desktop/ui/frontend', 'test:electron'], ['public/site', 'scripts/site-build/verify.mjs']]) {
    assert.ok(plan.some(command => command.cwd === cwd && command.argv.at(-1) === last), `${cwd} ${last}`);
  }
  assert.ok(plan.some(command => command.argv.includes('--run')));
  assert.ok(!plan.some(command => command.cwd === 'connect/client/typescript'),
    'the root verifier owns this prerequisite so it is not installed twice');
  assert.ok(!commands(['desktop'], 'base', 'head', 'darwin').some(command => command.argv.includes('xvfb-run')));
  assert.ok(commands(['client'], 'base', 'head', 'darwin').some(command => command.argv[0] === 'taskpolicy'));
  const clientOnly = commands(['client'], 'base', 'head', 'linux');
  const clientPrepare = clientOnly.findIndex(command => command.argv.includes('--prepare-connect-typescript'));
  const clientCargo = clientOnly.findIndex(command => command.argv[0] === 'cargo');
  assert.ok(clientPrepare >= 0 && clientPrepare < clientCargo,
    'standalone client verification prepares shares before Cargo');
  const build = plan.findIndex(command => command.argv.includes('--run'));
  const cli = plan.findIndex(command => command.argv.at(-1)?.endsWith('/test_issue_cli.py'));
  assert.ok(cli > build && build >= 0, 'rebuilt application must precede CLI integration');
  assert.ok(plan[cli].env?.APPARENT_BIN.endsWith('/debug/apparent'));
});

function fixture(action: (dir: string, env: NodeJS.ProcessEnv) => void) {
  const dir = mkdtempSync(join(process.env.TMPDIR || '/tmp', 'apparent-dev-local-hook-'));
  const env = { ...process.env };
  // cwd cannot isolate Git fixtures from a real push's GIT_DIR / index.
  // The imported production helpers spawn with process.env too; isolate that
  // synchronous scope, not just the explicit fixture setup commands.
  const inherited: Record<string, string> = {};
  for (const key of Object.keys(env)) if (key.startsWith('GIT_')) {
    inherited[key] = env[key]!;
    delete env[key]; delete process.env[key];
  }
  try { action(dir, env); } finally {
    Object.assign(process.env, inherited);
    rmSync(dir, { recursive: true, force: true });
  }
}

test('native temporary environment and Desktop gates fail before expensive Rust work', () => {
  for (const platform of ['linux', 'darwin']) {
    const plan = commands(['capture', 'desktop'], 'base', 'head', platform);
    const preflight = plan.findIndex(command => command.argv.includes('--preflight'));
    const rust = plan.findIndex(command => command.argv.includes('--run'));
    const desktop = plan.findIndex(command => command.argv.at(-1) === 'test:electron');
    assert.equal(preflight, 0, 'temporary-root failure must precede even cheap contracts');
    assert.ok(desktop > preflight && desktop < rust, 'Electron readiness must be established before Rust');
    assert.equal(plan.filter(command => command.argv.at(-1) === 'test:electron').length, 1);
  }
  for (const selected of [['capture'], ['engine'], ['client']] as const) {
    assert.ok(commands([...selected], 'base', 'head')[0].argv.includes('--preflight'));
  }
  assert.ok(!commands([], 'base', 'head').some(command => command.argv.includes('--preflight')));
});

test('real command execution stops at temporary-root or Electron failure without reaching Rust', () => fixture((dir, env) => {
  const bin = join(dir, 'bin'); mkdirSync(bin);
  const trace = join(dir, 'trace');
  for (const name of ['python3', 'npm', 'xvfb-run']) {
    const file = join(bin, name);
    writeFileSync(file, '#!/bin/sh\n' +
      (name === 'xvfb-run' ? 'shift\nexec "$@"\n' :
        'printf "%s\\n" "$*" >> "$PREFLIGHT_TRACE"\n' +
        'case " $* " in *" $PREFLIGHT_FAIL "*) exit 19;; esac\n'));
    chmodSync(file, 0o700);
  }
  const old = {...process.env};
  try {
    Object.assign(process.env, {PATH: bin + ':' + env.PATH, PREFLIGHT_TRACE: trace});
    for (const failure of ['--preflight', 'test:electron']) {
      process.env.PREFLIGHT_FAIL = failure;
      writeFileSync(trace, '');
      const plan = commands(['capture', 'desktop'], 'base', 'head', 'linux');
      // All package working directories exist, but no installed/generated
      // artifacts are available to conceal a missing prerequisite.
      mkdirSync(join(dir, 'capture/desktop/ui/frontend'), {recursive: true});
      assert.throws(() => execute(plan, dir), /failed \(19\)/);
      const calls = readFileSync(trace, 'utf8');
      assert.ok(calls.includes(failure));
      assert.doesNotMatch(calls, /--run|cargo/);
      if (failure === '--preflight') assert.doesNotMatch(calls, /--pr\b|ci --no-audit/);
    }
  } finally {
    for (const name of ['PATH', 'PREFLIGHT_TRACE', 'PREFLIGHT_FAIL']) {
      if (old[name] === undefined) delete process.env[name]; else process.env[name] = old[name];
    }
  }
}));

test('committed concurrency-only workflow edits select contracts rather than product suites', () => fixture((dir, env) => {
  const git = (...args: string[]) => {
    const result = spawnSync('git', args, { cwd: dir, env, encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr); return result.stdout.trim();
  };
  git('init', '-q'); git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'test@example.invalid');
  mkdirSync(join(dir, '.github/workflows'), {recursive: true});
  const path = '.github/workflows/checks.yml';
  const before = readFileSync(join(root, path), 'utf8');
  writeFileSync(join(dir, path), before);
  git('add', '.'); git('commit', '-qm', 'base'); const base = git('rev-parse', 'HEAD');
  writeFileSync(join(dir, path), before.replace(/  cancel-in-progress: .*/, '  cancel-in-progress: false'));
  git('add', '.'); git('commit', '-qm', 'concurrency');
  const result = selection(base, 'HEAD', dir);
  assert.deepEqual(result.workflowControls, [path]);
  assert.deepEqual(result.selected, ['tooling']);
  assert.deepEqual(commands(result.selected, base, 'HEAD').map(command => command.argv), [
    ['python3', 'enablement/verification.py', '--base', base, '--head', 'HEAD', '--pr'],
    ['python3', '-m', 'unittest', 'discover', '-s', 'enablement/directory-width/tests', '-p', 'test_*.py'],
  ]);
  assert.deepEqual(selection(base, 'HEAD', dir, true).selected, [...owners]);
  // A later safe workflow/docs commit cannot mask an earlier build change.
  writeFileSync(join(dir, 'Cargo.lock'), 'changed build graph');
  git('add', '.'); git('commit', '-qm', 'build input');
  writeFileSync(join(dir, 'README.md'), 'later docs');
  git('add', '.'); git('commit', '-qm', 'docs');
  assert.deepEqual(selection(base, 'HEAD', dir).selected, [...owners]);
  assert.deepEqual(selection('unavailable', 'HEAD', dir).selected, [...owners]);
  assert.deepEqual(selection('HEAD', 'HEAD', dir).selected, [...owners]);
}));

test('real Git delta includes earlier commits and both sides of cross-owner rename/deletion', () => fixture((dir, env) => {
  const git = (...args: string[]) => {
    const result = spawnSync('git', args, { cwd: dir, env, encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.trim();
  };
  git('init', '-q'); git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'test@example.invalid');
  mkdirSync(join(dir, 'capture/engine'), { recursive: true });
  writeFileSync(join(dir, 'capture/engine/old.rs'), 'original');
  git('add', '.'); git('commit', '-qm', 'base'); const base = git('rev-parse', 'HEAD');
  mkdirSync(join(dir, 'public/site'), { recursive: true });
  git('mv', 'capture/engine/old.rs', 'public/site/new.ts'); git('commit', '-qm', 'move');
  writeFileSync(join(dir, 'README.md'), 'docs'); git('add', '.'); git('commit', '-qm', 'later docs');
  const paths = changedPaths(base, 'HEAD', dir);
  assert.deepEqual(paths, ['README.md', 'capture/engine/old.rs', 'public/site/new.ts']);
  assert.deepEqual(select(paths), ['capture', 'engine', 'public']);
  git('rm', 'public/site/new.ts'); git('commit', '-qm', 'delete');
  assert.ok(changedPaths(base, 'HEAD', dir).includes('capture/engine/old.rs'));
}));

test('capture script changes execute offline contracts including hyphenated and new test files', () => fixture((dir, env) => {
  for (const path of ['capture/desktop/scripts/packaging/test_installer_guard.py',
    'capture/desktop/scripts/tests/mac/test_setup.py', 'capture/desktop/scripts/linux/gui/test-gui-vm.py',
    'capture/agent/scripts/tests/test_provision_narrator.py', 'capture/desktop/scripts/new/test_new.py']) {
    const parent = path.slice(0, path.lastIndexOf('/'));
    mkdirSync(join(dir, parent), { recursive: true }); writeFileSync(join(dir, path), '');
  }
  for (const args of [['init', '-q'], ['add', '.']]) assert.equal(spawnSync('git', args, { cwd: dir, env }).status, 0);
  const suites = captureToolTests(dir);
  assert.equal(suites.length, 5);
  assert.ok(suites.some(command => command.argv.includes('capture/desktop/scripts/new/test_new.py')));
  const hyphenated = suites.find(command => command.argv.includes('capture/desktop/scripts/linux/gui/test-gui-vm.py'))!;
  assert.ok(hyphenated);
  const receipt = join(dir, 'executed');
  writeFileSync(join(dir, hyphenated.argv.at(-1)!), `import unittest\nfrom pathlib import Path\nclass Contract(unittest.TestCase):\n def test_failure(self):\n  Path(${JSON.stringify(receipt)}).write_text('executed')\n  self.fail('hyphenated regression reached')\n`);
  assert.throws(() => execute([hyphenated], dir), /failed \(1\)/);
  assert.equal(readFileSync(receipt, 'utf8'), 'executed');
}));

test('new native prerequisite tests join the mandatory contract inventory automatically', () => fixture((dir) => {
  const folder = join(dir, 'enablement/prerequisites');
  mkdirSync(folder, { recursive: true });
  assert.throws(() => contractTests(dir), /inventory is empty/);
  writeFileSync(join(folder, 'new-picker.test.ts'), '');
  writeFileSync(join(folder, 'helper.ts'), '');
  const args = contractTests(dir)[0].argv;
  assert.ok(args.includes('enablement/prerequisites/new-picker.test.ts'));
  assert.ok(!args.includes('enablement/prerequisites/helper.ts'));
}));

test('command failure stops subsequent tests and absent required package fails', () => fixture((dir) => {
  const receipt = join(dir, 'must-not-exist');
  assert.throws(() => execute([
    { argv: [process.execPath, '-e', 'process.exit(7)'], cwd: '.' },
    { argv: [process.execPath, '-e', `require('fs').writeFileSync(${JSON.stringify(receipt)}, 'bad')`], cwd: '.' },
  ], dir), /failed \(7\)/);
  assert.throws(() => readFileSync(receipt));
  assert.throws(() => execute([{ argv: [process.execPath, '-e', ''], cwd: 'missing' }], dir));
}));

test('hook Git routing cannot redirect a selected test into the caller repository', () => fixture((dir, env) => {
  for (const name of ['parent', 'child']) {
    mkdirSync(join(dir, name));
    assert.equal(spawnSync('git', ['init', '-q'], { cwd: join(dir, name), env }).status, 0);
    assert.equal(spawnSync('git', ['config', 'user.name', name], { cwd: join(dir, name), env }).status, 0);
  }
  process.env.GIT_DIR = join(dir, 'parent/.git');
  process.env.GIT_WORK_TREE = join(dir, 'parent');
  try {
    execute([{ argv: ['git', 'config', 'user.name', 'selected-test'], cwd: 'child' }], dir);
    const name = (repo: string) => spawnSync('git', ['config', 'user.name'], { cwd: join(dir, repo), env, encoding: 'utf8' }).stdout.trim();
    assert.equal(name('parent'), 'parent');
    assert.equal(name('child'), 'selected-test');
  } finally { delete process.env.GIT_DIR; delete process.env.GIT_WORK_TREE; }
}));

test('actual feature push runs selected package tests and rejects their failure', () => fixture((dir, env) => {
  const git = (...args: string[]) => {
    const result = spawnSync('git', args, { cwd: dir, env, encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr); return result.stdout.trim();
  };
  mkdirSync(join(dir, 'enablement/local-tests'), { recursive: true });
  copyFileSync(join(root, 'enablement/local-tests/run.ts'), join(dir, 'enablement/local-tests/run.ts'));
  copyInputHelpers(dir);
  mkdirSync(join(dir, 'public/site'), { recursive: true });
  writeFileSync(join(dir, 'public/site/app.ts'), 'base');
  git('init', '-q'); git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'test@example.invalid');
  git('add', '.'); git('commit', '-qm', 'base'); git('update-ref', 'refs/remotes/origin/main', git('rev-parse', 'HEAD'));
  writeFileSync(join(dir, 'public/site/app.ts'), 'changed'); git('add', '.'); git('commit', '-qm', 'feature');
  const head = git('rev-parse', 'HEAD');
  // Tools after the real selector are inert boundaries. They record the selected
  // real hook commands without recursively running this test or building packages.
  const bin = join(dir, '.git/bin'); mkdirSync(bin);
  const log = join(dir, '.git/commands');
  for (const tool of ['node', 'python3', 'bun']) {
    const body = tool === 'node' ? `if [ "$2" = enablement/local-tests/run.ts ]; then exec '${process.execPath}' "$@"; fi\n` : '';
    writeFileSync(join(bin, tool), `#!/bin/sh\n${body}printf '%s\\n' '${tool}'" $*" >> "$COMMAND_LOG"\ncase "$*" in *scripts/site-build/verify.mjs*) exit "${'${FAIL_PACKAGE:-0}'}";; esac\n`, { mode: 0o755 });
  }
  const push = (fail: string) => spawnSync('sh', [join(root, '.githooks/pre-push')], {
    cwd: dir, env: { ...env, PATH: `${bin}:${env.PATH}`, COMMAND_LOG: log, FAIL_PACKAGE: fail },
    input: `refs/heads/feature ${head} refs/heads/feature ${'0'.repeat(40)}\n`, encoding: 'utf8',
  });
  const passed = push('0'); assert.equal(passed.status, 0, passed.stderr);
  assert.match(readFileSync(log, 'utf8'), /node scripts\/site-build\/verify.mjs/);
  assert.doesNotMatch(readFileSync(log, 'utf8'), /cargo|npm/);
  const failed = push('7'); assert.notEqual(failed.status, 0); assert.match(failed.stderr, /push rejected/);
}));

test('installed hook proves control-only selection, mandatory commands, failure and protected-ref fallback', () => fixture((dir, env) => {
  const git = (...args: string[]) => {
    const result = spawnSync('git', args, {cwd: dir, env, encoding: 'utf8'});
    assert.equal(result.status, 0, result.stderr); return result.stdout.trim();
  };
  git('init', '-q'); git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'test@example.invalid');
  mkdirSync(join(dir, 'enablement/local-tests'), {recursive: true});
  copyFileSync(join(root, 'enablement/local-tests/run.ts'), join(dir, 'enablement/local-tests/run.ts'));
  copyInputHelpers(dir);
  mkdirSync(join(dir, '.github/workflows'), {recursive: true});
  const path = '.github/workflows/checks.yml';
  const before = readFileSync(join(root, path), 'utf8').replace(/  cancel-in-progress: .*/, '  cancel-in-progress: true');
  writeFileSync(join(dir, path), before);
  git('add', '.'); git('commit', '-qm', 'base'); const base = git('rev-parse', 'HEAD');
  git('update-ref', 'refs/remotes/origin/main', base);
  writeFileSync(join(dir, path), before.replace('cancel-in-progress: true', 'cancel-in-progress: false'));
  git('add', '.'); git('commit', '-qm', 'control'); const head = git('rev-parse', 'HEAD');
  const bare = join(dir, '.git/remote.git'); git('init', '--bare', '-q', bare);
  git('remote', 'add', 'origin', bare);
  const hooks = join(dir, '.git/hooks');
  copyFileSync(join(root, '.githooks/pre-push'), join(hooks, 'pre-push'));
  git('config', 'core.hooksPath', hooks);
  const bin = join(dir, '.git/bin'); mkdirSync(bin);
  const log = join(dir, '.git/commands');
  const bun = spawnSync('which', ['bun'], {env, encoding: 'utf8'}).stdout.trim();
  // Keep the hook, Git history and YAML proof real. Observe downstream command
  // boundaries without recursively executing this same test through --pr.
  for (const tool of ['node', 'bun', 'python3']) {
    const forward = tool === 'node' ? `exec '${process.execPath}' "$@"\n`
      : tool === 'bun' ? `case "$1" in */workflow-controls.ts) exec '${bun}' "$@";; esac\n` : '';
    const observe = `printf '%s\\n' '${tool}'" $*" >> "$COMMAND_LOG"\n`;
    writeFileSync(join(bin, tool), `#!/bin/sh\n${forward}${observe}case "$*" in *--pr*) exit "${'${FAIL_CONTRACT:-0}'}";; esac\n`, {mode: 0o755});
  }
  const config = readFileSync(join(dir, '.git/config'), 'utf8');
  const index = readFileSync(join(dir, '.git/index'));
  const push = (ref: string, fail = '0') => spawnSync('git', ['push', 'origin', `HEAD:${ref}`], {
    cwd: dir, env: {...env, PATH: `${bin}:${env.PATH}`, COMMAND_LOG: log, FAIL_CONTRACT: fail}, encoding: 'utf8',
  });
  const passed = push('refs/heads/feature'); assert.equal(passed.status, 0, passed.stderr);
  assert.match(passed.stdout, /"workflowControls":\[".github\/workflows\/checks.yml"\]/);
  assert.match(passed.stdout, /"selected":\["tooling"\]/);
  assert.equal(git('--git-dir', bare, 'rev-parse', 'refs/heads/feature'), head);
  const invoked = readFileSync(log, 'utf8');
  assert.match(invoked, new RegExp(`python3 enablement/verification.py --base ${base} --head ${head} --pr`));
  assert.match(invoked, /python3 -m unittest discover -s enablement\/directory-width\/tests/);
  assert.doesNotMatch(invoked, /cargo|npm|site-build|--run/);
  const failed = push('refs/heads/failed', '7'); assert.notEqual(failed.status, 0);
  assert.match(failed.stderr, /Local test command failed \(7\).*--pr/);
  assert.match(failed.stderr, /push rejected/);
  assert.notEqual(spawnSync('git', ['--git-dir', bare, 'rev-parse', '--verify', 'refs/heads/failed'], {env}).status, 0);
  // Fail the first contract to observe full selection without running products.
  for (const ref of ['refs/heads/main', 'refs/tags/preview-fixture']) {
    const protectedPush = push(ref, '7'); assert.notEqual(protectedPush.status, 0);
    assert.match(protectedPush.stdout, /"workflowControls":\[\]/);
    const result = JSON.parse(protectedPush.stdout.trim());
    assert.deepEqual(result.selected, [...owners]);
  }
  assert.equal(readFileSync(join(dir, '.git/config'), 'utf8'), config);
  assert.deepEqual(readFileSync(join(dir, '.git/index')), index);
}));

function copyInputHelpers(dir: string) {
  for (const name of ['service.ts', 'receipt.ts'])
    copyFileSync(join(root, 'enablement/local-tests', name), join(dir, 'enablement/local-tests', name));
  mkdirSync(join(dir, 'enablement/ci'), {recursive: true});
  for (const name of ['inputs.ts', 'workflow-controls.ts'])
    copyFileSync(join(root, 'enablement/ci', name), join(dir, 'enablement/ci', name));
}
