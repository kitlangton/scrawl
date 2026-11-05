import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import test from 'node:test'
import { validate } from './consumer.ts'

const hash = (value: string | Buffer) => createHash('sha256').update(value).digest('hex')
function fixture() {
  const payload = Buffer.from(JSON.stringify({ sourceSha: 'a'.repeat(40), sourceTree: 'b'.repeat(40),
    workflowBlob: 'c'.repeat(40), trackedInputs: [{ path: 'main.rs', sha256: hash('native') }] }))
  const receipt = { schema: 2, kind: 'successful-full-native-verification-baseline', reusable: false,
    completeness: { externalInputs: false, eligible: false }, sourceSha: 'a'.repeat(40), sourceTree: 'b'.repeat(40),
    workflowBlob: 'c'.repeat(40), run: { repository: 'org/repo' }, platform: { os: 'linux', arch: 'x64', target: 'x86_64-unknown-linux-gnu' },
    profile: 'debug', environment: { image: {} }, payload: { sha256: hash(payload), bytes: payload.length } }
  const expected = { repository: 'org/repo', platform: 'linux', arch: 'x64', target: 'x86_64-unknown-linux-gnu',
    profile: 'debug', sourceSha: 'a'.repeat(40), sourceTree: 'b'.repeat(40), workflowBlob: 'c'.repeat(40),
    environment: {}, sourceInputs: [{ path: 'main.rs', sha256: hash('native') }] }
  return { receipt, payload, expected }
}

test('matching authenticated payload remains full-native-required while closure is incomplete', () => {
  const f = fixture()
  assert.equal(validate(f.receipt, f.payload, f.expected).decision, 'full-native-required')
})

test('payload tampering, identity drift, and source closure changes fail closed', () => {
  const f = fixture()
  assert.throws(() => validate(f.receipt, Buffer.from('tampered'), f.expected), /payload digest mismatch/)
  assert.throws(() => validate(f.receipt, f.payload, { ...f.expected, target: 'aarch64-unknown-linux-gnu' }), /strictly equal/)
  assert.throws(() => validate(f.receipt, f.payload, { ...f.expected, sourceInputs: [] }), /closure changed/)
})
