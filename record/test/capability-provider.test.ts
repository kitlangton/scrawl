import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import type { CapabilitySnapshot, PenColor, RecordCommand, RecordCaptureFacts, RecordNativeFacts, RecordPenConfigPort } from "../src/capabilities/index.ts";
import { createRecordFixture } from "../src/capabilities/fixtures.ts";
import { createRecordCapabilityProvider } from "../src/backend/capabilities/provider.ts";
import { createRecordHostRuntime } from "../dist/backend/host/runtime.js";
import type { EngineSdkPort, PreparedStart } from "../src/backend/engine/sdk-adapter.ts";
import type { RecordHostRuntime } from "../src/backend/host/runtime.ts";

function runtimeFixture() {
  let state: RecordHostRuntime["state"] = { phase: "idle", readiness: { ready: false, reason: "not queried" } };
  const listeners = new Set<(value: typeof state) => void>();
  let disposed = false;
  const runtime = {
    get state() { return state; },
    startup: { state: "ready", knownRequestIds: [] } as const,
    subscribe(listener: (value: typeof state) => void) { listeners.add(listener); return () => listeners.delete(listener); },
    async dispatch(command: { type: string; requestId?: string }) {
      if (command.type === "refresh-readiness") state = { phase: "idle", readiness: { ready: true } };
      if (command.type === "start" && command.requestId) state = { phase: "recording", captureId: command.requestId, readiness: { ready: true } };
      if (command.type === "stop" && "captureId" in state) state = { phase: "ready", captureId: state.captureId, readiness: state.readiness, bundlePath: "/private/fixture.apparent" };
      for (const listener of [...listeners]) listener(state);
      return state;
    },
    reconcileStartup: async () => ({ state: "ready", knownRequestIds: [] }),
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
  assert.deepEqual(snapshot.shortcutTrial, { state: "unavailable" });
  assert.equal(snapshot.shortcutTrialBeginBinding, undefined);
  assert.equal(snapshot.pen.effective, null);
  assert.equal((await client.execute({ ...command(snapshot, "start-capture", "start-no-runtime", "local-only") })).status, "refused");
  assert.equal((await client.execute({ type: "begin-shortcut-trial", requestId: "trial-no-native", reservationId: "trial-1", bindingId: "binding-1", effectiveBinding: "Ctrl+Shift+R", sourceEpoch: "native-epoch", sourceRevision: 1, expiresAt: "2026-01-01T00:01:00Z", expected: { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision } })).status, "refused");
  client.detach();
});

