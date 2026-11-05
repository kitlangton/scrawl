import assert from "node:assert/strict";
import test from "node:test";
import { createElement, type ButtonHTMLAttributes } from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { JSDOM } from "jsdom";
import { renderToStaticMarkup } from "react-dom/server";
import { createRecordFixture } from "@apparent/record/fixtures";
import { hasConfirmedArtifactOpen, type ArtifactOpenOutcome, type CaptureFact, type CaptureOutcome } from "@apparent/record/capabilities";
import { OnboardingController } from "../dist/controller.js";
import { initialProgress, normalizeProgress, type OnboardingProgress, type ProgressStore, type SaveResult } from "../dist/model.js";
import { OnboardingContribution } from "../dist/react.js";

class Store implements ProgressStore {
  revision = 1; value = initialProgress(); conflict = false; unknown = false;
  async read() { return { revision: this.revision, value: this.value }; }
  async compareAndSwap(expected: number | string, value: OnboardingProgress): Promise<SaveResult> {
    if (this.unknown) return { status: "unknown", message: "offline" };
    if (expected !== this.revision || this.conflict) return { status: "conflict", revision: ++this.revision, current: this.value };
    this.value = value; return { status: "saved", revision: ++this.revision };
  }
}
const TestButton = ({ children, variant: _variant, size: _size, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: string; size?: string }) => createElement("button", props, children);
function fixture(scenario: Parameters<typeof createRecordFixture>[0] = "reconnect-with-saved-history") {
  const progress = new Store(), record = createRecordFixture(scenario), controller = new OnboardingController({ progress, record: record.client, id: (() => { let n = 0; return () => `onboarding-${++n}`; })() });
  return { progress, ...record, controller };
}
function publish(driver: ReturnType<typeof createRecordFixture>["driver"], captures: CaptureFact[]) {
  const current = driver.current(); assert.equal(driver.publish({ ...current, revision: current.revision + 1, captures }), true);
}
function capture(id: string, phase: CaptureFact["phase"], outcomes: CaptureOutcome[], artifactId?: string): CaptureFact {
  return { captureId: id, phase, outcomes, ...(artifactId ? { artifactId } : {}) };
}
const outcome = (kind: "shortcut" | "first-frame-start" | "picker" | "whiteboard" | "accepted-stroke" | "processing" | "save" | "copy" | "open-dispatch", detail?: string): CaptureOutcome => ({ kind, at: "2026-11-05T00:00:00.000Z", ok: true, ...(detail ? { detail } : {}) });

test("normalizes legacy aliases and keeps defer distinct from completion", async () => {
  assert.equal(normalizeProgress({ page: "tutorial", deferred: true }).stage, "welcome");
  assert.equal(normalizeProgress({ page: "tutorial", deferred: true }).status, "deferred");
  assert.equal(normalizeProgress({ seen: true }).status, "completed");
  const f = fixture(); await f.controller.load(); await f.controller.dispatch("defer"); assert.equal(f.controller.state.progress.status, "deferred"); f.controller.dispose();
  const resumed = fixture(); resumed.progress.value = { ...initialProgress(), status: "deferred", stage: "screen-ink", attemptId: "unfinished", manualPasteConfirmed: true }; await resumed.controller.load(); await resumed.controller.dispatch("reopen"); assert.equal(resumed.controller.state.progress.stage, "screen-ink"); assert.equal(resumed.controller.state.progress.attemptId, "unfinished"); resumed.controller.dispose();
  const replay = fixture(); replay.progress.value = { ...initialProgress(), status: "completed", stage: "complete", attemptId: "old", manualPasteConfirmed: true }; await replay.controller.load(); await replay.controller.dispatch("reopen"); assert.equal(replay.controller.state.progress.stage, "welcome"); assert.equal(replay.controller.state.progress.attemptId, null); assert.equal(replay.controller.state.progress.manualPasteConfirmed, false); replay.controller.dispose();
});

test("renders the actual React contribution with accessible controls and packed Record fixture pen choices", async () => {
  const f = fixture(); await f.controller.load(); await f.controller.dispatch("continue");
  const html = renderToStaticMarkup(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() {} }));
  assert.match(html, /Pick your pen/); assert.match(html, /aria-label="ink pen"/); assert.match(html, /Frog guide/); assert.match(html, /Try later/); f.controller.dispose();
});

