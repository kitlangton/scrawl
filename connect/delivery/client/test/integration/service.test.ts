import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createServer, type Server } from "node:http";
import { once } from "node:events";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import * as ed25519 from "@noble/ed25519";
import {
  encodeArchive,
  encryptAge,
  generateAgeIdentity,
  signEnvelope,
  splitAgeStream,
  type Context,
} from "../../src/recipient/crypto.ts";
import { retrieveRecipientCapture } from "../../src/recipient/retrieve.ts";
import { RecipientTransportError } from "../../src/recipient/transport.ts";
import { uploadPreparedPlainCapture } from "../../src/sender/transport.ts";
import { uploadPreparedEncryptedCapture, type PreparedEncryptedGroup } from "../../src/sender/encrypted-transport.ts";
import { readSenderPeerAuthority } from "../../src/sender/authority.ts";

const serviceDir = resolve(fileURLToPath(new URL("../../../../service/", import.meta.url)));
const captureId = "a981426be1254f37938214f960aed501";
const encoder = new TextEncoder();
const digest = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

type Account = {
  id: string; token: string; read: string; cookie: string; csrf: string; origin: string;
};
type DeviceCredential = { accessToken: string; grantId: string; scope: string };

async function fixtureAt(serviceOrigin: string) {
  const cwd = process.cwd();
  process.chdir(serviceDir);
  try {
    // The helper is JavaScript supplied by the service test fixture; this file
    // remains TypeScript and Node 22 strips its types when run directly.
    // @ts-expect-error service fixture intentionally has no declaration file
    const { fixture } = await import("../../../../service/test/helpers.mjs");
    return await fixture({ API_ORIGIN: serviceOrigin, SITE_ORIGIN: "https://site.example" });
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
  scope: "read" | "upload",
  age: { identity: string; recipient: string },
  signing: Uint8Array,
): Promise<DeviceCredential> {
  const publicKey = await ed25519.getPublicKeyAsync(signing);
  const encryptionKey = {
    version: 1,
    recipient: age.recipient,
    signing: Array.from(publicKey, byte => byte.toString(16).padStart(2, "0")).join(""),
  };
  const started = await service.call("/v1/device/start", {
    method: "POST", body: { scope, encryptionKey },
  });
  assert.equal(started.status, 200, "device authorization starts in the real Worker");
  const device = await started.json() as { deviceCode: string; userCode: string };
  const approved = await service.call("/v1/device/approve", {
    method: "POST", body: { userCode: device.userCode },
    cookie: account.cookie, csrf: account.csrf, origin: account.origin,
  });
  assert.equal(approved.status, 200, "synthetic account approves the pending device via cookie and CSRF");
  const polled = await service.call("/v1/device/poll", { method: "POST", body: { deviceCode: device.deviceCode } });
  assert.equal(polled.status, 200);
  const credential = await polled.json() as DeviceCredential & { status: string };
  assert.equal(credential.status, "approved");
  assert.equal(credential.scope, scope);
  return credential;
}

async function uploadEncryptedCapture(
  origin: string,
  account: Account,
  reader: { identity: string; recipient: string },
  upload: DeviceCredential,
  uploadSeed: Uint8Array,
  senderRecipient: string,
  profile: "root" | "evidence-first-v1",
) {
  const context: Context = {
    origin, accountId: account.id, captureId, senderGrantId: upload.grantId,
  };
  const contents = [
    { path: "summary.md", bytes: encoder.encode("Synthetic encrypted recipient handoff.\n") },
    { path: "apparent.json", bytes: encoder.encode("{}") },
    { path: "contact-sheet.png", bytes: Uint8Array.of(4, 5, 6, 7) },
  ];
  if (profile === "evidence-first-v1") contents.push({ path: "audio.wav", bytes: Uint8Array.of(7, 8, 9) });
  const groups: PreparedEncryptedGroup[] = [], transport = new Map<string, Uint8Array>();
  for (const group of profile === "root" ? [undefined] : ["evidence", "media"] as const) {
    const bound: Context = { ...context, ...(group ? { group } : {}) };
    const selected = group ? contents.filter(file => (file.path === "audio.wav") === (group === "media")) : contents;
    const files = selected.map(file => ({ path: file.path, size: file.bytes.length, sha256: digest(file.bytes) }));
    const archive = encodeArchive({ version: 1, context: bound, files }, selected.map(file => file.bytes));
    const ciphertext = await encryptAge(archive, [reader.recipient]);
    const parts = splitAgeStream(ciphertext);
    const chunks = parts.map((bytes, index) => ({ path: `sealed/part-${String(index).padStart(6, "0")}.age`, size: bytes.length, sha256: digest(bytes) }));
    const signed = { version: 1, context: bound, chunks };
    const envelopeBytes = encoder.encode(JSON.stringify({ signed, signature: await signEnvelope(uploadSeed, signed) }));
    const prefix = group ? `${group}/` : "";
    const envelope = { path: prefix + "envelope.json", size: envelopeBytes.length, sha256: digest(envelopeBytes) };
    transport.set(envelope.path, envelopeBytes);
    chunks.forEach((chunk, index) => transport.set(prefix + chunk.path, parts[index]!));
    groups.push({ context: bound, envelope, chunks: chunks.map(chunk => ({ ...chunk, path: prefix + chunk.path })) });
  }
  // The fixture uses actual Worker peer facts, never a successful signature as authorization.
  const ownPublicKey = { version: 1 as const, recipient: senderRecipient,
    signing: Buffer.from(await ed25519.getPublicKeyAsync(uploadSeed)).toString("hex") };
  const peerFacts = () => readSenderPeerAuthority({ server: origin, accessToken: upload.accessToken,
    grantId: upload.grantId, ownPublicKey, active: () => true });
  const recipientGrantIds = (await peerFacts()).devices.filter(device => device.scope === "read").map(device => device.grantId);
  const options = { server: origin, captureId, context, recipientGrantIds, trustedSenderSigningKey: await ed25519.getPublicKeyAsync(uploadSeed),
    active: () => true, readAsset: async (name: string) => transport.get(name)!,
    authorize: async (approvedServer: string, scope: "upload") => {
      const current = await peerFacts();
      return { approvedServer, scope, token: upload.accessToken, accountId: current.accountId, senderGrantId: current.grantId,
        recipientGrantIds: recipientGrantIds.filter(id => current.devices.some(device => device.grantId === id)) };
    } };
  const prepared = profile === "root" ? { ...options, profile, envelope: groups[0]!.envelope, chunks: groups[0]!.chunks }
    : { ...options, profile, groups };
  await uploadPreparedEncryptedCapture(prepared);
  await uploadPreparedEncryptedCapture(prepared); // Existing ready captures still require verified readback.
  return contents;
}

async function exerciseProfile(mount: "" | "/connect", profile: "root" | "evidence-first-v1") {
  const server = `${mount}`;
  await proxy(mount, async ({ origin, request, service }) => {
    const serviceOrigin = origin + server;
    const account = await service.identity(`recipient-integration-${mount || "root"}`);
    const reader = await generateAgeIdentity();
    const sender = await generateAgeIdentity();
    const readerSeed = Uint8Array.from({ length: 32 }, (_, index) => index + 31);
    const senderSeed = Uint8Array.from({ length: 32 }, (_, index) => index + 91);
    const readCredential = await authorize(service, account, "read", reader, readerSeed);
    const uploadCredential = await authorize(service, account, "upload", sender, senderSeed);
    const contents = await uploadEncryptedCapture(serviceOrigin, account, reader, uploadCredential, senderSeed, sender.recipient, profile);
    const result = await retrieveRecipientCapture({
      credential: {
        server: serviceOrigin,
        token: readCredential.accessToken,
        grantId: readCredential.grantId,
        identity: { age: reader.identity, signing: readerSeed },
      },
      captureId,
    });
    assert.equal(result.server, serviceOrigin);
    assert.equal(result.captureId, captureId);
    assert.deepEqual(result.files.map(file => file.path).sort(), contents.map(file => file.path).sort());
    for (const file of result.files) assert.deepEqual(file.bytes, contents.find(entry => entry.path === file.path)!.bytes);
    if (profile === "evidence-first-v1") {
      const evidence = await retrieveRecipientCapture({ credential: { server: serviceOrigin, token: readCredential.accessToken,
        grantId: readCredential.grantId, identity: { age: reader.identity, signing: readerSeed } }, captureId, evidenceOnly: true });
      assert.equal(evidence.evidenceOnly, true);
      assert.ok(evidence.files.every(file => file.path !== "audio.wav"));
    }

    const revoked = await request(`${server}/v1/token/revoke`, {
      method: "POST", headers: { Authorization: `Bearer ${readCredential.accessToken}` },
    });
    assert.equal(revoked.status, 200, "the real Worker revokes the read grant");
    await assert.rejects(
      retrieveRecipientCapture({
        credential: {
          server: serviceOrigin,
          token: readCredential.accessToken,
          grantId: readCredential.grantId,
          identity: { age: reader.identity, signing: readerSeed },
        },
        captureId,
      }),
      (error: unknown) => error instanceof RecipientTransportError && error.category === "unauthorized",
    );
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
  });
});
