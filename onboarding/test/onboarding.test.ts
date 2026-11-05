import assert from "node:assert/strict";
import test from "node:test";
import { createElement, type ButtonHTMLAttributes } from "react";
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
  await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "permissions"); assert.match(f.controller.state.error ?? "", /readiness is not confirmed/); f.controller.dispose();
  const g = fixture("external-shortcut-save-failure"); g.progress.value = { ...initialProgress(), status: "in-progress", stage: "shortcut" }; await g.controller.load();
  await g.controller.dispatch("continue"); assert.equal(g.controller.state.progress.stage, "shortcut"); assert.match(g.controller.state.error ?? "", /effective registered shortcut/); g.controller.dispose();
});

test("only a same-capture shortcut and real first frame create the practice association", async () => {
  const f = fixture("unrelated-capture"); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "practice-start" }; await f.controller.load();
  await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "practice-start");
  const original = f.driver.current().captures;
  publish(f.driver, [...original, capture("menu", "recording", [outcome("first-frame-start")])]); await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "practice-start");
  publish(f.driver, [...original, capture("shortcut-no-frame", "starting", [outcome("shortcut")])]); await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "practice-start");
  publish(f.driver, [...original, capture("eligible", "recording", [outcome("shortcut"), outcome("first-frame-start")])]); await f.controller.dispatch("continue");
  assert.equal(f.controller.state.progress.stage, "screen"); assert.equal(f.controller.state.progress.attemptId, "eligible"); f.controller.dispose();
});

test("CAS conflict and unknown publication never overwrite or claim progress", async () => {
  const f = fixture(); await f.controller.load(); f.progress.conflict = true; await f.controller.dispatch("continue");
  assert.equal(f.controller.state.progress.stage, "welcome"); assert.match(f.controller.state.error ?? "", /changed elsewhere/); f.controller.dispose();
  const g = fixture(); await g.controller.load(); g.progress.unknown = true; await g.controller.dispatch("continue"); assert.equal(g.controller.state.publication, "unknown"); assert.equal(g.controller.state.progress.stage, "welcome"); g.controller.dispose();
});

test("capture facts drive saved, copy and manual paste; dispatch and foreign artifact cannot complete Locate", async () => {
  const f = fixture(); f.progress.value = { ...initialProgress(), status: "in-progress", stage: "practice-start" }; await f.controller.load();
  const outcomes: CaptureOutcome[] = [outcome("shortcut"), outcome("first-frame-start")];
  publish(f.driver, [capture("practice", "recording", outcomes)]); await f.controller.dispatch("continue");
  outcomes.push(outcome("picker", "screen")); publish(f.driver, [capture("practice", "recording", outcomes)]); await f.controller.dispatch("continue");
  outcomes.push(outcome("whiteboard")); publish(f.driver, [capture("practice", "recording", outcomes)]); await f.controller.dispatch("continue");
  outcomes.push(outcome("accepted-stroke", "whiteboard")); publish(f.driver, [capture("practice", "recording", outcomes)]); await f.controller.dispatch("continue");
  outcomes.push(outcome("accepted-stroke", "screen")); publish(f.driver, [capture("practice", "recording", outcomes)]); await f.controller.dispatch("continue");
  outcomes.push(outcome("processing")); publish(f.driver, [capture("practice", "processing", outcomes)]); await f.controller.dispatch("continue");
  outcomes.push(outcome("save"), outcome("copy")); publish(f.driver, [capture("practice", "saved", outcomes, "artifact-1")]); await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "copied");
  await f.controller.dispatch("continue"); await f.controller.dispatch("manual-paste-confirm"); assert.equal(f.controller.state.progress.stage, "locate");
  outcomes.push(outcome("open-dispatch")); publish(f.driver, [capture("practice", "saved", outcomes, "artifact-1")]); await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "locate");
  const foreignCapture = capture("foreign", "saved", [{ kind: "artifact-open", at: "2026-11-05T00:00:30.000Z", artifactId: "artifact-foreign", ok: true, evidence: "viewer-ack" }], "artifact-foreign");
  publish(f.driver, [capture("practice", "saved", outcomes, "artifact-1"), foreignCapture]); await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "locate"); assert.equal(hasConfirmedArtifactOpen(f.driver.current(), "foreign"), true);
  outcomes.push({ kind: "artifact-open", at: "2026-11-05T00:00:45.000Z", artifactId: "artifact-1", ok: false, reason: "refused" }); publish(f.driver, [capture("practice", "saved", outcomes, "artifact-1"), foreignCapture]); await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "locate");
  outcomes.push({ kind: "artifact-open", at: "2026-11-05T00:01:00.000Z", artifactId: "foreign", ok: true, evidence: "viewer-ack" } satisfies ArtifactOpenOutcome); publish(f.driver, [capture("practice", "saved", outcomes, "artifact-1")]); await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.stage, "locate");
  assert.equal(hasConfirmedArtifactOpen({ ...f.driver.current(), captures: [capture("practice", "saved", outcomes, "new-artifact")] }, "practice"), false);
  outcomes.push({ kind: "artifact-open", at: "2026-11-05T00:02:00.000Z", artifactId: "artifact-1", ok: true, evidence: "viewer-ack" } satisfies ArtifactOpenOutcome); publish(f.driver, [capture("practice", "saved", outcomes, "artifact-1")]); assert.equal(hasConfirmedArtifactOpen(f.driver.current(), "practice"), true); await f.controller.dispatch("continue"); assert.equal(f.controller.state.progress.status, "completed"); f.controller.dispose();
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
