import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import test from "node:test";
import type { CapabilitySnapshot, RecordClient, RecordCommand, CommandResult } from "../src/capabilities/index.ts";
import { RecordWorkflow, type CaptureReceipt, type EngineClient, type EngineEvent, type Readiness } from "../dist/backend/workflow/controller.js";
import { createRecordClientPillHost, createWorkflowPillHost, disposeWorkflowPillHost } from "../dist/ui/workflow-host.js";

class ScriptedEngine implements EngineClient {
  events = new EventEmitter();
  ready: Readiness = { ready: false, reason: "Native capture provider unavailable" };
  stops: string[] = [];
  cancels: string[] = [];
  inspections: string[] = [];
  stopError: Error | undefined;
  async readiness() { return this.ready; }
  async start(id: string): Promise<CaptureReceipt> { return { captureId: id, phase: "starting" }; }
  async stop(id: string): Promise<CaptureReceipt> { this.stops.push(id); if (this.stopError) throw this.stopError; return { captureId: id, phase: "processing" }; }
  async cancel(id: string): Promise<CaptureReceipt> { this.cancels.push(id); return { captureId: id, phase: "cancelled" }; }
  async inspect(id: string): Promise<CaptureReceipt> { this.inspections.push(id); return { captureId: id, phase: "recording" }; }
  subscribe(listener: (event: EngineEvent) => void) { this.events.on("event", listener); return () => this.events.off("event", listener); }
  emit(event: EngineEvent) { this.events.emit("event", event); }
}

test("pill host maps unavailable readiness honestly and exposes only supported actions", async () => {
  const engine = new ScriptedEngine(); const workflow = new RecordWorkflow(engine);
  const host = createWorkflowPillHost(workflow, { createRequestId: () => "owned-1" });
  const view = await host.read();
  assert.equal(view?.label, "Capture unavailable");
  assert.equal(view?.warning, "Not checked");
  assert.deepEqual(view?.actions.map(action => action.id), ["refresh"]);
  await host.act(view!.revision, "paste");
  assert.deepEqual(engine.stops, []);
  await host.act(view!.revision, "refresh");
  assert.equal((await host.read())?.warning, "Native capture provider unavailable");
  engine.ready = { ready: true };
  await host.act((await host.read())!.revision, "refresh");
  assert.deepEqual((await host.read())?.actions.map(action => action.id), ["record"]);
  disposeWorkflowPillHost(host); workflow.dispose();
});

test("owned ID flows through cancel, inspect and same-ID stop retry; events supersede commands", async () => {
  const engine = new ScriptedEngine(); engine.ready = { ready: true };
  let nextId = 0;
  const workflow = new RecordWorkflow(engine); const host = createWorkflowPillHost(workflow, { createRequestId: () => `owned-${++nextId}` });
  let view = (await host.read())!;
  await host.act(view.revision, "record");
  view = (await host.read())!;
  assert.equal(view.label, "Starting capture…");
  await host.act(view.revision, "cancel");
  assert.deepEqual(engine.cancels, ["owned-1"]);
  await host.act((await host.read())!.revision, "record");
  engine.emit({ type: "capture-changed", receipt: { captureId: "owned-2", phase: "recording" } });
  view = (await host.read())!;
  await host.act(view.revision - 1, "finish");
  assert.deepEqual(engine.stops, []);
  engine.stopError = new Error("socket timeout");
  await host.act(view.revision, "finish");
  view = (await host.read())!;
  assert.equal(view.label, "Finish could not be confirmed");
  await host.act(view.revision, "inspect");
  assert.deepEqual(engine.inspections, ["owned-2"]);
  assert.equal((await host.read())?.phase, "recording");
  engine.stopError = new Error("socket timeout");
  await host.act((await host.read())!.revision, "finish");
  view = (await host.read())!;
  engine.stopError = undefined;
  engine.stop = async id => { engine.stops.push(id); return { captureId: id, phase: "saved", bundlePath: "/private/capture.apparent" }; };
  await host.act(view.revision, "retry-stop");
  assert.equal((await host.read())?.phase, "completed");
  assert.deepEqual(engine.stops, ["owned-2", "owned-2", "owned-2"]);
  disposeWorkflowPillHost(host); workflow.dispose();
});

