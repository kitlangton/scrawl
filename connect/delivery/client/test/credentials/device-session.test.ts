import assert from "node:assert/strict";
import test from "node:test";
import { createDeviceLoginTransport, createStateBackedDeviceLoginTransport } from "../../dist/account/device-session.js";
import { authorizeDevice } from "../../dist/device-flow.cjs";
import { generateStoredIdentityRecord } from "../../dist/credentials/recipient-identity.js";

const context = (extra: Record<string, unknown> = {}) => ({ scope: "upload" as const,
  approvedServer: "https://api.example.test/connect", generation: "a".repeat(32), encryptionIdentity: null, ...extra });
const reply = (extra: Record<string, unknown> = {}) => ({ status: "approved", scope: "upload", accessToken: "access-token",
  refreshToken: "refresh-token", grantId: "grant-id", expiresIn: 3600, refreshExpiresIn: 7200, ...extra });
let serialTail = Promise.resolve();
const serial = (name: string, run: () => Promise<void> | void) => test(name, async () => {
  let release!: () => void;
  const previous = serialTail; serialTail = new Promise<void>(resolve => { release = resolve; });
  await previous; try { await run(); } finally { release(); }
});
function setup(options: { fetch?: (url: string, init?: RequestInit) => Promise<Response>; now?: () => number; issuedNow?: () => number;
  assertCurrent?: () => void | Promise<void>; commit?: (credential: unknown, signal: AbortSignal) => Promise<unknown> } = {}) {
  const calls: Array<{ url: string; body: Record<string, unknown> }> = [];
  const oldFetch = globalThis.fetch;
  globalThis.fetch = options.fetch ?? (async (url, init) => {
    const body = JSON.parse(new TextDecoder().decode(init?.body as ArrayBuffer)); calls.push({ url: String(url), body });
    return new Response(JSON.stringify(url.toString().endsWith("/v1/device/start")
      ? { deviceCode: "device-code", userCode: "ABCD", verificationUri: "https://site.example.test/activate" }
      : reply()), { status: 200, headers: { "content-type": "application/json" } });
  });
  const controller = new AbortController();
  let committed: unknown;
  const transport = createDeviceLoginTransport(context(), { signal: controller.signal,
    assertCurrent: options.assertCurrent ?? (() => {}), now: options.now ?? (() => 1000), issuedNow: options.issuedNow ?? (() => 1_700_000_000),
    commit: async (credential, signal) => { committed = credential; return options.commit ? await options.commit(credential, signal) as never : null; } });
  return { transport, calls, controller, restore: () => { globalThis.fetch = oldFetch; }, committed: () => committed };
}

serial("snapshots context, binds one started code, and commits a private proposal once", async () => {
  const s = setup();
  try {
    const start = await s.transport.start();
    assert.equal((start as { userCode: string }).userCode, "ABCD");
    await assert.rejects(s.transport.poll("wrong-code", 100));
    const polled = await s.transport.poll("device-code", 100);
    assert.equal((polled as { accessToken: string }).accessToken, "access-token");
    assert.equal(await s.transport.commit("https://site.example.test"), null);
    assert.equal((s.committed() as { expiresAt: number }).expiresAt, 1_700_003_600);
    await assert.rejects(s.transport.commit("https://site.example.test"));
    assert.deepEqual(s.calls.map(c => c.url.split("/").at(-1)), ["start", "poll"]);
    assert.equal("accessToken" in (s.committed() as object), false);
  } finally { s.restore(); }
});

serial("validates scope and identity shapes before any transport can run", () => {
  const signal = new AbortController().signal;
  const ports = { signal, assertCurrent() {}, async commit() { return null; } };
  assert.throws(() => createDeviceLoginTransport(context({ scope: "read" }), ports));
  assert.throws(() => createDeviceLoginTransport(context({ encryptionIdentity: "b".repeat(32) }), ports));
  assert.throws(() => createDeviceLoginTransport(context({ generation: "bad" }), ports));
  assert.throws(() => createDeviceLoginTransport(context({ approvedServer: "https://user:pass@example.test" }), ports));
});

serial("retains original poll issue time and rejects concurrent operations", async () => {
  let now = 5_000, issue = 1_700_000_000, release!: () => void;
  const s = setup({ now: () => now, issuedNow: () => issue, fetch: async (url, init) => {
    if (String(url).endsWith("/v1/device/start")) return new Response(JSON.stringify({ deviceCode: "device-code", userCode: "ABCD", verificationUri: "https://site.example.test/activate" }));
    await new Promise<void>(resolve => { release = resolve; });
    now += 50; issue += 100;
    return new Response(JSON.stringify(reply()));
  } });
  try {
    await s.transport.start();
    const pending = s.transport.poll("device-code", 100);
    await assert.rejects(s.transport.poll("device-code", 100));
    release(); await pending;
    assert.equal(await s.transport.commit("https://site.example.test"), null);
    assert.equal((s.committed() as { expiresAt: number }).expiresAt, 1_700_003_600);
  } finally { s.restore(); }
});

serial("projects callback uncertainty and consumes approved data before callback", async () => {
  const s = setup({ commit: async () => { throw new Error("secret token"); } });
  try {
    await s.transport.start(); await s.transport.poll("device-code", 100);
    assert.equal(await s.transport.commit("https://site.example.test"), "unverified-commit");
    await assert.rejects(s.transport.commit("https://site.example.test"));
  } finally { s.restore(); }
});

serial("disposal and abort invalidate pending use", async () => {
  const s = setup();
  try { s.transport.dispose(); await assert.rejects(s.transport.start()); }
  finally { s.restore(); }
  const a = setup();
  try { a.controller.abort(); await assert.rejects(a.transport.start()); }
  finally { a.restore(); }
});

