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
import type { RepairTimer } from "../dist/permissions/controller.js";
import type { HostedContribution } from "@apparent/app-contracts/host";

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
function mockPenHop(dom: JSDOM, decode: () => Promise<void> = async () => {}) {
  let start!: () => void, finish!: () => void;
  const started = new Promise<void>(resolve => { start = resolve; });
  const finished = new Promise<void>(resolve => { finish = resolve; });
  Object.defineProperty(dom.window.HTMLImageElement.prototype, "decode", { configurable: true, value: decode });
  Object.defineProperty(dom.window.HTMLImageElement.prototype, "getAnimations", { configurable: true, value(this: HTMLImageElement) { if (!this.classList.contains("onboarding-pond-frog")) return []; start(); return [{ animationName: "onboarding-pen-hop", finished }]; } });
  Object.defineProperty(dom.window, "matchMedia", { configurable: true, value: () => ({ matches: false }) });
  return { started, finish };
}
class FactoryTimer implements RepairTimer {
  entries = new Map<number, { callback: () => void; delay: number }>(); next = 0;
  set(callback: () => void, delay: number) { const id = ++this.next; this.entries.set(id, { callback, delay }); return id; }
  clear(handle: unknown) { this.entries.delete(handle as number); }
  fire(delay: number) { for (const [id, entry] of [...this.entries]) if (entry.delay === delay) { this.clear(id); entry.callback(); } }
}
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
  const replay = fixture(); replay.progress.value = { ...initialProgress(), status: "completed", stage: "complete", attemptId: "old", manualPasteConfirmed: true }; await replay.controller.load(); await replay.controller.dispatch("reopen"); assert.equal(replay.controller.state.progress.stage, "complete"); await replay.controller.dispatch("replay"); assert.equal(replay.controller.state.progress.stage, "welcome"); assert.equal(replay.controller.state.progress.attemptId, null); assert.equal(replay.controller.state.progress.manualPasteConfirmed, false); replay.controller.dispose();
});

test("host progress codec preserves owner metadata and rejects future schema", () => {
  const legacy = onboardingProgressCodec.decode({ seen: false, deferred: true, resume: "TryIt", tutorial: "practiced", relaunchedAt: 1700000000000, reopenFix: false, shortcutChosen: true, firstCaptureCompleted: true });
  assert.equal(legacy.status, "deferred"); assert.equal(legacy.stage, "practice-start");
  assert.equal(legacy.legacy.tutorial, "practiced"); assert.equal(legacy.legacy.relaunchedAt, 1700000000000); assert.equal(legacy.legacy.shortcutChosen, true); assert.equal(legacy.legacy.firstCaptureCompleted, true);
  const encoded = onboardingProgressCodec.encode(legacy); assert.notEqual(encoded, legacy);
  assert.throws(() => onboardingProgressCodec.decode({ schema: 3, status: "completed" }), /Unsupported onboarding progress schema/);
  const prior = onboardingProgressCodec.decode({ schema: 1, status: "completed", stage: "complete", attemptId: null, admissionId: null, practiceBaselineIds: [], legacy: initialProgress().legacy, manualPasteConfirmed: false });
  assert.equal(prior.setupCompleted, true, "schema-1 completion remains remembered during migration");
  const current = onboardingProgressCodec.decode({ ...initialProgress(), setupCompleted: false });
  assert.equal(current.setupCompleted, false);
  const accessor = { ...initialProgress() }; Object.defineProperty(accessor, "stage", { enumerable: true, get() { throw new Error("getter executed"); } });
  assert.throws(() => onboardingProgressCodec.encode(accessor), /accessors/);
});

test("setup completion requires this controller's exact observed physical shortcut trial and CAS confirmation", async () => {
  const f = fixture("shortcut-trial-idle");
  f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" };
  await f.controller.load();
  await f.controller.dispatch("continue");
  const request = f.driver.commands().find(c => c.type === "begin-shortcut-trial");
  assert.ok(request && request.type === "begin-shortcut-trial");
  assert.equal(f.controller.state.progress.setupCompleted, false, "dispatch acceptance is not a press and release");
  const base = f.driver.current();
  const observed = { state: "observed" as const, reservationId: request.reservationId, sourceEpoch: request.sourceEpoch, sourceRevision: request.sourceRevision, bindingId: request.bindingId, effectiveBinding: request.effectiveBinding, pressId: "physical-press-1", pressedAt: "2026-11-07T10:00:00.000Z", releasedAt: "2026-11-07T10:00:00.100Z" };
  assert.equal(f.driver.publish({ ...base, revision: base.revision + 1, shortcutTrial: observed }), true);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(f.controller.state.progress.setupCompleted, true);
  assert.equal(f.progress.value.setupCompleted, true, "the physical fact must be read back through CAS");
  f.controller.dispose();
});

test("first run with unavailable trial facts stays pending and issues no capture command", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" }; await f.controller.load();
  const markup = renderToStaticMarkup(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() {} }));
  assert.match(markup, /A physical shortcut check is not available yet/);
  await f.controller.dispatch("continue");
  assert.equal(f.controller.state.progress.setupCompleted, false);
  assert.equal(f.controller.state.progress.stage, "shortcut");
  assert.equal(f.driver.commands().some(c => c.type === "start-capture" || c.type === "arm-next-shortcut"), false);
  assert.match(f.controller.state.error ?? "", /physical shortcut trial is not available/);
  f.controller.dispose();
});

test("terminal trial rearm preserves its exact retained identity while using fresh current selectors", async () => {
  const f = fixture("shortcut-trial-terminal-rearm-available"); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" }; await f.controller.load();
  await f.controller.beginShortcutTrial();
  const command = f.driver.commands().find(c => c.type === "begin-shortcut-trial"); assert.ok(command && command.type === "begin-shortcut-trial");
  assert.deepEqual(command.replaces, { reservationId: "fixture-trial-reservation", sourceEpoch: "fixture-native-epoch", sourceRevision: 1, bindingId: "binding-fixture-1", effectiveBinding: "Ctrl+Shift+R" });
  assert.deepEqual({ sourceEpoch: command.sourceEpoch, sourceRevision: command.sourceRevision, bindingId: command.bindingId, effectiveBinding: command.effectiveBinding }, { sourceEpoch: "fixture-native-epoch-current", sourceRevision: 7, bindingId: "binding-current", effectiveBinding: "Alt+R" });
  assert.equal(f.progress.value.setupCompleted, false); f.controller.dispose();
});

test("unrelated, stale-epoch and unknown trial facts do not complete setup", async () => {
  for (const invalid of [
    { state: "observed" as const, reservationId: "someone-else", sourceEpoch: "fixture-native-epoch", sourceRevision: 1, bindingId: "binding-fixture-1", effectiveBinding: "Ctrl+Shift+R", pressId: "unrelated", pressedAt: "2026-11-07T10:00:00.000Z", releasedAt: "2026-11-07T10:00:00.100Z" },
  ]) {
    const f = fixture("shortcut-trial-idle"); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" }; await f.controller.load(); await f.controller.beginShortcutTrial();
    const base = f.driver.current(); f.driver.publish({ ...base, revision: base.revision + 1, shortcutTrial: invalid });
    await new Promise(resolve => setImmediate(resolve)); assert.equal(f.progress.value.setupCompleted, false); f.controller.dispose();
  }
  for (const scenario of ["shortcut-trial-stale-epoch", "shortcut-trial-unknown"] as const) {
    const f = fixture(scenario); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" }; await f.controller.load();
    await f.controller.beginShortcutTrial(); assert.equal(f.progress.value.setupCompleted, false); f.controller.dispose();
  }
});

