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
  service: Awaited<ReturnType<typeof fixtureAt>>,
  origin: string,
  account: Account,
  reader: { identity: string; recipient: string },
  upload: DeviceCredential,
  uploadSeed: Uint8Array,
) {
  const context: Context = {
    origin, accountId: account.id, captureId, senderGrantId: upload.grantId,
  };
  const contents = [
    { path: "summary.md", bytes: encoder.encode("Synthetic encrypted recipient handoff.\n") },
    { path: "apparent.json", bytes: encoder.encode("{}") },
    { path: "contact-sheet.png", bytes: Uint8Array.of(4, 5, 6, 7) },
  ];
  const innerFiles = contents.map(file => ({ path: file.path, size: file.bytes.length, sha256: digest(file.bytes) }));
  const archive = encodeArchive({ version: 1, context, files: innerFiles }, contents.map(file => file.bytes));
  const ciphertext = await encryptAge(archive, [reader.recipient]);
  const chunks = splitAgeStream(ciphertext).map((bytes, index) => ({
    path: `sealed/part-${String(index).padStart(6, "0")}.age`, bytes,
  }));
  const chunkAssets = chunks.map(({ path, bytes }) => ({ path, size: bytes.length, sha256: digest(bytes) }));
  const signed = { version: 1, context, chunks: chunkAssets };
  const envelope = encoder.encode(JSON.stringify({ signed, signature: await signEnvelope(uploadSeed, signed) }));
  const transportFiles = [
    { path: "envelope.json", bytes: envelope },
    ...chunks,
  ];
  const assets = transportFiles.map(({ path, bytes }) => ({ path, size: bytes.length, sha256: digest(bytes) }));
  const path = `/v1/captures/${captureId}`;
  const reserved = await service.call(path, {
    token: upload.accessToken, method: "POST", body: { format: "apparent-encrypted-v1", files: assets },
  });
  assert.equal(reserved.status, 200, "the real Worker reserves the encrypted capture");
  for (const file of transportFiles) {
    const put = await service.call(`${path}/files/${file.path}`, {
      token: upload.accessToken, method: "PUT", raw: file.bytes,
    });
    assert.equal(put.status, 200, `the real Worker stores ${file.path}`);
  }
  const completed = await service.call(`${path}/complete`, { token: upload.accessToken, method: "POST" });
  assert.equal(completed.status, 200, "the real Worker completes the capture");
  return contents;
}

async function exerciseProfile(mount: "" | "/connect") {
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
    const contents = await uploadEncryptedCapture(service, serviceOrigin, account, reader, uploadCredential, senderSeed);
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
    assert.deepEqual(result.files.map(file => file.path), contents.map(file => file.path));
    for (let index = 0; index < contents.length; index++) {
      assert.deepEqual(result.files[index]!.bytes, contents[index]!.bytes);
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
  await exerciseProfile("");
  await exerciseProfile("/connect");
});
