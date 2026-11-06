import assert from "node:assert/strict";
import test from "node:test";
import { projectLocalStatus } from "../../src/cli/status.ts";
import { readLocalStatus } from "../../src/cli/status-state.ts";
import { mkdtemp, realpath, rm, writeFile, readFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import type { NamespaceResources } from "../../src/state/namespace.ts";

const base = () => ({
  server: "https://hosted.example", serverValid: true, hostedSync: true, cloudEnabled: true,
  uploadConsent: true, revocationPending: false, encryption: true, queuePath: "/private/queue",
  nowSeconds: 100, readGrant: null as null | Record<string, unknown>, uploadGrant: null as null | Record<string, unknown>, jobs: [] as Record<string, unknown>[],
});
const grant = (scope: "read" | "upload", extra: Record<string, unknown> = {}) => ({
  scope, server: "https://hosted.example", authRequired: false, hasRefresh: false,
  expiresAt: null, refreshExpiresAt: null, ...extra,
});

test("legacy and renewable credentials preserve donor usability and auth classifications", () => {
  const facts = base();
  facts.readGrant = grant("read", { expiresAt: 101 });
  facts.uploadGrant = grant("upload", { hasRefresh: true, expiresAt: 90, refreshExpiresAt: 200 });
  const result = projectLocalStatus(facts);
  assert.equal(result.readSignedIn, true);
  assert.deepEqual(result.readAuth, { status: "legacy_reapproval_on_expiry", expiresAt: 101, refreshExpiresAt: null, action: null });
  assert.equal(result.uploadSignedIn, true);
  assert.deepEqual(result.uploadAuth, { status: "renewal_due", expiresAt: 90, refreshExpiresAt: 200, action: null });
});

test("renewal threshold and expiry equality use donor inclusive comparisons", () => {
  const facts = base();
  facts.readGrant = grant("read", { hasRefresh: true, expiresAt: 130, refreshExpiresAt: 101 });
  facts.uploadGrant = grant("upload", { hasRefresh: true, expiresAt: 101, refreshExpiresAt: 200 });
  const result = projectLocalStatus(facts);
  assert.equal(result.readAuth.status, "renewal_due");
  assert.equal(result.uploadAuth.status, "renewal_due");
  assert.equal(result.uploadSignedIn, true);
  facts.readGrant = grant("read", { hasRefresh: true, expiresAt: 100, refreshExpiresAt: 100 });
  const expired = projectLocalStatus(facts);
  assert.equal(expired.readAuth.status, "auth_required");
  assert.equal(expired.readSignedIn, false);
});

test("signed-out, current renewable and terminal legacy grants retain exact auth shapes", () => {
  const facts = base();
  assert.deepEqual(projectLocalStatus(facts).readAuth, { status: "signed_out" });
  facts.readGrant = grant("read", { hasRefresh: true, expiresAt: 131, refreshExpiresAt: 200 });
  assert.deepEqual(projectLocalStatus(facts).readAuth, { status: "signed_in", expiresAt: 131, refreshExpiresAt: 200, action: null });
  facts.readGrant = grant("read", { expiresAt: 100 });
  facts.uploadGrant = grant("upload", { authRequired: true });
  const terminal = projectLocalStatus(facts);
  assert.equal(terminal.readSignedIn, false); assert.equal(terminal.uploadSignedIn, false);
  assert.deepEqual(terminal.readAuth, { status: "auth_required", expiresAt: 100, refreshExpiresAt: null,
    action: "Run apparent connect login --scope read for this server" });
  assert.equal(terminal.uploadAuth.action, "Run apparent connect login --scope upload --consent-upload for this server");
});

test("enabled retains all settings and revocation gates independently", () => {
  for (const key of ["revocationPending", "hostedSync", "cloudEnabled", "uploadConsent", "serverValid"] as const) {
    const facts = base();
    Object.assign(facts, { [key]: key === "revocationPending" ? true : false });
    assert.equal(projectLocalStatus(facts).enabled, false, key);
  }
});

test("scope or server mismatches are signed out and malformed facts expose only a fixed category", () => {
  const facts = base();
  facts.uploadGrant = { ...grant("upload"), server: "https://other.example" };
  const result = projectLocalStatus(facts);
  assert.equal(result.uploadSignedIn, false);
  assert.equal(result.uploadAuth.status, "signed_out");
  assert.throws(() => projectLocalStatus({ ...facts, readGrant: grant("upload") }), { message: "invalid_local_status_facts" });
});

test("projects only donor queue fields and returns detached immutable output", () => {
  const facts = base();
  const job = { id: "abc", bundle: "/bundle", status: "pending", encrypted: false, attempts: 2, nextAttempt: 140, error: "retry", privateToken: "secret", unknownPrivate: { value: 1 } };
  facts.jobs = [job];
  const result = projectLocalStatus(facts);
  assert.deepEqual(result.queue, [{ id: "abc", bundle: "/bundle", status: "pending", encrypted: false, attempts: 2, nextAttempt: 140, error: "retry", sizeFailure: null }]);
  assert.equal(Object.isFrozen(result), true);
  assert.equal(Object.isFrozen(result.queue[0]), true);
  assert.notEqual(result.queue[0], job);
  assert.equal(Reflect.set(result.queue[0], "status", "ready"), false);
});

test("state-backed status joins private grants and queue under one lease without JSON writes", async () => {
  const directory = await mkdtemp(path.join(await realpath(os.tmpdir()), "connect-status-"));
  let locks = 0, held = false;
  const resources: NamespaceResources = {
    async checkAcl(handle) { await handle.close(); return { state: "accepted" }; },
    async lock(handle) {
      await handle.close(); assert.equal(held, false); held = true; locks++;
      return { state: "acquired", lease: { signal: new AbortController().signal,
        assertHeld() { assert.equal(held, true); }, async release() { held = false; } } };
    },
  };
  try {
    const settings = base(), id = "a".repeat(32);
    const credential = JSON.stringify({ server: settings.server, scope: "read", token: "private-status-token", generation: "b".repeat(32) });
    const job = JSON.stringify({ id, bundle: "/synthetic.apparent", server: settings.server, generation: "b".repeat(32),
      created: 1, attempts: 2, nextAttempt: 101, status: "pending", unknownSecret: "not-status-output" });
    await writeFile(path.join(directory, "read.credential"), credential, { mode: 0o600 });
    await writeFile(path.join(directory, `${id}.job`), job, { mode: 0o600 });
    const result = await readLocalStatus(directory, settings, resources);
    assert.equal(locks, 1); assert.equal(held, false);
    assert.equal(result.enabled, true); assert.equal(result.readSignedIn, true); assert.equal(result.uploadSignedIn, false);
    assert.equal(result.readAuth.status, "legacy_reapproval_on_expiry");
    assert.equal(result.queue[0]!.id, id);
    assert.ok(!JSON.stringify(result).includes("private-status-token"));
    assert.ok(!JSON.stringify(result).includes("not-status-output"));
    assert.equal(await readFile(path.join(directory, "read.credential"), "utf8"), credential);
    assert.equal(await readFile(path.join(directory, `${id}.job`), "utf8"), job);
    await writeFile(path.join(directory, "revocation-test.intent"), "{}", { mode: 0o600 });
    assert.equal((await readLocalStatus(directory, settings, resources)).enabled, false);
    await writeFile(path.join(directory, `${id}.job`), "{", { mode: 0o600 });
    await assert.rejects(readLocalStatus(directory, settings, resources), { message: "local_status_unavailable" });
    assert.equal(held, false);
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test("status validates settings before admission and cancellation does not acquire a lease", async () => {
  let calls = 0;
  const resources = { async checkAcl() { calls++; throw Error("private diagnostic"); },
    async lock() { calls++; throw Error("private diagnostic"); } } as NamespaceResources;
  await assert.rejects(readLocalStatus("/not-opened", { ...base(), nowSeconds: NaN }, resources), { message: "invalid_local_status_facts" });
  const controller = new AbortController(); controller.abort();
  await assert.rejects(readLocalStatus("/not-opened", base(), resources, { signal: controller.signal }), { message: "cancelled" });
  assert.equal(calls, 0);
});
