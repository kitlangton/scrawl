import assert from 'node:assert/strict';
import {test} from 'node:test';
import {LocationController, RetentionController} from '../dist/index.js';
import type {CaptureLocation, NativeRetentionResult, UsageResult} from '../dist/index.js';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return {promise, resolve, reject};
}
const location = (effectivePath = '/captures'): CaptureLocation => ({status: 'connected', storage: {
  savedPath: null, effectivePath, displayPath: effectivePath, source: 'builtin', custom: false,
}});
const retention = (media: number | null = 7, bundle: number | null = 30): NativeRetentionResult => ({
  status: 'connected', custom: true,
  policy: {mediaDays: {present: true, value: media}, bundleDays: {present: true, value: bundle}},
});
const usage = (formatted = '1 MB', effectivePath = '/captures'): UsageResult => ({
  status: 'connected', usage: {effectivePath, displayPath: effectivePath, formatted},
});

test('location ignores superseded read replies and their completion flags', async () => {
  const first = deferred<CaptureLocation>(), second = deferred<CaptureLocation>();
  const reads = [first, second];
  const controller = new LocationController({read: () => reads.shift()!.promise, change: async () => location(), reset: async () => location()});
  const old = controller.request('read'), current = controller.request('read');
  first.resolve(location('/old')); await old;
  assert.equal(controller.state.busy, true);
  assert.equal(controller.state.snapshot, undefined);
  second.resolve(location('/new')); await current;
  assert.equal(controller.state.snapshot?.effectivePath, '/new');
  assert.equal(controller.state.busy, false);
});

test('chooser cancellation preserves readback and serializes mutation admission', async () => {
  const chooser = deferred<CaptureLocation>();
  let reads = 0, resets = 0, changes = 0;
  const controller = new LocationController({read: async () => { reads++; return location(); },
    change: () => { changes++; return chooser.promise; }, reset: async () => { resets++; return location(); }});
  await controller.request('read');
  const pending = controller.request('change');
  await Promise.all([controller.request('read'), controller.request('reset'), controller.request('change')]);
  assert.deepEqual([reads, resets, changes], [1, 0, 1]);
  chooser.resolve({status: 'cancelled'}); await pending;
  assert.deepEqual(controller.state.result, location());
  assert.equal(controller.state.busy, false);
});

test('unknown location outcome forbids another mutation until authoritative readback', async () => {
  let changes = 0;
  const controller = new LocationController({read: async () => location('/actual'),
    change: async () => { changes++; throw Error('reply lost'); }, reset: async () => location()});
  await controller.request('read'); await controller.request('change');
  assert.deepEqual(controller.state.result, {status: 'error', code: 'outcome_unknown'});
  assert.equal(controller.state.snapshot, undefined);
  await controller.request('change'); assert.equal(changes, 1);
  await controller.request('read'); assert.equal(controller.state.snapshot?.effectivePath, '/actual');
  await controller.request('change'); assert.equal(changes, 2);
});

test('closing while chooser is open retires renderer replies without claiming native cancellation', async () => {
  const chooser = deferred<CaptureLocation>();
  const controller = new LocationController({read: async () => location(), change: () => chooser.promise, reset: async () => location()});
  let notifications = 0;
  controller.subscribe(() => { notifications++; });
  await controller.request('read'); const pending = controller.request('change');
  controller.dispose(); const before = notifications;
  controller.subscribe(() => { notifications++; });
  chooser.resolve(location('/late')); await pending;
  assert.equal(notifications, before);
  assert.equal(controller.state.snapshot?.effectivePath, '/captures');
});

test('invalid location replies clear trusted state; stale show failures cannot replace fresh readback', async () => {
  const show = deferred<void>();
  let next = location();
  const controller = new LocationController({read: async () => next, change: async () => next, reset: async () => next, show: () => show.promise});
  await controller.request('read'); const pending = controller.show();
  next = location('/new'); await controller.request('read'); show.reject(Error('old OS failure')); await pending;
  assert.equal(controller.state.snapshot?.effectivePath, '/new');
  next = {status: 'connected', storage: {}} as CaptureLocation;
  await controller.request('read');
  assert.deepEqual(controller.state.result, {status: 'error', code: 'invalid_response'});
  assert.equal(controller.state.snapshot, undefined);
});

test('retention reads preserve absent/null/custom policy interpretation and validate provider data', async () => {
  let next: NativeRetentionResult = {status: 'connected', custom: false, policy: {
    mediaDays: {present: false, value: null}, bundleDays: {present: false, value: null},
  }};
  const controller = new RetentionController({read: async () => next, save: async () => next, reset: async () => next, usage: async () => usage()}, '/captures');
  await controller.request('read'); assert.equal(controller.state.preset, 'week');
  next = retention(null, null); await controller.request('read'); assert.equal(controller.state.preset, 'forever');
  next = retention(13, 44); await controller.request('read'); assert.equal(controller.state.preset, '');
  assert.equal(controller.state.result?.status, 'connected');
  next = retention(-1); await controller.request('read');
  assert.deepEqual(controller.state.result, {status: 'error', code: 'invalid_response'});
});

