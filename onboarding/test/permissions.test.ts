import assert from "node:assert/strict";
import test from "node:test";
import { act, createElement, type ButtonHTMLAttributes } from "react";
import { createRoot } from "react-dom/client";
import { JSDOM } from "jsdom";
import { createRecordFixture } from "@apparent/record/fixtures";
import type { CapabilitySnapshot, NativeRequirement, RecordClient } from "@apparent/record/capabilities";
import { PermissionRepairController, type RepairTimer } from "../dist/permissions/controller.js";
import { PermissionRepairContribution } from "../dist/permissions/react.js";
import { OnboardingController } from "../dist/controller.js";
import { initialProgress, type ProgressStore, type OnboardingProgress, type SaveResult } from "../dist/model.js";

const dom = new JSDOM("<!doctype html><html><body></body></html>", { url: "http://localhost" });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, HTMLElement: dom.window.HTMLElement, IS_REACT_ACT_ENVIRONMENT: true });
Object.defineProperty(globalThis, "navigator", { configurable: true, value: dom.window.navigator });
const Button = ({ children, variant: _variant, size: _size, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: string; size?: string }) => createElement("button", props, children);
class FakeTimer implements RepairTimer {
  entries = new Map<number, () => void>(); next = 0;
  set(callback: () => void, ms: number) { assert.equal(ms, 800); const id = ++this.next; this.entries.set(id, callback); return id; }
  clear(handle: unknown) { this.entries.delete(handle as number); }
  fire() { const entries = [...this.entries.values()]; this.entries.clear(); for (const callback of entries) callback(); }
}
function fixture() {
  const record = createRecordFixture("reconnect-with-saved-history"); let next = 0;
  const timer = new FakeTimer();
  const controller = new PermissionRepairController({ record: record.client, id: () => `repair-${++next}`, timer });
  return { ...record, timer, controller };
}
function publish(driver: ReturnType<typeof createRecordFixture>["driver"], requirements: NativeRequirement[], connection: CapabilitySnapshot["connection"] = "connected") {
  const current = driver.current();
  assert.equal(driver.publish({ ...current, revision: current.revision + 1, connection, readiness: { requirements } }), true);
}
const requirement = (id: string, patch: Partial<NativeRequirement> = {}): NativeRequirement => ({ id, required: true, status: "not-determined", usableNow: false, relaunchNeeded: false, allowedRepairActions: ["request-permission"], ...patch });

test("explicit repair requests only its subset, including optional camera; permission acceptance is not a grant", async () => {
  const f = fixture();
  publish(f.driver, [requirement("screen-capture"), requirement("camera", { required: false })]);
  f.controller.activate(["camera"]);
  assert.equal(f.controller.state.phase, "blocked");
  const result = await f.controller.repair("camera");
  assert.equal(result?.status, "accepted");
  assert.deepEqual(f.driver.commands().map(command => command.type === "request-permission" ? command.requirementId : command.type), ["camera"]);
  assert.equal(f.controller.state.phase, "waiting");
  publish(f.driver, [requirement("screen-capture"), requirement("camera", { required: false, status: "granted", usableNow: true, allowedRepairActions: [] })]);
  assert.equal(f.controller.state.phase, "ready");
  assert.equal(f.timer.entries.size, 1);
  f.timer.fire(); assert.equal(f.controller.state.phase, "closed"); assert.equal(f.controller.state.closeReason, "ready");
  f.controller.dispose();
});

test("requested but unusable optional requirements block; Input Monitoring and unknown IDs stay unavailable", async () => {
  const f = fixture();
  publish(f.driver, [requirement("camera", { required: false, status: "granted", usableNow: false })]);
  f.controller.activate(["camera"]); assert.equal(f.controller.state.phase, "blocked");
  f.controller.activate(["input-monitoring"]); assert.equal(f.controller.state.phase, "error");
  f.controller.activate(["future-permission"]); assert.equal(f.controller.state.phase, "error");
  assert.equal(await f.controller.repair("camera"), null); assert.equal(f.driver.commands().length, 0);
  f.controller.dispose();
});

test("denied access opens settings; relaunch-needed blocks until usable facts arrive", async () => {
  const f = fixture();
  publish(f.driver, [requirement("microphone", { status: "denied", allowedRepairActions: ["open-system-settings"] })]);
  f.controller.activate(["microphone"]); await f.controller.repair("microphone");
  assert.equal(f.driver.commands()[0]?.type, "open-system-settings");
  publish(f.driver, [requirement("screen-capture", { status: "granted", usableNow: false, relaunchNeeded: true, allowedRepairActions: ["request-safe-relaunch"] })]);
  f.controller.activate(["screen-capture"]); assert.equal(f.controller.state.phase, "relaunch-needed");
  await f.controller.repair("screen-capture"); assert.equal(f.driver.commands()[1]?.type, "request-safe-relaunch");
  assert.equal(f.controller.state.phase, "waiting");
  publish(f.driver, [requirement("screen-capture", { status: "granted", usableNow: true, allowedRepairActions: [] })]);
  assert.equal(f.controller.state.phase, "ready"); f.controller.dispose();
});

