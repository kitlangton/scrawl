// Synthetic process fixture: the caller must exit naturally after public close.
// The peer deliberately leaves a finite descendant holding its output pipes.
import { spawn } from "node:child_process";
import { existsSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { CaptureClient, ChildProcessCaptureTransport } from "../../src/capture.ts";

const [mode, marker] = process.argv.slice(2);
const self = fileURLToPath(import.meta.url);
if (mode === "pipe-holder") {
  const deadline = Date.now() + 8000;
  const timer = setInterval(() => {
    const released = existsSync(marker + ".release");
    if (released || Date.now() >= deadline) {
      clearInterval(timer);
      writeFileSync(marker, JSON.stringify({ pid: process.pid, finished: released }));
    }
  }, 20);
} else if (mode === "peer" || mode === "peer-already") {
  const finish = () => {
    const holder = spawn(process.execPath, ["--experimental-strip-types", self, "pipe-holder", marker], {
      stdio: ["ignore", process.stdout, process.stderr],
    });
    holder.once("spawn", () => process.exit(0)); // The synthetic peer's native-exit event.
  };
  if (mode === "peer-already") finish();
  else {
    process.stdin.resume(); process.stdin.once("end", finish);
    writeFileSync(marker + ".ready", "ready");
  }
} else if (mode === "caller-eof" || mode === "caller-already") {
  const child = spawn(process.execPath, ["--experimental-strip-types", self,
    mode === "caller-already" ? "peer-already" : "peer", marker], { stdio: ["pipe", "pipe", "pipe"] });
  const client = new CaptureClient(new ChildProcessCaptureTransport(child, 100));
  if (mode === "caller-already") await new Promise<void>(done => child.once("exit", () => done()));
  else {
    const deadline = Date.now() + 3000;
    while (!existsSync(marker + ".ready")) {
      if (Date.now() >= deadline) throw new Error("synthetic peer did not become ready");
      await new Promise(done => setTimeout(done, 10));
    }
  }
  const outcome = await client.close();
  process.stdout.write(JSON.stringify({ outcome }) + "\n");
} else throw new Error("unknown close fixture mode");
