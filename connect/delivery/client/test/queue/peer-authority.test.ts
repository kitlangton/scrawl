import assert from "node:assert/strict";
import { test } from "node:test";
import { createServer, type RequestListener } from "node:http";
import { once } from "node:events";
import * as ed25519 from "@noble/ed25519";
import { generateAgeIdentity } from "../../src/recipient/crypto.ts";
import { decodePeerAuthority, PeerAuthorityError } from "../../src/recipient/peer-authority.ts";
import { readSenderPeerAuthority } from "../../src/sender/authority.ts";
import { RecipientTransportError } from "../../src/recipient/transport.ts";

const seed = new Uint8Array(32).fill(7);
const hex = (bytes: Uint8Array) => Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
async function frame(scope: "read" | "upload" = "read") {
  const age = await generateAgeIdentity(), signing = await ed25519.getPublicKeyAsync(seed);
  const ownKey = { version: 1 as const, recipient: age.recipient, signing: hex(signing) };
  return {
    ownKey,
    value: { accountId: "acct", grantId: "grant", scope,
      devices: [{ grantId: "grant", scope, key: { ...ownKey } }] },
  };
}
const rejects = (category: string) => (error: unknown) => error instanceof PeerAuthorityError
  && error.category === category && error.message === category;

test("decodes read and upload authority with validated AGE and Ed25519 public keys", async () => {
  for (const scope of ["read", "upload"] as const) {
    const f = await frame(scope);
    const result = decodePeerAuthority(f.value, { scope, grantId: "grant", ownPublicKey: f.ownKey });
    assert.equal(result.scope, scope);
    assert.equal(result.devices[0]!.key.recipient, f.ownKey.recipient);
    assert.ok(Object.isFrozen(result) && Object.isFrozen(result.devices) && Object.isFrozen(result.devices[0]) && Object.isFrozen(result.devices[0]!.key));
  }
});

test("requires exact grant, scope, and own-key binding", async () => {
  const f = await frame();
  assert.throws(() => decodePeerAuthority(f.value, { scope: "upload", grantId: "grant", ownPublicKey: f.ownKey }), rejects("invalid_encryption_authorization"));
  assert.throws(() => decodePeerAuthority(f.value, { scope: "read", grantId: "other", ownPublicKey: f.ownKey }), rejects("invalid_encryption_authorization"));
  assert.throws(() => decodePeerAuthority({ ...f.value, devices: [] }, { scope: "read", grantId: "grant", ownPublicKey: f.ownKey }), rejects("encryption_approval_required"));
  const other = await generateAgeIdentity();
  assert.throws(() => decodePeerAuthority(f.value, { scope: "read", grantId: "grant", ownPublicKey: { ...f.ownKey, recipient: other.recipient } }), rejects("encryption_key_binding_mismatch"));
});

test("rejects duplicate IDs, unknown fields, malformed keys, and over-limit devices", async () => {
  const f = await frame();
  const run = (value: unknown) => decodePeerAuthority(value, { scope: "read", grantId: "grant", ownPublicKey: f.ownKey });
  assert.throws(() => run({ ...f.value, extra: true }), rejects("invalid_encryption_authorization"));
  assert.throws(() => run({ ...f.value, devices: [f.value.devices[0], f.value.devices[0]] }), rejects("invalid_encryption_authorization"));
  assert.throws(() => run({ ...f.value, devices: Array.from({ length: 65 }, (_, i) => ({ ...f.value.devices[0], grantId: `d${i}` })) }), rejects("invalid_encryption_authorization"));
  const badAge = structuredClone(f.value); badAge.devices[0]!.key.recipient = "age1" + "a".repeat(58);
  assert.throws(() => run(badAge), rejects("invalid_encryption_authorization"));
  const badChecksum = structuredClone(f.value); badChecksum.devices[0]!.key.recipient = f.ownKey.recipient.slice(0, -1) + (f.ownKey.recipient.endsWith("a") ? "b" : "a");
  assert.throws(() => run(badChecksum), rejects("invalid_encryption_authorization"));
  for (const signing of ["00".repeat(32), "01" + "00".repeat(31)]) {
    const badPoint = structuredClone(f.value); badPoint.devices.push({ grantId: "rogue", scope: "upload", key: { ...f.ownKey, signing } });
    assert.throws(() => run(badPoint), rejects("invalid_encryption_authorization"));
  }
});

test("returns detached frozen public-only data and does not expose a token", async () => {
  const f = await frame(); const input = structuredClone(f.value);
  const result = decodePeerAuthority(input, { scope: "read", grantId: "grant", ownPublicKey: f.ownKey });
  input.accountId = "changed"; input.devices[0]!.key.recipient = "changed";
  assert.equal(result.accountId, "acct"); assert.equal(result.devices[0]!.key.recipient, f.ownKey.recipient);
  assert.equal("token" in result, false); assert.equal(JSON.stringify(result).includes("secret"), false);
});

