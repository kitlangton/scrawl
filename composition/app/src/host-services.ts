import type {
  ActivationContext, CoachingHost, CoachingView, ContributionFactory, HostedContribution,
  HostEffectResult, JsonValue, ProgressCodec, ProgressStore, Revision, SaveResult,
  TransactionalConfigBackend,
} from '@apparent/app-contracts/host';

const MAX_JSON_BYTES = 64 * 1024;
const MAX_DEPTH = 32;
const ownerPattern = /^[a-z][a-z0-9-]{0,63}$/;

function revision(value: unknown): value is Revision {
  return (typeof value === 'string' && value.length > 0 && value.length <= 256)
    || (typeof value === 'number' && Number.isSafeInteger(value) && value >= 0);
}

function cloneJson(value: unknown): JsonValue {
  const seen = new Set<object>();
  const visit = (item: unknown, depth: number): void => {
    if (depth > MAX_DEPTH) throw new TypeError('Owner state exceeds nesting limit');
    if (item === null || typeof item === 'string' || typeof item === 'boolean') return;
    if (typeof item === 'number' && Number.isFinite(item)) return;
    if (typeof item !== 'object') throw new TypeError('Owner state must contain only JSON values');
    if (seen.has(item)) throw new TypeError('Owner state cannot be circular or aliased');
    seen.add(item);
    if (Array.isArray(item)) {
      if (Object.getPrototypeOf(item) !== Array.prototype) throw new TypeError('Owner arrays must be plain arrays');
      if (Object.getOwnPropertySymbols(item).length) throw new TypeError('Owner state cannot contain symbols');
      const keys = Reflect.ownKeys(item);
      if (keys.length !== item.length + 1 || !keys.includes('length')
        || keys.filter((key): key is string => typeof key === 'string' && key !== 'length').some((key, index) => key !== String(index))) {
        throw new TypeError('Owner arrays must be dense and have no extra properties');
      }
      for (const key of keys.filter((entry): entry is string => typeof entry === 'string' && entry !== 'length')) {
        const descriptor = Object.getOwnPropertyDescriptor(item, key);
        if (!descriptor || !descriptor.enumerable || !('value' in descriptor)) throw new TypeError('Owner state cannot contain accessors');
        visit(descriptor.value, depth + 1);
      }
      return;
    }
    if (Object.getPrototypeOf(item) !== Object.prototype) throw new TypeError('Owner state must use plain objects');
    if (Object.getOwnPropertySymbols(item).length) throw new TypeError('Owner state cannot contain symbols');
    for (const key of Reflect.ownKeys(item)) {
      if (typeof key !== 'string' || key === '__proto__' || key === 'constructor' || key === 'prototype' || key === 'toJSON') {
        throw new TypeError('Owner state contains an unsupported key');
      }
      const descriptor = Object.getOwnPropertyDescriptor(item, key);
      if (!descriptor || !descriptor.enumerable || !('value' in descriptor)) throw new TypeError('Owner state cannot contain accessors');
      visit(descriptor.value, depth + 1);
    }
  };
  visit(value, 0);
  const encoded = JSON.stringify(value);
  if (encoded === undefined || Buffer.byteLength(encoded, 'utf8') > MAX_JSON_BYTES) throw new TypeError('Owner state exceeds JSON size limit');
  return JSON.parse(encoded) as JsonValue;
}