test("shortcut trial requires matching source epoch, revision, effective binding and reservation; dispatch is not observation", async () => {
  const { runtime } = runtimeFixture();
  const facts = nativeFacts();
  facts.shortcut = { ...facts.shortcut, bindingId: "binding-1" };
  facts.shortcutTrial = { state: "idle", sourceEpoch: facts.providerEpoch, sourceRevision: facts.revision, bindingId: "binding-1", effectiveBinding: "Ctrl+Shift+R" };
  const actions: RecordCommand[] = [];
  const client = createRecordCapabilityProvider({ runtime, providerEpoch: "provider-trial", native: {
    readFacts: async () => structuredClone(facts),
    dispatch: async action => { actions.push(structuredClone(action)); return "accepted"; },
  } });
  await new Promise(resolve => setImmediate(resolve));
  const idle = client.current();
  assert.deepEqual(idle.shortcutTrial?.state, "idle");
  const begin = { type: "begin-shortcut-trial" as const, requestId: "trial-dispatch", reservationId: "reservation-1", bindingId: "binding-1",
    effectiveBinding: "Ctrl+Shift+R", sourceEpoch: facts.providerEpoch, sourceRevision: facts.revision, expiresAt: "2026-01-01T00:01:00Z",
    expected: { providerEpoch: idle.providerEpoch, revision: idle.revision } };
  const dispatched = await client.execute(begin);
  assert.equal(dispatched.status, "accepted");
  assert.equal(client.current().shortcutTrial?.state, "idle", "accepted dispatch is not a physical press/release fact");
  assert.equal(actions[0]?.type, "begin-shortcut-trial");
  const staleDispatch = await client.execute({ ...begin, requestId: "trial-stale-source", sourceRevision: begin.sourceRevision - 1,
    expected: { providerEpoch: client.current().providerEpoch, revision: client.current().revision } });
  assert.equal(staleDispatch.status, "refused", "trial admission must match the current producer revision");
  const idleReplacement = await client.execute({ ...begin, requestId: "trial-idle-replacement", replaces: {
    sourceEpoch: facts.providerEpoch, sourceRevision: facts.revision, bindingId: "binding-1", effectiveBinding: "Ctrl+Shift+R", reservationId: "old-reservation",
  }, expected: { providerEpoch: client.current().providerEpoch, revision: client.current().revision } });
  assert.equal(idleReplacement.status, "refused", "idle admission cannot be used to replace a terminal identity");
  facts.revision++;
  facts.shortcutTrial = { state: "reserved", sourceEpoch: facts.providerEpoch, sourceRevision: 1, bindingId: "binding-1", effectiveBinding: "Ctrl+Shift+R",
    reservationId: "reservation-1", expiresAt: "2026-01-01T00:01:00Z" };
  const reservedSnapshot = client.current();
  assert.equal((await client.execute({ type: "refresh", requestId: "trial-reservation-readback", expected: { providerEpoch: reservedSnapshot.providerEpoch, revision: reservedSnapshot.revision } })).status, "accepted");
  assert.equal(client.current().shortcutTrial?.state, "reserved");
  const unrelatedCancel = await client.execute({ type: "cancel-shortcut-trial", requestId: "trial-cancel-unrelated", reservationId: "different-request-reservation",
    expected: { providerEpoch: client.current().providerEpoch, revision: client.current().revision } });
  assert.equal(unrelatedCancel.status, "refused", "a different request cannot consume this reservation");
  const cancelAccepted = await client.execute({ type: "cancel-shortcut-trial", requestId: "trial-cancel-dispatch", reservationId: "reservation-1",
    expected: { providerEpoch: client.current().providerEpoch, revision: client.current().revision } });
  assert.equal(cancelAccepted.status, "accepted");
  assert.equal(client.current().shortcutTrial?.state, "reserved", "accepted cancellation is not a terminal cancellation fact");
  facts.revision++;
  facts.shortcutTrial = { state: "observed", sourceEpoch: facts.providerEpoch, sourceRevision: 1, bindingId: "binding-1", effectiveBinding: "Ctrl+Shift+R",
    reservationId: "reservation-1", pressId: "physical-pair-1", pressedAt: "2026-01-01T00:00:01Z", releasedAt: "2026-01-01T00:00:01.100Z" };
  const refresh = client.current();
  const result = await client.execute({ type: "refresh", requestId: "trial-readback", expected: { providerEpoch: refresh.providerEpoch, revision: refresh.revision } });
  assert.equal(result.status, "accepted");
  assert.equal(client.current().shortcutTrial?.state, "observed");

  facts.revision++;
  facts.shortcut = { ...facts.shortcut, bindingId: "binding-2", effective: "Alt+R", label: "Alt R" };
  const changed = await client.execute({ type: "refresh", requestId: "trial-binding-change", expected: { providerEpoch: client.current().providerEpoch, revision: client.current().revision } });
  assert.equal(changed.status, "accepted");
  assert.equal(client.current().shortcutTrial?.state, "unknown", "binding changes invalidate the old reservation without erasing its identity");
  const cancel = await client.execute({ type: "cancel-shortcut-trial", requestId: "trial-cancel-stale", reservationId: "reservation-1", expected: { providerEpoch: client.current().providerEpoch, revision: client.current().revision } });
  assert.equal(cancel.status, "refused", "a stale-binding trial cannot be cancelled as though it still owns the current shortcut");
  client.detach();

  const malformedFacts = nativeFacts();
  malformedFacts.shortcut.bindingId = "binding-1";
  malformedFacts.shortcutTrial = { state: "observed", sourceEpoch: malformedFacts.providerEpoch, sourceRevision: 1, bindingId: "binding-1", effectiveBinding: "Ctrl+Shift+R",
    reservationId: "reservation-1", pressId: "press-1", pressedAt: "2026-01-01T00:00:02Z", releasedAt: "2026-01-01T00:00:01Z" };
  const malformed = createRecordCapabilityProvider({ runtime, native: { readFacts: async () => structuredClone(malformedFacts) }, providerEpoch: "provider-trial-malformed" });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(malformed.current().shortcutTrial, { state: "unavailable" }, "release before press is not a physical trial observation");
  malformed.detach();

  const editingFacts = nativeFacts();
  editingFacts.shortcut = { ...editingFacts.shortcut, bindingId: "binding-1", draft: { sessionId: "edit-1", value: "Alt+R" } };
  editingFacts.shortcutTrial = { state: "idle", sourceEpoch: editingFacts.providerEpoch, sourceRevision: editingFacts.revision, bindingId: "binding-1", effectiveBinding: "Ctrl+Shift+R" };
  const editing = createRecordCapabilityProvider({ runtime: runtimeFixture().runtime, native: { readFacts: async () => structuredClone(editingFacts), dispatch: async () => "accepted" }, providerEpoch: "provider-trial-editing" });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(editing.current().shortcutTrial, { state: "unavailable" }, "an active draft does not expose an admissible effective shortcut trial");
  const editingSnapshot = editing.current();
  assert.equal((await editing.execute({ type: "begin-shortcut-trial", requestId: "trial-during-edit", reservationId: "trial-edit", bindingId: "binding-1", effectiveBinding: "Ctrl+Shift+R",
    sourceEpoch: editingFacts.providerEpoch, sourceRevision: editingFacts.revision, expiresAt: "2026-01-01T00:01:00Z", expected: { providerEpoch: editingSnapshot.providerEpoch, revision: editingSnapshot.revision } })).status, "refused");
  editing.detach();

  const staleEpochFacts = nativeFacts();
  staleEpochFacts.shortcut.bindingId = "binding-1";
  staleEpochFacts.shortcutTrial = { state: "reserved", sourceEpoch: staleEpochFacts.providerEpoch, sourceRevision: 1, bindingId: "binding-1", effectiveBinding: "Ctrl+Shift+R",
    reservationId: "reservation-old-epoch", expiresAt: "2026-01-01T00:01:00Z" };
  const staleEpoch = createRecordCapabilityProvider({ runtime: runtimeFixture().runtime, native: { readFacts: async () => structuredClone(staleEpochFacts) }, providerEpoch: "provider-trial-reconnect" });
  await new Promise(resolve => setImmediate(resolve));
  staleEpochFacts.providerEpoch = "native-epoch-2";
  staleEpochFacts.revision = 1;
  const reconnectSnapshot = staleEpoch.current();
  await staleEpoch.execute({ type: "refresh", requestId: "trial-after-reconnect", expected: { providerEpoch: reconnectSnapshot.providerEpoch, revision: reconnectSnapshot.revision } });
  assert.equal(staleEpoch.current().shortcutTrial?.state, "unknown", "a reservation from another producer epoch stays unknown after reconnect");
  staleEpoch.detach();
});

