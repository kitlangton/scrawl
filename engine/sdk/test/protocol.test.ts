import test from 'node:test';
import assert from 'node:assert/strict';
import { runV1Scenarios, referenceEngineScenario, v1Scenarios } from './scenarios.ts';
test('v1 contract scenarios (test reference only)',async()=>{assert.equal(v1Scenarios.length,8);await runV1Scenarios(referenceEngineScenario);});
