import assert from "node:assert/strict";
import test from "node:test";
import { decodeStoredIdentityRecord, validateStoredIdentityRecord, StoredIdentityError } from "../../src/credentials/recipient-identity.ts";
import { generateAgeIdentity } from "../../src/recipient/crypto.ts";

const seed = "01".repeat(32);
const encoder = new TextEncoder();
const bytes = (v: unknown) => encoder.encode(JSON.stringify(v));
const record = (age: string, signing = seed) => ({ version: 1, age, signing });
async function fixture() {
  const identity = await generateAgeIdentity();
  return { age: identity.identity, signing: seed };
}
function category(error: unknown, expected: string) {
  assert.ok(error instanceof StoredIdentityError);
  assert.equal(error.category, expected);
  assert.equal(error.message, expected);
}

function rejectsDecode(action: () => unknown) {
  let error: unknown;
  try { action(); } catch (caught) { error = caught; }
  category(error, "encryption_identity_unavailable");
}

test("decodes canonical wire and validates a synthetic classical identity", async () => {
  const f = await fixture();
  const decoded = decodeStoredIdentityRecord(bytes(record(f.age, f.signing)));
  assert.deepEqual(decoded, { version: 1, age: f.age, signing: f.signing });
  assert.equal(Object.isFrozen(decoded), true);
  const validated = await validateStoredIdentityRecord(decoded);
  assert.equal(validated.age, f.age);
  assert.deepEqual(validated.signing, new Uint8Array(32).fill(1));
  assert.notEqual(validated.signing, (await validateStoredIdentityRecord(decoded)).signing);
});

test("rejects wrong fields, versions, duplicate keys, oversized and invalid UTF-8 JSON", async () => {
  const f = await fixture();
  for (const value of [
    { ...record(f.age), extra: "x" }, { age: f.age, signing: seed }, { ...record(f.age), version: 2 },
    { ...record(f.age), version: "1" },
  ]) rejectsDecode(() => decodeStoredIdentityRecord(bytes(value)));
  rejectsDecode(() => decodeStoredIdentityRecord(encoder.encode(`{"version":1,"age":"${f.age}","age":"${f.age}","signing":"${seed}"}`)));
  rejectsDecode(() => decodeStoredIdentityRecord(new Uint8Array(4097)));
  rejectsDecode(() => decodeStoredIdentityRecord(new Uint8Array([0xff])));
});

test("rejects aliases, PQ variants, malformed age keys and noncanonical signing hex", async () => {
  const f = await fixture();
  for (const age of [f.age.toLowerCase(), ` ${f.age}`, `${f.age} `, `AGE-SECRET-KEY-PQ${"A".repeat(100)}`, f.age.slice(0, -1) + "I"]) {
    try { decodeStoredIdentityRecord(bytes(record(age))); assert.fail(`accepted alias: ${age.slice(0, 24)}`); } catch (error) { category(error, "encryption_identity_unavailable"); }
  }
  for (const signing of ["A".repeat(64), "0".repeat(62), "0".repeat(64) + "00"])
    rejectsDecode(() => decodeStoredIdentityRecord(bytes(record(f.age, signing))));
  const invalidChecksum = f.age.slice(0, -1) + (f.age.endsWith("A") ? "B" : "A");
  category(await validateStoredIdentityRecord(record(invalidChecksum)).catch(e => e), "encryption_identity_unavailable");
});

test("honors pre-aborted and in-flight cancellation without mutating the record", async () => {
  const f = await fixture();
  const input = record(f.age);
  const before = { ...input };
  const pre = new AbortController(); pre.abort();
  category(await validateStoredIdentityRecord(input, { signal: pre.signal }).catch(e => e), "cancelled");
  const controller = new AbortController();
  const pending = validateStoredIdentityRecord(input, { signal: controller.signal });
  controller.abort();
  category(await pending.catch(e => e), "cancelled");
  assert.deepEqual(input, before);
});

test("snapshots mutable fields before validation awaits", async () => {
  const f = await fixture();
  const input = record(f.age);
  const pending = validateStoredIdentityRecord(input);
  input.age = "AGE-SECRET-KEY-1" + "A".repeat(58);
  input.signing = "ff".repeat(32);
  const validated = await pending;
  assert.equal(validated.age, f.age);
  assert.deepEqual(validated.signing, new Uint8Array(32).fill(1));
  assert.equal(await validateStoredIdentityRecord(record(f.age, "00".repeat(32))).then(v => v.signing.every(b => b === 0)), true);
  const redacted = await validateStoredIdentityRecord(record("private-marker" )).catch(e => e as Error);
  assert.equal(redacted.message.includes("private-marker"), false);
});