test("setup remains pending when its CAS result is unknown, its binding changes, or its provider epoch changes", async () => {
  const unknown = fixture("shortcut-trial-idle"); unknown.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" }; await unknown.controller.load(); await unknown.controller.beginShortcutTrial();
  const request = unknown.driver.commands().find(c => c.type === "begin-shortcut-trial"); assert.ok(request && request.type === "begin-shortcut-trial");
  unknown.progress.unknown = true;
  let current = unknown.driver.current(); unknown.driver.publish({ ...current, revision: current.revision + 1, shortcutTrial: { state: "observed", reservationId: request.reservationId, sourceEpoch: request.sourceEpoch, sourceRevision: request.sourceRevision, bindingId: request.bindingId, effectiveBinding: request.effectiveBinding, pressId: "confirmed-physical", pressedAt: "2026-11-07T10:00:00.000Z", releasedAt: "2026-11-07T10:00:00.100Z" } });
  await new Promise(resolve => setImmediate(resolve)); assert.equal(unknown.controller.state.progress.setupCompleted, false); assert.equal(unknown.controller.state.publication, "unknown"); unknown.controller.dispose();

  const conflict = fixture("shortcut-trial-idle"); conflict.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" }; await conflict.controller.load(); await conflict.controller.beginShortcutTrial();
  const conflictCommand = conflict.driver.commands().find(c => c.type === "begin-shortcut-trial"); assert.ok(conflictCommand && conflictCommand.type === "begin-shortcut-trial"); conflict.progress.conflict = true;
  current = conflict.driver.current(); conflict.driver.publish({ ...current, revision: current.revision + 1, shortcutTrial: { state: "observed", reservationId: conflictCommand.reservationId, sourceEpoch: conflictCommand.sourceEpoch, sourceRevision: conflictCommand.sourceRevision, bindingId: conflictCommand.bindingId, effectiveBinding: conflictCommand.effectiveBinding, pressId: "conflict-physical", pressedAt: "2026-11-07T10:00:00.000Z", releasedAt: "2026-11-07T10:00:00.100Z" } });
  await new Promise(resolve => setImmediate(resolve)); assert.equal(conflict.progress.value.setupCompleted, false); assert.equal(conflict.controller.state.error?.includes("changed elsewhere"), true); conflict.controller.dispose();

  const changed = fixture("shortcut-trial-idle"); changed.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" }; await changed.controller.load(); await changed.controller.beginShortcutTrial();
  const trialCommand = changed.driver.commands().find(c => c.type === "begin-shortcut-trial"); assert.ok(trialCommand && trialCommand.type === "begin-shortcut-trial");
  current = changed.driver.current(); changed.driver.publish({ ...current, revision: current.revision + 1, shortcut: { ...current.shortcut, bindingId: "replacement-binding" }, shortcutTrial: { state: "observed", reservationId: trialCommand.reservationId, sourceEpoch: trialCommand.sourceEpoch, sourceRevision: trialCommand.sourceRevision, bindingId: trialCommand.bindingId, effectiveBinding: trialCommand.effectiveBinding, pressId: "old-binding-physical", pressedAt: "2026-11-07T10:00:00.000Z", releasedAt: "2026-11-07T10:00:00.100Z" } });
  await new Promise(resolve => setImmediate(resolve)); assert.equal(changed.progress.value.setupCompleted, false); changed.controller.dispose();

  const epoch = fixture("shortcut-trial-idle"); epoch.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" }; await epoch.controller.load(); await epoch.controller.beginShortcutTrial();
  current = epoch.driver.current(); assert.equal(epoch.driver.reconnect({ ...current, providerEpoch: "fixture-epoch-new", revision: 1, shortcutTrial: { state: "observed", reservationId: "replayed", sourceEpoch: "fixture-native-epoch", sourceRevision: 1, bindingId: "binding-fixture-1", effectiveBinding: "Ctrl+Shift+R", pressId: "old-epoch-press", pressedAt: "2026-11-07T10:00:00.000Z", releasedAt: "2026-11-07T10:00:00.100Z" } }), true);
  await new Promise(resolve => setImmediate(resolve)); assert.equal(epoch.progress.value.setupCompleted, false); epoch.controller.dispose();
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
  const hiddenTour = dom.window.document.querySelector("main.onboarding-shell");
  assert.ok(hiddenTour?.parentElement?.hasAttribute("hidden"), "the stable tour remains mounted while hidden");
  await act(async () => { await hosted.activation.activate({ reason: "resume" }); });
  assert.equal(f.progress.value.stage, "screen-ink"); assert.equal(f.progress.value.status, "in-progress");
  await act(async () => { await Promise.resolve(); });
  assert.equal(dom.window.document.querySelector("main.onboarding-shell")?.getAttribute("data-stage"), "screen-ink");
  await act(async () => { root.unmount(); }); dom.window.close();
  await hosted.activation.dispose(); await assert.rejects(hosted.activation.activate({ reason: "resume" }), /cannot be activated/);
});

test("completed setup returns through the owner entry callback; replay is explicit and retains completion", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "completed", stage: "complete", setupCompleted: true };
  let entries = 0;
  const hosted = await onboardingContributionFactory.create({ progress: f.progress, record: f.client, id: () => "return-entry", Button: TestButton, close() {}, openRecordingEntry: async () => { entries++; return entries > 1; } });
  await assert.rejects(hosted.activation.activate({ reason: "resume" }), /not opened/);
  assert.equal(entries, 1); assert.equal(hosted.module.id, "onboarding");
  await hosted.activation.replayTour();
  assert.equal(f.progress.value.setupCompleted, true, "explicit replay retains remembered setup completion");
  assert.equal(f.progress.value.stage, "welcome");
  await hosted.activation.activate({ reason: "resume" });
  assert.equal(entries, 2);
  assert.equal(f.progress.value.setupCompleted, true);
  await hosted.activation.dispose();
});

test("renders the actual React contribution with accessible controls and packed Record fixture pen choices", async () => {
  const f = fixture(); await f.controller.load();
  const welcome = renderToStaticMarkup(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() {} }));
  assert.match(welcome, /Welcome\./); assert.match(welcome, /Sign-in is not available yet/); assert.match(welcome, /alt="apparent"/); assert.match(welcome, /assets\/apparent-app-icon\.svg/);
  await f.controller.dispatch("continue");
  const html = renderToStaticMarkup(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() {} }));
  assert.match(html, /Pick your pen\./); for (const color of ["coral", "purple", "blue", "green"]) assert.match(html, new RegExp(`aria-label="${color} pen"`)); assert.equal((html.match(/<span aria-hidden="true" style="--pencil:/g) ?? []).length, 4); assert.doesNotMatch(html, /Choose a color for your pen|ink pen|laser pen|cursor pen|Frog guide|A quick setup/); assert.match(html, /Try later/); f.controller.dispose();
});

test("setup presentation uses donor header and truthful three-step progress labels", async () => {
  const f = fixture(); await f.controller.load();
  const render = () => renderToStaticMarkup(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() {} }));
  assert.match(render(), /apparent · setup/); assert.match(render(), /1 \/ 3/);
  f.progress.value = { ...initialProgress(), status: "in-progress", stage: "pen" }; await f.controller.load(); assert.match(render(), /Pick your pen\./); assert.doesNotMatch(render(), /1 \/ 3/);
  f.progress.value = { ...initialProgress(), status: "in-progress", stage: "permissions" }; await f.controller.load(); assert.match(render(), /2 \/ 3/);
  f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" }; await f.controller.load(); assert.match(render(), /Choose your shortcut\./); assert.match(render(), /3 \/ 3/);
  f.controller.dispose();
});

