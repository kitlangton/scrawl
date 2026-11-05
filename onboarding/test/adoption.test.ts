import assert from "node:assert/strict";
import test from "node:test";
import type { ConfigAdoptionResult, ConfigAdoptionSnapshot, LegacyAdoptionBackend, LegacyConfigValue, JsonValue } from "@apparent/app-contracts/host";
import { adoptOnboardingProgress, initialProgress, onboardingProgressCodec } from "../dist/index.js";

function fixture(snapshot: ConfigAdoptionSnapshot, result: ConfigAdoptionResult = { status: "saved", revision: 1 }) {
  const calls: { owner: string; expected: LegacyConfigValue; value: JsonValue }[] = [];
  const backend: Pick<LegacyAdoptionBackend, "readForAdoption" | "adoptLegacy"> = {
    async readForAdoption(owner) { assert.equal(owner, "onboarding"); return structuredClone(snapshot); },
    async adoptLegacy(owner, expected, value) { calls.push({ owner, expected, value }); return result; },
  };
  return { backend, calls };
}
const absent = (legacy: LegacyConfigValue): ConfigAdoptionSnapshot => ({ owner: { present: false, revision: 0, value: null }, legacy });

test("bootstrap interprets legacy resume/defer metadata and atomically compares its exact snapshot", async () => {
  const legacy: LegacyConfigValue = { present: true, value: { seen: false, deferred: true, resume: "Shortcut", tutorial: "watched", granted: ["microphone"], relaunchedAt: 123, reopenFix: false } };
  const f = fixture(absent(structuredClone(legacy)));
  const result = await adoptOnboardingProgress(f.backend);
  assert.equal(result.status, "ready");
  if (result.status !== "ready") return;
  assert.equal(result.source, "adopted"); assert.equal(result.revision, 1);
  assert.equal(result.value.status, "deferred"); assert.equal(result.value.stage, "shortcut");
  assert.deepEqual(result.value.legacy.granted, ["microphone"]); assert.equal(result.value.legacy.relaunchedAt, 123);
  assert.deepEqual(f.calls, [{ owner: "onboarding", expected: legacy, value: onboardingProgressCodec.encode(result.value) }]);
});

test("existing owner state wins over legacy data without any adoption write", async () => {
  const current = { ...initialProgress(), status: "in-progress" as const, stage: "paste" as const, manualPasteConfirmed: false };
  const f = fixture({ owner: { present: true, revision: 9, value: onboardingProgressCodec.encode(current) }, legacy: { present: true, value: { seen: true } } });
  assert.deepEqual(await adoptOnboardingProgress(f.backend), { status: "ready", source: "existing", revision: 9, value: current });
  assert.equal(f.calls.length, 0);
});

test("stored owner null stays distinct from an absent owner and never falls back to completed legacy", async () => {
  const f = fixture({ owner: { present: true, revision: 3, value: null }, legacy: { present: true, value: { seen: true } } });
  assert.deepEqual(await adoptOnboardingProgress(f.backend), { status: "ready", source: "existing", revision: 3, value: initialProgress() });
  assert.equal(f.calls.length, 0);
});

test("missing legacy and stored null retain different transaction compare conditions", async () => {
  for (const present of [false, true]) {
    const f = fixture(absent({ present, value: null }));
    const result = await adoptOnboardingProgress(f.backend);
    assert.equal(result.status, "ready"); assert.deepEqual(f.calls[0]?.expected, { present, value: null });
  }
});

test("legacy changes and concurrent owner adoption return conflict without hidden retry", async () => {
  for (const snapshot of [absent({ present: true, value: { resume: "Permissions" } }), { owner: { present: true, revision: 1, value: onboardingProgressCodec.encode(initialProgress()) }, legacy: { present: true, value: { seen: true } } }]) {
    const f = fixture(absent({ present: false, value: null }), { status: "conflict", snapshot });
    assert.deepEqual(await adoptOnboardingProgress(f.backend), { status: "conflict", snapshot });
    assert.equal(f.calls.length, 1);
  }
});

test("unknown or thrown publication never claims ready or retries a possibly committed adoption", async () => {
  for (const throws of [false, true]) {
    const f = fixture(absent({ present: false, value: null }), { status: "unknown", message: "synthetic private diagnostic" });
    let count = 0;
    const result = await adoptOnboardingProgress({ ...f.backend, async adoptLegacy(...args) { count++; if (throws) throw new Error("synthetic private diagnostic"); return f.backend.adoptLegacy(...args); } });
    assert.equal(result.status, "unknown"); assert.equal(count, 1); assert.ok(!JSON.stringify(result).includes("private diagnostic"));
  }
});

test("unreadable storage and unsupported progress prevent adoption writes", async () => {
  const f = fixture(absent({ present: false, value: null }));
  assert.equal((await adoptOnboardingProgress({ ...f.backend, async readForAdoption() { throw new Error("private path"); } })).status, "unknown");
  assert.equal(f.calls.length, 0);
  for (const snapshot of [absent({ present: true, value: { schema: 999 } }), { owner: { present: true, revision: 1, value: { schema: 999 } }, legacy: { present: false, value: null } }]) {
    const unsupported = fixture(snapshot);
    await assert.rejects(adoptOnboardingProgress(unsupported.backend), /Unsupported/);
    assert.equal(unsupported.calls.length, 0);
  }
});
