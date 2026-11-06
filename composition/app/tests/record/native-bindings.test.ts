import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { MAX_UTF8_TEXT_BYTES } from '@apparent/native-host/clipboard';
import type { RecordLocalClipboardIdentity } from '@apparent/record/host/native';
import type { RecordNativeBindingOptions, RecordNativeBindings } from '../../src/record/native-bindings.ts';

type Calls = {
  hostFactories: number;
  penFactories: number;
  clipboardFactories: number;
  configOptions: unknown[];
  configCreates: number;
  configCloses: number;
  configReads: number;
  configWrites: number;
  penConfigValue: { present: boolean; value: unknown };
  penReads: number;
  penApplies: Array<{ rgb: unknown; expectedPenColor: unknown; expected: unknown }>;
  clipboardWrites: Array<{ text: string; observed: unknown }>;
};

const hostSnapshot = {
  api: 1 as const,
  processEpoch: 'epoch-record-binding',
  revision: 4,
  scope: 'running-desktop-process' as const,
  freshness: 'cached-gui-observation' as const,
  permissions: [{ id: 'screen-recording', status: 'granted' as const, usableNow: true,
    relaunchNeeded: false, actions: [] as Array<'request-permission' | 'open-system-settings'> }],
  shortcuts: {
    record: { configured: null, effective: null, registration: 'unknown' as const, canValidate: false, canSave: false },
    paste: { configured: null, effective: null, registration: 'unknown' as const, canValidate: false, canSave: false },
  },
};

async function loadBinding(clipboardResult: unknown = { status: 'outcome-unknown' }) {
  const calls: Calls = {
    hostFactories: 0, penFactories: 0, clipboardFactories: 0, configOptions: [],
    configCreates: 0, configCloses: 0, configReads: 0, configWrites: 0,
    penConfigValue: { present: false, value: null }, penReads: 0, penApplies: [], clipboardWrites: [],
  };
  let actualRgb: [number, number, number] | null = null;
  const mocks = {
    calls,
    hostFactory() {
      calls.hostFactories++;
      return {
        async readSnapshot() { return structuredClone(hostSnapshot); },
        async requestPermission() { throw new Error('must not mutate during binding'); },
        async openSystemSettings() { throw new Error('must not mutate during binding'); },
        async validateRecordShortcut() { throw new Error('must not mutate during binding'); },
        async saveRecordShortcut() { throw new Error('must not mutate during binding'); },
      };
    },
    penFactory() {
      calls.penFactories++;
      return {
        async readStrokeColor() {
          calls.penReads++;
          return { api: 1, processEpoch: hostSnapshot.processEpoch, scope: 'running-desktop-process', rgb: actualRgb };
        },
        async applyStrokeColor(rgb: unknown, expectedPenColor: unknown, expected: unknown) {
          calls.penApplies.push({ rgb, expectedPenColor, expected });
          actualRgb = rgb as [number, number, number] | null;
          return { status: 'applied', rgb: actualRgb };
        },
      };
    },
    clipboardFactory() {
      calls.clipboardFactories++;
      return {
        async writeText(text: string, observed: unknown) {
          calls.clipboardWrites.push({ text, observed });
          return clipboardResult;
        },
      };
    },
    async configFactory(options: unknown) {
      calls.configCreates++;
      calls.configOptions.push(options);
      return {
        async read() { calls.configReads++; return structuredClone(calls.penConfigValue); },
        async compareAndSwap(expected: { present: boolean; value: unknown }, next: { present: boolean; value: unknown }) {
          calls.configWrites++;
          if (expected.present !== calls.penConfigValue.present || expected.value !== calls.penConfigValue.value) {
            return { status: 'conflict', current: structuredClone(calls.penConfigValue) };
          }
          calls.penConfigValue = structuredClone(next);
          return { status: 'saved' };
        },
        async close() { calls.configCloses++; },
      };
    },
  };
  (globalThis as typeof globalThis & { __recordNativeBindingMocks?: typeof mocks }).__recordNativeBindingMocks = mocks;
  const entry = fileURLToPath(new URL('../../src/record/native-bindings.ts', import.meta.url));
  const output = await build({ entryPoints: [entry], bundle: true, platform: 'node', format: 'esm', write: false,
    plugins: [{ name: 'controlled-native-producers', setup(builder) {
      builder.onResolve({ filter: /^@apparent\/(native-host(?:\/pen|\/clipboard)?|config-host)$/ }, args => ({ path: args.path, namespace: 'native-producer-test' }));
      builder.onLoad({ filter: /.*/, namespace: 'native-producer-test' }, args => {
        const method = args.path === '@apparent/native-host' ? 'hostFactory'
          : args.path === '@apparent/native-host/pen' ? 'penFactory'
            : args.path === '@apparent/native-host/clipboard' ? 'clipboardFactory' : 'configFactory';
        const contents = method === 'configFactory'
          ? `export async function createRecordPenColorConfig(options) { return globalThis.__recordNativeBindingMocks.configFactory(options); }`
          : method === 'clipboardFactory'
            ? `export const MAX_UTF8_TEXT_BYTES = 21800; export function createNativeClipboardPort() { return globalThis.__recordNativeBindingMocks.clipboardFactory(); }`
            : `export function ${method === 'hostFactory' ? 'createNativeHostPort' : 'createNativePenPort'}() { return globalThis.__recordNativeBindingMocks.${method}(); }`;
        return { contents, loader: 'js' };
      });
    } }] });
  const module = await import(`data:text/javascript;base64,${Buffer.from(output.outputFiles[0].text).toString('base64')}#${Math.random()}`);
  return { createRecordNativeBindings: module.createRecordNativeBindings as (options: RecordNativeBindingOptions) => Promise<RecordNativeBindings>, calls };
}

