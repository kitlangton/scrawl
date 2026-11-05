import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  ChildProcessTransport,
  EngineClient,
  encodeRequest,
  validateHandshake,
  validateReceipt,
  type Request,
} from "../src/index.ts";
import { ReferenceEngine } from "../src/testing.ts";
import { runV1Scenarios } from "./scenarios.ts";

const fixture = fileURLToPath(new URL("./child-fixture.ts", import.meta.url));
const clientId = "a".repeat(32),
  captureId = "b".repeat(32),
  token = "c".repeat(64),
  rpcId = "d".repeat(32);
function startReq(overrides: Partial<Request> = {}): Request {
  return {
    protocolVersion: 1,
    clientId,
    rpcId,
    method: "start",
    ownerToken: token,
    captureId,
    bundleId: "bundle-x",
    bundlePath: "/tmp/bundle-x",
    options: { screen: true },
    ...overrides,
  } as Request;
}
function child(mode: string) {
  return new ChildProcessTransport(
    spawn(process.execPath, ["--experimental-strip-types", fixture, mode], {
      stdio: ["pipe", "pipe", "pipe"],
    }),
  );
}

test("v1 durable capture identity is distinct from each correlated RPC", async () => {
  const peer = new ReferenceEngine(),
    a = new EngineClient(peer),
    b = new EngineClient(peer);
  const prepared = a.prepareStart("bundle-x", "/tmp/bundle-x");
  const [x, y] = await Promise.all([a.start(prepared), a.start(prepared)]);
  assert.equal(x.captureId, y.captureId);
  assert.notEqual(x.receipt.rpcId, y.receipt.rpcId);
  const recovered = await b.control(
    "inspect",
    prepared.captureId,
    prepared.ownerToken,
  );
  assert.equal(recovered.state, "starting");
  await assert.rejects(
    b.start({ ...prepared, bundlePath: "/tmp/other" }),
    /identity_conflict/,
  );
  const wrongOwner = await peer.request({
    ...prepared,
    rpcId: "e".repeat(32),
    ownerToken: "f".repeat(64),
  });
  assert.equal(wrongOwner.error?.code, "permission_denied");
  await assert.rejects(
    b.control("inspect", prepared.captureId, "f".repeat(64)),
    /permission_denied/,
  );
  await a.close();
});

test("portable v1 consumer scenarios use only the public client and a reconnect client", async () => {
  const peer = new ReferenceEngine();
  await runV1Scenarios(new EngineClient(peer, { allowTestDouble: true }), new EngineClient(peer, { allowTestDouble: true }), {
    id: "portable",
    path: "/tmp/portable",
  });
});

test("scenario peer models explicit transitions, stop-before-ready, sticky cancel, and no fake artifact", async () => {
  const peer = new ReferenceEngine(),
    client = new EngineClient(peer, { allowTestDouble: true }),
    start = client.prepareStart("b", "/tmp/b");
  await client.handshake();
  const begun = await client.start(start);
  await assert.rejects(
    client.control("stop", start.captureId, start.ownerToken),
    /not_ready/,
  );
  peer.setScenarioState(start.captureId, "recording");
  assert.equal(
    (await client.control("stop", start.captureId, start.ownerToken)).state,
    "stopping",
  );
  await client.control("cancel", start.captureId, start.ownerToken);
  assert.equal(
    (await client.control("inspect", start.captureId, start.ownerToken)).state,
    "cancelled",
  );
  assert.equal(begun.receipt.state, "starting");
  assert.throws(() =>
    validateReceipt({
      protocolVersion: 1,
      rpcId,
      captureId,
      state: "saved",
      bundlePath: "/tmp/b",
      artifactVersion: 1,
      artifactProvenance: "simulation",
    }),
  );
});

test("test-only synthetic saved fixture is accepted only after explicit test-double handshake", async () => {
  const peer = new ReferenceEngine(),
    client = new EngineClient(peer, { allowTestDouble: true });
  await client.handshake();
  const start = client.prepareStart("synthetic", "/tmp/synthetic");
  await client.start(start);
  peer.setScenarioState(start.captureId, "recording");
  peer.setScenarioState(start.captureId, "stopping");
  peer.setScenarioState(start.captureId, "finalizing");
  peer.setSavedFixture(start.captureId);
  const receipt = await client.control(
    "inspect",
    start.captureId,
    start.ownerToken,
  );
  assert.equal(receipt.state, "saved");
  assert.equal(receipt.artifactProvenance, "test-synthetic");
  const unmarked = new EngineClient(peer);
  await assert.rejects(unmarked.handshake(), /explicit test-only opt-in/);
  await assert.rejects(
    unmarked.control("inspect", start.captureId, start.ownerToken),
    /synthetic test artifact/,
  );
});

