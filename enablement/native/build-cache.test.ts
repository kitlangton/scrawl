import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, chmodSync, rmSync, unlinkSync, existsSync, readdirSync, symlinkSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, dirname, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'

// These fixtures execute the production build script but do not claim Darwin proof.
test('native reuse preserves current Git identity and invalidates actual native inputs', () => {
  const root = mkdtempSync(join(tmpdir(), 'native-cache-'))
  const bin = join(root, 'bin')
  const out = join(root, 'target/debug/build/apparent-fixture/out')
  const log = join(root, 'compiler.log')
  const script = join(root, 'build-script')
  const repo = resolve('.')
  const env = { ...process.env, PATH: `${bin}:${process.env.PATH}`, CARGO_MANIFEST_DIR: root,
    CARGO_PKG_VERSION: '0.0.0', CARGO_CFG_TARGET_OS: 'macos', CARGO_CFG_TARGET_ARCH: 'x86_64',
    TARGET: 'x86_64-apple-darwin', HOST: 'x86_64-apple-darwin', PROFILE: 'debug', OUT_DIR: out,
    FIXTURE_BIN: bin, FIXTURE_LOG: log }
  for (const key of Object.keys(env)) if (key.startsWith('GIT_')) delete env[key]
  let lastStderr = ''
  function run(command: string, args: string[], overrides: Record<string, string> = {}) {
    const result = spawnSync(command, args, { cwd: root, env: { ...env, ...overrides }, encoding: 'utf8', timeout: 60_000 })
    assert.equal(result.status, 0, `${command}: ${result.error ?? ''}\n${result.stdout}\n${result.stderr}`)
    lastStderr = result.stderr
    return result.stdout
  }
  function file(path: string, bytes: string) {
    mkdirSync(dirname(join(root, path)), { recursive: true }); writeFileSync(join(root, path), bytes)
  }
  function count() { return readFileSync(log, 'utf8').trim().split('\n').length }
  try {
    mkdirSync(bin); mkdirSync(out, { recursive: true }); writeFileSync(log, '')
    const source = readFileSync(join(repo, 'build.rs'), 'utf8')
    for (const match of source.matchAll(/"((?:capture|shared)\/[^"\n]+\.(?:swift|metal))"/g)) file(match[1], `// fixture ${match[1]}\n`)
    file('shared/native/Info.plist', '<string>dev.kitlangton.apparent.helper</string>')
    file('unrelated.txt', 'initial')
    file('.gitignore', 'bin/\ntarget/\nbuild-script\ncompiler.log\n')
    const tool = `#!/bin/sh
set -eu
case "$*" in
  *--find*|*-f\\ *) printf '%s/tool\\n' "$FIXTURE_BIN"; exit 0 ;;
  *--show-sdk-path*) printf '%s/sdk\\n' "$FIXTURE_BIN"; exit 0 ;;
  *--show-sdk-version*) echo 26.0; exit 0 ;;
  *--show-sdk-build-version*) echo 25A123; exit 0 ;;
  *--version) echo 'fixture compiler 1'; exit 0 ;;
esac
test ! -e "$FIXTURE_BIN/fail-compiler"
printf '%s\\n' "$*" >> "$FIXTURE_LOG"
output=''
while [ "$#" -gt 0 ]; do
  if [ "$1" = '-o' ]; then shift; output="$1"; fi
  shift
done
test -n "$output"
printf 'fixture native output\\n' > "$output"
chmod 755 "$output"
`
    file('bin/xcrun', tool); file('bin/tool', '# fixture compiler bytes\n')
    file('bin/xcode-select', '#!/bin/sh\nprintf "%s/developer\\n" "$FIXTURE_BIN"\n')
    file('bin/xcodebuild', '#!/bin/sh\ntest ! -e "$FIXTURE_BIN/fail-identity" || exit 1\necho "Xcode 26.0"\necho "Build version 17A123"\n')
    file('bin/sdk/SDKSettings.json', '{"Version":"26.0"}')
    file('bin/sdk/SDKSettings.plist', '<plist>fixture SDK</plist>')
    file('bin/Version.plist', '<plist>fixture Xcode</plist>')
    for (const name of ['xcrun', 'xcode-select', 'xcodebuild', 'tool']) chmodSync(join(bin, name), 0o755)
    run('git', ['init', '-q']); run('git', ['config', 'user.email', 'fixture@example.invalid'])
    run('git', ['config', 'user.name', 'Fixture']); run('git', ['add', '.'])
    run('git', ['-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'fixture'])
    run('rustc', ['--edition=2024', join(repo, 'build.rs'), '-o', script])
    const initial = run(script, [])
    assert.match(initial, /cargo:rustc-env=APPARENT_VERSION_LABEL=0\.0\.0/)
    const cold = count(); assert.equal(cold, 5, 'two Metal stages per shader plus Swift')
    run(script, []); assert.equal(count(), cold, 'unchanged native outputs reused')
    file('unrelated.txt', 'modified')
    assert.match(run(script, []), /APPARENT_VERSION_LABEL=.*\+modified/)
    assert.equal(count(), cold, 'unrelated dirty source does not rebuild native outputs')
    run('git', ['add', 'unrelated.txt']); run('git', ['-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'next'])
    const sha = run('git', ['rev-parse', '--short=7', 'HEAD']).trim()
    assert.ok(run(script, []).includes(`0.0.0 · ${sha}`)); assert.equal(count(), cold)
    file('capture/desktop/native/Settings.swift', '// different Settings consumer\n')
    run(script, []); assert.equal(count(), cold + 1)
    file('shared/native/Info.plist', '<string>dev.kitlangton.apparent.helper</string>\n<!-- changed -->')
    run(script, []); assert.equal(count(), cold + 2)
    file('capture/desktop/native/pill/pill.metal', '// changed shader\n')
    run(script, []); assert.equal(count(), cold + 4)
    unlinkSync(join(root, 'target/debug/apparent-helper'))
    run(script, []); assert.equal(count(), cold + 4)
    assert.equal(readFileSync(join(root, 'target/debug/apparent-helper'), 'utf8'), 'fixture native output\n')
    writeFileSync(join(out, 'apparent-helper'), 'corrupt')
    run(script, []); assert.equal(count(), cold + 5)
    unlinkSync(join(out, 'pill.metallib'))
    run(script, []); assert.equal(count(), cold + 7)
    let before = count()
    file('bin/sdk/SDKSettings.json', '{"Version":"26.0","Changed":true}')
    run(script, []); assert.equal(count(), before + 5, 'SDK contents invalidate every native output')
    before = count()
    file('bin/sdk/usr/include/fixture.h', '// changed SDK header with unchanged SDK metadata\n')
    run(script, []); assert.equal(count(), before + 5, 'SDK header content is part of identity')
    before = count()
    file('bin/tool', '# changed compiler bytes\n')
    run(script, []); assert.equal(count(), before + 5, 'compiler bytes invalidate even unchanged version text')
    before = count()
    run(script, [], { CARGO_CFG_TARGET_ARCH: 'aarch64' }); assert.equal(count(), before + 5)
    run(script, [], { CARGO_CFG_TARGET_ARCH: 'aarch64' }); assert.equal(count(), before + 5)
    run(script, []); assert.equal(count(), before + 10, 'changing architecture back rebuilds')
    before = count()
    file('bin/fail-identity', '')
    run(script, []); run(script, [])
    assert.equal(count(), before + 10, 'failed identity must compile, never hit a partial key')
    unlinkSync(join(bin, 'fail-identity'))
    run(script, []); assert.equal(count(), before + 15, 'fallback invalidates previous receipt')
    before = count()
    file('capture/desktop/native/Settings.swift', '// failing changed source\n')
    file('bin/fail-compiler', '')
    const failed = spawnSync(script, [], { cwd: root, env, encoding: 'utf8', timeout: 60_000 })
    assert.notEqual(failed.status, 0, 'compiler failure must fail the production build script')
    unlinkSync(join(bin, 'fail-compiler'))
    run(script, []); assert.equal(count(), before + 1, 'failed compile cannot publish a success receipt')
    const nativeCount = count()
    file('runtime/libfixture.so', 'fixture runtime')
    file('modules/libfixture-cpu.so', 'fixture CPU module')
    run(script, [], { CARGO_CFG_TARGET_OS: 'linux', DEP_TRANSCRIBE_CPP_RUNTIME_DIR: join(root, 'runtime'),
      DEP_TRANSCRIBE_CPP_MODULE_DIR: join(root, 'modules') })
    assert.equal(count(), nativeCount, 'Linux must not invoke Apple toolchain probes or compilers')
    assert.equal(readFileSync(join(root, 'target/debug/deps/libfixture-cpu.so'), 'utf8'), 'fixture CPU module')
    // A tiny temporary Cargo package checks rerun decisions, without compiling
    // Apparent or substituting these synthetic tools for Darwin acceptance.
    for (const match of source.matchAll(/"((?:capture|shared|enablement)\/[^"\n]+)"/g)) {
      if (!existsSync(join(root, match[1]))) file(match[1], 'fixture watched input\n')
    }
    file('Cargo.toml', '[package]\nname="native-cache-fixture"\nversion="0.0.0"\nedition="2024"\nbuild="driver.rs"\n')
    file('src/main.rs', 'fn main() { println!("{}", env!("APPARENT_VERSION_LABEL")); }')
    file('driver.rs', `mod production { include!(${JSON.stringify(join(repo, 'build.rs'))}); pub fn execute() { main(); } }
fn main() { unsafe { std::env::set_var("CARGO_CFG_TARGET_OS", "macos"); } production::execute(); }
`)
    run('git', ['add', '.']); run('git', ['-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'Cargo fixture'])
    const cargoEnv = { RUSTC_WRAPPER: '', CARGO_TARGET_DIR: join(root, 'cargo-target') }
    file('.gitignore', readFileSync(join(root, '.gitignore'), 'utf8') + 'cargo-target/\n')
    run('cargo', ['build', '--offline', '-j', '1'], cargoEnv)
    before = count()
    run('cargo', ['build', '--offline', '-j', '1'], cargoEnv)
    assert.equal(count(), before, 'Cargo no-op reuses native outputs')
    assert.doesNotMatch(lastStderr, /Compiling native-cache-fixture/, 'no-op must remain Cargo Fresh')
    file('unrelated.txt', 'Cargo identity change')
    run('cargo', ['build', '--offline', '-j', '1'], cargoEnv)
    assert.equal(count(), before, 'Cargo rebuild for unrelated input still reuses native outputs')
    assert.match(run(join(root, 'cargo-target/debug/native-cache-fixture'), []), /\+modified/)
    const buildDir = join(root, 'cargo-target/debug/build')
    const cargoOut = readdirSync(buildDir).map(name => join(buildDir, name, 'out'))
      .find(path => existsSync(join(path, 'apparent-helper')))
    assert.ok(cargoOut)
    unlinkSync(join(cargoOut, 'pill.metallib'))
    file('unrelated.txt', 'rerun after native output removal')
    run('cargo', ['build', '--offline', '-j', '1'], cargoEnv)
    assert.equal(count(), before + 2, 'build-script rerun rebuilds missing native output instead of accepting a stale receipt')
    mkdirSync(join(bin, 'developer'))
    const explicit = { DEVELOPER_DIR: join(bin, 'developer') }
    run(script, [], explicit)
    before = count()
    run(script, [], explicit); assert.equal(count(), before)
    file('bin/sdk/usr/include/fixture.h', '// explicit selection SDK edit\n')
    run(script, [], explicit); assert.equal(count(), before + 5, 'explicit selection retains SDK content invalidation')
    before = count()
    file('bin/tool', '# explicit selection compiler edit\n')
    run(script, [], explicit); assert.equal(count(), before + 5, 'explicit selection retains compiler byte invalidation')
    before = count()
    for (const invalid of ['relative', join(bin, 'missing'), bin]) {
      run(script, [], { DEVELOPER_DIR: invalid })
      run(script, [], { DEVELOPER_DIR: invalid })
      assert.equal(count(), before + 10, 'invalid override must bypass reuse on both executions')
      before = count()
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('Cargo watches global selection only without a validated explicit developer directory', () => {
  const root = mkdtempSync(join(tmpdir(), 'developer-watch-'))
  try {
    const developer = join(root, 'Xcode/Contents/Developer')
    mkdirSync(developer, { recursive: true }); mkdirSync(join(root, 'src'))
    const metadata = join(root, 'Xcode/Contents/Info.plist')
    const unrelated = join(developer, 'unrelated')
    writeFileSync(metadata, 'metadata'); writeFileSync(unrelated, 'first')
    symlinkSync(developer, join(root, 'selector'), 'dir')
    writeFileSync(join(root, 'Cargo.toml'), '[package]\nname="developer-watch"\nversion="0.1.0"\nedition="2024"\n')
    writeFileSync(join(root, 'src/lib.rs'), '')
    writeFileSync(join(root, 'build.rs'), `#[allow(dead_code)] mod cache { include!(${JSON.stringify(resolve('enablement/native/build_cache.rs'))}); }
fn main() {
println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
let explicit = std::env::var_os("DEVELOPER_DIR");
for path in cache::developer_watches(std::path::Path::new(${JSON.stringify(developer)}), explicit.as_deref(), std::path::Path::new("selector")).expect("invalid explicit developer") {
if path.exists() { println!("cargo:rerun-if-changed={}", path.display()); }
}
}`)
    const build = (explicit?: string, success = true) => {
      const env = { ...process.env, CARGO_TARGET_DIR: join(root, 'target') }
      delete env.DEVELOPER_DIR
      if (explicit !== undefined) env.DEVELOPER_DIR = explicit
      const result = spawnSync('cargo', ['build', '--offline'], { cwd: root, env, encoding: 'utf8', timeout: 60000 })
      assert.equal(result.status === 0, success, result.stderr)
      return result.stderr
    }
    build(); assert.doesNotMatch(build(), /Compiling developer-watch/)
    writeFileSync(unrelated, 'global watched edit')
    assert.match(build(), /Compiling developer-watch/)
    build(developer); assert.doesNotMatch(build(developer), /Compiling developer-watch/)
    writeFileSync(unrelated, 'irrelevant global directory edit')
    assert.doesNotMatch(build(developer), /Compiling developer-watch/)
    writeFileSync(metadata, 'selected metadata edit')
    assert.match(build(developer), /Compiling developer-watch/)
    build('relative', false); build(root, false)
  } finally { rmSync(root, { recursive: true, force: true }) }
})