async function endpoint(handler: RequestListener, run: (origin: string) => Promise<void>) {
  const server = createServer(handler); server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address(); assert.ok(address && typeof address !== "string");
  try { await run(`http://127.0.0.1:${address.port}`); }
  finally { server.closeAllConnections(); await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve())); }
}
const transportRejects = (category: string) => (error: unknown) => error instanceof RecipientTransportError
  && error.category === category && error.message === category;

test("sender reads pinned public authority once on root and mounted API bases", async () => {
  const f = await frame("upload");
  for (const mount of ["", "/connect"]) {
    let requests = 0;
    await endpoint((request, response) => {
      requests++; assert.equal(request.method, "GET");
      assert.equal(request.url, mount + "/v1/encryption/peers");
      assert.equal(request.headers.authorization, "Bearer synthetic-token");
      response.end(JSON.stringify(f.value));
    }, async origin => {
      const result = await readSenderPeerAuthority({ server: origin + mount, accessToken: "synthetic-token",
        grantId: "grant", ownPublicKey: f.ownKey, active: () => true });
      assert.equal(requests, 1); assert.equal(result.server, origin + mount);
      assert.equal(result.scope, "upload"); assert.equal(result.accountId, "acct");
      assert.ok(Object.isFrozen(result) && Object.isFrozen(result.devices[0]!.key));
      assert.equal(JSON.stringify(result).includes("synthetic-token"), false);
    });
  }
});

test("sender refuses wrong scope, grant, binding and duplicate service peers", async () => {
  const f = await frame("upload");
  for (const [value, category] of [
    [{ ...f.value, scope: "read" }, "invalid_encryption_authorization"],
    [{ ...f.value, grantId: "other" }, "invalid_encryption_authorization"],
    [{ ...f.value, devices: [] }, "encryption_approval_required"],
    [{ ...f.value, devices: [{ ...f.value.devices[0], scope: "read" }] }, "encryption_key_binding_mismatch"],
    [{ ...f.value, devices: [f.value.devices[0], f.value.devices[0]] }, "invalid_encryption_authorization"],
  ] as const) {
    await endpoint((_request, response) => response.end(JSON.stringify(value)), async origin => {
      await assert.rejects(readSenderPeerAuthority({ server: origin, accessToken: "synthetic-token", grantId: "grant",
        ownPublicKey: f.ownKey, active: () => true }), rejects(category));
    });
  }
});

test("sender validates before dispatch and rechecks cancellation and freshness after GET", async () => {
  const f = await frame("upload"); let requests = 0, active = true;
  const abort = new AbortController(); abort.abort();
  await endpoint((_request, response) => { requests++; active = false; response.end(JSON.stringify(f.value)); }, async origin => {
    const input = { server: origin, accessToken: "synthetic-token", grantId: "grant", ownPublicKey: f.ownKey, active: () => active };
    await assert.rejects(readSenderPeerAuthority({ ...input, signal: abort.signal }), transportRejects("cancelled"));
    await assert.rejects(readSenderPeerAuthority({ ...input, active: () => false }), transportRejects("stale_snapshot"));
    await assert.rejects(readSenderPeerAuthority({ ...input, active: () => { throw new Error("private-path"); } }), transportRejects("stale_snapshot"));
    await assert.rejects(readSenderPeerAuthority({ ...input, ownPublicKey: { ...f.ownKey, signing: "bad" } }), rejects("invalid_encryption_authorization"));
    assert.equal(requests, 0);
    await assert.rejects(readSenderPeerAuthority(input), transportRejects("stale_snapshot"));
    assert.equal(requests, 1);
  });
});

test("sender snapshots caller facts before callbacks and never retries rejected or redirected GETs", async () => {
  const f = await frame("upload"); let requests = 0;
  await endpoint((request, response) => {
    requests++; assert.equal(request.headers.authorization, "Bearer synthetic-token");
    response.end(JSON.stringify(f.value));
  }, async origin => {
    const input = { server: origin, accessToken: "synthetic-token", grantId: "grant", ownPublicKey: { ...f.ownKey },
      active: () => { input.server = "https://changed.invalid"; input.accessToken = "changed";
        input.grantId = "changed"; input.ownPublicKey.signing = "changed"; return true; } };
    const result = await readSenderPeerAuthority(input);
    assert.equal(result.grantId, "grant"); assert.equal(result.server, origin); assert.equal(requests, 1);
  });
  for (const [status, category] of [[401, "unauthorized"], [302, "redirect_refused"]] as const) {
    let calls = 0;
    await endpoint((_request, response) => { calls++; response.writeHead(status, { location: "/private-token" }); response.end("private-token"); }, async origin => {
      await assert.rejects(readSenderPeerAuthority({ server: origin, accessToken: "synthetic-token", grantId: "grant",
        ownPublicKey: f.ownKey, active: () => true }), transportRejects(category));
      assert.equal(calls, 1);
    });
  }
});
