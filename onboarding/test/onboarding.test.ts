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
import { onboardingContributionFactory } from "../dist/factory.js";
import { onboardingProgressCodec } from "../dist/model.js";

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
function capture(id: string, phase: CaptureFact["phase"], outcomes: CaptureOutcome[], artifactId?: string, delivery: CaptureFact["delivery"] = "unknown", shortcutAdmissionId?: string): CaptureFact {
  return { captureId: id, phase, delivery, outcomes, ...(shortcutAdmissionId ? { shortcutAdmissionId } : {}), ...(artifactId ? { artifactId } : {}) };
}
const outcome = (kind: "shortcut" | "first-frame-start" | "picker" | "whiteboard" | "accepted-stroke" | "processing" | "save" | "copy" | "open-dispatch", detail?: string): CaptureOutcome => kind === "picker" ? { kind, at: "2026-11-05T00:00:00.000Z", ok: true, surface: "screen", target: "display" } : kind === "accepted-stroke" ? { kind, at: "2026-11-05T00:00:00.000Z", ok: true, surface: detail === "screen" ? "screen" : "whiteboard", strokeId: `stroke-${detail ?? "board"}` } : ({ kind, at: "2026-11-05T00:00:00.000Z", ok: true, ...(detail ? { detail } : {}) } as CaptureOutcome);

test("normalizes legacy aliases and keeps defer distinct from completion", async () => {
  assert.equal(normalizeProgress({ page: "tutorial", deferred: true }).stage, "welcome");
  assert.equal(normalizeProgress({ page: "tutorial", deferred: true }).status, "deferred");
  assert.equal(normalizeProgress({ seen: true }).status, "completed");
  const f = fixture(); await f.controller.load(); await f.controller.dispatch("defer"); assert.equal(f.controller.state.progress.status, "deferred"); f.controller.dispose();
  const resumed = fixture(); resumed.progress.value = { ...initialProgress(), status: "deferred", stage: "screen-ink", attemptId: "unfinished", manualPasteConfirmed: true }; await resumed.controller.load(); await resumed.controller.dispatch("reopen"); assert.equal(resumed.controller.state.progress.stage, "screen-ink"); assert.equal(resumed.controller.state.progress.attemptId, "unfinished"); resumed.controller.dispose();
  const replay = fixture(); replay.progress.value = { ...initialProgress(), status: "completed", stage: "complete", attemptId: "old", manualPasteConfirmed: true }; await replay.controller.load(); await replay.controller.dispatch("reopen"); assert.equal(replay.controller.state.progress.stage, "welcome"); assert.equal(replay.controller.state.progress.attemptId, null); assert.equal(replay.controller.state.progress.manualPasteConfirmed, false); replay.controller.dispose();
});

test("host progress codec preserves owner metadata and rejects future schema", () => {
  const legacy = onboardingProgressCodec.decode({ seen: false, deferred: true, resume: "TryIt", tutorial: "practiced", relaunchedAt: 1700000000000, reopenFix: false, shortcutChosen: true, firstCaptureCompleted: true });
  assert.equal(legacy.status, "deferred"); assert.equal(legacy.stage, "practice-start");
  assert.equal(legacy.legacy.tutorial, "practiced"); assert.equal(legacy.legacy.relaunchedAt, 1700000000000); assert.equal(legacy.legacy.shortcutChosen, true); assert.equal(legacy.legacy.firstCaptureCompleted, true);
  const encoded = onboardingProgressCodec.encode(legacy); assert.notEqual(encoded, legacy);
  assert.throws(() => onboardingProgressCodec.decode({ schema: 2, status: "completed" }), /Unsupported onboarding progress schema/);
  const accessor = { ...initialProgress() }; Object.defineProperty(accessor, "stage", { enumerable: true, get() { throw new Error("getter executed"); } });
  assert.throws(() => onboardingProgressCodec.encode(accessor), /accessors/);
});

