import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { ChildProcessCaptureTransport } from "../../src/capture.ts";
import { releaseLocalRequests } from "../../src/recovery/local.ts";
import { connectRecoveryPeer, RecoveryLaunchError } from "../../src/recovery/process.ts";
const mode = process.argv[2];
const completed = process.argv[3];
let transport: ChildProcessCaptureTransport | undefined;
try {
  const fixtureArgs = (fixtureMode: string) => ["--experimental-strip-types", fileURLToPath(new URL("./fixture.ts", import.meta.url)), fixtureMode, completed];
  const peer = await connectRecoveryPeer(process.execPath, fixtureArgs(mode === "pair-failure" ? "confirmed" : mode), "capture/2", child => {
    transport = new ChildProcessCaptureTransport(child);
    return { client: transport, releaseLocal: () => transport![releaseLocalRequests]() };
  });
  assert.notEqual(mode, "wrong-nonce");
  if (mode === "pair-failure") {
    await assert.rejects(connectRecoveryPeer(process.execPath, fixtureArgs("wrong-nonce"), "capture/2", () => { throw new Error("unsupported peer exposed a client"); }), RecoveryLaunchError);
  }
  const request = { channel: "capture" as const, protocolVersion: 2 as const, clientId: "1".repeat(32), rpcId: "2".repeat(32), method: "handshake" as const };
  const pending = assert.rejects(peer.client.request(request, 60_000), /released|ended|exited/);
  const release = peer.release();
  assert.equal(peer.release(), release);
  const outcome = await release;
  await pending;
  assert.equal(outcome.connection, "released");
  assert.equal(outcome.runtimeClearance, "not-established");
  assert.equal(outcome.inputClosure, mode === "lost-ack" ? "unconfirmed" : "confirmed");
  await assert.rejects(peer.client.request(request, 60_000), /released|ended|exited/);
  console.log(JSON.stringify(outcome));
} catch (error) {
  assert.ok(["wrong-nonce", "unsupported"].includes(mode));
  assert.ok(error instanceof RecoveryLaunchError);
  assert.equal(error.dispatched, false);
  assert.equal(error.connection, "released");
  assert.equal(error.nativeExitObserved, true);
  assert.equal(transport, undefined);
  console.log("unsupported-before-client");
}
// No process.exit(), unref(), handle access or forced close in this caller.