test("screen ink keeps the donor trackpad hint beside the semantic Shift guide", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "screen-ink" }; await f.controller.load();
  const html = renderToStaticMarkup(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() {} }));
  assert.match(html, /Hold Shift and move your pointer to draw/); assert.match(html, /Pressing down on your trackpad\?/); assert.match(html, /No need\. Just hold Shift and glide\./);
  f.controller.dispose();
});

test("mounted permission checklist stays compact and reports provider facts without false readiness", async () => {
  const f = fixture();
  f.progress.value = { ...initialProgress(), status: "in-progress", stage: "permissions" };
  const current = f.driver.current();
  f.driver.publish({ ...current, revision: current.revision + 1, readiness: { requirements: [
    { id: "screen-capture", required: true, status: "granted", usableNow: false, relaunchNeeded: true, allowedRepairActions: ["request-safe-relaunch"] },
    { id: "microphone", required: true, status: "denied", usableNow: false, relaunchNeeded: false, allowedRepairActions: ["open-system-settings"] },
    { id: "future-internal-id", required: false, status: "not-determined", usableNow: false, relaunchNeeded: false, allowedRepairActions: [] },
  ] } });
  await f.controller.load();
  const dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window });
  Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document });
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
  const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => { root.render(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() {} })); await Promise.resolve(); });
  const text = dom.window.document.body.textContent ?? "";
  assert.match(text, /Review recording access/); assert.match(text, /Screen Recording/); assert.match(text, /Reopen Apparent to use this access/);
  assert.match(text, /Microphone/); assert.match(text, /Not allowed/); assert.match(text, /Access/); assert.doesNotMatch(text, /screen-capture|future-internal-id|operating system remains/);
  const beforeUnknown = f.driver.current();
  f.driver.publish({ ...beforeUnknown, revision: beforeUnknown.revision + 1, readiness: { requirements: [{ id: "microphone", required: true, status: "unavailable", usableNow: false, relaunchNeeded: false, allowedRepairActions: [] }] } });
  await act(async () => { await Promise.resolve(); });
  assert.match(dom.window.document.body.textContent ?? "", /Status unknown/);
  assert.match(dom.window.document.body.textContent ?? "", /Access status is unknown/);
  assert.doesNotMatch(dom.window.document.body.textContent ?? "", /✓ Enabled/);
  const unavailable = f.driver.current();
  f.driver.publish({ ...unavailable, revision: unavailable.revision + 1, readiness: { requirements: [{ id: "microphone", required: true, status: "granted", usableNow: true, relaunchNeeded: false, allowedRepairActions: [] }] } });
  await act(async () => { await Promise.resolve(); });
  assert.match(dom.window.document.body.textContent ?? "", /✓ Enabled/);
  assert.doesNotMatch(dom.window.document.body.textContent ?? "", /Access status is unknown/);
  f.driver.disconnect();
  await act(async () => { await Promise.resolve(); });
  assert.match(dom.window.document.body.textContent ?? "", /Status unknown/);
  assert.doesNotMatch(dom.window.document.body.textContent ?? "", /✓ Enabled/);
  await act(async () => { root.unmount(); }); f.controller.dispose(); dom.window.close();
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
  await g.controller.dispatch("continue"); assert.equal(g.controller.state.progress.stage, "shortcut"); assert.match(g.controller.state.error ?? "", /physical shortcut trial is not available/); g.controller.dispose();
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
  assert.equal(f.controller.state.progress.stage, "pen", "the existing effective color does not confirm the page");
  await act(async () => { dom.window.document.querySelector<HTMLButtonElement>('[aria-label="purple pen"]')!.click(); });
  assert.equal(f.controller.state.snapshot?.pen.effective, "coral", "choosing a color is not a provider write");
  await act(async () => { [...dom.window.document.querySelectorAll("button")].find(b => b.textContent === "Use this pen")!.click(); await Promise.resolve(); });
  assert.equal(f.controller.state.snapshot?.pen.effective, "purple"); assert.equal(f.controller.state.progress.stage, "permissions");
  await act(async () => { await f.controller.dispatch("back"); await Promise.resolve(); });
  assert.equal(f.controller.state.progress.stage, "pen", "returning to a confirmed pen requires another explicit Use this pen action");
  assert.equal(dom.window.document.querySelector<HTMLButtonElement>('[aria-label="purple pen"]')!.disabled, false);
  await act(async () => { [...dom.window.document.querySelectorAll("button")].find(b => b.textContent === "Use this pen")!.click(); await Promise.resolve(); });
  assert.equal(f.controller.state.progress.stage, "permissions");
  await act(async () => { root.unmount(); }); f.controller.dispose(); dom.window.close();
});

test("setup Later stays in scrollable content and defers through its owner control", async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window }); Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
  f.progress.value = { ...initialProgress(), status: "in-progress", stage: "pen" }; await f.controller.load(); let closed = 0;
  const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => { root.render(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() { closed++; } })); await Promise.resolve(); });
  const later = [...dom.window.document.querySelectorAll("button")].find(button => button.textContent === "Try later")!;
  const section = dom.window.document.querySelector("main.onboarding-shell section")!;
  assert.ok(section.contains(later), "Later belongs to the pane that scrolls above the fixed setup footer");
  assert.ok(section.parentElement?.querySelector("footer"), "the setup navigation remains outside the scroll pane");
  await act(async () => { later.click(); await Promise.resolve(); });
  assert.equal(f.controller.state.progress.status, "deferred"); assert.equal(closed, 1);
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
  await act(async () => { [...dom.window.document.querySelectorAll("button")].find(b => /^(Choose|Change) shortcut$/.test(b.textContent ?? ""))!.click(); await Promise.resolve(); });
  assert.equal(f.driver.commands().at(-1)?.type, "shortcut-begin-edit");
  f.progress.value = { ...f.progress.value, stage: "screen-ink", attemptId: "cancelled-practice" };
  await act(async () => { await f.controller.load(); publish(f.driver, [capture("cancelled-practice", "cancelled", [])]); await new Promise(resolve => setTimeout(resolve, 0)); });
  assert.equal(f.controller.state.progress.stage, "practice-start"); assert.equal(f.controller.state.progress.attemptId, null);
  await act(async () => { root.unmount(); }); f.controller.dispose(); dom.window.close();
});