test("host factory loads before rendering and applies first-launch versus explicit resume policy", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "deferred", stage: "screen-ink", attemptId: "unfinished" };
  const hosted = await onboardingContributionFactory.create({ progress: f.progress, record: f.client, id: () => "factory-id", Button: TestButton, close() {} });
  await hosted.activation.activate({ reason: "firstLaunch" });
  assert.equal(f.progress.value.stage, "screen-ink"); assert.equal(f.progress.value.status, "deferred");
  const dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window }); Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
  const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => { root.render(createElement(hosted.module.Screen)); await Promise.resolve(); });
  assert.equal(dom.window.document.querySelector("main.onboarding-shell"), null);
  await hosted.activation.activate({ reason: "resume" });
  assert.equal(f.progress.value.stage, "screen-ink"); assert.equal(f.progress.value.status, "in-progress");
  await act(async () => { await Promise.resolve(); });
  assert.equal(dom.window.document.querySelector("main.onboarding-shell")?.getAttribute("data-stage"), "screen-ink");
  await act(async () => { root.unmount(); }); dom.window.close();
  await hosted.activation.dispose(); await assert.rejects(hosted.activation.activate({ reason: "resume" }), /cannot be activated/);
});

test("renders the actual React contribution with accessible controls and packed Record fixture pen choices", async () => {
  const f = fixture(); await f.controller.load(); await f.controller.dispatch("continue");
  const html = renderToStaticMarkup(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() {} }));
  assert.match(html, /Choose your pen color/); for (const color of ["coral", "purple", "blue", "green"]) assert.match(html, new RegExp(`aria-label="${color} pen"`)); assert.doesNotMatch(html, /ink pen|laser pen|cursor pen/); assert.match(html, /Frog guide/); assert.match(html, /Try later/); f.controller.dispose();
});

test("Record acceptance is dispatch only and failed pen writes preserve the effective value", async () => {
  const f = fixture("pen-write-failure-preserves-effective"); await f.controller.load(); await f.controller.dispatch("continue");
  assert.equal(await f.controller.selectPen("purple"), false); assert.equal(f.controller.state.snapshot?.pen.effective, "coral"); assert.equal(f.controller.state.progress.stage, "pen"); f.controller.dispose();
  const g = fixture(); const before = g.driver.current(); const result = await g.client.execute({ type: "refresh", requestId: "r1", expected: { providerEpoch: before.providerEpoch, revision: before.revision } }); assert.equal(result.status, "accepted"); assert.equal(result.snapshot.captures[0].phase, "saved"); g.controller.dispose();
});

test("granted-but-unusable permissions and external shortcuts do not pass setup", async () => {
  const f = fixture("permission-granted-relaunch-needed"); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "permissions" }; await f.controller.load();
  await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "permissions"); assert.match(f.controller.state.error ?? "", /Some required access/); f.controller.dispose();
  const g = fixture("external-shortcut-save-failure"); g.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" }; await g.controller.load();
  await g.controller.dispatch("continue"); assert.equal(g.controller.state.progress.stage, "shortcut"); assert.match(g.controller.state.error ?? "", /shortcut is not ready/); g.controller.dispose();
});

