import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createRecordFixture } from '@apparent/record/fixtures';
import {
  cloneCapabilitySnapshot, cloneCommandResult, cloneJsonDto, cloneRecordCommand,
} from '../../src/record/wire.ts';

test('preserves exact optional public initial-audio-gap warnings without implying recorded silence', () => {
  const snapshot = createRecordFixture('unsubscribe-during-recording').driver.current();
  assert.ok(snapshot.captures.length);
  const old = structuredClone(snapshot);
  assert.deepEqual(cloneCapabilitySnapshot(old), old);
  const current = structuredClone(snapshot) as any;
  current.captures[0].warnings = [{ kind: 'audioInitialGap', atMs: 140 }];
  assert.deepEqual(cloneCapabilitySnapshot(current), current);
  current.captures[0].warnings = [];
  assert.deepEqual(cloneCapabilitySnapshot(current), current);
  for (const warnings of [undefined, null, [{ kind: 'audioInitialGap', atMs: -1 }],
    [{ kind: 'audioInitialGap', atMs: 1.5 }], [{ kind: 'audioInitialGap', atMs: Number.MAX_SAFE_INTEGER + 1 }],
    [{ kind: 'audioInitialGap', atMs: 3, extra: true }], [{ kind: 'silence', atMs: 3 }],
    Array.from({ length: 257 }, () => ({ kind: 'audioInitialGap', atMs: 3 }))]) {
    current.captures[0].warnings = warnings;
    assert.equal(cloneCapabilitySnapshot(current), undefined);
  }
});

test('accepts public fixture snapshots, fixed command variants, and correlated results', async () => {
  const fixture = createRecordFixture();
  const snapshot = fixture.driver.current();
  assert.deepEqual(cloneCapabilitySnapshot(snapshot), snapshot);

  const refresh = { type: 'refresh', requestId: 'wire-refresh', expected: {
    providerEpoch: snapshot.providerEpoch, revision: snapshot.revision,
  } } as const;
  assert.deepEqual(cloneRecordCommand(refresh), refresh);
  const result = await fixture.client.execute(refresh);
  assert.deepEqual(cloneCommandResult(result, refresh.requestId), result);
  assert.equal(cloneCommandResult(result, 'different-request'), undefined);

  const selectPen = { type: 'select-pen', value: 'coral', requestId: 'wire-pen', expected: {
    providerEpoch: snapshot.providerEpoch, revision: snapshot.revision,
  } } as const;
  assert.deepEqual(cloneRecordCommand(selectPen), selectPen);
  assert.equal(cloneRecordCommand({ ...selectPen, value: 'system-default' }), undefined);
});

test('rejects selector fields, exotic objects, accessors, cycles, sparse and oversized graphs', () => {
  const fixture = createRecordFixture();
  const snapshot = fixture.driver.current();
  const command = { type: 'refresh', requestId: 'wire-invalid', expected: {
    providerEpoch: snapshot.providerEpoch, revision: snapshot.revision,
  } };
  assert.equal(cloneRecordCommand({ ...command, owner: 'onboarding' }), undefined);

  const accessor = Object.defineProperty({}, 'value', { enumerable: true, get() { throw new Error('must not run'); } });
  assert.equal(cloneJsonDto(accessor), undefined);
  const cyclic: Record<string, unknown> = {};
  cyclic.self = cyclic;
  assert.equal(cloneJsonDto(cyclic), undefined);
  assert.equal(cloneJsonDto(new Date()), undefined);
  assert.equal(cloneJsonDto([, 'sparse']), undefined);
  assert.equal(cloneJsonDto({ value: 'x'.repeat(64 * 1024) }), undefined);
  assert.equal(cloneJsonDto({ ['__proto__']: { polluted: true } }), undefined);
});

test('preserves authenticated native copy receipts from the public Record contract without accepting malformed evidence', () => {
  const current = createRecordFixture('unsubscribe-during-recording').driver.current();
  const receipt = { kind: 'copy', at: '2026-11-06T13:00:00Z', ok: true,
    evidence: 'native-write-readback', artifactId: 'artifact-synthetic', requestId: 'copy-synthetic',
    digest: 'a'.repeat(64), byteLength: 78 };
  const withReceipt = (outcome: unknown) => ({ ...current, captures: current.captures.map(capture => ({
    ...capture, phase: 'saved', artifactId: receipt.artifactId, outcomes: [outcome],
  })) });
  const snapshot = withReceipt(receipt);
  assert.deepEqual(cloneCapabilitySnapshot(snapshot), snapshot);
  for (const invalid of [
    { ...receipt, evidence: 'dispatch' }, { ...receipt, ok: false },
    { ...receipt, digest: 'A'.repeat(64) }, { ...receipt, byteLength: 0 },
    { ...receipt, byteLength: Number.MAX_SAFE_INTEGER + 1 }, { ...receipt, requestId: '' },
    { ...receipt, artifactId: '' }, { ...receipt, ambientClipboard: 'forbidden' },
  ]) assert.equal(cloneCapabilitySnapshot(withReceipt(invalid)), undefined);
});
