import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import type { CapabilitySnapshot, PenColor, RecordCommand, RecordCaptureFacts, RecordNativeFacts, RecordPenConfigPort } from "../src/capabilities/index.ts";
import { createRecordCapabilityProvider } from "../src/backend/capabilities/provider.ts";
import { createRecordHostRuntime } from "../dist/backend/host/runtime.js";
import type { EngineSdkPort, PreparedStart } from "../src/backend/engine/sdk-adapter.ts";
import type { RecordHostRuntime } from "../src/backend/host/runtime.ts";

function runtimeFixture() {
  let state: RecordHostRuntime["state"] = { phase: "idle", readiness: { ready: false, reason: "not queried" } };
  const listeners = new Set<(value: typeof state) => void>();
  let disposed = false;
  const completionEligible = new Set<string>();
  const runtime = {
    get state() { return state; },
    startup: { state: "ready", knownRequestIds: [] } as const,
    subscribe(listener: (value: typeof state) => void) { listeners.add(listener); return () => listeners.delete(listener); },
    async dispatch(command: { type: string; requestId?: string }) {
      if (command.type === "refresh-readiness") state = { phase: "idle", readiness: { ready: true } };
      if (command.type === "start" && command.requestId) state = { phase: "recording", captureId: command.requestId, readiness: { ready: true } };
      if (command.type === "stop" && "captureId" in state) { completionEligible.add(state.captureId); state = { phase: "ready", captureId: state.captureId, readiness: state.readiness, bundlePath: "/private/fixture.apparent" }; }
      for (const listener of [...listeners]) listener(state);
      return state;
    },
    reconcileStartup: async () => ({ state: "ready", knownRequestIds: [] }),
    completionEligibleCaptureIds: () => [...completionEligible],
    unresolvedOwnership: () => ({ requestIds: [], storageUnavailable: false, reconciliation: "complete" as const }),
    dispose() { disposed = true; listeners.clear(); return { requestIds: [], storageUnavailable: false, reconciliation: "complete" as const }; },
  } as unknown as RecordHostRuntime;
  return { runtime, isDisposed: () => disposed };
}

const nativeFacts = (): RecordNativeFacts => ({
  version: 1, providerEpoch: "native-epoch-1", revision: 1,
  readiness: [{ id: "screen-permission", required: true, status: "granted", usableNow: true, relaunchNeeded: false, allowedRepairActions: [] }],
  shortcut: { effective: "Ctrl+Shift+R", label: "Ctrl Shift R", registration: "registered", externallyManaged: false, draft: null },
});
const outputFacts = (): RecordCaptureFacts => ({ version: 1, providerEpoch: "output-epoch-1", revision: 1, captures: [], nextShortcutAdmission: { state: "unavailable" } });
const command = (snapshot: CapabilitySnapshot, type: "refresh" | "start-capture", requestId: string, delivery?: "local-only" | "host-default"): RecordCommand => {
  const base = { requestId, expected: { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision } };
  return type === "start-capture" ? { ...base, type, delivery: delivery ?? "host-default" } : { ...base, type };
};

test("provider publishes explicit unavailable host/config facts and refuses absent producers", async () => {
  const { runtime } = runtimeFixture();
  const client = createRecordCapabilityProvider({ runtime, providerEpoch: "provider-1" });
  const snapshot = client.current();
  assert.equal(snapshot.version, 2);
  assert.equal(snapshot.readiness.requirements.find(item => item.id === "native-host")?.status, "unavailable");
  assert.equal(snapshot.shortcut.registration, "unknown");
  assert.deepEqual(snapshot.nextShortcutAdmission, { state: "unavailable" });
  assert.equal(snapshot.pen.effective, null);
  assert.equal((await client.execute({ ...command(snapshot, "start-capture", "start-no-runtime", "local-only") })).status, "refused");
  client.detach();
});

test("stale commands are refused, snapshots are isolated, and detach only releases observation", async () => {
  const { runtime, isDisposed } = runtimeFixture();
  const native = { readFacts: async () => nativeFacts() };
  const client = createRecordCapabilityProvider({ runtime, native, providerEpoch: "provider-2" });
  await new Promise(resolve => setImmediate(resolve));
  const seen: CapabilitySnapshot[] = [];
  client.observe(value => { value.captures.push({ captureId: "observer-edit", delivery: "local-only", phase: "saved", outcomes: [] }); });
  client.observe(value => seen.push(value));
  const current = client.current();
  const stale = await client.execute({ ...command(current, "refresh", "refresh-1"), expected: { ...current, revision: current.revision - 1 } });
  assert.equal(stale.status, "refused");
  assert.equal(seen.at(-1)?.captures.length, 0);
  const refreshed = await client.execute(command(current, "refresh", "refresh-2"));
  assert.equal(refreshed.status, "accepted");
  client.detach();
  assert.equal(isDisposed(), true);
  assert.equal(client.current().connection, "disconnected");
});

