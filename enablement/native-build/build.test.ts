import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';

const repository = resolve(import.meta.dirname, '../..');
const production = join(repository, 'enablement/native-build/build.ts');
function fixture(t: { after: (fn: () => void) => void }) {
  const root = mkdtempSync(join(tmpdir(), 'apparent-dev-native-build-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  function put(path: string, body: string) {
    mkdirSync(join(root, path, '..'), { recursive: true });
    writeFileSync(join(root, path), body);
  }
  // Obtain the finite production source inventory; don't maintain a test copy.
  const source = readFileSync(production, 'utf8');
  for (const [path] of source.matchAll(/'(capture\/[^']+\.(?:swift|metal)|shared\/native\/[^']+\.(?:swift|plist))'/g)) {
    put(path.slice(1, -1), path.endsWith("Info.plist'") ? '<string>dev.kitlangton.apparent.helper</string>' : 'synthetic input');
  }
  put('enablement/native-build/build.ts', source);
  put('sdk/SDKSettings.json', '{"Version":"26"}');
  put('bin/compiler', 'synthetic compiler identity');
  put('bin/xcrun', `#!/bin/sh
set -eu
if [ "$1" = --sdk ]; then shift 2; fi
case "$1" in
  --show-sdk-path) printf '%s\\n' "$FAKE_ROOT/sdk"; exit ;;
  --show-sdk-version) echo 26.0; exit ;;
  --show-sdk-build-version) echo 26A; exit ;;
  --find) printf '%s\\n' "$FAKE_ROOT/bin/compiler"; exit ;;
esac
tool=$1; shift
if [ "$1" = --version ]; then echo "$tool version 1"; exit; fi
printf '%s\\n' "$tool" >> "$FAKE_ROOT/invocations"
printf '%s\\n' "$@" > "$FAKE_ROOT/$tool-arguments"
if [ "$FAIL_TOOL" = "$tool" ]; then exit 42; fi
output=
previous=
for arg do
  if [ "$previous" = -o ]; then output=$arg; fi
  previous=$arg
done
test -n "$output"
printf 'synthetic compiled %s\\n' "$tool" > "$output"
if [ "$tool" = swiftc ]; then chmod +x "$output"; fi
if [ "$tool" = swiftc ]; then cp "$arg" "$FAKE_ROOT/helper-plist"; fi
`);
  chmodSync(join(root, 'bin/xcrun'), 0o700);
  const out = join(root, 'target/debug/build/native/out');
  mkdirSync(out, { recursive: true });
  const env = { ...process.env, PATH: join(root, 'bin') + ':' + process.env.PATH,
    FAKE_ROOT: root, FAIL_TOOL: '', APPARENT_HELPER_BUNDLE_ID: undefined, APPARENT_PREVIEW_BUILD: undefined };
  const run = (arch = 'x86_64', extra = {}) => spawnSync(process.execPath,
    ['--experimental-strip-types', production, root, out, arch], { env: { ...env, ...extra }, encoding: 'utf8' });
  const count = () => existsSync(join(root, 'invocations')) ? readFileSync(join(root, 'invocations'), 'utf8').trim().split('\n') : [];
  return { root, out, env, put, run, count };
}

test('native outputs: cold build, verified reuse, restaging and per-artifact invalidation', t => {
  const f = fixture(t);
  assert.equal(f.run().status, 0);
  assert.deepEqual(f.count(), ['metal', 'metallib', 'metal', 'metallib', 'swiftc']);
  const swift = readFileSync(join(f.root, 'swiftc-arguments'), 'utf8').trim().split('\n');
  assert.deepEqual(swift.slice(0, 6), ['-O', '-swift-version', '5', '-target', 'x86_64-apple-macos15.0', join(f.root, 'capture/engine/native/Export.swift')]);
  assert.deepEqual(swift.filter(arg => arg.endsWith('.swift')).map(arg => arg.slice(arg.lastIndexOf('/') + 1)),
    ['Export.swift', 'LiveTranscribe.swift', 'Media.swift', 'Mirror.swift', 'Onboarding.swift', 'OnboardingBrand.swift',
      'OnboardingDemo.swift', 'PermissionPanel.swift', 'PlaybackVolume.swift', 'ScreenProbe.swift', 'Settings.swift', 'ShortcutKeys.swift', 'main.swift']);
  assert.deepEqual(swift.slice(-8, -1), ['-Xlinker', '-sectcreate', '-Xlinker', '__TEXT', '-Xlinker', '__info_plist', '-Xlinker']);
  rmSync(join(f.root, 'target/debug/apparent-helper'));
  assert.equal(f.run().status, 0);
  assert.equal(f.count().length, 5);
  assert.equal(readFileSync(join(f.root, 'target/debug/apparent-helper'), 'utf8'), 'synthetic compiled swiftc\n');
  f.put('capture/engine/native/Media.swift', 'changed native input');
  assert.equal(f.run().status, 0);
  assert.equal(f.count().length, 6);
  writeFileSync(join(f.out, 'pill.metallib'), 'corrupted');
  assert.equal(f.run().status, 0);
  assert.deepEqual(f.count().slice(6), ['metal', 'metallib']);
  f.put('shared/native/Info.plist', '<string>dev.kitlangton.apparent.helper</string> changed');
  assert.equal(f.run().status, 0);
  assert.equal(f.count().at(-1), 'swiftc');
  assert.equal(f.count().length, 9);
});

test('SDK, exact tool bytes, target and preview identity cannot reuse incompatible outputs', t => {
  const f = fixture(t);
  assert.equal(f.run().status, 0);
  f.put('sdk/SDKSettings.json', '{"Version":"26.1"}');
  assert.equal(f.run().status, 0);
  assert.equal(f.count().length, 10);
  f.put('bin/compiler', 'new compiler bytes, same reported version');
  assert.equal(f.run().status, 0);
  assert.equal(f.count().length, 15);
  assert.equal(f.run('aarch64').status, 0);
  assert.equal(f.count().length, 20);
  assert.equal(f.run('aarch64', { APPARENT_HELPER_BUNDLE_ID: 'dev.simonvanlaak.apparent.preview.helper' }).status, 0);
  assert.equal(f.count().length, 25);
  assert.equal(readFileSync(join(f.root, 'helper-plist'), 'utf8'), '<string>dev.simonvanlaak.apparent.preview.helper</string>');
  assert.match(readFileSync(join(f.root, 'swiftc-arguments'), 'utf8'), /arm64-apple-macos15\.0/);
  assert.equal(f.run('aarch64').status, 0);
  assert.equal(f.count().length, 30);
  assert.equal(readFileSync(join(f.root, 'helper-plist'), 'utf8'), '<string>dev.kitlangton.apparent.helper</string>');
  assert.notEqual(f.run('aarch64', { APPARENT_HELPER_BUNDLE_ID: 'untrusted' }).status, 0);
  assert.equal(f.count().length, 30);
});

test('partial compilation never publishes a successful receipt or replaces a prior output', t => {
  const f = fixture(t);
  assert.equal(f.run().status, 0);
  const receipt = readFileSync(join(f.out, 'native-build.json'), 'utf8');
  const output = readFileSync(join(f.out, 'apparent-helper'), 'utf8');
  f.put('capture/engine/native/Media.swift', 'changed');
  assert.notEqual(f.run('x86_64', { FAIL_TOOL: 'swiftc' }).status, 0);
  assert.equal(readFileSync(join(f.out, 'native-build.json'), 'utf8'), receipt);
  assert.equal(readFileSync(join(f.out, 'apparent-helper'), 'utf8'), output);
  assert.equal(f.run().status, 0);
  assert.equal(f.count().length, 7);
  writeFileSync(join(f.out, 'native-build.json'), 'broken');
  assert.equal(f.run().status, 0);
  assert.equal(f.count().length, 12);
});

test('cached helper mode and destination symlinks do not bypass executable validation or overwrite an unrelated file', t => {
  const f = fixture(t);
  assert.equal(f.run().status, 0);
  chmodSync(join(f.out, 'apparent-helper'), 0o600);
  f.put('unrelated', 'preserve');
  rmSync(join(f.root, 'target/debug/apparent-helper'));
  symlinkSync(join(f.root, 'unrelated'), join(f.root, 'target/debug/apparent-helper'));
  assert.equal(f.run().status, 0);
  assert.equal(f.count().length, 6);
  assert.equal(readFileSync(join(f.root, 'unrelated'), 'utf8'), 'preserve');
});

test('the production Cargo adapter remains fresh on no-op and reuses native outputs after a metadata-only rerun',
  { skip: process.env.APPARENT_NATIVE_CARGO_FIXTURE !== '1' }, t => {
    const f = fixture(t);
    const source = readFileSync(join(repository, 'build.rs'), 'utf8');
    const start = source.indexOf('fn compile_mac_native() {');
    assert.ok(start > 0);
    f.put('build.rs', 'use std::path::PathBuf;\nuse std::process::Command;\nfn main() { println!("cargo:rerun-if-changed=metadata"); compile_mac_native(); }\n' + source.slice(start));
    f.put('metadata', 'first revision');
    f.put('src/main.rs', 'fn main() {}');
    f.put('Cargo.toml', '[package]\nname="native-cache-fixture"\nversion="0.1.0"\nedition="2024"\n');
    const cargo = () => spawnSync('cargo', ['build', '--offline', '-vv'], {
      cwd: f.root, env: { ...f.env, CARGO_TARGET_DIR: join(f.root, 'target'), RUSTC_WRAPPER: '',
        CARGO_ENCODED_RUSTFLAGS: '', RUSTFLAGS: '', TMPDIR: f.root }, encoding: 'utf8', timeout: 60000 });
    let r = cargo();
    assert.equal(r.status, 0, r.stderr);
    assert.equal(f.count().length, 5);
    r = cargo();
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.stderr, /Fresh native-cache-fixture/);
    assert.equal(f.count().length, 5);
    f.put('metadata', 'second revision');
    r = cargo();
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.stdout, /verified reuse/);
    assert.equal(f.count().length, 5);
    r = cargo();
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.stderr, /Fresh native-cache-fixture/);
    assert.equal(f.count().length, 5);
  });
