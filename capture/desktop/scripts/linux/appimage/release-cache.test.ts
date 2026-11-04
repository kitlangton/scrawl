import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const workflow = readFileSync(new URL('../../../../../.github/workflows/release-linux.yml', import.meta.url), 'utf8')
const linux = workflow.split('  linux:\n')[1]?.split('\n  publish:\n')[0]
assert.ok(linux, 'release-linux must retain its linux packaging job')

function step(name: string): string {
  const marker = `      - name: ${name}\n`
  const start = linux.indexOf(marker)
  assert.notEqual(start, -1, `missing workflow step: ${name}`)
  const rest = linux.slice(start + marker.length)
  const end = rest.search(/^      - /m)
  return end < 0 ? rest : rest.slice(0, end)
}

test('Linux release caches use bounded paths and exact immutable identities', () => {
  const paths = step('Configure release cache paths')
  assert.ok(paths.includes('echo "APPARENT_TEST_MODEL_DIR=$RUNNER_TEMP/speech-model" >> "$GITHUB_ENV"'))
  assert.ok(paths.includes('echo "APPARENT_PACKAGING_TOOLS_MANIFEST=$RUNNER_TEMP/packaging-tools.json" >> "$GITHUB_ENV"'))
  assert.ok(paths.includes('echo "APPARENT_PACKAGING_TOOLS_DIR=$RUNNER_TEMP/packaging-tools" >> "$GITHUB_ENV"'))

  const node = linux.slice(linux.indexOf('      - uses: actions/setup-node@v4'))
    .split('\n      - ')[0]
  assert.match(node, /node-version: '22\.23\.0'/)
  assert.match(node, /cache: npm/)
  assert.match(node, /cache-dependency-path: capture\/desktop\/ui\/frontend\/package-lock\.json/)

  const model = step('Cache verified speech model')
  assert.match(model, /uses: actions\/cache@v4/)
  assert.match(model, /path: \$\{\{ runner\.temp \}\}\/speech-model/)
  assert.match(model, /key: release-linux-speech-model-\$\{\{ hashFiles\('capture\/engine\/runtime\/audio\/transcription\/speech\.rs'\) \}\}/)
  assert.doesNotMatch(model, /restore-keys:/)

  const electron = step('Cache Electron download')
  assert.match(electron, /path: ~\/\.cache\/electron/)
  assert.match(electron, /key: release-linux-electron-\$\{\{ runner\.os \}\}-\$\{\{ runner\.arch \}\}-\$\{\{ hashFiles\('capture\/desktop\/ui\/frontend\/package-lock\.json'\) \}\}/)
  assert.doesNotMatch(electron, /restore-keys:/)

  const resolver = step('Resolve pinned packaging tools')
  assert.match(resolver, /id: packaging-tools/)
  assert.match(resolver, /node --experimental-strip-types capture\/desktop\/scripts\/linux\/appimage\/packaging-tools\.ts resolve "\$RUNNER_TEMP\/packaging-tools\.json" "\$GITHUB_OUTPUT"/)
  assert.doesNotMatch(linux, /GH_TOKEN:/)

  const tools = step('Cache verified packaging tools')
  assert.match(tools, /path: \$\{\{ runner\.temp \}\}\/packaging-tools/)
  assert.match(tools, /key: release-linux-packaging-tools-\$\{\{ matrix\.arch \}\}-\$\{\{ steps\.packaging-tools\.outputs\.key \}\}/)
  assert.doesNotMatch(tools, /restore-keys:/)
  assert.ok(linux.indexOf('      - name: Resolve pinned packaging tools') < linux.indexOf('      - name: Cache verified packaging tools'))
  assert.ok(linux.indexOf('      - name: Cache verified packaging tools') < linux.indexOf('      - name: Package\n'))
})

test('cache hits cannot skip smoke gates or change release admission', () => {
  assert.doesNotMatch(linux, /cache-hit/)
  for (const name of [
    'AppImage runs headless without FUSE',
    'Bundled GStreamer decodes an H.264 MP4',
    'Bundled GStreamer encodes a playable H.264 MP4',
    'Run two seconds of audio with the packaged CPU backend',
  ]) {
    const gate = step(name)
    assert.doesNotMatch(gate, /^\s+if:/m, `${name} must run on cache hits and misses`)
    assert.match(gate, /\brun:/)
  }
  const packagedModel = step('Run two seconds of audio with the packaged CPU backend')
  assert.match(packagedModel, /smoke-cpu\.sh "dist\/\$\{\{ steps\.package\.outputs\.name \}\}" "\$SPEECH_TEST"/)
  assert.match(linux, /name: Run packaged model on Ice Lake \(AVX512-VNNI, no AVX-VNNI\)/)
  assert.match(linux, /\.\/capture\/desktop\/scripts\/linux\/appimage\/appimage\.sh target\/release\/apparent/)

  const publish = workflow.split('  publish:\n')[1]
  assert.ok(publish)
  assert.match(workflow, /needs: \[admit, preview-needed, linux\]/)
  assert.match(workflow, /if: always\(\) && needs\.admit\.result == 'success'.*needs\.linux\.result == 'success'/)
  assert.match(publish, /name: Publish admitted immutable artifact identities and complete ancestry/)
  assert.match(publish, /name: Publish unchanged verified assets/)
})