test("practice excludes baseline, unreserved, and host-default shortcut captures", async () => {
  const f = fixture("unrelated-capture"), admissionId = "reserved-practice"; const start = f.driver.current();
  assert.equal(f.driver.publish({ ...start, revision: start.revision + 1, nextShortcutAdmission: { state: "armed", admissionId, delivery: "local-only", expiresAt: "2026-11-05T00:05:00Z" } }), true);
  f.progress.value = { ...initialProgress(), status: "in-progress", stage: "practice-start", admissionId, practiceBaselineIds: ["capture-unrelated"] }; await f.controller.load();
  assert.equal(f.driver.publish({ ...f.driver.current(), revision: f.driver.current().revision + 1, nextShortcutAdmission: { state: "consumed", admissionId, captureId: "host-default" }, captures: [...f.driver.current().captures, capture("host-default", "recording", [outcome("shortcut")], undefined, "host-default", admissionId)] }), true);
  assert.equal(f.controller.state.progress.stage, "practice-start"); assert.equal(f.controller.state.progress.attemptId, null);
  assert.equal(f.driver.publish({ ...f.driver.current(), revision: f.driver.current().revision + 1, nextShortcutAdmission: { state: "consumed", admissionId, captureId: "menu" }, captures: [...f.driver.current().captures, capture("menu", "recording", [outcome("shortcut"), outcome("first-frame-start")])] }), true);
  assert.equal(f.controller.state.progress.stage, "practice-start"); assert.equal(f.controller.state.progress.attemptId, null);
  assert.equal(f.driver.publish({ ...f.driver.current(), revision: f.driver.current().revision + 1, nextShortcutAdmission: { state: "consumed", admissionId, captureId: "capture-unrelated" }, captures: f.driver.current().captures.map(c => c.captureId === "capture-unrelated" ? { ...c, delivery: "local-only", shortcutAdmissionId: admissionId } : c) }), true);
  assert.equal(f.controller.state.progress.stage, "practice-start", "an ID present before reservation cannot be adopted"); f.controller.dispose();
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
  await act(async () => { dom.window.document.querySelector<HTMLButtonElement>('[aria-label="purple pen"]')!.click(); });
  assert.equal(f.controller.state.snapshot?.pen.effective, "coral", "choosing a color is not a provider write");
  await act(async () => { [...dom.window.document.querySelectorAll("button")].find(b => b.textContent === "Use this pen")!.click(); await Promise.resolve(); });
  assert.equal(f.controller.state.snapshot?.pen.effective, "purple"); assert.equal(f.controller.state.progress.stage, "permissions");
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
  publish(f.driver, [capture("copy-retry", "saved", [outcome("save"), { kind: "copy", at: "2026-11-05T00:00:00.000Z", ok: false }], "artifact-copy")]);
  const result = await f.controller.retryPracticeCopy(); assert.equal(result?.status, "accepted"); assert.equal(f.driver.commands().at(-1)?.type, "copy-local"); f.controller.dispose();
});

test("mismatched Record response IDs cannot authorize progress", async () => {
  const f = fixture(); await f.controller.load(); await f.controller.dispatch("continue");
  f.client.execute = async command => ({ status: "applied", requestId: "retired-request", snapshot: f.driver.current() });
  assert.equal(await f.controller.selectPen("purple"), false); assert.equal(f.controller.state.command, "unknown"); assert.equal(f.controller.state.progress.stage, "pen");
  await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "pen");
  f.client.execute = async command => ({ status: "accepted", requestId: command.requestId, snapshot: f.driver.current() });
  await f.controller.dispatch("refresh"); assert.notEqual(f.controller.state.command, "unknown"); f.controller.dispose();
});