test("mounted shortcut recorder submits modifier and physical key names, then requires provider readback before save", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" }; await f.controller.load(); const initialEffective = f.controller.state.snapshot?.shortcut.effective;
  const dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" }); Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window }); Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
  const root = createRoot(dom.window.document.getElementById("root")!); let draft = null as null | { sessionId: string; value: string; error?: string }; let submitted = "", lastShortcutCommand = "";
  f.client.execute = async command => {
    const current = f.driver.current();
    if (command.type.startsWith("shortcut-")) lastShortcutCommand = command.type;
    if (command.type === "shortcut-begin-edit") draft = { sessionId: "edit-1", value: "" };
    if (command.type === "shortcut-submit") { draft = { sessionId: command.sessionId, value: command.value }; submitted = command.value; }
    if (command.type === "shortcut-cancel") draft = null;
    const shortcut = command.type === "shortcut-save" ? { ...current.shortcut, effective: "Ctrl+Shift+R", label: "Ctrl+Shift+R", registration: "registered" as const, draft: null } : { ...current.shortcut, draft };
    const updated = { ...current, revision: current.revision + 1, shortcut };
    f.driver.publish(updated);
    return { status: command.type === "shortcut-save" ? "applied" as const : "accepted" as const, requestId: command.requestId, snapshot: updated };
  };
  await act(async () => { root.render(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() {} })); await Promise.resolve(); });
  await act(async () => { [...dom.window.document.querySelectorAll("button")].find(b => /^(Choose|Change) shortcut$/.test(b.textContent ?? ""))!.click(); await Promise.resolve(); });
  const recorder = dom.window.document.querySelector<HTMLElement>("[aria-label='Shortcut recorder']")!;
  await act(async () => { recorder.dispatchEvent(new dom.window.KeyboardEvent("keydown", { key: "F1", code: "F1", bubbles: true, cancelable: true })); });
  assert.match(dom.window.document.querySelector('[role="alert"]')?.textContent ?? "", /Use a letter or number/);
  await act(async () => { recorder.dispatchEvent(new dom.window.KeyboardEvent("keydown", { key: "r", code: "KeyR", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true })); });
  assert.equal(dom.window.document.querySelector('[role="alert"]'), null);
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
  assert.equal(submitted, "Ctrl+Shift+R");
  assert.equal(f.controller.state.snapshot?.shortcut.effective, initialEffective); assert.equal(f.controller.state.progress.stage, "shortcut");
  assert.equal([...dom.window.document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Save shortcut")?.disabled, false);
  await act(async () => { [...dom.window.document.querySelectorAll("button")].find(b => b.textContent === "Save shortcut")!.click(); await Promise.resolve(); });
  assert.equal(f.controller.state.snapshot?.shortcut.effective, "Ctrl+Shift+R"); assert.equal(f.controller.state.snapshot?.shortcut.registration, "registered");
  assert.match(dom.window.document.querySelector(".onboarding-fact")?.textContent ?? "", /Current shortcut: Ctrl\+Shift\+R.*Ready to use\./s);
  assert.ok([...dom.window.document.querySelectorAll("button")].some(button => button.textContent === "Change shortcut"), "the saved effective shortcut is offered as a change, not presented as unselected");
  await act(async () => { [...dom.window.document.querySelectorAll("button")].find(b => /^(Choose|Change) shortcut$/.test(b.textContent ?? ""))!.click(); await Promise.resolve(); });
  await act(async () => { dom.window.document.querySelector<HTMLElement>("[aria-label='Shortcut recorder']")!.dispatchEvent(new dom.window.KeyboardEvent("keydown", { key: "Escape", code: "Escape", bubbles: true, cancelable: true })); await Promise.resolve(); });
  assert.equal(lastShortcutCommand, "shortcut-cancel"); assert.equal(f.driver.current().shortcut.draft, null);
  await act(async () => { root.unmount(); }); f.controller.dispose(); dom.window.close();
});

test("late pen animation completion after deferral cannot resume setup", async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window }); Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
  const hop = mockPenHop(dom);
  f.client.execute = async command => { const current = f.driver.current(); if (command.type !== "select-pen") return { status: "accepted", requestId: command.requestId, snapshot: current }; const updated = { ...current, revision: current.revision + 1, pen: { ...current.pen, effective: command.value } }; f.driver.publish(updated); return { status: "applied", requestId: command.requestId, snapshot: updated }; };
  await f.controller.load(); await f.controller.dispatch("continue"); const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => { root.render(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() {} })); await Promise.resolve(); });
  await act(async () => { dom.window.document.querySelector<HTMLButtonElement>('[aria-label="purple pen"]')!.click(); });
  await act(async () => { [...dom.window.document.querySelectorAll("button")].find(button => button.textContent === "Use this pen")!.click(); await Promise.resolve(); });
  await hop.started;
  await act(async () => { [...dom.window.document.querySelectorAll("button")].find(button => button.textContent === "Try later")!.click(); await Promise.resolve(); });
  await act(async () => { hop.finish(); await new Promise(resolve => setTimeout(resolve, 200)); });
  assert.equal(f.controller.state.progress.status, "deferred"); assert.equal(f.controller.state.progress.stage, "pen");
  await act(async () => { root.unmount(); }); f.controller.dispose(); dom.window.close();
});

test("pen animation decode failure is recoverable and confirmed hop completes after landing", async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window }); Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
  let decodes = 0; const hop = mockPenHop(dom, async () => { if (++decodes === 1) throw new Error("font/image decode failed"); });
  f.client.execute = async command => { const current = f.driver.current(); if (command.type !== "select-pen") return { status: "accepted", requestId: command.requestId, snapshot: current }; const updated = { ...current, revision: current.revision + 1, pen: { ...current.pen, effective: command.value } }; f.driver.publish(updated); return { status: "applied", requestId: command.requestId, snapshot: updated }; };
  await f.controller.load(); await f.controller.dispatch("continue"); const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => { root.render(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() {} })); await Promise.resolve(); });
  await act(async () => { dom.window.document.querySelector<HTMLButtonElement>('[aria-label="purple pen"]')!.click(); });
  const usePen = () => [...dom.window.document.querySelectorAll("button")].find(button => button.textContent === "Use this pen")!.click();
  await act(async () => { usePen(); await new Promise(resolve => setTimeout(resolve, 0)); });
  assert.match(dom.window.document.querySelector('[role="alert"]')?.textContent ?? "", /could not finish its jump/);
  await act(async () => { usePen(); await Promise.resolve(); }); await hop.started;
  await act(async () => { hop.finish(); await new Promise(resolve => setTimeout(resolve, 200)); });
  assert.equal(decodes, 2); assert.equal(f.controller.state.progress.stage, "permissions");
  await act(async () => { root.unmount(); }); f.controller.dispose(); dom.window.close();
});

test("unmount invalidates an unfinished pen hop", async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window }); Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
  const hop = mockPenHop(dom);
  f.client.execute = async command => { const current = f.driver.current(); if (command.type !== "select-pen") return { status: "accepted", requestId: command.requestId, snapshot: current }; const updated = { ...current, revision: current.revision + 1, pen: { ...current.pen, effective: command.value } }; f.driver.publish(updated); return { status: "applied", requestId: command.requestId, snapshot: updated }; };
  await f.controller.load(); await f.controller.dispatch("continue"); const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => { root.render(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() {} })); await Promise.resolve(); });
  await act(async () => { dom.window.document.querySelector<HTMLButtonElement>('[aria-label="purple pen"]')!.click(); });
  await act(async () => { [...dom.window.document.querySelectorAll("button")].find(button => button.textContent === "Use this pen")!.click(); await Promise.resolve(); });
  await hop.started; await act(async () => { root.unmount(); }); hop.finish(); await new Promise(resolve => setTimeout(resolve, 200));
  assert.equal(f.controller.state.progress.stage, "pen"); f.controller.dispose(); dom.window.close();
});

test("practice renderer is a compact coach in the owner screen without setup navigation chrome", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "screen" }; await f.controller.load();
  const html = renderToStaticMarkup(createElement(OnboardingContribution, { controller: f.controller, Button: TestButton, close() {} }));
  assert.match(html, /data-practice="true"/); assert.match(html, /class="onboarding-practice"/); assert.match(html, /class="onboarding-coach-frog"/); assert.doesNotMatch(html, /aria-label="Close setup"/); assert.match(html, /Try later/); f.controller.dispose();
});