test("Record acceptance is dispatch only and failed pen writes preserve the effective value", async () => {
  const f = fixture("pen-write-failure-preserves-effective"); await f.controller.load(); await f.controller.dispatch("continue");
  assert.equal(await f.controller.selectPen("laser"), false); assert.equal(f.controller.state.snapshot?.pen.effective, "ink"); assert.equal(f.controller.state.progress.stage, "pen"); f.controller.dispose();
  const g = fixture(); const before = g.driver.current(); const result = await g.client.execute({ type: "refresh", requestId: "r1", expected: { providerEpoch: before.providerEpoch, revision: before.revision } }); assert.equal(result.status, "accepted"); assert.equal(result.snapshot.captures[0].phase, "saved"); g.controller.dispose();
});

test("granted-but-unusable permissions and external shortcuts do not pass setup", async () => {
  const f = fixture("permission-granted-relaunch-needed"); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "permissions" }; await f.controller.load();
  await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "permissions"); assert.match(f.controller.state.error ?? "", /Some required access/); f.controller.dispose();
  const g = fixture("external-shortcut-save-failure"); g.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" }; await g.controller.load();
  await g.controller.dispatch("continue"); assert.equal(g.controller.state.progress.stage, "shortcut"); assert.match(g.controller.state.error ?? "", /shortcut is not ready/); g.controller.dispose();
});

test("practice excludes captures already active at attempt activation", async () => {
  const f = fixture("unrelated-capture"); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "practice-start" };
  const start = f.driver.current(); publish(f.driver, [...start.captures, capture("preexisting", "recording", [outcome("shortcut"), outcome("first-frame-start")])]); await f.controller.load();
  await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "practice-start");
  const original = f.driver.current().captures;
  publish(f.driver, [...original, capture("menu", "recording", [outcome("first-frame-start")])]); await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "practice-start");
  publish(f.driver, [...original, capture("shortcut-no-frame", "starting", [outcome("shortcut")])]); await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "practice-start");
  publish(f.driver, [...original, capture("preexisting", "recording", [outcome("shortcut"), outcome("first-frame-start")])]); await f.controller.dispatch("continue"); assert.match(f.controller.state.error ?? "", /new shortcut-started/);
  publish(f.driver, [...original, capture("eligible", "recording", [outcome("shortcut"), outcome("first-frame-start")])]); await f.controller.dispatch("continue");
  assert.equal(f.controller.state.progress.stage, "practice-start"); assert.equal(f.controller.state.progress.attemptId, null); assert.match(f.controller.state.error ?? "", /stay on this device/); f.controller.dispose();
});

test("CAS conflict and unknown publication never overwrite or claim progress", async () => {
  const f = fixture(); await f.controller.load(); f.progress.conflict = true; await f.controller.dispatch("continue");
  assert.equal(f.controller.state.progress.stage, "welcome"); assert.match(f.controller.state.error ?? "", /changed elsewhere/); f.controller.dispose();
  const g = fixture(); await g.controller.load(); g.progress.unknown = true; await g.controller.dispatch("continue"); assert.equal(g.controller.state.publication, "unknown"); assert.equal(g.controller.state.progress.stage, "welcome"); g.progress.unknown = false; await g.controller.dispatch("retry"); assert.equal(g.controller.state.publication, "confirmed"); g.controller.dispose();
});

