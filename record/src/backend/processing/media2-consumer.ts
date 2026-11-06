import { isDeepStrictEqual } from "node:util";
import {
  Media2PreflightError,
  Media2RpcError,
  encodeContactSheetLayout,
  preflightMedia2Dispatch,
  validateDerivative2Document,
} from "@apparent/engine-sdk/media2";
import type {
  Derivative2DocumentInput,
  Media2Client,
  Media2JobInput,
  Media2JobRequest,
  Media2JobStatus,
} from "@apparent/engine-sdk/media2";
import type { MaterialsDocument } from "@apparent/engine-sdk/capture";
import { Media2Store } from "./media2-store.js";

type Client = Media2Client;
export type RecordMedia2JobState = "ready" | "submitted" | "running" | "cancelling" | "unknown" | "unavailable" | "complete" | "failed" | "cancelled";
type StoredDerivative = {
  envelope: Derivative2DocumentInput["envelope"];
  bytesBase64: string;
  request: Media2JobRequest;
  layoutBytesBase64: string | null;
};
type Media2JobRecord = {
  schema: 1;
  state: RecordMedia2JobState;
  dispatchAttempted: boolean;
  cancelAttempted: boolean;
  request: Media2JobRequest;
  requestSha256: string;
  materials: MaterialsDocument;
  derivative?: StoredDerivative;
  error?: { code: string };
};

function validateRecord(value: unknown): asserts value is Media2JobRecord {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("invalid retained media2 journal");
  const r = value as Media2JobRecord;
  const states: RecordMedia2JobState[] = ["ready", "submitted", "running", "cancelling", "unknown", "unavailable", "complete", "failed", "cancelled"];
  if (r.schema !== 1 || !states.includes(r.state) || typeof r.dispatchAttempted !== "boolean" || typeof r.cancelAttempted !== "boolean" ||
      !r.request || !/^[0-9a-f]{32}$/u.test(r.request.jobId) || !/^[0-9a-f]{64}$/u.test(r.requestSha256) || !r.materials ||
      Object.hasOwn(r, "ownerToken")) throw new Error("invalid retained media2 journal");
  const undispatched = r.state === "ready" || r.state === "unavailable";
  if (undispatched !== !r.dispatchAttempted) throw new Error("invalid retained media2 dispatch state");
  if ((r.state === "complete") !== (r.derivative !== undefined)) throw new Error("invalid retained media2 derivative state");
  if (r.state === "failed" && !r.error) throw new Error("invalid retained media2 failure state");
  if (r.state !== "failed" && r.error !== undefined) throw new Error("invalid retained media2 failure state");
}

function storedDerivative(input: Derivative2DocumentInput): StoredDerivative {
  return {
    envelope: structuredClone(input.envelope),
    bytesBase64: Buffer.from(input.bytes).toString("base64"),
    request: structuredClone(input.request),
    layoutBytesBase64: input.layoutBytes === null ? null : Buffer.from(input.layoutBytes).toString("base64"),
  };
}

function restoreDerivative(value: StoredDerivative): Derivative2DocumentInput {
  const bytes = Buffer.from(value.bytesBase64, "base64");
  if (bytes.toString("base64") !== value.bytesBase64) throw new Error("invalid retained media2 receipt bytes");
  const layoutBytes = value.layoutBytesBase64 === null ? null : Buffer.from(value.layoutBytesBase64, "base64");
  if (layoutBytes && layoutBytes.toString("base64") !== value.layoutBytesBase64) throw new Error("invalid retained media2 layout bytes");
  return { envelope: structuredClone(value.envelope), bytes: new Uint8Array(bytes), request: structuredClone(value.request), layoutBytes: layoutBytes && new Uint8Array(layoutBytes) };
}

function validateComplete(input: Derivative2DocumentInput, request: Media2JobRequest, materials: MaterialsDocument): Derivative2DocumentInput {
  if (!isDeepStrictEqual(input.request, request)) throw new Error("media2 derivative changed the original request");
  const expectedLayout = request.contactSheet ? encodeContactSheetLayout(request.contactSheet) : null;
  const sameLayout = input.layoutBytes === null
    ? expectedLayout === null
    : expectedLayout !== null && Buffer.from(input.layoutBytes).equals(Buffer.from(expectedLayout));
  if (!sameLayout) throw new Error("media2 derivative changed the original layout bytes");
  const envelope = validateDerivative2Document(input, materials);
  return { ...structuredClone(input), envelope };
}

function isUndispatched(error: unknown): boolean {
  return error instanceof Media2PreflightError || (!!error && typeof error === "object" && (error as { dispatched?: unknown }).dispatched === false);
}