test("validators reject unknown fields, wrong versions, bad paths, errors and timeouts", async () => {
  assert.throws(() =>
    encodeRequest({ ...startReq(), mystery: true } as Request),
  );
  assert.throws(() =>
    encodeRequest({ ...startReq(), protocolVersion: 2 } as unknown as Request),
  );
  assert.throws(() => encodeRequest({ ...startReq(), bundlePath: "relative" }));
  assert.throws(() =>
    encodeRequest({ ...startReq(), method: "future" } as unknown as Request),
  );
  assert.throws(() =>
    encodeRequest({
      ...startReq(),
      options: { screen: true, audio: true },
    } as unknown as Request),
  );
  assert.throws(() => encodeRequest({ ...startReq(), bundleId: "../bundle" }));
  assert.throws(() => encodeRequest({ ...startReq(), bundleId: "" }));
  assert.throws(() => encodeRequest({ ...startReq(), bundleId: "bundle\u0085" }));
  assert.throws(() =>
    validateReceipt({ protocolVersion: 1, rpcId, captureId, state: "failed" }),
  );
  assert.throws(() =>
    validateReceipt({
      protocolVersion: 1,
      rpcId,
      captureId,
      state: "recording",
      error: { code: "internal", message: "unexpected" },
    }),
  );
  const privateText = validateReceipt({
    protocolVersion: 1,
    rpcId,
    captureId,
    state: "failed",
    error: { code: "internal", message: `private ${token} /home/user/capture` },
  });
  assert.ok(!privateText.error?.message.includes(token));
  assert.ok(!privateText.error?.message.includes("/home/user"));
  assert.throws(() =>
    validateHandshake({
      protocolVersion: 1,
      implementation: "test-double",
      capabilities: {
        screenCapture: true,
        audioCapture: false,
        windowCapture: false,
      },
      readiness: "ready",
    }),
  );
  const c = new EngineClient(new ReferenceEngine());
  await assert.rejects(c.handshake(0), /timeout/);
  await c.close();
  const mismatch: Transport = {
    async request(_request) {
      return {
        protocolVersion: 1,
        rpcId: "e".repeat(32),
        result: {
          protocolVersion: 1,
          implementation: "test-double",
          capabilities: {
            screenCapture: false,
            audioCapture: false,
            windowCapture: false,
          },
          readiness: "unavailable",
        },
      };
    },
    async close() {
      return "already-exited";
    },
  };
  await assert.rejects(
    new EngineClient(mismatch).handshake(),
    /response envelope/,
  );
});

test("real subprocess transport handles concurrent responses and late timeout replies by rpc id", async () => {
  const t = child("reply");
  const req1 = {
    protocolVersion: 1,
    clientId,
    rpcId: "1".repeat(32),
    method: "handshake",
  } as const;
  const req2 = { ...req1, rpcId: "2".repeat(32) };
  const [a, b] = await Promise.all([t.request(req1), t.request(req2)]);
  assert.equal(a.rpcId, req1.rpcId);
  assert.equal(b.rpcId, req2.rpcId);
  await t.close();
  const late = child("late");
  await assert.rejects(
    late.request({ ...req1, rpcId: "3".repeat(32) }, 15),
    /timed out/,
  );
  const next = { ...req1, rpcId: "4".repeat(32) };
  assert.equal((await late.request(next, 500)).rpcId, next.rpcId);
  await late.close();
});

test("real subprocess transport preserves split UTF-8 and accepts CRLF", async () => {
  const production = new EngineClient(child("crlf"));
  await assert.rejects(production.handshake(), /explicit test-only opt-in/);
  await production.close();
  const split = new EngineClient(child("unicode-split"));
  await assert.rejects(split.handshake(), /unavailable/);
  await split.close();
  const crlf = new EngineClient(child("crlf"), { allowTestDouble: true });
  assert.equal((await crlf.handshake()).implementation, "test-double");
  await crlf.close();
});

test("real subprocess transport rejects oversized unterminated and malformed output and settles EOF", async () => {
  const huge = child("huge");
  await assert.rejects(
    huge.request({
      protocolVersion: 1,
      clientId,
      rpcId,
      method: "handshake",
    } as Request),
    /exceeds protocol/,
  );
  await huge.close();
  const malformed = child("malformed");
  await assert.rejects(
    malformed.request({
      protocolVersion: 1,
      clientId,
      rpcId,
      method: "handshake",
    } as Request),
    /malformed/,
  );
  await malformed.close();
  const eof = child("eof");
  await assert.rejects(
    eof.request({
      protocolVersion: 1,
      clientId,
      rpcId,
      method: "handshake",
    } as Request),
    /exited/,
  );
  await eof.close();
});

test("close waits for bounded EOF cleanup and never claims a durable stop", async () => {
  const t = child("wait-eof");
  const began = Date.now();
  assert.equal(await t.close(), "eof-exited");
  assert.ok(Date.now() - began >= 30);
});

test("close reports bounded terminate and kill fallback outcome", async () => {
  const childProc = spawn(
    process.execPath,
    ["--experimental-strip-types", fixture, "hold"],
    {
      stdio: ["pipe", "pipe", "pipe"],
    },
  );
  const ready = new Promise<void>((resolve) =>
    childProc.stderr.once("data", () => resolve()),
  );
  const t = new ChildProcessTransport(childProc, 30);
  await ready;
  assert.equal(await t.close(), "kill-requested-after-timeout");
});
