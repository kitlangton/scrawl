import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, chmodSync, rmSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, dirname } from 'node:path';
import { checkNativeInputs } from './native.ts';
import { spawnSync } from 'node:child_process';
import '../native-build/build.test.ts';

function fixture(t: any) {
  const root = mkdtempSync(resolve(tmpdir(), 'native-inputs-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const put = (file: string, text = 'fixture') => {
    const path = resolve(root, file);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, text);
    return path;
  };
  put('capture/desktop/ui/frontend/package.json', '{}');
  const electron = put('capture/desktop/ui/frontend/node_modules/electron/bin', 'synthetic executable');
  chmodSync(electron, 0o700);
  put('capture/desktop/ui/frontend/node_modules/electron/index.js', 'module.exports = __dirname + "/bin"');
  for (const file of ['tests/reference-native.cjs', 'electron/main.cjs', 'electron/preload.cjs', 'electron/runtime.cjs', 'electron/storage-policy.cjs']) {
    put('capture/desktop/ui/frontend/' + file);
  }
  return { root, put, electron };
}

test('ready Linux inputs resolve the installed executable without launching it', t => {
  const f = fixture(t);
  assert.equal(checkNativeInputs(f.root, 'linux'), f.electron);
});
test('missing Electron download fails before tests can run', t => {
  const f = fixture(t);
  rmSync(f.electron);
  assert.throws(() => checkNativeInputs(f.root, 'linux'), /ENOENT/);
});
test('missing generated Storage policy fails before Cargo', t => {
  const f = fixture(t);
  rmSync(resolve(f.root, 'capture/desktop/ui/frontend/electron/storage-policy.cjs'));
  assert.throws(() => checkNativeInputs(f.root, 'linux'), /ENOENT/);
});
test('installer/setup failure propagates through Electron resolution', t => {
  const f = fixture(t);
  f.put('capture/desktop/ui/frontend/node_modules/electron/index.js', 'throw new Error("Electron setup failed")');
  assert.throws(() => checkNativeInputs(f.root, 'linux'), /Electron setup failed/);
});
test('non-executable Electron fails', t => {
  const f = fixture(t);
  chmodSync(f.electron, 0o600);
  assert.throws(() => checkNativeInputs(f.root, 'linux'), /EACCES/);
});
test('Mac packaging and native fixture inputs are required', t => {
  const f = fixture(t);
  assert.throws(() => checkNativeInputs(f.root, 'darwin'), /ENOENT/);
});
test('workflow installs and checks prerequisites before every Cargo command, preserves native checks and cleanup', () => {
  const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8');
  const rust = workflow.split('\n  rust:')[1].split('\n  release-ready:')[0];
  const firstCargo = rust.indexOf('cargo ');
  for (const marker of ['--preflight', 'npm --prefix capture/desktop/ui/frontend ci', 'node_modules/electron/install.js', 'scripts/build-storage-policy.ts', 'enablement/prerequisites/native.ts']) {
    assert.ok(rust.indexOf(marker) >= 0 && rust.indexOf(marker) < firstCargo, marker);
  }
  assert.ok(rust.indexOf('scripts/build-storage-policy.ts') < rust.indexOf('enablement/prerequisites/native.ts'));
  assert.equal((rust.match(/npm --prefix capture\/desktop\/ui\/frontend ci/g) || []).length, 1);
  assert.match(rust, /timeout-minutes: 5/);
  assert.match(rust, /_electron_rust_roundtrip -- --ignored --nocapture/);
  assert.match(rust, /mac-storage-native.cjs/);
  assert.match(rust, /Clean private prerequisite temporary root\n        if: always\(\)/);
});

test('workflow aborts before readiness and Cargo when locked install or download fails', t => {
  const f = fixture(t);
  const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8');
  const setup = workflow.split('      - name: Prepare locked native prerequisites before first Cargo')[1]
    .split('        run: |\n')[1].split('      - name:')[0]
    .split('\n').map(line => line.replace(/^          /, '')).join('\n');
  const bin = resolve(f.root, 'bin');
  mkdirSync(bin);
  for (const failure of ['npm', 'node']) {
    for (const command of ['npm', 'node', 'cargo']) {
      const script = f.put('bin/' + command,
        '#!/bin/sh\necho "' + command + ' $*" >> "$TRACE"\n' +
        (command === failure ? 'exit 19\n' : 'exit 0\n'));
      chmodSync(script, 0o700);
    }
    const trace = f.put('trace-' + failure, '');
    const result = spawnSync('bash', ['-e', '-c', setup + '\ncargo build'], {
      env: { ...process.env, PATH: bin + ':' + process.env.PATH, TRACE: trace },
      encoding: 'utf8',
    });
    assert.equal(result.status, 19);
    const log = readFileSync(trace, 'utf8');
    assert.doesNotMatch(log, /cargo|native.ts/);
    if (failure === 'node') assert.match(log, /node_modules\/electron\/install.js/);
  }
});

