import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmod, mkdir, mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "node:test";
import { EngineClient, EngineRpcError, launchEngine } from "../sdk/src/index.ts";

const executable = process.env.APPARENT_ENGINE_BIN;

async function rawExchange(input: string) {
  const root = await mkdtemp(join(await realpath(tmpdir()), "apparent-engine-wire-"));
  const runtime = join(root, "runtime");
  await mkdir(runtime, { mode: 0o700 });
  await chmod(runtime, 0o700);
  try {
    const child = spawn(resolve(executable!), ["--runtime-directory", runtime], { stdio: ["pipe", "pipe", "pipe"] });
    let stdout = "";
    child.stdout.setEncoding("utf8").on("data", (chunk: string) => { stdout += chunk; });
    child.stdin.end(input);
    const [code, signal] = await new Promise<[number | null, NodeJS.Signals | null]>((resolveExit, reject) => {
      child.once("error", reject);
      child.once("close", (exitCode, exitSignal) => resolveExit([exitCode, exitSignal]));
    });
    return { stdout, code, signal };
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

test("published SDK speaks v1 to the standalone Rust process across restart", { skip: !executable }, async () => {
  const root = await mkdtemp(join(await realpath(tmpdir()), "apparent-engine-provider-"));
  let first: EngineClient | undefined;
  let reconnect: EngineClient | undefined;
  try {
    const runtime = join(root, "runtime");
    const bundle = join(root, "capture.apparent");
    await mkdir(runtime, { mode: 0o700 });
    await chmod(runtime, 0o700);
    first = new EngineClient(launchEngine(resolve(executable!), runtime));
    const handshake = await first.handshake();
    assert.deepEqual(handshake, {
      protocolVersion: 1,
      implementation: "native",
      capabilities: { screenCapture: false, audioCapture: false, windowCapture: false },
      readiness: "unavailable",
    });
    const request = first.prepareStart("sdk-native-peer-test", bundle);
    const begun = await first.start(request);
    assert.equal(begun.receipt.state, "failed");
    await first.close();
    first = undefined;

    reconnect = new EngineClient(launchEngine(resolve(executable!), runtime));
    const replay = await reconnect.start(request);
    assert.equal(replay.receipt.captureId, request.captureId);
    assert.equal(replay.receipt.state, "failed");
    await assert.rejects(
      reconnect.control("inspect", request.captureId, "f".repeat(64)),
      (error: unknown) => error instanceof EngineRpcError && error.code === "permission_denied",
    );
    await assert.rejects(
      reconnect.start({ ...request, bundlePath: `${bundle}-conflict` }),
      (error: unknown) => error instanceof EngineRpcError && error.code === "identity_conflict",
    );
    assert.equal((await reconnect.control("inspect", request.captureId, request.ownerToken)).state, "failed");
  } finally {
    await first?.close();
    await reconnect?.close();
    await rm(root, { recursive: true, force: true });
  }
});

test("standalone transport rejects unknown fields and versions and bounds unterminated input", { skip: !executable }, async () => {
  const base = { protocolVersion: 1, clientId: "a".repeat(32), rpcId: "b".repeat(32), method: "handshake" };
  const unknownVersion = await rawExchange(`${JSON.stringify({ ...base, protocolVersion: 2 })}\n`);
  assert.equal(JSON.parse(unknownVersion.stdout).error.code, "unsupported_version");

  const unknownField = await rawExchange(`${JSON.stringify({ ...base, extra: true })}\n`);
  assert.equal(JSON.parse(unknownField.stdout).error.code, "malformed_request");

  const invalidPath = await rawExchange(`${JSON.stringify({
    ...base,
    method: "start",
    ownerToken: "c".repeat(64),
    captureId: "d".repeat(32),
    bundleId: "bundle",
    bundlePath: "/tmp/../capture.apparent",
    options: { screen: true },
  })}\n`);
  assert.equal(JSON.parse(invalidPath.stdout).error.code, "malformed_request");

  const invalidControl = await rawExchange(`${JSON.stringify({
    ...base,
    method: "start",
    ownerToken: "c".repeat(64),
    captureId: "d".repeat(32),
    bundleId: "bundle\u0085",
    bundlePath: "/tmp/capture.apparent",
    options: { screen: true },
  })}\n`);
  assert.equal(JSON.parse(invalidControl.stdout).error.code, "malformed_request");

  const oversized = await rawExchange(`${" ".repeat(65 * 1024)}`);
  assert.equal(oversized.stdout, "");
  assert.equal(oversized.code, 0);

  const duplicate = await rawExchange(`${JSON.stringify(base)}\n${JSON.stringify(base)}\n`);
  assert.equal(duplicate.stdout.trim().split("\n").length, 1);
});