test("provider enforces observed repair permission and external shortcut ownership before dispatch", async () => {
  const { runtime } = runtimeFixture();
  const facts = nativeFacts();
  facts.readiness[0].allowedRepairActions = ["request-permission"];
  facts.shortcut = { effective: null, label: null, registration: "externally-managed", externallyManaged: true, draft: null };
  const dispatched: string[] = [];
  const client = createRecordCapabilityProvider({
    runtime,
    native: { readFacts: async () => structuredClone(facts), dispatch: async action => { dispatched.push(action.type); return "accepted"; } },
    providerEpoch: "provider-repair",
  });
  await new Promise(resolve => setImmediate(resolve));
  const before = client.current();
  const denied = await client.execute({ type: "shortcut-begin-edit", requestId: "shortcut-edit", expected: { providerEpoch: before.providerEpoch, revision: before.revision } });
  assert.equal(denied.status, "refused");
  const permission = await client.execute({ type: "request-permission", requirementId: "screen-permission", requestId: "permission-request", expected: { providerEpoch: client.current().providerEpoch, revision: client.current().revision } });
  assert.equal(permission.status, "accepted");
  assert.deepEqual(dispatched, ["request-permission"]);
  client.detach();
});

test("native fact epochs and revisions reject stale frames and retired epochs", async () => {
  const { runtime } = runtimeFixture();
  let facts = nativeFacts();
  const listeners = new Set<() => void>();
  const client = createRecordCapabilityProvider({
    runtime,
    native: { readFacts: async () => structuredClone(facts), observe: listener => { listeners.add(listener); return () => listeners.delete(listener); } },
    providerEpoch: "provider-source-epochs",
  });
  await new Promise(resolve => setImmediate(resolve));
  facts = { ...facts, providerEpoch: "native-epoch-2", revision: 1, shortcut: { ...facts.shortcut, effective: "Alt+R" } };
  for (const listener of listeners) listener();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(client.current().shortcut.effective, "Alt+R");
  facts = { ...facts, providerEpoch: "native-epoch-1", revision: 99, shortcut: { ...facts.shortcut, effective: "stale" } };
  for (const listener of listeners) listener();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(client.current().shortcut.effective, null, "retired producer epochs must not restore old bindings");
  client.detach();
});

test("Engine lifecycle is composed, local-only never reaches it, and v1 saved waits for Record assembly", async () => {
  const { runtime } = runtimeFixture();
  const client = createRecordCapabilityProvider({ runtime, native: { readFacts: async () => nativeFacts() }, providerEpoch: "provider-3" });
  await new Promise(resolve => setImmediate(resolve));
  const before = client.current();
  assert.equal((await client.execute(command(before, "refresh", "refresh"))).status, "accepted");
  const ready = client.current();
  assert.equal(ready.readiness.requirements.find(item => item.id === "engine-screen-capture")?.usableNow, true);
  assert.equal((await client.execute(command(ready, "start-capture", "local-attempt", "local-only"))).status, "refused");
  assert.equal(client.current().captures.length, 0);
  const start = await client.execute(command(client.current(), "start-capture", "capture-1"));
  assert.equal(start.status, "accepted");
  assert.equal(client.current().captures[0].phase, "recording");
  assert.equal(client.current().captures[0].delivery, "unknown");
  const stopCommand = { type: "stop-capture" as const, captureId: "capture-1", requestId: "stop-1", expected: { providerEpoch: client.current().providerEpoch, revision: client.current().revision } };
  assert.equal((await client.execute(stopCommand)).status, "accepted");
  assert.equal(client.current().captures[0].phase, "processing");
  assert.equal(client.current().captures[0].outcomes.some(outcome => outcome.kind === "processing" && outcome.ok), true);
  assert.equal(client.current().captures[0].outcomes.some(outcome => outcome.kind === "save" && outcome.ok), false);
  assert.equal(client.current().captures[0].artifactId, undefined);
  client.detach();
});

