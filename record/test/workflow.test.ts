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
