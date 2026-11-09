import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createHmac} from 'node:crypto';
import {spawnSync, execFileSync} from 'node:child_process';
import {chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, symlinkSync, writeFileSync} from 'node:fs';
import {join} from 'node:path';
import {digest, receiptStore, reuse} from './receipt.ts';
import {files, glibcNative, serviceEnvironment, serviceTests} from './service.ts';
import {root} from './run.ts';

function fixture(action: (dir: string, env: NodeJS.ProcessEnv) => void) {
  const dir = mkdtempSync('/tmp/apparent-dev-reuse-test-');
  const env = {...process.env};
  for (const key of Object.keys(env)) if (key.startsWith('GIT_')) delete env[key];
  env.GIT_CONFIG_GLOBAL = '/dev/null'; env.GIT_CONFIG_NOSYSTEM = '1';
  try { action(dir, env); } finally {
    rmSync(receiptStore(dir), {recursive: true, force: true});
    rmSync(dir, {recursive: true, force: true});
  }
}

test('real command runs, reuses, invalidates and never saves a failure', () => fixture((dir, env) => {
  const input = join(dir, 'input'), output = join(dir, 'count');
  writeFileSync(input, 'pass');
  const argv = [process.execPath, '-e', `const fs = require('fs');
    fs.appendFileSync(${JSON.stringify(output)}, 'run\\n');
    process.exit(fs.readFileSync(${JSON.stringify(input)}, 'utf8') === 'pass' ? 0 : 7)`];
  let environment = {...env};
  const snapshot = () => {
    const value = files([input], [dir]);
    return {key: digest(JSON.stringify([value.key, argv, environment])), stable: value.stable, head: 'a'.repeat(40)};
  };
  const messages: string[] = [];
  const run = () => {
    const result = spawnSync(argv[0], argv.slice(1), {env: environment});
    if (result.status !== 0) throw Error(`failed ${result.status}`);
  };
  const check = () => reuse(dir, snapshot, run, message => messages.push(message));
  check(); check();
  assert.equal(readFileSync(output, 'utf8'), 'run\n');
  assert.match(messages.at(-1)!, /reused/);
  environment = {...environment, QUALIFIED_TEST_SETTING: 'changed'};
  check(); assert.equal(readFileSync(output, 'utf8'), 'run\nrun\n');
  writeFileSync(input, 'fail');
  assert.throws(check, /failed 7/);
  assert.equal(existsSync(join(receiptStore(dir), 'success.json')), false);
  assert.throws(check, /failed 7/);
  writeFileSync(input, 'pass'); check(); check();
  assert.equal(readFileSync(output, 'utf8').split('\n').length - 1, 5);
}));

test('receipt corruption, expiry, unsafe modes, contention and deletion execute fresh', () => fixture(dir => {
  const input = join(dir, 'input'); writeFileSync(input, 'same');
  const snapshot = () => ({key: files([input], [dir]).key, stable: 'stable', head: 'b'.repeat(40)});
  let count = 0;
  const check = () => reuse(dir, snapshot, () => {count++;}, () => {});
  check(); check(); assert.equal(count, 1);
  const store = receiptStore(dir), path = join(store, 'success.json');
  writeFileSync(path, '{}'); check(); assert.equal(count, 2);
  const saved = JSON.parse(readFileSync(path, 'utf8'));
  const forged = JSON.parse(saved.payload); forged.elapsedMs = 0;
  writeFileSync(path, JSON.stringify({...saved, payload: JSON.stringify(forged)}));
  check(); assert.equal(count, 3, 'mutated payload with old seal rejected');
  for (const finished of [Date.now() - 7 * 60 * 60 * 1000, Date.now() + 60_000]) {
    const data = JSON.parse(JSON.parse(readFileSync(path, 'utf8')).payload);
    const payload = JSON.stringify({...data, finished});
    const seal = createHmac('sha256', readFileSync(join(store, 'key'))).update(payload).digest('hex');
    writeFileSync(path, JSON.stringify({payload, seal})); check();
  }
  assert.equal(count, 5, 'validly sealed stale/future evidence rejected');
  chmodSync(path, 0o644); check(); assert.equal(count, 6);
  mkdirSync(join(store, 'lock'), {mode: 0o700});
  check(); assert.equal(count, 7, 'concurrent/interrupted holder cannot authorize a hit');
  rmSync(join(store, 'lock'), {recursive: true});
  rmSync(store, {recursive: true}); check(); assert.equal(count, 8);
  check(); assert.equal(count, 8);
}));

