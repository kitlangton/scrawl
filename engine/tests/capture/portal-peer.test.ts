import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { chmodSync, copyFileSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import test from "node:test";
import { CaptureClient, capture2MaterialsRoot, launchCaptureEngine, validateMaterialsDocument, type CaptureStatus } from "../../sdk/src/capture.ts";

// Explicit native qualification only: never substitute Xvfb or suppress Wayland.
const enabled = process.env.APPARENT_TEST_OMARCHY_PORTAL === "1";
test("public capture/2 records the consented Omarchy portal display and cold-verifies its sealed movie", { skip: !enabled, timeout: 180_000 }, async () => {
  assert.ok(process.env.WAYLAND_DISPLAY && process.env.HYPRLAND_INSTANCE_SIGNATURE);
  const executable = resolve(process.env.APPARENT_ENGINE_CAPTURE_BIN!);
  const evidence = resolve(process.env.APPARENT_CAPTURE_EVIDENCE_DIR!);
  mkdirSync(evidence, { recursive: true, mode: 0o700 });
  const runtime = join(evidence, "runtime");
  mkdirSync(runtime, { mode: 0o700 });
  chmodSync(runtime, 0o700);
  const authority = { captureId: randomBytes(16).toString("hex"), ownerToken: randomBytes(32).toString("hex") };
  writeFileSync(join(evidence, "authority.json"), JSON.stringify(authority), { mode: 0o600 });
  const stage = (value: string) => {
    writeFileSync(join(evidence, "stage.json"), JSON.stringify({ stage: value, captureId: authority.captureId }), { mode: 0o600 });
    console.log(`portal capture: ${value}`);
  };
  const client = new CaptureClient(launchCaptureEngine(executable, runtime, 10_000));
  let terminal = false;
  async function waitFor(states: string[], milliseconds: number): Promise<CaptureStatus> {
    const deadline = Date.now() + milliseconds;
    while (Date.now() < deadline) {
      const status = await client.inspect(authority);
      if (states.includes(status.state)) return status;
      if (status.state === "failed" || status.state === "cancelled") {
        terminal = true;
        writeFileSync(join(evidence, "failure-status.json"), JSON.stringify(status), { mode: 0o600 });
        throw new Error(`portal capture ended as ${status.state}: ${status.error?.code ?? "none"}`);
      }
      await delay(100);
    }
    throw new Error(`portal capture did not reach ${states.join("/")}`);
  }
  try {
    const hello = await client.handshake();
    assert.equal(hello.implementation, "native");
    assert.equal(hello.readiness, "ready");
    assert.equal(hello.capabilities.areaCapture, false);
    const catalog = await client.discoverSources();
    assert.equal(catalog.state, "available");
    assert.equal(catalog.sources.length, 1);
    const source = catalog.sources[0];
    assert.equal(source.kind, "display");
    writeFileSync(join(evidence, "source.json"), JSON.stringify(source), { mode: 0o600 });
    const started = await client.start({ ...authority, source,
      bundleId: "portal-native.apparent", bundlePath: join(evidence, "portal-native.apparent"),
      options: { audio: "none", localOnly: true },
      invocation: { kind: "explicit", invocationId: randomBytes(16).toString("hex"), observedAtMs: Date.now() },
    });
    assert.ok(["starting", "recording"].includes(started.state));
    stage("awaiting-native-consent");
    await waitFor(["recording"], 110_000);
    stage("recording");
    await delay(1_500);
    await client.stop(authority);
    const ready = await waitFor(["materials-ready"], 20_000);
    terminal = true;
    assert.ok(ready.materials);
    const root = capture2MaterialsRoot(runtime, authority.captureId);
    assert.equal(ready.materials.root, root);
    const payloadBytes = readFileSync(join(root, ready.materials.payloadFile.relativePath));
    const document = validateMaterialsDocument(ready.materials, payloadBytes);
    assert.equal(document.payload.identity.captureId, authority.captureId);
    for (const file of document.payload.files) {
      const path = join(root, file.relativePath);
      const stat = statSync(path, { bigint: true });
      assert.equal(stat.ino.toString(), file.identity.inode);
      assert.equal(stat.dev.toString(), file.identity.device);
      assert.equal(createHash("sha256").update(readFileSync(path)).digest("hex"), file.sha256);
    }
    const movie = join(root, document.payload.video.file);
    copyFileSync(movie, join(evidence, "recording.mp4"));
    execFileSync(process.env.APPARENT_FFMPEG ?? "ffmpeg", ["-v", "error", "-i", movie, "-f", "null", "-"], { timeout: 20_000, stdio: "pipe" });
    writeFileSync(join(evidence, "materials.json"), JSON.stringify(document), { mode: 0o600 });
    await client.close();
    const cold = new CaptureClient(launchCaptureEngine(executable, runtime, 10_000));
    try {
      await cold.handshake();
      const inspected = await cold.inspect(authority);
      assert.equal(inspected.state, "materials-ready");
      assert.deepEqual(inspected.materials, ready.materials);
    } finally { await cold.close(); }
    stage("sealed-movie-and-cold-inspection-passed");
  } finally {
    // Explicit synthetic test abandonment authorizes Cancel, never a false Saved.
    // Retain all evidence and authority on failure for cold diagnosis.
    if (!terminal) {
      try { await client.cancel(authority); } catch { /* preserve unknown outcome */ }
    }
    await client.close();
  }
});
