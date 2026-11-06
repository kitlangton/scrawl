import test from "node:test";
import "./recovery/process.test.ts";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { MediaChildProcessTransport, MediaProcessTransport, type MediaTransport } from "../src/media.ts";
import { Media2Client, Media2PreflightError, Media2RpcError, Media2TransportError, preflightMedia2Dispatch, type Media2Handshake, type Media2JobInput, type Media2RpcRequest, type Media2RpcResponse, type Media2Transport } from "../src/media2/index.ts";
import { encodeMedia2Rpc, MEDIA2_MAX_LINE } from "../src/media2/protocol.ts";
import { fixture } from "./media2/fixture.ts";

// Metadata fixtures/injected peers test SDK correlation, never native origin.
const hello = (): Media2Handshake => ({ channel: "media", protocolVersion: 2, implementation: "native", capabilities: { retainedFrames: true, contactSheet: true, nativeJobAuthority: true } });
const reply = (r: Media2RpcRequest, result: unknown): Media2RpcResponse => ({ channel: "media", protocolVersion: 2, rpcId: r.rpcId, result });
class Peer implements Media2Transport {
  calls: Media2RpcRequest[] = [];
  handshake = hello();
  handler: (r: Exclude<Media2RpcRequest, { method: "handshake" }>) => Promise<Media2RpcResponse> | Media2RpcResponse = r => reply(r, { jobId: r.jobId, requestSha256: r.requestSha256, state: "running" });
  async request(r: Media2RpcRequest): Promise<Media2RpcResponse> { this.calls.push(structuredClone(r)); return r.method === "handshake" ? reply(r, structuredClone(this.handshake)) : this.handler(r); }
  async close() { return "eof-exited" as const; }
}
const job = () => { const f = fixture(); const input: Media2JobInput = { ownerToken: "a".repeat(64), request: f.request, materials: f.materials }; return { f, input }; };
// Compile-time compatibility for the original public, nongeneric constructor.
const legacyTransportType = (value: InstanceType<typeof MediaChildProcessTransport>): MediaTransport => value;

test("media2 preflight rejects locally before dispatch and distinguishes unavailable capability", async () => {
  const { input } = job(); const peer = new Peer(); const client = new Media2Client(peer);
  await client.handshake();
  for (const value of [{ ...input, ownerToken: "secret" }, { ...input, request: { ...input.request, limits: { ...input.request.limits, maxFrames: 1 } } }]) {
    assert.throws(() => preflightMedia2Dispatch(value), (error: unknown) => error instanceof Media2PreflightError && error.dispatched === false);
    await assert.rejects(client.derive(value), Media2PreflightError);
  }
  assert.throws(() => preflightMedia2Dispatch(input, 0), Media2PreflightError);
  await assert.rejects(client.derive(input, 0), Media2PreflightError);
  assert.equal(peer.calls.length, 1);
  const cold = new Peer(); cold.handshake.capabilities = { retainedFrames: false, contactSheet: false, nativeJobAuthority: true };
  const controls = new Media2Client(cold); await controls.handshake();
  await assert.rejects(controls.derive(input), (e: unknown) => e instanceof Media2RpcError && e.code === "unavailable" && e.dispatched === false);
  assert.equal((await controls.inspect(input)).state, "running");
  assert.equal((await controls.cancel(input)).state, "running");
});

test("media2 job responses preserve original request hash and exact detached frame/sheet receipt", async () => {
  const { f, input } = job(); const peer = new Peer();
  peer.handler = r => reply(r, { jobId: r.jobId, requestSha256: r.requestSha256, state: "complete", derivative: { envelope: f.document().envelope, receiptJson: Buffer.from(f.document().bytes).toString("utf8") } });
  const client = new Media2Client(peer); const claimed = await client.handshake(); claimed.capabilities.retainedFrames = false;
  const result = await client.derive(input);
  assert.equal(result.state, "complete");
  if (result.state !== "complete") throw new Error("missing fixture result");
  assert.deepEqual(result.derivative.request.requestedBundleMs, [0, 900, 900, 5000]);
  assert.equal(Buffer.from(result.derivative.layoutBytes!).toString(), Buffer.from(f.document().layoutBytes!).toString());
  const request = peer.calls[1]; assert.equal(request.protocolVersion, 2);
  if (request.method !== "derive") throw new Error("wrong request");
  assert.equal(request.requestJson, Buffer.from(preflightMedia2Dispatch(input).prepared.bytes).toString());
  const again = await client.cancel(input); assert.equal(again.state, "complete", "a completed native winner remains complete");
});

