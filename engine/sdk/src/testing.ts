import type {
  CloseOutcome,
  Handshake,
  Receipt,
  Request,
  Response,
  State,
  Transport,
} from "./index.ts";
import { encodeRequest } from "./index.ts";

/** TEST ONLY protocol peer. It cannot claim readiness or produce artifacts. */
export class ReferenceEngine implements Transport {
  readonly handshake: Handshake = {
    protocolVersion: 1,
    implementation: "test-double",
    capabilities: {
      screenCapture: false,
      audioCapture: false,
      windowCapture: false,
    },
    readiness: "unavailable",
  };
  private readonly records = new Map<
    string,
    {
      owner: string;
      bundleId: string;
      path: string;
      options: string;
      receipt: Receipt;
    }
  >();
  private closed = false;
  async request(request: Request): Promise<Response> {
    encodeRequest(request);
    if (this.closed) throw new Error("test reference peer closed");
    if (request.method === "handshake")
      return {
        protocolVersion: 1,
        rpcId: request.rpcId,
        result: this.handshake,
      };
    const old = this.records.get(request.captureId);
    if (request.method === "start") {
      if (old) {
        if (old.owner !== request.ownerToken)
          return this.error(
            request.rpcId,
            "permission_denied",
            "owner authority rejected",
          );
        if (
          old.bundleId !== request.bundleId ||
          old.path !== request.bundlePath ||
          old.options !== JSON.stringify(request.options)
        )
          return this.error(
            request.rpcId,
            "identity_conflict",
            "capture identity conflicts with original request",
          );
        return {
          protocolVersion: 1,
          rpcId: request.rpcId,
          result: { ...old.receipt, rpcId: request.rpcId },
        };
      }
      const receipt: Receipt = {
        protocolVersion: 1,
        rpcId: request.rpcId,
        captureId: request.captureId,
        state: "starting",
      };
      this.records.set(request.captureId, {
        owner: request.ownerToken,
        bundleId: request.bundleId,
        path: request.bundlePath,
        options: JSON.stringify(request.options),
        receipt,
      });
      return { protocolVersion: 1, rpcId: request.rpcId, result: receipt };
    }
    if (!old) return this.error(request.rpcId, "not_found", "unknown capture");
    if (old.owner !== request.ownerToken)
      return this.error(
        request.rpcId,
        "permission_denied",
        "owner authority rejected",
      );
    const prior = old.receipt.state;
    if (request.method === "stop") {
      if (prior === "starting")
        return this.error(
          request.rpcId,
          "not_ready",
          "capture has not reached recording",
        );
      if (prior === "recording")
        old.receipt = { ...old.receipt, state: "stopping" };
    } else if (
      request.method === "cancel" &&
      ["starting", "recording", "stopping", "finalizing"].includes(prior)
    )
      old.receipt = { ...old.receipt, state: "cancelled" };
    const receipt = { ...old.receipt, rpcId: request.rpcId };
    return { protocolVersion: 1, rpcId: request.rpcId, result: receipt };
  }
  async close(): Promise<CloseOutcome> {
    this.closed = true;
    return "already-exited";
  }
  private error(
    rpcId: string,
    code: NonNullable<Response["error"]>["code"],
    message: string,
  ): Response {
    return { protocolVersion: 1, rpcId, error: { code, message } };
  }
  /** Test setup control; excluded from the shared protocol scenario API. */
  setScenarioState(
    captureId: string,
    state: Extract<
      State,
      "recording" | "stopping" | "finalizing" | "failed" | "cancelled"
    >,
  ) {
    const record = this.records.get(captureId);
    if (!record) throw new Error("missing scenario capture");
    const current = record.receipt.state;
    const next: Record<string, string[]> = {
      starting: ["recording", "cancelled", "failed"],
      recording: ["stopping", "cancelled", "failed"],
      stopping: ["finalizing", "cancelled", "failed"],
      finalizing: ["cancelled", "failed"],
    };
    if (current === state && ["cancelled", "failed"].includes(state)) return;
    if (!next[current]?.includes(state))
      throw new Error(`invalid test transition ${current} -> ${state}`);
    const { error: _error, ...base } = record.receipt;
    record.receipt = {
      ...base,
      state,
      ...(state === "failed"
        ? { error: { code: "unavailable", message: "simulated test failure" } }
        : {}),
    };
  }
  /** Explicit test fixture only; no media is written or claimed as native. */
  setSavedFixture(captureId: string) {
    const record = this.records.get(captureId);
    if (!record) throw new Error("missing scenario capture");
    if (record.receipt.state !== "finalizing")
      throw new Error("synthetic saved fixture requires finalizing state");
    record.receipt = {
      ...record.receipt,
      state: "saved",
      bundlePath: record.path,
      artifactVersion: 1,
      artifactProvenance: "test-synthetic",
    };
  }
}
