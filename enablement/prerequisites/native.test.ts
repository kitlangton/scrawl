import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, chmodSync, rmSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, dirname } from 'node:path';
import { checkNativeInputs } from './native.ts';
import { spawnSync } from 'node:child_process';

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
  for (const file of ['tests/reference-native.cjs', 'electron/main.cjs', 'electron/preload.cjs', 'electron/runtime.cjs']) {
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
  for (const marker of ['--preflight', 'npm --prefix capture/desktop/ui/frontend ci', 'node_modules/electron/install.js', 'enablement/prerequisites/native.ts']) {
    assert.ok(rust.indexOf(marker) >= 0 && rust.indexOf(marker) < firstCargo, marker);
  }
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
