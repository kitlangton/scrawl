import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, renameSync, rmSync, symlinkSync, utimesSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';

const repository = resolve(import.meta.dirname, '../..');
const production = join(repository, 'enablement/native-build/build.ts');
function fixture(t: { after: (fn: () => void) => void }) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'apparent-dev-native-build-')));
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
  // Redirect only OS-owned absolute paths; execute the production policy and
  // Rust adapter against real symlinks without touching the host. Keep the
  // standard /var link's relative target (private/var) unchanged.
  assert.ok(source.includes("'/var/db/xcode_select_link'"));
  put('enablement/native-build/build.ts', source.replaceAll('/var/db/xcode_select_link', join(root, 'selector'))
    .replaceAll("'/var'", JSON.stringify(join(root, 'var')))
    .replaceAll("'/private/var'", JSON.stringify(join(root, 'private/var'))));
  const developer = join(root, 'Xcode.app/Contents/Developer');
  const otherDeveloper = join(root, 'Other.app/Contents/Developer');
  for (const app of ['Xcode.app', 'Other.app']) {
    put(`${app}/Contents/Developer/unrelated/data`, 'unrelated Xcode content');
    put(`${app}/Contents/Info.plist`, 'Xcode metadata');
    put(`${app}/Contents/version.plist`, 'Xcode version');
  }
  symlinkSync(developer, join(root, 'selector'));
  put('bin/xcode-select', `#!/bin/sh
set -eu
test "$1" = -p
selected=\${DEVELOPER_DIR:-$FAKE_ROOT/selector}
printf '%s\\n' "$selected"
`);
  chmodSync(join(root, 'bin/xcode-select'), 0o700);
  put('sdk/SDKSettings.json', '{"Version":"26"}');
  for (const tool of ['swiftc', 'metal', 'metallib', 'swift-frontend', 'clang', 'ld']) put(`bin/${tool}`, `synthetic ${tool} identity`);
  put('bin/xcrun', `#!/bin/sh
set -eu
# Discovery can echo raw or invalid overrides. Model xcrun's validation
# separately so xcode-select never supplies production normalization for us.
selected=\${DEVELOPER_DIR:-$FAKE_ROOT/selector}
if [ -d "$selected/Contents/Developer" ]; then selected="$selected/Contents/Developer"; fi
test -d "$selected"
if [ "$1" = --sdk ]; then shift 2; fi
case "$1" in
  --show-sdk-path) printf '%s\\n' "$FAKE_ROOT/sdk"; exit ;;
  --show-sdk-version) echo 26.0; exit ;;
  --show-sdk-build-version) echo 26A; exit ;;
  --find) printf '%s\\n' "$FAKE_ROOT/bin/$2"; exit ;;
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
    FAKE_ROOT: root, FAIL_TOOL: '', DEVELOPER_DIR: developer, SDKROOT: undefined, TOOLCHAINS: undefined,
    APPARENT_HELPER_BUNDLE_ID: undefined, APPARENT_PREVIEW_BUILD: undefined };
  const run = (arch = 'x86_64', extra = {}) => spawnSync(process.execPath,
    ['--experimental-strip-types', join(root, 'enablement/native-build/build.ts'), root, out, arch],
    { cwd: root, env: { ...env, ...extra }, encoding: 'utf8' });
  const count = () => existsSync(join(root, 'invocations')) ? readFileSync(join(root, 'invocations'), 'utf8').trim().split('\n') : [];
  const select = (path: string) => {
    rmSync(join(root, 'selector'));
    symlinkSync(path, join(root, 'selector'));
  };
  return { root, out, developer, otherDeveloper, env, put, run, count, select };
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
  f.put('bin/swiftc', 'new compiler bytes, same reported version');
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

test('only a validated explicit developer selection omits the recursive global watch', t => {
  const f = fixture(t);
  const globalWatch = `cargo:rerun-if-changed=${join(f.root, 'selector')}`;
  symlinkSync(f.developer, join(f.root, 'developer-alias'));
  for (const explicit of [f.developer, join(f.root, 'Xcode.app')]) {
    const r = f.run('x86_64', { DEVELOPER_DIR: explicit });
    assert.equal(r.status, 0, r.stderr);
    assert.ok(!r.stdout.includes(globalWatch), 'explicit selection must not watch the global Xcode tree');
    for (const name of ['Info.plist', 'version.plist']) {
      assert.ok(r.stdout.includes(`cargo:rerun-if-changed=${resolve(f.developer, '..', name)}`));
    }
  }
  for (const explicit of [undefined, '', 'Xcode.app/Contents/Developer']) {
    const r = f.run('x86_64', { DEVELOPER_DIR: explicit });
    assert.equal(r.status, 0, r.stderr);
    assert.ok(r.stdout.includes(globalWatch), 'unset/empty/relative selection keeps the conservative watch');
  }
  const alias = join(f.root, 'developer-alias');
  const aliased = f.run('x86_64', { DEVELOPER_DIR: alias });
  assert.equal(aliased.status, 0, aliased.stderr);
  assert.ok(aliased.stdout.includes(`cargo:rerun-if-changed=${alias}`), 'mutable aliases retain a conservative watch on their unresolved path');
  const watches = aliased.stdout.split('\n').filter(line => line.startsWith('cargo:rerun-if-changed='));
  assert.equal(watches.length, new Set(watches).size, 'shared alias components emit only one Cargo watch per invocation');
  // A successful but mismatched discovery must not authorize dropping the watch.
  f.put('bin/xcode-select', `#!/bin/sh\nprintf '%s\\n' "$FAKE_ROOT/Other.app/Contents/Developer"\n`);
  const r = f.run();
  assert.equal(r.status, 0, r.stderr);
  assert.ok(r.stdout.includes(globalWatch));
});

