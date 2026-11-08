import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  isLocationSnapshot, isRetentionPolicy, isStoredRetention, retentionForPreset, retentionView,
} from '../../src/storage/policy.ts';

test('retention defaults, choices and month policy are owned by TypeScript', () => {
  const defaults = retentionView({
    mediaDays: { present: false, value: null }, bundleDays: { present: false, value: null },
  });
  assert.equal(defaults.custom, false);
  assert.equal(defaults.retention.preset, 'week');
  assert.deepEqual(defaults.retention.options.map(({ id }) => id), ['week', 'month', 'forever']);
  assert.equal(defaults.retention.note, 'Video and audio go to the Trash after 7 days, the rest of each capture after 30 days.');
  assert.deepEqual(retentionForPreset('week'), { mediaDays: 7, bundleDays: 30 });
  assert.deepEqual(retentionForPreset('month'), { mediaDays: 30, bundleDays: 90 });
  assert.deepEqual(retentionForPreset('forever'), { mediaDays: null, bundleDays: null });
});

test('partial custom values preserve absent-versus-null meaning and render truthful notes', () => {
  const partial = retentionView({
    mediaDays: { present: true, value: 2 }, bundleDays: { present: false, value: null },
  });
  assert.equal(partial.custom, true);
  assert.equal(partial.retention.preset, null);
  assert.equal(partial.retention.note, 'Video and audio go to the Trash after 2 days, the rest of each capture after 30 days.');
  const forever = retentionView({
    mediaDays: { present: true, value: null }, bundleDays: { present: true, value: null },
  });
  assert.equal(forever.retention.preset, 'forever');
  assert.equal(forever.retention.note, '');
});

test('a malformed stored policy can show the native cleanup default without exposing its value', () => {
  const recovered = retentionView({
    mediaDays: { present: false, value: null }, bundleDays: { present: false, value: null },
  }, true);
  assert.equal(recovered.custom, true);
  assert.equal(recovered.retention.preset, 'week');
  assert.equal(recovered.retention.note, 'Video and audio go to the Trash after 7 days, the rest of each capture after 30 days.');
});

test('storage preference schemas fail closed on malformed or unbounded values', () => {
  assert.equal(isStoredRetention({ mediaDays: { present: true, value: 0 }, bundleDays: { present: true, value: null } }), true);
  assert.equal(isStoredRetention({ mediaDays: { present: false, value: 7 }, bundleDays: { present: false, value: null } }), false);
  assert.equal(isStoredRetention({ mediaDays: { present: true, value: -1 }, bundleDays: { present: false, value: null } }), false);
  assert.equal(isRetentionPolicy({ mediaDays: 7, bundleDays: 30 }), true);
  assert.equal(isRetentionPolicy({ mediaDays: null, bundleDays: 90, other: true }), false);
  assert.equal(isRetentionPolicy({ mediaDays: 0x1_0000_0000, bundleDays: 30 }), false);
});

test('capture-location readback preserves saved/effective distinction and source semantics', () => {
  assert.equal(isLocationSnapshot({ savedPath: '/saved', effectivePath: '/env', displayPath: '/env',
    custom: true, source: 'environment' }), true);
  assert.equal(isLocationSnapshot({ savedPath: null, effectivePath: '/builtin', displayPath: '~/.captures',
    custom: false, source: 'builtin' }), true);
  assert.equal(isLocationSnapshot({ savedPath: null, effectivePath: '/saved', displayPath: '/saved',
    custom: false, source: 'configured' }), false);
  assert.equal(isLocationSnapshot({ savedPath: '/saved', effectivePath: '/builtin', displayPath: '/builtin',
    custom: true, source: 'fallback' }), true);
  assert.equal(isLocationSnapshot({ savedPath: '/saved', effectivePath: '/saved\0secret', displayPath: '/saved',
    custom: true, source: 'configured' }), false);
});
