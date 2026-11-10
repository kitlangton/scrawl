// Run after the ordinary root build with APPARENT_BIN naming that exact binary.
// The existing Rust build metadata producer remains the sole identity authority.
import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { isAbsolute, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const version = readFileSync(join(root, 'Cargo.toml'), 'utf8').match(/^version = "([^"]+)"$/m)![1];
const environment = { ...process.env };
// Hooks export Git routing for their own checkout. Fixture Git must be isolated.
for (const key of Object.keys(environment)) {
  if (key.startsWith('GIT_')) delete environment[key];
}
let directory: string;
let emitter: string;

function run(command: string, args: string[], cwd: string, env = environment): string {
  const result = spawnSync(command, args, { cwd, env, encoding: 'utf8', timeout: 30_000 });
  assert.equal(result.status, 0, `${command}: ${result.error ?? result.stderr}`);
  return result.stdout;
}

before(() => {
  directory = mkdtempSync('/tmp/apparent-dev-cli-version-');
  emitter = join(directory, 'identity');
  copyFileSync(join(root, 'enablement/build_version.rs'), join(directory, 'build_version.rs'));
  writeFileSync(join(directory, 'main.rs'), 'mod build_version;\nfn main() { build_version::emit(); }\n');
  run('rustc', ['--edition=2024', join(directory, 'main.rs'), '-o', emitter], root);
});
after(() => { if (directory) rmSync(directory, { recursive: true, force: true }); });

function identity(source: string, githubSha = ''): Record<string, string> {
  const output = run(emitter, [], directory, {
    ...environment, CARGO_MANIFEST_DIR: source, CARGO_PKG_VERSION: version, GITHUB_SHA: githubSha,
  });
  return Object.fromEntries([...output.matchAll(/^cargo:rustc-env=([^=]+)=(.*)$/gm)]
    .map(match => [match[1], match[2]]));
}

test('ordinary long version matches the compiled candidate; short version stays compatible', () => {
  const binary = process.env.APPARENT_BIN;
  assert.ok(binary && isAbsolute(binary), 'APPARENT_BIN must name the freshly built production CLI');
  const expected = identity(root, process.env.GITHUB_SHA);
  const outside = join(directory, 'outside');
  mkdirSync(outside);
  const config = join(outside, 'config.json');
  const env = {
    ...environment, HOME: outside, XDG_CONFIG_HOME: outside, XDG_CACHE_HOME: outside,
    APPARENT_CONFIG: config, PATH: '',
    APPARENT_VERSION_LABEL: 'runtime-forgery', APPARENT_COMMIT_SHA: 'f'.repeat(40),
    APPARENT_BUILD_MODIFIED: 'false', APPARENT_RELEASE_VERSION: 'runtime-release',
    APPARENT_RELEASE_CHANNEL: 'stable', APPARENT_REPORTING_ENVIRONMENT: 'production',
    GITHUB_SHA: 'e'.repeat(40), CARGO_PKG_VERSION: '99.99.99',
  };
  // No Git or helper executable is available. Neither form may start the app,
  // create config/cache state, or replace the build identity with runtime inputs.
  for (const cwd of [root, outside]) {
    assert.equal(run(binary, ['--version'], cwd, env), `apparent ${expected.APPARENT_VERSION_LABEL}\n`);
    assert.equal(run(binary, ['-V'], cwd, env), `apparent ${version}\n`);
  }
  assert.deepEqual(readdirSync(outside), []);
  const original = '{"privateUnknown":"preserve","experiments":{"hostedEncryption":true}}\n';
  writeFileSync(config, original, { mode: 0o600 });
  assert.equal(run(binary, ['--version'], outside, env), `apparent ${expected.APPARENT_VERSION_LABEL}\n`);
  assert.equal(readFileSync(config, 'utf8'), original);
  assert.deepEqual(readdirSync(outside), ['config.json']);
});

test('same-package builds retain distinct revisions, tracked modifications and archive fallback', () => {
  const checkout = join(directory, 'checkout');
  mkdirSync(checkout);
  const git = (...args: string[]) => run('git', [
    '-c', 'user.name=Version fixture', '-c', 'user.email=version@example.invalid',
    '-c', 'commit.gpgsign=false', '-c', 'core.hooksPath=/dev/null', ...args,
  ], checkout, { ...environment, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_NOSYSTEM: '1' }).trim();
  git('init', '-q');
  const tracked = join(checkout, 'source');
  writeFileSync(tracked, 'first build\n');
  git('add', 'source');
  git('commit', '-qm', 'first build');
  const firstSha = git('rev-parse', 'HEAD');
  const first = identity(checkout, 'f'.repeat(40));
  assert.equal(first.APPARENT_COMMIT_SHA, firstSha, 'checkout identity wins over CI fallback');
  assert.equal(first.APPARENT_VERSION_LABEL, `${version} · ${firstSha.slice(0, 7)}`);
  assert.equal(first.APPARENT_BUILD_MODIFIED, 'false');

  writeFileSync(tracked, 'second build\n');
  const modified = identity(checkout);
  assert.equal(modified.APPARENT_COMMIT_SHA, firstSha);
  assert.equal(modified.APPARENT_BUILD_MODIFIED, 'true');
  assert.equal(modified.APPARENT_VERSION_LABEL, `${first.APPARENT_VERSION_LABEL} +modified`);
  git('add', 'source');
  git('commit', '-qm', 'second build');
  const secondSha = git('rev-parse', 'HEAD');
  const second = identity(checkout);
  assert.equal(second.APPARENT_VERSION_LABEL, `${version} · ${secondSha.slice(0, 7)}`);
  assert.notEqual(first.APPARENT_VERSION_LABEL, second.APPARENT_VERSION_LABEL);
  assert.equal(second.APPARENT_BUILD_MODIFIED, 'false');

  // An archive inside an unrelated checkout must never inherit its parent's SHA.
  const archive = join(checkout, 'archive');
  mkdirSync(archive);
  assert.equal(existsSync(join(archive, '.git')), false);
  for (const fallback of ['', 'not-a-revision', '2362f2b']) {
    const unknown = identity(archive, fallback);
    assert.equal(unknown.APPARENT_COMMIT_SHA, 'unknown');
    assert.equal(unknown.APPARENT_VERSION_LABEL, `${version} · build unknown`);
    assert.equal(unknown.APPARENT_BUILD_MODIFIED, 'false');
  }
  const fromCi = identity(archive, firstSha);
  assert.equal(fromCi.APPARENT_COMMIT_SHA, firstSha);
  assert.equal(fromCi.APPARENT_VERSION_LABEL, first.APPARENT_VERSION_LABEL);
  assert.equal(identity(checkout).APPARENT_BUILD_MODIFIED, 'false', 'untracked files keep the existing contract');
});