test("media2 rejects wrong reply identity, raw errors and unsupported terminal fields", async () => {
  const { input } = job(); const peer = new Peer(); const client = new Media2Client(peer); await client.handshake();
  for (const changed of [{ jobId: "e".repeat(32) }, { requestSha256: "f".repeat(64) }, { state: "saved" }, { derivative: {} }, { error: { code: input.ownerToken } }]) {
    peer.handler = r => reply(r, { jobId: r.jobId, requestSha256: r.requestSha256, state: "running", ...changed });
    await assert.rejects(client.inspect(input));
  }
  peer.handler = r => ({ channel: "media", protocolVersion: 2, rpcId: r.rpcId, error: { code: input.ownerToken as "internal" } });
  await assert.rejects(client.inspect(input), e => !String(e).includes(input.ownerToken));
  peer.handler = r => ({ ...reply(r, {}), rpcId: "0".repeat(32) });
  await assert.rejects(client.inspect(input));
});

test("media2 transport uncertainty never sends cancel or loses the restorable job tuple", async () => {
  const { input } = job(); const peer = new Peer(); const client = new Media2Client(peer); await client.handshake();
  peer.handler = () => { throw new Error(input.ownerToken); };
  await assert.rejects(client.derive(input), e => e instanceof Media2TransportError && e.dispatched === "unknown" && !e.message.includes(input.ownerToken));
  assert.equal(peer.calls.filter(r => r.method === "cancel").length, 0);
  const hash = preflightMedia2Dispatch(input).prepared.sha256;
  const reconnect = new Peer(); reconnect.handshake.capabilities = { retainedFrames: false, contactSheet: false, nativeJobAuthority: true };
  const cold = new Media2Client(reconnect); await cold.handshake(); await cold.inspect(input);
  const request = reconnect.calls[1]; if (request.method === "handshake") throw new Error("missing control");
  assert.equal(request.jobId, input.request.jobId); assert.equal(request.requestSha256, hash);
});

test("media2 test peer opt-in cannot manufacture native authority", async () => {
  const peer = new Peer(); peer.handshake.implementation = "test-double"; peer.handshake.capabilities = { retainedFrames: false, contactSheet: false, nativeJobAuthority: false };
  await assert.rejects(new Media2Client(peer).handshake(), /opt-in/);
  const client = new Media2Client(peer, { allowTestDouble: true }); await client.handshake();
  const { f, input } = job(); peer.handler = r => reply(r, { jobId: r.jobId, requestSha256: r.requestSha256, state: "complete", derivative: { envelope: f.document().envelope, receiptJson: Buffer.from(f.document().bytes).toString() } });
  await assert.rejects(client.inspect(input), /native authority/);
});

test("media2 reuses bounded process framing and refuses media1 fallback", async () => {
  const path = fileURLToPath(new URL("./media-child-fixture.ts", import.meta.url));
  for (const mode of ["media2-unavailable", "normal", "media2-oversized"]) {
    const transport = new MediaProcessTransport<Media2RpcRequest, Media2RpcResponse>(spawn(process.execPath, ["--experimental-strip-types", path, mode], { stdio: "pipe" }), 200, { version: 2, encode: encodeMedia2Rpc, maxLine: MEDIA2_MAX_LINE });
    const client = new Media2Client(transport, { allowTestDouble: true });
    try {
      if (mode === "media2-unavailable") {
        const h = await client.handshake(); assert.equal(h.protocolVersion, 2);
        await assert.rejects(client.inspect(job().input), (e: unknown) => e instanceof Media2RpcError && e.code === "unavailable");
      } else await assert.rejects(client.handshake(1000));
    } finally { await client.close(); }
  }
});
