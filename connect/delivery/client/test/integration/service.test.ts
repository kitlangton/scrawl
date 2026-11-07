import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { createServer, type Server } from "node:http";
import { once } from "node:events";
import { resolve } from "node:path";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import * as ed25519 from "@noble/ed25519";
import { generateStoredIdentityRecord, decodeStoredIdentityRecord, validateStoredIdentityRecord } from "../../src/credentials/recipient-identity.ts";
import type { PeerAuthorityKey } from "../../src/recipient/peer-authority.ts";
import { retrieveRecipientCapture } from "../../src/recipient/retrieve.ts";
import { RecipientTransportError } from "../../src/recipient/transport.ts";
import { uploadPreparedPlainCapture } from "../../src/sender/transport.ts";
import { uploadPreparedEncryptedCapture } from "../../src/sender/encrypted-transport.ts";
import { readSenderPeerAuthority } from "../../src/sender/authority.ts";
import { revokeDeviceGrant } from "../../src/account/revoke.ts";
import { decodeStoredCredential } from "../../src/credentials/wire.ts";
import { createDeviceLoginTransport } from "../../dist/account/device-session.js";
import { authorizeDevice } from "../../dist/device-flow.cjs";
import type { StoredCredential } from "../../src/credentials/wire.ts";
import { readAccountStatus } from "../../src/account/status.ts";
import { listShares, revokeShare } from "../../src/account/shares.ts";
import { sealCapture } from "../../src/sender/seal.ts";

const serviceDir = resolve(fileURLToPath(new URL("../../../../service/", import.meta.url)));
const captureId = "a981426be1254f37938214f960aed501";
const encoder = new TextEncoder();
const digest = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

type Account = {
  id: string; token: string; read: string; cookie: string; csrf: string; origin: string;
};
type DeviceCredential = { accessToken: string; grantId: string; scope: string; refreshToken: string; expiresIn: number; refreshExpiresIn: number };

async function fixtureAt(serviceOrigin: string) {
  const cwd = process.cwd();
  process.chdir(serviceDir);
  try {
    // The helper is JavaScript supplied by the service test fixture; this file
    // remains TypeScript and Node 22 strips its types when run directly.
    // @ts-expect-error service fixture intentionally has no declaration file
    const { fixture } = await import("../../../../service/test/helpers.mjs");
    const service = await fixture({ API_ORIGIN: serviceOrigin, SITE_ORIGIN: "https://site.example", SHARING_ENABLED: "true" });
    try {
      // Same additive sharing schema used by the service sharing fixtures.
      await service.db.exec((await readFile("migrations/0003.sql", "utf8")).replaceAll("\n", " "));
      return service;
    } catch (error) { await service.close(); throw error; }
  } finally {
    process.chdir(cwd);
  }
}

async function proxy(
  mount: "" | "/connect",
  run: (context: {
    origin: string;
    service: Awaited<ReturnType<typeof fixtureAt>>;
    request(path: string, init?: RequestInit): Promise<Response>;
  }) => Promise<void>,
) {
  let service: Awaited<ReturnType<typeof fixtureAt>> | undefined;
  const http: Server = createServer(async (request, response) => {
    try {
      let path = request.url ?? "/";
      if (mount) {
        if (!path.startsWith(mount + "/")) { response.writeHead(404).end(); return; }
        path = path.slice(mount.length);
      } else if (path.startsWith("/connect")) { response.writeHead(404).end(); return; }
      const headers = Object.fromEntries(Object.entries(request.headers).filter(([, value]) => value !== undefined)
        .map(([name, value]) => [name, Array.isArray(value) ? value.join(", ") : value]));
      const chunks: Buffer[] = [];
      for await (const chunk of request) chunks.push(Buffer.from(chunk));
      const result = await service!.call(path, {
        method: request.method,
        headers,
        ...(chunks.length ? { raw: Buffer.concat(chunks) } : {}),
      });
      response.writeHead(result.status, Object.fromEntries(result.headers));
      response.end(Buffer.from(await result.arrayBuffer()));
    } catch {
      response.writeHead(500).end("Local Worker proxy failed");
    }
  });
  http.listen(0, "127.0.0.1");
  await once(http, "listening");
  const address = http.address();
  assert(address && typeof address !== "string");
  const rootOrigin = `http://127.0.0.1:${address.port}`;
  try {
    service = await fixtureAt(rootOrigin + mount);
    await run({
      origin: rootOrigin,
      service,
      request: (path, init = {}) => fetch(`${rootOrigin}${path}`, init),
    });
  } finally {
    await service?.close();
    http.close();
    await once(http, "close");
  }
}

