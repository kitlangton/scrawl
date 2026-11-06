import assert from "node:assert/strict";
import test from "node:test";
import { projectLocalStatus } from "../../src/cli/status.ts";

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
