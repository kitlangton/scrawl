import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, copyFileSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import test from 'node:test'

// Execute the real shell harness with synthetic packaging tools, never a model
// download or a real emulator. Failures must stop before emulated inference.
for (const scenario of ['success', 'bad-model', 'missing-filter', 'bad-inference'] as const) {
  test(`emulated packaging smoke: ${scenario}`, () => {
    const root = mkdtempSync(join(tmpdir(), 'apparent-cpu-smoke-'))
    try {
      const scripts = join(root, 'capture/desktop/scripts/linux/appimage')
      mkdirSync(scripts, { recursive: true })
      mkdirSync(join(root, 'target/release'), { recursive: true })
      copyFileSync(new URL('./smoke-cpu.sh', import.meta.url), join(scripts, 'smoke-cpu.sh'))
      const executable = (name: string, body: string) => {
        const path = join(root, name)
        writeFileSync(path, `#!/bin/bash\nset -eu\n${body}\n`, { mode: 0o755 })
        return path
      }
      const image = executable('image', `mkdir -p squashfs-root/usr/lib squashfs-root/usr/bin
cat > squashfs-root/usr/bin/apparent <<'APP'
#!/bin/bash
set -eu
echo demo >> "$TRACE"
mkdir -p "$3"
echo '{}' > "$3/apparent.json"
APP
chmod +x squashfs-root/usr/bin/apparent`)
      const speech = executable('speech', `if [[ "$*" == *--list* ]]; then
  if [[ "$SCENARIO" == missing-filter && "$1" == *runs_prepared_parakeet ]]; then exit 0; fi
  echo "$1: test"
  exit 0
fi
if [[ "$1" == *downloads_and_runs_parakeet ]]; then
  test -z "\${APPARENT_TEST_NO_AVX_VNNI:-}"
  test -z "\${APPARENT_TEST_NO_AVX512:-}"
  echo native-verify >> "$TRACE"
  [[ "$SCENARIO" != bad-model ]] || exit 9
  mkdir -p "$APPARENT_TEST_MODEL_DIR"
  echo verified > "$APPARENT_TEST_MODEL_DIR/model"
else
  test "$APPARENT_TEST_NO_AVX_VNNI" = 1
  test "$APPARENT_TEST_NO_AVX512" = 1
  test -s "$APPARENT_TEST_MODEL_DIR/model"
  echo prepared-inference >> "$TRACE"
  [[ "$SCENARIO" != bad-inference ]] || exit 10
fi`)
      const emulator = executable('emulator', `echo emulator >> "$TRACE"
test "$1" = --
shift
exec "$@"`)
      const trace = join(root, 'trace')
      writeFileSync(trace, '')
      const result = spawnSync('bash', [join(scripts, 'smoke-cpu.sh'), image, speech, emulator], {
        encoding: 'utf8', env: { ...process.env, SCENARIO: scenario, TRACE: trace, APPARENT_TEST_MODEL_DIR: join(root, 'model'),
          APPARENT_TEST_NO_AVX_VNNI: '1', APPARENT_TEST_NO_AVX512: '1' },
      })
      const events = readFileSync(trace, 'utf8').trim().split('\n').filter(Boolean)
      if (scenario === 'success') {
        assert.equal(result.status, 0, result.stderr)
        assert.deepEqual(events, ['native-verify', 'emulator', 'prepared-inference', 'emulator', 'demo'])
      } else {
        assert.notEqual(result.status, 0)
        const expected = scenario === 'bad-model' ? ['native-verify']
          : scenario === 'bad-inference' ? ['native-verify', 'emulator', 'prepared-inference'] : []
        assert.deepEqual(events, expected)
        if (scenario === 'missing-filter') assert.match(result.stderr, /Expected exactly one ignored test/)
      }
    } finally { rmSync(root, { recursive: true, force: true }) }
  })
}