async function authorize(
  service: Awaited<ReturnType<typeof fixtureAt>>,
  account: Account,
  approvedServer: string,
  scope: "read" | "upload",
  encryptionKey: PeerAuthorityKey,
): Promise<DeviceCredential> {
  let committed: StoredCredential | undefined;
  let issuedAt = 0;
  let approval: Promise<void> | undefined;
  const transport = createDeviceLoginTransport({ approvedServer, scope, encryptionKey,
    generation: randomBytes(16).toString("hex"), encryptionIdentity: randomBytes(16).toString("hex") }, {
    signal: new AbortController().signal,
    issuedNow() { issuedAt = Math.floor(Date.now() / 1000); return issuedAt; },
    // Synthetic host facts and commit sink; this fixture writes no local state.
    assertCurrent() {}, async commit(value) { committed = value; return null; },
  });
  try {
    const result = await authorizeDevice(transport, new AbortController().signal, value => {
      approval = (async () => {
        const response = await service.call("/v1/device/approve", { method: "POST", body: { userCode: value.userCode },
          cookie: account.cookie, csrf: account.csrf, origin: account.origin });
        assert.equal(response.status, 200, "synthetic account approves via cookie and CSRF");
      })();
    }, { expectedScope: scope, sleep: async () => { assert.ok(approval); await approval; } });
    assert.equal(result, null);
  } finally { transport.dispose(); }
  assert.ok(committed);
  const credential = committed;
  assert.equal(credential.site, account.origin);
  assert.equal(credential.scope, scope);
  assert.equal(Object.isFrozen(credential), true);
  assert.deepEqual(decodeStoredCredential(encoder.encode(JSON.stringify(credential)), scope, approvedServer), credential);
  return { accessToken: credential.token, refreshToken: credential.refreshToken!, grantId: credential.grantId!, scope,
    expiresIn: credential.expiresAt! - issuedAt, refreshExpiresIn: credential.refreshExpiresAt! - issuedAt };
}

async function uploadEncryptedCapture(
  origin: string,
  upload: DeviceCredential,
  uploadSeed: Uint8Array,
  senderRecipient: string,
  profile: "root" | "evidence-first-v1",
) {
  const contents = [
    { path: "summary.md", bytes: encoder.encode("Synthetic encrypted recipient handoff.\n") },
    { path: "apparent.json", bytes: encoder.encode("{}") },
    { path: "contact-sheet.png", bytes: Uint8Array.of(4, 5, 6, 7) },
  ];
  if (profile === "evidence-first-v1") contents.push({ path: "audio.wav", bytes: Uint8Array.of(7, 8, 9) });
  // The fixture uses actual Worker peer facts, never a successful signature as authorization.
  const ownPublicKey = { version: 1 as const, recipient: senderRecipient,
    signing: Buffer.from(await ed25519.getPublicKeyAsync(uploadSeed)).toString("hex") };
  const peerFacts = () => readSenderPeerAuthority({ server: origin, accessToken: upload.accessToken,
    grantId: upload.grantId, ownPublicKey, active: () => true });
  const sealed = await sealCapture({ server: origin, captureId, ownPublicKey, signingSeed: uploadSeed,
    authority: await peerFacts(), profile, files: contents, active: () => true });
  const transport = new Map(sealed.assets.map(asset => [asset.path, asset.bytes]));
  const recipientGrantIds = sealed.recipientGrantIds;
  assert.ok(recipientGrantIds.includes(upload.grantId)); // Sender self-access matches the legacy packer.
  const options = { server: origin, captureId, ...sealed, trustedSenderSigningKey: await ed25519.getPublicKeyAsync(uploadSeed),
    active: () => true, readAsset: async (name: string) => transport.get(name)!,
    authorize: async (approvedServer: string, scope: "upload") => {
      const current = await peerFacts();
      return { approvedServer, scope, token: upload.accessToken, accountId: current.accountId, senderGrantId: current.grantId,
        recipientGrantIds: recipientGrantIds.filter(id => current.devices.some(device => device.grantId === id)) };
    } };
  try {
    await uploadPreparedEncryptedCapture(options);
    await uploadPreparedEncryptedCapture(options); // Ready reuse verifies the same immutable ciphertext.
  } finally { for (const asset of sealed.assets) asset.bytes.fill(0); }
  return contents;
}