test("a response from a retired provider epoch cannot confirm a pen write", async () => {
  const f = fixture(); await f.controller.load(); await f.controller.dispatch("continue");
  let finish!: (result: any) => void, requestId = ""; f.client.execute = command => new Promise(resolve => { requestId = command.requestId; finish = resolve; });
  const selecting = f.controller.selectPen("purple"); await Promise.resolve();
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
  const f = fixture("unsubscribe-during-recording"), admissionId = "dispose-pending"; f.progress.value = { ...initialProgress(), status: "in-progress", stage: "practice-start", admissionId };
  const snapshot = f.driver.current(); f.driver.publish({ ...snapshot, revision: snapshot.revision + 1, nextShortcutAdmission: { state: "armed", admissionId, delivery: "local-only", expiresAt: "2026-11-05T00:05:00Z" } });
  await f.controller.load(); assert.equal(f.driver.captureContinues(), true); f.controller.dispose(); await Promise.resolve();
  assert.equal(f.driver.captureContinues(), true); assert.deepEqual(f.driver.commands().map(c => c.type), ["cancel-next-shortcut"]);
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

test("complete mounted branded lesson follows one local-only admitted capture to confirmed open", async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window }); Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
  Object.defineProperty(dom.window, "matchMedia", { configurable: true, value: () => ({ matches: false }) });
  let commandId = 0;
  f.client.execute = async command => {
    let current = f.driver.current();
    if (command.type === "select-pen") {
      current = { ...current, revision: current.revision + 1, pen: { ...current.pen, effective: command.value } };
      f.driver.publish(current); return { status: "applied", requestId: command.requestId, snapshot: current };
    }
    if (command.type === "arm-next-shortcut") {
      current = { ...current, revision: current.revision + 1, nextShortcutAdmission: { state: "armed", admissionId: command.admissionId, delivery: command.delivery, expiresAt: command.expiresAt } };
      f.driver.publish(current); return { status: "accepted", requestId: command.requestId, snapshot: current };
    }
    if (command.type === "cancel-next-shortcut") {
      current = { ...current, revision: current.revision + 1, nextShortcutAdmission: { state: "cancelled", admissionId: command.admissionId } };
      f.driver.publish(current); return { status: "applied", requestId: command.requestId, snapshot: current };
    }
    return { status: "accepted", requestId: command.requestId, snapshot: current };
  };
  const initialSnapshot = f.driver.current(); assert.equal(f.driver.publish({ ...initialSnapshot, revision: initialSnapshot.revision + 1, nextShortcutAdmission: { state: "idle" } }), true);
  await f.controller.load();
  let closed = 0, pasted = 0; const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => { root.render(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() { closed++; }, onManualPasteInstructions() { pasted++; } })); await new Promise(resolve => setTimeout(resolve, 0)); });
  const click = async (label: string) => act(async () => { const button = [...dom.window.document.querySelectorAll("button")].find(b => b.textContent === label || b.getAttribute("aria-label") === label); assert.ok(button, `missing button ${label}`); button.click(); await new Promise(resolve => setTimeout(resolve, 0)); });
  const publishSnapshot = async (update: (snapshot: ReturnType<typeof f.driver.current>) => ReturnType<typeof f.driver.current>) => act(async () => { const current = f.driver.current(); assert.equal(f.driver.publish({ ...update(current), revision: current.revision + 1 }), true); await new Promise(resolve => setTimeout(resolve, 0)); });
  await click("Get started"); assert.equal(f.controller.state.progress.stage, "pen");
  await click("purple pen"); assert.equal(f.driver.current().pen.effective, "coral", "selecting a visible pen color does not write it yet");
  await click("Use this pen"); assert.equal(f.controller.state.progress.stage, "pen", "confirmed choice waits for the donor hop to finish");
  const frogNode = dom.window.document.querySelector(".onboarding-pond-frog")!;
  const animationEnd = new dom.window.Event("animationend", { bubbles: true }); Object.defineProperty(animationEnd, "animationName", { value: "onboarding-pen-hop" });
  await act(async () => { frogNode.dispatchEvent(animationEnd); await new Promise(resolve => setTimeout(resolve, 0)); });
  assert.equal(f.controller.state.progress.stage, "permissions");
  await click("Continue"); assert.equal(f.controller.state.progress.stage, "shortcut");
  await click("Continue"); assert.equal(f.controller.state.progress.stage, "practice-start");
  const admissionId = f.controller.state.progress.admissionId!; assert.equal(f.driver.current().nextShortcutAdmission.state, "armed");
  const practice = capture("lesson-capture", "starting", [outcome("shortcut")], undefined, "local-only", admissionId);
  await publishSnapshot(s => ({ ...s, nextShortcutAdmission: { state: "consumed", admissionId, captureId: practice.captureId }, captures: [...s.captures, practice] }));
  assert.equal(f.controller.state.snapshot?.nextShortcutAdmission.state, "consumed"); assert.equal(f.controller.state.progress.admissionId, admissionId); assert.equal(f.controller.state.progress.status, "in-progress");
  assert.equal(f.controller.state.progress.stage, "screen"); assert.equal(f.controller.state.progress.attemptId, "lesson-capture");
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "starting", [outcome("shortcut"), outcome("picker")], undefined, "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.stage, "screen", "screen selection can precede the first frame");
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "recording", [outcome("shortcut"), outcome("picker"), outcome("first-frame-start")], undefined, "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.stage, "whiteboard");
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "recording", [outcome("shortcut"), outcome("picker"), outcome("first-frame-start"), outcome("whiteboard")], undefined, "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.stage, "board-ink");
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "recording", [outcome("shortcut"), outcome("picker"), outcome("first-frame-start"), outcome("whiteboard"), outcome("accepted-stroke")], undefined, "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.stage, "screen-ink");
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "recording", [outcome("shortcut"), outcome("picker"), outcome("first-frame-start"), outcome("whiteboard"), outcome("accepted-stroke"), outcome("accepted-stroke", "screen")], undefined, "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.stage, "stop");
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "processing", [outcome("shortcut"), outcome("picker"), outcome("first-frame-start"), outcome("whiteboard"), outcome("accepted-stroke"), outcome("accepted-stroke", "screen"), outcome("processing")], undefined, "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.stage, "preparing");
  const outcomes = [outcome("shortcut"), outcome("picker"), outcome("first-frame-start"), outcome("whiteboard"), outcome("accepted-stroke"), outcome("accepted-stroke", "screen"), outcome("processing"), outcome("save"), outcome("copy")];
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "saved", outcomes, "lesson-artifact", "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.stage, "copied");
  await click("How to paste"); assert.equal(f.controller.state.progress.stage, "paste");
  await click("Show paste instructions"); assert.equal(pasted, 1);
  await click("I pasted it"); assert.equal(f.controller.state.progress.stage, "locate"); assert.equal(f.controller.state.progress.manualPasteConfirmed, true);
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "saved", [...outcomes, { kind: "artifact-open", at: "2026-11-05T00:05:00.000Z", artifactId: "lesson-artifact", ok: true, evidence: "viewer-ack" }], "lesson-artifact", "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.status, "completed"); assert.equal(f.controller.state.progress.stage, "complete");
  await click("Finish setup"); assert.equal(closed, 1); assert.equal(f.controller.state.progress.attemptId, "lesson-capture");
  await act(async () => { root.unmount(); }); f.controller.dispose(); dom.window.close();
});

