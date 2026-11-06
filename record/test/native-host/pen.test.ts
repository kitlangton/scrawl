import assert from "node:assert/strict";
import test from "node:test";
import { createRecordPenConfigPort } from "../../dist/backend/host/native/pen.js";
import { PEN_COLOR_RGB } from "../../dist/capabilities/helpers.js";
import { createRecordCapabilityProvider } from "../../dist/backend/capabilities/provider.js";
import type { PenColor } from "../../src/capabilities/index.ts";

type Legacy = { present: boolean; value: unknown };
const epoch = "desktop-epoch-1";
function fixture(options: {
  config?: Legacy;
  rgb?: [number, number, number] | null;
  cas?: "saved" | "conflict" | "unknown";
  apply?: "applied" | "conflict" | "unavailable" | "unknown";
  failGetter?: boolean;
  getterEpoch?: string;
  afterCas?: (change: (value: Legacy) => void) => void;
  afterApply?: (change: (value: Legacy) => void) => void;
  snapshotEpoch?: string;
  snapshotRevision?: number;
} = {}) {
  let config: Legacy = options.config ?? { present: false, value: null };
  let rgb = options.rgb ?? null;
  let casCalls = 0, applyCalls = 0, reads = 0, closeCalls = 0;
  let revision = 1;
  const configPort = {
    async read() { reads++; return structuredClone(config); },
    async close() { closeCalls++; },
    async compareAndSwap(expected: Legacy, next: Legacy) {
      casCalls++;
      if (options.cas === "conflict" || !same(config, expected)) return { status: "conflict" as const, current: structuredClone(config) };
      if (options.cas === "unknown") return { status: "unknown" as const, message: "lost" };
      config = structuredClone(next);
      options.afterCas?.(value => { config = structuredClone(value); });
      return { status: "saved" as const };
    },
  };
  const nativePen = {
    async readStrokeColor() {
      if (options.failGetter && reads > 1) throw new Error("lost getter");
      return { api: 1 as const, processEpoch: options.getterEpoch ?? epoch, scope: "running-desktop-process" as const, rgb: structuredClone(rgb) };
    },
    async applyStrokeColor(next: [number, number, number] | null, expected: Legacy, state: { processEpoch: string; revision: number }) {
      applyCalls++;
      await new Promise<void>(resolve => setTimeout(resolve, 1));
      if (options.apply === "conflict" || state.processEpoch !== epoch || state.revision !== revision || !same(config, expected)) {
        return { status: "conflict" as const, current: structuredClone(config) };
      }
      if (options.apply === "unavailable") return { status: "unavailable" as const };
      if (options.apply === "unknown") return { status: "outcome-unknown" as const };
      rgb = structuredClone(next);
      options.afterApply?.(value => { config = structuredClone(value); });
      revision++;
      return { status: "applied" as const, rgb: structuredClone(rgb) };
    },
  };
  const nativeHost = { async readSnapshot() { return { api: 1 as const, processEpoch: options.snapshotEpoch ?? epoch,
    revision: options.snapshotRevision ?? revision, scope: "running-desktop-process" as const }; } };
  return {
    port: createRecordPenConfigPort({ config: configPort, nativePen, nativeHost }),
    get config() { return structuredClone(config); }, get rgb() { return structuredClone(rgb); },
    get calls() { return { casCalls, applyCalls, reads }; },
    get closeCalls() { return closeCalls; },
    changeConfig(value: Legacy) { config = structuredClone(value); },
  };
}
function same(a: Legacy, b: Legacy) { return a.present === b.present && Object.is(a.value, b.value); }

test("reads persisted absence/null separately from exact named effective palette", async () => {
  for (const config of [{ present: false, value: null }, { present: true, value: null }]) {
    const f = fixture({ config, rgb: [...PEN_COLOR_RGB.coral] });
    assert.deepEqual(await f.port.read(), { persisted: null, effective: "coral" });
  }
  const black = fixture({ config: { present: true, value: "blue" }, rgb: [0, 0, 0] });
  assert.deepEqual(await black.port.read(), { persisted: "blue", effective: null });
});

test("maps only exact palette RGB values; unknown RGB never becomes a default", async () => {
  for (const color of Object.keys(PEN_COLOR_RGB) as PenColor[]) {
    const f = fixture({ config: { present: true, value: color }, rgb: [...PEN_COLOR_RGB[color]] });
    assert.deepEqual(await f.port.read(), { persisted: color, effective: color });
  }
  const unnamed = fixture({ rgb: [217, 76, 69] });
  assert.equal((await unnamed.port.read()).effective, null);
});

test("CAS conflict or unknown never dispatches native apply", async () => {
  for (const cas of ["conflict", "unknown"] as const) {
    const f = fixture({ cas });
    assert.deepEqual(await f.port.writeApplyAndReadBack("purple"), { status: "unknown" });
    assert.equal(f.calls.applyCalls, 0);
  }
});

