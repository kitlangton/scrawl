import assert from "node:assert/strict";
import { constants } from "node:fs";
import { chmod, link, mkdir, mkdtemp, open, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { CredentialReadError, readStoredCredentialHandle, type CredentialReadResources } from "../../src/credentials/private-read.ts";

const server = "https://app.tryapparent.com/connect";
const record = () => ({ server, scope: "read", token: "private-access", generation: "a".repeat(32), authRequired: true });
const context = { scope: "read" as const, approvedServer: server };
const admitted: CredentialReadResources = { checkAcl: async handle => { await handle.close(); return { state: "accepted" }; } };
const category = (value: string) => (error: unknown) => error instanceof CredentialReadError && error.category === value;
async function fixture(run: (file: string) => Promise<void>) {
  const root = await mkdtemp(path.join(await realpath(tmpdir()), "apparent-credential-read-"));
  try { await chmod(root, 0o700); const file = path.join(root, "read.credential"); await writeFile(file, JSON.stringify(record()), { mode: 0o600 }); await run(file); }
  finally { await rm(root, { recursive: true, force: true }); }
}
const ownedHandle = (file: string) => open(file, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);

test("reads private legacy facts without clearing authRequired and consumes the handle", () => fixture(async file => {
  const handle = await ownedHandle(file);
  const value = await readStoredCredentialHandle(handle, context, admitted);
  assert.equal(value.token, record().token); assert.equal(value.authRequired, true);
  assert.equal(value.server, server); assert.equal(value.refreshToken, null);
  await assert.rejects(handle.stat());
}));

test("uses positional bounded reads regardless of the caller's file offset", () => fixture(async file => {
  const handle = await ownedHandle(file); await handle.read(Buffer.alloc(9), 0, 9, null);
  assert.equal((await readStoredCredentialHandle(handle, context, admitted)).token, record().token);
}));

test("data readers retain legacy hardlink compatibility without weakening private modes", () => fixture(async file => {
  const alias = `${file}.alias`; await link(file, alias);
  assert.equal((await readStoredCredentialHandle(await ownedHandle(alias), context, admitted)).token, record().token);
  await chmod(file, 0o640);
  await assert.rejects(readStoredCredentialHandle(await ownedHandle(alias), context, admitted), category("unsafe_file"));
}));

test("refuses non-files, shared modes and oversized files before ACL dispatch", () => fixture(async file => {
  let aclCalls = 0; const resources: CredentialReadResources = { checkAcl: async h => { aclCalls++; await h.close(); return { state: "accepted" }; } };
  await chmod(file, 0o644);
  const shared = await ownedHandle(file);
  await assert.rejects(readStoredCredentialHandle(shared, context, resources), category("unsafe_file")); await assert.rejects(shared.stat());
  await chmod(file, 0o600); await writeFile(file, Buffer.alloc(32769));
  await assert.rejects(readStoredCredentialHandle(await ownedHandle(file), context, resources), category("unsafe_file"));
  const directory = `${file}.dir`; await mkdir(directory);
  await assert.rejects(readStoredCredentialHandle(await ownedHandle(directory), context, resources), category("unsafe_file"));
  assert.equal(aclCalls, 0);
}));

test("native ACL refusal and failures expose no private input or native error", () => fixture(async file => {
  const refused: CredentialReadResources = { checkAcl: async h => { await h.close(); return { state: "refused" }; } };
  await assert.rejects(readStoredCredentialHandle(await ownedHandle(file), context, refused), category("unsafe_file"));
  const failure: CredentialReadResources = { checkAcl: async h => { await h.close(); throw new Error("private-access native diagnostics"); } };
  await assert.rejects(readStoredCredentialHandle(await ownedHandle(file), context, failure), error => {
    assert.equal(String(error), "CredentialReadError: unavailable"); return category("unavailable")(error);
  });
}));

test("strict decoding pins scope/server and rejects malformed or duplicate stored fields", () => fixture(async file => {
  for (const value of ["{", JSON.stringify({ ...record(), scope: "upload" }), JSON.stringify({ ...record(), server: "https://another.example" }), JSON.stringify(record()).replace('"scope":"read"', '"scope":"read","scope":"read"')]) {
    await writeFile(file, value);
    await assert.rejects(readStoredCredentialHandle(await ownedHandle(file), context, admitted), category("invalid_credential"));
  }
}));

test("cancellation before reading and during ACL verification consumes handles without returning credentials", () => fixture(async file => {
  const early = new AbortController(); early.abort();
  const handle = await ownedHandle(file);
  await assert.rejects(readStoredCredentialHandle(handle, context, admitted, { signal: early.signal }), category("cancelled")); await assert.rejects(handle.stat());
  const during = new AbortController(); const resources: CredentialReadResources = { checkAcl: async h => { await h.close(); during.abort(); return { state: "accepted" }; } };
  await assert.rejects(readStoredCredentialHandle(await ownedHandle(file), context, resources, { signal: during.signal }), category("cancelled"));
}));


test("accepts the exact byte boundary without reading past its bounded buffer", () => fixture(async file => {
  const json = JSON.stringify(record());
  await writeFile(file, json + " ".repeat(32768 - Buffer.byteLength(json)));
  assert.equal((await readStoredCredentialHandle(await ownedHandle(file), context, admitted)).token, record().token);
}));
