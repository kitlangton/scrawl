import { test } from 'node:test'
import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'

const action = readFileSync('.github/actions/mac-rust-cache/action.yml', 'utf8')
const shell = action.split('      run: |\n')[1].split('\n').map(line => line.slice(8)).join('\n')
const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8')

// Execute the production guard with synthetic toolchain commands. No installed
// toolchain, cache or application data is changed by this fixture.
for (const scenario of ['old', 'current', 'only-selected', 'unknown', 'wrong-active', 'wrong-runner', 'extra-toolchain', 'partial-uninstall']) {
  test(`cache inventory: ${scenario}`, () => {
    const mock = `
selected=1.99.1-x86_64-apple-darwin
legacy=stable-x86_64-apple-darwin
removed=0
rustc() {
  echo 'release: 1.99.1'
  echo 'host: x86_64-apple-darwin'
  echo 'commit-hash: 58cf8f9c35b355199727627d07ae34a4e2846d67'
}
rustup() {
  case "$1 $2" in
    'show active-toolchain')
      if [ "$SCENARIO" = wrong-active ]; then echo "$legacy"; else echo "$selected (default)"; fi ;;
    'toolchain list')
      echo "$selected"
      if [ "$SCENARIO" = extra-toolchain ]; then echo nightly-x86_64-apple-darwin; fi
      if [ "$removed" = 0 ] && [ "$SCENARIO" != only-selected ]; then echo "$legacy"; fi ;;
    'toolchain uninstall')
      echo UNINSTALL >&2
      if [ "$SCENARIO" != partial-uninstall ]; then removed=1; fi ;;
    "run $selected") rustc ;;
    "run $legacy")
      case "$SCENARIO" in
        current) rustc ;;
        unknown) echo 'release: unknown' ;;
        *) echo 'release: 1.98.0'; echo 'host: x86_64-apple-darwin'; echo 'commit-hash: 88d9e12ae178fab0fb5cc050a94da85685d449ea' ;;
      esac ;;
    *) return 91 ;;
  esac
}
`
    assert.ok(shell.includes('rustup toolchain uninstall'))
    const result = spawnSync('bash', ['-c', mock + '\n' + shell], {
      encoding: 'utf8', env: { ...process.env, SCENARIO: scenario,
        GITHUB_ACTIONS: 'true', RUNNER_ENVIRONMENT: 'github-hosted',
        RUNNER_OS: scenario === 'wrong-runner' ? 'Linux' : 'macOS', RUNNER_ARCH: 'X64' },
    })
    assert.equal(result.status === 0, ['old', 'current', 'only-selected'].includes(scenario), result.stderr)
    assert.equal(result.stderr.includes('UNINSTALL'), ['old', 'partial-uninstall'].includes(scenario))
  })
}

test('cache compatibility retains full paths, invalidation defaults and branch seeds', () => {
  const rust = workflow.slice(workflow.indexOf('\n  rust:'))
  assert.match(rust, /toolchain: \$\{\{ runner\.os == 'macOS' && '1\.99\.1' \|\| 'stable' \}\}/)
  assert.ok(rust.indexOf('uses: ./.github/actions/mac-rust-cache') < rust.indexOf('uses: Swatinem/rust-cache@v2'))
  const cache = rust.split('uses: Swatinem/rust-cache@v2')[1].split('      - uses:')[0]
  assert.doesNotMatch(cache, /cache-on-failure:/)
  assert.match(cache, /workspaces: \|\n            \. -> target\n            capture\/engine -> target\n            connect\/client -> target/)
  assert.doesNotMatch(cache, /save-if:|shared-key:|env-vars:|cache-workspace-crates:|add-rust-environment-hash-key:/)
  const arm = rust.indexOf("run: echo 'CACHE_ON_FAILURE=true'")
  assert.ok(arm > rust.indexOf('python3 enablement/verification.py --base "$BASE" --run --online'))
  assert.ok(arm < rust.indexOf('name: Actual Electron to Rust Settings integration'))
  assert.match(rust.slice(rust.lastIndexOf('      - name:', arm), arm), /if: runner.os == 'macOS'/)
  assert.doesNotMatch(rust.slice(rust.lastIndexOf('      - name:', arm), arm), /always\(\)|failure\(\)/)
})
