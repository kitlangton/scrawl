import assert from "node:assert/strict";
import test from "node:test";
import { act, createElement, type ButtonHTMLAttributes } from "react";
import { createRoot } from "react-dom/client";
import { JSDOM } from "jsdom";
import { createRecordFixture } from "@apparent/record/fixtures";
import type { CapabilitySnapshot, NativeRequirement, RecordClient } from "@apparent/record/capabilities";
import { PermissionRepairController, type RepairTimer } from "../dist/permissions/controller.js";
import { PermissionRepairContribution } from "../dist/permissions/react.js";
import { permissionName, permissionStatus } from "../dist/permissions/presentation.js";
import { OnboardingController } from "../dist/controller.js";
import { initialProgress, type ProgressStore, type OnboardingProgress, type SaveResult } from "../dist/model.js";

const dom = new JSDOM("<!doctype html><html><body></body></html>", { url: "http://localhost" });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, HTMLElement: dom.window.HTMLElement, IS_REACT_ACT_ENVIRONMENT: true });
Object.defineProperty(globalThis, "navigator", { configurable: true, value: dom.window.navigator });
const Button = ({ children, variant: _variant, size: _size, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: string; size?: string }) => createElement("button", props, children);
class FakeTimer implements RepairTimer {
  entries = new Map<number, () => void>(); next = 0;
  delays = new Map<number, number>();
  set(callback: () => void, ms: number) { assert.ok([800, 1000].includes(ms)); const id = ++this.next; this.entries.set(id, callback); this.delays.set(id, ms); return id; }
  clear(handle: unknown) { this.entries.delete(handle as number); this.delays.delete(handle as number); }
  fire(ms?: number) { const entries = [...this.entries].filter(([id]) => ms === undefined || this.delays.get(id) === ms); for (const [id, callback] of entries) { this.clear(id); callback(); } }
}
function fixture() {
  const record = createRecordFixture("reconnect-with-saved-history"); let next = 0;
  const timer = new FakeTimer();
  const storage = { value: initialProgress(), revision: 1, outcome: "saved" as "saved" | "conflict" | "unknown" };
  const progress: ProgressStore = {
    async read() { return { revision: storage.revision, value: structuredClone(storage.value) }; },
    async compareAndSwap(expected, value) {
      if (storage.outcome === "unknown") return { status: "unknown", message: "uncertain synthetic write" };
      if (expected !== storage.revision || storage.outcome === "conflict") return { status: "conflict", revision: storage.revision, current: structuredClone(storage.value) };
      storage.value = structuredClone(value); return { status: "saved", revision: ++storage.revision };
    },
  };
  const controller = new PermissionRepairController({ record: record.client, id: () => `repair-${++next}`, timer, progress, now: () => 1_000_000 });
  return { ...record, timer, controller, progress, storage };
}
function publish(driver: ReturnType<typeof createRecordFixture>["driver"], requirements: NativeRequirement[], connection: CapabilitySnapshot["connection"] = "connected") {
  const current = driver.current();
  assert.equal(driver.publish({ ...current, revision: current.revision + 1, connection, readiness: { requirements } }), true);
}
const requirement = (id: string, patch: Partial<NativeRequirement> = {}): NativeRequirement => ({ id, required: true, status: "not-determined", usableNow: false, relaunchNeeded: false, allowedRepairActions: ["request-permission"], ...patch });

test("permission presentation names supported IDs, safely labels future IDs, and describes readiness facts", () => {
  assert.equal(permissionName("screen-capture"), "Screen Recording");
  assert.equal(permissionName("screen-recording"), "Screen & System Audio Recording");
  assert.equal(permissionName("speech-recognition"), "Speech Recognition");
  assert.equal(permissionName("future-internal-id"), "Access");
  for (const id of ["constructor", "__proto__", "toString", "hasOwnProperty"]) assert.equal(permissionName(id), "Access");
  assert.equal(permissionStatus(undefined), "Unavailable");
  assert.equal(permissionStatus(requirement("camera", { status: "granted", usableNow: false })), "Granted · not ready");
  assert.equal(permissionStatus(requirement("camera", { status: "granted", usableNow: true })), "Ready");
  assert.equal(permissionStatus(requirement("camera", { status: "granted", relaunchNeeded: true })), "Reopen Apparent to use this access");
});

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
  assert.equal(f.controller.state.phase, "relaunch-needed");
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
  assert.match(host.textContent ?? "", /Camera/); assert.doesNotMatch(host.textContent ?? "", /Screen Recording/); assert.doesNotMatch(host.textContent ?? "", /camera/);
  const request = [...host.querySelectorAll("button")].find(button => button.textContent === "Request access"); assert.ok(request);
  await act(async () => { request!.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true })); await new Promise(resolve => setTimeout(resolve, 0)); });
  assert.deepEqual(f.driver.commands().map(command => command.type), ["request-permission"]);
  await act(async () => { const later = [...host.querySelectorAll("button")].find(button => button.textContent === "Later"); later!.click(); });
  assert.equal(f.controller.state.phase, "closed"); assert.equal(f.controller.state.closeReason, "later"); assert.equal(closed, 1);
  assert.equal(tour.state.progress.status, "completed"); assert.equal(tour.state.progress.stage, "complete");
  f.controller.dispose(); tour.dispose(); await act(async () => root.unmount()); host.remove();
  assert.deepEqual(f.driver.commands().map(command => command.type), ["request-permission"]);
});