test("adapter revisions advance monotonically, observer failures detach, disposal only unsubscribes", async () => {
  const engine = new ScriptedEngine(); engine.ready = { ready: true };
  const workflow = new RecordWorkflow(engine); const host = createWorkflowPillHost(workflow, { createRequestId: () => "capture-dispose" });
  const seen: number[] = [];
  host.subscribe(() => { throw new Error("broken observer"); });
  host.subscribe(view => seen.push(view.revision));
  const first = (await host.read())!;
  await host.act(first.revision, "record");
  assert.ok(seen.length > 0);
  assert.ok(seen.every((value, index) => index === 0 || value > seen[index - 1]!));
  const before = workflow.state;
  const observedAtDispose = seen.length;
  disposeWorkflowPillHost(host);
  engine.emit({ type: "capture-changed", receipt: { captureId: "capture-dispose", phase: "recording" } });
  assert.equal(workflow.state.phase, "recording");
  assert.notEqual(workflow.state, before);
  assert.equal(seen.length, observedAtDispose);
  workflow.dispose();
});

test("authoritative events overtake a pending pill command and replacement hosts own fresh revisions", async () => {
  const engine = new ScriptedEngine(); engine.ready = { ready: true };
  const workflow = new RecordWorkflow(engine);
  const firstHost = createWorkflowPillHost(workflow, { createRequestId: () => "event-owned" });
  await firstHost.act((await firstHost.read())!.revision, "record");
  engine.emit({ type: "capture-changed", receipt: { captureId: "event-owned", phase: "recording" } });
  let finish!: (receipt: CaptureReceipt) => void;
  engine.stop = id => new Promise(resolve => { finish = resolve; engine.stops.push(id); });
  const pendingFinish = firstHost.act((await firstHost.read())!.revision, "finish");
  assert.equal((await firstHost.read())?.label, "Finishing recording…");
  engine.emit({ type: "capture-changed", receipt: { captureId: "event-owned", phase: "processing" } });
  assert.equal((await firstHost.read())?.label, "Processing recording…");
  finish({ captureId: "event-owned", phase: "recording" });
  await pendingFinish;
  assert.equal(workflow.state.phase, "processing");

  const oldView = (await firstHost.read())!;
  const replacement = createWorkflowPillHost(workflow, { createRequestId: () => "unused" });
  const newView = (await replacement.read())!;
  assert.equal(newView.revision, 1);
  assert.ok(newView.revision < oldView.revision);
  const oldUpdates: number[] = [];
  firstHost.subscribe(view => oldUpdates.push(view.revision));
  disposeWorkflowPillHost(firstHost);
  engine.emit({ type: "capture-changed", receipt: { captureId: "event-owned", phase: "saved", bundlePath: "/private/event.apparent" } });
  assert.deepEqual(oldUpdates, []);
  assert.equal((await replacement.read())?.phase, "completed");
  assert.equal(workflow.state.phase, "ready");
  disposeWorkflowPillHost(replacement);
  workflow.dispose();
});

test("reads and observers cannot mutate authoritative revisions or each other's nested actions", async () => {
  const engine = new ScriptedEngine(); engine.ready = { ready: true };
  const workflow = new RecordWorkflow(engine);
  const host = createWorkflowPillHost(workflow, { createRequestId: () => "isolated-view" });
  const original = (await host.read())!;
  const revision = original.revision;
  original.revision = 999; original.actions[0]!.label = "Fake success";
  original.pocket!.title = "Fake readiness";
  const current = (await host.read())!;
  assert.equal(current.revision, revision);
  assert.notEqual(current.actions[0]!.label, "Fake success");
  assert.notEqual(current.pocket!.title, "Fake readiness");
  await host.act(999, "record");
  assert.equal(workflow.state.phase, "idle");
  host.subscribe(view => { view.revision = 999; view.actions.splice(0); });
  const seen: number[] = [];
  host.subscribe(view => { seen.push(view.revision); if (view.label === "Recording") assert.deepEqual(view.actions.map(a => a.id), ["finish", "cancel"]); });
  await host.act(revision, "record");
  engine.emit({ type: "capture-changed", receipt: { captureId: "isolated-view", phase: "recording" } });
  assert.ok(seen.length >= 3);
  assert.ok(seen.every(value => value !== 999));
  assert.deepEqual((await host.read())!.actions.map(a => a.id), ["finish", "cancel"]);
  disposeWorkflowPillHost(host); workflow.dispose();
});

