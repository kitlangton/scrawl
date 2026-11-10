import {test} from 'node:test';
import assert from 'node:assert/strict';
import {LocationWorkflow} from '../dist/index.js';
import type {LocationWorkflowPorts, NativeLocationFacts} from '../dist/index.js';
const facts: NativeLocationFacts = {revision: '0123456789abcdef', saved: {kind: 'absent'},
  environment: {present: false, value: null}, homePath: '/home/owner', builtinPath: '/captures'};
function fixture(overrides: Partial<LocationWorkflowPorts> = {}) {
  const writes: unknown[] = [], probes: unknown[] = [];
  let choices = 0;
  const ports: LocationWorkflowPorts = {
    facts: async () => ({status: 'connected', location: facts}),
    probe: async (path, expected) => {probes.push({path, expected}); return {status: 'connected', available: true};},
    choose: async path => {assert.equal(path, '/captures'); choices++; return {canceled: false, filePaths: [' /selected captures ']};},
    write: async (operation, fields) => {writes.push({operation, fields}); return {status: 'connected', location: facts};},
    ...overrides,
  };
  return {workflow: new LocationWorkflow(ports), writes, probes, choices: () => choices};
}
test('location workflow uses fresh revision, normalizes intent, and projects no revision', async () => {
  const f = fixture();
  const result = await f.workflow.mutate('change', () => true);
  assert.equal(result.status, 'connected'); assert.equal('revision' in result, false);
  assert.deepEqual(f.writes, [{operation: 'set', fields: {expected: facts.revision, path: '/selected captures'}}]);
  assert.equal(f.choices(), 1);
});
test('reset removes the leaf through the native executor without opening a chooser', async () => {
  const f = fixture(); await f.workflow.mutate('reset', () => true);
  assert.deepEqual(f.writes, [{operation: 'reset', fields: {expected: facts.revision}}]); assert.equal(f.choices(), 0);
});
test('cancel and malformed chooser results never publish', async () => {
  for (const selected of [{canceled: true, filePaths: ['/valid']}, {canceled: false, filePaths: []},
    {canceled: false, filePaths: ['/one', '/two']}, {canceled: false, filePaths: ['relative']}]) {
    const f = fixture({choose: async () => selected});
    const result = await f.workflow.mutate('change', () => true);
    assert.equal(result.status, selected.canceled ? 'cancelled' : 'error'); assert.deepEqual(f.writes, []);
  }
});
test('closing while the chooser is open cannot publish a preference', async () => {
  let current = true;
  const f = fixture({choose: async () => {current = false; return {canceled: false, filePaths: ['/valid']};}});
  assert.deepEqual(await f.workflow.mutate('change', () => current), {status: 'cancelled'});
  assert.deepEqual(f.writes, []);
});
test('native conflict and unknown outcomes propagate without a second write', async () => {
  for (const code of ['conflict', 'outcome_unknown']) {
    let calls = 0;
    const f = fixture({write: async () => {calls++; return {status: 'error', code};}});
    assert.deepEqual(await f.workflow.mutate('reset', () => true), {status: 'error', code}); assert.equal(calls, 1);
  }
  let calls = 0;
  const f = fixture({write: async () => {calls++; throw Error('transport lost');}});
  assert.deepEqual(await f.workflow.mutate('reset', () => true), {status: 'error', code: 'outcome_unknown'});
  assert.equal(calls, 1);
});
test('configured location probe uses the same facts revision and native availability', async () => {
  const configured = {...facts, saved: {kind: 'string' as const, value: '~/chosen'}};
  const f = fixture({facts: async () => ({status: 'connected', location: configured})});
  const result = await f.workflow.read(); assert.equal(result.status, 'connected');
  assert.deepEqual(f.probes, [{path: '/home/owner/chosen', expected: facts.revision}]);
  if (result.status === 'connected') assert.equal(result.storage.effectivePath, '/home/owner/chosen');
});
test('stale publication/readback is suppressed, without replaying native execution', async () => {
  let current = true, calls = 0;
  const f = fixture({write: async () => {calls++; current = false; return {status: 'connected', location: facts};}});
  assert.deepEqual(await f.workflow.mutate('reset', () => current), {status: 'cancelled'}); assert.equal(calls, 1);
});
test('post-save facts and revision-checked native availability determine authoritative readback', async () => {
  const saved = {...facts, revision: 'fedcba9876543210', saved: {kind: 'string' as const, value: '/saved'}};
  const f = fixture({write: async () => ({status: 'connected', location: saved}),
    probe: async (path, expected) => {assert.equal(path, '/saved'); assert.equal(expected, saved.revision); return {status: 'connected', available: false};}});
  const result = await f.workflow.mutate('reset', () => true);
  assert.equal(result.status, 'connected');
  if (result.status === 'connected') {
    assert.equal(result.storage.savedPath, '/saved'); assert.equal(result.storage.effectivePath, '/captures');
    assert.equal(result.storage.source, 'fallback'); assert.equal('revision' in result, false);
  }
});
test('a post-publication resolution failure is unknown and never replays the write', async () => {
  let calls = 0;
  const f = fixture({write: async () => {calls++; return {status: 'connected', location: {...facts, revision: 'invalid'}};}});
  assert.deepEqual(await f.workflow.mutate('reset', () => true), {status: 'error', code: 'outcome_unknown'});
  assert.equal(calls, 1);
});