test("only the Record output producer can publish saved state; copy dispatch is not a copy receipt", async () => {
  const { runtime } = runtimeFixture();
  const capture = { captureId: "assembled-1", delivery: "unknown" as const, artifactId: "artifact-1", phase: "saved" as const,
    outcomes: [{ kind: "save" as const, at: "2026-11-05T12:00:00Z", ok: true }] };
  let dispatched = 0;
  const client = createRecordCapabilityProvider({
    runtime,
    native: { readFacts: async () => nativeFacts(), dispatch: async () => { dispatched++; return "accepted"; } },
    captureFacts: { readFacts: async () => ({ ...outputFacts(), captures: [capture] }) },
    providerEpoch: "provider-assembled-output",
  });
  await new Promise(resolve => setImmediate(resolve));
  const snapshot = client.current();
  assert.equal(snapshot.captures[0].phase, "saved");
  const copied = await client.execute({ type: "copy-local", captureId: capture.captureId, requestId: "copy-dispatch", expected: { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision } });
  assert.equal(copied.status, "accepted");
  assert.equal(dispatched, 1);
  assert.equal(client.current().captures[0].outcomes.some(outcome => outcome.kind === "copy"), false);
  client.detach();
});

test("pen selection uses the authoritative writer and only publishes confirmed readback", async () => {
  const { runtime } = runtimeFixture();
  let persisted: PenColor = "coral";
  let effective: PenColor = "coral";
  const config: RecordPenConfigPort = {
    read: async () => ({ persisted, effective }),
    writeApplyAndReadBack: async color => { persisted = color; effective = color; return { status: "saved", effective }; },
  };
  const client = createRecordCapabilityProvider({ runtime, native: { readFacts: async () => nativeFacts() }, penConfig: config, providerEpoch: "provider-4" });
  await new Promise(resolve => setImmediate(resolve));
  const before = client.current();
  const result = await client.execute({ type: "select-pen", value: "blue", requestId: "pen-1", expected: { providerEpoch: before.providerEpoch, revision: before.revision } });
  assert.equal(result.status, "applied");
  assert.equal(result.snapshot.pen.effective, "blue");
  assert.equal(persisted, "blue");
  assert.equal(effective, "blue");
  client.detach();
});

test("pen effective value stays confirmed when the authoritative writer cannot confirm apply", async () => {
  const { runtime } = runtimeFixture();
  const config: RecordPenConfigPort = {
    read: async () => ({ persisted: "coral", effective: "coral" }),
    writeApplyAndReadBack: async () => ({ status: "unknown" }),
  };
  const client = createRecordCapabilityProvider({ runtime, penConfig: config, providerEpoch: "provider-pen-unknown" });
  await new Promise(resolve => setImmediate(resolve));
  const before = client.current();
  const result = await client.execute({ type: "select-pen", value: "blue", requestId: "pen-write-unknown", expected: { providerEpoch: before.providerEpoch, revision: before.revision } });
  assert.equal(result.status, "outcome-unknown");
  assert.equal(result.snapshot.pen.effective, "coral");
  assert.match(result.snapshot.pen.error ?? "", /unknown/u);
  client.detach();
});