test('changed and mutate-restore inputs during execution cannot publish success', () => fixture(dir => {
  const input = join(dir, 'input'); writeFileSync(input, 'one');
  const snapshot = () => {
    const value = files([input], [dir]);
    return {key: value.key, stable: value.stable, head: 'c'.repeat(40)};
  };
  for (const restore of [false, true]) {
    reuse(dir, snapshot, () => {writeFileSync(input, 'two'); if (restore) writeFileSync(input, 'one');}, () => {});
    assert.equal(existsSync(join(receiptStore(dir), 'success.json')), false);
    writeFileSync(input, 'one');
  }
  reuse(dir, snapshot, () => {}, () => {});
  let calls = 0, ran = false;
  reuse(dir, () => {
    const result = snapshot();
    if (++calls === 2) writeFileSync(input, 'changed during lookup');
    return calls === 2 ? snapshot() : result;
  }, () => {ran = true;}, () => {});
  assert.ok(ran);
}));

test('file identities include bytes, modes, link targets, inventory and dependency reinstall', () => fixture(dir => {
  const input = join(dir, 'dependency'); writeFileSync(input, 'same');
  const first = files([input], [dir]);
  writeFileSync(join(dir, 'reinstalled'), 'same'); renameSync(join(dir, 'reinstalled'), input);
  const reinstall = files([input], [dir]);
  assert.equal(first.key, reinstall.key); assert.notEqual(first.stable, reinstall.stable);
  chmodSync(input, 0o755); assert.notEqual(reinstall.key, files([input], [dir]).key);
  const linked = join(dir, 'linked'); symlinkSync('dependency', linked);
  const before = files([linked], [dir]);
  writeFileSync(input, 'mutated install'); assert.notEqual(before.key, files([linked], [dir]).key);
  rmSync(linked); symlinkSync('/etc/passwd', linked);
  assert.throws(() => files([linked], [dir]), /external symlink/);
}));

test('receipt symlinks and foreign keys never authenticate evidence', () => fixture(dir => {
  const store = receiptStore(dir), outside = join(dir, 'outside');
  const snapshot = () => ({key: 'key', stable: 'stable', head: 'd'.repeat(40)});
  let calls = 0;
  const check = () => reuse(dir, snapshot, () => {calls++;}, () => {});
  check();
  writeFileSync(outside, readFileSync(join(store, 'success.json')), {mode: 0o600});
  rmSync(join(store, 'success.json')); symlinkSync(outside, join(store, 'success.json'));
  check(); assert.equal(calls, 2);
  writeFileSync(join(store, 'key'), Buffer.alloc(32, 1)); check(); assert.equal(calls, 3);
  rmSync(join(store, 'key')); symlinkSync(outside, join(store, 'key'));
  check(); assert.equal(calls, 4);
  assert.ok(readFileSync(outside).length > 0, 'external target is untouched');
}));

test('operational IDs are absent from execution, semantic environment remains an input', () => {
  const env = serviceEnvironment({PATH: '/bin', CUSTOM: 'input', CODEX_TOOL_CALL_ID: 'volatile', INVOCATION_ID: 'volatile'});
  assert.deepEqual(env, {PATH: '/bin', CUSTOM: 'input', CLOUDFLARE_CF_FETCH_ENABLED: 'false'});
  assert.equal(serviceTests({argv: ['npm', 'test'], cwd: 'connect/service'}, root, {}, undefined, () => assert.fail()), false);
  assert.equal(serviceTests({argv: ['cargo', 'test'], cwd: '.'}, root, {}, undefined, () => assert.fail()), false);
});