test("terminal shortcut trial can rearm only from its exact retained identity and fresh current selectors", async () => {
  for (const state of ["observed", "expired", "cancelled", "unknown"] as const) {
    const facts = nativeFacts();
    facts.providerEpoch = "native-epoch-current";
    facts.revision = 4;
    facts.shortcut = { ...facts.shortcut, bindingId: "binding-current", effective: "Alt+R", label: "Alt R" };
    const previous = { sourceEpoch: "native-epoch-original", sourceRevision: 2, bindingId: "binding-original", effectiveBinding: "Ctrl+Shift+R", reservationId: `reservation-original-${state}` };
    facts.shortcutTrial = state === "observed"
      ? { state, ...previous, pressId: "press-1", pressedAt: "2026-01-01T00:00:01.000Z", releasedAt: "2026-01-01T00:00:01.100Z" }
      : { state, ...previous };
    const actions: RecordCommand[] = [];
    const dispatchStatus = state === "unknown" ? "outcome-unknown" as const : "accepted" as const;
    const client = createRecordCapabilityProvider({ runtime: runtimeFixture().runtime, providerEpoch: `provider-terminal-rearm-${state}`, native: {
      readFacts: async () => structuredClone(facts),
      dispatch: async action => { actions.push(structuredClone(action)); return dispatchStatus; },
    } });
    await new Promise(resolve => setImmediate(resolve));
    const before = client.current();
    const fresh = { sourceEpoch: facts.providerEpoch, sourceRevision: facts.revision, bindingId: "binding-current", effectiveBinding: "Alt+R" };
    assert.deepEqual(before.shortcutTrialBeginBinding, fresh);
    assert.deepEqual(before.shortcutTrial, { state: "unknown", ...previous }, "old terminal selectors and reservation identity survive binding/epoch change");

    const requestFor = (requestId: string, selectors = fresh, replaces?: typeof previous) => ({
      type: "begin-shortcut-trial" as const, requestId, reservationId: `reservation-next-${state}`, ...selectors,
      ...(replaces ? { replaces } : {}), expiresAt: "2026-11-07T00:00:00.000Z",
      expected: { providerEpoch: client.current().providerEpoch, revision: client.current().revision },
    });
    assert.equal((await client.execute(requestFor(`trial-missing-replaces-${state}`))).status, "refused");
    const wrongFields: Array<Partial<typeof previous>> = [
      { sourceEpoch: "wrong-epoch" }, { sourceRevision: 3 }, { bindingId: "wrong-binding" },
      { effectiveBinding: "Wrong+R" }, { reservationId: "wrong-reservation" },
    ];
    for (const [index, wrong] of wrongFields.entries())
      assert.equal((await client.execute(requestFor(`trial-wrong-prior-${state}-${index}`, fresh, { ...previous, ...wrong }))).status, "refused");
    const sameId = await client.execute(requestFor(`trial-same-id-${state}`, fresh, { ...previous, reservationId: `reservation-next-${state}` }));
    assert.equal(sameId.status, "refused");
    const extraPrior = await client.execute(requestFor(`trial-extra-prior-${state}`, fresh, { ...previous, privateData: "ignored" } as typeof previous));
    assert.equal(extraPrior.status, "refused");
    const staleSelector = await client.execute(requestFor(`trial-stale-selector-${state}`, { ...fresh, sourceRevision: fresh.sourceRevision - 1 }));
    assert.equal(staleSelector.status, "refused");

    const old = client.current();
    facts.revision++;
    assert.equal((await client.execute({ type: "refresh", requestId: `trial-refresh-${state}`, expected: { providerEpoch: old.providerEpoch, revision: old.revision } })).status, "accepted");
    const current = client.current();
    assert.equal((await client.execute({ ...requestFor(`trial-stale-command-fence-${state}`, fresh), expected: { providerEpoch: old.providerEpoch, revision: old.revision } })).status, "refused");
    const currentFresh = current.shortcutTrialBeginBinding!;
    const withUntrustedTopLevel = { ...requestFor(`trial-rearm-${state}`, currentFresh, previous), privateMetadata: "do-not-forward" };
    const result = await client.execute(withUntrustedTopLevel as RecordCommand);
    assert.equal(result.status, dispatchStatus);
    assert.equal(actions.length, 1);
    assert.equal(Object.hasOwn(actions[0]!, "privateMetadata"), false, "native dispatch receives only the validated trial fields");
    assert.deepEqual(actions[0] && "replaces" in actions[0] ? actions[0].replaces : undefined, previous);
    assert.deepEqual(client.current().shortcutTrial, { state: "unknown", ...previous }, "accepted or ambiguous dispatch preserves the retained terminal fact");
    assert.deepEqual(client.current().shortcutTrialBeginBinding, currentFresh);
    assert.equal(client.current().captures.length, 0);
    if (state === "unknown") {
      const afterUnknown = client.current();
      const retry = await client.execute({ ...requestFor(`trial-rearm-${state}`, afterUnknown.shortcutTrialBeginBinding!, previous),
        expected: { providerEpoch: afterUnknown.providerEpoch, revision: afterUnknown.revision } });
      assert.equal(retry.status, "refused", "an ambiguous dispatch keeps the original request ID consumed and is not retried");
    }
    client.detach();
  }
});

