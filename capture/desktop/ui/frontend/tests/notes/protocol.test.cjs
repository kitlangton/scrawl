const { test } = require('node:test');
const assert = require('node:assert/strict');
const { validCommand, validSnapshot, popupBounds, badgeBounds } = require('../../electron/notes/protocol.cjs');
test('notes bridge accepts only bounded note actions, never renderer paths or window commands', () => {
  assert.equal(validCommand({ action: 'save', pin: 1, text: 'Change this\nnot that' }), true);
  for (const message of [{ action: 'save', pin: 1, text: 'x', path: '/tmp/private' },
    { action: 'windows', ids: [9] }, { action: 'surface', id: 9, rect: [0,0,10,10], focus: true }, { action: 'ready' }, { action: 'open', pin: 1 },
    { action: 'save', pin: 1, text: 'é'.repeat(4097) }, { action: 'delete', pin: -1 }]) {
    assert.ok(!validCommand(message));
  }
});
test('notes snapshot retains an exact click time and validates draft and saved text', () => {
  const n = { pin: 1, t: 1234, x: .25, y: .5, text: 'Keep me' };
  const s = { mode: 'editor', current: 1, position: [-300, 42], notes: [], drafts: [n], error: null };
  assert.equal(validSnapshot(s), true);
  assert.equal(validSnapshot({ ...s, drafts: [{ ...n, x: NaN }] }), false);
  assert.equal(validSnapshot({ ...s, notes: [{ ...n, text: '\0' }] }), false);
});
test('popup stays inside work areas including negative display coordinates and small screens', () => {
  assert.deepEqual(popupBounds({ x: -1, y: 999 }, { x: -1280, y: 0, width: 1280, height: 800 }, 220),
    { x: -332, y: 580, width: 332, height: 220 });
  assert.deepEqual(popupBounds({ x: 300, y: 300 }, { x: 0, y: 0, width: 240, height: 180 }, 350),
    { x: 0, y: 0, width: 240, height: 180 });
});

test('counter falls left or below instead of covering the capsule at a display edge', () => {
  const area = { x: -1280, y: 0, width: 1280, height: 800 };
  assert.deepEqual(badgeBounds([-1000, 40, 350, 40], area), { x: -644, y: 40 });
  assert.deepEqual(badgeBounds([-370, 40, 350, 40], area), { x: -526, y: 40 });
  assert.deepEqual(badgeBounds([-1270, 40, 1260, 40], area), { x: -715, y: 86 });
});