test('both Mac workflows export the canonical selection before native builds', t => {
  const f = fixture(t);
  for (const [file, job, build, condition] of [
    ['verification-platform.yml', 'rust', 'cargo ', "runner.os == 'macOS' && needs.select.outputs.root == 'true'"],
    ['verification-mac.yml', 'onboarding-candidate', 'bash capture/desktop/scripts/packaging/build-mac-preview.sh', null],
  ] as const) {
    const workflow = readFileSync(join(repository, '.github/workflows', file), 'utf8');
    const body = workflow.split(`\n  ${job}:`)[1];
    assert.ok(body, `${file}: native job exists`);
    const steps = body.split('      - ');
    const selection = steps.findIndex(step => step.includes('uses: maxim-lobanov/setup-xcode@v1'));
    const pin = steps[selection + 1];
    assert.match(pin, /name: Pin selected Xcode for native builds/);
    const actualCondition = pin.match(/^        if: (.+)$/m)?.[1] ?? null;
    assert.equal(actualCondition, condition);
    assert.ok(body.indexOf(pin) < body.indexOf(build), `${file}: selection reaches native build consumers`);
    const shell = pin.split('        run: |\n')[1].split('\n').map(line => line.replace(/^          /, '')).join('\n');
    assert.ok(shell.includes('/usr/bin/xcode-select'), 'use the OS selector, independent of PATH');
    const environment = join(f.root, `${file}.env`);
    const r = spawnSync('bash', ['-eu', '-c', shell.replaceAll('/usr/bin/xcode-select', join(f.root, 'bin/xcode-select'))], {
      env: { ...f.env, DEVELOPER_DIR: f.otherDeveloper, GITHUB_ENV: environment }, encoding: 'utf8',
    });
    assert.equal(r.status, 0, r.stderr);
    assert.equal(readFileSync(environment, 'utf8'), `DEVELOPER_DIR=${f.developer}\n`, 'clear a stale inherited override and publish the canonical global selection for subsequent steps');
  }
});

