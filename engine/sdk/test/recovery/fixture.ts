// Explicit synthetic process fixture: lifecycle-shaped bytes are not native
// authority/effect proof. Native artifacts require separate conformance.
import { writeFileSync } from "node:fs";
const mode = process.argv[2];
const completed = process.argv[3];
const peerId = process.argv.at(-1)!;
const facts = { schema: "apparent.engine-recovery-peer/1", peerId, channel: process.argv[4], implementation: "native", role: "recovery", startsFreshCapture: false, onInputEof: "retain-admitted-finite-work" };
if (mode === "unsupported") {
  setInterval(() => {}, 1000);
} else {
  process.stderr.write(JSON.stringify({ ...facts, peerId: mode === "wrong-nonce" ? "0".repeat(32) : peerId, event: "ready" }) + "\n");
  process.stdin.resume();
  process.stdin.on("data", () => {}); // Deliberately lose every RPC response.
  process.stdin.on("end", () => {
    if (mode !== "lost-ack") process.stderr.write(JSON.stringify({ ...facts, event: "input-closed" }) + "\n");
    // Keep finite process work alive after releasing the caller's connection.
    setTimeout(() => { writeFileSync(completed, "finite-work-finished"); }, mode === "lost-ack" ? 2700 : 700);
  });
}
