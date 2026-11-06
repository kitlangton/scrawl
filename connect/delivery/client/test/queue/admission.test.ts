import assert from "node:assert/strict";
import test from "node:test";
import { planAdmission, ADMISSION_MAX_JOBS, ADMISSION_RETENTION_SECONDS, type AdmissionInput } from "../../src/queue/admission.ts";
import { decodeJob, type Job } from "../../src/queue/job.ts";

const id = (c: string) => c.repeat(32);
const base = (extra: Partial<AdmissionInput> = {}): AdmissionInput => ({ intent: "automatic-private", bundle: "/synthetic/capture.apparent", server: "https://api.example.test", generation: id("b"), now: 1_000_000, retentionSeconds: ADMISSION_RETENTION_SECONDS, enabled: true, sharingEnabled: true, encryptionEnabled: true, candidateId: id("f"), jobs: [], ...extra });
function job(extra: Record<string, unknown> = {}): Job {
  return decodeJob(new TextEncoder().encode(JSON.stringify({ id: id("a"), bundle: "/synthetic/capture.apparent", server: "https://api.example.test", generation: id("b"), created: 1, attempts: 0, nextAttempt: 0, status: "pending", error: null, ...extra })));
}
const run = (input: AdmissionInput) => planAdmission(input);

test("automatic private reuses first matching retained record regardless of status, age, or generation", () => {
  const first = job({ status: "expired", created: 2, generation: id("c") });
  const second = job({ id: id("d"), status: "pending" });
  const result = run(base({ jobs: [first, second] }));
  assert.equal(result.kind, "reusable");
  if (result.kind === "reusable") assert.equal(result.id, first.id);
});

test("explicit private skips deduplication and proposes cancellation before prune and capacity", () => {
  const stale = job({ status: "failed", created: 1, futureLegacy: { kept: true } });
  const ready = job({ id: id("c"), status: "ready" });
  const input = base({ intent: "explicit-private", jobs: [stale, ready] });
  const result = run(input);
  assert.equal(result.kind, "admit");
  assert.deepEqual(result.proposals.map(p => p.kind), ["cancel", "prune", "prune"]);
  if (result.proposals[0]?.kind === "cancel") {
    assert.equal(result.proposals[0].job.status, "cancelled");
    assert.deepEqual(result.proposals[0].job.futureLegacy, { kept: true });
    assert.notEqual(result.proposals[0].job, stale);
  }
  assert.equal(stale.status, "failed");
});

test("public copy deduplicates only same-generation live retained public records and preserves private records", () => {
  const privateJob = job({ shared: false, publicCopy: false, generation: id("d") });
  const stalePublic = job({ id: id("c"), publicCopy: true, shared: true, generation: id("c") });
  const current = job({ id: id("e"), publicCopy: true, shared: true, generation: id("b"), status: "ready", created: 1_000_000 - ADMISSION_RETENTION_SECONDS });
  const result = run(base({ intent: "public-copy", jobs: [privateJob, stalePublic, current] }));
  assert.equal(result.kind, "reusable");
  if (result.kind === "reusable") assert.equal(result.id, current.id);
});

test("retention equality and future creation use saturating age; public-copy statuses exclude cancelled and expired", () => {
  const equal = job({ publicCopy: true, shared: true, created: 100, generation: id("b") });
  const cancelled = job({ id: id("c"), publicCopy: true, shared: true, status: "cancelled", created: 90 });
  const input = base({ intent: "public-copy", now: 200, retentionSeconds: 100, jobs: [equal, cancelled] });
  const result = run(input);
  assert.equal(result.kind, "reusable");
  if (result.kind === "reusable") assert.equal(result.id, equal.id);
  const futurePrivate = job({ created: 300, generation: id("c") });
  assert.equal(run(base({ now: 200, jobs: [futurePrivate] })).kind, "reusable");
});