test('glibc runtime closure excludes only the inactive optional musl loaders', () => {
  for (const path of ['@img/sharp-linuxmusl-x64/lib/addon.node', '@parcel/watcher-linux-x64-musl/watcher.node',
    'lightningcss-linux-x64-musl/addon.node']) assert.equal(glibcNative(`/repo/node_modules/${path}`), false);
  for (const path of ['@img/sharp-linux-x64/lib/addon.node', '@parcel/watcher-linux-x64-glibc/watcher.node',
    '@cloudflare/workerd-linux-64/bin/workerd', 'unknown/addon.node', 'unknown-linux-x64-musl/addon.node']) assert.equal(glibcNative(`/repo/node_modules/${path}`), true);
});

test('installed Git hook keeps fresh guards around ancestry-only reuse and failed changed inputs', () => fixture((dir, env) => {
  const git = (...args: string[]) => execFileSync('git', args, {cwd: dir, env, encoding: 'utf8'}).trim();
  git('init', '-q'); git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'fixture@example.invalid');
  const hooks = join(dir, '.git/hooks'); git('config', 'core.hooksPath', hooks);
  copyFileSync(join(root, '.githooks/pre-push'), join(hooks, 'pre-push'));
  mkdirSync(join(dir, 'enablement/local-tests'), {recursive: true});
  copyFileSync(join(root, 'enablement/local-tests/receipt.ts'), join(dir, 'enablement/local-tests/receipt.ts'));
  const input = join(dir, 'input'); writeFileSync(input, 'pass');
  // Synthetic test phase behind the actual installed hook. The real package
  // suite has a separate measured proof; this fixture exercises Git admission.
  writeFileSync(join(dir, 'enablement/local-tests/run.ts'), `
    import {execFileSync, spawnSync} from 'node:child_process';
    import {appendFileSync, readFileSync} from 'node:fs';
    import {digest, reuse} from './receipt.ts';
    const env = {...process.env};
    for (const key of execFileSync('git', ['rev-parse', '--local-env-vars'], {encoding:'utf8'}).trim().split('\\n')) delete env[key];
    const git = (...args) => execFileSync('git', args, {env, encoding:'utf8'}).trim();
    appendFileSync('.git/fresh-guards', git('rev-parse', 'HEAD') + '\\n');
    if (process.env.FAIL_FRESH_GUARD) throw Error('fresh guard failed');
    const snapshot = () => ({key:digest(git('rev-parse', 'HEAD^{tree}')), stable:'same', head:git('rev-parse','HEAD')});
    reuse(process.cwd(), snapshot, () => {
      appendFileSync('.git/executions', 'run\\n');
      if (readFileSync('input','utf8') !== 'pass') throw Error('changed input failed');
    });
  `);
  git('add', '.'); git('commit', '-qm', 'baseline');
  git('update-ref', 'refs/remotes/origin/main', git('rev-parse', 'HEAD'));
  const remote = join(dir, '.git/remote.git'); git('init', '--bare', '-q', remote); git('remote', 'add', 'origin', remote);
  const push = (ref: string, extra = {}) => spawnSync('git', ['push', 'origin', `HEAD:refs/heads/${ref}`], {cwd: dir, env: {...env, ...extra}, encoding: 'utf8'});
  assert.equal(push('first').status, 0);
  git('commit', '--allow-empty', '-qm', 'ancestry only');
  const reused = push('retry'); assert.equal(reused.status, 0, reused.stderr); assert.match(reused.stderr, /reused/);
  assert.equal(readFileSync(join(dir, '.git/executions'), 'utf8'), 'run\n');
  assert.equal(readFileSync(join(dir, '.git/fresh-guards'), 'utf8').trim().split('\n').length, 2);
  assert.notEqual(push('fresh-failure', {FAIL_FRESH_GUARD: '1'}).status, 0);
  writeFileSync(input, 'dirty'); assert.match(push('dirty').stderr, /working tree is dirty/);
  git('update-index', '--assume-unchanged', 'input'); assert.match(push('hidden').stderr, /hidden index flags/);
  git('update-index', '--no-assume-unchanged', 'input');
  git('add', '.'); git('commit', '-qm', 'changed input');
  const failed = push('failed'); assert.notEqual(failed.status, 0); assert.match(failed.stderr, /changed input failed/);
  assert.notEqual(push('failed-again').status, 0);
  assert.equal(existsSync(join(receiptStore(dir), 'success.json')), false);
  assert.equal(readFileSync(join(dir, '.git/executions'), 'utf8').trim().split('\n').length, 3);
}));