test("raw reserved or pressing trial never receives fresh rearm selectors after projected unknown", async () => {
  for (const state of ["reserved", "pressing"] as const) {
    const facts = nativeFacts();
    facts.providerEpoch = "native-current";
    facts.revision = 8;
    facts.shortcut = { ...facts.shortcut, bindingId: "binding-current", effective: "Alt+R", label: "Alt R" };
    const prior = { sourceEpoch: "native-old", sourceRevision: 2, bindingId: "binding-old", effectiveBinding: "Ctrl+Shift+R", reservationId: `active-${state}` };
    facts.shortcutTrial = { state, ...prior, expiresAt: "2026-11-07T00:00:00.000Z" };
    let dispatched = 0;
    const client = createRecordCapabilityProvider({ runtime: runtimeFixture().runtime, providerEpoch: `provider-active-${state}`, native: {
      readFacts: async () => structuredClone(facts), dispatch: async () => { dispatched++; return "accepted"; },
    } });
    await new Promise(resolve => setImmediate(resolve));
    const snapshot = client.current();
    assert.equal(snapshot.shortcutTrial?.state, "unknown");
    assert.equal(snapshot.shortcutTrialBeginBinding, undefined);
    const result = await client.execute({ type: "begin-shortcut-trial", requestId: `replace-active-${state}`, reservationId: `new-${state}`,
      sourceEpoch: facts.providerEpoch, sourceRevision: facts.revision, bindingId: "binding-current", effectiveBinding: "Alt+R",
      replaces: prior, expiresAt: "2026-11-07T00:00:00.000Z", expected: { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision } });
    assert.equal(result.status, "refused");
    assert.equal(dispatched, 0);
    client.detach();
  }

  const externalFacts = nativeFacts();
  externalFacts.shortcut = { effective: "Ctrl+Shift+R", label: "Ctrl Shift R", bindingId: "external-binding", registration: "externally-managed", externallyManaged: true, draft: null };
  externalFacts.shortcutTrial = { state: "expired", sourceEpoch: "native-old", sourceRevision: 1, bindingId: "old-binding", effectiveBinding: "Ctrl+Shift+R", reservationId: "expired-external" };
  const external = createRecordCapabilityProvider({ runtime: runtimeFixture().runtime, providerEpoch: "provider-external-rearm", native: { readFacts: async () => structuredClone(externalFacts) } });
  await new Promise(resolve => setImmediate(resolve));
  const snapshot = external.current();
  assert.equal(snapshot.shortcutTrialBeginBinding, undefined, "externally managed shortcut has no fresh rearm selector");
  assert.equal((await external.execute({ type: "begin-shortcut-trial", requestId: "trial-external-rearm", reservationId: "new-external",
    sourceEpoch: externalFacts.providerEpoch, sourceRevision: externalFacts.revision, bindingId: "external-binding", effectiveBinding: "Ctrl+Shift+R",
    replaces: { sourceEpoch: "native-old", sourceRevision: 1, bindingId: "old-binding", effectiveBinding: "Ctrl+Shift+R", reservationId: "expired-external" },
    expiresAt: "2026-11-07T00:00:00.000Z", expected: { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision } })).status, "refused");
  external.detach();
});

