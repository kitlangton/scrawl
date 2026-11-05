import assert from 'node:assert/strict';
import { test } from 'node:test';
import { validateCoachingView } from '@apparent/app-contracts/host';
import type {
  CoachingView, ContributionFactory, JsonValue, ProgressCodec, ProgressStore,
  SaveResult, TransactionalConfigBackend,
} from '@apparent/app-contracts/host';
import type { StreamModule } from '@apparent/app-contracts';
import {
  bindContribution, bindOwnerProgress, unavailableCoachingHost,
} from '../src/host-services.ts';

type Progress = { status: 'new' | 'completed'; stage: string; schema: 1 };
const codec: ProgressCodec<Progress> = {
  decode(value) {
    if (value === null) return { schema: 1, status: 'new', stage: 'welcome' };
    if (typeof value !== 'object' || Array.isArray(value)) throw new TypeError('bad progress');
    const status = value.status === 'completed' ? 'completed' : 'new';
    const aliases: Record<string, string> = { tutorial: 'welcome', done: 'complete' };
    const stage = typeof value.stage === 'string' ? aliases[value.stage] ?? value.stage : 'welcome';
    return { schema: 1, status, stage };
  },
  encode(value) { return { schema: value.schema, status: value.status, stage: value.stage }; },
};

function memoryBackend(seed: Record<string, JsonValue | null> = {}) {
  const values = new Map(Object.entries(seed));
  const revisions = new Map([...values.keys()].map(owner => [owner, 1]));
  const calls: string[] = [];
  const backend: TransactionalConfigBackend = {
    async read(ownerId) {
      calls.push(`read:${ownerId}`);
      return { revision: revisions.get(ownerId) ?? 0, value: structuredClone(values.get(ownerId) ?? null) };
    },
    async compareAndSwap(ownerId, expected, value) {
      calls.push(`cas:${ownerId}`);
      const currentRevision = revisions.get(ownerId) ?? 0;
      if (expected !== currentRevision) {
        return { status: 'conflict', revision: currentRevision, current: structuredClone(values.get(ownerId) ?? null) };
      }
      const nextRevision = currentRevision + 1;
      values.set(ownerId, structuredClone(value));
      revisions.set(ownerId, nextRevision);
      return { status: 'saved', revision: nextRevision };
    },
  };
  return { backend, values, calls };
}

test('progress adapter matches Onboarding API and delegates atomic same-revision CAS', async () => {
  const { backend, values, calls } = memoryBackend({
    alpha: { schema: 1, status: 'new', stage: 'tutorial' },
    beta: { preserve: true },
  });
  const alpha = bindOwnerProgress(backend, 'alpha', codec);
  const read = await alpha.read();
  assert.deepEqual(read, { revision: 1, value: { schema: 1, status: 'new', stage: 'welcome' } });
  const next = { schema: 1 as const, status: 'completed' as const, stage: 'complete' };
  const [left, right] = await Promise.all([
    alpha.compareAndSwap(read.revision, next),
    alpha.compareAndSwap(read.revision, { ...next, stage: 'stale' }),
  ]);
  assert.deepEqual([left.status, right.status].sort(), ['conflict', 'saved']);
  const loser = [left, right].find((result): result is Extract<typeof result, { status: 'conflict' }> => result.status === 'conflict');
  assert.ok(loser);
  assert.equal(loser.revision, 2);
  assert.equal(loser.current.status, 'completed');
  assert.deepEqual(values.get('beta'), { preserve: true });
  assert.deepEqual(calls, ['read:alpha', 'cas:alpha', 'cas:alpha']);
  const onboardingCompatible: ProgressStore<Progress> = alpha;
  const result: SaveResult<Progress> = await onboardingCompatible.compareAndSwap(2, next);
  assert.equal(result.status, 'saved');
});

test('owner namespace stays bound and caller/backend mutations are isolated', async () => {
  const { backend, values, calls } = memoryBackend({ alpha: { schema: 1, status: 'new', stage: 'welcome' } });
  const store = bindOwnerProgress(backend, 'alpha', codec);
  const read = await store.read();
  read.value.stage = 'mutated';
  assert.equal((values.get('alpha') as any).stage, 'welcome');
  const submitted = { schema: 1 as const, status: 'completed' as const, stage: 'complete' };
  await store.compareAndSwap(read.revision, submitted);
  submitted.stage = 'caller mutation';
  assert.equal((values.get('alpha') as any).stage, 'complete');
  assert.deepEqual(calls, ['read:alpha', 'cas:alpha']);
  assert.throws(() => bindOwnerProgress(backend, '../beta', codec), /owner/i);
});

