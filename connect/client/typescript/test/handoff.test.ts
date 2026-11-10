import { test } from 'node:test';
import assert from 'node:assert/strict';
import { handoffPointer, alternatives, fetchFailure } from '../src/handoff.ts';

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

test('recipient failures preserve 404 ambiguity and approved-origin guidance', () => {
  assert.match(fetchFailure('unavailable')!, /Capture unavailable.*sender may still be uploading/);
  assert.match(fetchFailure('unavailable')!, /expired or been deleted.*account.*API origin/);
  assert.match(fetchFailure('unavailable')!, /fresh handoff.*--wait 120/);
  assert.match(fetchFailure('authorization')!, /authorization failed.*configured API origin/);
  assert.equal(fetchFailure('unknown'), null);
});