test("armed practice says the effective shortcut once and keeps local-only and unarmed states clear", async () => {
  const armed = fixture(), admissionId = "practice-render-admission";
  armed.progress.value = { ...initialProgress(), status: "in-progress", stage: "practice-start", admissionId };
  const before = armed.driver.current();
  armed.driver.publish({ ...before, revision: before.revision + 1, shortcut: { ...before.shortcut, effective: "⌥⇧S", label: "⌥⇧S", registration: "registered" }, nextShortcutAdmission: { state: "armed", admissionId, delivery: "local-only", expiresAt: new Date(Date.now() + 60_000).toISOString() } });
  await armed.controller.load();
  const ready = renderToStaticMarkup(createElement(OnboardingContribution, { controller: armed.controller, Button: TestButton, close() {} }));
  assert.match(ready, /Press ⌥⇧S to start recording\./); assert.equal((ready.match(/⌥⇧S/g) ?? []).length, 1);
  assert.match(ready, /Local-only practice/); assert.doesNotMatch(ready, /Ready when you are|onboarding-waiting|Try again/); assert.match(ready, /Try later/);
  armed.controller.dispose();

  const preparing = fixture(); preparing.progress.value = { ...initialProgress(), status: "in-progress", stage: "practice-start", admissionId: "pending-admission" }; await preparing.controller.load();
  const preparingHtml = renderToStaticMarkup(createElement(OnboardingContribution, { controller: preparing.controller, Button: TestButton, close() {} }));
  assert.match(preparingHtml, /Preparing your local practice/); assert.doesNotMatch(preparingHtml, /Press .* to start recording/); preparing.controller.dispose();

  const ended = fixture(); ended.progress.value = { ...initialProgress(), status: "in-progress", stage: "practice-start" }; await ended.controller.load();
  const endedHtml = renderToStaticMarkup(createElement(OnboardingContribution, { controller: ended.controller, Button: TestButton, close() {} }));
  assert.match(endedHtml, /The previous practice ended/); assert.match(endedHtml, /Try again/); ended.controller.dispose();
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
  const f = fixture("shortcut-trial-idle"), dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window }); Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
  Object.defineProperty(dom.window, "matchMedia", { configurable: true, value: () => ({ matches: true }) });
  let commandId = 0;
  f.client.execute = async command => {
    let current = f.driver.current();
    if (command.type === "select-pen") {
      current = { ...current, revision: current.revision + 1, pen: { ...current.pen, effective: command.value } };
      f.driver.publish(current); return { status: "applied", requestId: command.requestId, snapshot: current };
    }
    if (command.type === "begin-shortcut-trial") {
      current = { ...current, revision: current.revision + 1, shortcutTrial: { state: "observed", reservationId: command.reservationId, sourceEpoch: command.sourceEpoch, sourceRevision: command.sourceRevision, bindingId: command.bindingId, effectiveBinding: command.effectiveBinding, pressId: "lesson-press", pressedAt: "2026-11-07T10:00:00.000Z", releasedAt: "2026-11-07T10:00:00.100Z" } };
      f.driver.publish(current); return { status: "accepted", requestId: command.requestId, snapshot: current };
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
  const title = (expected: string) => assert.equal(dom.window.document.querySelector("h1")?.textContent, expected);
  const publishSnapshot = async (update: (snapshot: ReturnType<typeof f.driver.current>) => ReturnType<typeof f.driver.current>) => act(async () => { const current = f.driver.current(); assert.equal(f.driver.publish({ ...update(current), revision: current.revision + 1 }), true); await new Promise(resolve => setTimeout(resolve, 0)); });
  await click("Choose a pen"); assert.equal(f.controller.state.progress.stage, "pen");
  await click("purple pen"); assert.equal(f.driver.current().pen.effective, "coral", "selecting a visible pen color does not write it yet");
  await click("Use this pen"); assert.equal(f.controller.state.progress.stage, "permissions", "reduced motion advances after confirmed provider readback");
  assert.equal(f.controller.state.progress.stage, "permissions");
  await click("Choose recording shortcut"); assert.equal(f.controller.state.progress.stage, "shortcut");
  await click("Try your shortcut"); assert.equal(f.controller.state.progress.setupCompleted, true);
  await click("Continue to practice"); assert.equal(f.controller.state.progress.stage, "practice-start");
  const admissionId = f.controller.state.progress.admissionId!; assert.equal(f.driver.current().nextShortcutAdmission.state, "armed");
  const practice = capture("lesson-capture", "starting", [outcome("shortcut")], undefined, "local-only", admissionId);
  await publishSnapshot(s => ({ ...s, nextShortcutAdmission: { state: "consumed", admissionId, captureId: practice.captureId }, captures: [...s.captures, practice] }));
  assert.equal(f.controller.state.snapshot?.nextShortcutAdmission.state, "consumed"); assert.equal(f.controller.state.progress.admissionId, admissionId); assert.equal(f.controller.state.progress.status, "in-progress");
  assert.equal(f.controller.state.progress.stage, "screen"); assert.equal(f.controller.state.progress.attemptId, "lesson-capture");
  title("Choose Screen.");
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "starting", [outcome("shortcut"), outcome("picker")], undefined, "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.stage, "screen", "screen selection can precede the first frame");
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "recording", [outcome("shortcut"), outcome("picker"), outcome("first-frame-start")], undefined, "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.stage, "whiteboard");
  title("Open Whiteboard.");
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "recording", [outcome("shortcut"), outcome("picker"), outcome("first-frame-start"), outcome("whiteboard")], undefined, "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.stage, "board-ink");
  title("Draw on the whiteboard.");
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "recording", [outcome("shortcut"), outcome("picker"), outcome("first-frame-start"), outcome("whiteboard"), outcome("accepted-stroke")], undefined, "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.stage, "note");
  title("Add a note.");
  const noteFact: CaptureOutcome = { kind: "note-save", ok: true, at: "2026-11-07T10:00:00Z", noteId: "lesson-note", receiptId: "lesson-receipt", screenStrokeBarrier: { historyId: "lesson-history", sequence: 4 } };
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "recording", [outcome("accepted-stroke"), noteFact], undefined, "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.stage, "screen-ink");
  title("Draw on your screen.");
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "recording", [outcome("accepted-stroke"), noteFact, { kind: "accepted-stroke", ok: true, at: "2026-11-07T10:00:01Z", surface: "screen", strokeId: "post-note", cursor: { historyId: "lesson-history", sequence: 5 } }], undefined, "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.stage, "stop");
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "processing", [outcome("shortcut"), outcome("picker"), outcome("first-frame-start"), outcome("whiteboard"), outcome("accepted-stroke"), outcome("accepted-stroke", "screen"), outcome("processing")], undefined, "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.stage, "preparing");
  title("Preparing your capture…");
  const outcomes = [outcome("shortcut"), outcome("picker"), outcome("first-frame-start"), outcome("whiteboard"), outcome("accepted-stroke"), outcome("accepted-stroke", "screen"), outcome("processing"), outcome("save"), outcome("copy")];
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "saved", outcomes, "lesson-artifact", "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.stage, "copied");
  title("Copied. Try pasting it.");
  await click("Show paste steps"); assert.equal(f.controller.state.progress.stage, "paste");
  await click("Show paste instructions"); assert.equal(pasted, 1);
  await click("I pasted it"); assert.equal(f.controller.state.progress.stage, "locate"); assert.equal(f.controller.state.progress.manualPasteConfirmed, true);
  title("apparent lives here.");
  await publishSnapshot(s => ({ ...s, captures: [capture("lesson-capture", "saved", [...outcomes, { kind: "artifact-open", at: "2026-11-05T00:05:00.000Z", artifactId: "lesson-artifact", ok: true, evidence: "viewer-ack" }], "lesson-artifact", "local-only", admissionId)] }));
  assert.equal(f.controller.state.progress.status, "completed"); assert.equal(f.controller.state.progress.stage, "complete");
  await click("Finish setup"); assert.equal(closed, 1); assert.equal(f.controller.state.progress.attemptId, "lesson-capture");
  await act(async () => { root.unmount(); }); f.controller.dispose(); dom.window.close();
});

