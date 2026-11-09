import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync, writeFileSync, mkdirSync, mkdtempSync, rmSync, symlinkSync} from 'node:fs';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';
import {concurrencyOnly} from './workflow-controls.ts';
import {changedPaths, controlWorkflows, workflowControlPaths} from './inputs.ts';

const root = new URL('../../', import.meta.url);
const original = readFileSync(new URL(controlWorkflows[0], root), 'utf8');
const update = (source: string) => source.replace(/  cancel-in-progress: .*/, '  cancel-in-progress: false');

test('real orchestrators accept only a changed root concurrency block and adjacent comments', () => {
  for (const path of controlWorkflows) {
    const before = readFileSync(new URL(path, root), 'utf8');
    // Test both directions so this survives the independently owned concurrency fix.
    const after = before.replace(/  cancel-in-progress: .*/, '  cancel-in-progress: false')
      .replace(/(?:#.*\n)?concurrency:/, '# Main preserves active runs.\nconcurrency:');
    assert.equal(concurrencyOnly(before, after), true, path);
    assert.equal(concurrencyOnly(after, before), true, path);
    assert.equal(concurrencyOnly(before, before), false, 'unchanged bytes are not positive proof');
  }
});

test('execution-sensitive changes, ambiguous YAML and changed alias consumers retain coverage', () => {
  const safe = update(original);
  assert.equal(concurrencyOnly(original, safe), true, 'each negative starts with a qualifying control edit');
  for (const after of [
    safe.replace('contents: read', 'contents: write'),
    safe.replace('ubuntu-24.04', 'ubuntu-22.04'),
    safe.replace('fetch-depth: 0', 'fetch-depth: 1'),
    safe.replace('actions/checkout@v4', 'actions/checkout@v3'),
    safe.replace('bun enablement/ci/select.ts', 'echo skip'),
    safe.replace('release_receipt: false', 'release_receipt: true'),
    safe.replace('needs: [linux, contracts, hermes]', 'needs: [contracts]'),
    safe.replace('branches: [main]', 'branches: [other]'),
    safe.replace('jobs:', 'env:\n  CARGO_BUILD_JOBS: 1\n\njobs:'),
    safe.replace('concurrency:', 'concurrency: &policy'),
    safe.replace('  group: repository', '  group: &execution repository'),
    safe.replace('concurrency:', 'concurrency:\n  unknown: true'),
    safe.replace('concurrency:', 'concurrency:\n  group: duplicate'),
    safe.replace('  group: repository', '  group: >\n    repository'),
    safe.replace('concurrency:', 'concurrency: {group: inline}\nother:'),
    safe.replace('cancel-in-progress:', 'cancel-in-progress: !custom'),
    safe + '\nconcurrency:\n  group: duplicate\n  cancel-in-progress: true\n',
    '---\n' + safe,
    safe.replace('permissions:', 'permissions: ['),
    safe.replace('      - uses: actions/checkout@v4', '      # changed execution-region comment\n      - uses: actions/checkout@v4'),
  ]) assert.equal(concurrencyOnly(original, after), false, after);
  const alias = original.replace('  group: repository', '  group: &execution repository')
    .replace('jobs:', 'env:\n  BUILD_INPUT: *execution\njobs:');
  assert.equal(concurrencyOnly(alias, update(alias)), false);
});

function repository(action: (git: (...args: string[]) => string, dir: string) => void) {
  const dir = mkdtempSync('/tmp/apparent-dev-workflow-inputs-');
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('GIT_')));
  const inherited = {...process.env};
  try {
    // The production helpers intentionally inherit the real repository routing.
    // Isolate this fixture's entire synchronous scope from parent hook variables.
    for (const key of Object.keys(process.env)) if (key.startsWith('GIT_')) delete process.env[key];
    const git = (...args: string[]) => execFileSync('git', args, {cwd: dir, env, encoding: 'utf8'}).trim();
    git('init', '-q'); git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'test@example.invalid');
    action(git, dir);
  } finally { Object.assign(process.env, inherited); rmSync(dir, {recursive: true, force: true}); }
}