test("cancelled practice can reserve again after reconnect and deferral releases the pending scope", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" };
  const issued: string[] = [];
  const first = f.driver.current(); f.driver.publish({ ...first, revision: first.revision + 1, nextShortcutAdmission: { state: "idle" } });
  f.client.execute = async command => {
    issued.push(command.type);
    let current = f.driver.current();
    if (command.type === "arm-next-shortcut") {
      current = { ...current, revision: current.revision + 1, nextShortcutAdmission: { state: "armed", admissionId: command.admissionId, delivery: command.delivery, expiresAt: command.expiresAt } };
      f.driver.publish(current); return { status: "accepted", requestId: command.requestId, snapshot: current };
    }
    if (command.type === "cancel-next-shortcut") {
      current = { ...current, revision: current.revision + 1, nextShortcutAdmission: { state: "cancelled", admissionId: command.admissionId } };
      f.driver.publish(current); return { status: "applied", requestId: command.requestId, snapshot: current };
    }
    return { status: "accepted", requestId: command.requestId, snapshot: current };
  };
  await f.controller.load(); await f.controller.dispatch("continue"); const firstAdmission = f.controller.state.progress.admissionId!;
  assert.equal(f.controller.state.progress.stage, "practice-start");
  const admitted = capture("attempt-one", "recording", [outcome("shortcut")], undefined, "local-only", firstAdmission);
  let current = f.driver.current(); assert.equal(f.driver.publish({ ...current, revision: current.revision + 1, nextShortcutAdmission: { state: "consumed", admissionId: firstAdmission, captureId: admitted.captureId }, captures: [admitted] }), true);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(f.controller.state.snapshot?.nextShortcutAdmission.state, "consumed"); assert.equal(f.controller.state.progress.admissionId, firstAdmission);
  assert.equal(f.controller.state.progress.stage, "screen");
  current = f.driver.current(); f.driver.publish({ ...current, revision: current.revision + 1, captures: [{ ...admitted, phase: "cancelled" }] });
  await new Promise(resolve => setTimeout(resolve, 0)); assert.equal(f.controller.state.progress.stage, "practice-start"); assert.equal(f.controller.state.progress.attemptId, null); assert.equal(f.controller.state.progress.admissionId, null);
  current = f.driver.current(); assert.equal(f.driver.reconnect({ ...current, providerEpoch: "fixture-epoch-2", revision: 1, connection: "connected", nextShortcutAdmission: { state: "idle" }, captures: [] }), true);
  await f.controller.dispatch("continue"); const retryAdmission = f.controller.state.progress.admissionId!;
  assert.notEqual(retryAdmission, firstAdmission); assert.equal(f.controller.state.progress.stage, "practice-start");
  const deferred = await f.controller.dispatch("defer"); assert.equal(deferred.progress.status, "deferred"); assert.equal(f.driver.current().nextShortcutAdmission.state, "cancelled");
  assert.equal(issued.slice(-2).join(","), "arm-next-shortcut,cancel-next-shortcut"); assert.equal(issued.some(c => c === "stop-capture" || c === "cancel-capture"), false); f.controller.dispose();
});