/** Durable Record owner for one caller-selected retained-media/2 request.
 * The caller injects the already-composed Media2Client and supplies ownerToken
 * for each operation; the token is never retained or journaled. */
export class RecordMedia2Job {
  readonly journalPath: string;
  private readonly client: Client;
  private record: Media2JobRecord;
  private busy = false;
  private readonly store: Media2Store;

  private constructor(client: Client, journalPath: string, record: Media2JobRecord) {
    this.store = new Media2Store(journalPath);
    this.client = client;
    this.journalPath = journalPath;
    this.record = record;
  }

  static async create(client: Client, journalPath: string, value: Media2JobInput): Promise<RecordMedia2Job> {
    const store = new Media2Store(journalPath);
    const snapshot = structuredClone(value);
    const { input, prepared } = preflightMedia2Dispatch(snapshot);
    const record: Media2JobRecord = {
      schema: 1, state: "ready", dispatchAttempted: false, cancelAttempted: false,
      request: structuredClone(input.request), requestSha256: prepared.sha256, materials: structuredClone(input.materials),
    };
    validateRecord(record);
    await store.create(record);
    return new RecordMedia2Job(client, journalPath, record);
  }

  static async reopen(client: Client, journalPath: string, value: Media2JobInput): Promise<RecordMedia2Job> {
    const store = new Media2Store(journalPath);
    const record = await store.read() as Media2JobRecord;
    validateRecord(record);
    const snapshot = structuredClone(value);
    const { input, prepared } = preflightMedia2Dispatch(snapshot);
    if (prepared.sha256 !== record.requestSha256 || !isDeepStrictEqual(input.request, record.request) || !isDeepStrictEqual(input.materials, record.materials))
      throw new Error("reopened media2 job does not match its original request and materials");
    if (record.state === "complete") {
      const document = validateComplete(restoreDerivative(record.derivative!), record.request, record.materials);
      record.derivative = storedDerivative(document);
    }
    return new RecordMedia2Job(client, journalPath, record);
  }

  /** Reopen the exact retained request/material binding without rereading source files. */
  static async reopenRetained(client: Client, journalPath: string, ownerToken: string, expected: {
    request: Media2JobRequest; materials: MaterialsDocument["manifest"]["key"]; leaseId: string; bundleId: string;
  }): Promise<RecordMedia2Job> {
    const store = new Media2Store(journalPath);
    const record = await store.read() as Media2JobRecord;
    validateRecord(record);
    const { prepared } = preflightMedia2Dispatch({ ownerToken, request: record.request, materials: record.materials });
    if (prepared.sha256 !== record.requestSha256 || !isDeepStrictEqual(record.request, expected.request) ||
        !isDeepStrictEqual(record.materials.manifest.key, expected.materials) || record.materials.manifest.leaseId !== expected.leaseId ||
        record.materials.payload.bundleId !== expected.bundleId)
      throw new Error("reopened retained media2 job conflicts with the original coordinator binding");
    if (record.state === "complete") {
      const document = validateComplete(restoreDerivative(record.derivative!), record.request, record.materials);
      record.derivative = storedDerivative(document);
    }
    return new RecordMedia2Job(client, journalPath, record);
  }

  get jobId(): string { return this.record.request.jobId; }
  get state(): RecordMedia2JobState { return this.record.state; }
  get requestSha256(): string { return this.record.requestSha256; }
  get derivative(): Derivative2DocumentInput | undefined {
    return this.record.derivative ? restoreDerivative(this.record.derivative) : undefined;
  }

  private async exclusive<T>(operation: () => Promise<T>): Promise<T> {
    if (this.busy) throw new Error("retained media2 job operation already in progress");
    this.busy = true;
    try { await this.refresh(); return await operation(); }
    finally { this.busy = false; }
  }

  private async refresh(): Promise<void> {
    const current = await this.store.read() as Media2JobRecord;
    validateRecord(current);
    if (current.requestSha256 !== this.record.requestSha256 || !isDeepStrictEqual(current.request, this.record.request) ||
        !isDeepStrictEqual(current.materials, this.record.materials)) throw new Error("retained media2 journal original binding changed");
    this.record = current;
  }

  private async persist(next: Media2JobRecord): Promise<void> {
    validateRecord(next);
    if (!await this.store.compareExchange(this.record, next)) {
      await this.refresh();
      throw new Error("retained media2 journal changed; inspection is required");
    }
    this.record = next;
  }

  private async markUnknown(): Promise<void> {
    await this.refresh();
    if (!["complete", "failed", "cancelled"].includes(this.record.state))
      await this.persist({ ...this.record, state: "unknown" });
  }