test("locate requires the exact saved artifact open proof", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "locate", attemptId: "practice", manualPasteConfirmed: true }; await f.controller.load();
  const outcomes: CaptureOutcome[] = [outcome("save"), outcome("copy")];
  publish(f.driver, [capture("practice", "saved", outcomes, "artifact-1")]);
  outcomes.push(outcome("open-dispatch")); publish(f.driver, [capture("practice", "saved", outcomes, "artifact-1")]); await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "locate");
  const foreignCapture = capture("foreign", "saved", [{ kind: "artifact-open", at: "2026-11-05T00:00:30.000Z", artifactId: "artifact-foreign", ok: true, evidence: "viewer-ack" }], "artifact-foreign");
  publish(f.driver, [capture("practice", "saved", outcomes, "artifact-1"), foreignCapture]); await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "locate"); assert.equal(hasConfirmedArtifactOpen(f.driver.current(), "foreign"), true);
  outcomes.push({ kind: "artifact-open", at: "2026-11-05T00:00:45.000Z", artifactId: "artifact-1", ok: false, reason: "refused" }); publish(f.driver, [capture("practice", "saved", outcomes, "artifact-1"), foreignCapture]); await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "locate");
  outcomes.push({ kind: "artifact-open", at: "2026-11-05T00:01:00.000Z", artifactId: "foreign", ok: true, evidence: "viewer-ack" } satisfies ArtifactOpenOutcome); publish(f.driver, [capture("practice", "saved", outcomes, "artifact-1")]); await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "locate");
  assert.equal(hasConfirmedArtifactOpen({ ...f.driver.current(), captures: [capture("practice", "saved", outcomes, "new-artifact")] }, "practice"), false);
  outcomes.push({ kind: "artifact-open", at: "2026-11-05T00:02:00.000Z", artifactId: "artifact-1", ok: true, evidence: "viewer-ack" } satisfies ArtifactOpenOutcome); publish(f.driver, [capture("practice", "saved", outcomes, "artifact-1")]); assert.equal(hasConfirmedArtifactOpen(f.driver.current(), "practice"), true); await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.status, "completed"); f.controller.dispose();
});

test("fast save and reconnect advance from stop through copied using retained processing facts", async () => {
  const f = fixture("reconnect-with-saved-history"); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "stop", attemptId: "practice" }; await f.controller.load();
  publish(f.driver, [capture("practice", "saved", [outcome("processing"), outcome("save"), outcome("copy")], "artifact-fast")]);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(f.controller.state.progress.stage, "copied"); assert.equal(f.controller.state.progress.attemptId, "practice"); f.controller.dispose();
});

test("mounted React paste confirmation is a separate working control from instructions", async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window }); Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
  f.progress.value = { ...initialProgress(), status: "in-progress", stage: "paste", attemptId: "practice" };
  let instructions = 0, closed = 0; const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => { root.render(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() { closed++; }, onManualPasteInstructions() { instructions++; } })); await Promise.resolve(); });
  const buttons = [...dom.window.document.querySelectorAll("button")];
  await act(async () => { buttons.find(b => b.textContent === "Show paste instructions")!.click(); });
  assert.equal(instructions, 1); assert.equal(f.controller.state.progress.stage, "paste");
  await act(async () => { [...dom.window.document.querySelectorAll("button")].find(b => b.textContent === "I pasted it")!.click(); await Promise.resolve(); });
  assert.equal(f.controller.state.progress.stage, "locate"); assert.equal(instructions, 1); assert.equal(closed, 0);
  await act(async () => { root.unmount(); }); f.controller.dispose(); dom.window.close();
});

test("mounted pen choice commits only on Use this pen and reduced motion advances after readback", async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window }); Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
  Object.defineProperty(dom.window, "matchMedia", { configurable: true, value: () => ({ matches: true }) });
  f.client.execute = async command => { const current = f.driver.current(); if (command.type !== "select-pen") return { status: "accepted", requestId: command.requestId, snapshot: current }; const updated = { ...current, revision: current.revision + 1, pen: { ...current.pen, effective: command.value } }; f.driver.publish(updated); return { status: "applied", requestId: command.requestId, snapshot: updated }; };
  await f.controller.load(); await f.controller.dispatch("continue");
  const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => { root.render(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() {} })); await Promise.resolve(); });
  await act(async () => { dom.window.document.querySelector<HTMLButtonElement>('[aria-label="laser pen"]')!.click(); });
  assert.equal(f.controller.state.snapshot?.pen.effective, "ink", "choosing a swatch is not a provider write");
  await act(async () => { [...dom.window.document.querySelectorAll("button")].find(b => b.textContent === "Use this pen")!.click(); await Promise.resolve(); });
  assert.equal(f.controller.state.snapshot?.pen.effective, "laser"); assert.equal(f.controller.state.progress.stage, "permissions");
  await act(async () => { root.unmount(); }); f.controller.dispose(); dom.window.close();
});

