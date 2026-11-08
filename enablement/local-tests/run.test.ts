import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, copyFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { select, owners, changedPaths, captureToolTests, commands, contractTests, execute, root } from './run.ts';

test('owner selection retains consumers and unrelated packages stay out', () => {
  assert.deepEqual(select(['enablement/verification-status/reporter.ts']), ['tooling']);
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

test('CI reporter, prerequisite and timing regressions are mandatory even on docs pushes', () => {
  const plan = commands([], 'base', 'head');
  const args = [...plan, ...contractTests()].flatMap(command => command.argv);
  for (const path of ['enablement/verification-status/',
    'enablement/prerequisites/native.test.ts', 'enablement/timing/command.test.ts', 'enablement/local-tests/run.test.ts']) {
    assert.ok(args.includes(path), path);
  }
  assert.ok(args.includes('--pr'));
  assert.ok(!args.includes('cargo'));
});

test('package gates execute complete canonical suites including browser and Electron smoke', () => {
  const plan = commands([...owners], 'base', 'head', 'linux');
  for (const [cwd, last] of [['connect/web', 'test'], ['connect/service', 'test:recipient'],
    ['capture/desktop/ui/frontend', 'test:electron'], ['public/site', 'scripts/site-build/verify.mjs']]) {
    assert.ok(plan.some(command => command.cwd === cwd && command.argv.at(-1) === last), `${cwd} ${last}`);
  }
  assert.ok(plan.some(command => command.argv.includes('--run')));
  assert.ok(!commands(['desktop'], 'base', 'head', 'darwin').some(command => command.argv.includes('xvfb-run')));
  assert.ok(commands(['client'], 'base', 'head', 'darwin').some(command => command.argv[0] === 'taskpolicy'));
});

function fixture(action: (dir: string, env: NodeJS.ProcessEnv) => void) {
  const dir = mkdtempSync(join(tmpdir(), 'hook-'));
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

test('capture script changes discover offline contracts including nested and new test files', () => fixture((dir, env) => {
  for (const path of ['capture/desktop/scripts/packaging/test_installer_guard.py',
    'capture/desktop/scripts/tests/mac/test_setup.py', 'capture/desktop/scripts/linux/gui/test-gui-vm.py',
    'capture/agent/scripts/tests/test_provision_narrator.py', 'capture/desktop/scripts/new/test_new.py']) {
    const parent = path.slice(0, path.lastIndexOf('/'));
    mkdirSync(join(dir, parent), { recursive: true }); writeFileSync(join(dir, path), '');
  }
  for (const args of [['init', '-q'], ['add', '.']]) assert.equal(spawnSync('git', args, { cwd: dir, env }).status, 0);
  const suites = captureToolTests(dir);
  assert.equal(suites.length, 5);
  assert.ok(suites.some(command => command.argv.includes('capture/desktop/scripts/linux/gui')));
  assert.ok(suites.some(command => command.argv.includes('capture/desktop/scripts/new')));
  assert.ok(suites.every(command => command.argv.at(-1) === 'test*.py'));
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

test('actual feature push runs selected package tests and rejects their failure', () => fixture((dir, env) => {
  const git = (...args: string[]) => {
    const result = spawnSync('git', args, { cwd: dir, env, encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr); return result.stdout.trim();
  };
  mkdirSync(join(dir, 'enablement/local-tests'), { recursive: true });
  copyFileSync(join(root, 'enablement/local-tests/run.ts'), join(dir, 'enablement/local-tests/run.ts'));
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
