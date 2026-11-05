import type {
  DecodeRequest,
  MediaJobStatus,
  MediaRequest,
  MediaResponse,
  MediaTransport,
} from "./media.ts";
import {
  encodeMediaRequest,
  MEDIA_CHANNEL,
  MEDIA_PROTOCOL_VERSION,
} from "./media.ts";

export type MediaTestScenario =
  | "success"
  | "partial-decode-failure"
  | "cancelled"
  | "timeout"
  | "source-mismatch"
  | "output-exists";

/** Synthetic test peer. References are not PNGs and are never native proof. */
export class ReferenceMediaPeer implements MediaTransport {
  readonly requests: MediaRequest[] = [];
  private closed = false;
  private readonly scenario: MediaTestScenario;
  private readonly jobs = new Map<
    string,
    {
      binding: string;
      status: MediaJobStatus;
      decode?: DecodeRequest;
      timer?: ReturnType<typeof setTimeout>;
    }
  >();
  private readonly waiters = new Map<
    string,
    {
      request: DecodeRequest;
      resolve: (value: MediaResponse) => void;
      reject: (error: Error) => void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();

  constructor(scenario: MediaTestScenario = "success") {
    this.scenario = scenario;
  }

  async request(
    request: MediaRequest,
    timeoutMs = 5000,
  ): Promise<MediaResponse> {
    encodeMediaRequest(request);
    if (this.closed) throw new Error("test media peer closed");
    this.requests.push(request);
    if (this.requests.length > 128) this.requests.shift();
    if (request.method === "handshake")
      return {
        channel: MEDIA_CHANNEL,
        protocolVersion: MEDIA_PROTOCOL_VERSION,
        rpcId: request.rpcId,
        result: {
          channel: MEDIA_CHANNEL,
          protocolVersion: MEDIA_PROTOCOL_VERSION,
          implementation: "test-double",
          capabilities: { decodeFrames: false },
        },
      };
    if (request.method === "inspect") {
      const job = this.jobs.get(request.jobId);
      return job
        ? {
            channel: MEDIA_CHANNEL,
            protocolVersion: MEDIA_PROTOCOL_VERSION,
            rpcId: request.rpcId,
            result: job.status,
          }
        : {
            channel: MEDIA_CHANNEL,
            protocolVersion: MEDIA_PROTOCOL_VERSION,
            rpcId: request.rpcId,
            error: { code: "job_not_found", message: "job not found" },
          };
    }
    if (request.method === "cancel")
      return this.cancel(request.rpcId, request.jobId);

    const s = request.source,
      i = s.identity,
      l = request.limits;
    const binding = JSON.stringify([
      s.bundleId,
      s.bundlePath,
      s.videoPath,
      s.videoOffsetMs,
      i.device,
      i.inode,
      i.size,
      i.modifiedNs,
      [...request.requestedBundleMs],
      request.outputDirectory,
      l.maxFrames,
      l.maxEdge,
      l.maxTotalPixels,
      l.deadlineMs,
    ]);
    const existing = this.jobs.get(request.jobId);
    if (existing)
      return {
        channel: MEDIA_CHANNEL,
        protocolVersion: MEDIA_PROTOCOL_VERSION,
        rpcId: request.rpcId,
        error: {
          code: existing.binding === binding ? "busy" : "identity_conflict",
          message: "job already exists",
        },
      };
    if (this.jobs.size >= 32)
      return {
        channel: MEDIA_CHANNEL,
        protocolVersion: MEDIA_PROTOCOL_VERSION,
        rpcId: request.rpcId,
        error: { code: "busy", message: "job history full" },
      };
    const job: {
      binding: string;
      status: MediaJobStatus;
      decode?: DecodeRequest;
      timer?: ReturnType<typeof setTimeout>;
    } = {
      binding,
      status: { jobId: request.jobId, state: "decoding" },
      decode: request,
    };
    this.jobs.set(request.jobId, job);

    if (this.scenario === "timeout") {
      const result = new Promise<MediaResponse>((resolve, reject) => {
        const timer = setTimeout(() => {
          this.waiters.delete(request.rpcId);
          reject(new Error("media RPC timed out; decode was not cancelled"));
        }, timeoutMs);
        this.waiters.set(request.rpcId, { request, resolve, reject, timer });
      });
      // Delayed completion exercises timeout recovery through inspect.
      job.timer = setTimeout(() => this.finish(job), 100);
      return result;
    }

    if (this.scenario !== "success") {
      const code =
        this.scenario === "source-mismatch"
          ? "source_mismatch"
          : this.scenario === "output-exists"
            ? "invalid_output"
            : this.scenario === "cancelled"
              ? "cancelled"
              : "decode_failed";
      job.status = {
        jobId: request.jobId,
        state: this.scenario === "cancelled" ? "cancelled" : "failed",
        error: { code, message: "synthetic test failure" },
        completedCount: 0,
      };
      return {
        channel: MEDIA_CHANNEL,
        protocolVersion: MEDIA_PROTOCOL_VERSION,
        rpcId: request.rpcId,
        error: { code, message: "synthetic test failure" },
      };
    }
    return this.finish(job, request.rpcId);
  }

  private cancel(rpcId: string, jobId: string): MediaResponse {
    const job = this.jobs.get(jobId);
    if (!job)
      return {
        channel: MEDIA_CHANNEL,
        protocolVersion: MEDIA_PROTOCOL_VERSION,
        rpcId,
        error: { code: "job_not_found", message: "job not found" },
      };
    const state = job.status.state;
    if (state === "complete" || state === "failed" || state === "cancelled")
      return {
        channel: MEDIA_CHANNEL,
        protocolVersion: MEDIA_PROTOCOL_VERSION,
        rpcId,
        result: { jobId, state },
      };
    if (job.timer) clearTimeout(job.timer);
    job.status = {
      jobId,
      state: "cancelled",
      error: { code: "cancelled", message: "cancelled" },
      completedCount: 0,
    };
    for (const [id, waiter] of this.waiters) {
      if (waiter.request.jobId === jobId) {
        clearTimeout(waiter.timer);
        this.waiters.delete(id);
        waiter.reject(new Error("media job cancelled"));
      }
    }
    return {
      channel: MEDIA_CHANNEL,
      protocolVersion: MEDIA_PROTOCOL_VERSION,
      rpcId,
      result: { jobId, state: "cancelled" },
    };
  }

  private finish(
    job: { status: MediaJobStatus; decode?: DecodeRequest },
    rpcId = job.decode!.rpcId,
  ): MediaResponse {
    if (!job.decode || job.status.state === "cancelled")
      throw new Error("media job cancelled");
    const request = job.decode;
    const perFrame = Math.floor(request.limits.maxTotalPixels / request.requestedBundleMs.length);
    const width = Math.min(640, request.limits.maxEdge, Math.max(1, Math.floor(Math.sqrt(perFrame))));
    const height = Math.min(480, request.limits.maxEdge, Math.floor(perFrame / width));
    const frames = request.requestedBundleMs.map(
      (requestedBundleMs, index) => ({
        requestIndex: index + 1,
        requestedBundleMs,
        effectiveVideoMs: Math.max(
          0,
          requestedBundleMs - request.source.videoOffsetMs,
        ),
        clampedToStart: requestedBundleMs < request.source.videoOffsetMs,
        clampedToEnd: false,
        path: `${request.outputDirectory}/frame-${String(index + 1).padStart(3, "0")}.png`,
        width,
        height,
        byteLength: 128,
        sha256: "a".repeat(64),
      }),
    );
    job.status = perFrame < 1
      ? { jobId: request.jobId, state: "failed", error: { code: "limits_exceeded", message: "synthetic pixel budget exceeded" }, completedCount: 0 }
      : { jobId: request.jobId, state: "complete", frames };
    const response = {
      channel: MEDIA_CHANNEL,
      protocolVersion: MEDIA_PROTOCOL_VERSION,
      rpcId,
      ...(job.status.state === "failed" ? { error: job.status.error } : { result: job.status }),
    } as MediaResponse;
    const waiter = this.waiters.get(request.rpcId);
    if (waiter) {
      clearTimeout(waiter.timer);
      this.waiters.delete(request.rpcId);
      waiter.resolve(response);
    }
    return response;
  }

  async close(): Promise<"already-exited"> {
    if (this.closed) return "already-exited";
    this.closed = true;
    for (const waiter of this.waiters.values()) {
      clearTimeout(waiter.timer);
      waiter.reject(new Error("test media peer closed"));
    }
    this.waiters.clear();
    for (const job of this.jobs.values())
      if (job.timer) clearTimeout(job.timer);
    return "already-exited";
  }
}
