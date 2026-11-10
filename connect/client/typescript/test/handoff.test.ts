import { test } from 'node:test';
import assert from 'node:assert/strict';
import { handoffPointer, alternatives } from '../src/handoff.ts';

test('only pending and ready captures select Connect, never sign-in or terminal jobs', () => {
  const id = 'a'.repeat(32);
  for (const status of ['pending', 'ready']) {
    assert.equal(handoffPointer({ id, status }), `apparent inspect ${id}\n`);
  }
  for (const status of ['failed', 'expired', 'cancelled', 'awaiting_share', 'unknown', undefined]) {
    assert.equal(handoffPointer({ id, status }), null);
  }
  for (const value of [null, { authenticated: true }, { id: '../private', status: 'ready' }]) {
    assert.equal(handoffPointer(value), null);
  }
});

test('recovery explains account admission, complete local transfer and consent', () => {
  assert.match(alternatives, /waitlisted accounts cannot approve devices/);
  assert.match(alternatives, /scp -r/);
  assert.match(alternatives, /apparent inspect \.\/capture.apparent/);
  assert.match(alternatives, /do not enable remote access/);
  assert.match(alternatives, /Previously pasted Connect IDs do not change/);
});