test('retention save intent is bounded and unknown publication requires readback', async () => {
  const write = deferred<NativeRetentionResult>();
  const policies: unknown[] = []; let reads = 0, resets = 0;
  const controller = new RetentionController({read: async () => { reads++; return retention(); },
    save: policy => { policies.push(policy); return write.promise; }, reset: async () => { resets++; return retention(); }, usage: async () => usage()}, '/captures');
  await controller.request('read'); await Promise.resolve();
  controller.select('invalid'); assert.equal(controller.state.preset, 'week');
  controller.select('month'); const pending = controller.request('save');
  controller.select('forever');
  await Promise.all([controller.request('read'), controller.request('save'), controller.request('reset')]);
  assert.deepEqual(policies, [{mediaDays: 30, bundleDays: 90}]);
  assert.deepEqual([reads, resets], [1, 0]);
  write.reject(Error('lost receipt')); await pending;
  assert.deepEqual(controller.state.result, {status: 'error', code: 'outcome_unknown'});
  await controller.request('reset'); assert.equal(resets, 0);
  await controller.request('read'); await controller.request('reset'); assert.equal(resets, 1);
});

test('pre-save usage and superseded measurements cannot overwrite current directory usage', async () => {
  const old = deferred<UsageResult>(), fresh = deferred<UsageResult>(), latest = deferred<UsageResult>();
  const scans = [old, fresh, latest];
  const controller = new RetentionController({read: async () => retention(), save: async () => retention(30, 90),
    reset: async () => retention(), usage: () => scans.shift()!.promise}, '/captures');
  await controller.request('read');
  await controller.request('reset');
  old.resolve(usage('obsolete')); await Promise.resolve();
  assert.equal(controller.state.usage, undefined); assert.equal(controller.state.scanning, true);
  const measurement = controller.measure();
  fresh.resolve(usage('superseded')); await Promise.resolve();
  assert.equal(controller.state.usage, undefined);
  latest.resolve(usage('current')); await measurement;
  assert.deepEqual(controller.state.usage, usage('current'));
});

test('usage from a changed native directory is a conflict; disposal retires outstanding scans', async () => {
  const pending = deferred<UsageResult>(); let calls = 0;
  const controller = new RetentionController({read: async () => retention(), save: async () => retention(), reset: async () => retention(),
    usage: () => ++calls === 1 ? Promise.resolve(usage('wrong', '/other')) : pending.promise}, '/captures');
  await controller.request('read'); await Promise.resolve();
  assert.deepEqual(controller.state.usage, {status: 'error', code: 'conflict'});
  const scan = controller.measure(); controller.dispose(); const state = controller.state;
  pending.resolve(usage()); await scan;
  assert.equal(controller.state, state);
  await controller.request('read'); await controller.measure(); assert.equal(calls, 2);
});

test('closing retention during a write does not start a follow-up scan or notify retired consumers', async () => {
  const pending = deferred<NativeRetentionResult>(); let scans = 0, notifications = 0;
  const controller = new RetentionController({read: async () => retention(), save: () => pending.promise, reset: async () => retention(),
    usage: async () => { scans++; return usage(); }}, '/captures');
  controller.subscribe(() => { notifications++; });
  await controller.request('read'); await Promise.resolve();
  const write = controller.request('save'); controller.dispose(); const before = notifications;
  controller.subscribe(() => { notifications++; }); pending.resolve(retention()); await write;
  assert.equal(scans, 1); assert.equal(notifications, before);
});

test('plain JavaScript callers cannot turn unknown operations into mutation intent', async () => {
  let mutations = 0;
  const l = new LocationController({read: async () => location(), change: async () => { mutations++; return location(); }, reset: async () => { mutations++; return location(); }});
  const r = new RetentionController({read: async () => retention(), save: async () => { mutations++; return retention(); }, reset: async () => { mutations++; return retention(); }, usage: async () => usage()}, '/captures');
  await l.request('read'); await r.request('read');
  const locationState = l.state;
  for (const value of ['invalid', '', null, undefined, 0, {}, 'save']) {
    await l.request(value as 'read');
  }
  for (const value of ['invalid', '', null, undefined, 0, {}, 'change']) {
    await r.request(value as 'read');
  }
  assert.equal(mutations, 0); assert.equal(l.state, locationState);
  assert.equal(r.state.result?.status, 'connected');
});