test('invalid developer discovery fails without publishing or replacing a success receipt', t => {
  const f = fixture(t);
  const missing = { DEVELOPER_DIR: join(f.root, 'missing') };
  assert.notEqual(f.run('x86_64', missing).status, 0);
  assert.ok(!existsSync(join(f.out, 'native-build.json')));
  assert.equal(f.count().length, 0);
  assert.equal(f.run().status, 0);
  const receipt = readFileSync(join(f.out, 'native-build.json'), 'utf8');
  assert.notEqual(f.run('x86_64', missing).status, 0);
  assert.equal(readFileSync(join(f.out, 'native-build.json'), 'utf8'), receipt);
  assert.equal(f.count().length, 5);
  assert.equal(f.run().status, 0);
  assert.equal(f.count().length, 5);
});

function cargoFixture(t: { after: (fn: () => void) => void }) {
  const f = fixture(t);
  const source = readFileSync(join(repository, 'build.rs'), 'utf8');
  const start = source.indexOf('fn compile_mac_native() {');
  assert.ok(start > 0);
  f.put('build.rs', 'use std::path::PathBuf;\nuse std::process::Command;\nfn main() { println!("cargo:rerun-if-changed=metadata"); compile_mac_native(); }\n' + source.slice(start));
  f.put('metadata', 'first revision');
  f.put('src/main.rs', 'fn main() {}');
  f.put('Cargo.toml', '[package]\nname="native-cache-fixture"\nversion="0.1.0"\nedition="2024"\n');
  const cargo = (extra = {}) => spawnSync('cargo', ['build', '--offline', '-vv'], {
    cwd: f.root, env: { ...f.env, CARGO_TARGET_DIR: join(f.root, 'target'), RUSTC_WRAPPER: '',
      CARGO_ENCODED_RUSTFLAGS: '', RUSTFLAGS: '', TMPDIR: f.root, ...extra }, encoding: 'utf8', timeout: 60000 });
  return { ...f, cargo };
}

test('the production Cargo adapter remains fresh on no-op and reuses native outputs after a metadata-only rerun',
  { skip: process.env.APPARENT_NATIVE_CARGO_FIXTURE !== '1' }, t => {
    const f = cargoFixture(t), { cargo } = f;
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

test('Cargo with explicit selection ignores unrelated Xcode descendants but schedules selected native identity changes',
  { skip: process.env.APPARENT_NATIVE_CARGO_FIXTURE !== '1' }, t => {
    const f = cargoFixture(t);
    function build(fresh: boolean, extra = {}) {
      const r = f.cargo(extra);
      assert.equal(r.status, 0, r.stderr);
      if (fresh) assert.match(r.stderr, /Fresh native-cache-fixture/);
      else assert.match(r.stdout, /native-build helper: compiling/);
    }
    build(false); build(true);
    f.put('Xcode.app/Contents/Developer/unrelated/data', 'irrelevant descendant edit');
    build(true);
    f.select(f.otherDeveloper);
    build(true);
    assert.equal(f.count().length, 5);
    let builds = 1;
    for (const path of ['Xcode.app/Contents/Info.plist', 'Xcode.app/Contents/version.plist',
      'bin/swiftc', 'bin/metal', 'bin/metallib', 'bin/swift-frontend', 'bin/clang', 'bin/ld', 'sdk/SDKSettings.json']) {
      f.put(path, 'changed identity bytes');
      build(false); build(true);
      assert.equal(f.count().length, ++builds * 5, path);
    }
    for (const extra of [{ TOOLCHAINS: 'alternate' }, { SDKROOT: join(f.root, 'sdk') }, { DEVELOPER_DIR: f.otherDeveloper }]) {
      build(false, extra); build(true, extra);
      assert.equal(f.count().length, ++builds * 5);
    }
  });

test('Cargo without an explicit override tracks global selection and preserves reuse and failure semantics',
  { skip: process.env.APPARENT_NATIVE_CARGO_FIXTURE !== '1' }, t => {
    const f = cargoFixture(t);
    const cargo = (extra = {}) => f.cargo({ DEVELOPER_DIR: undefined, ...extra });
    let r = cargo();
    assert.equal(r.status, 0, r.stderr);
    r = cargo();
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.stderr, /Fresh native-cache-fixture/);
    f.put('Xcode.app/Contents/Developer/unrelated/data', 'global descendant edit');
    r = cargo();
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.stdout, /native-build helper: verified reuse/);
    assert.equal(f.count().length, 5);
    f.select(f.otherDeveloper);
    r = cargo();
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.stdout, /native-build helper: compiling/);
    assert.equal(f.count().length, 10);
    r = cargo();
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.stderr, /Fresh native-cache-fixture/);
    const native = readdirSync(join(f.root, 'target/debug/build')).find(name => existsSync(join(f.root, 'target/debug/build', name, 'out/native-build.json')))!;
    const manifest = join(f.root, 'target/debug/build', native, 'out/native-build.json');
    const receipt = readFileSync(manifest, 'utf8');
    r = cargo({ DEVELOPER_DIR: join(f.root, 'missing') });
    assert.notEqual(r.status, 0);
    assert.equal(readFileSync(manifest, 'utf8'), receipt);
    assert.equal(f.count().length, 10);
    r = cargo();
    assert.equal(r.status, 0, r.stderr);
    assert.equal(f.count().length, 10);
  });