async function exerciseProfile(mount: "" | "/connect", profile: "root" | "evidence-first-v1") {
  const server = `${mount}`;
  await proxy(mount, async ({ origin, service }) => {
    const serviceOrigin = origin + server;
    const account = await service.identity(`recipient-integration-${mount || "root"}`);
    const ownedSeeds: Uint8Array[] = [];
    try {
      const endpoint = async () => {
        const generated = await generateStoredIdentityRecord();
        try {
          const privateIdentity = await validateStoredIdentityRecord(decodeStoredIdentityRecord(generated.privateRecord));
          ownedSeeds.push(privateIdentity.signing);
          return { ...privateIdentity, publicKey: generated.publicKey };
        } finally { generated.privateRecord.fill(0); }
      };
      const reader = await endpoint(), sender = await endpoint();
      const readerSeed = reader.signing, senderSeed = sender.signing;
      const readCredential = await authorize(service, account, serviceOrigin, "read", reader.publicKey);
      const uploadCredential = await authorize(service, account, serviceOrigin, "upload", sender.publicKey);
      const contents = await uploadEncryptedCapture(serviceOrigin, uploadCredential, senderSeed, sender.publicKey.recipient, profile);
      const result = await retrieveRecipientCapture({
        credential: {
          server: serviceOrigin,
          token: readCredential.accessToken,
          grantId: readCredential.grantId,
          identity: { age: reader.age, signing: readerSeed },
        },
        captureId,
      });
      assert.equal(result.server, serviceOrigin);
      assert.equal(result.captureId, captureId);
      assert.deepEqual(result.files.map(file => file.path).sort(), contents.map(file => file.path).sort());
      for (const file of result.files) assert.deepEqual(file.bytes, contents.find(entry => entry.path === file.path)!.bytes);
      if (profile === "evidence-first-v1") {
        const evidence = await retrieveRecipientCapture({ credential: { server: serviceOrigin, token: readCredential.accessToken,
          grantId: readCredential.grantId, identity: { age: reader.age, signing: readerSeed } }, captureId, evidenceOnly: true });
        assert.equal(evidence.evidenceOnly, true);
        assert.ok(evidence.files.every(file => file.path !== "audio.wav"));
      }

      const stored = (grant: DeviceCredential, renewable: boolean) => decodeStoredCredential(encoder.encode(JSON.stringify({
        server: serviceOrigin, scope: grant.scope, token: grant.accessToken, generation: "c".repeat(32), authRequired: true,
        ...(renewable ? { refreshToken: grant.refreshToken, expiresAt: 1, refreshExpiresAt: 2, grantId: grant.grantId } : {}),
      })), grant.scope, serviceOrigin);
      assert.deepEqual(await revokeDeviceGrant(stored(readCredential, true), { approvedServer: serviceOrigin }), { ok: true });
      const accountOptions = (grant: DeviceCredential) => ({ server: serviceOrigin, scope: grant.scope as "read" | "upload",
        generation: "c".repeat(32), active: () => true,
        authorize: async (approvedServer: string, scope: "read" | "upload") => ({ approvedServer, scope, generation: "c".repeat(32), token: grant.accessToken }) });
      assert.equal((await readAccountStatus(accountOptions(uploadCredential))).beta, "active", "revoking the read device preserves the sibling upload grant");
      await assert.rejects(
        retrieveRecipientCapture({
          credential: {
            server: serviceOrigin,
            token: readCredential.accessToken,
            grantId: readCredential.grantId,
            identity: { age: reader.age, signing: readerSeed },
          },
          captureId,
        }),
        (error: unknown) => error instanceof RecipientTransportError && error.category === "unauthorized",
      );
      assert.deepEqual(await revokeDeviceGrant(stored(uploadCredential, false), { approvedServer: serviceOrigin }), { ok: true });
      await assert.rejects(readAccountStatus(accountOptions(uploadCredential)), { category: "unauthorized" });
    } finally { for (const seed of ownedSeeds) seed.fill(0); }
  });
}

