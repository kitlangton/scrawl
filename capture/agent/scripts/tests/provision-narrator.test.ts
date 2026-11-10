import { test } from 'node:test';
import assert from 'node:assert/strict';
import { plan, validWav } from '../provision-narrator.ts';

test('installed worker and pinned assets reuse without build or download', () => {
  assert.deepEqual(plan({ assets: true, worker: true }, false), []);
  assert.deepEqual(plan({ assets: true, worker: true }, true, true), ['build']);
  assert.deepEqual(plan({ assets: false, worker: true }, false), ['assets']);
});
test('first setup exposes the source-build prerequisite before downloading', () => {
  assert.throws(() => plan({ assets: false, worker: false }, false), /No published narrator worker/);
  assert.deepEqual(plan({ assets: false, worker: false }, true), ['assets', 'build']);
});
test('readiness rejects missing, truncated, wrong-format synthetic WAV', () => {
  const data = Buffer.alloc(48);
  data.write('RIFF'); data.writeUInt32LE(40, 4); data.write('WAVE', 8);
  data.write('fmt ', 12); data.writeUInt32LE(16, 16); data.writeUInt32LE(48000, 28); data.writeUInt16LE(2, 32);
  data.writeUInt16LE(1, 20); data.writeUInt16LE(1, 22); data.writeUInt32LE(24000, 24);
  data.writeUInt16LE(16, 34); data.write('data', 36); data.writeUInt32LE(4, 40); data.writeInt16LE(100, 44);
  assert.equal(validWav(data), true);
  assert.equal(validWav(data.subarray(0, 46)), false);
  data.writeUInt32LE(16000, 24); assert.equal(validWav(data), false);
  assert.equal(validWav(Buffer.alloc(0)), false);
});
