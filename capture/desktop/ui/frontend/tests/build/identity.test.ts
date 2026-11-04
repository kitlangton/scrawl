import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatBuildIdentity, UNKNOWN_BUILD_IDENTITY } from '../../../../../../shared/ui/src/build-identity/label.ts';

test('build identity shows only a valid short commit and tracked edit marker', () => {
  assert.equal(formatBuildIdentity('Version 0.3.13 · ad8d4b1'), 'ad8d4b1');
  assert.equal(formatBuildIdentity('0.3.13 · ABCD123 +modified'), 'abcd123 +modified');
});

test('build identity uses the honest unknown label for absent, failed, or malformed values', () => {
  for (const value of [undefined, null, '', 'build unknown', '0.3.13 · ad8d4b', '0.3.13 · ad8d4b1 extra']) {
    assert.equal(formatBuildIdentity(value), UNKNOWN_BUILD_IDENTITY);
  }
});