test("a refused action can be retried from the same authoritative readiness facts", async () => {
  const f = fixture(); publish(f.driver, [requirement("camera")]); f.controller.activate(["camera"]);
  f.driver.setNextResult("refused");
  assert.equal((await f.controller.repair("camera"))?.status, "refused");
  assert.equal(f.controller.state.phase, "error");
  assert.equal((await f.controller.repair("camera"))?.status, "accepted");
  assert.equal(f.driver.commands().map(command => command.type).join(","), "request-permission,request-permission");
  f.controller.dispose();
});

test("missing facts and disconnect never report ready; reconnect accepts a fresh provider epoch", () => {
  const f = fixture(); f.controller.activate(["camera"]);
  assert.equal(f.controller.state.phase, "blocked", "missing requested row is unavailable, never ready");
  f.driver.disconnect(); assert.equal(f.controller.state.phase, "checking");
  const current = f.driver.current();
  assert.equal(f.driver.reconnect({ ...current, providerEpoch: "fixture-epoch-2", revision: 1, connection: "connected", readiness: { requirements: [requirement("camera", { status: "granted", usableNow: true, allowedRepairActions: [] })] } }), true);
  assert.equal(f.controller.state.phase, "ready"); f.controller.dispose();
});

test("newer facts suppress a late action reply and repeated activation does not duplicate actions", async () => {
  const f = fixture(); publish(f.driver, [requirement("camera")]); f.controller.activate(["camera"]);
  let release!: (value: Awaited<ReturnType<RecordClient["execute"]>>) => void;
  const client: RecordClient = { ...f.client, execute(command) { f.driver.setNextResult("accepted"); return new Promise(resolve => { release = resolve; void f.client.execute(command).then(resolve); }); } };
  const replacement = new PermissionRepairController({ record: client, id: () => "slow" });
  replacement.activate(["camera"]); const pending = replacement.repair("camera");
  replacement.activate(["camera"]); assert.equal(f.driver.commands().length, 1);
  publish(f.driver, [requirement("camera", { status: "granted", usableNow: true, allowedRepairActions: [] })]);
  const latest = f.driver.current();
  release({ status: "accepted", requestId: "slow", snapshot: latest }); await pending;
  assert.equal(replacement.state.phase, "ready"); replacement.dispose(); f.controller.dispose();
});

test("mounted repair controls are owner activation seam; Later preserves tour progress and disposal sends no capture commands", async () => {
  const f = fixture();
  publish(f.driver, [requirement("camera", { required: false })]);
  const store: ProgressStore = { read: async () => ({ revision: 1, value: { ...initialProgress(), status: "completed", stage: "complete" } }), compareAndSwap: async (_: string | number, __: OnboardingProgress): Promise<SaveResult> => { throw new Error("repair must not write tour progress"); } };
  const tour = new OnboardingController({ progress: store, record: f.client, id: () => "tour" }); await tour.load();
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  let closed = 0;
  await act(async () => { root.render(createElement(PermissionRepairContribution, { controller: f.controller, requestedIds: ["camera"], Button, close: () => { closed++; } })); });
  assert.match(host.textContent ?? "", /Camera/); assert.doesNotMatch(host.textContent ?? "", /Screen Recording/);
  const request = [...host.querySelectorAll("button")].find(button => button.textContent === "Request access"); assert.ok(request);
  await act(async () => { request!.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true })); await new Promise(resolve => setTimeout(resolve, 0)); });
  assert.deepEqual(f.driver.commands().map(command => command.type), ["request-permission"]);
  await act(async () => { const later = [...host.querySelectorAll("button")].find(button => button.textContent === "Later"); later!.click(); });
  assert.equal(f.controller.state.phase, "closed"); assert.equal(f.controller.state.closeReason, "later"); assert.equal(closed, 1);
  assert.equal(tour.state.progress.status, "completed"); assert.equal(tour.state.progress.stage, "complete");
  f.controller.dispose(); tour.dispose(); await act(async () => root.unmount()); host.remove();
  assert.deepEqual(f.driver.commands().map(command => command.type), ["request-permission"]);
});