test('packaging inherits the prepared Electron download cache without bypassing its installer', t => {
  const f = fixture(t);
  const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8');
  const rust = workflow.split('\n  rust:')[1].split('\n  release-ready:')[0];
  const cacheStep = rust.split('      - name: Cache Electron downloads')[1].split('      - name:')[0];
  const downloadCache = cacheStep.match(/^          path: (.+)$/m)?.[1];
  assert.ok(downloadCache, 'The prepared archive must retain its existing cache action');
  const preparation = rust.split('      - name: Prepare locked native prerequisites before first Cargo')[1]
    .split('      - name:')[0];
  const prepareCache = preparation.split('        run: |')[0]
    .match(/^          electron_config_cache: (.+)$/m)?.[1];
  assert.equal(prepareCache, downloadCache, 'First installation must fill the retained cache');
  const expanded = (value: string) => value.replace('${{ runner.temp }}', f.root);
  const cache = expanded(downloadCache);
  const bin = resolve(f.root, 'bin');
  mkdirSync(bin);
  for (const name of ['npm', 'node']) {
    const command = f.put('bin/' + name, '#!/bin/sh\nset -eu\n' + (name === 'node' ? String.raw`
if [ "$1" = capture/desktop/ui/frontend/node_modules/electron/install.js ]; then
  mkdir -p "$electron_config_cache"
  printf %s 'prepared before Cargo' > "$electron_config_cache/prepared-archive-marker"
fi
` : '') + '\nexit 0\n');
    chmodSync(command, 0o700);
  }
  const githubEnv = f.put('github-env', '');
  const setup = preparation.split('        run: |\n')[1]
    .split('\n').map(line => line.replace(/^          /, '')).join('\n');
  const prepared = spawnSync('bash', ['-e', '-c', setup], {
    env: { ...process.env, PATH: bin + ':' + process.env.PATH,
      electron_config_cache: cache, GITHUB_ENV: githubEnv }, encoding: 'utf8',
  });
  assert.equal(prepared.status, 0, prepared.stderr);
  const env: Record<string, string | undefined> = { ...process.env, ELECTRON_SKIP_BINARY_DOWNLOAD: '1' };
  delete env.electron_config_cache;
  // Model Actions applying the actual preparation step's environment-file writes
  // to a later packaging step; do not leak the preparation-only process env.
  for (const line of readFileSync(githubEnv, 'utf8').split('\n')) {
    if (line.startsWith('electron_config_cache=')) env.electron_config_cache = line.slice('electron_config_cache='.length);
  }
  // Run the real packaging entry point through its installer boundary. Its
  // subprocess mock stops there, before build/staging/signing or any network.
  const probe = String.raw`
import importlib.util, os, sys
from pathlib import Path
from unittest.mock import patch
sys.dont_write_bytecode = True
script, cache, destination = sys.argv[1:]
spec = importlib.util.spec_from_file_location('stage_settings', script)
stage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stage)
calls = []
class InspectedInstaller(Exception): pass
def command(args, **kwargs):
    calls.append(args)
    assert kwargs['check'] is True
    if args == ['node', 'node_modules/electron/install.js']:
        assert kwargs['env'].get('electron_config_cache') == cache, 'packaging lost the prepared download cache'
        assert 'ELECTRON_SKIP_BINARY_DOWNLOAD' not in kwargs['env'], 'packaging must run its ordinary installer'
        assert (Path(cache) / 'prepared-archive-marker').read_text() == 'prepared before Cargo'
        raise InspectedInstaller()
with patch.object(sys, 'argv', [script, destination]), patch.object(stage.subprocess, 'run', side_effect=command):
    try: stage.main()
    except InspectedInstaller: pass
assert calls == [['npm', 'ci', '--ignore-scripts', '--no-audit', '--no-fund'], ['node', 'node_modules/electron/install.js']]
assert not Path(destination).exists()
`;
  const result = spawnSync('python3', ['-c', probe, resolve('capture/desktop/scripts/packaging/stage-settings.py'),
    cache, resolve(f.root, 'staged-app')], { env, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
});