test("progress load errors fail closed and retry reloads before enabling transitions", async () => {
  const f = fixture(); let unavailable = true; const read = f.progress.read.bind(f.progress);
  f.progress.read = async () => { if (unavailable) throw new Error("progress store unavailable"); return read(); };
  await f.controller.load(); assert.equal(f.controller.state.publication, "unknown"); assert.equal(f.controller.state.progress.stage, "welcome");
  await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "welcome");
  unavailable = false; await f.controller.dispatch("retry"); assert.equal(f.controller.state.publication, "confirmed"); await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "pen"); f.controller.dispose();
});

test("a late progress read after dispose cannot publish stale setup state", async () => {
  const f = fixture(); let finish!: (value: { revision: number; value: OnboardingProgress }) => void;
  f.progress.read = () => new Promise(resolve => { finish = resolve; });
  const loading = f.controller.load(); f.controller.dispose();
  finish({ revision: 9, value: { ...initialProgress(), status: "in-progress", stage: "locate", manualPasteConfirmed: true } });
  await loading; assert.equal(f.controller.state.progress.stage, "welcome"); assert.equal(f.controller.state.progress.status, "new");
});

test("accepted arm retains association until delayed armed and consumed snapshots", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" };
  const s = f.driver.current(); f.driver.publish({ ...s, revision: s.revision + 1, nextShortcutAdmission: { state: "idle" } });
  await f.controller.load(); await f.controller.dispatch("continue");
  const admissionId = f.controller.state.progress.admissionId!;
  assert.ok(admissionId); assert.equal(f.controller.state.progress.stage, "practice-start");
  assert.equal(f.driver.current().nextShortcutAdmission.state, "idle", "accepted dispatch alone is not armed");
  let current = f.driver.current(); f.driver.publish({ ...current, revision: current.revision + 1, nextShortcutAdmission: { state: "armed", admissionId, delivery: "local-only", expiresAt: new Date(Date.now() + 10000).toISOString() } });
  current = f.driver.current(); f.driver.publish({ ...current, revision: current.revision + 1, nextShortcutAdmission: { state: "consumed", admissionId, captureId: "delayed" }, captures: [capture("delayed", "starting", [outcome("shortcut")], undefined, "local-only", admissionId)] });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(f.controller.state.progress.attemptId, "delayed"); assert.equal(f.controller.state.progress.stage, "screen");
  f.controller.dispose();
});

