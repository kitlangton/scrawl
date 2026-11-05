import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import test from "node:test";
import { RecordWorkflow, type CaptureReceipt, type EngineClient, type EngineEvent, type Readiness } from "../dist/backend/workflow/controller.js";
import { createWorkflowPillHost, disposeWorkflowPillHost } from "../dist/ui/workflow-host.js";

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