test("mounted Close keeps setup visible when deferral save is unknown", async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window }); Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
  f.progress.unknown = true; let closed = 0; const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => { root.render(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() { closed++; } })); await Promise.resolve(); });
  await act(async () => { dom.window.document.querySelector<HTMLButtonElement>('[aria-label="Close setup"]')!.click(); await Promise.resolve(); });
  assert.equal(closed, 0); assert.equal(f.controller.state.progress.status, "new"); assert.equal(f.controller.state.publication, "unknown");
  await act(async () => { root.unmount(); }); f.controller.dispose(); dom.window.close();
});

test("mounted shortcut action reaches Record and cancellation returns practice to a fresh attempt", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" }; await f.controller.load();
  const dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" }); Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window }); Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
  const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => { root.render(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() {} })); await Promise.resolve(); });
  await act(async () => { [...dom.window.document.querySelectorAll("button")].find(b => b.textContent === "Change shortcut")!.click(); await Promise.resolve(); });
  assert.equal(f.driver.commands().at(-1)?.type, "shortcut-begin-edit");
  f.progress.value = { ...f.progress.value, stage: "screen-ink", attemptId: "cancelled-practice" };
  await act(async () => { await f.controller.load(); publish(f.driver, [capture("cancelled-practice", "cancelled", [])]); await new Promise(resolve => setTimeout(resolve, 0)); });
  assert.equal(f.controller.state.progress.stage, "practice-start"); assert.equal(f.controller.state.progress.attemptId, null);
  await act(async () => { root.unmount(); }); f.controller.dispose(); dom.window.close();
});

test("failed copy offers a scoped copy retry for the saved practice capture", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "preparing", attemptId: "copy-retry" }; await f.controller.load();
  publish(f.driver, [capture("copy-retry", "saved", [outcome("save"), { ...outcome("copy"), ok: false }], "artifact-copy")]);
  const result = await f.controller.retryPracticeCopy(); assert.equal(result?.status, "accepted"); assert.equal(f.driver.commands().at(-1)?.type, "copy-local"); f.controller.dispose();
});

test("mismatched Record response IDs cannot authorize progress", async () => {
  const f = fixture(); await f.controller.load(); await f.controller.dispatch("continue");
  f.client.execute = async command => ({ status: "applied", requestId: "retired-request", snapshot: f.driver.current() });
  assert.equal(await f.controller.selectPen("laser"), false); assert.equal(f.controller.state.command, "unknown"); assert.equal(f.controller.state.progress.stage, "pen");
  await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "pen");
  f.client.execute = async command => ({ status: "accepted", requestId: command.requestId, snapshot: f.driver.current() });
  await f.controller.dispatch("refresh"); assert.notEqual(f.controller.state.command, "unknown"); f.controller.dispose();
});

test("a response from a retired provider epoch cannot confirm a pen write", async () => {
  const f = fixture(); await f.controller.load(); await f.controller.dispatch("continue");
  let finish!: (result: any) => void, requestId = ""; f.client.execute = command => new Promise(resolve => { requestId = command.requestId; finish = resolve; });
  const selecting = f.controller.selectPen("laser"); await Promise.resolve();
  const old = f.driver.current(); f.driver.reconnect({ ...old, providerEpoch: "fixture-epoch-retired", revision: 1, connection: "connected" });
  finish({ status: "applied", requestId, snapshot: old });
  assert.equal(await selecting, false); assert.equal(f.controller.state.command, "unknown"); assert.equal(f.controller.state.snapshot?.providerEpoch, "fixture-epoch-retired"); f.controller.dispose();
});