for (const kind of ['developer', 'app', 'parent', 'global'] as const) {
  test(`Cargo notices retargeting an explicit ${kind} alias without an environment change`,
    { skip: process.env.APPARENT_NATIVE_CARGO_FIXTURE !== '1' }, t => {
      const f = cargoFixture(t);
      const alias = join(f.root, kind === 'global' ? 'selector' : 'developer-alias');
      const target = (developer: string) => kind === 'app' ? resolve(developer, '../..')
        : kind === 'parent' ? resolve(developer, '..') : developer;
      if (kind !== 'global') symlinkSync(target(f.developer), alias);
      const env = { DEVELOPER_DIR: kind === 'parent' ? join(alias, 'Developer') : alias };
      let r = f.cargo(env);
      assert.equal(r.status, 0, r.stderr);
      assert.equal(f.count().length, 5);
      r = f.cargo(env);
      assert.equal(r.status, 0, r.stderr);
      assert.match(r.stderr, /Fresh native-cache-fixture/);
      rmSync(alias);
      symlinkSync(target(f.otherDeveloper), alias);
      r = f.cargo(env);
      assert.equal(r.status, 0, r.stderr);
      assert.match(r.stderr, /Dirty native-cache-fixture/);
      assert.equal(f.count().length, 10, 'retargeting schedules native identity validation and recompilation');
      r = f.cargo(env);
      assert.equal(r.status, 0, r.stderr);
      assert.match(r.stderr, /Fresh native-cache-fixture/);
      assert.equal(f.count().length, 10);
    });
}

function checkRetarget(t: Parameters<typeof cargoFixture>[0],
  prepare: (f: ReturnType<typeof cargoFixture>) => { extra?: Record<string, string>; retarget: () => void },
  f = cargoFixture(t)) {
  const { extra = {}, retarget } = prepare(f);
  let r = f.cargo(extra);
  assert.equal(r.status, 0, r.stderr);
  assert.equal(f.count().length, 5);
  r = f.cargo(extra);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stderr, /Fresh native-cache-fixture/);
  assert.doesNotMatch(r.stderr, /Running `/);
  retarget();
  r = f.cargo(extra);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stderr, /Dirty native-cache-fixture/);
  assert.equal(f.count().length, 10, 'Cargo must schedule validation of the newly selected native inputs');
  r = f.cargo(extra);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stderr, /Fresh native-cache-fixture/);
  assert.doesNotMatch(r.stderr, /Running `/);
  assert.equal(f.count().length, 10);
}

