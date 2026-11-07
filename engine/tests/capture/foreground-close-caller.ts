import { chmodSync, readFileSync, writeFileSync } from "node:fs";
import {
  CaptureClient,
  CaptureRpcError,
  launchCaptureEngine,
  type CaptureCloseOutcome,
  type CaptureRequest,
  type CaptureResponse,
  type CaptureTransport,
} from "../../sdk/src/capture.ts";
import { isDeepStrictEqual } from "node:util";

type StartInput = Parameters<CaptureClient["start"]>[0];

class DropStopResponseTransport implements CaptureTransport {
  private readonly active = new Set<Promise<void>>();
  private readonly native: CaptureTransport;
  private readonly authority: Pick<StartInput, "captureId" | "ownerToken">;
  startDispatches = 0;
  stopDispatches = 0;
  stopResponseDropped = false;

  constructor(
    native: CaptureTransport,
    authority: Pick<StartInput, "captureId" | "ownerToken">,
  ) {
    this.native = native;
    this.authority = authority;
  }

  async request(
    request: CaptureRequest,
    timeoutMs?: number,
  ): Promise<CaptureResponse> {
    let finish!: () => void;
    const settled = new Promise<void>((resolve) => {
      finish = resolve;
    });
    this.active.add(settled);
    try {
      if (request.method === "start") this.startDispatches += 1;
      if (request.method === "stop") {
        this.stopDispatches += 1;
        if (
          this.stopDispatches !== 1 ||
          request.captureId !== this.authority.captureId ||
          request.ownerToken !== this.authority.ownerToken
        ) {
          throw new Error("foreground close fixture refused a non-original Stop");
        }
        const response = await this.native.request(request, timeoutMs);
        const result = response.result;
        if (
          response.error !== undefined ||
          !result ||
          typeof result !== "object" ||
          Array.isArray(result) ||
          !("state" in result) ||
          !["stopping", "materials-ready"].includes(String(result.state))
        ) {
          throw new Error("foreground close fixture did not receive an accepted native Stop");
        }
        this.stopResponseDropped = true;
        throw new Error("foreground close fixture withheld the Stop response");
      }
      return await this.native.request(request, timeoutMs);
    } finally {
      this.active.delete(settled);
      finish();
    }
  }

  async drain(): Promise<boolean> {
    await Promise.all([...this.active]);
    return this.active.size === 0;
  }

  get requestsDrained(): boolean {
    return this.active.size === 0;
  }

  async close(): Promise<CaptureCloseOutcome> {
    if (!(await this.drain())) {
      throw new Error("foreground close fixture found an undrained request");
    }
    return this.native.close();
  }
}

