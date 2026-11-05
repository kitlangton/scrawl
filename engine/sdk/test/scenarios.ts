import assert from "node:assert/strict";
import { EngineClient, EngineRpcError } from "../src/index.ts";

/** Shared consumer-only checks. The same function accepts a future native client. */
export async function runV1Scenarios(
  client: EngineClient,
  reconnect: EngineClient,
  bundle: { id: string; path: string },
) {
  const hello = await client.handshake();
  assert.equal(hello.protocolVersion, 1);
  const start = client.prepareStart(bundle.id, bundle.path);
  const begun = await client.start(start);
  assert.equal(begun.captureId, start.captureId);
  const replay = await client.start(start);
  assert.equal(replay.captureId, start.captureId);
  // Native work may advance between RPCs; retries preserve identity, not time.
  assert.equal(replay.receipt.captureId, begun.receipt.captureId);
  const inspected = await reconnect.control(
    "inspect",
    start.captureId,
    start.ownerToken,
  );
  assert.equal(inspected.captureId, start.captureId);
  await assert.rejects(
    reconnect.control("inspect", start.captureId, "f".repeat(64)),
    (error: unknown) =>
      error instanceof EngineRpcError && error.code === "permission_denied",
  );
  await assert.rejects(
    reconnect.start({ ...start, bundlePath: bundle.path + ".conflict" }),
    (error: unknown) =>
      error instanceof EngineRpcError && error.code === "identity_conflict",
  );
  const cancelled = await reconnect.control(
    "cancel",
    start.captureId,
    start.ownerToken,
  );
  assert.equal(cancelled.state, "cancelled");
  assert.equal(
    (await client.control("inspect", start.captureId, start.ownerToken)).state,
    "cancelled",
  );
}
