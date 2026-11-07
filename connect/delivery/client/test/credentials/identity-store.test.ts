import test from "node:test";
import assert from "node:assert/strict";
import { access, chmod, lstat, mkdtemp, readFile, realpath, rename, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import type { FileHandle } from "node:fs/promises";
import { generateStoredIdentityRecord, StoredIdentityError } from "../../src/credentials/recipient-identity.ts";
import { publishLoginIdentity, type LoginIdentityResources } from "../../src/credentials/identity-store.ts";

const id = "a".repeat(32), server = "https://sync.example";
async function fixture(run: (f: { root: string; put(name: string, bytes: string | Uint8Array): Promise<void>;
  resources(overrides?: Partial<LoginIdentityResources>): LoginIdentityResources }) => Promise<void>) {
  const root = await mkdtemp(path.join(await realpath(os.tmpdir()), "connect-identity-store-")); await chmod(root, 0o700);
  const put = (name: string, bytes: string | Uint8Array) => writeFile(path.join(root, name), bytes, { mode: 0o600 });
  await put("read.login", JSON.stringify(id)); await put("upload.login", JSON.stringify(id));
  const resources = (overrides: Partial<LoginIdentityResources> = {}) => ({
    async checkAcl(handle: FileHandle) { await handle.close(); return { state: "accepted" as const }; },
    async lock(handle: FileHandle, { signal }: { signal?: AbortSignal }) {
      const initial = await handle.stat({ bigint: true }); await handle.close();
      const queueIdentity = `${initial.dev}:${initial.ino}`;
      return { state: "acquired" as const, lease: { signal: signal ?? new AbortController().signal, assertHeld() {}, async assertCurrent() {
        const current = await lstat(path.join(root, "queue.lock"), { bigint: true });
        if (`${current.dev}:${current.ino}` !== queueIdentity) throw new Error("queue_changed");
      }, async release() {} } };
    },
    async replaceFile(parent: FileHandle, source: string, destination: string, expected: { destination: unknown }, options?: { signal?: AbortSignal }) {
      try {
        if (options?.signal?.aborted) return { state: "refused" as const };
        if (expected.destination !== null || await access(path.join(root, destination)).then(() => true, () => false)) return { state: "destination_changed" as const };
        await rename(path.join(root, source), path.join(root, destination)); return { state: "replaced" as const };
      } finally { await parent.close(); }
    },
    ...overrides,
  }) as LoginIdentityResources;
  try { await run({ root, put, resources }); } finally { await rm(root, { recursive: true, force: true }); }
}

async function generated() { return generateStoredIdentityRecord(); }
const context = (scope: "read" | "upload" = "read") => ({ scope, approvedServer: server, loginId: id, cancellation: null });

test("publishes a copied private record once and returns only its public binding", async () => fixture(async f => {
  const proposal = await generated(), original = new Uint8Array(proposal.privateRecord);
  const result = await publishLoginIdentity(f.root, context(), proposal.privateRecord, f.resources());
  assert.equal(result.state, "published"); assert.equal(result.identityId, id);
  assert.deepEqual(proposal.privateRecord, original);
  assert.deepEqual((await readFile(path.join(f.root, `${id}.identity`))).toString(), new TextDecoder().decode(original));
  assert.equal("privateRecord" in result, false);
  proposal.privateRecord.fill(0);
}));

test("an existing valid identity wins over a different proposal and damaged records fail closed", async () => fixture(async f => {
  const winner = await generated(), proposal = await generated();
  await f.put(`${id}.identity`, winner.privateRecord);
  const result = await publishLoginIdentity(f.root, context(), proposal.privateRecord, f.resources());
  assert.equal(result.state, "existing"); assert.deepEqual(result.publicKey, winner.publicKey);
  assert.deepEqual(await readFile(path.join(f.root, `${id}.identity`)), Buffer.from(winner.privateRecord));
  await f.put(`${id}.identity`, "damaged");
  await assert.rejects(publishLoginIdentity(f.root, context(), proposal.privateRecord, f.resources()));
}));

test("login, cancellation and pending revocation fence both scopes", async () => fixture(async f => {
  const proposal = await generated();
  await assert.rejects(publishLoginIdentity(f.root, { ...context(), loginId: "b".repeat(32) }, proposal.privateRecord, f.resources()));
  await f.put("cancellation", JSON.stringify("changed"));
  await assert.rejects(publishLoginIdentity(f.root, context(), proposal.privateRecord, f.resources()));
  await f.put("cancellation", JSON.stringify(null)); await f.put("revocation-x.intent", "{}");
  await assert.rejects(publishLoginIdentity(f.root, context("upload"), proposal.privateRecord, f.resources()));
}));

test("expected-absent conflict does not retry and uncertain publication exposes no key", async () => fixture(async f => {
  const proposal = await generated(); let calls = 0;
  const conflict = f.resources({ async replaceFile(parent) { calls++; await parent.close(); return { state: "destination_changed" }; } });
  await assert.rejects(publishLoginIdentity(f.root, context("upload"), proposal.privateRecord, conflict)); assert.equal(calls, 1);
  const uncertain = f.resources({ async replaceFile(parent, source, destination) {
    calls++; await rename(path.join(f.root, source), path.join(f.root, destination)); await parent.close(); return { state: "publication_uncertain" };
  } });
  const result = await publishLoginIdentity(f.root, context("upload"), proposal.privateRecord, uncertain);
  assert.deepEqual(result, { state: "uncertain", identityId: id }); assert.equal("publicKey" in result, false);
}));

test("queue release failure after confirmed publication remains uncertain", async () => fixture(async f => {
  const proposal = await generated(); let held = false;
  const resources = f.resources({ async lock(handle, { signal }) { await handle.close(); held = true; return { state: "acquired", lease: { signal: signal ?? new AbortController().signal,
    assertHeld() { if (!held) throw new Error(); }, async assertCurrent() { if (!held) throw new Error(); }, async release() { held = false; throw new Error(); } } }; } });
  const result = await publishLoginIdentity(f.root, context(), proposal.privateRecord, resources);
  assert.deepEqual(result, { state: "uncertain", identityId: id });
}));

test("login, cancellation, queue inode and record changes during dispatch never return a public key", async () => fixture(async f => {
  const proposal = await generated();
  for (const mutation of ["login", "cancellation", "queue", "record"] as const) {
    const resources = f.resources({ async replaceFile(parent, source, destination) {
      try {
        await rename(path.join(f.root, source), path.join(f.root, destination));
        if (mutation === "login") await f.put("read.login", JSON.stringify("b".repeat(32)));
        if (mutation === "cancellation") await f.put("cancellation", JSON.stringify("changed"));
        if (mutation === "queue") {
          await rename(path.join(f.root, "queue.lock"), path.join(f.root, "queue.old"));
          await writeFile(path.join(f.root, "queue.lock"), "", { mode: 0o600 });
        }
        if (mutation === "record") await writeFile(path.join(f.root, destination), "changed", { mode: 0o600 });
        return { state: "replaced" as const };
      } finally { await parent.close(); }
    } });
    const result = await publishLoginIdentity(f.root, context(), proposal.privateRecord, resources);
    assert.deepEqual(result, { state: "uncertain", identityId: id });
    assert.equal("publicKey" in result, false);
    // Reset each synthetic transaction's login, cancellation and queue fence.
    await f.put("read.login", JSON.stringify(id)); await rm(path.join(f.root, "cancellation"), { force: true });
    await rm(path.join(f.root, "queue.lock"), { force: true }); await rm(path.join(f.root, "queue.old"), { force: true });
    await rm(path.join(f.root, `${id}.identity`), { force: true });
  }
}));

test("resource methods may live on the installed provider prototype", async () => fixture(async f => {
  const proposal = await generated();
  try {
    const inherited = Object.create(f.resources()) as LoginIdentityResources;
    assert.equal((await publishLoginIdentity(f.root, context(), proposal.privateRecord, inherited)).state, "published");
  } finally { proposal.privateRecord.fill(0); }
}));

test("valid legacy identity JSON whitespace and field order remain readable without replacement", async () => fixture(async f => {
  const winner = await generated(), proposal = await generated();
  try {
    const record = JSON.parse(new TextDecoder().decode(winner.privateRecord));
    const legacy = JSON.stringify({ signing: record.signing, age: record.age, version: record.version }, null, 2) + "\n";
    await f.put(`${id}.identity`, legacy);
    const result = await publishLoginIdentity(f.root, context(), proposal.privateRecord, f.resources());
    assert.equal(result.state, "existing"); assert.deepEqual(result.publicKey, winner.publicKey);
    assert.equal(await readFile(path.join(f.root, `${id}.identity`), "utf8"), legacy);
  } finally { winner.privateRecord.fill(0); proposal.privateRecord.fill(0); }
}));


test("pre-publication failures expose no resource exception content", async () => fixture(async f => {
  const proposal = await generated();
  try {
    await assert.rejects(publishLoginIdentity(f.root, context(), proposal.privateRecord, f.resources({
      async checkAcl(handle) { await handle.close(); throw new Error("private host diagnostic /secret/path"); },
    })), error => error instanceof StoredIdentityError && error.category === "encryption_identity_unavailable"
      && error.message === "encryption_identity_unavailable" && !String(error).includes("secret"));
  } finally { proposal.privateRecord.fill(0); }
}));