test("provider composes the real host runtime and authoritative pen config writer", async t => {
  const root = await mkdtemp(join(tmpdir(), "record-capability-provider-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const token = "a".repeat(64);
  const phases = new Map<string, { captureId: string; state: "recording" | "saved"; bundlePath?: string }>();
  let sequence = 0;
  const sdk: EngineSdkPort = {
    clientId: "b".repeat(32),
    handshake: async () => ({ implementation: "native", capabilities: { screenCapture: true }, readiness: "ready" }),
    prepareStart: (bundleId, bundlePath): PreparedStart => {
      const captureId = (++sequence).toString(16).padStart(32, "0");
      return { protocolVersion: 1, clientId: "b".repeat(32), rpcId: "c".repeat(32), method: "start", captureId, ownerToken: token, bundleId, bundlePath, options: { screen: true } };
    },
    start: async request => {
      const receipt = { captureId: request.captureId, state: "recording" as const, bundlePath: request.bundlePath };
      phases.set(request.captureId, receipt);
      return { receipt, captureId: request.captureId, ownerToken: token };
    },
    control: async (method, captureId, ownerToken) => {
      assert.equal(ownerToken, token);
      const current = phases.get(captureId);
      if (!current) return { captureId, state: "rejected", error: { code: "missing", message: "missing" } };
      if (method === "stop") {
        const saved = { captureId, state: "saved" as const, bundlePath: current.bundlePath };
        phases.set(captureId, saved);
        return { ...saved, artifactProvenance: "native" as const };
      }
      return current;
    },
    close: async () => { throw new Error("provider must not close the caller-owned SDK"); },
  };
  const roots = { mappingRoot: join(root, "mapping"), bundleRoot: join(root, "bundles") };
  const runtime = await createRecordHostRuntime(sdk, roots);
  let stored: PenColor = "coral";
  let effective: PenColor = "coral";
  const config: RecordPenConfigPort = {
    read: async () => ({ persisted: stored, effective }),
    writeApplyAndReadBack: async color => { stored = color; effective = color; return { status: "saved", effective }; },
  };
  const client = createRecordCapabilityProvider({ runtime, native: { readFacts: async () => nativeFacts() }, captureFacts: { readFacts: async () => outputFacts() }, penConfig: config, providerEpoch: "runtime-provider" });
  await new Promise(resolve => setImmediate(resolve));
  const before = client.current();
  const pen = await client.execute({ type: "select-pen", value: "blue", requestId: "pen-runtime", expected: { providerEpoch: before.providerEpoch, revision: before.revision } });
  assert.equal(pen.status, "applied");
  assert.equal(stored, "blue");
  const ready = await client.execute(command(client.current(), "refresh", "refresh-runtime"));
  assert.equal(ready.status, "accepted");
  const start = await client.execute(command(client.current(), "start-capture", "runtime-capture"));
  assert.equal(start.status, "accepted");
  assert.equal(client.current().captures[0].phase, "recording");
  const stop = { type: "stop-capture" as const, captureId: "runtime-capture", requestId: "stop-runtime", expected: { providerEpoch: client.current().providerEpoch, revision: client.current().revision } };
  assert.equal((await client.execute(stop)).status, "accepted");
  assert.equal(client.current().captures[0].phase, "processing");
  assert.equal(client.current().captures[0].outcomes.some(outcome => outcome.kind === "save" && outcome.ok), false);
  client.detach();
});


test("superseded reads cannot retire current native or output producer epochs", async () => {
  const { runtime } = runtimeFixture();
  let resolveNative!: (facts: RecordNativeFacts) => void;
  let resolveOutput!: (facts: RecordCaptureFacts) => void;
  let nativeReads = 0;
  let outputReads = 0;
  let notify!: () => void;
  const newerNative = { ...nativeFacts(), providerEpoch: "native-new", shortcut: { ...nativeFacts().shortcut, effective: "Alt+R" } };
  const newerOutput = { ...outputFacts(), providerEpoch: "output-new", nextShortcutAdmission: { state: "idle" as const } };
  const client = createRecordCapabilityProvider({
    runtime, providerEpoch: "overlapping-reads",
    native: {
      readFacts: () => ++nativeReads === 1 ? new Promise(resolve => { resolveNative = resolve; }) : Promise.resolve(newerNative),
      observe: listener => { notify = listener; return () => undefined; },
    },
    captureFacts: { readFacts: () => ++outputReads === 1 ? new Promise(resolve => { resolveOutput = resolve; }) : Promise.resolve(newerOutput) },
  });
  notify();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(client.current().shortcut.effective, "Alt+R");
  assert.equal(client.current().nextShortcutAdmission.state, "idle");
  resolveNative(nativeFacts());
  resolveOutput(outputFacts());
  await new Promise(resolve => setImmediate(resolve));
  notify();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(client.current().shortcut.effective, "Alt+R");
  assert.equal(client.current().nextShortcutAdmission.state, "idle");
  client.detach();
});

test("cached saved output cannot override a live capture or admit copy and open", async () => {
  const { runtime } = runtimeFixture();
  await runtime.dispatch({ type: "start", requestId: "overlapping-capture" });
  let dispatched = 0;
  const client = createRecordCapabilityProvider({
    runtime, providerEpoch: "live-lifecycle",
    native: { readFacts: async () => nativeFacts(), dispatch: async () => { dispatched++; return "accepted"; } },
    captureFacts: { readFacts: async () => ({ ...outputFacts(), captures: [{
      captureId: "overlapping-capture", delivery: "unknown", phase: "saved", artifactId: "prior-artifact",
      outcomes: [{ kind: "save", at: "2026-11-05T12:00:00Z", ok: true }],
    }] }) },
  });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(client.current().captures[0].phase, "recording");
  for (const type of ["copy-local", "open-capture"] as const) {
    const snapshot = client.current();
    const result = await client.execute({ type, captureId: "overlapping-capture", requestId: type,
      expected: { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision } });
    assert.equal(result.status, "refused");
  }
  assert.equal(dispatched, 0);
  await runtime.dispatch({ type: "stop" });
  assert.equal(client.current().captures[0].phase, "saved", "assembled output may complete stopped materials");
  client.detach();
});