test("cancelled practice can reserve again after reconnect and deferral releases the pending scope", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut", setupCompleted: true };
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
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut", setupCompleted: true };
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
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut", setupCompleted: true };
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
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut", setupCompleted: true };
  const s = f.driver.current(); f.driver.publish({ ...s, revision: s.revision + 1, nextShortcutAdmission: { state: "idle" } });
  f.driver.setNextResult("refused"); await f.controller.load(); await f.controller.dispatch("continue");
  assert.equal(f.controller.state.progress.admissionId, null);
  await f.controller.dispatch("continue"); assert.ok(f.controller.state.progress.admissionId);
  f.controller.dispose();
});

test("disposal cancels a reservation armed after its accepted command", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut", setupCompleted: true };
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

test('codec rejects malformed values and preserves legacy grant history without invoking getters', () => {
  for (const value of [true, 'future', 4, [], { unrelated: 1 }, { ...initialProgress(), status: 'completed', stage: 'welcome' }]) {
    assert.throws(() => onboardingProgressCodec.decode(value as any));
  }
  const legacy = onboardingProgressCodec.decode({ seen: false, deferred: true, resume: 'shortcut', granted: ['screen-recording', 'accessibility'] });
  assert.deepEqual(onboardingProgressCodec.decode(onboardingProgressCodec.encode(legacy)).legacy.granted, ['screen-recording', 'accessibility']);
  let accessed = false;
  const baseline: string[] = [];
  Object.defineProperty(baseline, '0', { enumerable: true, get() { accessed = true; return 'secret'; } });
  assert.throws(() => onboardingProgressCodec.encode({ ...initialProgress(), practiceBaselineIds: baseline }), /accessors/);
  assert.equal(accessed, false);
});

test('Settings displays activation failure instead of an unhandled rejection', async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: 'http://localhost' });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: dom.window }); Object.defineProperty(globalThis, 'document', { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', { configurable: true, value: true });
  f.progress.read = async () => { throw new Error('private synthetic storage detail'); };
  const hosted = await onboardingContributionFactory.create({ progress: f.progress, record: f.client, id: () => 'settings-error', Button: TestButton, close() {} });
  const root = createRoot(dom.window.document.getElementById('root')!);
  await act(async () => { root.render(createElement(hosted.module.Settings)); });
  await act(async () => { dom.window.document.querySelector<HTMLButtonElement>('button')!.click(); });
  assert.match(dom.window.document.querySelector('[role=alert]')?.textContent ?? '', /could not be opened/);
  assert.doesNotMatch(dom.window.document.body.textContent ?? '', /private synthetic/);
  await act(async () => root.unmount()); await hosted.activation.dispose(); f.controller.dispose(); dom.window.close();
});

test('failed host disposal is retryable and never permits activation or stops capture', async () => {
  const f = fixture('unsubscribe-during-recording'); let unavailable = true, hides = 0;
  const hosted = await onboardingContributionFactory.create({ progress: f.progress, record: f.client, id: () => 'cleanup', Button: TestButton, close() {}, coaching: {
    async show() { return { status: 'ok' }; }, async update() { return { status: 'ok' }; },
    async hide() { hides++; return unavailable ? { status: 'unavailable', capability: 'coaching.hide' } : { status: 'ok' }; },
  } });
  await assert.rejects(hosted.activation.dispose(), /could not be removed/);
  await assert.rejects(hosted.activation.activate({ reason: 'resume' }), /cannot be activated/);
  unavailable = false; await hosted.activation.dispose(); await hosted.activation.dispose();
  assert.equal(hides, 2); assert.equal(f.driver.captureContinues(), true); assert.deepEqual(f.driver.commands(), []); f.controller.dispose();
});

test('host activation waits for a pending owner write before reading progress again', async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: 'http://localhost' });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: dom.window }); Object.defineProperty(globalThis, 'document', { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', { configurable: true, value: true });
  let reads = 0; const read = f.progress.read.bind(f.progress); f.progress.read = async () => { reads++; return read(); };
  let finish!: () => void; const write = f.progress.compareAndSwap.bind(f.progress);
  f.progress.compareAndSwap = async (revision, value) => { await new Promise<void>(resolve => { finish = resolve; }); return write(revision, value); };
  const hosted = await onboardingContributionFactory.create({ progress: f.progress, record: f.client, id: () => 'write-race', Button: TestButton, close() {} });
  await hosted.activation.activate({ reason: 'firstLaunch' });
  const root = createRoot(dom.window.document.getElementById('root')!);
  await act(async () => root.render(createElement(hosted.module.Screen)));
  await act(async () => { [...dom.window.document.querySelectorAll('button')].find(b => b.textContent === 'Choose a pen')!.click(); });
  const activation = hosted.activation.activate({ reason: 'resume' }); await Promise.resolve();
  assert.equal(reads, 1);
  await act(async () => { finish(); await activation; });
  assert.equal(reads, 2); assert.equal(dom.window.document.querySelector('main')?.getAttribute('data-stage'), 'pen');
  await act(async () => root.unmount()); await hosted.activation.dispose(); f.controller.dispose(); dom.window.close();
});

test("one factory Screen switches tour to repair and back without changing tour state", async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: dom.window }); Object.defineProperty(globalThis, 'document', { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', { configurable: true, value: true });
  const timer = new FactoryTimer(); let closed = 0;
  const hosted = await onboardingContributionFactory.create({ progress: f.progress, record: f.client, id: () => "same-owner-id", Button: TestButton, close() { closed++; }, repairTimer: timer });
  await hosted.activation.activate({ reason: "resume" });
  const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => root.render(createElement(hosted.module.Screen)));
  assert.equal(dom.window.document.querySelector("main.onboarding-shell")?.getAttribute("data-stage"), "welcome");
  const before = structuredClone(f.progress.value);
  await act(async () => hosted.activation.permissionRepair.activate(["camera"]));
  assert.match(dom.window.document.querySelector("#permission-repair-title")?.textContent ?? "", /get access working/);
  assert.deepEqual(f.progress.value, before);
  await act(async () => { [...dom.window.document.querySelectorAll("button")].find(button => button.textContent === "Later")!.click(); await Promise.resolve(); });
  assert.equal(dom.window.document.querySelector("main.onboarding-shell")?.getAttribute("data-stage"), "welcome");
  assert.deepEqual(f.progress.value, before); assert.equal(closed, 0);
  await act(async () => root.unmount()); await hosted.activation.dispose(); dom.window.close();
});