test("provider preserves cursor-qualified note receipts but refuses malformed cursors or note content", async () => {
  const practice = (): RecordCaptureFacts => ({ ...outputFacts(), captures: [{ captureId: "practice-one", delivery: "unknown", phase: "recording", outcomes: [
    { kind: "note-save", at: "2026-01-01T00:00:03Z", ok: true, noteId: "opaque-note", receiptId: "opaque-receipt", screenStrokeBarrier: { historyId: "screen-history-1", sequence: 8 } },
    { kind: "accepted-stroke", at: "2026-01-01T00:00:02Z", ok: true, surface: "screen", strokeId: "opaque-stroke", cursor: { historyId: "screen-history-1", sequence: 9 } },
  ] }] });
  const output = practice();
  const { runtime } = runtimeFixture();
  const client = createRecordCapabilityProvider({ runtime, captureFacts: { readFacts: async () => structuredClone(output) }, providerEpoch: "provider-practice" });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(client.current().captures[0]?.outcomes, output.captures[0]?.outcomes, "cursor sequence, not timestamp or array arrival order, carries causality");
  client.detach();

  const invalidFrames = [
    (() => { const value = practice(); value.captures[0]!.outcomes[1] = { ...value.captures[0]!.outcomes[1]!, cursor: { historyId: "screen-history-1", sequence: 0 } } as never; return value; })(),
    (() => { const value = practice(); value.captures[0]!.outcomes.push(structuredClone(value.captures[0]!.outcomes[1])); return value; })(),
    (() => { const value = practice(); value.captures[0]!.outcomes[0] = { ...value.captures[0]!.outcomes[0]!, privateText: "/private/note.txt" } as never; return value; })(),
    (() => { const value = practice(); value.captures[0]!.outcomes[0] = { ...value.captures[0]!.outcomes[0]!, noteId: "   " } as never; return value; })(),
    (() => { const value = practice(); value.captures[0]!.outcomes[0] = { ...value.captures[0]!.outcomes[0]!, at: "not-a-time" } as never; return value; })(),
  ];
  for (const [index, invalid] of invalidFrames.entries()) {
    const fixture = runtimeFixture();
    const rejected = createRecordCapabilityProvider({ runtime: fixture.runtime, captureFacts: { readFacts: async () => invalid }, providerEpoch: `provider-invalid-practice-${index}` });
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(rejected.current().captures, [], "invalid or duplicate cursor and malformed note payload cannot publish scoped practice facts");
    rejected.detach();
  }
  assert.equal(createRecordFixture("practice-note-cancelled").driver.current().captures[0]?.outcomes[0]?.kind, "note-save");
});

