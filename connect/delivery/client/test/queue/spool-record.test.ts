import assert from "node:assert/strict";
import { test } from "node:test";
import { decodeStoredEncryptedSpool } from "../../src/sender/spool-record.ts";
import type { Context } from "../../src/recipient/crypto.ts";

const context: Context = { origin: "https://connect.test", accountId: "acct", captureId: "0123456789abcdef0123456789abcdef", senderGrantId: "sender" };
const hash = "a".repeat(64);
const asset = (path: string, size = 1) => ({ path, size, sha256: hash });
const rootFiles = [asset("envelope.json"), asset("sealed/part-000000.age")];
const record = (files = rootFiles, ctx: unknown = context, grants: unknown = ["recipient"]) => ({ metadata: { context: ctx, recipientGrants: grants }, files });
const decode = (value: unknown, profile: "root" | "evidence-first-v1" = "root", ctx = context) => decodeStoredEncryptedSpool(new TextEncoder().encode(JSON.stringify(value)), { context: ctx, profile });
const rejected = (value: unknown, profile: "root" | "evidence-first-v1" = "root") => assert.throws(() => decode(value, profile), { message: "invalid_encrypted_spool" });

test("decodes the donor Rust root shape and detaches, freezes its DTO", () => {
  const input = record(); const result = decode(input);
  assert.deepEqual(result.metadata.recipientGrants, ["recipient"]);
  assert.deepEqual(result.files, rootFiles);
  assert.notEqual(result.files, input.files);
  assert.ok(Object.isFrozen(result) && Object.isFrozen(result.metadata) && Object.isFrozen(result.metadata.context));
  assert.ok(Object.isFrozen(result.files) && Object.isFrozen(result.files[0]));
});

test("accepts camelCase recipientGrants and grouped root receipt paths", () => {
  const grouped = ["evidence", "media"].flatMap(group => [asset(`${group}/envelope.json`), asset(`${group}/sealed/part-000000.age`)]);
  assert.equal(decode(record(grouped), "evidence-first-v1").files.length, 4);
  rejected({ metadata: { context, recipient_grants: ["r"] }, files: rootFiles });
});

test("pins context, and treats absent or null optional group as Rust None", () => {
  assert.equal(decode(record(rootFiles, { ...context, group: null })).metadata.context.group, undefined);
  assert.equal(decode(record(rootFiles, context)).metadata.context.group, undefined);
  rejected(record(rootFiles, { ...context, group: "evidence" }));
  for (const mismatch of [
    { ...context, origin: "https://other.test" }, { ...context, accountId: "other" },
    { ...context, captureId: "fedcba9876543210fedcba9876543210" }, { ...context, senderGrantId: "other" },
  ]) rejected(record(rootFiles, mismatch));
});

test("fails closed on unknown fields and duplicate JSON keys", () => {
  rejected({ ...record(), extra: 1 });
  rejected({ metadata: { ...record().metadata, extra: 1 }, files: rootFiles });
  const duplicate = new TextEncoder().encode('{"metadata":{"context":{"origin":"https://connect.test","accountId":"acct","captureId":"0123456789abcdef0123456789abcdef","senderGrantId":"sender","senderGrantId":"x"},"recipientGrants":[]},"files":[]}');
  assert.throws(() => decodeStoredEncryptedSpool(duplicate, { context, profile: "root" }), { message: "invalid_encrypted_spool" });
  // Unlike the receipt and metadata, Rust Asset has no deny_unknown_fields.
  assert.deepEqual(decode(record([{ ...rootFiles[0]!, future: "ignored" }, rootFiles[1]!])).files, rootFiles);
  assert.throws(() => decodeStoredEncryptedSpool(new TextEncoder().encode(JSON.stringify(record())),
    { context, profile: "unsupported" as "root" }), { message: "invalid_encrypted_spool" });
});

test("rejects duplicate, malformed, near-prefix, out-of-sequence, and oversized transport assets", () => {
  rejected(record([asset("envelope.json"), asset("sealed/part-000000.age"), asset("sealed/part-000000.age")]));
  rejected(record([asset("envelope.json"), asset("sealed/part-000001.age")]));
  rejected(record([asset("envelope.json"), asset("sealed/part-000000.age", 32 * 1024 * 1024 + 1)]));
  rejected(record([asset("envelope.json", 128 * 1024 + 1), asset("sealed/part-000000.age")]));
  rejected(record([asset("evidence/envelope.json"), asset("evidence/sealed/part-000000.age")]));
  const badGroup = [asset("evidence/envelope.json"), asset("evidence/sealed/part-000000.age"), asset("mediax/envelope.json"), asset("mediax/sealed/part-000000.age")];
  rejected(record(badGroup), "evidence-first-v1");
});

test("bounds recipient grant metadata without creating asset buffers", () => {
  rejected(record(rootFiles, context, ["same", "same"]));
  rejected(record(rootFiles, context, ["x".repeat(257)]));
  rejected(record(rootFiles, context, [""]));
});
