type Request = {
  rpcId: string;
  method: string;
  jobId: string;
  requestedBundleMs?: number[];
  source?: { videoOffsetMs: number };
  outputDirectory?: string;
  limits?: { maxEdge: number };
};
const mode = process.argv[2];
let buffer = "";
const jobs = new Map<string, { state: string; request: Request }>();
const reply = (request: Request, result: unknown) =>
  process.stdout.write(
    JSON.stringify({
      channel: "media",
      protocolVersion: 1,
      rpcId: request.rpcId,
      result,
    }) + (mode === "crlf" ? "\r\n" : "\n"),
  );
const frames = (request: Request) =>
  (request.requestedBundleMs ?? []).map((requestedBundleMs, index) => ({
    requestIndex: index + 1,
    requestedBundleMs,
    effectiveVideoMs: Math.max(
      0,
      requestedBundleMs - (request.source?.videoOffsetMs ?? 0),
    ),
    clampedToStart: requestedBundleMs < (request.source?.videoOffsetMs ?? 0),
    clampedToEnd: false,
    path: `${request.outputDirectory}/frame-${String(index + 1).padStart(3, "0")}.png`,
    width: Math.min(640, request.limits?.maxEdge ?? 640),
    height: Math.min(480, request.limits?.maxEdge ?? 480),
    byteLength: 128,
    sha256: "a".repeat(64),
  }));

process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk: string) => {
  buffer += chunk;
  while (buffer.includes("\n")) {
    const index = buffer.indexOf("\n");
    const request = JSON.parse(buffer.slice(0, index)) as Request;
    buffer = buffer.slice(index + 1);
    if (mode === "media2-oversized") {
      process.stdout.write("x".repeat(1024 * 1024 + 1));
      continue;
    }
    if (mode === "media2-unavailable") {
      const result = request.method === "handshake"
        ? { result: { channel: "media", protocolVersion: 2, implementation: "test-double", capabilities: { retainedFrames: false, contactSheet: false, nativeJobAuthority: false } } }
        : { error: { code: "unavailable" } };
      process.stdout.write(JSON.stringify({ channel: "media", protocolVersion: 2, rpcId: request.rpcId, ...result }) + "\n");
      continue;
    }
    if (mode === "bad-utf8") {
      process.stdout.write(Buffer.from([0xff, 0x0a]));
      continue;
    }
    if (mode === "oversized") {
      process.stdout.write("x".repeat(64 * 1024 + 1));
      continue;
    }
    if (mode === "wrong-channel" || mode === "wrong-version") {
      process.stdout.write(
        JSON.stringify({
          channel: mode === "wrong-channel" ? "capture" : "media",
          protocolVersion: mode === "wrong-version" ? 2 : 1,
          rpcId: request.rpcId,
          result: {},
        }) + "\n",
      );
      continue;
    }
    if (mode === "unknown-fields") {
      reply(request, {
        channel: "media",
        protocolVersion: 1,
        implementation: "test-double",
        capabilities: { decodeFrames: false },
        unexpected: true,
      });
      continue;
    }
    if (request.method === "handshake") {
      reply(request, {
        channel: "media",
        protocolVersion: 1,
        implementation: "test-double",
        capabilities: { decodeFrames: false },
      });
    } else if (request.method === "decode") {
      const job = { state: "decoding", request };
      jobs.set(request.jobId, job);
      if (mode === "timeout") {
        setTimeout(() => {
          if (job.state !== "decoding") return;
          job.state = "complete";
          reply(request, {
            jobId: request.jobId,
            state: "complete",
            frames: frames(request),
          });
        }, 100);
      } else {
        job.state = "complete";
        reply(request, {
          jobId: request.jobId,
          state: "complete",
          frames: frames(request),
        });
      }
    } else if (request.method === "inspect") {
      const job = jobs.get(request.jobId);
      if (!job)
        process.stdout.write(
          JSON.stringify({
            channel: "media",
            protocolVersion: 1,
            rpcId: request.rpcId,
            error: { code: "job_not_found", message: "job not found" },
          }) + "\n",
        );
      else
        reply(
          request,
          job.state === "complete"
            ? {
                jobId: request.jobId,
                state: "complete",
                frames: frames(job.request),
              }
            : { jobId: request.jobId, state: job.state },
        );
    } else if (request.method === "cancel") {
      const job = jobs.get(request.jobId);
      if (!job)
        process.stdout.write(
          JSON.stringify({
            channel: "media",
            protocolVersion: 1,
            rpcId: request.rpcId,
            error: { code: "job_not_found", message: "job not found" },
          }) + "\n",
        );
      else if (job.state === "complete")
        reply(request, { jobId: request.jobId, state: "complete" });
      else {
        job.state = "cancelled";
        reply(request, { jobId: request.jobId, state: "cancelled" });
      }
    }
  }
});
if (mode === "hold") {
  process.on("SIGTERM", () => {});
  setInterval(() => {}, 1_000);
  process.stderr.write("ready\n");
} else {
  process.stdin.on("end", () => process.exit(0));
}