test("deferral cannot hide a pending accepted reservation without terminal readback", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" };
  const s = f.driver.current(); f.driver.publish({ ...s, revision: s.revision + 1, nextShortcutAdmission: { state: "idle" } });
  await f.controller.load(); await f.controller.dispatch("continue");
  const admissionId = f.controller.state.progress.admissionId!;
  await f.controller.dispatch("defer"); assert.notEqual(f.controller.state.progress.status, "deferred");
  assert.match(f.controller.state.error ?? "", /could not be released/);
  const current = f.driver.current(); f.driver.publish({ ...current, revision: current.revision + 1, nextShortcutAdmission: { state: "cancelled", admissionId } });
  await new Promise(resolve => setTimeout(resolve, 0));
  await f.controller.dispatch("defer"); assert.equal(f.controller.state.progress.status, "deferred");
  f.controller.dispose();
});

test("refused reservation clears intent and permits another attempt", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" };
  const s = f.driver.current(); f.driver.publish({ ...s, revision: s.revision + 1, nextShortcutAdmission: { state: "idle" } });
  f.driver.setNextResult("refused"); await f.controller.load(); await f.controller.dispatch("continue");
  assert.equal(f.controller.state.progress.admissionId, null);
  await f.controller.dispatch("continue"); assert.ok(f.controller.state.progress.admissionId);
  f.controller.dispose();
});

test("disposal cancels a reservation armed after its accepted command", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" };
  let s = f.driver.current(); f.driver.publish({ ...s, revision: s.revision + 1, nextShortcutAdmission: { state: "idle" } });
  await f.controller.load(); await f.controller.dispatch("continue");
  const admissionId = f.controller.state.progress.admissionId!;
  f.controller.dispose(); s = f.driver.current();
  f.driver.publish({ ...s, revision: s.revision + 1, nextShortcutAdmission: { state: "armed", admissionId, delivery: "local-only", expiresAt: new Date(Date.now() + 10000).toISOString() } });
  await Promise.resolve(); assert.equal(f.driver.commands().at(-1)?.type, "cancel-next-shortcut");
  assert.ok(!f.driver.commands().some(c => c.type === "cancel-capture" || c.type === "stop-capture"));
  s = f.driver.current(); f.driver.publish({ ...s, revision: s.revision + 1, nextShortcutAdmission: { state: "cancelled", admissionId } });
});

test("an accepted pen choice waits for subsequent authoritative color readback", async () => {
  const f = fixture(); await f.controller.load(); await f.controller.dispatch("continue");
  assert.equal(await f.controller.selectPen("purple"), false);
  assert.equal(f.controller.penSelectionConfirmed("purple"), false);
  const s = f.driver.current(); f.driver.publish({ ...s, revision: s.revision + 1, pen: { ...s.pen, effective: "purple" } });
  assert.equal(f.controller.penSelectionConfirmed("purple"), true);
  await f.controller.completePenSelection(); assert.equal(f.controller.state.progress.stage, "permissions");
  f.controller.dispose();
});

test("mounted deferred setup exposes Resume without resetting its saved step", async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window }); Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
  f.progress.value = { ...initialProgress(), status: "deferred", stage: "permissions" };
  const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => { root.render(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() {} })); });
  await act(async () => { [...dom.window.document.querySelectorAll("button")].find(b => b.textContent === "Resume setup")!.click(); });
  assert.equal(f.controller.state.progress.status, "in-progress"); assert.equal(f.controller.state.progress.stage, "permissions");
  await act(async () => { root.unmount(); }); f.controller.dispose(); dom.window.close();
});
