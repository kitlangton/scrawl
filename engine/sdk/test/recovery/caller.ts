import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { CapturePublishInput } from "../../src/capture.ts";
import { launchCaptureRecoveryPeer, launchMedia2RecoveryPeer } from "../../src/recovery/index.ts";
import { ChildProcessCaptureTransport } from "../../src/capture.ts";
import { MediaProcessTransport } from "../../src/media.ts";
import { encodeMedia2Rpc, MEDIA2_MAX_LINE, type Media2RpcRequest, type Media2RpcResponse } from "../../src/media2/protocol.ts";
import { releaseLocalRequests } from "../../src/recovery/local.ts";
import { connectRecoveryPeer, RecoveryLaunchError } from "../../src/recovery/process.ts";
const mode = process.argv[2];
const fixtureMode = mode.replace(/^media-/, "");
const channel = mode.startsWith("media-") ? "media/2" : "capture/2";
const completed = process.argv[3];
let constructed = false;
async function nativeCaller(): Promise<void> {
  const capture = await launchCaptureRecoveryPeer(process.env.APPARENT_ENGINE_CAPTURE_BIN!, completed);
  let media: Awaited<ReturnType<typeof launchMedia2RecoveryPeer>> | undefined;
  try {
    const hello = await capture.client.handshake();
    assert.equal(hello.readiness, "unavailable");
    assert.equal(hello.capabilities.displayCapture, false);
    assert.equal(hello.capabilities.sourceDiscovery, false);
    await assert.rejects(capture.client.close(), /peer.release/);
    if (mode === "native-publication") {
      const raw = JSON.parse(readFileSync(process.argv[4], "utf8"));
      const input: CapturePublishInput = { ...raw, planBytes: readFileSync(join(raw.planFile.stagingRoot, raw.planFile.planFile.relativePath)) };
      await assert.rejects(capture.client.publish(input, 100), /operation was not cancelled/);
    } else {
      media = await launchMedia2RecoveryPeer(process.env.APPARENT_ENGINE_MEDIA2_BIN!, completed);
      assert.equal((await media.client.handshake()).implementation, "native");
      await assert.rejects(media.client.close(), /peer.release/);
    }
    // Every local operation above has settled; native outcome can remain unknown.
    const released = await Promise.all([capture.release(), media?.release()]);
    await assert.rejects(capture.client.close(), /peer.release/);
    await assert.rejects(capture.client.handshake(), /unknown|released|ended|exited/);
    if (media) await assert.rejects(media.client.close(), /peer.release/);
    assert.ok(Object.isFrozen(capture.lifetime));
    console.log(JSON.stringify(released));
  } finally { await Promise.all([capture.release(), media?.release()]); }
}
if (mode.startsWith("native-")) await nativeCaller();
else try {
  const fixtureArgs = (value: string) => ["--experimental-strip-types", fileURLToPath(new URL("./fixture.ts", import.meta.url)), value, completed, channel];
  const peer = await connectRecoveryPeer<{ request: () => Promise<unknown> }>(process.execPath, fixtureArgs(fixtureMode === "pair-failure" ? "confirmed" : fixtureMode), channel, child => {
    constructed = true;
    const common = { protocolVersion: 2 as const, clientId: "1".repeat(32), rpcId: "2".repeat(32), method: "handshake" as const };
    if (channel === "media/2") {
      const transport = new MediaProcessTransport<Media2RpcRequest, Media2RpcResponse>(child, 3000, { version: 2, encode: encodeMedia2Rpc, maxLine: MEDIA2_MAX_LINE });
      return { client: { request: () => transport.request({ ...common, channel: "media" }, 60_000) }, releaseLocal: () => transport[releaseLocalRequests]() };
    }
    const transport = new ChildProcessCaptureTransport(child);
    return { client: { request: () => transport.request({ ...common, channel: "capture" }, 60_000) }, releaseLocal: () => transport[releaseLocalRequests]() };
  });
  assert.notEqual(mode, "wrong-nonce");
  if (mode === "pair-failure") {
    await assert.rejects(connectRecoveryPeer(process.execPath, fixtureArgs("wrong-nonce"), "capture/2", () => { throw new Error("unsupported peer exposed a client"); }), RecoveryLaunchError);
  }
  const pending = assert.rejects(peer.client.request(), /released|ended|exited/);
  const release = peer.release();
  assert.equal(peer.release(), release);
  const outcome = await release;
  await pending;
  assert.equal(outcome.connection, "released");
  assert.equal(outcome.runtimeClearance, "not-established");
  assert.equal(outcome.inputClosure, fixtureMode === "lost-ack" ? "unconfirmed" : "confirmed");
  await assert.rejects(peer.client.request(), /released|ended|exited/);
  console.log(JSON.stringify(outcome));
} catch (error) {
  assert.ok(["wrong-nonce", "unsupported"].includes(mode));
  assert.ok(error instanceof RecoveryLaunchError);
  assert.equal(error.dispatched, false);
  assert.equal(error.connection, "released");
  assert.equal(error.nativeExitObserved, true);
  assert.equal(constructed, false);
  console.log("unsupported-before-client");
}
// No process.exit(), unref(), handle access or forced close in this caller.
