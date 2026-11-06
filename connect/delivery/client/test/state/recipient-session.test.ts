import assert from "node:assert/strict";
import test from "node:test";
import { mkdtemp, mkdir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { tmpdir } from "node:os";
import { openRecipientReadSession, RecipientSessionError } from "../../src/state/recipient-session.ts";
import type { NamespaceResources } from "../../src/state/namespace.ts";
import { generateAgeIdentity } from "../../src/recipient/crypto.ts";
const server = "https://app.tryapparent.com/connect", id = "b".repeat(32);
const ports: NamespaceResources = {
  checkAcl: async h => { await h.close(); return { state: "accepted" }; },
  lock: async h => { await h.close(); let held = true; return { state: "acquired", lease: {
    signal: new AbortController().signal, assertHeld() { if (!held) throw new Error(); }, async release() { held = false; },
  } }; },
};
const category = (expected: string) => (error: unknown) => error instanceof RecipientSessionError && error.category === expected && error.message === expected;
async function fixture(run: (f: { root: string; state: string; put(name: string, value: unknown): Promise<void>; credential: Record<string, unknown>; key: string }) => Promise<void>) {
  const root = await mkdtemp(path.join(await realpath(tmpdir()), "connect-read-session-")), state = path.join(root, "state");
  await mkdir(state, { mode: 0o700 });
  const put = (name: string, value: unknown) => writeFile(path.join(state, name), JSON.stringify(value), { mode: 0o600 });
  const key = (await generateAgeIdentity()).identity;
  const credential = { server, scope: "read", token: "synthetic", generation: "a".repeat(32), refreshToken: "refresh", expiresAt: 200,
    refreshExpiresAt: 400, grantId: "reader", encryption_identity: id, authRequired: false };
  try { await put("read.credential", credential); await put(`${id}.identity`, { version: 1, age: key, signing: "01".repeat(32) }); await run({ root, state, put, credential, key }); }
  finally { await rm(root, { recursive: true, force: true }); }
}
const options = { nowSeconds: () => 100 };

test("loads the legacy identity file lazily and wipes caller-owned signing bytes on disposal", () => fixture(async f => {
  const session = await openRecipientReadSession(f.state, server, ports, options);
  assert.equal(session.credential.identity, undefined);
  const identity = await session.resolveIdentity();
  assert.equal(identity.age, f.key); assert.deepEqual(identity.signing, new Uint8Array(32).fill(1));
  assert.equal(await session.resolveIdentity(), identity);
  session.dispose(); assert.deepEqual(identity.signing, new Uint8Array(32));
  await assert.rejects(session.assertCurrent(), category("cancelled"));
  await assert.rejects(session.resolveIdentity(), category("cancelled"));
}));

test("legacy grants need no key until an encrypted manifest requests it", () => fixture(async f => {
  await f.put("read.credential", { server, scope: "read", token: "legacy", generation: "a".repeat(32) });
  const session = await openRecipientReadSession(f.state, server, ports, options);
  try { assert.equal(session.credential.grantId, ""); await assert.rejects(session.resolveIdentity(), category("encryption_identity_missing")); }
  finally { session.dispose(); }
}));

test("expiry and authRequired never initiate a renewal or use unusable credentials", () => fixture(async f => {
  for (const [changes, failure] of [[{ expiresAt: 100 }, "renewal_required"], [{ refreshExpiresAt: 100 }, "auth_required"], [{ authRequired: true }, "auth_required"]] as const) {
    await f.put("read.credential", { ...f.credential, ...changes });
    const before = await readFile(path.join(f.state, "read.credential"));
    await assert.rejects(openRecipientReadSession(f.state, server, ports, options), category(failure));
    assert.deepEqual(await readFile(path.join(f.state, "read.credential")), before);
  }
}));

test("missing, malformed and unsafe identity references never create or replace keys", () => fixture(async f => {
  for (const value of ["../outside", null, "UPPERCASE"]) {
    await f.put("read.credential", { ...f.credential, encryption_identity: value });
    const session = await openRecipientReadSession(f.state, server, ports, options);
    try { await assert.rejects(session.resolveIdentity(), category("encryption_identity_missing")); } finally { session.dispose(); }
  }
  await f.put("read.credential", f.credential); await f.put(`${id}.identity`, { version: 2, age: f.key, signing: "01".repeat(32) });
  const before = await readFile(path.join(f.state, `${id}.identity`));
  const session = await openRecipientReadSession(f.state, server, ports, options);
  try { await assert.rejects(session.resolveIdentity(), category("encryption_identity_unavailable")); } finally { session.dispose(); }
  assert.deepEqual(await readFile(path.join(f.state, `${id}.identity`)), before);
}));

test("local epoch and revocation changes prevent an admitted session from publishing", () => fixture(async f => {
  const session = await openRecipientReadSession(f.state, server, ports, options);
  try {
    await f.put("cancellation", "different epoch");
    await assert.rejects(session.assertCurrent(), category("stale_snapshot"));
    await f.put("revocation-pending.intent", false);
    await assert.rejects(session.assertCurrent(), category("revocation_pending"));
  } finally { session.dispose(); }
}));

test("invalid server and cancellation fail before touching local state", async () => {
  const resources: NamespaceResources = { checkAcl: async () => { throw new Error("must not run"); }, lock: async () => { throw new Error("must not run"); } };
  await assert.rejects(openRecipientReadSession("/not-read", "file:///private", resources), category("invalid_server"));
  const signal = new AbortController(); signal.abort();
  await assert.rejects(openRecipientReadSession("/not-read", server, resources, { signal: signal.signal }), category("cancelled"));
});
