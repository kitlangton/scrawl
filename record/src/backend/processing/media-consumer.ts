import { constants } from "node:fs";
import { randomUUID } from "node:crypto";
import { link, lstat, mkdir, open, realpath, rename, rm, stat, unlink } from "node:fs/promises";
import { dirname, isAbsolute, resolve } from "node:path";
import type { DecodeSelection, ProcessingInput, ProcessingOutput, PublishedMediaClient, SelectedFrame } from "./index.js";

export type MediaJobRequest = Parameters<PublishedMediaClient["decode"]>[0];
export type MediaJobStatus =
  | { jobId: string; state: "accepted" | "decoding" | "cancelling" }
  | { jobId: string; state: "complete"; frames: Array<{ path: string; requestIndex: number; requestedBundleMs: number; effectiveVideoMs: number; width: number; height: number; byteLength: number; sha256: string }> }
  | { jobId: string; state: "failed" | "cancelled"; completedCount: 0; error: { code: string } };
export interface ManagedMediaClient extends PublishedMediaClient {
  inspect(input: MediaJobRequest, timeoutMs?: number): Promise<MediaJobStatus>;
  cancel(jobId: string, timeoutMs?: number): Promise<"cancelling" | "cancelled" | "complete" | "failed">;
}
export interface RecordProcessingFunctions {
  decodeSelectedFrames(media: PublishedMediaClient, selection: DecodeSelection): Promise<SelectedFrame[]>;
  assembleProcessingOutput(input: ProcessingInput): ProcessingOutput;
}
export type ProcessingJobRecord = { schema: 1; state: "submitted" | "complete" | "failed" | "cancelled"; dispatchAttempted: boolean; request: MediaJobRequest; selection: DecodeSelection["frames"]; frames?: SelectedFrame[]; terminal?: "failed" | "cancelled" };

function validateRecord(value: unknown): asserts value is ProcessingJobRecord {
  const r = value as ProcessingJobRecord;
  const integer = (n: unknown) => Number.isSafeInteger(n) && (n as number) >= 0;
  const path = (s: unknown) => typeof s === "string" && !s.includes("\0") && isAbsolute(s) && resolve(s) === s;
  const q = r?.request, source = q?.source, limits = q?.limits;
  if (r?.schema !== 1 || !["submitted", "complete", "failed", "cancelled"].includes(r.state) || typeof r.dispatchAttempted !== "boolean" ||
      !q || !/^[0-9a-f]{32}$/u.test(q.jobId) || !path(q.outputDirectory) || !source || typeof source.bundleId !== "string" || !source.bundleId ||
      !path(source.bundlePath) || !path(source.videoPath) || !Number.isSafeInteger(source.videoOffsetMs) ||
      !source.identity || ![source.identity.device, source.identity.inode, source.identity.modifiedNs].every(s => typeof s === "string" && /^\d+$/u.test(s)) ||
      !integer(source.identity.size) || !limits || !Object.values(limits).every(n => integer(n) && n > 0) ||
      ![limits.maxFrames, limits.maxEdge, limits.maxTotalPixels, limits.deadlineMs].every(n => integer(n) && n > 0) ||
      !Array.isArray(r.selection) || !r.selection.length || r.selection.length > limits.maxFrames || !Array.isArray(q.requestedBundleMs) || q.requestedBundleMs.length !== r.selection.length)
    throw new Error("invalid processing journal");
  let previous = -1;
  for (const [i, frame] of r.selection.entries()) {
    if (!integer(frame.index) || frame.index <= previous || !integer(frame.timeMs) || frame.timeMs !== q.requestedBundleMs[i] || (frame.text !== undefined && typeof frame.text !== "string")) throw new Error("invalid processing journal selection");
    previous = frame.index;
  }
  if (r.state === "submitted") {
    if (r.frames !== undefined || r.terminal !== undefined) throw new Error("invalid unresolved processing journal");
  } else if (r.state === "complete") {
    if (!r.dispatchAttempted || r.terminal !== undefined || !Array.isArray(r.frames) || r.frames.length !== r.selection.length) throw new Error("invalid complete processing journal");
    for (const [i, frame] of r.frames.entries()) {
      const selected = r.selection[i];
      if (frame.index !== selected.index || frame.timeMs !== selected.timeMs || frame.text !== selected.text || !path(frame.absolutePath) || !frame.absolutePath.startsWith(`${q.outputDirectory}/`)) throw new Error("invalid journal frame inventory");
    }
  } else if (r.frames !== undefined || r.terminal !== r.state) throw new Error("invalid terminal processing journal");
}