test("throwing clock is cosmetic and never detaches subsequent workflow updates", async () => {
  const engine = new ScriptedEngine(); engine.ready = { ready: true };
  const workflow = new RecordWorkflow(engine);
  let broken = true;
  const host = createWorkflowPillHost(workflow, { createRequestId: () => "clock-safe", clock: () => { if (broken) throw new Error("clock unavailable"); return "00:03"; } });
  assert.equal((await host.read())!.clock, "");
  await host.act((await host.read())!.revision, "record");
  engine.emit({ type: "capture-changed", receipt: { captureId: "clock-safe", phase: "recording" } });
  assert.equal((await host.read())!.phase, "recording");
  broken = false;
  await host.act((await host.read())!.revision, "finish");
  assert.equal((await host.read())!.phase, "processing");
  assert.equal((await host.read())!.clock, "00:03");
  engine.emit({ type: "capture-changed", receipt: { captureId: "clock-safe", phase: "saved", bundlePath: "/private/clock.apparent" } });
  assert.equal((await host.read())!.phase, "completed");
  disposeWorkflowPillHost(host); workflow.dispose();
});

test("unavailable, checking and starting states carry visible status content", async () => {
  const engine = new ScriptedEngine();
  const workflow = new RecordWorkflow(engine);
  const host = createWorkflowPillHost(workflow, { createRequestId: () => "visible-readiness" });
  assert.equal((await host.read())!.pocket?.title, "Capture unavailable");
  let resolve!: (readiness: Readiness) => void;
  engine.readiness = () => new Promise(done => { resolve = done; });
  const checking = host.act((await host.read())!.revision, "record");
  assert.equal((await host.read())!.status, "Checking capture readiness…");
  resolve({ ready: true }); await checking;
  assert.equal((await host.read())!.status, "Starting capture…");
  disposeWorkflowPillHost(host); workflow.dispose();
});

function recordClientFacts(): CapabilitySnapshot {
  return { version: 2, providerEpoch: "provider-a", revision: 3, connection: "connected",
    readiness: { requirements: [{ id: "screen", required: true, status: "granted", usableNow: true, relaunchNeeded: false, allowedRepairActions: [] }] },
    shortcut: { effective: null, label: null, registration: "unknown", externallyManaged: false, draft: null },
    pen: { effective: null, supported: [] }, captures: [], nextShortcutAdmission: { state: "idle" } };
}

function recordClientFixture() {
  let value = recordClientFacts();
  const observers = new Set<(snapshot: CapabilitySnapshot) => void>();
  const commands: RecordCommand[] = [];
  let detached = false;
  const client: RecordClient = {
    observe(listener) { observers.add(listener); listener(structuredClone(value)); return () => observers.delete(listener); },
    async execute(command) {
      commands.push(structuredClone(command));
      if (command.expected.providerEpoch !== value.providerEpoch || command.expected.revision !== value.revision)
        return { status: "refused", requestId: command.requestId, reason: "stale", snapshot: structuredClone(value) };
      if (command.type === "start-capture") value = { ...value, revision: value.revision + 1, captures: [{ captureId: command.requestId, delivery: "host-default", phase: "recording", outcomes: [] }] };
      for (const listener of observers) listener(structuredClone(value));
      return { status: "accepted", requestId: command.requestId, snapshot: structuredClone(value) } as CommandResult;
    },
    detach() { detached = true; },
  };
  return { client, commands, value: () => value, emit(next: CapabilitySnapshot) { value = structuredClone(next); for (const listener of observers) listener(structuredClone(value)); }, isDetached: () => detached };
}

test("RecordClient pill sends host-default start through client and exact-ID lifecycle commands", async () => {
  const fixture = recordClientFixture();
  let ids = 0;
  const host = createRecordClientPillHost(fixture.client, { createRequestId: () => `pill-${++ids}` });
  let view = (await host.read())!;
  assert.deepEqual(view.actions.map(action => action.id), ["record"]);
  await host.act(view.revision, "record");
  assert.equal(fixture.commands[0]?.type, "start-capture");
  assert.equal(fixture.commands[0]?.type === "start-capture" && fixture.commands[0].delivery, "host-default");
  view = (await host.read())!;
  await host.act(view.revision, "finish");
  assert.deepEqual(fixture.commands.map(command => command.type), ["start-capture", "stop-capture"]);
  assert.equal(fixture.commands[1]?.type === "stop-capture" && fixture.commands[1].captureId, "pill-1");
  assert.equal(fixture.commands[1]?.expected.revision, 4);
  disposeWorkflowPillHost(host);
  assert.equal(fixture.isDetached(), false);
});

