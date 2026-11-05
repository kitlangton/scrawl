import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import test from "node:test";
import { RecordWorkflow, type CaptureReceipt, type EngineClient, type EngineEvent, type Readiness } from "../src/backend/workflow/controller.ts";

class FakeEngine implements EngineClient {
  events = new EventEmitter();
  ready: Readiness = { ready: true };
  stopCalls: string[] = [];
  cancelCalls: string[] = [];
  stopResult: CaptureReceipt = { captureId: "capture-a", phase: "processing" };
  stopError: Error | undefined;
  async readiness() { return this.ready; }
  async start(requestId: string): Promise<CaptureReceipt> {
    return { captureId: requestId, phase: "starting" };
  }
  async stop(captureId: string): Promise<CaptureReceipt> {
    this.stopCalls.push(captureId);
    if (this.stopError) throw this.stopError;
    return this.stopResult;
  }
  async cancel(captureId: string): Promise<CaptureReceipt> {
    this.cancelCalls.push(captureId);
    return { captureId, phase: "cancelled" };
  }
  async inspect(captureId: string): Promise<CaptureReceipt> {
    return { captureId, phase: "recording" };
  }
  subscribe(listener: (event: EngineEvent) => void): () => void {
    this.events.on("event", listener);
    return () => this.events.off("event", listener);
  }
  emit(event: EngineEvent) { this.events.emit("event", event); }
}

test("readiness gates start, and pending capture can be cancelled by its owned ID", async () => {
  const engine = new FakeEngine();
  engine.ready = { ready: false, reason: "Screen permission required" };
  const flow = new RecordWorkflow(engine);
  await flow.dispatch({ type: "refresh-readiness" });
  await flow.dispatch({ type: "start", requestId: "capture-a" });
  assert.equal(flow.state.phase, "idle");
  assert.deepEqual(flow.state.readiness, engine.ready);
  engine.ready = { ready: true };
  engine.emit({ type: "readiness-changed", readiness: { ready: true } });
  await flow.dispatch({ type: "start", requestId: "capture-a" });
  assert.equal(flow.state.phase, "starting");
  await flow.dispatch({ type: "cancel" });
  assert.equal(flow.state.phase, "cancelled");
  assert.deepEqual(engine.cancelCalls, ["capture-a"]);
  flow.dispose();
});

test("late event for an older capture cannot replace the active session", async () => {
  const engine = new FakeEngine();
  const flow = new RecordWorkflow(engine);
  await flow.dispatch({ type: "refresh-readiness" });
  await flow.dispatch({ type: "start", requestId: "capture-b" });
  engine.emit({ type: "capture-changed", receipt: { captureId: "capture-a", phase: "recording" } });
  assert.deepEqual(flow.state, { phase: "starting", captureId: "capture-b", readiness: { ready: true } });
  engine.emit({ type: "capture-changed", receipt: { captureId: "capture-b", phase: "recording" } });
  assert.equal(flow.state.phase, "recording");
  engine.emit({ type: "capture-changed", receipt: { captureId: "capture-b", phase: "starting" } });
  assert.equal(flow.state.phase, "recording");
  flow.dispose();
});