function assertPrivatePath(path: string) {
  if (!isAbsolute(path) || resolve(path) !== path || path.includes("\0")) throw new TypeError("job record path must be absolute and normalized");
}
async function writeDurable(path: string, value: ProcessingJobRecord, create = false) {
  const parent = dirname(path); const dir = await stat(parent);
  if (!dir.isDirectory() || (dir.mode & 0o077) !== 0 || dir.uid !== process.getuid?.() || await realpath(parent) !== parent) throw new Error("processing journal directory must be private and owned");
  if (!create) { const existing = await lstat(path); if (!existing.isFile() || existing.isSymbolicLink() || existing.nlink !== 1 || existing.uid !== process.getuid?.() || (existing.mode & 0o077) !== 0) throw new Error("processing journal must be a private owned file"); }
  const tmp = `${path}.tmp-${randomUUID()}`;
  const handle = await open(tmp, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY | (constants.O_NOFOLLOW ?? 0), 0o600);
  try { await handle.writeFile(`${JSON.stringify(value)}\n`); await handle.sync(); } finally { await handle.close(); }
  try { if (create) await link(tmp, path); else await rename(tmp, path); }
  finally { await unlink(tmp).catch(() => {}); }
  const dh = await open(parent, constants.O_RDONLY | (constants.O_DIRECTORY ?? 0) | (constants.O_NOFOLLOW ?? 0));
  try { await dh.sync(); } finally { await dh.close(); }
}

