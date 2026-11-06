import assert from "node:assert/strict";
import test from "node:test";
import { createElement, type ButtonHTMLAttributes } from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { JSDOM } from "jsdom";
import { createRecordFixture } from "@apparent/record/fixtures";
import { initialProgress, type OnboardingProgress, type ProgressStore, type SaveResult } from "../dist/index.js";
import { onboardingContributionFactory, type OnboardingSurfaceSnapshot } from "../dist/index.js";

class Store implements ProgressStore {
  revision = 1;
  value = initialProgress();
  async read() { return { revision: this.revision, value: this.value }; }
  async compareAndSwap(expected: number | string, value: OnboardingProgress): Promise<SaveResult> {
    if (expected !== this.revision) return { status: "conflict", revision: this.revision, current: this.value };
    this.value = value;
    return { status: "saved", revision: ++this.revision };
  }
}
const Button = ({ children, ...props }: ButtonHTMLAttributes<HTMLButtonElement>) => createElement("button", props, children);
const setup = () => ({ progress: new Store(), record: createRecordFixture("reconnect-with-saved-history") });

test("factory observer reports immutable initial and authoritative states before Screen mounts", async () => {
  const f = setup(), seen: OnboardingSurfaceSnapshot[] = [];
  const hosted = await onboardingContributionFactory.create({ progress: f.progress, record: f.record.client, id: () => "observer", Button, close() {}, onSurfaceChange(snapshot) { seen.push(snapshot); } });
  assert.deepEqual(seen.map(x => x.kind), ["hidden"]);
  assert.ok(Object.isFrozen(seen[0]));
  assert.throws(() => { (seen[0] as { kind: string }).kind = "tour"; }, TypeError);
  await hosted.activation.activate({ reason: "resume" });
  assert.deepEqual(seen.map(x => x.kind), ["hidden", "tour"]);
  await hosted.activation.permissionRepair.activate(["camera"]);
  assert.deepEqual(seen.map(x => x.kind), ["hidden", "tour", "repair"]);
  await hosted.activation.permissionRepair.activate(["microphone"]); // Retarget is still the repair surface.
  assert.deepEqual(seen.map(x => x.kind), ["hidden", "tour", "repair"]);
  await hosted.activation.dispose();
  assert.deepEqual(seen.map(x => x.kind), ["hidden", "tour", "repair", "hidden"]);
});

test("completed and deferred first launch stays hidden; repair close reports only after host close", async () => {
  for (const status of ["completed", "deferred"] as const) {
    const f = setup();
    f.progress.value = status === "completed"
      ? { ...initialProgress(), status, stage: "complete" }
      : { ...initialProgress(), status, stage: "screen-ink" };
    const seen: string[] = [];
    let failClose = true, closed = 0;
    const hosted = await onboardingContributionFactory.create({ progress: f.progress, record: f.record.client, id: () => "hidden-repair", Button, close() { if (failClose) throw new Error("host unavailable"); closed++; }, onSurfaceChange: s => seen.push(s.kind) });
    await hosted.activation.activate({ reason: "firstLaunch" });
    assert.deepEqual(seen, ["hidden"]);
    await hosted.activation.permissionRepair.activate(["camera"]);
    assert.deepEqual(seen, ["hidden", "repair"]);
    const dom = new JSDOM("<div id='root'></div>");
    Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window });
    Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document });
    Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
    const root = createRoot(dom.window.document.getElementById("root")!);
    await act(async () => root.render(createElement(hosted.module.Screen)));
    await act(async () => [...dom.window.document.querySelectorAll("button")].find(b => b.textContent === "Later")!.click());
    assert.deepEqual(seen, ["hidden", "repair"]);
    assert.equal(closed, 0);
    failClose = false;
    await act(async () => [...dom.window.document.querySelectorAll("button")].find(b => b.textContent === "Try closing again")!.click());
    assert.deepEqual(seen, ["hidden", "repair", "hidden"]);
    assert.equal(closed, 1);
    assert.deepEqual(f.progress.value.status, status);
    await act(async () => root.unmount());
    await hosted.activation.dispose();
    dom.window.close();
  }
});

test("tour restoration, callback errors, unmount and stale repair completion preserve owner lifecycle", async () => {
  const f = setup(), dom = new JSDOM("<div id='root'></div>");
  Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window });
  Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document });
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
  const seen: string[] = [];
  const hosted = await onboardingContributionFactory.create({ progress: f.progress, record: f.record.client, id: () => "tour-repair", Button, close() {}, onSurfaceChange(snapshot) { seen.push(snapshot.kind); throw new Error("observer failure"); } });
  const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => root.render(createElement(hosted.module.Screen)));
  await act(async () => hosted.activation.activate({ reason: "resume" }));
  await act(async () => hosted.activation.permissionRepair.activate(["camera"]));
  await act(async () => [...dom.window.document.querySelectorAll("button")].find(b => b.textContent === "Later")!.click());
  assert.deepEqual(seen, ["hidden", "tour", "repair", "tour"]);
  await act(async () => hosted.activation.permissionRepair.activate(["microphone"]));
  assert.deepEqual(seen, ["hidden", "tour", "repair", "tour", "repair"]);
  await act(async () => root.unmount());
  assert.deepEqual(seen, ["hidden", "tour", "repair", "tour", "repair"]);
  await hosted.activation.dispose();
  assert.deepEqual(seen, ["hidden", "tour", "repair", "tour", "repair", "hidden"]);
  dom.window.close();
});
