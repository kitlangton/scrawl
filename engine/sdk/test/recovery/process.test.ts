import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

async function caller(mode: string, completed: string): Promise<{ output: string; elapsed: number }> {
  const started = performance.now();
  const child = spawn(process.execPath, ["--experimental-strip-types", fileURLToPath(new URL("./caller.ts", import.meta.url)), mode, completed], { stdio: ["ignore", "pipe", "pipe"] });
  let output = "", errors = "";
  child.stdout.on("data", chunk => { output += chunk; });
  child.stderr.on("data", chunk => { errors += chunk; });
  await new Promise<void>((done, reject) => {
    const timer = setTimeout(() => { child.kill("SIGKILL"); reject(new Error(`One-shot caller did not exit: ${errors}`)); }, 6000);
    child.on("error", error => { clearTimeout(timer); reject(error); });
    child.on("exit", code => { clearTimeout(timer); code === 0 ? done() : reject(new Error(errors)); });
  });
  return { output, elapsed: performance.now() - started };
}
for (const mode of ["confirmed", "lost-ack", "pair-failure", "media-confirmed", "media-lost-ack"]) test(`recovery candidate frees caller handles with ${mode}; finite synthetic process work survives`, async () => {
  const root = await mkdtemp(join(tmpdir(), "engine-recovery-local-"));
  const completed = join(root, "completed");
  try {
    const result = await caller(mode, completed);
    assert.ok(result.elapsed < 4000, result.output);
    assert.equal(JSON.parse(result.output).connection, "released");
    // The subprocess has exited; the existing finite child still owns its work.
    await delay(900);
    assert.equal(await readFile(completed, "utf8"), "finite-work-finished");
  } finally { await rm(root, { recursive: true, force: true }); }
});
for (const mode of ["wrong-nonce", "unsupported"]) test(`recovery startup rejects ${mode} before exposing a client and cleans up its fresh launch`, async () => {
  const result = await caller(mode, "/unused");
  assert.equal(result.output.trim(), "unsupported-before-client");
  assert.ok(result.elapsed < 5000);
});
