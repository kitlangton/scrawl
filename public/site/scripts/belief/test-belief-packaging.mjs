import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

assert(process.env.SITE_BASELINE, 'SITE_BASELINE must name an initial, pre-belief release')
const script = fileURLToPath(new URL('./package-belief-release.mjs', import.meta.url))
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'belief-packaging-'))
function pack(baseline, name) {
  const candidate = path.join(tmp, name), manifest = path.join(tmp, name + '.json')
  const result = spawnSync(process.execPath, ['--experimental-strip-types', script, baseline, candidate, manifest], {encoding:'utf8'})
  return {candidate, manifest, result}
}
try {
  const first = pack(path.resolve(process.env.SITE_BASELINE), 'first')
  assert.equal(first.result.status, 0, first.result.stderr)
  const second = pack(first.candidate, 'second')
  assert.equal(second.result.status, 0, second.result.stderr)
  const html = fs.readFileSync(path.join(second.candidate, 'index.html'), 'utf8')
  assert.equal((html.match(/<apparent-belief\b/g) || []).length, 1)
  assert.equal((html.match(/<link rel="stylesheet" href="\/assets\/belief-/g) || []).length, 1)
  assert.equal((html.match(/<script type="module" src="\/assets\/belief-/g) || []).length, 1)
  assert(html.includes('</section></apparent-belief></section><section class="pond-team"'), 'Nested belief and wrapper sections close before the preserved team section')
  assert(html.includes('</section></main><footer class="frog-footer">'), 'Preserved sibling sections remain inside main through its closing boundary')
  assert.equal(fs.readFileSync(path.join(first.candidate, 'index.html'), 'utf8'), html, 'Repeat packaging is idempotent')
  assert.deepEqual(JSON.parse(fs.readFileSync(second.manifest)).added, [])
  fs.writeFileSync(path.join(first.candidate, 'pond/belief/hug.png'), 'conflicting fixture')
  const conflict = pack(first.candidate, 'conflict')
  assert.notEqual(conflict.result.status, 0)
  assert.match(conflict.result.stderr, /Immutable asset conflict: pond\/belief\/hug\.png/)
  console.log('PASS: initial overlay, nested repeat, single resource pair, idempotence, immutable asset conflict refusal')
} finally {
  fs.rmSync(tmp, {recursive:true, force:true})
}
