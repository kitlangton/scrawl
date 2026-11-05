/** Validate an externally attested baseline payload against a current renderer candidate. */
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { environmentSnapshot } from './producer.ts'

const self = fileURLToPath(import.meta.url)
const sha = (b: Buffer | string) => createHash('sha256').update(b).digest('hex')
export function validate(receipt: any, payloadBytes: Buffer, expected: {
  repository: string; platform: string; arch: string; target: string; profile: string;
  sourceSha: string; sourceTree: string; workflowBlob: string; environment: Record<string, unknown>;
  sourceInputs: Array<{ path: string; sha256: string; mode?: number }>;
}) {
  assert(receipt?.schema === 2 && receipt.kind === 'successful-full-native-verification-baseline', 'Unsupported baseline receipt')
  assert(receipt.completeness?.eligible === false && receipt.completeness.externalInputs === false,
    'Baseline does not prove a reusable complete closure')
  assert.equal(sha(payloadBytes), receipt.payload?.sha256, 'Baseline payload digest mismatch')
  assert.equal(payloadBytes.length, receipt.payload?.bytes, 'Baseline payload size mismatch')
  const payload = JSON.parse(payloadBytes.toString('utf8'))
  for (const [key, value] of Object.entries({ sourceSha: receipt.sourceSha, sourceTree: receipt.sourceTree, workflowBlob: receipt.workflowBlob }))
    assert.equal(payload[key], value, `Payload ${key} mismatch`)
  assert.equal(receipt.run?.repository, expected.repository)
  assert.equal(receipt.platform?.os, expected.platform)
  assert.equal(receipt.platform?.arch, expected.arch)
  assert.equal(receipt.platform?.target, expected.target)
  assert.equal(receipt.profile, expected.profile)
  assert.equal(payload.workflowBlob, expected.workflowBlob, 'Verification workflow changed')
  assert.deepEqual(receipt.environment?.image, expected.environment, 'Native toolchain/environment changed')
  assert.deepEqual(payload.trackedInputs, expected.sourceInputs, 'Native source/runtime/test input closure changed')
  assert.equal(expected.sourceSha, receipt.sourceSha, 'This validator only admits exact-source candidates')
  return { decision: 'full-native-required', sourceSha: receipt.sourceSha }
}
if (process.argv[1] && resolve(process.argv[1]) === self) {
  assert(process.argv.length === 5, 'Usage: consumer.ts RECEIPT PAYLOAD EXPECTED_JSON')
  const receiptBytes = readFileSync(process.argv[2])
  const receipt = JSON.parse(receiptBytes.toString('utf8'))
  const expected = JSON.parse(readFileSync(process.argv[4], 'utf8'))
  validate(receipt, readFileSync(process.argv[3]), expected)
  console.log('Baseline validated; full native verification remains required until closure is eligible.')
}