test("prune precedes capacity; full preserves earlier explicit cancellations and prune proposals", () => {
  const expired = Array.from({ length: ADMISSION_MAX_JOBS }, (_, n) => job({ id: n === 0 ? id("a") : n.toString(16).padStart(32, "0"), created: 1, status: "pending" }));
  const result = run(base({ intent: "explicit-private", jobs: expired }));
  assert.equal(result.kind, "admit"); // pruning frees capacity
  assert.equal(result.proposals.filter(p => p.kind === "prune").length, ADMISSION_MAX_JOBS);
  const live = expired.map((j, n) => ({ ...j, shared: n === 0, bundle: n === 0 ? "/synthetic/capture.apparent" : `/synthetic/other-${n}.apparent`, created: 1_000_000 }));
  const full = run(base({ intent: "explicit-private", jobs: live }));
  assert.equal(full.kind, "full");
  assert.equal(full.proposals.filter(p => p.kind === "cancel").length, 1);
  assert.deepEqual(full.proposals.slice(1).map(p => p.kind), []);
});

test("explicit cancellation frees a nonshared slot before capacity is checked", () => {
  const jobs = Array.from({ length: ADMISSION_MAX_JOBS }, (_, n) => job({ id: n.toString(16).padStart(32, "0"),
    bundle: n === 0 ? "/synthetic/capture.apparent" : `/synthetic/other-${n}.apparent`, created: 1_000_000 }));
  const result = run(base({ intent: "explicit-private", jobs }));
  assert.equal(result.kind, "admit");
  assert.deepEqual(result.proposals.map(p => p.kind), ["cancel", "prune"]);
  assert.equal(jobs[0]!.status, "pending");
});

test("new record fields match donor private and public defaults with no encryption fallback", () => {
  const privateResult = run(base());
  assert.equal(privateResult.kind, "admit");
  if (privateResult.kind === "admit") assert.deepEqual([privateResult.job.status, privateResult.job.encrypted, privateResult.job.grouped, privateResult.job.encryption, privateResult.job.files, privateResult.job.attempts, privateResult.job.nextAttempt, privateResult.job.error, privateResult.job.sizeFailure], ["pending", true, true, null, [], 0, 0, null, undefined]);
  const publicResult = run(base({ intent: "public-copy", encryptionEnabled: true }));
  assert.equal(publicResult.kind, "admit");
  if (publicResult.kind === "admit") assert.deepEqual([publicResult.job.status, publicResult.job.encrypted, publicResult.job.grouped, publicResult.job.shared, publicResult.job.publicCopy], ["awaiting_share", false, false, true, true]);
});

test("malformed facts and blocked gates fail with fixed generic error and do not expose paths", () => {
  for (const input of [base({ enabled: false }), base({ intent: "public-copy", sharingEnabled: false }), base({ bundle: "/private/secret" }), base({ generation: "bad" }), base({ now: -1 })]) {
    assert.throws(() => run(input), { message: "invalid_queue_admission" });
  }
});

test("outputs are detached and inputs remain unchanged", () => {
  const original = job({ futureLegacy: { nested: [1] } });
  const result = run(base({ intent: "explicit-private", jobs: [original] }));
  assert.equal(original.status, "pending");
  assert.equal(result.kind, "admit");
  if (result.kind === "admit") {
    assert.equal(Object.isFrozen(result), true);
    assert.equal(Object.isFrozen(result.job.files), true);
    assert.equal(Reflect.set(result.job, "futureLegacy", null), false);
    assert.deepEqual(original.futureLegacy, { nested: [1] });
  }
  const cancellation = result.proposals.find(p => p.kind === "cancel");
  assert.ok(cancellation && Object.isFrozen(cancellation.job));
  assert.equal(Object.isFrozen(cancellation.job.futureLegacy), true);
  assert.equal(Object.isFrozen(original), false);
});

test("server and bundle syntax retain root/connect loopback compatibility without granting authority", () => {
  assert.equal(run(base({ server: "http://127.0.0.1:4567/connect" })).kind, "admit");
  for (const input of [base({ bundle: "/synthetic/../capture.apparent" }), base({ bundle: "/synthetic/.apparent" }),
    base({ server: "https://user:secret@api.example.test" }), base({ server: "https://api.example.test/connect-extra" }),
    base({ jobs: [job(), job()] }), base({ candidateId: id("a"), jobs: [job({ status: "expired" })] })]) {
    assert.throws(() => run(input), { message: "invalid_queue_admission" });
  }
});