for (const kind of ['intermediate', 'target-parent', 'target-parent-traversal'] as const) {
  test(`Cargo tracks developer alias ${kind} resolution`,
    { skip: process.env.APPARENT_NATIVE_CARGO_FIXTURE !== '1' }, t => {
      checkRetarget(t, f => {
        const outer = join(f.root, 'outer'), inner = join(f.root, 'inner');
        const target = (developer: string) => kind === 'target-parent' ? resolve(developer, '..') : developer;
        symlinkSync(target(f.developer), inner);
        symlinkSync(kind === 'intermediate' ? 'inner'
          : kind === 'target-parent' ? 'inner/Developer' : 'inner/../Developer', outer);
        return { extra: { DEVELOPER_DIR: outer }, retarget: () => {
          rmSync(inner); symlinkSync(target(f.otherDeveloper), inner);
        } };
      });
    });
}

function redirectDiscovery(f: ReturnType<typeof cargoFixture>, oldText: string, newText: string) {
  const path = join(f.root, 'bin/xcrun'), body = readFileSync(path, 'utf8');
  assert.ok(body.includes(oldText));
  writeFileSync(path, body.replace(oldText, newText));
}

function cryptexFixture(t: Parameters<typeof cargoFixture>[0]) {
  const f = cargoFixture(t);
  const mount = 'run/com.apple.security.cryptexd/mnt/com.apple.MobileAsset.MetalToolchain-v27.2.47.0.fixture';
  const bin = `private/var/${mount}/Metal.xctoolchain/usr/bin`;
  f.put(`${bin}/metal`, 'synthetic Metal compiler');
  f.put(`${bin}/air-lld`, 'synthetic Metal linker');
  symlinkSync('air-lld', join(f.root, bin, 'metallib'));
  f.put('private/var/log/unrelated', 'ordinary system activity');
  symlinkSync('private/var', join(f.root, 'var'));
  redirectDiscovery(f, '--find) printf', `--find) case "$2" in metal|metallib)
    printf '%s\\n' "$FAKE_ROOT/var/${mount}/Metal.xctoolchain/usr/bin/$2"; exit;; esac; printf`);
  return { ...f, metalBin: join(f.root, bin) };
}