test('pre-validates JSON before serialization and bounds depth and size', async () => {
  let writes = 0;
  const backend: TransactionalConfigBackend = {
    async read() { return { revision: 'r1', value: null }; },
    async compareAndSwap() { writes++; return { status: 'saved', revision: 'r2' }; },
  };
  const permissiveCodec: ProgressCodec<any> = { decode: value => value, encode: value => value };
  const store = bindOwnerProgress(backend, 'alpha', permissiveCodec);
  const accessor = Object.defineProperty({}, 'x', { enumerable: true, get() { throw new Error('getter ran'); } });
  const customToJson = { toJSON() { return { safe: true }; } };
  let tooDeep: any = 1;
  for (let index = 0; index < 40; index++) tooDeep = { a: tooDeep };
  for (const invalid of [NaN, undefined, () => 1, accessor, customToJson, tooDeep, { text: 'x'.repeat(70_000) }]) {
    await assert.rejects(store.compareAndSwap('r1', invalid));
  }
  assert.equal(writes, 0);
});

test('read failure rejects; uncertain and malformed writes never claim saved', async () => {
  const readFailure: TransactionalConfigBackend = {
    async read() { throw new Error('read failed'); },
    async compareAndSwap() { throw new Error('write timed out'); },
  };
  const store = bindOwnerProgress(readFailure, 'alpha', codec);
  await assert.rejects(store.read(), /read failed/);
  assert.deepEqual(await store.compareAndSwap('r1', codec.decode(null)), { status: 'unknown', message: 'write timed out' });
  const malformed = bindOwnerProgress({
    async read() { return { revision: 1, value: null }; },
    async compareAndSwap() { return { status: 'saved', revision: Number.NaN } as any; },
  }, 'alpha', codec);
  assert.equal((await malformed.compareAndSwap(1, codec.decode(null))).status, 'unknown');
});

test('compiled-in contribution identity and activation/disposal lifecycle are explicit', async () => {
  const Component = (() => null) as unknown as StreamModule['Screen'];
  const module = { id: 'alpha', Screen: Component, Settings: Component } satisfies StreamModule;
  let activations = 0; let disposals = 0;
  const factory: ContributionFactory<{ token: string }> = {
    ownerId: 'alpha',
    async create(dependencies) {
      assert.equal(dependencies.token, 'main-bound');
      return { module, activation: {
        async activate(context) { assert.equal(context.reason, 'firstLaunch'); activations++; },
        async dispose() { disposals++; },
      } };
    },
  };
  await assert.rejects(bindContribution('beta', factory, { token: 'main-bound' }), /owner mismatch/);
  const bound = await bindContribution('alpha', factory, { token: 'main-bound' });
  await bound.activation.activate({ reason: 'firstLaunch' });
  await bound.activation.dispose();
  await bound.activation.dispose();
  assert.equal(activations, 1);
  assert.equal(disposals, 1);
  await assert.rejects(bound.activation.activate({ reason: 'resume' }), /disposed/);
});

test('unsupported coaching returns unavailable and accepts only declared view shape', async () => {
  const view: CoachingView<'welcome'> = { id: 'welcome', placement: 'inline', title: 'Hello', body: 'Start', actions: [] };
  assert.deepEqual(validateCoachingView(view, ['welcome']), view);
  assert.throws(() => validateCoachingView(view, ['settings']), /not allowed/);
  assert.throws(() => validateCoachingView({ ...view, placement: 'window' }, ['welcome']), /placement/);
  assert.throws(() => validateCoachingView({ ...view, body: 'x'.repeat(501) }, ['welcome']), /limits/);
  assert.deepEqual(await unavailableCoachingHost.show(view), { status: 'unavailable', capability: 'coaching.show' });
  assert.deepEqual(await unavailableCoachingHost.update(view), { status: 'unavailable', capability: 'coaching.update' });
  assert.deepEqual(await unavailableCoachingHost.hide('welcome'), { status: 'unavailable', capability: 'coaching.hide' });
});

test('disposal waits for activation and failed cleanup can be retried', async () => {
  let release!: () => void;
  const wait = new Promise<void>(resolve => { release = resolve; });
  const events: string[] = [];
  let attempts = 0;
  const Component = () => null;
  const bound = await bindContribution('alpha', { ownerId: 'alpha', async create() {
    return { module: { id: 'alpha', Screen: Component, Settings: Component }, activation: {
      async activate() { events.push('activate'); await wait; events.push('activated'); },
      async dispose() { events.push('dispose'); if (++attempts === 1) throw new Error('cleanup failed'); },
    } };
  } }, {});
  const activation = bound.activation.activate({ reason: 'resume' });
  await Promise.resolve();
  const disposal = bound.activation.dispose();
  const concurrent = bound.activation.dispose();
  assert.deepEqual(events, ['activate']);
  const firstResult = assert.rejects(disposal, /cleanup failed/);
  const concurrentResult = assert.rejects(concurrent, /cleanup failed/);
  release(); await activation; await firstResult; await concurrentResult;
  assert.deepEqual(events, ['activate', 'activated', 'dispose']);
  await assert.rejects(bound.activation.activate({ reason: 'resume' }), /disposed/);
  await bound.activation.dispose(); await bound.activation.dispose();
  assert.equal(attempts, 2);
});