test("retired output epochs cannot restore old note or stroke receipts", async () => {
  const { runtime } = runtimeFixture();
  let facts = outputFacts();
  const client = createRecordCapabilityProvider({ runtime, providerEpoch: "provider-practice-retired", captureFacts: { readFacts: async () => structuredClone(facts) } });
  await new Promise(resolve => setImmediate(resolve));
  facts = { ...outputFacts(), providerEpoch: "output-epoch-2", revision: 1 };
  let current = client.current();
  await client.execute({ type: "refresh", requestId: "practice-new-epoch", expected: { providerEpoch: current.providerEpoch, revision: current.revision } });
  facts = { ...outputFacts(), providerEpoch: "output-epoch-1", revision: 99, captures: [{ captureId: "retired-note", delivery: "unknown", phase: "recording", outcomes: [
    { kind: "note-save", at: "2026-01-01T00:00:03Z", ok: true, noteId: "opaque-note", receiptId: "opaque-receipt", screenStrokeBarrier: { historyId: "screen-history-1", sequence: 8 } },
  ] }] };
  current = client.current();
  await client.execute({ type: "refresh", requestId: "practice-retired-epoch", expected: { providerEpoch: current.providerEpoch, revision: current.revision } });
  assert.deepEqual(client.current().captures, [], "retired producer epochs cannot restore scoped practice facts");
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

test("confirmed clipboard receipt is scoped to the saved artifact and command, never dispatch acceptance", async () => {
  const { runtime } = runtimeFixture();
  const facts = outputFacts();
  facts.captures = [{ captureId: "saved-copy", artifactId: "artifact-copy", phase: "saved", delivery: "unknown", outcomes: [{ kind: "save", at: "2026-11-06T12:00:00Z", ok: true }] }];
  let writes = 0;
  const client = createRecordCapabilityProvider({ runtime, providerEpoch: "provider-copy", captureFacts: { readFacts: async () => facts },
    clipboard: { async copyLocal(input: { captureId: string; artifactId: string }) { writes++; facts.captures[0]!.delivery = "local-only"; facts.revision++; return { status: "applied" as const, ...input, digest: "a".repeat(64), byteLength: 42 }; } },
  });
  await new Promise(resolve => setImmediate(resolve));
  const snapshot = client.current();
  const request = { type: "copy-local" as const, captureId: "saved-copy", requestId: "copy-explicit", expected: { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision } };
  const pending = client.execute(request);
  request.captureId = "caller-mutated";
  const result = await pending;
  assert.equal(result.status, "applied");
  const receipt = result.snapshot.captures.find(c => c.captureId === "saved-copy")!.outcomes.find(o => o.kind === "copy");
  assert.deepEqual(receipt, { kind: "copy", at: receipt?.at, ok: true, evidence: "native-write-readback", artifactId: "artifact-copy", requestId: "copy-explicit", digest: "a".repeat(64), byteLength: 42 });
  assert.equal(writes, 1);
  assert.equal(result.snapshot.captures[0]!.delivery, "local-only", "copy readback cannot regress newer authoritative capture facts");
  assert.equal((await client.execute({ ...request, captureId: "saved-copy", expected: { providerEpoch: result.snapshot.providerEpoch, revision: result.snapshot.revision } })).status, "refused");
  assert.equal(writes, 1, "same command is never retried");
  client.detach();
});

test("clipboard refusal, ambiguity and mismatched receipts never publish copy success", async () => {
  for (const outcome of ["refused", "outcome-unknown", "foreign", "invalid-digest", "throw"] as const) {
    const { runtime } = runtimeFixture(); const facts = outputFacts();
    facts.captures = [{ captureId: "saved-copy", artifactId: "artifact-copy", phase: "saved", delivery: "unknown", outcomes: [{ kind: "save", at: "2026-11-06T12:00:00Z", ok: true }] }];
    let writes = 0;
    const client = createRecordCapabilityProvider({ runtime, providerEpoch: `provider-${outcome}`, captureFacts: { readFacts: async () => facts }, clipboard: {
      async copyLocal(input: { captureId: string; artifactId: string }) {
        writes++;
        if (outcome === "throw") throw new Error("private path must not escape");
        if (outcome === "refused") return { status: "refused" as const, reason: "too-large" };
        if (outcome === "outcome-unknown") return { status: "outcome-unknown" as const };
        return { status: "applied" as const, captureId: outcome === "foreign" ? "other" : input.captureId, artifactId: input.artifactId, digest: outcome === "invalid-digest" ? "invalid" : "a".repeat(64), byteLength: 42 };
      },
    } });
    await new Promise(resolve => setImmediate(resolve));
    const current = client.current();
    const result = await client.execute({ type: "copy-local", captureId: "saved-copy", requestId: "copy-once", expected: { providerEpoch: current.providerEpoch, revision: current.revision } });
    assert.equal(result.status, outcome === "refused" ? "refused" : "outcome-unknown");
    assert.equal(writes, 1);
    assert.equal(result.snapshot.captures[0]!.outcomes.some(o => o.kind === "copy" && o.ok), false);
    assert.doesNotMatch(JSON.stringify(result), /private path/);
    client.detach();
  }
});

test("clipboard port requires saved artifact identity and suppresses late detached receipts", async () => {
  const { runtime } = runtimeFixture(); const facts = outputFacts();
  facts.captures = [{ captureId: "no-artifact", phase: "saved", delivery: "unknown", outcomes: [{ kind: "save", at: "2026-11-06T12:00:00Z", ok: true }] }];
  let writes = 0; let finish!: (value: { status: "applied"; captureId: string; artifactId: string; digest: string; byteLength: number }) => void;
  let start!: () => void; const started = new Promise<void>(resolve => start = resolve);
  const client = createRecordCapabilityProvider({ runtime, providerEpoch: "provider-late-copy", captureFacts: { readFacts: async () => facts }, clipboard: {
    copyLocal(input: { captureId: string; artifactId: string }) { writes++; start(); return new Promise(resolve => finish = resolve); },
  } });
  await new Promise(resolve => setImmediate(resolve));
  let current = client.current();
  assert.equal((await client.execute({ type: "copy-local", captureId: "no-artifact", requestId: "missing-artifact", expected: { providerEpoch: current.providerEpoch, revision: current.revision } })).status, "refused");
  assert.equal(writes, 0);
  facts.captures[0]!.artifactId = "artifact-copy"; facts.revision++;
  await client.execute(command(client.current(), "refresh", "refresh-copy"));
  current = client.current();
  const pending = client.execute({ type: "copy-local", captureId: "no-artifact", requestId: "late-copy", expected: { providerEpoch: current.providerEpoch, revision: current.revision } });
  await started; client.detach(); finish({ status: "applied", captureId: "no-artifact", artifactId: "artifact-copy", digest: "a".repeat(64), byteLength: 42 });
  assert.equal((await pending).status, "outcome-unknown");
  assert.equal(client.current().captures[0]!.outcomes.some(o => o.kind === "copy"), false);
});


test("stopped processing failures and unknowns are visible without overriding active capture", async () => {
  for (const phase of ["failed", "unknown"] as const) {
    const { runtime } = runtimeFixture();
    await runtime.dispatch({ type: "start", requestId: "processing-error" });
    const client = createRecordCapabilityProvider({ runtime, providerEpoch: "processing-issues",
      captureFacts: { readFacts: async () => ({ ...outputFacts(), captures: [{ captureId: "processing-error", delivery: "unknown", phase,
        outcomes: [{ kind: "processing", at: "2026-11-06T12:00:00Z", ok: false, detail: "media-failed" }] }] }) },
    });
    try {
      await new Promise(resolve => setImmediate(resolve));
      assert.equal(client.current().captures[0]?.phase, "recording", "output failures never replace active native ownership");
      await runtime.dispatch({ type: "stop" });
      assert.equal(client.current().captures[0]?.phase, phase);
      assert.ok(client.current().captures[0]?.outcomes.some(outcome => outcome.kind === "processing" && !outcome.ok));
    } finally { client.detach(); }
  }
});

test("causal receipt identities cannot be rebound across snapshots or reconnect", async () => {
  const note = { kind: "note-save" as const, at: "2026-01-01T00:00:03Z", ok: true as const, noteId: "saved-note", receiptId: "save-receipt",
    screenStrokeBarrier: { historyId: "durable-history", sequence: 10 } };
  const stroke = { kind: "accepted-stroke" as const, at: "2026-01-01T00:00:02Z", ok: true as const, surface: "screen" as const,
    strokeId: "pre-note-stroke", cursor: { historyId: "durable-history", sequence: 9 } };
  let facts: RecordCaptureFacts = { ...outputFacts(), captures: [{ captureId: "causal-capture", phase: "recording", delivery: "unknown", outcomes: [note, stroke] }] };
  const client = createRecordCapabilityProvider({ runtime: runtimeFixture().runtime, providerEpoch: "causal-history-provider", captureFacts: { readFacts: async () => structuredClone(facts) } });
  try {
    await new Promise(resolve => setImmediate(resolve));
    for (const outcomes of [
      [{ ...note, screenStrokeBarrier: { ...note.screenStrokeBarrier, sequence: 8 } }, stroke],
      [note, { ...stroke, cursor: { ...stroke.cursor, sequence: 11 } }],
      [{ ...note, noteId: "different-note" }, stroke],
    ]) {
      facts = { ...facts, revision: facts.revision + 1, captures: [{ ...facts.captures[0]!, outcomes }] };
      const current = client.current();
      await client.execute({ type: "refresh", requestId: `contradiction-${facts.revision}`, expected: { providerEpoch: current.providerEpoch, revision: current.revision } });
      const retained = client.current().captures[0]!.outcomes;
      assert.deepEqual(retained, [note, stroke], "a revised receipt cannot turn the earlier stroke into a post-note effect");
    }
    facts = { ...facts, providerEpoch: "output-reconnected", revision: 1, captures: [{ ...facts.captures[0]!, outcomes: [note, { ...stroke, cursor: { ...stroke.cursor, sequence: 11 } }] }] };
    const reconnect = client.current();
    await client.execute({ type: "refresh", requestId: "contradictory-reconnect", expected: { providerEpoch: reconnect.providerEpoch, revision: reconnect.revision } });
    assert.deepEqual(client.current().captures[0]!.outcomes, [note, stroke], "reconnect cannot rebind durable IDs");
    // An older, previously unseen event can arrive late; its own cursor remains below the barrier.
    const older = { ...stroke, strokeId: "late-pre-note-stroke", cursor: { ...stroke.cursor, sequence: 5 } };
    facts = { ...facts, revision: 2, captures: [{ ...facts.captures[0]!, outcomes: [note, stroke, older] }] };
    const current = client.current();
    await client.execute({ type: "refresh", requestId: "late-original-event", expected: { providerEpoch: current.providerEpoch, revision: current.revision } });
    assert.deepEqual(client.current().captures[0]!.outcomes, [note, stroke, older], "arrival order does not replace durable causal order");
  } finally { client.detach(); }
});