test("guarded native admission rejects stale epoch/revision or config changed after CAS", async () => {
  const f = fixture({ apply: "conflict" });
  assert.deepEqual(await f.port.writeApplyAndReadBack("green"), { status: "unknown" });
  assert.equal(f.calls.applyCalls, 1);
  assert.deepEqual(f.rgb, null);

  for (const variant of [{ snapshotEpoch: "old-process" }, { snapshotRevision: 0 }]) {
    const stale = fixture(variant);
    assert.deepEqual(await stale.port.writeApplyAndReadBack("green"), { status: "unknown" });
    assert.deepEqual(stale.rgb, null);
  }
  // Change at the precise boundary: CAS commits green, then another writer wins before native apply.
  const raced = fixture({ afterCas: change => change({ present: true, value: "coral" }) });
  assert.deepEqual(await raced.port.writeApplyAndReadBack("green"), { status: "unknown" });
  assert.deepEqual(raced.config, { present: true, value: "coral" });
  assert.deepEqual(raced.rgb, null);
});

test("successful save requires persisted config and same-process getter readback", async () => {
  const f = fixture();
  assert.deepEqual(await f.port.writeApplyAndReadBack("blue"), { status: "saved", effective: "blue" });
  assert.deepEqual(f.config, { present: true, value: "blue" });
  assert.deepEqual(f.rgb, [...PEN_COLOR_RGB.blue]);

  const lost = fixture({ failGetter: true });
  assert.deepEqual(await lost.port.writeApplyAndReadBack("blue"), { status: "unknown" });
  assert.deepEqual(lost.config, { present: true, value: "blue" }); // No compensating rollback after uncertain apply.

  const processChanged = fixture({ getterEpoch: "new-process" });
  assert.deepEqual(await processChanged.port.writeApplyAndReadBack("blue"), { status: "unknown" });

  const configChangedAfterApply = fixture({ afterApply: change => change({ present: true, value: "coral" }) });
  assert.deepEqual(await configChangedAfterApply.port.writeApplyAndReadBack("blue"), { status: "unknown" });
  assert.deepEqual(configChangedAfterApply.config, { present: true, value: "coral" });
});

test("unsupported existing config is preserved and no competing writer is attempted", async () => {
  const f = fixture({ config: { present: true, value: { custom: "keep" } } });
  assert.deepEqual(await f.port.writeApplyAndReadBack("coral"), { status: "unknown" });
  assert.deepEqual(f.config, { present: true, value: { custom: "keep" } });
  assert.equal(f.calls.casCalls, 0);
  await assert.rejects(f.port.read(), /Unsupported stored pen color/);
});

test("parallel mutations serialize and apply only the latest CAS observation", async () => {
  const f = fixture();
  const [first, second] = await Promise.all([
    f.port.writeApplyAndReadBack("coral"), f.port.writeApplyAndReadBack("purple"),
  ]);
  assert.deepEqual(first, { status: "saved", effective: "coral" });
  assert.deepEqual(second, { status: "saved", effective: "purple" });
  assert.deepEqual(f.config, { present: true, value: "purple" });
  assert.deepEqual(f.rgb, [...PEN_COLOR_RGB.purple]);
  assert.equal(f.calls.applyCalls, 2);
  assert.equal("dispose" in f.port, false);
  assert.equal(f.closeCalls, 0);
});

test("real Record provider reports only confirmed pen facts and preserves effective color on unknown apply", async () => {
  const runtime = {
    state: { phase: "idle", readiness: { ready: true } },
    startup: { state: "ready", knownRequestIds: [] },
    subscribe: () => () => undefined,
    dispatch: async () => ({ phase: "idle", readiness: { ready: true } }),
    dispose: () => undefined,
  } as never;
  const confirmed = fixture({ config: { present: true, value: "coral" }, rgb: [...PEN_COLOR_RGB.coral] });
  const client = createRecordCapabilityProvider({ runtime, providerEpoch: "pen-provider", penConfig: confirmed.port });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(client.current().pen.effective, "coral");
  const before = client.current();
  const applied = await client.execute({ type: "select-pen", value: "blue", requestId: "pen-blue",
    expected: { providerEpoch: before.providerEpoch, revision: before.revision } });
  assert.equal(applied.status, "applied");
  assert.equal(client.current().pen.effective, "blue");
  client.detach();

  const uncertain = fixture({ config: { present: true, value: "coral" }, rgb: [...PEN_COLOR_RGB.coral], failGetter: true });
  const second = createRecordCapabilityProvider({ runtime, providerEpoch: "pen-provider-unknown", penConfig: uncertain.port });
  await new Promise(resolve => setImmediate(resolve));
  const snapshot = second.current();
  const result = await second.execute({ type: "select-pen", value: "green", requestId: "pen-green-unknown",
    expected: { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision } });
  assert.equal(result.status, "outcome-unknown");
  assert.equal(second.current().pen.effective, "coral");
  assert.match(second.current().pen.error ?? "", /unknown/);
  second.detach();
});