test("retrieves through real local Workers on root and /connect origins and rejects revoked credentials", async () => {
  for (const profile of ["root", "evidence-first-v1"] as const) {
    await exerciseProfile("", profile);
    await exerciseProfile("/connect", profile);
  }
});

test("prepared plaintext sender reserves, uploads and verifies through real Workers on both bases", async () => {
  for (const mount of ["", "/connect"] as const) await proxy(mount, async ({ origin, service }) => {
    const server = origin + mount, account = await service.identity(`plaintext-sender-${mount || "root"}`);
    const bytes = new Map([
      ["apparent.json", encoder.encode("{}")], ["summary.md", encoder.encode("Synthetic prepared upload.")],
      ["contact-sheet.png", Uint8Array.of(3, 2, 1)],
    ]);
    const files = [...bytes].map(([path, value]) => ({ path, size: value.length, sha256: digest(value) }));
    const options = { server, captureId, files, active: () => true,
      readAsset: async (name: string) => bytes.get(name)!,
      authorize: async (approvedServer: string, scope: "upload") => ({ approvedServer, scope, token: account.token }) };
    await uploadPreparedPlainCapture(options);
    await uploadPreparedPlainCapture({ ...options, readAsset: async () => { throw Error("ready captures need no bytes"); } });
    const result = await retrieveRecipientCapture({ credential: { server, token: account.read, grantId: "synthetic-read-grant" },
      captureId, allowLegacyPlaintext: true });
    assert.equal(result.files.length, files.length);
    for (const file of result.files) assert.deepEqual(file.bytes, bytes.get(file.path));
    const sharedResponse = await service.call(`/v1/captures/${captureId}/share`, { method: "POST", token: account.token });
    assert.equal(sharedResponse.status, 200);
    const shared = await sharedResponse.json() as { url: string };
    const generation = "b".repeat(32), calls: string[] = [];
    const management = (scope: "read" | "upload") => ({ server, scope, generation, active: () => true,
      authorize: async (approvedServer: string, requested: "read" | "upload") => {
        assert.equal(approvedServer, server); assert.equal(requested, scope); calls.push(requested);
        return { approvedServer, scope: requested, generation, token: scope === "read" ? account.read : account.token };
      } });
    for (const scope of ["upload", "read"] as const) {
      const status = await readAccountStatus(management(scope));
      assert.deepEqual(status, { beta: "active", quotaBytes: 500_000_000, usedBytes: files.reduce((total, file) => total + file.size, 0) });
      assert.equal(Object.isFrozen(status), true);
    }
    const listed = await listShares(management("read"));
    assert.equal(listed.shares.length, 1); assert.equal(listed.shares[0]!.captureId, captureId);
    assert.equal(listed.shares[0]!.url, shared.url); assert.equal(listed.shares[0]!.status, "ready");
    assert.equal(Object.isFrozen(listed.shares[0]), true);
    const before = calls.length;
    await assert.rejects(revokeShare(management("read"), listed.shares[0]!.id), { category: "auth_required" });
    assert.equal(calls.length, before, "read scope cannot initiate revocation");
    await revokeShare(management("upload"), listed.shares[0]!.id);
    const after = await listShares(management("read"));
    assert.equal(after.shares[0]!.status, "revoked"); assert.equal(after.shares[0]!.url, null);
    assert.deepEqual(calls, ["upload", "read", "read", "upload", "upload", "read"]);
    assert.equal((await fetch(shared.url)).status, 410, "revoked capability is denied by the actual Worker");

  });
});