test("stale pill action is dropped; a complete replacement snapshot changes owned identity", async () => {
  const fixture = recordClientFixture();
  const host = createRecordClientPillHost(fixture.client, { createRequestId: () => "action" });
  const old = (await host.read())!;
  fixture.emit({ ...fixture.value(), revision: 4, captures: [{ captureId: "other", delivery: "unknown", phase: "recording", outcomes: [] }] });
  await host.act(old.revision, "finish");
  assert.equal(fixture.commands.length, 0);
  const current = (await host.read())!;
  await host.act(current.revision, "cancel");
  assert.equal(fixture.commands[0]?.type, "cancel-capture");
  assert.equal(fixture.commands[0]?.type === "cancel-capture" && fixture.commands[0].captureId, "other");
  assert.equal((await host.read())?.warning, "Delivery status unavailable");
  disposeWorkflowPillHost(host);
});

test("ambiguous historical active facts are never arbitrarily bound to stop or cancel", async () => {
  const fixture = recordClientFixture();
  const host = createRecordClientPillHost(fixture.client, { createRequestId: () => "action" });
  fixture.emit({ ...fixture.value(), revision: 4, captures: [
    { captureId: "prior-saved", delivery: "unknown", artifactId: "opaque-artifact", phase: "saved", outcomes: [{ kind: "save", at: "then", ok: true }] },
    { captureId: "old-processing", delivery: "unknown", phase: "processing", outcomes: [] },
    { captureId: "other-recording", delivery: "unknown", phase: "recording", outcomes: [] },
  ] });
  const view = (await host.read())!;
  assert.equal(view.label, "Capture ownership needs reconciliation");
  assert.notEqual(view.phase, "completed");
  assert.deepEqual(view.actions.map(action => action.id), ["refresh"]);
  await host.act(view.revision, "finish");
  await host.act(view.revision, "cancel");
  assert.equal(fixture.commands.length, 0);
  disposeWorkflowPillHost(host);
});

test("only authenticated artifact identity displays saved state; local-only and readiness refusal stay visible", async () => {
  const fixture = recordClientFixture();
  const host = createRecordClientPillHost(fixture.client, { createRequestId: () => "action" });
  fixture.emit({ ...fixture.value(), revision: 4, readiness: { requirements: [{ id: "screen", required: true, status: "denied", usableNow: false, relaunchNeeded: false, allowedRepairActions: [] }] }, captures: [
    { captureId: "historical-path-only", delivery: "local-only", phase: "saved", outcomes: [{ kind: "save", at: "now", ok: true, detail: "/private/fake.apparent" }] },
  ] });
  const view = (await host.read())!;
  assert.notEqual(view.phase, "completed");
  assert.notEqual(view.notice, "/private/fake.apparent");
  assert.deepEqual(view.actions.map(action => action.id), ["refresh"]);
  disposeWorkflowPillHost(host);
});

test("RecordClient pill retires old provider epochs after reconnect", async () => {
  const fixture = recordClientFixture();
  const host = createRecordClientPillHost(fixture.client, { createRequestId: () => "reconnect-action" });
  const old = fixture.value();
  fixture.emit({ ...old, providerEpoch: "provider-b", revision: 1, captures: [{ captureId: "new-capture", delivery: "unknown", phase: "recording", outcomes: [] }] });
  const replacement = (await host.read())!;
  fixture.emit({ ...old, revision: 100, captures: [{ captureId: "retired-capture", delivery: "unknown", phase: "recording", outcomes: [] }] });
  assert.equal((await host.read())!.revision, replacement.revision, "retired epoch cannot replace the current frame");
  await host.act(replacement.revision, "finish");
  assert.equal(fixture.commands[0]?.expected.providerEpoch, "provider-b");
  assert.equal(fixture.commands[0]?.type === "stop-capture" && fixture.commands[0].captureId, "new-capture");
  disposeWorkflowPillHost(host);
});