/** Main-side owner binding; no renderer input can select the namespace. */
export function bindOwnerProgress<T>(
  backend: TransactionalConfigBackend,
  ownerId: string,
  codec: ProgressCodec<T>,
): ProgressStore<T> {
  if (!ownerPattern.test(ownerId)) throw new TypeError('Invalid compiled-in owner id');
  return {
    async read() {
      const result = await backend.read(ownerId);
      if (!result || typeof result !== 'object' || !revision(result.revision)) throw new TypeError('Backend returned an invalid revision');
      const raw = result.value === null ? null : cloneJson(result.value);
      return { revision: result.revision, value: codec.decode(raw) };
    },
    async compareAndSwap(expected, value): Promise<SaveResult<T>> {
      if (!revision(expected)) throw new TypeError('Invalid expected revision');
      const encoded = cloneJson(codec.encode(value));
      let result: Awaited<ReturnType<TransactionalConfigBackend['compareAndSwap']>>;
      try {
        result = await backend.compareAndSwap(ownerId, expected, encoded);
      } catch (error) {
        return { status: 'unknown', message: error instanceof Error ? error.message : 'Configuration outcome unknown' };
      }
      try {
        if (!result || typeof result !== 'object') return { status: 'unknown', message: 'Malformed backend write result' };
        if (result.status === 'unknown') {
          return exactKeys(result, ['status', 'message']) && typeof result.message === 'string' && result.message.length > 0
            ? { status: 'unknown', message: result.message }
            : { status: 'unknown', message: 'Malformed backend uncertainty result' };
        }
        if (!revision(result.revision)) return { status: 'unknown', message: 'Backend returned an invalid write revision' };
        if (result.status === 'saved' && exactKeys(result, ['status', 'revision'])) return { status: 'saved', revision: result.revision };
        if (result.status !== 'conflict' || !exactKeys(result, ['status', 'revision', 'current'])) {
          return { status: 'unknown', message: 'Malformed backend write result' };
        }
        const rawCurrent = result.current === null ? null : cloneJson(result.current);
        return { status: 'conflict', revision: result.revision, current: codec.decode(rawCurrent) };
      } catch (error) {
        return { status: 'unknown', message: error instanceof Error ? error.message : 'Malformed backend write result' };
      }
    },
  };
}

function exactKeys(value: object, expected: string[]): boolean {
  const keys = Reflect.ownKeys(value);
  return keys.length === expected.length && keys.every(key => typeof key === 'string' && expected.includes(key));
}

/** Binds one compiled-in factory and gives the host explicit, idempotent disposal. */
export async function bindContribution<Dependencies>(
  expectedOwnerId: string,
  factory: ContributionFactory<Dependencies>,
  dependencies: Dependencies,
): Promise<HostedContribution> {
  if (!ownerPattern.test(expectedOwnerId)) throw new TypeError('Invalid compiled-in owner id');
  if (factory.ownerId !== expectedOwnerId) throw new TypeError('Contribution owner mismatch');
  const contribution = await factory.create(dependencies);
  if (contribution.module.id !== expectedOwnerId) {
    await contribution.activation.dispose();
    throw new TypeError('Contribution module identity mismatch');
  }
  let phase: 'ready' | 'disposing' | 'disposed' | 'dispose-failed' = 'ready';
  let pending: Promise<void> = Promise.resolve();
  let disposal: Promise<void> | undefined;
  return {
    module: contribution.module,
    activation: {
      async activate(context: ActivationContext) {
        if (phase !== 'ready') throw new Error('Contribution is disposed or disposing');
        const activation = pending.then(() => contribution.activation.activate(context));
        pending = activation.catch(() => {});
        await activation;
      },
      async dispose() {
        if (disposal) return disposal;
        phase = 'disposing';
        disposal = pending.then(() => contribution.activation.dispose()).then(() => {
          phase = 'disposed';
        }, error => {
          phase = 'dispose-failed';
          disposal = undefined; // A failed cleanup can be retried, never called complete.
          throw error;
        });
        pending = disposal.catch(() => {});
        return disposal;
      },
    },
  };
}

/** Foundation has no native coaching provider; unsupported calls stay explicit. */
export const unavailableCoachingHost: CoachingHost = {
  async show(_view: CoachingView): Promise<HostEffectResult> { return { status: 'unavailable', capability: 'coaching.show' }; },
  async update(_view: CoachingView): Promise<HostEffectResult> { return { status: 'unavailable', capability: 'coaching.update' }; },
  async hide(_id: string): Promise<HostEffectResult> { return { status: 'unavailable', capability: 'coaching.hide' }; },
};