const identity = { captureId: 'capture-1', artifactId: 'artifact-1' };
const handoff = {
  savedBundle: { status: 'saved' as const, bundlePath: '/synthetic/capture.apparent' },
  processing: {
    durationMs: 1000,
    bundlePath: '/synthetic/capture.apparent',
    transcript: { text: '', segments: [] },
    frames: [],
  },
  requestVideoProof: false,
};

test('binds real Record adapters to public producer facades and closes only its config backend', async () => {
  const { createRecordNativeBindings, calls } = await loadBinding();
  const requiredPermissionIds = ['screen-recording'];
  const config = { executable: '/verified/config-host', executableSha256: 'a'.repeat(64), configPath: '/synthetic/config.json' };
  const resolverCalls: unknown[] = [];
  const resolveHandoff = async (requested: Readonly<typeof identity>) => {
    resolverCalls.push(requested);
    return { ...identity, handoff };
  };
  const options: RecordNativeBindingOptions = { requiredPermissionIds, resolveHandoff, config };
  const bindings = await createRecordNativeBindings(options);

  requiredPermissionIds[0] = 'microphone';
  config.configPath = '/mutated/config.json';
  options.resolveHandoff = async () => null;
  assert.deepEqual(calls.configOptions, [{ executable: '/verified/config-host', executableSha256: 'a'.repeat(64), configPath: '/synthetic/config.json' }]);
  assert.deepEqual([calls.hostFactories, calls.penFactories, calls.clipboardFactories, calls.configCreates], [1, 1, 1, 1]);

  const facts = await bindings.native.readFacts();
  assert.equal(facts.readiness.find(item => item.id === 'screen-recording')?.required, true);
  assert.deepEqual(facts.readiness.map(item => item.id), ['screen-recording']);
  assert.deepEqual(await bindings.penConfig.read(), { persisted: null, effective: null });
  assert.equal(calls.configReads, 1);
  assert.equal(calls.penReads, 1);
  assert.equal(calls.configWrites, 0, 'binding/readback setup does not change the authoritative config');
  assert.equal(calls.penApplies.length, 0, 'binding/readback setup does not change runtime telemetry');

  const result = await bindings.clipboard.copyLocal(identity);
  assert.equal(result.status, 'outcome-unknown');
  assert.equal(resolverCalls.length, 1);
  assert.deepEqual(resolverCalls[0], identity);
  assert.equal(Object.isFrozen(resolverCalls[0]), true);
  assert.equal(calls.clipboardWrites.length, 1);
  assert.deepEqual(calls.clipboardWrites[0].observed, { processEpoch: hostSnapshot.processEpoch, revision: hostSnapshot.revision });
  assert.equal(calls.clipboardWrites[0].text.endsWith('\n'), true);
  assert.equal(calls.configCloses, 0);

  assert.deepEqual(await bindings.penConfig.writeApplyAndReadBack('coral'), { status: 'saved', effective: 'coral' });
  assert.deepEqual(calls.penApplies, [{
    rgb: [217, 76, 68], expectedPenColor: { present: true, value: 'coral' },
    expected: { processEpoch: hostSnapshot.processEpoch, revision: hostSnapshot.revision },
  }]);
  assert.deepEqual(calls.penConfigValue, { present: true, value: 'coral' });

  await Promise.all([bindings.close(), bindings.close()]);
  assert.equal(calls.configCloses, 1);
  assert.equal(calls.configReads, 3, 'only the explicit pen read and apply/readback touch config');
  assert.equal(calls.configWrites, 1);
  assert.equal(calls.clipboardWrites.length, 1, 'close does not mutate or retry clipboard work');
});

