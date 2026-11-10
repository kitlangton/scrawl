import {test} from 'node:test';
import assert from 'node:assert/strict';
import {RetentionWorkflow, isRetentionPolicy} from '../dist/index.js';
import type {RevisionedRetention, RetentionWorkflowPorts, UsageResult} from '../dist/index.js';
const revision = '0123456789abcdef', nextRevision = 'fedcba9876543210';
const result = (rev = revision): RevisionedRetention => ({status: 'connected', revision: rev, custom: false,
  policy: {mediaDays: {present: false, value: null}, bundleDays: {present: true, value: null}}});
function deferred<T>() {let resolve!: (value: T) => void, reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => {resolve = yes; reject = no;}); return {promise, resolve, reject};}
function fixture(overrides: Partial<RetentionWorkflowPorts> = {}) {
  const writes: unknown[] = [];
  const workflow = new RetentionWorkflow({read: async () => result(), usage: async () => ({status: 'disconnected'}),
    write: async (operation, fields) => {writes.push({operation, fields}); return result(nextRevision);}, ...overrides});
  return {workflow, writes};
}
test('retention intent accepts only own bounded known fields', () => {
  for (const policy of [{mediaDays: 0, bundleDays: 0xffffffff}, {mediaDays: null, bundleDays: null}]) assert.equal(isRetentionPolicy(policy), true);
  for (const policy of [null, [], {mediaDays: -1, bundleDays: 2}, {mediaDays: 1.2, bundleDays: 2},
    {mediaDays: 0x100000000, bundleDays: 2}, {mediaDays: 1}, {mediaDays: 1, bundleDays: 2, extra: true},
    Object.assign(Object.create({mediaDays: 7, bundleDays: 30}), {one: true, two: true})]) assert.equal(isRetentionPolicy(policy), false);
});
test('coalesced reads preserve presence/null and project no revision or unknown fields', async () => {
  const pending = deferred<RevisionedRetention>(); let reads = 0;
  const {workflow} = fixture({read: () => {reads++; return pending.promise;}});
  const one = workflow.read(0, () => true), two = workflow.read(0, () => true);
  assert.equal(reads, 1); pending.resolve(Object.assign(result(), {secret: 'not projected'}));
  const values = await Promise.all([one, two]);
  for (const value of values) {
    assert.equal(value.status, 'connected'); assert.equal('revision' in value, false); assert.equal('secret' in value, false);
    if (value.status === 'connected') assert.deepEqual(value.policy, {mediaDays: {present: false, value: null}, bundleDays: {present: true, value: null}});
  }
  assert.equal(workflow.ready, true);
});
test('save and reset consume authoritative revisions and reset carries no policy', async () => {
  const {workflow, writes} = fixture();
  assert.deepEqual(await workflow.mutate('reset', undefined, () => true), {status: 'error', code: 'refresh_required'});
  await workflow.read(0, () => true);
  await workflow.mutate('save', {mediaDays: 7, bundleDays: 30}, () => true);
  await workflow.mutate('reset', undefined, () => true);
  assert.deepEqual(writes, [{operation: 'save', fields: {expected: revision, policy: {mediaDays: 7, bundleDays: 30}}},
    {operation: 'reset', fields: {expected: nextRevision}}]);
});
test('invalid policy and operation cannot dispatch a mutation', async () => {
  const {workflow, writes} = fixture(); await workflow.read(0, () => true);
  await assert.rejects(workflow.mutate('save', {mediaDays: -1, bundleDays: 30}, () => true), /Invalid retention/);
  assert.deepEqual(await workflow.mutate('unknown' as 'save', {}, () => true), {status: 'error', code: 'invalid_request'});
  assert.deepEqual(writes, []); assert.equal(workflow.ready, true);
});
test('uncertain publication requires readback and never replays the write', async () => {
  let writes = 0;
  const {workflow} = fixture({write: async () => {writes++; throw Error('transport lost');}});
  await workflow.read(0, () => true);
  assert.deepEqual(await workflow.mutate('reset', undefined, () => true), {status: 'error', code: 'outcome_unknown'});
  assert.equal(workflow.ready, false);
  assert.deepEqual(await workflow.mutate('reset', undefined, () => true), {status: 'error', code: 'refresh_required'});
  assert.equal(writes, 1); await workflow.read(2, () => true);
  await workflow.mutate('reset', undefined, () => true); assert.equal(writes, 2);
});
test('explicit conflict/refusal clears mutation readiness without automatic replay', async () => {
  for (const code of ['conflict', 'admission_expired', 'outcome_unknown', 'app_busy']) {
    let writes = 0; const {workflow} = fixture({write: async () => {writes++; return {status: 'error', code};}});
    await workflow.read(0, () => true);
    assert.deepEqual(await workflow.mutate('reset', undefined, () => true), {status: 'error', code});
    assert.equal(workflow.ready, false); assert.equal(writes, 1);
  }
});
test('a read during a pending write cannot authorize another intent after an uncertain result', async () => {
  for (const thrown of [false, true]) {
    const pending = deferred<RevisionedRetention>(); const {workflow} = fixture({write: () => pending.promise});
    await workflow.read(0, () => true); const write = workflow.mutate('reset', undefined, () => true);
    await workflow.read(1, () => true); assert.equal(workflow.ready, true);
    if (thrown) pending.reject(Error('transport lost')); else pending.resolve({status: 'error', code: 'outcome_unknown'});
    assert.deepEqual(await write, {status: 'error', code: 'outcome_unknown'}); assert.equal(workflow.ready, false);
    assert.deepEqual(await workflow.mutate('reset', undefined, () => true), {status: 'error', code: 'refresh_required'});
  }
});
test('invalid post-publication facts are unknown rather than trusted write readback', async () => {
  const {workflow} = fixture({write: async () => result('invalid')}); await workflow.read(0, () => true);
  assert.deepEqual(await workflow.mutate('reset', undefined, () => true), {status: 'error', code: 'outcome_unknown'});
  assert.equal(workflow.ready, false);
});
test('an old read cannot replace the revision from a newer version', async () => {
  const old = deferred<RevisionedRetention>(), fresh = deferred<RevisionedRetention>(); let reads = 0, version = 0;
  const {workflow, writes} = fixture({read: () => (++reads === 1 ? old : fresh).promise});
  const before = workflow.read(0, () => version === 0); version = 1;
  const after = workflow.read(1, () => version === 1); fresh.resolve(result(nextRevision)); await after;
  old.resolve(result()); assert.deepEqual(await before, {status: 'cancelled'});
  await workflow.mutate('reset', undefined, () => true);
  assert.deepEqual(writes, [{operation: 'reset', fields: {expected: nextRevision}}]);
});
test('invalidate retires pending reads and prevents revision resurrection', async () => {
  const pending = deferred<RevisionedRetention>(); const {workflow} = fixture({read: () => pending.promise});
  const read = workflow.read(0, () => true); workflow.invalidate(); pending.resolve(result());
  assert.deepEqual(await read, {status: 'cancelled'}); assert.equal(workflow.ready, false);
});
test('invalidate retires a pending mutation without replay or trusted readback', async () => {
  const pending = deferred<RevisionedRetention>(); let writes = 0;
  const {workflow} = fixture({write: () => {writes++; return pending.promise;}});
  await workflow.read(0, () => true); const write = workflow.mutate('reset', undefined, () => true);
  workflow.invalidate(); pending.resolve(result(nextRevision));
  assert.deepEqual(await write, {status: 'cancelled'}); assert.equal(workflow.ready, false); assert.equal(writes, 1);
});
test('usage results are cancelled by host version change or owner invalidation', async () => {
  for (const invalidate of [false, true]) {
    const pending = deferred<UsageResult>(); let current = true;
    const {workflow} = fixture({usage: () => pending.promise}); const usage = workflow.measure(() => current);
    if (invalidate) workflow.invalidate(); else current = false;
    pending.resolve({status: 'connected', usage: {effectivePath: '/captures', displayPath: '~/captures', formatted: '0 MB'}});
    assert.deepEqual(await usage, {status: 'cancelled'});
  }
});
test('rejected coalesced reads clear readiness and settle both consumers', async () => {
  const pending = deferred<RevisionedRetention>(); const {workflow} = fixture({read: () => pending.promise});
  const one = workflow.read(0, () => true), two = workflow.read(0, () => true); pending.reject(Error('closed'));
  assert.deepEqual(await Promise.all([one, two]), [{status: 'error', code: 'bridge_failed'}, {status: 'error', code: 'bridge_failed'}]);
  assert.equal(workflow.ready, false);
});
test('invalid revision/presence facts never grant write readiness', async () => {
  for (const raw of [result('invalid'), {...result(), revision: 1234567890123456}, {status: 'connected', revision, custom: true,
    policy: {mediaDays: {present: false, value: 7}, bundleDays: {present: true, value: null}}}]) {
    const {workflow} = fixture({read: async () => raw as RevisionedRetention});
    assert.deepEqual(await workflow.read(0, () => true), {status: 'error', code: 'invalid_response'}); assert.equal(workflow.ready, false);
  }
});
