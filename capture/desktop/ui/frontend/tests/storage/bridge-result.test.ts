import { test } from 'node:test';
import assert from 'node:assert/strict';
import { STORAGE_BRIDGE_TEST, verifyBridgeListing, verifyBridgeResult } from '../../scripts/settings/bridge-result.ts';

test('prepared bridge rejects a tampered or nonexistent test name', () => {
  assert.doesNotThrow(() => verifyBridgeListing(STORAGE_BRIDGE_TEST, 0, `${STORAGE_BRIDGE_TEST}: test\n`));
  assert.throws(() => verifyBridgeListing('nonexistent', 0, `${STORAGE_BRIDGE_TEST}: test\n`));
  assert.throws(() => verifyBridgeListing(STORAGE_BRIDGE_TEST, 0, 'other: test\n'));
  assert.throws(() => verifyBridgeListing(STORAGE_BRIDGE_TEST, 1, `${STORAGE_BRIDGE_TEST}: test\n`));
});

test('zero tests or an ignored bridge cannot certify native integration', () => {
  const passed = `test ${STORAGE_BRIDGE_TEST} ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 930 filtered out; finished in 0.5s\n`;
  assert.doesNotThrow(() => verifyBridgeResult(0, passed));
  assert.throws(() => verifyBridgeResult(1, passed));
  assert.throws(() => verifyBridgeResult(0, 'test result: ok. 0 passed; 0 failed; 0 ignored; 931 filtered out;\n'));
  assert.throws(() => verifyBridgeResult(0, `test ${STORAGE_BRIDGE_TEST} ... ignored\ntest result: ok. 0 passed; 0 failed; 1 ignored;\n`));
});