  private operationInput(ownerToken: string, timeoutMs: number): Media2JobInput {
    const { input, prepared } = preflightMedia2Dispatch({ ownerToken, request: this.record.request, materials: this.record.materials }, timeoutMs);
    if (prepared.sha256 !== this.record.requestSha256) throw new Error("retained media2 request hash changed");
    return input;
  }

  /** Preflight is undispatched. Once submitted is durable, any uncertain reply
   * must be reconciled with inspect; this method never retries automatically. */
  async derive(ownerToken: string, timeoutMs = 5000): Promise<Media2JobStatus> {
    return this.exclusive(async () => {
      if (this.record.dispatchAttempted || !["ready", "unavailable"].includes(this.record.state))
        throw new Error("retained media2 job requires inspection before another derive");
      const input = this.operationInput(ownerToken, timeoutMs);
      await this.persist({ ...this.record, state: "submitted", dispatchAttempted: true, error: undefined });
      let status: Media2JobStatus;
      try { status = await this.client.derive(input, timeoutMs); }
      catch (error) {
        if (isUndispatched(error)) {
          const unavailable = error instanceof Media2RpcError && error.code === "unavailable";
          await this.persist({ ...this.record, state: unavailable ? "unavailable" : "ready", dispatchAttempted: false });
        } else await this.markUnknown();
        throw error;
      }
      return this.acceptStatus(status);
    });
  }

  async inspect(ownerToken: string, timeoutMs = 5000): Promise<Media2JobStatus> {
    return this.exclusive(async () => {
      if (!this.record.dispatchAttempted) throw new Error("retained media2 job has no dispatched request to inspect");
      const input = this.operationInput(ownerToken, timeoutMs);
      let status: Media2JobStatus;
      try { status = await this.client.inspect(input, timeoutMs); }
      catch {
        // A failed observation cannot erase a previously validated terminal result.
        if (!["complete", "failed", "cancelled"].includes(this.record.state)) {
          await this.markUnknown();
        }
        throw new Error("retained media2 inspect outcome is unknown");
      }
      return this.acceptStatus(status);
    });
  }

  /** Cancellation is only sent after an explicit caller request. */
  async cancel(ownerToken: string, timeoutMs = 5000): Promise<Media2JobStatus> {
    return this.exclusive(async () => {
      if (!this.record.dispatchAttempted || !["submitted", "running", "cancelling", "unknown"].includes(this.record.state))
        throw new Error("retained media2 job is not cancellable");
      const input = this.operationInput(ownerToken, timeoutMs);
      const previous = this.record;
      await this.persist({ ...this.record, state: "cancelling", cancelAttempted: true });
      let status: Media2JobStatus;
      try { status = await this.client.cancel(input, timeoutMs); }
      catch (error) {
        if (isUndispatched(error)) await this.persist(previous);
        else await this.markUnknown();
        throw error;
      }
      return this.acceptStatus(status);
    });
  }

  private async acceptStatus(status: Media2JobStatus): Promise<Media2JobStatus> {
    await this.refresh();
    if (status.jobId !== this.jobId || status.requestSha256 !== this.record.requestSha256) {
      if (!["complete", "failed", "cancelled"].includes(this.record.state)) {
        await this.markUnknown();
      }
      throw new Error("media2 client returned a mismatched original job binding");
    }
    if (["complete", "failed", "cancelled"].includes(this.record.state) && status.state !== this.record.state)
      throw new Error("media2 observation conflicts with the durable terminal result");
    let next: Media2JobRecord;
    if (status.state === "complete") {
      const derivative = validateComplete(status.derivative, this.record.request, this.record.materials);
      next = { ...this.record, state: "complete", dispatchAttempted: true, derivative: storedDerivative(derivative), error: undefined };
      status = { ...status, derivative };
    } else if (status.state === "failed") {
      next = { ...this.record, state: "failed", dispatchAttempted: true, error: { ...status.error } };
    } else {
      next = { ...this.record, state: status.state, dispatchAttempted: true, error: undefined };
    }
    validateRecord(next);
    if (["complete", "failed", "cancelled"].includes(this.record.state) &&
        (!isDeepStrictEqual(next.derivative, this.record.derivative) || !isDeepStrictEqual(next.error, this.record.error)))
      throw new Error("media2 observation conflicts with the durable terminal result");
    await this.persist(next);
    return structuredClone(status);
  }

  async cleanupJournal(): Promise<void> {
    return this.exclusive(async () => {
      const safe = !this.record.dispatchAttempted || ["complete", "failed", "cancelled"].includes(this.record.state);
      if (!safe) throw new Error("cannot clean an unresolved retained media2 job");
      if (!await this.store.compareExchange(this.record, null)) throw new Error("retained media2 journal changed during cleanup");
    });
  }
}