test("RecordClient pill copies the visible authenticated saved artifact and recognizes its command receipt", async () => {
  const fixture = recordClientFixture();
  let requests = 0;
  const host = createRecordClientPillHost(fixture.client, { createRequestId: () => `copy-${++requests}` });
  fixture.emit({ ...fixture.value(), revision: 4, captures: [
    { captureId: "saved-visible", artifactId: "artifact-visible", delivery: "unknown", phase: "saved", outcomes: [{ kind: "save", at: "now", ok: true }] },
    { captureId: "path-only-history", delivery: "unknown", phase: "saved", outcomes: [{ kind: "save", at: "now", ok: true }] },
  ] });
  fixture.client.execute = async command => {
    fixture.commands.push(command);
    assert.equal(command.type, "copy-local");
    assert.equal(command.type === "copy-local" && command.captureId, "saved-visible");
    fixture.emit({ ...fixture.value(), revision: 5, captures: fixture.value().captures.map(capture => capture.captureId === "saved-visible" ? { ...capture, outcomes: [...capture.outcomes,
      { kind: "copy", at: "now", ok: true, evidence: "native-write-readback", artifactId: "artifact-visible", requestId: command.requestId, digest: "a".repeat(64), byteLength: 42 },
    ] } : capture) });
    return { status: "applied", requestId: command.requestId, snapshot: fixture.value() };
  };
  const before = (await host.read())!;
  assert.equal(before.actions.find(action => action.id === "copy")?.label, "Copy handoff");
  assert.equal(before.notice, "Saved locally");
  await host.act(before.revision, "copy");
  assert.equal((await host.read())!.notice, "Handoff copied");
  assert.equal(fixture.commands.length, 1);
  fixture.emit({ ...fixture.value(), revision: 6, captures: [{ ...fixture.value().captures[0]!, artifactId: "replacement-artifact" }] });
  assert.equal((await host.read())!.notice, "Saved locally", "prior artifact receipt does not prove replacement copied");
  await host.act(before.revision, "copy");
  assert.equal(fixture.commands.length, 1, "stale view cannot dispatch another copy");
  disposeWorkflowPillHost(host);
  assert.equal(fixture.isDetached(), false);
});

test("RecordClient pill never presents accepted, refused or unknown copy as a fresh confirmed effect", async () => {
  for (const status of ["accepted", "refused", "outcome-unknown", "throw"] as const) {
    const fixture = recordClientFixture();
    const host = createRecordClientPillHost(fixture.client, { createRequestId: () => "fresh-copy" });
    fixture.emit({ ...fixture.value(), revision: 4, captures: [{ captureId: "saved", artifactId: "artifact", delivery: "unknown", phase: "saved", outcomes: [
      { kind: "save", at: "then", ok: true },
      { kind: "copy", at: "then", ok: true, evidence: "native-write-readback", artifactId: "artifact", requestId: "past-copy", digest: "a".repeat(64), byteLength: 42 },
    ] }] });
    fixture.client.execute = async command => {
      fixture.commands.push(command);
      if (status === "throw") throw new Error("private path should stay private");
      return status === "refused" ? { status, reason: "unavailable", requestId: command.requestId, snapshot: fixture.value() }
        : { status, requestId: command.requestId, snapshot: fixture.value() };
    };
    const view = (await host.read())!;
    await host.act(view.revision, "copy");
    const after = (await host.read())!;
    assert.equal(after.warning, status === "refused" ? "Copy unavailable" : "Copy outcome unknown");
    assert.equal(after.notice, null);
    assert.equal(fixture.commands.length, 1, "no automatic retry");
    assert.doesNotMatch(JSON.stringify(after), /private path/);
    disposeWorkflowPillHost(host);
  }
});