test("completed and deferred repair preserves progress; hidden repair returns to host close", async () => {
  for (const status of ["completed", "deferred"] as const) {
    const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
    Object.defineProperty(globalThis, 'window', { configurable: true, value: dom.window }); Object.defineProperty(globalThis, 'document', { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', { configurable: true, value: true });
    f.progress.value = status === "completed"
      ? { ...initialProgress(), status, stage: "complete", attemptId: "saved", manualPasteConfirmed: true }
      : { ...initialProgress(), status, stage: "screen-ink", attemptId: "practice", manualPasteConfirmed: true };
    const before = structuredClone(f.progress.value); let closed = 0;
    const hosted = await onboardingContributionFactory.create({ progress: f.progress, record: f.client, id: () => "repair-only", Button: TestButton, close() { closed++; } });
    await hosted.activation.activate({ reason: "firstLaunch" });
    const root = createRoot(dom.window.document.getElementById("root")!);
    await act(async () => root.render(createElement(hosted.module.Screen)));
    await act(async () => hosted.activation.permissionRepair.activate(["camera"]));
    assert.deepEqual(f.progress.value, before);
    await act(async () => { [...dom.window.document.querySelectorAll("button")].find(button => button.textContent === "Later")!.click(); await Promise.resolve(); });
    assert.equal(closed, 1); assert.deepEqual(f.progress.value, before);
    assert.deepEqual(f.progress.value, before);
    await act(async () => root.unmount()); await hosted.activation.dispose(); dom.window.close();
  }
});

test("repair activation retargets, identical activation is idempotent, ready closes after 800ms, and unmount adds no late close", async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: dom.window }); Object.defineProperty(globalThis, 'document', { configurable: true, value: dom.window.document }); Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', { configurable: true, value: true });
  const timer = new FactoryTimer(); let closed = 0;
  const hosted = await onboardingContributionFactory.create({ progress: f.progress, record: f.client, id: () => "repair-timer", Button: TestButton, close() { closed++; }, repairTimer: timer });
  await hosted.activation.activate({ reason: "resume" });
  const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => root.render(createElement(hosted.module.Screen)));
  await act(async () => hosted.activation.permissionRepair.activate(["camera"]));
  await act(async () => hosted.activation.permissionRepair.activate(["camera"]));
  await act(async () => hosted.activation.permissionRepair.activate(["camera", "camera"]));
  assert.equal(timer.entries.size, 0);
  await act(async () => hosted.activation.permissionRepair.activate(["microphone"]));
  const current = f.driver.current();
  const ready = { id: "microphone", required: false, status: "granted" as const, usableNow: true, relaunchNeeded: false, allowedRepairActions: [] };
  assert.equal(f.driver.publish({ ...current, revision: current.revision + 1, readiness: { requirements: [ready] } }), true);
  assert.equal(timer.entries.size, 1); assert.equal([...timer.entries.values()][0].delay, 800);
  await act(async () => timer.fire(800)); await act(async () => { await Promise.resolve(); });
  assert.equal(dom.window.document.querySelector("main.onboarding-shell")?.getAttribute("data-stage"), "welcome");
  assert.equal(closed, 0);
  await act(async () => root.unmount()); timer.fire(800); assert.equal(closed, 0);
  await hosted.activation.dispose(); dom.window.close();
});

test("concrete owner repair capability stays owner-only while generic host shape remains compatible", async () => {
  const f = fixture();
  const concrete = await onboardingContributionFactory.create({ progress: f.progress, record: f.client, id: () => "typed-owner", Button: TestButton, close() {} });
  const generic: HostedContribution = concrete;
  assert.equal(typeof concrete.activation.permissionRepair.activate, "function");
  assert.equal(Object.keys(generic).sort().join(","), "activation,module");
  assert.equal(typeof generic.activation.activate, "function");
  await concrete.activation.dispose();
});

test('repair unmount cancels its timers without closing the host; remount resumes the request', async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: 'http://localhost' });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: dom.window }); Object.defineProperty(globalThis, 'document', { configurable: true, value: dom.window.document });
  const timer = new FactoryTimer(); let closes = 0;
  const hosted = await onboardingContributionFactory.create({ progress: f.progress, record: f.client, id: () => 'unmount', Button: TestButton, close() { closes++; }, repairTimer: timer });
  const current = f.driver.current(); f.driver.publish({ ...current, revision: current.revision + 1, readiness: { requirements: [{ id: 'microphone', required: true, status: 'granted', usableNow: true, relaunchNeeded: false, allowedRepairActions: [] }] } });
  await hosted.activation.permissionRepair.activate(['microphone']);
  const root = createRoot(dom.window.document.getElementById('root')!);
  await act(async () => root.render(createElement(hosted.module.Screen)));
  assert.equal(timer.entries.size, 1);
  await act(async () => root.render(null));
  assert.equal(timer.entries.size, 0); timer.fire(800); assert.equal(closes, 0);
  await act(async () => root.render(createElement(hosted.module.Screen)));
  assert.equal(timer.entries.size, 1);
  await act(async () => timer.fire(800));
  assert.equal(closes, 1);
  await act(async () => root.unmount()); await hosted.activation.dispose(); f.controller.dispose(); dom.window.close();
});

test('failed disposal clears repair timers before coaching cleanup and rejects every activation', async () => {
  const f = fixture('permission-granted-relaunch-needed'), timer = new FactoryTimer(); let unavailable = true;
  const hosted = await onboardingContributionFactory.create({ progress: f.progress, record: f.client, id: () => 'dispose-repair', Button: TestButton, close() {}, repairTimer: timer, coaching: {
    async show() { return { status: 'ok' }; }, async update() { return { status: 'ok' }; }, async hide() { return unavailable ? { status: 'unavailable', capability: 'hide' } : { status: 'ok' }; },
  } });
  await hosted.activation.permissionRepair.activate(['screen-capture']);
  assert.equal(timer.entries.size, 1);
  await assert.rejects(hosted.activation.dispose(), /could not be removed/);
  assert.equal(timer.entries.size, 0); timer.fire(1000); assert.deepEqual(f.driver.commands(), []);
  await assert.rejects(hosted.activation.permissionRepair.activate(['microphone']), /cannot be activated/);
  await assert.rejects(hosted.activation.activate({ reason: 'resume' }), /cannot be activated/);
  unavailable = false; await hosted.activation.dispose(); f.controller.dispose();
});

test('a queued old repair close cannot hide the next retargeted request', async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: 'http://localhost' });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: dom.window }); Object.defineProperty(globalThis, 'document', { configurable: true, value: dom.window.document });
  const hosted = await onboardingContributionFactory.create({ progress: f.progress, record: f.client, id: () => 'retarget-race', Button: TestButton, close() { assert.fail('must retain new repair'); } });
  await hosted.activation.permissionRepair.activate(['camera']);
  const root = createRoot(dom.window.document.getElementById('root')!);
  await act(async () => root.render(createElement(hosted.module.Screen)));
  await act(async () => {
    const retarget = hosted.activation.permissionRepair.activate(['microphone']);
    [...dom.window.document.querySelectorAll('button')].find(b => b.textContent === 'Later')!.click();
    await retarget;
  });
  assert.equal(dom.window.document.querySelector('.permission-repair-row strong')?.textContent, 'Microphone');
  await assert.rejects(hosted.activation.activate({ reason: 'resume' }), /during permission repair/);
  await act(async () => root.unmount()); await hosted.activation.dispose(); f.controller.dispose(); dom.window.close();
});