test('commands cannot request a permission outside the explicit subset', async () => {
  const f = fixture(); publish(f.driver, [requirement('microphone'), requirement('camera')]);
  f.controller.activate(['microphone']); assert.equal(await f.controller.repair('camera'), null);
  assert.equal(f.driver.commands().length, 0); f.controller.dispose();
});

test('empty repair never becomes ready when later snapshots arrive', () => {
  const f = fixture(); f.controller.activate([]);
  publish(f.driver, [requirement('camera', { status: 'granted', usableNow: true })]);
  assert.equal(f.controller.state.phase, 'error'); assert.equal(f.timer.entries.size, 0); f.controller.dispose();
});

test('steady usable polling preserves one close deadline and a revoked grant cancels it', () => {
  const f = fixture(), ready = requirement('camera', { status: 'granted', usableNow: true });
  publish(f.driver, [ready]); f.controller.activate(['camera']);
  const original = [...f.timer.entries.keys()]; const staleCallback = [...f.timer.entries.values()][0];
  publish(f.driver, [ready]); publish(f.driver, [ready]);
  assert.deepEqual([...f.timer.entries.keys()], original);
  publish(f.driver, [requirement('camera', { status: 'denied' })]); assert.equal(f.timer.entries.size, 0);
  publish(f.driver, [ready]); staleCallback(); assert.equal(f.controller.state.phase, 'ready', 'a cleared deadline cannot close a fresh interval');
  f.timer.fire(800); assert.equal(f.controller.state.phase, 'closed'); f.controller.dispose();
});

test('accepted responses never replace authoritative ready facts with waiting', async () => {
  const f = fixture(); publish(f.driver, [requirement('camera')]); f.controller.activate(['camera']);
  f.client.execute = async command => ({ status: 'accepted', requestId: command.requestId, snapshot: { ...f.driver.current(), revision: f.driver.current().revision + 1, readiness: { requirements: [requirement('camera', { status: 'granted', usableNow: true })] } } });
  await f.controller.repair('camera'); assert.equal(f.controller.state.phase, 'ready'); f.controller.dispose();
});

test('stale refresh responses do not manufacture usable access', async () => {
  const f = fixture(); publish(f.driver, [requirement('camera')]); f.controller.activate(['camera']);
  f.client.execute = async command => ({ status: 'accepted', requestId: command.requestId, snapshot: { ...f.driver.current(), revision: 0, readiness: { requirements: [requirement('camera', { status: 'granted', usableNow: true })] } } });
  await f.controller.refresh(); assert.equal(f.controller.state.phase, 'blocked'); assert.equal(f.timer.entries.size, 0); f.controller.dispose();
});

test('throwing UI subscribers cannot interrupt permission fact observation', () => {
  const f = fixture(); let calls = 0, bad = 0;
  f.controller.subscribe(() => { bad++; throw new Error('presentation failure'); }); f.controller.subscribe(() => { calls++; });
  publish(f.driver, [requirement('camera')]); f.controller.activate(['camera']);
  publish(f.driver, [requirement('camera', { status: 'granted', usableNow: true })]);
  assert.equal(bad, 1); assert.ok(calls >= 2); assert.equal(f.controller.state.phase, 'ready'); f.controller.dispose();
});

test('unmount cancels the repair deadline without closing a host afterward', async () => {
  const f = fixture(); publish(f.driver, [requirement('camera', { status: 'granted', usableNow: true })]);
  const host = document.createElement('div'); document.body.append(host); const root = createRoot(host); let closed = 0;
  await act(async () => root.render(createElement(PermissionRepairContribution, { controller: f.controller, requestedIds: ['camera'], Button, close() { closed++; } })));
  assert.equal(f.timer.entries.size, 1); await act(async () => root.unmount());
  assert.equal(f.timer.entries.size, 0); f.timer.fire(); assert.equal(closed, 0); host.remove(); f.controller.dispose();
});