test("RecordClient pill presents transient text readback without claiming ownership", async () => {
  const fixture = recordClientFixture();
  let requests = 0;
  const host = createRecordClientPillHost(fixture.client, { createRequestId: () => `copy-${++requests}` });
  fixture.emit({ ...fixture.value(), revision: 4, captures: [
    { captureId: "saved-visible", artifactId: "artifact-visible", delivery: "unknown", phase: "saved", outcomes: [{ kind: "save", at: "now", ok: true }] },
    { captureId: "path-only-history", delivery: "unknown", phase: "saved", outcomes: [{ kind: "save", at: "now", ok: true }] },
  ] });
  fixture.client.execute = async command => {
    fixture.commands.push(command);
    assert.equal(command.type, "copy-local");
    assert.equal(command.type === "copy-local" && command.captureId, "saved-visible");
    fixture.emit({ ...fixture.value(), revision: 5, captures: fixture.value().captures.map(capture => capture.captureId === "saved-visible" ? { ...capture, outcomes: [...capture.outcomes,
      { kind: "copy", at: "now", ok: true, evidence: "native-text-readback", artifactId: "artifact-visible", requestId: command.requestId, digest: "a".repeat(64), byteLength: 42 },
    ] } : capture) });
    return { status: "written-readback", requestId: command.requestId, snapshot: fixture.value() };
  };
  const before = (await host.read())!;
  assert.equal(before.actions.find(action => action.id === "copy")?.label, "Copy handoff");
  assert.equal(before.notice, "Saved locally");
  await host.act(before.revision, "copy");
  assert.equal((await host.read())!.notice, "Handoff submitted; matching clipboard text observed");
  assert.equal(fixture.commands.length, 1);
  fixture.emit({ ...fixture.value(), revision: 6, captures: [{ ...fixture.value().captures[0]!, artifactId: "replacement-artifact" }] });
  assert.equal((await host.read())!.notice, "Saved locally", "prior artifact receipt does not prove replacement copied");
  await host.act(before.revision, "copy");
  assert.equal(fixture.commands.length, 1, "stale view cannot dispatch another copy");
  disposeWorkflowPillHost(host);
  assert.equal(fixture.isDetached(), false);
});

 test("Saved pill displays the sealed initial audio gap without claiming recorded silence",async()=>{
  const fixture=recordClientFixture();const host=createRecordClientPillHost(fixture.client,{createRequestId:()=>"gap-copy"});
  fixture.emit({...fixture.value(),revision:4,captures:[{captureId:"saved-gap",artifactId:"artifact-gap",phase:"saved",delivery:"unknown",transcriptStatus:{state:"collected",content:"nonempty",segmentCount:2},outcomes:[{kind:"save",ok:true,at:"now"}],warnings:[{kind:"audioInitialGap",atMs:125}]}]});
  const view=(await host.read())!;assert.equal(view.label,"Recording saved");assert.equal(view.notice,"Saved locally · Transcript collected; 2 segments");
  assert.match(view.warning!,/unobserved/u,"the audio-gap warning keeps priority over transcript status");
  fixture.emit({...fixture.value(),revision:5,captures:[{captureId:"saved-empty",artifactId:"artifact-empty",phase:"saved",delivery:"unknown",transcriptStatus:{state:"collected",content:"empty",segmentCount:0},outcomes:[{kind:"save",ok:true,at:"now"}]}]});
  assert.equal((await host.read())!.notice,"Saved locally · Transcript collected; no words");
  fixture.emit({...fixture.value(),revision:6,captures:[{captureId:"saved-unavailable",artifactId:"artifact-unavailable",phase:"saved",delivery:"unknown",transcriptStatus:{state:"unavailable"},warnings:[{kind:"transcriptUnavailable"}],outcomes:[{kind:"save",ok:true,at:"now"}]}]});
  assert.equal((await host.read())!.notice,"Saved locally · Transcript unavailable");
  assert.match(view.warning!,/\[0, 125 ms\).*unobserved.*padded with zeros.*not recorded silence/);assert.equal(view.pocket?.tone,"warning");
  disposeWorkflowPillHost(host);
 });