async function main(): Promise<void> {
  const [executable, runtime, authorityPath] = process.argv.slice(2);
  if (!executable || !runtime || !authorityPath) {
    process.stdout.write(`${JSON.stringify({ status: "invalid-fixture-input" })}\n`);
    process.exitCode = 1;
    return;
  }

  const input = JSON.parse(readFileSync(authorityPath, "utf8")) as StartInput;
  const progressPath = `${authorityPath}.progress`;
  const mark = (value: string) => writeFileSync(progressPath, `${value}\n`, { mode: 0o600 });
  mark("input-loaded");
  const native = launchCaptureEngine(executable, runtime, 40_000);
  const transport = new DropStopResponseTransport(native, input);
  const client = new CaptureClient(transport);
  let finishIntent = false;
  let clientClosed = false;
  let closeOutcome: CaptureCloseOutcome | undefined;
  let stage = "input-loaded";

  try {
    stage = "handshake";
    mark(stage);
    const hello = await client.handshake(15_000);
    if (hello.implementation !== "native" || hello.readiness !== "ready") {
      throw new Error("native source readiness was not established");
    }

    stage = "source-discovery";
    mark(stage);
    const discovery = await client.discoverSources();
    const currentSource = discovery.sources.find((source) => source.id === input.source.id);
    if (discovery.state !== "available" || !currentSource) {
      throw new Error("the original X11 source is unavailable to the one-shot peer");
    }
    const { generation: _originalGeneration, ...originalSource } = structuredClone(input.source);
    const { generation: _currentGeneration, ...currentSourceIdentity } = structuredClone(currentSource);
    if (!isDeepStrictEqual(originalSource, currentSourceIdentity)) {
      throw new Error("the original X11 source identity changed before dispatch");
    }
    input.source = currentSource;
    writeFileSync(authorityPath, `${JSON.stringify(input)}\n`, { mode: 0o600 });
    chmodSync(authorityPath, 0o600);

    stage = "start";
    mark(stage);
    const start = await client.start(input);
    if (
      start.captureId !== input.captureId ||
      !["starting", "recording"].includes(start.state)
    ) {
      throw new Error("the original capture request was not admitted");
    }

    const readyBy = Date.now() + 12_000;
    let state = start.state;
    stage = "recording-readiness";
    mark(stage);
    while (state !== "recording" && Date.now() < readyBy) {
      const observed = await client.inspect({
        captureId: input.captureId,
        ownerToken: input.ownerToken,
      });
      state = observed.state;
      if (["failed", "cancelled"].includes(state)) {
        throw new Error("the original capture did not become recording");
      }
      if (state !== "recording") await new Promise((resolve) => setTimeout(resolve, 20));
    }
    if (state !== "recording") {
      throw new Error("recording readiness deadline expired");
    }

    await new Promise((resolve) => setTimeout(resolve, 160));
    finishIntent = true;
    let stopOutcome: "unknown";
    stage = "stop-dispatch";
    mark(stage);
    try {
      await client.stop({ captureId: input.captureId, ownerToken: input.ownerToken });
      throw new Error("the Stop response was unexpectedly returned to the caller");
    } catch (error) {
      if (error instanceof CaptureRpcError) {
        throw new Error("Stop returned a native RPC error instead of an unknown transport outcome");
      }
      if (!(error instanceof Error) || !error.message.includes("outcome is unknown")) {
        throw error;
      }
      stopOutcome = "unknown";
    }

    stage = "request-drain";
    mark(stage);
    const requestsDrained = await transport.drain();
    if (!requestsDrained || transport.startDispatches !== 1 || transport.stopDispatches !== 1 || !transport.stopResponseDropped) {
      throw new Error("the one-shot Stop request did not drain after its response was dropped");
    }

    stage = "ordinary-close";
    mark(stage);
    closeOutcome = await client.close();
    clientClosed = true;
    stage = "complete";
    mark(stage);
    process.stdout.write(`${JSON.stringify({
      status: "finish-intent-complete",
      intent: "finish",
      stopOutcome,
      requestsDrained,
      startDispatches: transport.startDispatches,
      stopDispatches: transport.stopDispatches,
      stopResponseDropped: transport.stopResponseDropped,
      closeOutcome,
    })}\n`);
    process.exitCode = 3;
  } catch {
    if (finishIntent && !clientClosed) {
      try {
        await transport.drain();
        closeOutcome = await client.close();
        clientClosed = true;
      } catch {
        // The parent harness retains this private runtime and treats caller
        // termination as failure, never as native completion evidence.
      }
    } else if (transport.startDispatches === 0 && !clientClosed) {
      try {
        closeOutcome = await client.close();
        clientClosed = true;
      } catch {
        // No capture was admitted; preserve the fixture's original failure.
      }
    }
    process.stdout.write(`${JSON.stringify({
      status: "finish-intent-failed",
      stage,
      stopOutcome: finishIntent ? "unknown" : "not-issued",
      closeOutcome: closeOutcome ?? "unobserved",
      requestsDrained: transport.requestsDrained,
      startDispatches: transport.startDispatches,
      stopDispatches: transport.stopDispatches,
      stopResponseDropped: transport.stopResponseDropped,
    })}\n`);
    process.exitCode = 1;
  }
}

await main();