test('Cargo cryptex Metal selection stays Fresh through unrelated system churn',
  { skip: process.env.APPARENT_NATIVE_CARGO_FIXTURE !== '1' }, t => {
    const f = cryptexFixture(t);
    const cold = f.cargo();
    assert.equal(cold.status, 0, cold.stderr);
    const watches = cold.stdout.split('\n').flatMap(line => {
      const match = line.match(/cargo:rerun-if-changed=(.*)$/);
      return match ? [match[1]] : [];
    });
    t.diagnostic('System watches: ' + JSON.stringify(watches.filter(path => path.startsWith(join(f.root, 'var'))
      || path.startsWith(join(f.root, 'private')))));
    const warm = f.cargo();
    assert.equal(warm.status, 0, warm.stderr);
    assert.match(warm.stderr, /Fresh native-cache-fixture/);
    assert.doesNotMatch(warm.stderr, /Running `/);
    const samples = [];
    for (let i = 0; i < 3; i++) {
      f.put('private/var/log/unrelated', `unrelated system churn ${i}`);
      const r = f.cargo();
      assert.equal(r.status, 0, r.stderr);
      assert.equal(f.count().length, 5, 'churn must not compile native outputs');
      samples.push(r);
      t.diagnostic(`churn ${i}: ` + r.stderr.split('\n').filter(line => /Dirty |Fresh |Running `/.test(line)).join('\n')
        + '\n' + r.stdout.split('\n').filter(line => /native-build /.test(line)).join('\n'));
    }
    for (const r of samples) {
      assert.match(r.stderr, /Fresh native-cache-fixture/);
      assert.doesNotMatch(r.stderr, /Running `/);
      assert.doesNotMatch(r.stdout, /native-build /);
    }
    assert.deepEqual(watches.filter(path => path.startsWith(join(f.root, 'var'))
      || path.startsWith(join(f.root, 'private'))).sort(),
      [f.metalBin, ...['metal', 'metallib', 'air-lld'].map(name => join(f.metalBin, name))].sort());
    assert.ok(!watches.includes(join(f.root, 'selector')));
  });

for (const kind of ['metal-bytes', 'linker-bytes', 'file-alias', 'target-parent', 'transitive'] as const) {
  test(`Cargo cryptex selection still validates ${kind} changes`,
    { skip: process.env.APPARENT_NATIVE_CARGO_FIXTURE !== '1' }, t => {
      const f = cryptexFixture(t);
      checkRetarget(t, () => {
        if (kind === 'metal-bytes' || kind === 'linker-bytes') return { retarget: () => {
          writeFileSync(join(f.metalBin, kind === 'metal-bytes' ? 'metal' : 'air-lld'), 'changed executable bytes');
        } };
        if (kind === 'file-alias') {
          const older = join(f.metalBin, 'older-air-lld');
          writeFileSync(older, 'different linker with old mtime');
          utimesSync(older, 1, 1);
          return { retarget: () => {
            rmSync(join(f.metalBin, 'metallib'));
            symlinkSync('older-air-lld', join(f.metalBin, 'metallib'));
          } };
        }
        // The tool file alias introduces a target-parent link below /var;
        // a second directory link exercises transitive expansion there too.
        for (const name of ['old', 'new']) {
          const path = join(f.metalBin, name);
          mkdirSync(path); writeFileSync(join(path, 'metal'), name + ' compiler');
        }
        const alias = join(f.metalBin, 'selected');
        symlinkSync('old', alias);
        if (kind === 'transitive') symlinkSync('selected', join(f.metalBin, 'middle'));
        rmSync(join(f.metalBin, 'metal'));
        symlinkSync(`${kind === 'transitive' ? 'middle' : 'selected'}/metal`, join(f.metalBin, 'metal'));
        return { retarget: () => { rmSync(alias); symlinkSync('new', alias); } };
      }, f);
    });
}

test('the /var exception requires the exact root alias, target spelling and canonical destination', t => {
  const f = cryptexFixture(t);
  const alias = join(f.root, 'var');
  const broadWatch = `cargo:rerun-if-changed=${alias}\n`;
  // Alternate spelling or a mutable intermediate target stays conservative,
  // even if it currently reaches the same files as the standard root alias.
  symlinkSync('private/var', join(f.root, 'intermediate'));
  for (const target of [join(f.root, 'private/var'), 'intermediate']) {
    rmSync(alias); symlinkSync(target, alias);
    const r = f.run();
    assert.equal(r.status, 0, r.stderr);
    assert.ok(r.stdout.includes(broadWatch), target);
  }
  rmSync(alias); symlinkSync('private/var', alias);
  renameSync(join(f.root, 'private/var'), join(f.root, 'private/other'));
  symlinkSync('other', join(f.root, 'private/var'));
  const redirected = f.run();
  assert.equal(redirected.status, 0, redirected.stderr);
  assert.ok(redirected.stdout.includes(broadWatch), 'noncanonical destination is not OS layout');
  assert.ok(redirected.stdout.includes(`cargo:rerun-if-changed=${join(f.root, 'private/var')}\n`));

  // A user-owned lookalike /something/var -> private/var is not exempt.
  f.put('user/private/var/metal', 'compiler');
  symlinkSync('private/var', join(f.root, 'user/var'));
  redirectDiscovery(f, '"$FAKE_ROOT/bin/$2"', '"$FAKE_ROOT/user/var/metal"');
  const lookalike = f.run();
  assert.equal(lookalike.status, 0, lookalike.stderr);
  assert.ok(lookalike.stdout.includes(`cargo:rerun-if-changed=${join(f.root, 'user/var')}\n`));
});

test('Cargo refuses dangling and cyclic cryptex aliases without replacing a receipt, then returns Fresh',
  { skip: process.env.APPARENT_NATIVE_CARGO_FIXTURE !== '1' }, t => {
    const f = cryptexFixture(t);
    const cold = f.cargo();
    assert.equal(cold.status, 0, cold.stderr);
    const native = readdirSync(join(f.root, 'target/debug/build')).find(name => existsSync(join(f.root, 'target/debug/build', name, 'out/native-build.json')))!;
    const manifest = join(f.root, 'target/debug/build', native, 'out/native-build.json');
    const receipt = readFileSync(manifest, 'utf8');
    for (const target of ['missing-linker', 'metallib']) {
      rmSync(join(f.metalBin, 'metallib')); symlinkSync(target, join(f.metalBin, 'metallib'));
      const failed = f.cargo();
      assert.notEqual(failed.status, 0, target);
      assert.match(failed.stderr, /Mac helper\/shader build failed/);
      assert.equal(readFileSync(manifest, 'utf8'), receipt);
      assert.equal(f.count().length, 5);
      rmSync(join(f.metalBin, 'metallib')); symlinkSync('air-lld', join(f.metalBin, 'metallib'));
      const recovered = f.cargo();
      assert.equal(recovered.status, 0, recovered.stderr);
      assert.match(recovered.stdout, /native-build helper: verified reuse/);
      assert.equal(f.count().length, 5);
      const warm = f.cargo();
      assert.equal(warm.status, 0, warm.stderr);
      assert.match(warm.stderr, /Fresh native-cache-fixture/);
      assert.doesNotMatch(warm.stderr, /Running `/);
    }
  });