test("current cancellation requires its observed terminal fact and preserves previous Saved copy identity", async () => {
  const fixture=recordClientFixture(); const host=createRecordClientPillHost(fixture.client,{createRequestId:()=>"cancel-or-copy"});
  const previous:CapabilitySnapshot["captures"][number]={captureId:"previous",artifactId:"previous-artifact",phase:"saved",delivery:"unknown",outcomes:[],transcriptStatus:{state:"collected",content:"nonempty",segmentCount:2}};
  const current:CapabilitySnapshot["captures"][number]={captureId:"current",phase:"recording",delivery:"unknown",outcomes:[]};
  const unrelated:CapabilitySnapshot["captures"][number]={captureId:"historical-cancel",phase:"cancelled",delivery:"unknown",outcomes:[]};
  fixture.emit({...fixture.value(),revision:4,captures:[previous,current,unrelated]});
  await host.act((await host.read())!.revision,"cancel");
  assert.equal(fixture.commands[0]?.type,"cancel-capture");
  assert.equal((fixture.commands[0] as {captureId:string}).captureId,"current");
  assert.equal((await host.read())!.label,"Recording","accepted dispatch is not cancellation");
  fixture.emit({...fixture.value(),revision:5,captures:[previous,{...current,phase:"unknown"},unrelated]});
  assert.doesNotMatch((await host.read())!.notice??"",/cancelled/);
  fixture.emit({...fixture.value(),revision:4,captures:[previous,{...current,phase:"cancelled"}]});
  assert.notEqual((await host.read())!.label,"Capture cancelled","stale terminal is ignored");
  fixture.emit({...fixture.value(),revision:6,captures:[previous,{...current,phase:"cancelled"},unrelated]});
  const view=(await host.read())!;
  assert.equal(view.label,"Capture cancelled");
  assert.match(view.notice!,/Current capture cancelled\. Previous recording remains saved\./);
  assert.match(view.notice!,/Transcript collected; 2 segments/);
  assert.deepEqual(view.actions.map(action=>action.id),["copy","record"]);
  assert.deepEqual(fixture.value().captures[0],previous);
  fixture.client.execute = async command => {
    fixture.commands.push(command);
    return { status: "accepted", requestId: command.requestId,
      snapshot: { ...fixture.value(), captures: [previous, current] } };
  };
  await host.act(view.revision,"copy");
  const copied=fixture.commands.at(-1)!;
  assert.equal(copied.type,"copy-local");assert.equal("captureId"in copied&&copied.captureId,"previous");
  assert.match((await host.read())!.notice!,/Current capture cancelled/);
  assert.equal((await host.read())!.label,"Capture cancelled","equal-revision command reply cannot replace observed cancellation");
  assert.match((await host.read())!.warning!,/Copy outcome unknown/);
  fixture.emit({...fixture.value(),revision:7,captures:[previous,{...current,captureId:"next"},{...current,phase:"cancelled"}]});
  assert.equal((await host.read())!.label,"Recording");
  assert.doesNotMatch((await host.read())!.notice??"",/cancelled/);
  disposeWorkflowPillHost(host);
});

for(const boundary of ["unrelated","ambiguous","epoch","reconnect","same-revision","initial-unknown"] as const) test(`cancellation correlation refuses ${boundary} history`,async()=>{
  const fixture=recordClientFixture(),host=createRecordClientPillHost(fixture.client,{createRequestId:()=>"action"});
  const previous:CapabilitySnapshot["captures"][number]={captureId:"previous",artifactId:"artifact",phase:"saved",delivery:"unknown",outcomes:[]};
  const current:CapabilitySnapshot["captures"][number]={captureId:"current",phase:"recording",delivery:"unknown",outcomes:[]};
  fixture.emit({...fixture.value(),revision:4,captures:[previous,{...current,phase:boundary==="initial-unknown"?"unknown":"recording"}]});
  if(boundary==="ambiguous")fixture.emit({...fixture.value(),revision:5,captures:[previous,current,{...current,captureId:"other"}]});
  if(boundary==="reconnect")fixture.emit({...fixture.value(),revision:5,connection:"disconnected",captures:[previous,current]});
  fixture.emit({...fixture.value(),revision:boundary==="same-revision"?4:6,providerEpoch:boundary==="epoch"?"provider-b":"provider-a",connection:"connected",captures:[previous,{...current,captureId:boundary==="unrelated"?"unrelated":"current",phase:"cancelled"}]});
  assert.notEqual((await host.read())!.label,"Capture cancelled");
  assert.doesNotMatch((await host.read())!.notice??"",/Current capture cancelled/);
  disposeWorkflowPillHost(host);
});

test("a single observed capture can cancel without prior Saved output or a local Cancel command", async () => {
  const fixture = recordClientFixture();
  const host = createRecordClientPillHost(fixture.client, { createRequestId: () => "unused" });
  const current: CapabilitySnapshot["captures"][number] = { captureId: "current", phase: "starting", delivery: "unknown", outcomes: [] };
  fixture.emit({ ...fixture.value(), revision: 4, captures: [current] });
  fixture.emit({ ...fixture.value(), revision: 5, captures: [{ ...current, phase: "cancelled" }] });
  const view = (await host.read())!;
  assert.equal(view.label, "Capture cancelled");
  assert.equal(view.status, "Capture cancelled");
  assert.equal(view.notice, "Current capture cancelled.");
  assert.equal(view.actions.some(action => action.id === "copy"), false);
  assert.deepEqual(fixture.commands, []);
  disposeWorkflowPillHost(host);
});