test("legacy in-progress resume is preserved and completion requires explicit legacy completion", () => {
  assert.deepEqual(normalizeProgress({ resume: "TryIt", deferred: true }).stage, "practice-start");
  assert.equal(normalizeProgress({ resume: "Permissions", seen: false }).stage, "permissions");
  const legacy = normalizeProgress({ seen: false, deferred: true, resume: "try-it", tutorial: "practiced", relaunchedAt: 1700000000000, reopenFix: false, shortcutChosen: true, firstCaptureCompleted: true });
  assert.equal(legacy.stage, "practice-start"); assert.equal(legacy.status, "deferred"); assert.equal(legacy.legacy.tutorial, "practiced"); assert.equal(legacy.legacy.relaunchedAt, 1700000000000); assert.equal(legacy.legacy.firstCaptureCompleted, true);
  assert.equal(normalizeProgress({ seen: false, resume: "microphone", reopenFix: true }).stage, "permissions");
  assert.equal(normalizeProgress({ schema: 1, stage: "complete", status: "in-progress" }).stage, "welcome");
  assert.equal(normalizeProgress({ seen: true, resume: "Permissions" }).stage, "complete");
});

test("reconnect with retained matching open proof remains acceptable", async () => {
  const f = fixture("reconnect-with-saved-history"); const first = f.driver.current();
  const retained = capture("practice", "saved", [{ kind: "artifact-open", at: "2026-11-05T00:00:00Z", artifactId: "a1", ok: true, evidence: "os-observation" }], "a1");
  f.driver.publish({ ...first, revision: first.revision + 1, captures: [retained] });
  f.progress.value = { ...initialProgress(), status: "in-progress", stage: "locate", attemptId: "practice", manualPasteConfirmed: true }; await f.controller.load();
  f.driver.disconnect(); const down = f.driver.current(); f.driver.reconnect({ ...down, providerEpoch: "fixture-epoch-2", revision: 1, connection: "connected", captures: [retained] });
  await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.status, "completed"); f.controller.dispose();
});

test("new provider epoch without the associated capture clears the stale practice identity", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "screen-ink", attemptId: "old-capture" }; await f.controller.load();
  const current = f.driver.current(); f.driver.reconnect({ ...current, providerEpoch: "fixture-epoch-2", revision: 1, connection: "connected", captures: [] });
  assert.equal(f.controller.state.progress.stage, "practice-start"); assert.equal(f.controller.state.progress.attemptId, null); f.controller.dispose();
});

test("disposing Onboarding detaches observation without stopping an active Record capture", async () => {
  const f = fixture("unsubscribe-during-recording"); await f.controller.load(); assert.equal(f.driver.captureContinues(), true); f.controller.dispose();
  assert.equal(f.driver.captureContinues(), true); assert.deepEqual(f.driver.commands(), []);
});

test("pen progression cannot bypass an explicitly confirmed selection", async () => {
  const f = fixture(); await f.controller.load(); await f.controller.dispatch("continue");
  await f.controller.dispatch("continue"); await f.controller.completePenSelection();
  assert.equal(f.controller.state.progress.stage, "pen");
  f.controller.dispose();
});

test("deferred practice does not progress on incoming capture facts", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "deferred", stage: "stop", attemptId: "practice" };
  await f.controller.load();
  publish(f.driver, [capture("practice", "saved", [outcome("processing"), outcome("save"), outcome("copy")])]);
  await Promise.resolve(); await f.controller.dispatch("continue");
  assert.equal(f.controller.state.progress.status, "deferred"); assert.equal(f.controller.state.progress.stage, "stop");
  f.controller.dispose();
});

test("normalized legacy metadata survives a persistence round trip", () => {
  const once = normalizeProgress({ seen: false, resume: "shortcut", tutorial: "practiced", relaunchedAt: 1234, shortcutChosen: true });
  assert.deepEqual(normalizeProgress(JSON.parse(JSON.stringify(once))), once);
});

test("capture updates during a progress write are reconciled after it completes", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "stop", attemptId: "practice" }; await f.controller.load();
  let finish!: () => void;
  const original = f.progress.compareAndSwap.bind(f.progress);
  let delayed = true;
  f.progress.compareAndSwap = async (expected, value) => {
    if (delayed) { delayed = false; await new Promise<void>(resolve => { finish = resolve; }); }
    return original(expected, value);
  };
  publish(f.driver, [capture("practice", "processing", [outcome("processing")])]);
  publish(f.driver, [capture("practice", "saved", [outcome("processing"), outcome("save"), outcome("copy")])]);
  finish(); await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(f.controller.state.progress.stage, "copied"); f.controller.dispose();
});