test('Git modes/status/blobs bind proof to the full committed delta, never working bytes', () => repository((git, dir) => {
  mkdirSync(join(dir, '.github/workflows'), {recursive: true});
  const path = controlWorkflows[0], other = controlWorkflows[1];
  const commit = () => { git('add', '.'); git('commit', '-qm', 'fixture'); return git('rev-parse', 'HEAD'); };
  writeFileSync(join(dir, path), original);
  const base = commit();
  const revised = original.replace(/  cancel-in-progress: .*/, '  cancel-in-progress: false');
  writeFileSync(join(dir, path), revised); commit();
  const controls = () => workflowControlPaths(base, 'HEAD', changedPaths(base, 'HEAD', dir), dir);
  assert.deepEqual(controls(), [path]);
  writeFileSync(join(dir, path), revised.replace('contents: read', 'contents: write'));
  assert.deepEqual(controls(), [path], 'uncommitted content cannot change committed proof');
  commit();
  assert.deepEqual(controls(), [], 'mixed execution changes cannot be hidden by a safe last commit');
  git('checkout', base, '--', path); commit();
  writeFileSync(join(dir, path), revised); git('update-index', '--chmod=+x', path);
  git('add', path); git('update-index', '--chmod=+x', path); git('commit', '-qm', 'mode');
  assert.deepEqual(controls(), []);
  git('rm', '-f', path); commit(); assert.deepEqual(controls(), []);
  mkdirSync(join(dir, '.github/workflows'), {recursive: true});
  writeFileSync(join(dir, other), revised); commit(); assert.deepEqual(controls(), [], 'addition');
  git('mv', other, path); commit(); assert.deepEqual(controls(), [path], 'net regular-file modification is compared to merge-base');
  git('rm', path); mkdirSync(join(dir, '.github/workflows'), {recursive: true});
  symlinkSync('somewhere', join(dir, path)); commit(); assert.deepEqual(controls(), []);
  assert.deepEqual(workflowControlPaths('missing', 'HEAD', [path], dir), []);
}));

test('renames, unknown workflows and unavailable YAML tools do not qualify', () => repository((git, dir) => {
  const path = controlWorkflows[0];
  mkdirSync(join(dir, '.github/workflows'), {recursive: true});
  writeFileSync(join(dir, path), original);
  git('add', '.'); git('commit', '-qm', 'base'); const base = git('rev-parse', 'HEAD');
  git('mv', path, controlWorkflows[1]); git('commit', '-qm', 'rename');
  assert.deepEqual(workflowControlPaths(base, 'HEAD', changedPaths(base, 'HEAD', dir), dir), []);
  assert.deepEqual(workflowControlPaths(base, 'HEAD', ['.github/workflows/verification-platform.yml'], dir), []);
  git('mv', controlWorkflows[1], path);
  writeFileSync(join(dir, path), original.replace(/  cancel-in-progress: .*/, '  cancel-in-progress: false'));
  git('add', '.'); git('commit', '-qm', 'control');
  const bin = join(dir, 'bin'); mkdirSync(bin);
  symlinkSync(execFileSync('which', ['git'], {encoding: 'utf8'}).trim(), join(bin, 'git'));
  // Exercise the actual Node consumer with Bun absent, avoiding Bun's own cached
  // executable resolution in this Bun-hosted contract suite.
  const node = execFileSync('which', ['node'], {encoding: 'utf8'}).trim();
  const script = `import {workflowControlPaths} from ${JSON.stringify(new URL('./inputs.ts', import.meta.url).href)};
    console.log(JSON.stringify(workflowControlPaths(${JSON.stringify(base)}, 'HEAD', [${JSON.stringify(path)}], ${JSON.stringify(dir)})));`;
  assert.equal(execFileSync(node, ['--experimental-strip-types', '--input-type=module', '-e', script],
    {cwd: dir, env: {...process.env, PATH: bin}, encoding: 'utf8'}).trim(), '[]');
}));

test('invalid UTF-8 cannot turn different execution bytes into equal replacement characters', () => repository((git, dir) => {
  const path = controlWorkflows[0];
  mkdirSync(join(dir, '.github/workflows'), {recursive: true});
  writeFileSync(join(dir, path), Buffer.concat([Buffer.from(original), Buffer.from([0xff])]));
  git('add', '.'); git('commit', '-qm', 'base'); const base = git('rev-parse', 'HEAD');
  writeFileSync(join(dir, path), Buffer.concat([Buffer.from(update(original)), Buffer.from([0xfe])]));
  git('add', '.'); git('commit', '-qm', 'control plus invalid bytes');
  assert.deepEqual(workflowControlPaths(base, 'HEAD', [path], dir), []);
}));
