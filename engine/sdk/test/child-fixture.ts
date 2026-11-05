import { setTimeout as delay } from "node:timers/promises";
const mode = process.argv[2];
if (mode === "hold") {
  process.on("SIGTERM", () => {});
  process.stderr.write("ready");
  setInterval(() => {}, 1000);
}
let buffer = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", async (chunk) => {
  buffer += chunk;
  while (buffer.includes("\n")) {
    const i = buffer.indexOf("\n"),
      line = buffer.slice(0, i);
    buffer = buffer.slice(i + 1);
    if (mode === "huge") {
      process.stdout.write("x".repeat(70 * 1024));
      continue;
    }
    if (mode === "malformed") {
      process.stdout.write("{broken}\n");
      continue;
    }
    const request = JSON.parse(line) as { rpcId: string };
    if (mode === "unicode-split") {
      const wire = Buffer.from(
        JSON.stringify({
          protocolVersion: 1,
          rpcId: request.rpcId,
          error: { code: "unavailable", message: "écran 🎥" },
        }) + "\r\n",
      );
      const split = wire.indexOf(Buffer.from("🎥")) + 2;
      process.stdout.write(wire.subarray(0, split));
      await delay(5);
      process.stdout.write(wire.subarray(split));
      continue;
    }
    if (mode === "late" && request.rpcId === "3".repeat(32)) {
      await delay(50);
      process.stdout.write(
        JSON.stringify({
          protocolVersion: 1,
          rpcId: request.rpcId,
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
        }) + "\n",
      );
      continue;
    }
    if (mode === "eof") {
      process.exit(0);
    }
    process.stdout.write(
      JSON.stringify({
        protocolVersion: 1,
        rpcId: request.rpcId,
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
      }) + (mode === "crlf" ? "\r\n" : "\n"),
    );
  }
});
process.stdin.on("end", async () => {
  if (mode === "hold") return;
  if (mode === "wait-eof") await delay(50);
  process.exit(0);
});