test('host close failure stays visible and retryable without an unhandled rejection', async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: 'http://localhost' });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: dom.window }); Object.defineProperty(globalThis, 'document', { configurable: true, value: dom.window.document });
  let fails = true, closes = 0, attempts = 0;
  const hosted = await onboardingContributionFactory.create({ progress: f.progress, record: f.client, id: () => 'close-error', Button: TestButton, close() { attempts++; if (fails) throw new Error('private native detail'); closes++; } });
  const root = createRoot(dom.window.document.getElementById('root')!);
  await act(async () => root.render(createElement(hosted.module.Screen)));
  await act(async () => hosted.activation.permissionRepair.activate(['camera']));
  await act(async () => [...dom.window.document.querySelectorAll('button')].find(b => b.textContent === 'Later')!.click());
  assert.match(dom.window.document.querySelector('[role=alert]')?.textContent ?? '', /could not be closed/);
  assert.doesNotMatch(dom.window.document.body.textContent ?? '', /private native/);
  await act(async () => { const current = f.driver.current(); f.driver.publish({ ...current, revision: current.revision + 1 }); });
  assert.equal(attempts, 1, 'provider polling must not retry host close');
  fails = false;
  await act(async () => [...dom.window.document.querySelectorAll('button')].find(b => b.textContent === 'Try closing again')!.click());
  assert.equal(closes, 1); assert.equal(dom.window.document.querySelector('[role=alert]'), null);
  await act(async () => root.unmount()); await hosted.activation.dispose(); f.controller.dispose(); dom.window.close();
});

test('the hidden tour keeps factual practice progress without stopping the associated capture', async () => {
  const f = fixture(), dom = new JSDOM("<div id='root'></div>", { url: 'http://localhost' });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: dom.window }); Object.defineProperty(globalThis, 'document', { configurable: true, value: dom.window.document });
  f.progress.value = { ...initialProgress(), status: 'in-progress', stage: 'stop', attemptId: 'practice-live' };
  publish(f.driver, [capture('practice-live', 'recording', [outcome('first-frame-start')], undefined, 'local-only')]);
  const hosted = await onboardingContributionFactory.create({ progress: f.progress, record: f.client, id: () => 'hidden-practice', Button: TestButton, close() {} });
  await hosted.activation.activate({ reason: 'resume' });
  const root = createRoot(dom.window.document.getElementById('root')!);
  await act(async () => root.render(createElement(hosted.module.Screen)));
  await act(async () => hosted.activation.permissionRepair.activate(['camera']));
  assert.equal([...dom.window.document.querySelectorAll('main')].filter(main => !main.closest('[hidden]')).length, 1);
  await act(async () => publish(f.driver, [capture('practice-live', 'saved', [outcome('processing'), outcome('save'), outcome('copy')], 'artifact-live', 'local-only')]));
  assert.equal(f.progress.value.stage, 'copied');
  assert.equal(f.progress.value.attemptId, 'practice-live'); assert.deepEqual(f.driver.commands(), []);
  await act(async () => [...dom.window.document.querySelectorAll('button')].find(b => b.textContent === 'Later')!.click());
  assert.equal(dom.window.document.querySelector('main[data-stage=copied]')?.parentElement?.hasAttribute('hidden'), false);
  await act(async () => root.unmount()); await hosted.activation.dispose(); f.controller.dispose(); dom.window.close();
});


test("note lesson rejects empty notes and earlier or foreign-history screen marks", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "note", attemptId: "noted" }; await f.controller.load();
  const note: CaptureOutcome = { kind: "note-save", ok: true, at: "2026-11-07T10:00:00Z", noteId: "note", receiptId: "receipt", screenStrokeBarrier: { historyId: "history", sequence: 4 } };
  const mark = (historyId: string, sequence: number): CaptureOutcome => ({ kind: "accepted-stroke", ok: true, at: "2026-11-07T10:00:01Z", surface: "screen", strokeId: `stroke-${historyId}-${sequence}`, cursor: { historyId, sequence } });
  publish(f.driver, [capture("noted", "recording", [{kind:"note-save",ok:false,at:"2026-11-07T10:00:00Z",reason:"empty"},mark("history",9)])]);
  await new Promise(r=>setImmediate(r)); assert.equal(f.controller.state.progress.stage,"note");
  publish(f.driver,[capture("noted","recording",[note,mark("history",4),mark("other",99)])]);
  await new Promise(r=>setImmediate(r)); assert.equal(f.controller.state.progress.stage,"screen-ink");
  publish(f.driver,[capture("noted","recording",[note,mark("history",5)])]);
  await new Promise(r=>setImmediate(r)); assert.equal(f.controller.state.progress.stage,"stop"); f.controller.dispose();
});

test("late observed trial cannot complete setup after navigation", async () => {
  const f=fixture("shortcut-trial-idle");f.progress.value={...initialProgress(),status:"in-progress",stage:"shortcut"};await f.controller.load();await f.controller.beginShortcutTrial();
  const cmd=f.driver.commands().find(c=>c.type==="begin-shortcut-trial");assert.ok(cmd&&cmd.type==="begin-shortcut-trial");
  await f.controller.dispatch("back");
  const current=f.driver.current();f.driver.publish({...current,revision:current.revision+1,shortcutTrial:{state:"observed",reservationId:cmd.reservationId,sourceEpoch:cmd.sourceEpoch,sourceRevision:cmd.sourceRevision,bindingId:cmd.bindingId,effectiveBinding:cmd.effectiveBinding,pressId:"late",pressedAt:"2026-11-07T10:00:00Z",releasedAt:"2026-11-07T10:00:01Z"}});
  await new Promise(r=>setImmediate(r));assert.equal(f.controller.state.progress.setupCompleted,false);f.controller.dispose();
});

test("replay with unknown persistence cannot publish a tour surface", async () => {
 const f=fixture();f.progress.value={...initialProgress(),status:"completed",stage:"complete",setupCompleted:true};const kinds:string[]=[];
 const hosted=await onboardingContributionFactory.create({progress:f.progress,record:f.client,id:()=>"replay-unknown",Button:TestButton,close(){},onSurfaceChange:s=>kinds.push(s.kind)});
 f.progress.unknown=true;await assert.rejects(hosted.activation.replayTour(),/could not be saved/);assert.deepEqual(kinds,["hidden"]);await hosted.activation.dispose();
});


test("restart and repeated return use the existing recording entry without onboarding writes or commands", async () => {
 const f=fixture();f.progress.value={...initialProgress(),status:"in-progress",stage:"shortcut",setupCompleted:true};let entries=0;const revision=f.progress.revision;
 for (const reason of ["firstLaunch","resume","resume"] as const) {
  const hosted=await onboardingContributionFactory.create({progress:f.progress,record:f.client,id:()=>"restart",Button:TestButton,close(){throw new Error("recording entry must not be closed");},openRecordingEntry:async()=>{entries++;return true;}});
  await hosted.activation.activate({reason});await hosted.activation.dispose();
 }
 assert.equal(entries,3);assert.equal(f.progress.revision,revision);assert.equal(f.progress.value.stage,"shortcut");assert.deepEqual(f.driver.commands(),[]);f.controller.dispose();
});


test("legacy donor successful trial is remembered without treating empty or future markers as completion", () => {
 const legacy={seen:false,resume:"TryIt",shortcutChosen:true,shortcutTestedFor:"Ctrl+Shift+R",tourRevision:1};
 assert.equal(onboardingProgressCodec.decode(legacy).setupCompleted,true);
 assert.equal(onboardingProgressCodec.decode({...legacy,shortcutTestedFor:""}).setupCompleted,false);
 assert.equal(onboardingProgressCodec.decode({...legacy,shortcutChosen:false}).setupCompleted,false);
 assert.equal(onboardingProgressCodec.decode({...legacy,tourRevision:2}).setupCompleted,false);
});