test('automatic reopen persists its marker, respects restart cooldown and preserves tour state', async () => {
  const f = fixture(); f.storage.value = { ...initialProgress(), status: 'deferred', stage: 'shortcut' };
  const row = requirement('screen-recording', { status: 'granted', usableNow: false, relaunchNeeded: true, allowedRepairActions: ['request-safe-relaunch'] });
  publish(f.driver, [row]); f.controller.activate(['screen-recording']); f.timer.fire(1000);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(f.driver.commands().filter(c => c.type === 'request-safe-relaunch').length, 1);
  assert.equal(f.storage.value.status, 'deferred'); assert.equal(f.storage.value.stage, 'shortcut'); assert.equal(f.storage.value.legacy.reopenFix, true); assert.equal(f.storage.value.legacy.relaunchedAt, 1_000_000);
  f.controller.dispose();
  const timer = new FakeTimer(), resumed = new PermissionRepairController({ record: f.client, id: () => 'restarted', timer, progress: f.progress, now: () => 1_000_100 });
  resumed.activate(['screen-recording']); timer.fire(1000); await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(f.driver.commands().filter(c => c.type === 'request-safe-relaunch').length, 1, 'restart must not create an automatic loop');
  await resumed.repair('screen-recording'); assert.equal(f.driver.commands().filter(c => c.type === 'request-safe-relaunch').length, 2, 'an explicit user request remains distinct from automatic reopening');
  resumed.dispose();
});

test('uncertain/conflicting restart-state publication prevents a relaunch command', async () => {
  for (const outcome of ['unknown', 'conflict'] as const) {
    const f = fixture(); f.storage.outcome = outcome;
    publish(f.driver, [requirement('screen-capture', { status: 'granted', usableNow: false, relaunchNeeded: true, allowedRepairActions: ['request-safe-relaunch'] })]);
    f.controller.activate(['screen-capture']); await f.controller.repair('screen-capture');
    assert.equal(f.driver.commands().length, 0); assert.equal(f.controller.state.phase, 'error'); f.controller.dispose();
  }
});

test('a stale automatic reopen callback cannot request a permission after readiness changes', () => {
  const f = fixture(); publish(f.driver, [requirement('screen-recording', { status: 'granted', usableNow: false, relaunchNeeded: true, allowedRepairActions: ['request-safe-relaunch'] })]);
  f.controller.activate(['screen-recording']); const callback = [...f.timer.entries.values()][0];
  publish(f.driver, [requirement('screen-recording')]); callback();
  assert.equal(f.driver.commands().length, 0); f.controller.dispose();
});

test("relaunch rechecks eligibility after a delayed CAS even if allowed actions still list restart", async () => {
  const f = fixture(); let release!: () => void;
  const paused = new Promise<void>(resolve => { release = resolve; });
  const progress: ProgressStore = { read: () => f.progress.read(), async compareAndSwap(revision, value) { await paused; return f.progress.compareAndSwap(revision, value); } };
  const controller = new PermissionRepairController({ record: f.client, id: () => "delayed-restart", progress, timer: f.timer });
  publish(f.driver, [requirement("screen-recording", { status: "granted", relaunchNeeded: true, allowedRepairActions: ["request-safe-relaunch"] })]);
  controller.activate(["screen-recording"]);
  const pending = controller.repair("screen-recording");
  await Promise.resolve();
  publish(f.driver, [requirement("screen-recording", { status: "granted", usableNow: true, relaunchNeeded: false, allowedRepairActions: ["request-safe-relaunch"] })]);
  release(); await pending;
  assert.equal(f.driver.commands().length, 0);
  assert.equal(controller.state.phase, "ready");
  controller.dispose(); f.controller.dispose();
});

test("a completed CAS after closing cannot replace the closed state with a publication error", async () => {
  const f = fixture(); let release!: () => void;
  const paused = new Promise<void>(resolve => { release = resolve; });
  const progress: ProgressStore = { read: () => f.progress.read(), async compareAndSwap() { await paused; return { status: "unknown", message: "synthetic ambiguity" }; } };
  const controller = new PermissionRepairController({ record: f.client, id: () => "closing-restart", progress, timer: f.timer });
  publish(f.driver, [requirement("screen-recording", { status: "granted", relaunchNeeded: true, allowedRepairActions: ["request-safe-relaunch"] })]);
  controller.activate(["screen-recording"]);
  const pending = controller.repair("screen-recording"); await Promise.resolve(); controller.close();
  release(); await pending;
  assert.equal(controller.state.phase, "closed"); assert.equal(controller.state.closeReason, "close"); assert.equal(f.driver.commands().length, 0);
  controller.dispose(); f.controller.dispose();
});

test("refresh while ready preserves the continuous-ready deadline", async () => {
  const f = fixture();
  publish(f.driver, [requirement("camera", { status: "granted", usableNow: true, allowedRepairActions: [] })]);
  f.controller.activate(["camera"]); const first = [...f.timer.entries.keys()];
  await f.controller.refresh(); assert.deepEqual([...f.timer.entries.keys()], first);
  f.timer.fire(800); assert.equal(f.controller.state.phase, "closed"); f.controller.dispose();
});