test('public Record clipboard adapter enforces the producer byte bound before any clipboard effect', async () => {
  const { createRecordNativeBindings, calls } = await loadBinding();
  const bindings = await createRecordNativeBindings({
    requiredPermissionIds: [],
    resolveHandoff: async (requested: Readonly<RecordLocalClipboardIdentity>) => ({ ...requested, handoff: {
      ...handoff,
      processing: { ...handoff.processing, note: 'x'.repeat(22_000) },
    } }),
    config: { executable: '/verified/config-host', executableSha256: 'b'.repeat(64), configPath: '/synthetic/config.json' },
  });
  const result = await bindings.clipboard.copyLocal(identity);
  assert.equal(result.status, 'refused');
  assert.equal(result.reason, 'too-large');
  assert.equal(calls.clipboardWrites.length, 0);
  assert.equal(calls.hostFactories, 1);
  await bindings.close();
});

test('uses the public byte limit and preserves an explicit native unavailable refusal', async () => {
  assert.equal(MAX_UTF8_TEXT_BYTES, 21_800);
  const { createRecordNativeBindings, calls } = await loadBinding({ status: 'refused', reason: 'unavailable' });
  const bindings = await createRecordNativeBindings({
    requiredPermissionIds: [],
    resolveHandoff: async requested => ({ ...requested, handoff }),
    config: { executable: '/verified/config-host', executableSha256: 'd'.repeat(64), configPath: '/synthetic/config.json' },
  });
  assert.deepEqual(await bindings.clipboard.copyLocal(identity), {
    status: 'refused', ...identity, reason: 'unavailable',
  });
  assert.equal(calls.clipboardWrites.length, 1);
  await bindings.close();
});

test('adapter setup failure closes the config backend before rejecting construction', async () => {
  const { createRecordNativeBindings, calls } = await loadBinding();
  await assert.rejects(createRecordNativeBindings({
    requiredPermissionIds: ['not-a-permission'],
    resolveHandoff: async () => null,
    config: { executable: '/verified/config-host', executableSha256: 'c'.repeat(64), configPath: '/synthetic/config.json' },
  }), /supported native permissions/);
  assert.equal(calls.configCreates, 1);
  assert.equal(calls.configCloses, 1);
});
