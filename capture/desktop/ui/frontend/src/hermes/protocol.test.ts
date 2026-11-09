import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Recording, destination } from './protocol.ts';
const id = '0123456789abcdef0123456789abcdef';
test('capture ownership survives readiness timeout without starting another take', async () => {
  const calls: string[][] = [];
  const recording = new Recording(id, async args => { calls.push(args); return { capture_id: id, status: args[1] === 'start' ? 'starting' : 'saved', error: args[1] === 'start' ? 'timeout' : null, source: '/private/owned.apparent' }; });
  assert.equal((await recording.start()).error, 'timeout');
  await recording.inspect();
  assert.equal(await recording.source(), '/private/owned.apparent');
  assert.deepEqual(calls.map(c => c[1]), ['start', 'inspect', 'handoff']);
  assert.ok(calls.every(c => c.includes(id)));
});
test('another capture cannot provide a result, and occupied admission cannot become success', async () => {
  const wrong = new Recording(id, async () => ({ capture_id: 'other', status: 'saved', source: '/private/other.apparent' }));
  await assert.rejects(wrong.source());
  const failed = new Recording(id, async () => ({ capture_id: id, error: 'occupied' }));
  await assert.rejects(failed.start());
});
test('launch request accepts only a bounded loopback port and opaque token', () => {
  assert.deepEqual(destination(`${id}/45000`), { token: id, port: 45000 });
  for (const suffix of ['45000/extra', '80', '45000?callback=evil', '../45000']) assert.throws(() => destination(`${id}/${suffix}`));
});
