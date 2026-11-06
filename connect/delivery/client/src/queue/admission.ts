import { decodeJob, encodeJob, type Job } from "./job.ts";
import path from "node:path";
import { normalizeRecipientServer } from "../recipient/transport.ts";

export const ADMISSION_RETENTION_SECONDS = 7 * 24 * 60 * 60;
export const ADMISSION_MAX_JOBS = 100;

export type AdmissionIntent = "automatic-private" | "explicit-private" | "public-copy";
export interface AdmissionInput {
  intent: AdmissionIntent;
  /** Canonical completed bundle admitted by the caller; string checks are not filesystem proof. */
  bundle: string;
  server: string;
  generation: string;
  now: number;
  retentionSeconds: number;
  enabled: boolean;
  sharingEnabled: boolean;
  encryptionEnabled: boolean;
  /** A fresh caller-generated ID. This module never generates IDs. */
  candidateId: string;
  jobs: readonly Job[];
}
export type AdmissionProposal =
  | Readonly<{ kind: "cancel"; job: Readonly<Job> }>
  | Readonly<{ kind: "prune"; id: string }>;
export type AdmissionResult =
  | Readonly<{ kind: "reusable"; id: string; proposals: readonly AdmissionProposal[] }>
  | Readonly<{ kind: "admit"; job: Readonly<Job>; proposals: readonly AdmissionProposal[] }>
  | Readonly<{ kind: "full"; proposals: readonly AdmissionProposal[] }>;

const invalid = (): never => { throw new Error("invalid_queue_admission"); };
const isObject = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
function valid(input: AdmissionInput): void {
  if (!isObject(input) || !["automatic-private", "explicit-private", "public-copy"].includes(input.intent) ||
      typeof input.bundle !== "string" || !path.isAbsolute(input.bundle) || input.bundle.includes("\0")
      || path.normalize(input.bundle) !== input.bundle || path.extname(input.bundle) !== ".apparent" ||
      typeof input.server !== "string" || normalizeRecipientServer(input.server) !== input.server ||
      typeof input.generation !== "string" || !/^[0-9a-f]{32}$/.test(input.generation) ||
      typeof input.candidateId !== "string" || !/^[0-9a-f]{32}$/.test(input.candidateId) ||
      !Number.isSafeInteger(input.now) || input.now < 0 || !Number.isSafeInteger(input.retentionSeconds) || input.retentionSeconds < 0 ||
      typeof input.enabled !== "boolean" || typeof input.sharingEnabled !== "boolean" || typeof input.encryptionEnabled !== "boolean" ||
      !Array.isArray(input.jobs) || input.jobs.length > ADMISSION_MAX_JOBS) invalid();
  for (const job of input.jobs) {
    if (!isObject(job) || typeof job.id !== "string" || !/^[0-9a-f]{32}$/.test(job.id) || typeof job.bundle !== "string" ||
        typeof job.server !== "string" || typeof job.generation !== "string" || !Number.isSafeInteger(job.created) || job.created < 0 ||
        typeof job.status !== "string" || typeof job.publicCopy !== "boolean" || typeof job.shared !== "boolean") invalid();
    try { encodeJob(job as Job); } catch { invalid(); }
  }
  if (new Set(input.jobs.map(job => job.id)).size !== input.jobs.length) invalid();
  if (input.jobs.some(j => j.id === input.candidateId)) invalid();
}
const elapsed = (now: number, created: number) => Math.max(0, now - created);
const privateMode = (i: AdmissionInput) => i.intent !== "public-copy";

function buildPlan(input: AdmissionInput): AdmissionResult {
  valid(input);
  input = { ...input, jobs: input.jobs.map(job => decodeJob(encodeJob(job))) };
  const proposals: AdmissionProposal[] = [];
  // Mirrors enqueue's gate order; caller facts are only supplied facts, not proof.
  if (!input.enabled || (input.intent === "public-copy" && !input.sharingEnabled)) invalid();

  if (input.intent !== "explicit-private") {
    const match = input.jobs.find(j => j.bundle === input.bundle && j.server === input.server && j.publicCopy === !privateMode(input) &&
      (input.intent !== "public-copy" || (j.generation === input.generation && j.status !== "cancelled" && j.status !== "expired" && elapsed(input.now, j.created) <= input.retentionSeconds)));
    if (match) return { kind: "reusable", id: match.id, proposals };
  } else {
    for (const original of input.jobs) {
      if (!original.publicCopy && original.bundle === input.bundle && original.server === input.server && original.status !== "ready") {
        const cloned = structuredClone(original); cloned.status = "cancelled";
        // Preserve legacy fields while detaching the cancellation proposal.
        proposals.push({ kind: "cancel", job: decodeJob(encodeJob(cloned)) });
        // Rust mutates this working set before retention/pruning and capacity.
        original.status = "cancelled";
      }
    }
  }
  const retained = input.jobs.filter(j => {
    if (input.intent === "public-copy" && !j.publicCopy) return true;
    const prune = elapsed(input.now, j.created) > input.retentionSeconds || (!j.shared && ["ready", "cancelled", "expired"].includes(j.status));
    if (prune) proposals.push({ kind: "prune", id: j.id });
    return !prune;
  });
  if (retained.length >= ADMISSION_MAX_JOBS) return { kind: "full", proposals };

  const encrypted = input.intent !== "public-copy" && input.encryptionEnabled;
  const job = decodeJob(encodeJob({
    id: input.candidateId, bundle: input.bundle, server: input.server, generation: input.generation,
    created: input.now, attempts: 0, nextAttempt: 0,
    status: input.intent === "public-copy" ? "awaiting_share" : "pending", error: null,
    files: [], shared: input.intent === "public-copy", publicCopy: input.intent === "public-copy",
    encrypted, encryption: null, grouped: encrypted,
  } as Job));
  return { kind: "admit", job, proposals };
}

function freeze<T>(value: T): T {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const child of Object.values(value as Record<string, unknown>)) freeze(child);
  }
  return value;
}

/** Pure immutable plan. Supplied facts do not establish consent, server approval
 * or lease ownership. Proposals retain the donor's mutation order, even on full. */
export function planAdmission(input: AdmissionInput): AdmissionResult {
  try { return freeze(buildPlan(input)); } catch { return invalid(); }
}