/** Durable owner for one media/1 job. Host injects the admitted client and processing functions. */
export class RecordMediaJob {
  readonly client: ManagedMediaClient;
  readonly journalPath: string;
  private record: ProcessingJobRecord;
  private busy = false;
  private readonly processing: RecordProcessingFunctions;
  private constructor(client: ManagedMediaClient, journalPath: string, record: ProcessingJobRecord, processing: RecordProcessingFunctions) {
    this.client = client; this.journalPath = journalPath; this.record = record; this.processing = processing;
  }
  static async create(client: ManagedMediaClient, processing: RecordProcessingFunctions, journalPath: string, selection: DecodeSelection): Promise<RecordMediaJob> {
    selection = structuredClone(selection);
    assertPrivatePath(journalPath);
    if (!selection.frames.length) throw new Error("cannot create an empty media job");
    if (!/^[0-9a-f]{32}$/u.test(selection.jobId) || !isAbsolute(selection.outputDirectory) || resolve(selection.outputDirectory) !== selection.outputDirectory || selection.frames.length > selection.limits.maxFrames) throw new TypeError("invalid media job identity, output path, or frame count");
    const output = await stat(selection.outputDirectory);
    if (!output.isDirectory() || output.uid !== process.getuid?.() || (output.mode & 0o077) !== 0 || await realpath(selection.outputDirectory) !== selection.outputDirectory) throw new Error("decode output directory must be private and owned");
    let priorIndex = 0;
    for (const frame of selection.frames) {
      if (!Number.isSafeInteger(frame.index) || frame.index <= priorIndex || !Number.isSafeInteger(frame.timeMs) || frame.timeMs < 0) throw new TypeError("invalid Record frame selection");
      priorIndex = frame.index;
    }
    const request: MediaJobRequest = { jobId: selection.jobId, source: selection.source, requestedBundleMs: selection.frames.map(frame => frame.timeMs), outputDirectory: selection.outputDirectory, limits: selection.limits };
    const record: ProcessingJobRecord = { schema: 1, state: "submitted", dispatchAttempted: false, request, selection: selection.frames.map(frame => ({ ...frame })) };
    validateRecord(record);
    await writeDurable(journalPath, record, true);
    return new RecordMediaJob(client, journalPath, record, processing);
  }
  static async reopen(client: ManagedMediaClient, processing: RecordProcessingFunctions, journalPath: string): Promise<RecordMediaJob> {
    assertPrivatePath(journalPath); const info = await lstat(journalPath);
    if (!info.isFile() || info.isSymbolicLink() || info.nlink !== 1 || info.uid !== process.getuid?.() || (info.mode & 0o077) !== 0 || await realpath(journalPath) !== journalPath) throw new Error("processing journal must be a private owned file");
    const journal = await open(journalPath, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
    let contents: string; try { contents = await journal.readFile("utf8"); } finally { await journal.close(); }
    const record: unknown = JSON.parse(contents);
    validateRecord(record);
    return new RecordMediaJob(client, journalPath, record, processing);
  }
  get jobId() { return this.record.request.jobId; }
  get state() { return this.record.state; }
  get outputDirectory() { return this.record.request.outputDirectory; }
  get frames() { return this.record.frames ? structuredClone(this.record.frames) : undefined; }
  private async exclusive<T>(operation: () => Promise<T>): Promise<T> {
    if (this.busy) throw new Error("processing job operation already in progress");
    this.busy = true;
    try { return await operation(); } finally { this.busy = false; }
  }
  /** Rejection/timeout leaves the request unresolved; inspect is required before another dispatch or cleanup. */
  async decode(selection: DecodeSelection): Promise<SelectedFrame[]> {
    const input = structuredClone(selection);
    return this.exclusive(() => this.decodeOwned(input));
  }
  private async decodeOwned(selection: DecodeSelection): Promise<SelectedFrame[]> {
    if (selection.jobId !== this.jobId || JSON.stringify(selection.source) !== JSON.stringify(this.record.request.source) || JSON.stringify(selection.limits) !== JSON.stringify(this.record.request.limits) || selection.outputDirectory !== this.outputDirectory || JSON.stringify(selection.frames) !== JSON.stringify(this.record.selection)) throw new Error("selection does not match the durable processing job");
    if (this.record.state !== "submitted" || this.record.dispatchAttempted) throw new Error("processing job requires reconciliation before another decode dispatch");
    this.record = { ...this.record, dispatchAttempted: true }; await writeDurable(this.journalPath, this.record);
    const frames = await this.processing.decodeSelectedFrames(this.client, structuredClone({ ...selection, source: this.record.request.source, limits: this.record.request.limits, frames: this.record.selection }));
    const complete: ProcessingJobRecord = { ...this.record, state: "complete", frames: structuredClone(frames) };
    validateRecord(complete); await writeDurable(this.journalPath, complete); this.record = complete; return structuredClone(frames);
  }
  async inspect(timeoutMs = 5000): Promise<MediaJobStatus> {
    return this.exclusive(() => this.inspectOwned(timeoutMs));
  }
  private async inspectOwned(timeoutMs: number): Promise<MediaJobStatus> {
    const status = await this.client.inspect(structuredClone(this.record.request), timeoutMs);
    if (status.jobId !== this.jobId) throw new Error("media SDK returned a mismatched job identity");
    if (status.state === "complete") {
      const selected = this.record.selection;
      const mapped = await this.processing.decodeSelectedFrames({ decode: async () => ({ jobId: this.jobId, state: "complete", frames: status.frames }) }, {
        frames: selected, jobId: this.jobId, source: this.record.request.source, outputDirectory: this.outputDirectory, limits: this.record.request.limits,
      });
      const complete: ProcessingJobRecord = { ...this.record, dispatchAttempted: true, state: "complete", frames: mapped.map((frame, i) => ({ ...frame, index: selected[i].index, text: selected[i].text })) };
      validateRecord(complete); await writeDurable(this.journalPath, complete); this.record = complete;
    } else if (status.state === "failed" || status.state === "cancelled") {
      const terminal: ProcessingJobRecord = { ...this.record, state: status.state, terminal: status.state };
      validateRecord(terminal); await writeDurable(this.journalPath, terminal); this.record = terminal;
    }
    return status;
  }
  cancel(timeoutMs = 5000) { return this.client.cancel(this.jobId, timeoutMs); }
  async cleanupJournal(): Promise<void> {
    return this.exclusive(() => this.cleanupOwned());
  }
  private async cleanupOwned(): Promise<void> {
    if (this.record.state === "submitted") throw new Error("cannot clean an unresolved media job");
    const info = await lstat(this.journalPath);
    if (!info.isFile() || info.isSymbolicLink() || info.nlink !== 1 || info.uid !== process.getuid?.() || (info.mode & 0o077) !== 0) throw new Error("processing journal must be a private owned file");
    await rm(this.journalPath, { force: false });
  }
}

/** Runs Record's selected decode and semantic output assembly under one durable job owner. */
export async function processCompletedBundle(job: RecordMediaJob, processing: RecordProcessingFunctions, selection: DecodeSelection, semanticInput: Omit<ProcessingInput, "frames">): Promise<{ frames: SelectedFrame[]; output: ProcessingOutput }> {
  processing.assembleProcessingOutput({ ...semanticInput, frames: [] });
  const frames = await job.decode(selection);
  return { frames, output: processing.assembleProcessingOutput({ ...semanticInput, frames }) };
}

/** Creates one isolated private decode output directory; never cleans it implicitly. */
export async function createProcessingOutputDirectory(parent: string, jobId: string): Promise<string> {
  assertPrivatePath(parent); if (!/^[0-9a-f]{32}$/.test(jobId)) throw new TypeError("invalid media job identity");
  await mkdir(parent, { recursive: true, mode: 0o700 });
  const root = await stat(parent); if (!root.isDirectory() || root.uid !== process.getuid?.() || (root.mode & 0o077) !== 0 || await realpath(parent) !== parent) throw new Error("processing output root must be private and owned");
  const path = resolve(parent, jobId); await mkdir(path, { mode: 0o700 }); return path;
}