test('unresolvable native aliases fail before accepting or replacing a receipt', t => {
  const f = fixture(t);
  assert.equal(f.run().status, 0);
  const receipt = readFileSync(join(f.out, 'native-build.json'), 'utf8');
  const alias = join(f.root, 'sdk-alias');
  redirectDiscovery(f, '"$FAKE_ROOT/sdk"', '"$FAKE_ROOT/sdk-alias"');
  for (const target of ['missing-sdk', 'sdk-alias']) {
    symlinkSync(target, alias);
    const failed = f.run();
    assert.notEqual(failed.status, 0, target);
    assert.equal(readFileSync(join(f.out, 'native-build.json'), 'utf8'), receipt);
    assert.equal(f.count().length, 5);
    rmSync(alias);
  }
});

for (const kind of ['tool', 'tool-parent', 'sdk'] as const) {
  test(`Cargo with canonical developer selection tracks ${kind} aliases`,
    { skip: process.env.APPARENT_NATIVE_CARGO_FIXTURE !== '1' }, t => {
      checkRetarget(t, f => {
        const relative = 'Xcode.app/Contents/Developer';
        let alias: string, replacement: string;
        if (kind === 'sdk') {
          f.put(`${relative}/SDKs/old.sdk/SDKSettings.json`, '{"Version":"26.0"}');
          f.put(`${relative}/SDKs/new.sdk/SDKSettings.json`, '{"Version":"26.1"}');
          alias = join(f.developer, 'SDKs/MacOSX.sdk');
          symlinkSync('old.sdk', alias); replacement = 'new.sdk';
          redirectDiscovery(f, '"$FAKE_ROOT/sdk"', '"$FAKE_ROOT/Xcode.app/Contents/Developer/SDKs/MacOSX.sdk"');
        } else {
          f.put(`${relative}/Toolchains/old.xctoolchain/usr/bin/swiftc`, 'old compiler bytes');
          f.put(`${relative}/Toolchains/new.xctoolchain/usr/bin/swiftc`, 'new compiler bytes');
          alias = join(f.developer, kind === 'tool' ? 'swiftc' : 'Toolchains/Default.xctoolchain');
          symlinkSync(kind === 'tool' ? 'Toolchains/old.xctoolchain/usr/bin/swiftc' : 'old.xctoolchain', alias);
          replacement = kind === 'tool' ? 'Toolchains/new.xctoolchain/usr/bin/swiftc' : 'new.xctoolchain';
          const tool = kind === 'tool' ? 'swiftc' : 'Toolchains/Default.xctoolchain/usr/bin/swiftc';
          redirectDiscovery(f, '--find) printf', `--find) if [ "$2" = swiftc ]; then printf '%s\\n' "$FAKE_ROOT/Xcode.app/Contents/Developer/${tool}"; exit; fi; printf`);
        }
        return { retarget: () => { rmSync(alias); symlinkSync(replacement, alias); } };
      });
    });
}