serial("fence loss before dispatch prevents HTTP and TTL expires the started session", async () => {
  let clock = 10_000, fenced = true;
  const s = setup({ now: () => clock, assertCurrent: () => { if (fenced) throw new Error("stale login secret"); } });
  try {
    await assert.rejects(s.transport.start(), /cancelled/);
    assert.equal(s.calls.length, 0);
  } finally { s.restore(); }
  const t = setup({ now: () => clock });
  try {
    await t.transport.start(); clock += 15 * 60_000;
    await assert.rejects(t.transport.poll("device-code", 100));
    assert.equal(t.calls.length, 1);
  } finally { t.restore(); }
});

serial("fence loss after a network await discards its reply", async () => {
  let fenced = false, fetches = 0;
  const s = setup({ assertCurrent: () => { if (fenced) throw new Error("stale login secret"); }, fetch: async url => {
    fetches++;
    if (String(url).endsWith("/v1/device/start")) return new Response(JSON.stringify({ deviceCode: "device-code", userCode: "ABCD", verificationUri: "https://site.example.test/activate" }));
    const response = new Response(JSON.stringify(reply())); fenced = true; return response;
  } });
  try {
    await s.transport.start();
    await assert.rejects(s.transport.poll("device-code", 100), /cancelled/);
    assert.equal(fetches, 2);
  } finally { s.restore(); }
});

serial("post-commit fence loss never reports verified success", async () => {
  let current = true;
  const s = setup({ assertCurrent: () => { if (!current) throw Error('changed'); }, commit: async () => { current = false; return null; } });
  try {
    await s.transport.start(); await s.transport.poll('device-code', 100);
    assert.equal(await s.transport.commit('https://site.example.test'), 'unverified-commit');
  } finally { s.restore(); }
});

serial("lifetime starts at construction, including idle time before start", async () => {
  let clock = 1000;
  const s = setup({ now: () => clock });
  try {
    clock += 15 * 60_000;
    await assert.rejects(s.transport.start());
    assert.equal(s.calls.length, 0);
  } finally { s.restore(); }
});

serial("disposal while commit is pending prevents a verified receipt", async () => {
  const s = setup({ commit: async () => { s.transport.dispose(); return null; } });
  try {
    await s.transport.start(); await s.transport.poll('device-code', 100);
    assert.equal(await s.transport.commit('https://site.example.test'), 'unverified-commit');
  } finally { s.restore(); }
});

serial("snapshots context and host ports and retains an independent approved reply", async () => {
  const s = setup();
  let saved: { token: string } | undefined;
  const input = context();
  const ports = { signal: new AbortController().signal, assertCurrent() {}, now: () => 1000,
    async commit(value: { token: string }) { saved = value; return null; } };
  const transport = createDeviceLoginTransport(input, ports);
  try {
    input.approvedServer = 'https://other.example.test';
    ports.assertCurrent = () => { throw Error('mutated fence'); };
    ports.commit = async () => { throw Error('mutated commit'); };
    await transport.start();
    const value = await transport.poll('device-code', 100) as { accessToken: string };
    value.accessToken = 'mutated-response';
    assert.equal(await transport.commit('https://site.example.test'), null);
    assert.equal(saved?.token, 'access-token');
    assert.ok(s.calls.every(call => call.url.startsWith('https://api.example.test/connect/')));
  } finally { transport.dispose(); s.restore(); }
});

serial("read sessions accept a validated endpoint key and policy receives safe cancellation", async () => {
  const generated = await generateStoredIdentityRecord();
  try {
    const signal = new AbortController().signal;
    const transport = createDeviceLoginTransport(context({ scope: 'read', encryptionIdentity: 'b'.repeat(32), encryptionKey: generated.publicKey }), {
      signal, assertCurrent() {}, async commit() { return null; },
    });
    transport.dispose();
  } finally { generated.privateRecord.fill(0); }
  const s = setup();
  try {
    s.controller.abort();
    assert.equal(await authorizeDevice(s.transport, new AbortController().signal, () => {}), 'cancelled');
    assert.equal(s.calls.length, 0);
  } finally { s.restore(); }
});

serial("expiry while awaiting the host fence prevents network dispatch", async () => {
  let clock = 1000;
  const s = setup({ now: () => clock, assertCurrent: () => { clock += 15 * 60_000; } });
  try {
    await assert.rejects(s.transport.start(), /expired/);
    assert.equal(s.calls.length, 0);
  } finally { s.restore(); }
});

serial("state composition rejects uncertain receipts and checks host allowance before state access", async () => {
  let calls = 0;
  const receipt = { state: 'begun' as const, scope: 'upload' as const, approvedServer: 'https://api.example.test/connect',
    loginId: 'b'.repeat(32), cancellation: null, directoryIdentity: '1:2:16832:1000' };
  const resources = { async checkAcl() { calls++; throw Error('unexpected ACL'); }, async lock() { calls++; throw Error('unexpected lock'); } };
  const ports = { generation: 'a'.repeat(32), signal: new AbortController().signal,
    assertAllowed() { throw Error('private policy refused'); }, async commit() { calls++; return null; } };
  assert.throws(() => createStateBackedDeviceLoginTransport('/unused', { ...receipt, state: 'uncertain' } as never, null, resources, ports));
  const transport = createStateBackedDeviceLoginTransport('/unused', receipt, null, resources, ports);
  try { await assert.rejects(transport.start(), /cancelled/); assert.equal(calls, 0); }
  finally { transport.dispose(); }
});