test("failed stop is retryable with the same ID and saved receipt becomes ready", async () => {
  const engine = new FakeEngine();
  engine.stopError = new Error("control socket timeout");
  const flow = new RecordWorkflow(engine);
  await flow.dispatch({ type: "refresh-readiness" });
  await flow.dispatch({ type: "start", requestId: "capture-a" });
  engine.emit({ type: "capture-changed", receipt: { captureId: "capture-a", phase: "recording" } });
  await flow.dispatch({ type: "stop" });
  assert.equal(flow.state.phase, "stop-failed");
  assert.deepEqual(engine.stopCalls, ["capture-a"]);
  engine.stopError = undefined;
  engine.stopResult = { captureId: "capture-a", phase: "saved", bundlePath: "/captures/a.apparent" };
  await flow.dispatch({ type: "retry-stop" });
  assert.equal(flow.state.phase, "ready");
  assert.deepEqual(engine.stopCalls, ["capture-a", "capture-a"]);
  assert.equal(flow.state.phase === "ready" ? flow.state.bundlePath : "", "/captures/a.apparent");
  flow.dispose();
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
for (const secondId of ["capture-a", "capture-b"]) {
  test(`atomic start admission (${secondId})`, async () => {
    const engine = new FakeEngine();
    const readiness = deferred<Readiness>();
    const reply = deferred<CaptureReceipt>();
    const calls: string[] = [];
    engine.readiness = () => readiness.promise;
    engine.start = id => { calls.push(id); return reply.promise; };
    const flow = new RecordWorkflow(engine);
    const start = flow.dispatch({ type: "start", requestId: "capture-a" });
    void flow.dispatch({ type: "start", requestId: secondId });
    assert.equal(flow.state.phase, "checking");
    readiness.resolve({ ready: true });
    await Promise.resolve();
    void flow.dispatch({ type: "start", requestId: secondId });
    assert.deepEqual(calls, ["capture-a"]);
    reply.resolve({ captureId: "capture-a", phase: "starting" });
    await start;
    flow.dispose();
  });
}
for (const phase of ["recording", "saved"] as const) {
  test(`early ${phase} event supersedes start reply`, async () => {
    const engine = new FakeEngine();
    const reply = deferred<CaptureReceipt>();
    engine.start = () => reply.promise;
    const flow = new RecordWorkflow(engine);
    const start = flow.dispatch({ type: "start", requestId: "capture-a" });
    await Promise.resolve();
    engine.emit({ type: "capture-changed", receipt: { captureId: "capture-a", phase, bundlePath: "/captures/a.apparent" } });
    reply.resolve({ captureId: "capture-a", phase: "starting" });
    await start;
    assert.equal(flow.state.phase, phase === "saved" ? "ready" : "recording");
    flow.dispose();
  });
}
for (const outcome of ["reject", "resolve"] as const) {
  test(`saved event survives late stop ${outcome}`, async () => {
    const engine = new FakeEngine();
    const reply = deferred<CaptureReceipt>();
    engine.stop = () => reply.promise;
    const flow = new RecordWorkflow(engine);
    await flow.dispatch({ type: "start", requestId: "capture-a" });
    engine.emit({ type: "capture-changed", receipt: { captureId: "capture-a", phase: "recording" } });
    const stop = flow.dispatch({ type: "stop" });
    engine.emit({ type: "capture-changed", receipt: { captureId: "capture-a", phase: "saved", bundlePath: "/captures/a.apparent" } });
    if (outcome === "reject") reply.reject(new Error("late timeout"));
    else reply.resolve({ captureId: "capture-a", phase: "processing" });
    await stop;
    assert.equal(flow.state.phase, "ready");
    flow.dispose();
  });
}
test("ambiguous cancel keeps capture ownership until reconciliation", async () => {
  const engine = new FakeEngine();
  engine.cancel = async () => { throw new Error("timeout after admission"); };
  const flow = new RecordWorkflow(engine);
  await flow.dispatch({ type: "start", requestId: "capture-a" });
  await flow.dispatch({ type: "cancel" });
  assert.equal(flow.state.phase, "uncertain");
  await flow.dispatch({ type: "start", requestId: "capture-b" });
  assert.equal(flow.state.phase, "uncertain");
  await flow.dispatch({ type: "reconcile" });
  assert.equal(flow.state.phase, "recording");
  flow.dispose();
});
test("in-flight cancel and stop exclude competing mutations", async () => {
  const engine = new FakeEngine();
  const reply = deferred<CaptureReceipt>();
  engine.cancel = id => { engine.cancelCalls.push(id); return reply.promise; };
  const flow = new RecordWorkflow(engine);
  await flow.dispatch({ type: "start", requestId: "capture-a" });
  engine.emit({ type: "capture-changed", receipt: { captureId: "capture-a", phase: "recording" } });
  const cancel = flow.dispatch({ type: "cancel" });
  await flow.dispatch({ type: "stop" });
  await flow.dispatch({ type: "cancel" });
  assert.deepEqual(engine.stopCalls, []);
  assert.deepEqual(engine.cancelCalls, ["capture-a"]);
  reply.resolve({ captureId: "capture-a", phase: "cancelled" });
  await cancel;
  await flow.dispatch({ type: "start", requestId: "capture-b" });
  engine.emit({ type: "capture-changed", receipt: { captureId: "capture-b", phase: "recording" } });
  const stopped = deferred<CaptureReceipt>();
  engine.stop = () => stopped.promise;
  const stop = flow.dispatch({ type: "stop" });
  await flow.dispatch({ type: "cancel" });
  assert.deepEqual(engine.cancelCalls, ["capture-a"]);
  stopped.resolve({ captureId: "capture-b", phase: "processing" });
  await stop;
  assert.equal(flow.state.phase, "processing");
  flow.dispose();
});
test("ambiguous start retains ownership; disposal ignores pending replies", async () => {
  const engine = new FakeEngine();
  engine.start = async () => { throw new Error("timeout"); };
  const flow = new RecordWorkflow(engine);
  await flow.dispatch({ type: "start", requestId: "capture-a" });
  assert.equal(flow.state.phase, "uncertain");
  await flow.dispatch({ type: "reconcile" });
  assert.equal(flow.state.phase, "recording");
  const reply = deferred<CaptureReceipt>();
  engine.stop = () => reply.promise;
  const stop = flow.dispatch({ type: "stop" });
  const before = flow.state;
  flow.dispose();
  reply.resolve({ captureId: "capture-a", phase: "saved", bundlePath: "/captures/a.apparent" });
  await stop;
  assert.equal(flow.state, before);
});

for (const failingPhase of ["checking", "recording"] as const) {
  test(`throwing observer on ${failingPhase} cannot interrupt Engine ownership or other observers`, async () => {
    const engine = new FakeEngine();
    const flow = new RecordWorkflow(engine);
    let failures = 0;
    const observed: string[] = [];
    flow.subscribe(state => {
      if (state.phase === failingPhase) { failures++; throw new Error("broken UI observer"); }
    });
    flow.subscribe(state => observed.push(state.phase));
    await flow.dispatch({ type: "start", requestId: "capture-a" });
    engine.emit({ type: "capture-changed", receipt: { captureId: "capture-a", phase: "recording" } });
    assert.equal(flow.state.phase, "recording");
    await flow.dispatch({ type: "stop" });
    assert.equal(flow.state.phase, "processing");
    engine.emit({ type: "capture-changed", receipt: { captureId: "capture-a", phase: "saved", bundlePath: "/captures/a.apparent" } });
    await flow.dispatch({ type: "start", requestId: "capture-b" });
    assert.equal(flow.state.phase, "starting");
    assert.equal(failures, 1);
    assert.ok(observed.includes(failingPhase));
    assert.ok(observed.includes("processing"));
    flow.dispose();
  });
}
