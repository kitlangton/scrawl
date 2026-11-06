import assert from "node:assert/strict";
import test from "node:test";
import { decodeRecipientRenewalReply } from "../../src/state/renewal-owner.ts";

const bytes = (value: unknown) => new TextEncoder().encode(JSON.stringify(value));
test("renewal replies require exact read-only completion and matching process exit", () => {
  const reply = { protocol: 1, result: { scope: "read", state: "current" } };
  assert.deepEqual(decodeRecipientRenewalReply(bytes(reply), 0), { state: "current" });
  for (const value of [{ ...reply, extra: true }, { ...reply, protocol: 2 },
    { protocol: 1, result: { scope: "upload", state: "current" } },
    { protocol: 1, result: { ...reply.result, token: "secret" } }]) {
    assert.deepEqual(decodeRecipientRenewalReply(bytes(value), 0), { state: "state_unavailable" });
  }
  for (const exit of [1, -1, 42, NaN]) assert.deepEqual(decodeRecipientRenewalReply(bytes(reply), exit), { state: "state_unavailable" });
  const rejected = bytes({ protocol: 1, result: { scope: "read", state: "rejected" } });
  assert.deepEqual(decodeRecipientRenewalReply(rejected, 0, "reject_read"), { state: "rejected" });
  assert.deepEqual(decodeRecipientRenewalReply(rejected, 0), { state: "state_unavailable" });
  assert.deepEqual(decodeRecipientRenewalReply(bytes(reply), 0, "reject_read"), { state: "state_unavailable" });
});
test("renewal failures return only allowlisted categories, never writer diagnostics", () => {
  for (const message of ["auth_required", "offline", "cancelled", "invalid_server", "state_unavailable"]) {
    const value = bytes({ protocol: 1, error: { code: "operation_failed", message } });
    assert.deepEqual(decodeRecipientRenewalReply(value, 1), { state: message });
    assert.deepEqual(decodeRecipientRenewalReply(value, 0), { state: "state_unavailable" });
  }
  for (const message of ["secret-token", "auth_required: secret-token", "current"]) {
    assert.deepEqual(decodeRecipientRenewalReply(bytes({ protocol: 1, error: { code: "operation_failed", message } }), 1), { state: "state_unavailable" });
  }
  assert.deepEqual(decodeRecipientRenewalReply(bytes({ protocol: 1, error: { code: "cancelled", message: "offline" } }), 1), { state: "state_unavailable" });
});
test("renewal decoding rejects duplicate fields, oversized bytes and multiple replies", () => {
  for (const value of ['{"protocol":1,"protocol":1,"result":{"scope":"read","state":"current"}}',
    '{}\n{}', ' '.repeat(4097), '\ufffd']) assert.deepEqual(decodeRecipientRenewalReply(new TextEncoder().encode(value), 0), { state: "state_unavailable" });
});
